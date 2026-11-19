//! CLI contract tests for `quillon caption import|export` and
//! `quillon audio analyze` (plan §12 v0.5): exit codes, machine-readable
//! output, and the SRT round trip keeping timing, through the real binary.

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_quillon");
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn run(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(BIN)
        .args(args)
        .output()
        .expect("spawn quillon");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn fixture(name: &str) -> String {
    format!("{FIXTURES}/{name}")
}

/// Scratch outputs go next to the fixtures so relative asset paths resolve.
fn scratch(name: &str) -> PathBuf {
    let p = PathBuf::from(format!("{FIXTURES}/{name}"));
    let _ = std::fs::remove_file(&p);
    p
}

#[test]
fn srt_import_appends_a_valid_caption_track() {
    let out = scratch("imported.json");
    let (code, stdout, stderr) = run(&[
        "caption",
        "import",
        &fixture("subs.srt"),
        "--doc",
        &fixture("caption-target.json"),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");
    let (code, stdout, stderr) = run(&["validate", out.to_str().unwrap()]);
    assert_eq!(code, 0, "imported doc must validate: {stdout} {stderr}");
    let body = std::fs::read_to_string(&out).unwrap();
    assert!(body.contains("\"captions\""), "track attached: {body}");
    assert_eq!(body.matches("\"from\"").count(), 2, "two cues: {body}");
}

#[test]
fn srt_round_trip_keeps_timing_exactly() {
    // The fixture's cues already satisfy the regularization bounds, so the
    // default pipeline is a no-op and the exported SRT keeps every
    // timestamp byte-for-byte (acceptance v0.5 #3).
    let out = scratch("roundtrip.json");
    let (code, _, stderr) = run(&[
        "caption",
        "import",
        &fixture("subs.srt"),
        "--doc",
        &fixture("caption-target.json"),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stderr}");
    let (code, stdout, stderr) = run(&["caption", "export", out.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    let original = std::fs::read_to_string(fixture("subs.srt")).unwrap();
    let original_timing: Vec<&str> = original.lines().filter(|l| l.contains("-->")).collect();
    let exported_timing: Vec<&str> = stdout.lines().filter(|l| l.contains("-->")).collect();
    assert_eq!(
        original_timing, exported_timing,
        "round trip must keep cue timing exactly"
    );
    assert!(stdout.contains("Hi") && stdout.contains("Bye"));
}

#[test]
fn vtt_import_flows_through_the_same_pipeline() {
    let out = scratch("vtt-imported.json");
    let (code, _, stderr) = run(&[
        "caption",
        "import",
        &fixture("subs.vtt"),
        "--doc",
        &fixture("caption-target.json"),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stderr}");
    let (code, stdout, stderr) = run(&["caption", "export", out.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("VTT cue"), "{stdout}");
    assert!(stdout.contains("00:00:03,000 --> 00:00:04,000"), "{stdout}");
}

#[test]
fn ass_import_takes_dialogue_text_only_and_strips_tags() {
    let out = scratch("ass-imported.json");
    let (code, _, stderr) = run(&[
        "caption",
        "import",
        &fixture("subs.ass"),
        "--doc",
        &fixture("caption-target.json"),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stderr}");
    let (code, stdout, stderr) = run(&["caption", "export", out.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("Styled text"), "tags stripped: {stdout}");
    assert!(
        !stdout.contains("comment must not import"),
        "comment lines are not cues: {stdout}"
    );
}

#[test]
fn unsupported_subtitle_format_exits_one() {
    let not_subs = fixture("valid.json");
    let (code, _stdout, stderr) = run(&[
        "caption",
        "import",
        &not_subs,
        "--doc",
        &fixture("caption-target.json"),
    ]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stderr.contains("unsupported subtitle format"),
        "actionable message: {stderr}"
    );
}

#[test]
fn import_with_invalid_style_flag_exits_two() {
    let out = scratch("bad-style.json");
    let (code, _stdout, stderr) = run(&[
        "caption",
        "import",
        &fixture("subs.srt"),
        "--doc",
        &fixture("caption-target.json"),
        "--size",
        "0",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 2, "the produced document must be refused: {stderr}");
    assert!(!out.exists(), "an invalid document is not written");
}

#[test]
fn export_track_out_of_range_exits_one() {
    let (code, _stdout, stderr) = run(&[
        "caption",
        "export",
        &fixture("caption-target.json"),
        "--track",
        "3",
    ]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("caption track"), "{stderr}");
}

#[test]
fn export_vtt_writes_the_webvtt_header() {
    let out = scratch("roundtrip2.json");
    let (code, _, stderr) = run(&[
        "caption",
        "import",
        &fixture("subs.srt"),
        "--doc",
        &fixture("caption-target.json"),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stderr}");
    let (code, stdout, stderr) = run(&[
        "caption",
        "export",
        out.to_str().unwrap(),
        "--format",
        "vtt",
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.starts_with("WEBVTT\n"), "{stdout}");
}

#[test]
fn audio_analyze_reports_per_scene_json() {
    let (code, stdout, stderr) = run(&[
        "audio",
        "analyze",
        &fixture("analyze.json"),
        "--json",
        "--min-silence-ms",
        "400",
    ]);
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");
    let body: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let scenes = body["scenes"].as_array().expect("scenes array");
    assert_eq!(scenes.len(), 2, "one entry per scene");
    // The fixture is 0.5s silence, 1s tone, 0.5s silence: scene "a" starts
    // with silence, scene "b" ends with it, and both carry the tone.
    let a = &scenes[0];
    let b = &scenes[1];
    assert!(a["lufs"].is_number(), "scene a carries the tone: {a}");
    assert!(a["true_peak_dbtp"].is_number(), "{a}");
    let a_silence = a["silence"].as_array().expect("silence array");
    assert!(
        !a_silence.is_empty() && a_silence[0]["from"].as_i64() == Some(0),
        "scene a opens with silence: {a}"
    );
    assert!(b["lufs"].is_number(), "scene b carries the tone: {b}");
    assert!(
        !b["silence"].as_array().unwrap().is_empty(),
        "scene b ends with silence"
    );
    assert_eq!(body["audio_rate"], 48000);
}

#[test]
fn audio_analyze_human_output_mentions_lufs() {
    let (code, stdout, stderr) = run(&["audio", "analyze", &fixture("analyze.json")]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("LUFS"), "{stdout}");
    assert!(
        stdout.contains("scene a") && stdout.contains("scene b"),
        "{stdout}"
    );
}

#[test]
fn audio_analyze_without_audio_exits_one() {
    let (code, _stdout, stderr) = run(&["audio", "analyze", &fixture("valid.json")]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stderr.contains("no audio tracks"),
        "actionable message: {stderr}"
    );
}
