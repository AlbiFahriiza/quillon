//! End-to-end v0.3 acceptance tests through the real CLI binary and the
//! real system ffmpeg/ffprobe (plan §12 v0.3). Media is generated on the
//! fly; nothing is a binary fixture.

use quillon_core::time::FLICKS_PER_SECOND as TIMEBASE;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("quillon-e2e-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("tempdir");
        Dir(dir)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn ffmpeg(args: &[&str]) {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .expect("ffmpeg");
    assert!(
        out.status.success(),
        "ffmpeg {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn quillon(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_quillon"))
        .args(args)
        .output()
        .expect("quillon binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// 3 s, 320x240, 10 fps, keyframe only at frame 0 (GOP 250), 440 Hz tone.
fn gop250(dir: &Dir, name: &str, video: &str, audio: &str) -> PathBuf {
    let path = dir.path(name);
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x240:rate=10:duration=3",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=3",
        "-c:v",
        video,
        "-g",
        "250",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        audio,
        "-shortest",
        &path.to_string_lossy(),
    ]);
    path
}

// ---------------------------------------------------------------------------
// Acceptance #1 through the CLI: probe reports duration/res/fps/codec for
// MP4 (H.264+AAC), WebM (VP9+Opus), MOV.
// ---------------------------------------------------------------------------

#[test]
fn cli_probe_reports_mp4_webm_mov() {
    let dir = Dir::new("probe");
    let cases = [
        ("clip.mp4", "libx264", "aac", "h264", "aac"),
        ("clip.mov", "libx264", "pcm_s16le", "h264", "pcm_s16le"),
    ];
    for (name, venc, aenc, vcodec, acodec) in cases {
        let path = gop250(&dir, name, venc, aenc);
        let (ok, stdout, stderr) = quillon(&["probe", "--json", &path.to_string_lossy()]);
        assert!(ok, "probe {name}: {stderr}");
        let body: Value = serde_json::from_str(&stdout).expect("json stdout");
        assert_eq!(body["video_codec"], vcodec, "{name}");
        assert_eq!(body["audio_codec"], acodec, "{name}");
        assert_eq!(body["width"], 320, "{name}");
        assert_eq!(body["height"], 240, "{name}");
        assert_eq!(body["fps"]["num"], 10, "{name}");
        assert_eq!(body["fps"]["den"], 1, "{name}");
        let duration = body["duration_seconds"].as_f64().expect("duration");
        assert!((duration - 3.0).abs() < 0.05, "{name}: {duration}");
    }

    // WebM needs libvpx-vp9 in this ffmpeg build; skip when absent.
    let encoders = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .expect("encoders");
    if String::from_utf8_lossy(&encoders.stdout)
        .to_lowercase()
        .contains("libvpx-vp9")
    {
        let path = gop250(&dir, "clip.webm", "libvpx-vp9", "libopus");
        let (ok, stdout, stderr) = quillon(&["probe", "--json", &path.to_string_lossy()]);
        assert!(ok, "probe webm: {stderr}");
        let body: Value = serde_json::from_str(&stdout).expect("json stdout");
        assert_eq!(body["video_codec"], "vp9");
        assert_eq!(body["audio_codec"], "opus");
        assert_eq!(body["width"], 320);
        assert_eq!(body["height"], 240);
    } else {
        eprintln!("skipping webm case: no libvpx-vp9 encoder");
    }
}

#[test]
fn cli_probe_rejects_non_media() {
    let dir = Dir::new("notmedia");
    let path = dir.path("plain.txt");
    std::fs::write(&path, b"not media").expect("write");
    let (ok, _, _) = quillon(&["probe", &path.to_string_lossy()]);
    assert!(!ok, "probe must fail on a non-media file");
}

// ---------------------------------------------------------------------------
// Acceptance #2 through the full render path: a document trims a GOP-250
// clip at a non-keyframe tick; the rendered frame equals the sequential
// decode reference.
// ---------------------------------------------------------------------------

#[test]
fn render_uses_trimmed_non_keyframe_frame() {
    let dir = Dir::new("render");
    let clip = gop250(&dir, "clip.mp4", "libx264", "aac");
    // Reference: frame 2 decoded by full sequential decode.
    let reference = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(&clip)
        .args([
            "-vf",
            "select=eq(n\\,2)",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .output()
        .expect("reference decode");
    assert_eq!(reference.stdout.len(), 320 * 240 * 4);

    let doc = format!(
        r#"{{
        "v": 1, "size": {{ "w": 320, "h": 240 }}, "fps": 10,
        "assets": {{ "clip": {{ "type": "video", "src": "clip.mp4" }} }},
        "scenes": [{{ "id": "s", "duration": {TIMEBASE},
          "elements": [{{ "type": "video", "asset": "clip", "x": 0, "y": 0,
            "w": 320, "h": 240, "fit": "contain",
            "source_range": {{ "from": {}, "to": {} }} }}] }}]
    }}"#,
        2 * TIMEBASE / 10,
        3 * TIMEBASE
    );
    let doc_path = dir.path("doc.json");
    std::fs::write(&doc_path, doc).expect("write doc");
    let out = dir.path("frames");
    let (ok, _, stderr) = quillon(&[
        "render",
        &doc_path.to_string_lossy(),
        "--format",
        "png",
        "-o",
        &out.to_string_lossy(),
    ]);
    assert!(ok, "render: {stderr}");

    let png = out.join("frame_000000.png");
    let img = image::open(&png).expect("rendered frame").to_rgba8();
    // 320x240 contain of a 320x240 source is an exact blit; the canvas is
    // the clip frame itself.
    assert_eq!(
        img.as_raw(),
        &reference.stdout[..],
        "frame must match the sequential-decode reference"
    );
}

// ---------------------------------------------------------------------------
// Acceptance #4 end to end: a document with audio renders to MP4 with an
// AAC track whose measured loudness is -23 +/- 0.5 LUFS.
// ---------------------------------------------------------------------------

#[test]
fn render_muxes_audio_normalized_to_lufs() {
    let dir = Dir::new("audio");
    // A loud 2-second tone as the audio asset.
    let wav = dir.path("tone.wav");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=2:sample_rate=48000",
        "-ac",
        "2",
        "-c:a",
        "pcm_s16le",
        &wav.to_string_lossy(),
    ]);
    let doc = r#"{
        "v": 1, "size": { "w": 320, "h": 240 }, "fps": 10,
        "assets": { "tone": { "type": "audio", "src": "tone.wav" } },
        "audio": [ { "asset": "tone", "gain": 0.9 } ],
        "scenes": [ { "id": "s", "duration": 1411200000, "elements": [] } ]
    }"#;
    let doc_path = dir.path("doc.json");
    std::fs::write(&doc_path, doc).expect("write doc");
    let out = dir.path("out.mp4");
    let (ok, _, stderr) = quillon(&[
        "render",
        &doc_path.to_string_lossy(),
        "-o",
        &out.to_string_lossy(),
        "--lufs=-23",
    ]);
    assert!(ok, "render: {stderr}");

    // The MP4 must carry both streams.
    let streams = Command::new("ffprobe")
        .args(["-v", "quiet", "-print_format", "json", "-show_streams"])
        .arg(&out)
        .output()
        .expect("ffprobe");
    let body: Value = serde_json::from_slice(&streams.stdout).expect("ffprobe json");
    let codecs: Vec<&str> = body["streams"]
        .as_array()
        .expect("streams")
        .iter()
        .map(|s| s["codec_name"].as_str().expect("codec"))
        .collect();
    assert!(codecs.contains(&"h264"), "video stream: {codecs:?}");
    assert!(codecs.contains(&"aac"), "audio stream: {codecs:?}");

    // Decode the audio back and measure integrated loudness.
    let raw = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(&out)
        .args([
            "-map", "0:a", "-f", "s16le", "-ar", "48000", "-ac", "2", "-",
        ])
        .output()
        .expect("audio decode");
    let samples: Vec<f32> = raw
        .stdout
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect();
    let lufs = quillon_audio::measure_lufs(&samples, 48_000).expect("measure");
    assert!(
        (lufs - (-23.0)).abs() <= 0.5,
        "expected -23 +/- 0.5 LUFS, measured {lufs}"
    );
}

// ---------------------------------------------------------------------------
// inspect: summary of a validated document.
// ---------------------------------------------------------------------------

#[test]
fn inspect_summarizes_documents() {
    let dir = Dir::new("inspect");
    let doc_path = dir.path("doc.json");
    std::fs::write(
        &doc_path,
        r#"{
        "v": 1, "size": { "w": 320, "h": 240 }, "fps": 10,
        "assets": { "f": { "type": "font", "src": "font.ttf" } },
        "scenes": [ { "id": "s1", "duration": 705600000,
          "elements": [ { "type": "text", "text": "x", "font": "f",
            "x": 10, "y": 10, "size": 12 } ] } ]
    }"#,
    )
    .expect("write doc");
    // The font asset must exist for validation.
    std::fs::write(
        dir.path("font.ttf"),
        std::include_bytes!("fixtures/assets/font.ttf"),
    )
    .expect("font fixture");
    let (ok, stdout, stderr) = quillon(&["inspect", "--json", &doc_path.to_string_lossy()]);
    assert!(ok, "inspect: {stderr}");
    let body: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(body["frames"], 10);
    assert_eq!(body["duration_ticks"], TIMEBASE);
    assert_eq!(body["scenes"][0]["id"], "s1");
    assert_eq!(body["assets"][0]["type"], "font");
    assert_eq!(body["audio"].as_array().map(Vec::len), Some(0));
}
