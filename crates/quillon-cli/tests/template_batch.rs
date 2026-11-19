//! CLI contract tests for `quillon render --data` (plan §12 v0.6
//! acceptance 1, small scale): one video per CSV row, and the shared
//! content-addressed cache renders the template's static scene only once —
//! proven from the cache counters in the `--json` summary.

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

/// A scratch directory next to the fixtures that removes itself.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(name: &str) -> Scratch {
    let p = PathBuf::from(format!("{FIXTURES}/{name}"));
    let _ = std::fs::remove_dir_all(&p);
    Scratch(p)
}

#[test]
fn batch_renders_one_video_per_row_with_shared_static_cache() {
    let out = scratch("batch-out");
    let cache = scratch("batch-cache");
    let out_pattern = format!("{}/{{{{name}}}}.mp4", out.0.display());
    let (code, stdout, stderr) = run(&[
        "render",
        &format!("{FIXTURES}/template.json"),
        "--data",
        &format!("{FIXTURES}/rows.csv"),
        "-o",
        &out_pattern,
        "--cache-dir",
        cache.0.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");

    let body: serde_json::Value = serde_json::from_str(&stdout).expect("json summary");
    assert_eq!(body["rows"], 25, "{body}");
    // 10 frames per video: 5 static (shared) + 5 variable (unique names).
    // Static frames: 5 misses, then 24 x 5 = 120 hits.
    let hits = body["cache"]["hits"].as_u64().expect("hits");
    let misses = body["cache"]["misses"].as_u64().expect("misses");
    assert!(
        hits >= 24 * 5,
        "the static scene must hit the shared cache across rows: hits {hits}, misses {misses}"
    );
    assert_eq!(
        misses,
        5 + 25 * 5,
        "one static-scene render plus every variable frame"
    );
    let outputs = body["outputs"].as_array().expect("outputs");
    assert_eq!(outputs.len(), 25);
    for path in outputs {
        assert!(
            std::path::Path::new(path.as_str().unwrap()).exists(),
            "output {path} must exist"
        );
    }
}

#[test]
fn batch_without_data_still_renders_a_single_document() {
    // Backward compatibility: no --data means today's single render, with
    // variables falling back to their declared defaults.
    let out = scratch("single-out");
    let (code, stdout, stderr) = run(&[
        "render",
        &format!("{FIXTURES}/template.json"),
        "-o",
        out.0.join("single.mp4").to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");
    assert!(out.0.join("single.mp4").exists());
}

#[test]
fn bad_row_column_stops_the_batch_with_row_context() {
    let bad = Scratch(PathBuf::from(format!("{FIXTURES}/bad-rows.csv")));
    std::fs::write(&bad.0, "nmae\nAda\n").unwrap();
    let out = scratch("bad-out");
    let (code, _stdout, stderr) = run(&[
        "render",
        &format!("{FIXTURES}/template.json"),
        "--data",
        bad.0.to_str().unwrap(),
        "-o",
        &format!("{}/{{{{nmae}}}}.mp4", out.0.display()),
    ]);
    assert_eq!(code, 1, "an undeclared column is operational: {stderr}");
    assert!(
        stderr.contains("row 2") && stderr.contains("\"nmae\""),
        "the error names the row and the column: {stderr}"
    );
}

#[test]
fn row_that_makes_an_invalid_document_exits_two() {
    // name is fine, but a {{name}} filled into a 10-char-limited field
    // would be invalid; here the template is valid, so instead we point a
    // color-typed variable at garbage via a dedicated fixture.
    let bad = Scratch(PathBuf::from(format!("{FIXTURES}/color-rows.csv")));
    std::fs::write(&bad.0, "accent\nnotacolor\n").unwrap();
    let doc = Scratch(PathBuf::from(format!("{FIXTURES}/color-template.json")));
    std::fs::write(
        &doc.0,
        r##"{
            "v": 1,
            "size": { "w": 320, "h": 240 },
            "assets": { "body": { "type": "font", "src": "assets/font.ttf" } },
            "variables": { "accent": { "type": "color", "default": "#ffffff" } },
            "scenes": [ { "id": "s", "duration": 352800000,
                "elements": [ { "type": "rect", "x": 0, "y": 0, "w": 10, "h": 10, "color": "{{accent}}" } ] } ]
        }"##,
    )
    .unwrap();
    let out = scratch("color-out");
    let (code, _stdout, stderr) = run(&[
        "render",
        doc.0.to_str().unwrap(),
        "--data",
        bad.0.to_str().unwrap(),
        "-o",
        &format!("{}/{{{{accent}}}}.mp4", out.0.display()),
    ]);
    assert_eq!(
        code, 1,
        "a wrong-typed value is caught at substitution: {stderr}"
    );
    assert!(
        stderr.contains("row 2") && stderr.contains("accent"),
        "{stderr}"
    );
}
