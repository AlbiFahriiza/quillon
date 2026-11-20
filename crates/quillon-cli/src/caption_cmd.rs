//! `quillon caption import|export` (plan §12 v0.5). SRT and VTT go through
//! our own core parsers; ASS/SSA through rsubs-lib (research/H §1), taking
//! timing and text only. Imported cues always pass timing regularization
//! unless `--no-regularize` is passed — raw parser output is never burned
//! directly.

use quillon_core::caption::{CaptionCue, CaptionPosition, CaptionStyle, CaptionTrack};
use quillon_core::document::{Asset, Document};
use quillon_core::{
    RegularizeConfig, ms_to_ticks, parse_srt, parse_vtt, regularize_cues, to_srt, to_vtt,
};
use std::path::Path;

/// The style-related flags of `caption import`, bundled to keep the
/// function signature under the parameter budget.
pub struct ImportFlags {
    pub font: Option<String>,
    pub size: f32,
    pub margin: i64,
    pub position: String,
    pub bg: Option<String>,
}

/// Import a subtitle file as a new caption track on `doc`. Returns the
/// captioned document, or an operational error message (unreadable file,
/// unsupported format, no font asset).
pub fn import(
    subs: &Path,
    doc: &Document,
    flags: &ImportFlags,
    no_regularize: bool,
) -> Result<Document, String> {
    let content = std::fs::read_to_string(subs)
        .map_err(|e| format!("cannot read {}: {e}", subs.display()))?;
    let timebase = doc.timebase;
    let cues = match subs.extension().and_then(|e| e.to_str()) {
        Some("srt") => parse_srt(&content, timebase).map_err(|e| e.to_string())?,
        Some("vtt") => parse_vtt(&content, timebase).map_err(|e| e.to_string())?,
        Some("ass") | Some("ssa") => parse_ass(&content, timebase)?,
        other => {
            return Err(format!(
                "unsupported subtitle format {other:?}; expected .srt, .vtt, .ass or .ssa"
            ));
        }
    };
    finish(doc, flags, cues, no_regularize)
}

/// Transcribe an audio file with whisper and attach the result as a
/// caption track (plan §12 v0.5; only with the `transcribe` feature). The
/// model is downloaded on first use when `--model` is absent.
#[cfg(feature = "transcribe")]
pub fn transcribe(
    audio: &Path,
    doc: &Document,
    flags: &ImportFlags,
    model: Option<&Path>,
    threads: u32,
    no_regularize: bool,
) -> Result<Document, String> {
    let decoded = quillon_audio::transcribe::decode_mono_16k(audio).map_err(|e| e.to_string())?;
    let model_path = match model {
        Some(path) => path.to_path_buf(),
        None => quillon_audio::transcribe::default_model_path().ok_or_else(|| {
            "cannot derive a cache dir for the whisper model; pass --model".to_string()
        })?,
    };
    quillon_audio::transcribe::ensure_model(&model_path).map_err(|e| e.to_string())?;
    let cues =
        quillon_audio::transcribe::transcribe(&model_path, &decoded.samples, doc.timebase, threads)
            .map_err(|e| e.to_string())?;
    if cues.is_empty() {
        return Err("whisper produced no transcribable segments in this audio".into());
    }
    finish(doc, flags, cues, no_regularize)
}

/// The shared tail of every caption source: optional regularization, the
/// clamp into the document, style resolution, and track attachment.
fn finish(
    doc: &Document,
    flags: &ImportFlags,
    mut cues: Vec<CaptionCue>,
    no_regularize: bool,
) -> Result<Document, String> {
    if !no_regularize {
        cues = regularize_cues(&cues, &RegularizeConfig::default(), doc.timebase);
    }
    // Cue windows live inside the document: clamp the tail (SRT files
    // routinely trail the video), drop what no longer fits.
    let duration = doc.total_duration();
    cues.retain(|c| c.from < duration);
    for cue in &mut cues {
        cue.to = cue.to.min(duration);
    }

    let font = match &flags.font {
        Some(id) => id.clone(),
        None => doc
            .assets
            .iter()
            .find_map(|(id, a)| matches!(a, Asset::Font { .. }).then(|| id.clone()))
            .ok_or_else(|| {
                "the document has no font asset; pass --font with a font asset id".to_string()
            })?,
    };
    let position = parse_position(&flags.position)?;
    let bg = match &flags.bg {
        Some(hex) => Some(
            quillon_core::Color::parse(hex)
                .ok_or_else(|| format!("--bg must be #rrggbb, got {hex}"))?,
        ),
        None => None,
    };
    let mut doc = doc.clone();
    doc.captions.push(CaptionTrack {
        style: CaptionStyle {
            font,
            size: flags.size,
            color: quillon_core::Color::from_rgb8(255, 255, 255),
            align: quillon_core::TextAlign::Center,
            position,
            margin: flags.margin,
            shadow: None,
            bg,
        },
        cues,
    });
    Ok(doc)
}

/// Export one caption track as SRT or VTT.
pub fn export(doc: &Document, track: usize, format: &str) -> Result<String, String> {
    let track = doc.captions.get(track).ok_or_else(|| {
        format!(
            "the document has {} caption track(s); --track must index one",
            doc.captions.len()
        )
    })?;
    match format {
        "srt" => Ok(to_srt(&track.cues, doc.timebase)),
        "vtt" => Ok(to_vtt(&track.cues, doc.timebase)),
        other => Err(format!(
            "unsupported export format {other:?}; expected srt or vtt"
        )),
    }
}

fn parse_position(text: &str) -> Result<CaptionPosition, String> {
    match text {
        "bottom" => Ok(CaptionPosition::Bottom),
        "center" => Ok(CaptionPosition::Center),
        "top" => Ok(CaptionPosition::Top),
        other => Err(format!(
            "--position must be bottom, center or top, got {other:?}"
        )),
    }
}

/// Parse ASS/SSA through rsubs-lib: dialogue events only, override tags
/// stripped (`{\...}` removed, `\N` and `\n` become line breaks).
fn parse_ass(content: &str, timebase: i64) -> Result<Vec<CaptionCue>, String> {
    let ssa = rsubs_lib::SSA::parse_lenient(content).map_err(|e| e.to_string())?;
    let mut cues = Vec::new();
    for event in &ssa.events {
        if !matches!(event.line_type, rsubs_lib::SSAEventLineType::Dialogue) {
            continue;
        }
        let text = strip_ass_tags(&event.text);
        if text.is_empty() {
            continue;
        }
        // Same caps the SRT/VTT parsers enforce (r1 F2): untrusted input
        // is bounded before any timing work runs.
        if text.len() > quillon_core::MAX_CUE_TEXT_BYTES {
            return Err(format!(
                "an ASS cue text is {} bytes; the limit is {} bytes per cue",
                text.len(),
                quillon_core::MAX_CUE_TEXT_BYTES
            ));
        }
        if cues.len() >= quillon_core::MAX_PARSED_CUES {
            return Err(format!(
                "the file has more than {} cues; split it into several tracks or files",
                quillon_core::MAX_PARSED_CUES
            ));
        }
        let (sh, sm, ss, sms) = event.start.as_hms_milli();
        let (eh, em, es, ems) = event.end.as_hms_milli();
        let from_ms =
            ((i64::from(sh) * 60 + i64::from(sm)) * 60 + i64::from(ss)) * 1000 + i64::from(sms);
        let to_ms =
            ((i64::from(eh) * 60 + i64::from(em)) * 60 + i64::from(es)) * 1000 + i64::from(ems);
        if to_ms <= from_ms {
            continue;
        }
        cues.push(CaptionCue {
            from: ms_to_ticks(from_ms, timebase),
            to: ms_to_ticks(to_ms, timebase),
            text,
        });
    }
    Ok(cues)
}

/// Strip ASS override tags, keeping `\N` (and the soft `\n`) as breaks.
/// An unterminated `{` swallows the rest of the line — the lenient parser
/// accepts rough files, and half a tag is never displayable text.
fn strip_ass_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                for c2 in chars.by_ref() {
                    if c2 == '}' {
                        break;
                    }
                }
            }
            '\\' if matches!(chars.peek(), Some('N') | Some('n')) => {
                chars.next();
                out.push('\n');
            }
            _ => out.push(c),
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ass_tags_are_stripped_keep_bare_breaks() {
        // `{\...}` blocks vanish entirely (including a brace-wrapped \N);
        // only a bare \N outside braces becomes a line break.
        assert_eq!(
            strip_ass_tags("{\\i1}hello{\\i0} {\\N}world{\\pos(1,2)}"),
            "hello world"
        );
        assert_eq!(strip_ass_tags("hard\\Nbreak"), "hard\nbreak");
        assert_eq!(strip_ass_tags("soft\\nbreak"), "soft\nbreak");
        assert_eq!(strip_ass_tags("{unterminated"), "");
    }

    #[test]
    fn position_parser_rejects_garbage() {
        assert!(parse_position("bottom").is_ok());
        assert_eq!(
            parse_position("middle").unwrap_err(),
            "--position must be bottom, center or top, got \"middle\""
        );
    }
}
