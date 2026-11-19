//! The CLI exit-code and `--json` contract, as one table (hardening
//! step 7). Scripts and the MCP server build on these codes; this file
//! is the single source of truth they can be checked against:
//!
//!   0  success
//!   1  operational failure (unreadable input, bad data row, misuse)
//!   2  the document is invalid
//!   3  the document is valid but lint findings were reported

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_quillon");
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn fixture(name: &str) -> String {
    format!("{FIXTURES}/{name}")
}

fn run(args: &[String]) -> (i32, String, String) {
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
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(name: &str) -> Scratch {
    let p = std::env::temp_dir().join(format!("quillon-contract-{name}"));
    let _ = std::fs::remove_dir_all(&p);
    Scratch(p)
}

/// One row of the contract: the full command, the exit code it
/// promises, and whether stdout must parse as JSON.
struct Case {
    name: &'static str,
    args: Vec<String>,
    code: i32,
    json_stdout: bool,
}

fn s(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

#[test]
fn cli_contract_table() {
    let render_out = scratch("render-out");
    let batch_out = scratch("batch-out");
    let otio_out = scratch("otio-out");
    // The caption document sits directly in the fixtures tree so its
    // relative assets ("assets/font.ttf") resolve like the other
    // caption tests.
    let caption_doc = Scratch(PathBuf::from(format!("{FIXTURES}/contract-doc.json")));
    let caption_vtt = scratch("caption-out");

    let cases: Vec<Case> = vec![
        Case {
            name: "validate ok",
            args: s(&["validate", &fixture("valid.json")]),
            code: 0,
            json_stdout: false,
        },
        Case {
            name: "validate ok --json",
            args: s(&["validate", &fixture("valid.json"), "--json"]),
            code: 0,
            json_stdout: true,
        },
        Case {
            name: "validate invalid document",
            args: s(&["validate", &fixture("bad.json")]),
            code: 2,
            json_stdout: false,
        },
        Case {
            name: "validate lint findings",
            args: s(&["validate", &fixture("linty.json")]),
            code: 3,
            json_stdout: false,
        },
        Case {
            name: "validate lint allowed",
            args: s(&["validate", &fixture("linty.json"), "--allow-lint"]),
            code: 0,
            json_stdout: false,
        },
        Case {
            name: "validate missing file",
            args: s(&["validate", "/nonexistent/doc.json"]),
            code: 1,
            json_stdout: false,
        },
        Case {
            name: "usage error",
            args: s(&["validate", "--definitely-not-a-flag"]),
            code: 1,
            json_stdout: false,
        },
        Case {
            name: "render ok --json",
            args: s(&[
                "render",
                &fixture("valid.json"),
                "-o",
                &render_out.0.join("out.mp4").display().to_string(),
                "--json",
            ]),
            code: 0,
            json_stdout: true,
        },
        Case {
            name: "render zero workers (D6)",
            args: s(&[
                "render",
                &fixture("valid.json"),
                "-o",
                &render_out.0.join("w0.mp4").display().to_string(),
                "--workers",
                "0",
            ]),
            code: 1,
            json_stdout: false,
        },
        Case {
            name: "render traversal row (D3)",
            args: s(&[
                "render",
                &fixture("template.json"),
                "--data",
                &fixture("evil-rows.csv"),
                "-o",
                &format!("{}/{{{{name}}}}.mp4", batch_out.0.display()),
            ]),
            code: 1,
            json_stdout: false,
        },
        Case {
            name: "otio import ok",
            args: s(&[
                "otio",
                "import",
                &fixture("timeline.otio"),
                "-o",
                &otio_out.0.join("doc.json").display().to_string(),
            ]),
            code: 0,
            json_stdout: false,
        },
        Case {
            name: "caption import ok",
            args: s(&[
                "caption",
                "import",
                &fixture("subs.srt"),
                "--doc",
                &fixture("caption-target.json"),
                "-o",
                &caption_doc.0.display().to_string(),
            ]),
            code: 0,
            json_stdout: false,
        },
        Case {
            name: "caption export ok",
            args: s(&[
                "caption",
                "export",
                &caption_doc.0.display().to_string(),
                "-o",
                &caption_vtt.0.join("out.vtt").display().to_string(),
            ]),
            code: 0,
            json_stdout: false,
        },
    ];

    assert!(cases.len() >= 10, "the table must stay comprehensive");
    for case in &cases {
        let (code, stdout, stderr) = run(&case.args);
        assert_eq!(
            code, case.code,
            "contract case {:?} (args {:?}): stdout: {stdout}, stderr: {stderr}",
            case.name, case.args
        );
        if case.json_stdout {
            let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
            assert!(
                parsed.is_ok(),
                "contract case {:?}: stdout must be valid JSON, got: {stdout}",
                case.name
            );
        }
    }
}
