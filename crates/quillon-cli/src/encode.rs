//! Hardware-encode selection for the ffmpeg sidecar (plan §12 v0.6,
//! design note D4; research/E §2): vainfo-style probing of
//! `ffmpeg -encoders`, an explicit `software` default, `auto` preferring
//! VAAPI then NVENC with a printed fallback note, and named encoders that
//! error listing what ffmpeg actually offers — never a silent fallback.
//! Hardware encoders change encoded bytes only; frame RGBA stays the
//! deterministic, golden-tested path.

use std::process::Command;

/// Runtime hardware availability (research/E §2: an encoder appearing in
/// `ffmpeg -encoders` only means it was compiled in; VAAPI needs a DRI
/// render node, NVENC needs an NVIDIA device — "vainfo verifies entry
/// points").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HwDevices {
    /// A VAAPI DRI render node exists (/dev/dri/renderD128).
    pub vaapi: bool,
    /// An NVIDIA device node exists.
    pub nvidia: bool,
}

impl HwDevices {
    pub fn probe() -> HwDevices {
        HwDevices {
            vaapi: std::path::Path::new("/dev/dri/renderD128").exists(),
            nvidia: std::path::Path::new("/dev/nvidia0").exists()
                || std::path::Path::new("/dev/nvidiactl").exists(),
        }
    }
}

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

/// Resolve the `--encode` choice against the compiled-in encoders AND the
/// hardware devices present. Deterministic and testable: both are inputs.
pub fn resolve(
    choice: &EncodeChoice,
    available: &[String],
    devices: &HwDevices,
) -> Result<EncoderPlan, String> {
    match choice {
        EncodeChoice::Software => Ok(EncoderPlan {
            codec: "libx264".to_string(),
            hardware: false,
            fallback_note: None,
        }),
        EncodeChoice::Auto => {
            // An encoder that is compiled in but has no device is not
            // usable: choosing it would make ffmpeg die on the first
            // frame (observed: h264_vaapi without /dev/dri).
            let vaapi_usable = devices.vaapi && available.iter().any(|a| a == "h264_vaapi");
            let nvenc_usable = devices.nvidia && available.iter().any(|a| a == "h264_nvenc");
            if vaapi_usable {
                return Ok(EncoderPlan {
                    codec: "h264_vaapi".to_string(),
                    hardware: true,
                    fallback_note: None,
                });
            }
            if nvenc_usable {
                return Ok(EncoderPlan {
                    codec: "h264_nvenc".to_string(),
                    hardware: true,
                    fallback_note: None,
                });
            }
            let reason = if available.iter().any(|a| a == "h264_vaapi") && !devices.vaapi {
                "h264_vaapi is compiled into ffmpeg but no /dev/dri render device is present"
            } else if available.iter().any(|a| a == "h264_nvenc") && !devices.nvidia {
                "h264_nvenc is compiled into ffmpeg but no NVIDIA device is present"
            } else {
                "no hardware encoder found (looked for h264_vaapi, h264_nvenc)"
            };
            Ok(EncoderPlan {
                codec: "libx264".to_string(),
                hardware: false,
                fallback_note: Some(format!("{reason}; using software libx264")),
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

    fn no_hw() -> HwDevices {
        HwDevices::default()
    }

    fn all_hw() -> HwDevices {
        HwDevices {
            vaapi: true,
            nvidia: true,
        }
    }

    fn with_hw() -> Vec<String> {
        let mut list = available();
        list.push("h264_vaapi".to_string());
        list
    }

    #[test]
    fn software_is_pinned_libx264() {
        let plan = resolve(&EncodeChoice::Software, &available(), &no_hw()).unwrap();
        assert_eq!(plan.codec, "libx264");
        assert!(!plan.hardware);
        assert!(plan.fallback_note.is_none());
    }

    #[test]
    fn auto_prefers_vaapi_then_nvenc_and_notes_the_fallback() {
        let plan = resolve(&EncodeChoice::Auto, &with_hw(), &all_hw()).unwrap();
        assert_eq!(plan.codec, "h264_vaapi");
        assert!(plan.hardware);

        let mut nvenc = available();
        nvenc.push("h264_nvenc".to_string());
        let plan = resolve(&EncodeChoice::Auto, &nvenc, &all_hw()).unwrap();
        assert_eq!(plan.codec, "h264_nvenc");

        let plan = resolve(&EncodeChoice::Auto, &available(), &no_hw()).unwrap();
        assert_eq!(plan.codec, "libx264");
        assert!(!plan.hardware);
        assert!(plan.fallback_note.is_some(), "fallback is never silent");
    }

    #[test]
    fn named_missing_encoders_error_with_the_offer() {
        let error = resolve(
            &EncodeChoice::Named("h264_bogus".into()),
            &available(),
            &no_hw(),
        )
        .unwrap_err();
        assert!(error.contains("h264_bogus"), "{error}");
        assert!(error.contains("libx264"), "must list what exists: {error}");
    }

    #[test]
    fn compiled_in_without_a_device_is_not_usable() {
        // The exact case this sandbox hit: h264_vaapi is compiled into
        // ffmpeg, but there is no /dev/dri render node — choosing it
        // would kill ffmpeg on the first frame.
        let plan = resolve(&EncodeChoice::Auto, &with_hw(), &no_hw()).unwrap();
        assert_eq!(plan.codec, "libx264");
        assert!(
            plan.fallback_note
                .as_deref()
                .unwrap()
                .contains("no /dev/dri render device"),
            "the note names the real reason: {:?}",
            plan.fallback_note
        );
    }

    #[test]
    fn named_available_encoders_are_used() {
        let plan = resolve(
            &EncodeChoice::Named("libx265".into()),
            &available(),
            &no_hw(),
        )
        .unwrap();
        assert_eq!(plan.codec, "libx265");
        assert!(!plan.hardware, "software codecs are not hardware");
    }

    #[test]
    fn encoder_list_parsing_skips_the_banner() {
        let raw = "ffmpeg version 7.x Copyright\nEncoders:\n \
 -----\n \
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
