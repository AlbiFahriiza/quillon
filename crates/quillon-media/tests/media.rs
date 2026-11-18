//! Integration tests for the media adapters (plan §12 v0.3 acceptance #1
//! and #2). All media is generated on the fly by the system ffmpeg into a
//! tempdir — no binary fixtures in the repo, and the same ffmpeg that will
//! run in CI produces the inputs.

use quillon_core::frame_source::FrameSource;
use quillon_core::time::FLICKS_PER_SECOND as TIMEBASE;
use quillon_media::decoder::SidecarDecoder;
use quillon_media::probe;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

struct MediaDir(PathBuf);

impl MediaDir {
    fn new(tag: &str) -> MediaDir {
        let dir =
            std::env::temp_dir().join(format!("quillon-media-test-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tempdir");
        MediaDir(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for MediaDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Run ffmpeg to generate a fixture; panics with the captured output on
/// failure so the test names the real cause.
fn run_ffmpeg(args: &[&str]) {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .expect("spawn ffmpeg (is it on PATH?)");
    assert!(
        out.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn have_encoder(encoder: &str) -> bool {
    static CHECKS: OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    let cache = CHECKS.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()));
    let mut set = cache.lock().unwrap();
    if set.contains(encoder) {
        return true;
    }
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .expect("ffmpeg -encoders");
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    let found = text.contains(encoder);
    if found {
        set.insert(encoder.to_string());
    }
    found
}

/// 3 s of 320x240 @ 10 fps test pattern with a 440 Hz tone, keyframes only
/// at frame 0 (GOP 250 > clip length): every later frame is a non-keyframe.
fn gop250_clip(dir: &MediaDir, name: &str, video: &[&str], audio: &[&str]) -> PathBuf {
    let path = dir.path(name);
    let mut args: Vec<&str> = vec![
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x240:rate=10:duration=3",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=3",
        "-c:v",
    ];
    args.extend_from_slice(video);
    args.extend_from_slice(&["-g", "250", "-pix_fmt", "yuv420p", "-c:a"]);
    args.extend_from_slice(audio);
    args.extend_from_slice(&["-shortest"]);
    let path_str = path.to_string_lossy().into_owned();
    args.push(&path_str);
    run_ffmpeg(&args);
    path
}

// ---------------------------------------------------------------------------
// Acceptance #1: probe reports duration/resolution/fps/codec for MP4
// (H.264+AAC), WebM (VP9+Opus), and MOV.
// ---------------------------------------------------------------------------

#[test]
fn probe_reports_mp4_h264_aac() {
    let dir = MediaDir::new("mp4");
    let path = gop250_clip(&dir, "clip.mp4", &["libx264"], &["aac"]);
    let info = probe::probe(&path).expect("probe mp4");
    assert_eq!(info.video_codec.as_deref(), Some("h264"));
    assert_eq!(info.audio_codec.as_deref(), Some("aac"));
    assert_eq!((info.width, info.height), (Some(320), Some(240)));
    assert_eq!(info.fps, Some((10, 1)));
    let ticks = info.duration_ticks(TIMEBASE).expect("duration");
    let secs = ticks as f64 / TIMEBASE as f64;
    assert!((secs - 3.0).abs() < 0.05, "duration {secs}");
}

#[test]
fn probe_reports_webm_vp9_opus() {
    if !have_encoder("libvpx-vp9") {
        eprintln!("skipping: this ffmpeg build has no libvpx-vp9 encoder");
        return;
    }
    let dir = MediaDir::new("webm");
    let path = gop250_clip(&dir, "clip.webm", &["libvpx-vp9"], &["libopus"]);
    let info = probe::probe(&path).expect("probe webm");
    assert_eq!(info.video_codec.as_deref(), Some("vp9"));
    assert_eq!(info.audio_codec.as_deref(), Some("opus"));
    assert_eq!((info.width, info.height), (Some(320), Some(240)));
}

#[test]
fn probe_reports_mov() {
    let dir = MediaDir::new("mov");
    let path = gop250_clip(&dir, "clip.mov", &["libx264"], &["pcm_s16le"]);
    let info = probe::probe(&path).expect("probe mov");
    assert_eq!(info.video_codec.as_deref(), Some("h264"));
    assert_eq!(info.audio_codec.as_deref(), Some("pcm_s16le"));
    assert_eq!((info.width, info.height), (Some(320), Some(240)));
}

#[test]
fn probe_rejects_non_media_files() {
    let dir = MediaDir::new("notmedia");
    let path = dir.path("plain.txt");
    std::fs::write(&path, b"definitely not a media container").expect("write");
    assert!(probe::probe(&path).is_err());
}

// ---------------------------------------------------------------------------
// Acceptance #2: a trim at a non-keyframe tick decodes the right frame
// (decode-forward from the earlier keyframe; the clip's only keyframe is
// frame 0 because GOP 250 exceeds the clip).
// ---------------------------------------------------------------------------

/// Decode one reference frame by full sequential decode: `select=eq(n,N)`.
fn reference_frame(path: &Path, index: u32) -> Vec<u8> {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error"])
        .arg("-i")
        .arg(path)
        .arg("-vf")
        .arg(format!("select=eq(n\\,{index})"))
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output()
        .expect("reference decode");
    assert!(
        out.status.success(),
        "reference decode failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout.len(), 320 * 240 * 4, "one full RGBA frame");
    out.stdout
}

#[test]
fn non_keyframe_trim_decodes_the_exact_frame() {
    let dir = MediaDir::new("gop");
    let path = gop250_clip(&dir, "gop250.mp4", &["libx264"], &["aac"]);

    let decoder = SidecarDecoder::open(&path, TIMEBASE).expect("open");
    // Frame 17 sits between the keyframe at 0 and the clip end: every seek
    // to it must decode forward through frames 1..17.
    let tick = 17 * TIMEBASE / 10;
    let frame = decoder.frame_at(tick).expect("decode frame 17");

    assert_eq!((frame.width, frame.height), (320, 240));
    let reference = reference_frame(&path, 17);
    assert_eq!(
        frame.rgba, reference,
        "decoded frame must be byte-identical to a full sequential decode"
    );

    // A second, later non-keyframe from the same clip.
    let tick25 = 25 * TIMEBASE / 10;
    let frame25 = decoder.frame_at(tick25).expect("decode frame 25");
    assert_eq!(frame25.rgba, reference_frame(&path, 25));
}

#[test]
fn decoder_cache_avoids_respawning_per_frame() {
    let dir = MediaDir::new("cache");
    let path = gop250_clip(&dir, "gop250.mp4", &["libx264"], &["aac"]);
    let decoder = SidecarDecoder::open(&path, TIMEBASE).expect("open");

    // Sequential frames 0..9 — the pipeline's access pattern.
    for n in 0..10i64 {
        decoder.frame_at(n * TIMEBASE / 10).expect("decode");
    }
    assert_eq!(
        decoder.spawns(),
        1,
        "sequential frames within one chunk must spawn exactly one ffmpeg"
    );

    // The same frame again: pure cache hit, no new process.
    decoder.frame_at(5 * TIMEBASE / 10).expect("cached");
    assert_eq!(decoder.spawns(), 1);
}

#[test]
fn ticks_past_the_end_hold_the_last_frame() {
    let dir = MediaDir::new("hold");
    let path = gop250_clip(&dir, "gop250.mp4", &["libx264"], &["aac"]);
    let decoder = SidecarDecoder::open(&path, TIMEBASE).expect("open");

    let last = decoder.frame_at(10 * TIMEBASE).expect("past the end");
    let reference = reference_frame(&path, 29); // 3 s at 10 fps: frames 0..=29
    assert_eq!(last.rgba, reference);
}
