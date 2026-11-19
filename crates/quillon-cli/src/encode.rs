//! Hardware-encode selection for the ffmpeg sidecar (plan §12 v0.6,
//! design note D4; research/E §2): vainfo-style probing of
//! `ffmpeg -encoders`, an explicit `software` default, `auto` preferring
//! VAAPI then NVENC with a printed fallback note, and named encoders that
//! error listing what ffmpeg actually offers — never a silent fallback.
//! Hardware encoders change encoded bytes only; frame RGBA stays the
//! deterministic, golden-tested path.

use std::process::Command;

/// The `--encode` flag's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeChoice {
    Software,
    Auto,
    Named(String),
}

impl EncodeChoice {
    pub fn parse(text: &str) -> Result<EncodeChoice, String> {
        match text {
            "software" => Ok(EncodeChoice::Software),
            "auto" => Ok(EncodeChoice::Auto),
            other if other.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
                Ok(EncodeChoice::Named(other.to_string()))
            }
            other => Err(format!(
                "--encode must be \"software\", \"auto\" or an encoder name, got {other:?}"
            )),
        }
    }
}

/// What the render will use, after probing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderPlan {
    /// The ffmpeg `-c:v` value ("libx264" for software).
    pub codec: String,
    /// True when a hardware encoder was selected.
    pub hardware: bool,
    /// Set when `auto` fell back to software — printed once, never silent.
    pub fallback_note: Option<String>,
}

/// Parse `ffmpeg -hide_banner -encoders` output into encoder names.
pub fn parse_encoder_list(output: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in output.lines().skip_while(|l| !l.contains("-----")) {
        // Lines look like " V....D h264_vaapi    H.264 ..."
        let mut fields = line.split_whitespace();
        let _flags = fields.next();
        if let Some(name) = fields.next() {
            if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && name != "-----" {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// Query the system ffmpeg for its encoder list (the probe).
pub fn probe_encoders() -> Result<Vec<String>, String> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .map_err(|e| format!("cannot run ffmpeg to probe encoders: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(parse_encoder_list(&text))
}

/// Resolve the `--encode` choice against the available encoders.
/// Deterministic and testable: the encoder list is an input.
pub fn resolve(choice: &EncodeChoice, available: &[String]) -> Result<EncoderPlan, String> {
    match choice {
        EncodeChoice::Software => Ok(EncoderPlan {
            codec: "libx264".to_string(),
            hardware: false,
            fallback_note: None,
        }),
        EncodeChoice::Auto => {
            for candidate in ["h264_vaapi", "h264_nvenc"] {
                if available.iter().any(|a| a == candidate) {
                    return Ok(EncoderPlan {
                        codec: candidate.to_string(),
                        hardware: true,
                        fallback_note: None,
                    });
                }
            }
            Ok(EncoderPlan {
                codec: "libx264".to_string(),
                hardware: false,
                fallback_note: Some(
                    "no hardware encoder found (looked for h264_vaapi, h264_nvenc); using software libx264"
                        .to_string(),
                ),
            })
        }
        EncodeChoice::Named(name) => {
            if available.iter().any(|a| a == name) {
                let hardware = name.contains("vaapi")
                    || name.contains("nvenc")
                    || name.contains("qsv")
                    || name.contains("amf");
                Ok(EncoderPlan {
                    codec: name.clone(),
                    hardware,
                    fallback_note: None,
                })
            } else {
                Err(format!(
                    "encoder \"{name}\" is not available in this ffmpeg; it offers: {}",
                    summarize_available(available)
                ))
            }
        }
    }
}

fn summarize_available(available: &[String]) -> String {
    let interesting: Vec<&String> = available
        .iter()
        .filter(|a| {
            a.contains("h264") || a.contains("hevc") || a.contains("libx") || a.contains("aac")
        })
        .collect();
    if interesting.is_empty() {
        format!("{} encoder(s) in total", available.len())
    } else {
        interesting
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available() -> Vec<String> {
        vec![
            "libx264".to_string(),
            "aac".to_string(),
            "libx265".to_string(),
        ]
    }

    fn with_hw() -> Vec<String> {
        let mut list = available();
        list.push("h264_vaapi".to_string());
        list
    }

    #[test]
    fn software_is_pinned_libx264() {
        let plan = resolve(&EncodeChoice::Software, &available()).unwrap();
        assert_eq!(plan.codec, "libx264");
        assert!(!plan.hardware);
        assert!(plan.fallback_note.is_none());
    }

    #[test]
    fn auto_prefers_vaapi_then_nvenc_and_notes_the_fallback() {
        let plan = resolve(&EncodeChoice::Auto, &with_hw()).unwrap();
        assert_eq!(plan.codec, "h264_vaapi");
        assert!(plan.hardware);

        let mut nvenc = available();
        nvenc.push("h264_nvenc".to_string());
        let plan = resolve(&EncodeChoice::Auto, &nvenc).unwrap();
        assert_eq!(plan.codec, "h264_nvenc");

        let plan = resolve(&EncodeChoice::Auto, &available()).unwrap();
        assert_eq!(plan.codec, "libx264");
        assert!(!plan.hardware);
        assert!(plan.fallback_note.is_some(), "fallback is never silent");
    }

    #[test]
    fn named_missing_encoders_error_with_the_offer() {
        let error = resolve(&EncodeChoice::Named("h264_bogus".into()), &available()).unwrap_err();
        assert!(error.contains("h264_bogus"), "{error}");
        assert!(error.contains("libx264"), "must list what exists: {error}");
    }

    #[test]
    fn named_available_encoders_are_used() {
        let plan = resolve(&EncodeChoice::Named("libx265".into()), &available()).unwrap();
        assert_eq!(plan.codec, "libx265");
        assert!(!plan.hardware, "software codecs are not hardware");
    }

    #[test]
    fn encoder_list_parsing_skips_the_banner() {
        let raw = "ffmpeg version 7.x Copyright\nEncoders:\n \
 V....D aac    AAC encoder\n \
 V....D libx264  libx264 H.264\n";
        let names = parse_encoder_list(raw);
        assert_eq!(names, vec!["aac", "libx264"]);
    }

    #[test]
    fn bogus_flag_values_are_rejected() {
        assert!(EncodeChoice::parse("software").is_ok());
        assert!(EncodeChoice::parse("auto").is_ok());
        assert!(EncodeChoice::parse("h264_vaapi").is_ok());
        assert!(EncodeChoice::parse("bad name").is_err());
    }
}
