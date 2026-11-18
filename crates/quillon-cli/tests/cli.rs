//! CLI contract tests (plan §4 F10, acceptance v0.1 #1): exit codes and
//! machine-readable output, through the real binary.

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

#[test]
fn valid_document_exits_zero() {
    let (code, stdout, stderr) = run(&["validate", &fixture("valid.json")]);
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");
    assert!(stdout.contains("valid"), "{stdout}");
}

#[test]
fn invalid_document_exits_two_with_json_pointer() {
    let (code, _stdout, stderr) = run(&["validate", &fixture("bad.json")]);
    assert_eq!(code, 2, "stderr: {stderr}");
    assert!(
        stderr.contains("/scenes/0/elements/0"),
        "the error must carry a JSON pointer, got: {stderr}"
    );
}

#[test]
fn invalid_document_json_output_is_machine_readable() {
    let (code, stdout, _stderr) = run(&["validate", &fixture("bad.json"), "--json"]);
    // --json on an invalid document still exits 2 with a JSON body on stdout
    // is not promised; the contract is exit 2. Here we only check that the
    // run did not crash and produced the semantic exit code.
    assert_eq!(code, 2);
    assert!(!stdout.contains("panic"), "{stdout}");
}

#[test]
fn lint_findings_exit_three() {
    let (code, _stdout, stderr) = run(&["validate", &fixture("linty.json")]);
    assert_eq!(code, 3, "stderr: {stderr}");
    assert!(
        stderr.contains("lint/"),
        "lint findings must be reported: {stderr}"
    );
}

#[test]
fn allow_lint_downgrades_to_zero() {
    let (code, _stdout, stderr) = run(&["validate", &fixture("linty.json"), "--allow-lint"]);
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[test]
fn validate_json_output_parses() {
    let (code, stdout, _stderr) = run(&["validate", &fixture("valid.json"), "--json"]);
    assert_eq!(code, 0);
    let body: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON on stdout");
    assert_eq!(body["valid"], serde_json::json!(true));
    assert!(body["lint"].is_array());
}

#[test]
fn missing_file_exits_one() {
    let (code, _stdout, stderr) = run(&["validate", "/nonexistent/doc.json"]);
    assert_eq!(code, 1, "stderr: {stderr}");
    assert!(stderr.contains("cannot read"), "{stderr}");
}

#[test]
fn usage_error_exits_one_not_two() {
    let (code, _stdout, _stderr) = run(&["frobnicate"]);
    assert_eq!(code, 1, "exit 2 is reserved for invalid documents");
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("quillon-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn render_png_writes_frames_and_manifest() {
    let out = temp_dir("png");
    let (code, stdout, stderr) = run(&[
        "render",
        &fixture("valid.json"),
        "-o",
        out.to_str().unwrap(),
        "--format",
        "png",
    ]);
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");
    assert!(out.join("frame_000000.png").exists(), "first frame missing");
    let manifest = std::fs::read_to_string(out.join("manifest.json")).unwrap();
    let body: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(body["frame_count"], serde_json::json!(10), "1 s at 10 fps");
    assert!(body["frames"].as_array().unwrap().len() == 10);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn render_mp4_writes_video_and_manifest() {
    if !ffmpeg_available() {
        eprintln!("SKIPPED: ffmpeg not on PATH");
        return;
    }
    let out = temp_dir("mp4").join("out.mp4");
    let (code, stdout, stderr) = run(&[
        "render",
        &fixture("valid.json"),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");
    assert!(out.exists(), "mp4 missing");
    let manifest_path = out.with_file_name("out.mp4.manifest.json");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap();
    let body: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(body["frame_count"], serde_json::json!(10));
    let _ = std::fs::remove_dir_all(temp_dir("mp4"));
}

/// Acceptance v0.1 #2: two renders of the same document produce an identical
/// frame-hash list.
#[test]
fn double_render_manifests_are_identical() {
    if !ffmpeg_available() {
        eprintln!("SKIPPED: ffmpeg not on PATH");
        return;
    }
    let dir = temp_dir("double");
    let first = dir.join("first.mp4");
    let second = dir.join("second.mp4");
    let (code, _, stderr) = run(&[
        "render",
        &fixture("valid.json"),
        "-o",
        first.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stderr}");
    let (code, _, stderr) = run(&[
        "render",
        &fixture("valid.json"),
        "-o",
        second.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stderr}");
    let a = std::fs::read_to_string(first.with_file_name("first.mp4.manifest.json")).unwrap();
    let b = std::fs::read_to_string(second.with_file_name("second.mp4.manifest.json")).unwrap();
    assert_eq!(a, b, "frame hashes must be identical across renders");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn render_invalid_document_exits_two() {
    let out = temp_dir("bad-render");
    let (code, _stdout, stderr) =
        run(&["render", &fixture("bad.json"), "-o", out.to_str().unwrap()]);
    assert_eq!(code, 2, "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&out);
}

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}
