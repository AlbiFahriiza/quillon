//! Deterministic fuzz smoke (hardening design note D2): every external
//! input channel must either return or return Err — never panic. A
//! seeded xorshift PRNG generates arbitrary strings and JSON trees
//! (valid-looking and garbage) and pushes them through the public
//! parsers. No new dependency; the seed is printed on any panic so the
//! exact failing input can be replayed with FUZZ_SEED.
//!
//! 10k iterations run in CI; the 100k run is `#[ignore]` for local
//! verification (`cargo test --release -- --ignored`).

use quillon_core::{
    VariableDef, VariableType, apply_template_json, fill_pattern, import_otio, parse_srt,
    parse_vtt, validate_str,
};
use std::collections::BTreeMap;

/// xorshift64* — deterministic across platforms for a given seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// Alphabet mixing document syntax, template markers, quotes,
/// control-ish characters and multi-byte UTF-8 — the characters that
/// parsers make mistakes on.
const CHARS: &[&str] = &[
    "{", "}", "{{", "}}", "{{x}}", "\"", "\\", ":", ",", "[", "]", "\n", "\r", "\t", " ", "a", "é",
    "文", "😀", "0", "-", "9", ".", "e", "#", "/", "..", "null", "true", "<", ">",
];

fn random_string(rng: &mut Rng, max_units: usize) -> String {
    let units = rng.below(max_units as u64 + 1) as usize;
    let mut out = String::new();
    for _ in 0..units {
        out.push_str(CHARS[rng.below(CHARS.len() as u64) as usize]);
    }
    out
}

/// JSON value generator: depth-bounded, mixing plausible document keys
/// with junk so validate_str exercises both its happy paths and its
/// rejection paths.
fn random_json(rng: &mut Rng, depth: u32) -> serde_json::Value {
    use serde_json::json;
    if depth == 0 {
        return match rng.below(7) {
            0 => json!(null),
            1 => json!(rng.below(2) == 0),
            2 => match rng.below(4) {
                // Mix "large" with "near the type bounds" so the r1
                // duration/geometry caps are exercised too.
                0 => json!((rng.next() as i64) - (1 << 40)),
                1 => json!(i64::MAX - rng.below(4) as i64),
                2 => json!(i64::MIN + rng.below(4) as i64),
                _ => json!(rng.below(400) as i64 * 705_600_000),
            },
            3 => json!(rng.next() as f64 / 65536.0), // fractional, also NaN-free
            4 => json!(random_string(rng, 12)),
            5 => json!(rng.next()), // u64 magnitude
            _ => json!(random_string(rng, 3)),
        };
    }
    match rng.below(10) {
        0..=2 => serde_json::Value::Array(
            (0..rng.below(5))
                .map(|_| random_json(rng, depth - 1))
                .collect(),
        ),
        3..=7 => {
            // Plausible document shape, so real validation paths run.
            let mut map = serde_json::Map::new();
            let keys = match rng.below(4) {
                0 => vec!["version", "fps", "size", "scenes", "assets"],
                1 => vec!["id", "duration", "elements", "text", "color"],
                2 => vec!["type", "src", "variables", "default"],
                _ => vec!["scenes", "elements", "x", "y", "w", "h"],
            };
            for key in keys {
                map.insert(key.to_string(), random_json(rng, depth - 1));
            }
            serde_json::Value::Object(map)
        }
        8 => {
            let mut map = serde_json::Map::new();
            map.insert(random_string(rng, 4), random_json(rng, depth - 1));
            serde_json::Value::Object(map)
        }
        _ => serde_json::Value::String(random_string(rng, 8)),
    }
}

fn random_defs(rng: &mut Rng) -> BTreeMap<String, VariableDef> {
    let types = [
        VariableType::String,
        VariableType::Number,
        VariableType::Color,
        VariableType::Asset,
    ];
    let mut defs = BTreeMap::new();
    for (i, name) in ["name", "score", "accent", "shot"].iter().enumerate() {
        if rng.below(4) != i as u64 % 4 || i == 0 {
            defs.insert(
                (*name).to_string(),
                VariableDef {
                    variable_type: types[rng.below(4) as usize],
                    default: random_string(rng, 2),
                },
            );
        }
    }
    defs
}

fn random_row(rng: &mut Rng) -> BTreeMap<String, String> {
    let mut row = BTreeMap::new();
    for name in ["name", "score", "accent", "shot", "typo_column"] {
        if rng.below(3) == 0 {
            row.insert(name.to_string(), random_string(rng, 4));
        }
    }
    row
}

/// Push one generated input through every external channel. Any panic
/// is caught and re-thrown with the seed so the input can be replayed.
fn one_iteration(rng: &mut Rng, seed: u64, i: usize) {
    let text = random_string(rng, 64);
    let tree = random_json(rng, 4);
    let tree_text = serde_json::to_string(&tree).unwrap_or_default();
    let defs = random_defs(rng);
    let row = random_row(rng);
    let pattern = random_string(rng, 6);
    let base = std::env::temp_dir();

    let payload = format!(
        "FUZZ PANIC seed={seed} iter={i}\n  text={text:?}\n  tree={tree_text}\n  pattern={pattern:?}"
    );
    let result = std::panic::catch_unwind(|| {
        let _ = validate_str(&text, &base);
        let _ = validate_str(&tree_text, &base);
        let _ = parse_srt(&text, 600_000_000);
        let _ = parse_vtt(&text, 600_000_000);
        let _ = import_otio(&text, Some("/tmp"));
        let _ = import_otio(&tree_text, None);
        let _ = apply_template_json(&tree, &defs, &row);
        let _ = fill_pattern(&pattern, &defs, &row);
    });
    if result.is_err() {
        panic!("{payload}");
    }
}

fn run_fuzz(seed: u64, iterations: usize) {
    let mut rng = Rng(seed);
    for i in 0..iterations {
        one_iteration(&mut rng, seed, i);
    }
}

#[test]
fn hardening_fuzz_10k() {
    let seed = std::env::var("FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x0005_1D11_C0DE);
    run_fuzz(seed, 10_000);
}

#[test]
#[ignore = "100k iterations for local verification runs"]
fn hardening_fuzz_100k() {
    run_fuzz(0xC0FF_EE00_0001, 100_000);
}
