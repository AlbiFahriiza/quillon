# AGENTS.md

Conventions for coding agents working in this repository.

## Commands

- Build: `cargo build`
- Test: `cargo test --workspace`
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`
- Format: `cargo fmt --check` (run `cargo fmt` before committing)
- Schema export: `cargo xtask export-schema` (writes `docs/schema.json`)
- Golden refresh: `cargo xtask golden` (only with a deliberate renderer change;
  see `myquillonplan.md` §14 for the commit policy)
- Gates: `python3 <disciplined-coding>/scripts/check_diff.py`,
  `arch_gate.py`, `stop_gate.py` from the repo root (config: `disciplined.yaml`)

## Architecture rules (myquillonplan.md §3, §6)

- One-way dependencies: `quillon-core` imports no internal crate; `render`,
  `media` import only `core`; `cli` imports `core`, `render`, `media`. No
  cycles. `arch_gate.py` enforces this.
- `render_tick` is a pure function of `(document, tick)` — no inter-frame
  state, no wall clock, no randomness. Frame hashes are the determinism
  contract.
- Time is integer ticks (705600000/s). Never introduce float seconds into the
  document format.
- Compositing happens in linear light; sRGB conversion happens once, at
  output.
- Fonts come from document assets only (pinned by hash) — never system font
  fallback; that would break reproducibility.
- The validation pipeline is serde-first, then jsonschema; all errors are
  wrapped in `ValidationError` with JSON-pointer paths. Raw `oneOf` errors
  must not reach users (myquillonplan.md §5.2).
- New dependencies must be verified against the crates.io API on the day they
  are added (plan §16.4) and recorded in NOTICE.

## Exit-code contract

`0` ok · `2` invalid document · `3` lint findings · `1` everything else. CLI
usage errors exit `1`, not `2` — `2` is reserved for document validation.

## Testing

- Every validator boundary has a test (plan §9.5): negative ticks, zero
  durations, `to <= from`, missing assets, hash mismatch, fps not dividing
  the timebase, empty text, opacity outside [0,1].
- Golden hashes: `crates/quillon-render/testdata/golden/hashes.json`. If a
  renderer change shifts pixels intentionally, bump `RENDERER_VERSION` and
  refresh goldens in a separate commit (plan §14).
- A test that has never failed is not evidence; bug fixes come with a
  regression test that failed before the fix.
