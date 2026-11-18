//! Golden hashes (plan §9.1): render the corpus, sha256 every RGBA frame
//! buffer, compare against `testdata/golden/hashes.json`.
//!
//! Policy: a renderer change that shifts pixels without an intended visual
//! change is a BUG, not a new golden (Kineto's rule). Regenerate only via
//! `cargo xtask golden` in a commit that contains nothing else (plan §14).

use quillon_core::frame_source::NullFrameSource;
use quillon_core::time::frame_tick;
use quillon_core::{Document, validate_str};
use quillon_render::Renderer;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const CORPUS: &[&str] = &["static.json", "anim.json", "text.json", "visuals.json"];

fn corpus_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/corpus"))
}

fn golden_path() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/testdata/golden/hashes.json"
    ))
}

fn load_corpus_doc(name: &str) -> Document {
    let path = corpus_dir().join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    validate_str(&text, &corpus_dir())
        .unwrap_or_else(|errors| panic!("corpus doc {name} must be valid: {errors:?}"))
}

fn frame_hashes(doc: &Document, renderer: &Renderer) -> Vec<String> {
    (0..doc.frame_count())
        .map(|i| {
            let tick = (i as i64) * frame_tick(doc.fps);
            let frame = renderer
                .render_tick(tick, &NullFrameSource)
                .unwrap_or_else(|e| panic!("frame {i}: {e}"));
            let digest = Sha256::digest(&frame.rgba);
            let mut hash = String::with_capacity(64);
            for byte in digest.iter() {
                hash.push_str(&format!("{byte:02x}"));
            }
            hash
        })
        .collect()
}

fn load_golden() -> Value {
    let text = std::fs::read_to_string(golden_path())
        .expect("golden hashes missing; run `cargo xtask golden` and commit the result");
    serde_json::from_str(&text).expect("golden hashes file must be valid JSON")
}

#[test]
fn golden_hashes_match() {
    let golden = load_golden();
    for name in CORPUS {
        let doc = load_corpus_doc(name);
        let renderer =
            Renderer::new(&doc, &corpus_dir()).unwrap_or_else(|e| panic!("prepare {name}: {e}"));
        let actual = frame_hashes(&doc, &renderer);
        let expected = golden
            .pointer(&format!("/{name}/frames"))
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("golden entry missing for {name}"))
            .iter()
            .map(|v| v.as_str().expect("hash string").to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            actual.len(),
            expected.len(),
            "{name}: frame count changed; renderer version {}/golden {}",
            quillon_render::RENDERER_VERSION,
            golden
                .pointer(&format!("/{name}/renderer_version"))
                .and_then(Value::as_i64)
                .unwrap_or(-1),
        );
        for (i, (a, e)) in actual.iter().zip(&expected).enumerate() {
            assert_eq!(a, e, "{name} frame {i} hash changed");
        }
    }
}

/// Acceptance v0.1 #2: the same document renders to the identical frame hash
/// list twice, and frame N never depends on frame N-1 (render the whole
/// document backwards).
#[test]
fn frames_are_deterministic_and_order_independent() {
    for name in CORPUS {
        let doc = load_corpus_doc(name);
        let renderer = Renderer::new(&doc, &corpus_dir()).unwrap();
        let forward = frame_hashes(&doc, &renderer);

        let mut backward = Vec::with_capacity(forward.len());
        for i in (0..doc.frame_count()).rev() {
            let tick = (i as i64) * frame_tick(doc.fps);
            let frame = renderer.render_tick(tick, &NullFrameSource).unwrap();
            backward.push((i, hex_sha256(&frame.rgba)));
        }
        backward.reverse();
        for (i, hash) in backward {
            assert_eq!(
                hash, forward[i as usize],
                "{name} frame {i} depends on render order"
            );
        }
    }
}

#[test]
fn golden_file_covers_exactly_the_corpus() {
    let golden = load_golden();
    let mut covered: Vec<&str> = golden
        .as_object()
        .expect("golden root object")
        .keys()
        .map(String::as_str)
        .collect();
    covered.sort_unstable();
    let mut expected = CORPUS.to_vec();
    expected.sort_unstable();
    assert_eq!(
        covered, expected,
        "golden file and corpus must stay in sync"
    );
}

/// Behavior checks through the public interface (not only hashes): the
/// static corpus must put real pixels where its elements are.
#[test]
fn static_frame_pixels_are_where_the_document_puts_them() {
    let doc = load_corpus_doc("static.json");
    let renderer = Renderer::new(&doc, &corpus_dir()).unwrap();
    let frame = renderer.render_tick(0, &NullFrameSource).unwrap();
    let px = |x: u32, y: u32| {
        let i = ((y * doc.size.w + x) * 4) as usize;
        [
            frame.rgba[i],
            frame.rgba[i + 1],
            frame.rgba[i + 2],
            frame.rgba[i + 3],
        ]
    };
    // Background away from every element.
    assert_eq!(px(5, 200), [16, 20, 24, 255]);
    // Solid rect interior: exact color through the identity path.
    assert_eq!(px(80, 50), [0xc2, 0x3b, 0x22, 255]);
    // Image contain box is 96x96 with a square source: the top-left quadrant
    // of the box is red.
    assert_eq!(px(224, 64), [255, 0, 0, 255]);
    // Text area: not background (glyph coverage landed).
    let text_area_has_ink = (160..320)
        .step_by(4)
        .any(|x| px(x, 180) != [16, 20, 24, 255]);
    assert!(text_area_has_ink, "text did not rasterize");
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest.iter() {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
