//! OpenTimelineIO subset import (plan §4 F13, §12 v0.6; research/D §5:
//! no mature Rust binding exists, so we parse the JSON subset ourselves —
//! declared "best effort, subset"). Flat mapping: Timeline → document,
//! first Video-kind Track → scenes (one per clip), Audio-kind tracks →
//! audio tracks. RationalTime `value/rate` becomes exact ticks.

use crate::builder::DocumentBuilder;
use crate::document::{Asset, Document, Size, VideoElement};
use serde_json::Value;
/// FPS values the flicks timebase divides exactly (the validator's rule);
/// an OTIO rate outside this set is a named error suggesting the closest.
pub const LEGAL_FPS: &[f64] = &[12.0, 24.0, 25.0, 30.0, 50.0, 60.0];

/// An OTIO import failure: operational input (bad OTIO file), surfaced by
/// the CLI with exit 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtioError {
    pub message: String,
}

impl std::fmt::Display for OtioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for OtioError {}

fn err(message: impl Into<String>) -> OtioError {
    OtioError {
        message: message.into(),
    }
}

fn schema_of(value: &Value) -> &str {
    value["OTIO_SCHEMA"].as_str().unwrap_or("")
}

fn rational_to_ticks(time: &Value, what: &str, fps: f64, timebase: i64) -> Result<i64, OtioError> {
    let rate = time["rate"].as_f64().unwrap_or(fps);
    if rate <= 0.0 {
        return Err(err(format!("{what} has a non-positive rate {rate}")));
    }
    let value = time["value"].as_f64().unwrap_or(0.0);
    let ticks = value * timebase as f64 / rate;
    // Hardening D4: a float cast out of range saturates silently
    // (infinity becomes i64::MAX), so an absurd or infinite value must
    // be a named error instead of a clamped timeline position.
    if !ticks.is_finite() || ticks >= i64::MAX as f64 || ticks <= i64::MIN as f64 {
        return Err(err(format!(
            "{what} has a value ({value}) that overflows the timeline at rate {rate}"
        )));
    }
    Ok(ticks.round() as i64)
}

/// The fps the document will use: the first clip rate that maps to a legal
/// Quillon fps. Rates that do not divide the timebase are rejected with
/// the nearest legal suggestion.
fn pick_fps(tracks: &[Value]) -> Result<u32, OtioError> {
    for track in tracks {
        if track["kind"].as_str() != Some("Video") {
            continue;
        }
        for clip in track["children"].as_array().unwrap_or(&vec![]) {
            let Some(rate) = clip["source_range"]["duration"]["rate"].as_f64() else {
                continue;
            };
            for &legal in LEGAL_FPS {
                if (rate - legal).abs() < 0.5 {
                    return Ok(legal as u32);
                }
            }
            let nearest = LEGAL_FPS
                .iter()
                .min_by(|a, b| {
                    (**a - rate)
                        .abs()
                        .partial_cmp(&(**b - rate).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied()
                .unwrap_or(30.0);
            return Err(err(format!(
                "clip rate {rate} does not divide the flicks timebase exactly; \
                 the nearest legal fps is {nearest} — conform the OTIO to {nearest} fps"
            )));
        }
    }
    Ok(30)
}

/// Import an OTIO subset timeline into a Quillon document. `media_dir`
/// resolves relative `target_url`s (already absolute paths and `file://`
/// URLs are used as-is after stripping the scheme).
pub fn import_otio(input: &str, media_dir: Option<&str>) -> Result<Document, OtioError> {
    let value: Value =
        serde_json::from_str(input).map_err(|e| err(format!("not valid JSON: {e}")))?;
    if schema_of(&value) != "Timeline.1" {
        return Err(err(format!(
            "expected an OTIO Timeline.1 at the root, found {:?}",
            schema_of(&value)
        )));
    }
    let tracks: Vec<Value> = value["tracks"]["children"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if tracks.is_empty() {
        return Err(err("the timeline has no tracks"));
    }
    let video_tracks: Vec<&Value> = tracks
        .iter()
        .filter(|t| schema_of(t) == "Track.1" && t["kind"].as_str() == Some("Video"))
        .collect();
    if video_tracks.is_empty() {
        return Err(err("the timeline has no Video-kind track"));
    }

    let timebase = crate::time::FLICKS_PER_SECOND;
    let fps = pick_fps(&tracks)?;
    let mut builder = DocumentBuilder::new(Size { w: 1920, h: 1080 }).fps(fps);

    // First video track wins; extras are a warning, not an error.
    let mut asset_seq = 0usize;
    for (ti, track) in video_tracks.iter().enumerate() {
        if ti > 0 {
            // Extra video tracks are reported by subset_warnings instead.
            continue;
        }
        for clip in track["children"].as_array().unwrap_or(&vec![]) {
            if schema_of(clip) != "Clip.1" {
                continue; // Gap.1 / nested Stack.1 are outside the subset
            }
            let name = clip["name"].as_str().unwrap_or("clip");
            let url = clip["media_reference"]["target_url"]
                .as_str()
                .ok_or_else(|| err(format!("clip \"{name}\" has no media_reference target_url")))?;
            let src = resolve_media(url, media_dir)?;
            let asset_id = format!("otio{asset_seq}");
            asset_seq += 1;
            builder = builder.asset(&asset_id, Asset::Video { src, sha256: None });
            let source_range = clip["source_range"]
                .as_object()
                .ok_or_else(|| err(format!("clip \"{name}\" has no source_range")))?;
            let from = rational_to_ticks(
                &source_range["start_time"],
                &format!("clip \"{name}\" start"),
                fps as f64,
                timebase,
            )?;
            let duration = rational_to_ticks(
                &source_range["duration"],
                &format!("clip \"{name}\" duration"),
                fps as f64,
                timebase,
            )?;
            if duration <= 0 {
                return Err(err(format!("clip \"{name}\" has a non-positive duration")));
            }
            builder = builder.scene(crate::document::Scene {
                id: name.to_string(),
                duration,
                transition_in: None,
                elements: vec![crate::document::Element::Video(VideoElement {
                    asset: asset_id,
                    x: 0,
                    y: 0,
                    w: 1920,
                    h: 1080,
                    fit: crate::document::Fit::Contain,
                    source_range: Some(crate::document::SourceRange {
                        from,
                        to: from + duration,
                    }),
                    speed: 1.0,
                    shadow: None,
                    clip: None,
                    anim: None,
                })],
            });
        }
    }

    // Audio tracks map to the mix, in track order.
    for track in &tracks {
        if schema_of(track) != "Track.1" || track["kind"].as_str() != Some("Audio") {
            continue;
        }
        for clip in track["children"].as_array().unwrap_or(&vec![]) {
            if schema_of(clip) != "Clip.1" {
                continue;
            }
            let name = clip["name"].as_str().unwrap_or("audio");
            let Some(url) = clip["media_reference"]["target_url"].as_str() else {
                return Err(err(format!("audio clip \"{name}\" has no target_url")));
            };
            let src = resolve_media(url, media_dir)?;
            let asset_id = format!("otio{asset_seq}");
            asset_seq += 1;
            builder = builder.asset(&asset_id, Asset::Audio { src, sha256: None });
            let from = rational_to_ticks(
                &clip["source_range"]["start_time"],
                &format!("audio clip \"{name}\" start"),
                fps as f64,
                timebase,
            )
            .unwrap_or(0);
            let duration = rational_to_ticks(
                &clip["source_range"]["duration"],
                &format!("audio clip \"{name}\" duration"),
                fps as f64,
                timebase,
            )
            .unwrap_or(0);
            builder = builder.audio_track(crate::document::AudioTrack {
                asset: asset_id,
                gain: 1.0,
                start_at: 0,
                source_range: Some(crate::document::SourceRange {
                    from,
                    to: from + duration.max(1),
                }),
                fades: Vec::new(),
            });
        }
    }

    let doc = builder.build();
    Ok(doc)
}

/// Media URLs become asset `src` values, which the validator requires to
/// be RELATIVE and confined to the document's directory (plan §11). A
/// relative target_url is used as-is (the validator resolves it against
/// the document's base); an absolute path must live inside `media_dir` and
/// is rebased relative to it.
fn resolve_media(url: &str, media_dir: Option<&str>) -> Result<String, OtioError> {
    let path = url.strip_prefix("file://").unwrap_or(url);
    if !path.starts_with('/') {
        return Ok(path.to_string());
    }
    let Some(dir) = media_dir else {
        return Err(err(format!(
            "media {path} is an absolute path; pass the media directory so it can be rebased"
        )));
    };
    let dir = dir.trim_end_matches('/');
    if let Ok(rel) = std::path::Path::new(path).strip_prefix(dir) {
        let rel = rel.to_string_lossy().replace('\\', "/");
        if rel.is_empty() {
            return Err(err(format!("media {path} is the media directory itself")));
        }
        return Ok(rel);
    }
    Err(err(format!(
        "media {path} is outside the media directory {dir}; Quillon keeps assets inside the document directory"
    )))
}

/// The flat-subset warnings the CLI prints after a successful import
/// (extra video tracks are ignored rather than failed).
pub fn subset_warnings(input: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(input) else {
        return Vec::new();
    };
    let tracks: Vec<Value> = value["tracks"]["children"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let video: Vec<usize> = tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| schema_of(t) == "Track.1" && t["kind"].as_str() == Some("Video"))
        .map(|(i, _)| i)
        .collect();
    let mut warnings = Vec::new();
    for i in video.iter().skip(1) {
        warnings.push(format!(
            "video track {} is ignored: the flat v0.6 subset imports the first Video track only",
            i + 1
        ));
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMELINE: &str = r##"{
        "OTIO_SCHEMA": "Timeline.1",
        "name": "demo",
        "tracks": {
            "OTIO_SCHEMA": "Stack.1",
            "children": [
                {
                    "OTIO_SCHEMA": "Track.1", "kind": "Video",
                    "children": [
                        {
                            "OTIO_SCHEMA": "Clip.1", "name": "intro",
                            "media_reference": { "OTIO_SCHEMA": "ExternalReference.1",
                                "target_url": "file://clips/intro.mp4" },
                            "source_range": { "OTIO_SCHEMA": "TimeRange.1",
                                "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": 30.0, "value": 30.0 },
                                "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": 30.0, "value": 60.0 } }
                        },
                        {
                            "OTIO_SCHEMA": "Clip.1", "name": "outro",
                            "media_reference": { "OTIO_SCHEMA": "ExternalReference.1",
                                "target_url": "clips/outro.mp4" },
                            "source_range": { "OTIO_SCHEMA": "TimeRange.1",
                                "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": 30.0, "value": 0.0 },
                                "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": 30.0, "value": 30.0 } }
                        }
                    ]
                }
            ]
        }
    }"##;

    #[test]
    fn absurd_durations_are_named_errors_not_saturated_casts() {
        // Hardening D4: a value beyond i64 range used to saturate the
        // float cast silently (i64::MAX), pinning the scene at a
        // nonsense tick instead of failing.
        let evil = TIMELINE.replace("\"value\": 60.0", "\"value\": 1e300");
        let error = import_otio(&evil, None).unwrap_err();
        assert!(
            error.message.contains("overflows the timeline"),
            "{}",
            error.message
        );
    }

    #[test]
    fn imports_clips_with_exact_source_ranges() {
        let doc = import_otio(TIMELINE, Some("media")).unwrap();
        assert_eq!(doc.fps, 30);
        assert_eq!(doc.scenes.len(), 2);
        assert_eq!(doc.scenes[0].id, "intro");
        let s = 705_600_000i64;
        assert_eq!(doc.scenes[0].duration, 2 * s, "60 frames @30fps = 2s");
        let video = match &doc.scenes[0].elements[0] {
            crate::document::Element::Video(v) => v,
            other => panic!("expected a video element, got {other:?}"),
        };
        assert_eq!(
            video.source_range,
            Some(crate::document::SourceRange { from: s, to: 3 * s }),
            "start 30 @30fps = 1s, duration 2s"
        );
        // Relative URLs stay relative: the validator resolves them against
        // the document's directory (path confinement, plan §11).
        assert_eq!(
            doc.assets[&video.asset],
            Asset::Video {
                src: "clips/intro.mp4".into(),
                sha256: None
            }
        );
        let outro = match &doc.scenes[1].elements[0] {
            crate::document::Element::Video(v) => v,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            doc.assets[&outro.asset],
            Asset::Video {
                src: "clips/outro.mp4".into(),
                sha256: None
            }
        );
    }

    #[test]
    fn non_timeline_roots_are_named_errors() {
        let error = import_otio(r#"{"OTIO_SCHEMA": "Stack.1"}"#, None).unwrap_err();
        assert!(error.message.contains("Timeline.1"), "{}", error.message);
        // A JSON array is valid JSON but not a Timeline: named error too.
        let error = import_otio("[]", None).unwrap_err();
        assert!(error.message.contains("Timeline.1"), "{}", error.message);
    }

    #[test]
    fn missing_video_track_is_rejected() {
        let audio_only = TIMELINE.replace("\"Video\"", "\"Audio\"");
        let error = import_otio(&audio_only, None).unwrap_err();
        assert!(error.message.contains("Video-kind"), "{}", error.message);
    }

    #[test]
    fn ntsc_rates_snap_to_their_legal_fps() {
        // 29.97 is within half a frame of 30: the document uses fps 30 and
        // ticks stay exact per the clip's own rate.
        let ntsc = TIMELINE.replace("\"rate\": 30.0", "\"rate\": 29.97");
        let doc = import_otio(&ntsc, None).expect("29.97 snaps to 30");
        assert_eq!(doc.fps, 30);
    }

    #[test]
    fn illegal_rates_suggest_the_nearest_legal_fps() {
        let odd = TIMELINE.replace("\"rate\": 30.0", "\"rate\": 17.0");
        let error = import_otio(&odd, None).unwrap_err();
        assert!(
            error.message.contains("nearest legal fps is 12"),
            "must suggest the nearest legal fps: {}",
            error.message
        );
    }

    #[test]
    fn extra_video_tracks_warn() {
        let two = TIMELINE.replace(
            "\"OTIO_SCHEMA\": \"Track.1\", \"kind\": \"Video\"",
            "\"OTIO_SCHEMA\": \"Track.1\", \"kind\": \"Video\", \"name\": \"first\"",
        );
        // Append a second (empty) video track after the first.
        let two = two.replace(
            "                }
            ]
        }
    }",
            "                },
                { \"OTIO_SCHEMA\": \"Track.1\", \"kind\": \"Video\", \"children\": [] }
            ]
        }
    }",
        );
        assert_eq!(subset_warnings(&two).len(), 1, "{two}");
        assert_eq!(subset_warnings(TIMELINE).len(), 0);
    }
}
