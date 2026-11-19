//! CLI contract tests for `--encode` (plan §12 v0.6, design note D4):
//! software is pinned, `auto` falls back with a printed note on this
//! CPU-only machine, and unknown encoder names list what ffmpeg offers.

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

#[test]
fn software_encode_renders_and_reports_not_hw() {
    let out = format!("{FIXTURES}/encode-soft.mp4");
    let _ = std::fs::remove_file(&out);
    let (code, stdout, stderr) = run(&[
        "render",
        &format!("{FIXTURES}/valid.json"),
        "-o",
        &out,
        "--encode",
        "software",
        "--json",
    ]);
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");
    let body: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(body["encoder"], "libx264");
    assert_eq!(body["hw_encode"], false);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn auto_encode_never_falls_back_silently() {
    // This machine has no GPU encoder, so `auto` must render with
    // libx264 AND print the fallback note.
    let out = format!("{FIXTURES}/encode-auto.mp4");
    let _ = std::fs::remove_file(&out);
    let (code, stdout, stderr) = run(&[
        "render",
        &format!("{FIXTURES}/valid.json"),
        "-o",
        &out,
        "--encode",
        "auto",
    ]);
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");
    assert!(
        stderr.contains("using software libx264"),
        "the fallback note must be printed: {stderr}"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn unknown_encoder_names_error_listing_the_available() {
    let (code, _stdout, stderr) = run(&[
        "render",
        &format!("{FIXTURES}/valid.json"),
        "-o",
        &format!("{FIXTURES}/never.mp4"),
        "--encode",
        "h264_bogus",
    ]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stderr.contains("h264_bogus") && stderr.contains("not available"),
        "{stderr}"
    );
    assert!(!std::path::Path::new(&format!("{FIXTURES}/never.mp4")).exists());
}
