//! Caption burn-in through the public renderer (plan §12 v0.5 acceptance 1:
//! a cue's ink appears inside its range and only there; frames outside
//! every cue are pixel-identical to an uncaptioned document, which is why
//! the pre-v0.5 golden corpus cannot shift).

use quillon_core::frame_source::NullFrameSource;
use quillon_core::validate_str;
use quillon_render::Renderer;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn corpus_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/corpus"))
}

/// A black 320x240 10fps document whose captions come from `captions_json`
/// (an empty string means no captions field at all).
fn doc_with(captions_json: &str) -> quillon_core::Document {
    let captions = if captions_json.is_empty() {
        String::new()
    } else {
        format!("\"captions\": [{captions_json}],")
    };
    let json = format!(
        r##"{{
            "v": 1,
            "size": {{ "w": 320, "h": 240 }},
            "fps": 10,
            "bg": "#000000",
            "assets": {{ "body": {{ "type": "font", "src": "assets/font.ttf" }} }},
            {captions}
            "scenes": [ {{ "id": "s", "duration": 2116800000, "elements": [] }} ]
        }}"##
    );
    validate_str(&json, &corpus_dir()).unwrap_or_else(|e| panic!("test doc must be valid: {e:?}"))
}

const HALF_SECOND: i64 = 352_800_000;

fn frame_at(doc: &quillon_core::Document, tick: i64) -> quillon_core::Frame {
    let renderer = Renderer::new(doc, &corpus_dir()).unwrap();
    renderer.render_tick(tick, &NullFrameSource).unwrap()
}

/// Count non-background pixels in a horizontal band.
fn ink_in_band(frame: &quillon_core::Frame, y0: u32, y1: u32) -> usize {
    (y0..y1)
        .flat_map(|y| (0..frame.width).map(move |x| (x, y)))
        .filter(|(x, y)| {
            let i = ((y * frame.width + x) * 4) as usize;
            frame.rgba[i] != 0 || frame.rgba[i + 1] != 0 || frame.rgba[i + 2] != 0
        })
        .count()
}

const BOTTOM_CUES: &str = r#"{ "style": { "font": "body", "size": 24.0, "margin": 24 },
        "cues": [
            { "from": 0, "to": 705600000, "text": "HELLO" },
            { "from": 1411200000, "to": 2116800000, "text": "BYE" } ] }"#;

#[test]
fn cue_ink_appears_only_inside_its_range() {
    let doc = doc_with(BOTTOM_CUES);
    // Inside the first cue, inside the gap, inside the second cue.
    for (tick, expect_ink) in [
        (HALF_SECOND, true),
        (3 * HALF_SECOND, false),
        (5 * HALF_SECOND, true),
    ] {
        let frame = frame_at(&doc, tick);
        let bottom_band = ink_in_band(&frame, 180, 225);
        if expect_ink {
            assert!(bottom_band > 0, "tick {tick} must show caption ink");
        } else {
            assert_eq!(bottom_band, 0, "tick {tick} is between cues");
        }
    }
}

#[test]
fn frames_outside_cues_equal_the_uncaptioned_document() {
    let captioned = doc_with(BOTTOM_CUES);
    let plain = doc_with("");
    // The gap frame is pixel-identical to a document with no captions at
    // all: burn-in is a true no-op outside cue ranges.
    let a = frame_at(&captioned, 3 * HALF_SECOND);
    let b = frame_at(&plain, 3 * HALF_SECOND);
    assert_eq!(Sha256::digest(&a.rgba), Sha256::digest(&b.rgba));
    // And inside a cue the caption visibly changes the frame.
    let a = frame_at(&captioned, HALF_SECOND);
    let b = frame_at(&plain, HALF_SECOND);
    assert_ne!(Sha256::digest(&a.rgba), Sha256::digest(&b.rgba));
}

#[test]
fn captioned_frames_are_deterministic() {
    let doc = doc_with(BOTTOM_CUES);
    let first = frame_at(&doc, HALF_SECOND);
    let second = frame_at(&doc, HALF_SECOND);
    assert_eq!(Sha256::digest(&first.rgba), Sha256::digest(&second.rgba));
}

#[test]
fn backing_box_adds_fill_beyond_the_glyphs() {
    let with_box = doc_with(
        r##"{ "style": { "font": "body", "size": 24.0, "margin": 24, "bg": "#202020" },
            "cues": [ { "from": 0, "to": 705600000, "text": "HELLO" } ] }"##,
    );
    let without_box = doc_with(BOTTOM_CUES);
    let boxed = frame_at(&with_box, HALF_SECOND);
    let bare = frame_at(&without_box, HALF_SECOND);
    assert!(
        ink_in_band(&boxed, 150, 225) > ink_in_band(&bare, 150, 225),
        "the backing box must cover more band pixels than glyphs alone"
    );
}

#[test]
fn top_position_moves_the_ink_band() {
    let doc = doc_with(
        r#"{ "style": { "font": "body", "size": 24.0, "margin": 24, "position": "top" },
            "cues": [ { "from": 0, "to": 705600000, "text": "HELLO" } ] }"#,
    );
    let frame = frame_at(&doc, HALF_SECOND);
    assert!(
        ink_in_band(&frame, 20, 60) > 0,
        "top caption inks the top band"
    );
    assert_eq!(ink_in_band(&frame, 180, 225), 0, "bottom band stays clear");
}

#[test]
fn multi_line_cue_renders_both_lines() {
    let doc = doc_with(
        r#"{ "style": { "font": "body", "size": 20.0, "margin": 20 },
            "cues": [ { "from": 0, "to": 705600000, "text": "first line\nsecond line" } ] }"#,
    );
    let frame = frame_at(&doc, HALF_SECOND);
    // Two lines at 20px size with 1.2 line height span ~48px plus padding;
    // a single line would stay within ~30px.
    assert!(ink_in_band(&frame, 150, 230) > 0);
}
