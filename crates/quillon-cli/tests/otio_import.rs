//! CLI contract tests for `quillon otio import` (plan §12 v0.6 acceptance
//! 2): the flat OTIO subset becomes a valid Quillon document with exact
//! source ranges.

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

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn otio_import_produces_a_valid_document_with_exact_ranges() {
    let out = Scratch(PathBuf::from(format!("{FIXTURES}/otio-imported.json")));
    let (code, stdout, stderr) = run(&[
        "otio",
        "import",
        &format!("{FIXTURES}/timeline.otio"),
        "-o",
        out.0.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");

    // The result must itself validate (media probe included).
    let (code, stdout, stderr) = run(&["validate", out.0.to_str().unwrap()]);
    assert_eq!(code, 0, "{stdout} {stderr}");

    // Acceptance 2: source ranges land in ticks exactly.
    let body: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out.0).unwrap()).unwrap();
    assert_eq!(body["fps"], 30);
    let scenes = body["scenes"].as_array().unwrap();
    assert_eq!(scenes.len(), 2);
    assert_eq!(scenes[0]["id"], "intro");
    assert_eq!(scenes[0]["duration"], 1_411_200_000, "60 frames @30fps");
    let video = &scenes[0]["elements"][0];
    assert_eq!(video["type"], "video");
    assert_eq!(
        video["source_range"]["from"], 705_600_000,
        "start 30 @30fps"
    );
    assert_eq!(video["source_range"]["to"], 2_116_800_000);
}

#[test]
fn otio_import_to_stdout_round_trips_through_validate() {
    let (code, stdout, stderr) = run(&["otio", "import", &format!("{FIXTURES}/timeline.otio")]);
    assert_eq!(code, 0, "{stderr}");
    let doc: serde_json::Value = serde_json::from_str(&stdout).expect("document JSON on stdout");
    assert_eq!(doc["scenes"].as_array().unwrap().len(), 2);
}

#[test]
fn non_timeline_otio_exits_one_with_a_named_error() {
    let bad = Scratch(PathBuf::from(format!("{FIXTURES}/bad.otio")));
    std::fs::write(&bad.0, "{\"OTIO_SCHEMA\": \"Stack.1\"}").unwrap();
    let (code, _stdout, stderr) = run(&["otio", "import", bad.0.to_str().unwrap()]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("Timeline.1"), "{stderr}");
}

#[test]
fn missing_media_makes_the_result_invalid_exiting_two() {
    let bad = Scratch(PathBuf::from(format!("{FIXTURES}/ghost.otio")));
    std::fs::write(
        &bad.0,
        r##"{
            "OTIO_SCHEMA": "Timeline.1",
            "tracks": { "OTIO_SCHEMA": "Stack.1", "children": [
                { "OTIO_SCHEMA": "Track.1", "kind": "Video", "children": [
                    { "OTIO_SCHEMA": "Clip.1", "name": "ghost",
                      "media_reference": { "OTIO_SCHEMA": "ExternalReference.1", "target_url": "assets/ghost.mp4" },
                      "source_range": { "OTIO_SCHEMA": "TimeRange.1",
                        "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": 30.0, "value": 0.0 },
                        "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": 30.0, "value": 30.0 } } }
                ] }
            ] }
        }"##,
    )
    .unwrap();
    let out = Scratch(PathBuf::from(format!("{FIXTURES}/ghost.json")));
    let (code, _stdout, stderr) = run(&[
        "otio",
        "import",
        bad.0.to_str().unwrap(),
        "-o",
        out.0.to_str().unwrap(),
    ]);
    assert_eq!(code, 2, "unprobed media must fail validation: {stderr}");
    assert!(!out.0.exists(), "an invalid document is not written");
}
