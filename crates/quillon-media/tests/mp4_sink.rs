//! End-to-end MP4 encode: document -> render_tick frames -> ffmpeg -> MP4.
//! Requires ffmpeg on PATH (CI installs it; the runner here has 7.1.5).

use quillon_core::frame_source::NullFrameSource;
use quillon_core::time::frame_tick;
use quillon_core::{Document, validate_str};
use quillon_media::Mp4Sink;
use quillon_render::Renderer;
use std::path::PathBuf;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

const DOC: &str = r##"{
    "v": 1,
    "size": { "w": 320, "h": 240 },
    "fps": 10,
    "bg": "#182028",
    "assets": { "body": { "type": "font", "src": "assets/font.ttf" } },
    "scenes": [
        {
            "id": "s1",
            "duration": 352800000,
            "elements": [
                { "type": "rect", "x": 10, "y": 10, "w": 60, "h": 40, "color": "#cc4422",
                  "anim": { "opacity": { "keys": [ { "t": 0, "v": 0.2 }, { "t": 352800000, "v": 1.0 } ] } } },
                { "type": "text", "text": "encode", "font": "body", "x": 160, "y": 120, "size": 24.0 }
            ]
        }
    ]
}"##;

fn load_doc() -> Document {
    validate_str(DOC, PathBuf::from(FIXTURES).as_path())
        .unwrap_or_else(|errors| panic!("fixture doc must be valid: {errors:?}"))
}

#[test]
fn renders_document_to_a_playable_mp4() {
    if !ffmpeg_available() {
        eprintln!("SKIPPED: ffmpeg not on PATH");
        return;
    }
    let doc = load_doc();
    let renderer = Renderer::new(&doc, PathBuf::from(FIXTURES).as_path()).unwrap();
    let out = std::env::temp_dir().join(format!("quillon-media-test-{}.mp4", std::process::id()));
    let mut sink = Mp4Sink::new(&out, doc.size.w, doc.size.h, doc.fps, "libx264").unwrap();

    for i in 0..doc.frame_count() {
        let tick = (i as i64) * frame_tick(doc.fps);
        let frame = renderer.render_tick(tick, &NullFrameSource).unwrap();
        sink.write_frame(&frame).unwrap();
    }
    sink.finish().unwrap();

    let bytes = std::fs::read(&out).unwrap();
    assert!(
        bytes.len() > 1024,
        "MP4 suspiciously small: {} bytes",
        bytes.len()
    );
    // MP4 files start with a size then "ftyp".
    assert_eq!(&bytes[4..8], b"ftyp", "not an MP4 container");
    std::fs::remove_file(&out).ok();
}

fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}
