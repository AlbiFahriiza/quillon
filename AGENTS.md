# AGENTS.md

Conventions for coding agents working in this repository. Version:
1.0.0-beta.1 (stability policy: `docs/STABILITY.md`).

## Commands

- Build: `cargo build`
- Test: `cargo test --workspace`
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`
- Format: `cargo fmt --check` (run `cargo fmt` before committing)
- Schema export: `cargo xtask export-schema` (writes `docs/schema.json`)
- Golden refresh: `cargo xtask golden` (only with a deliberate renderer change;
  see `myquillonplan.md` §14 for the commit policy)
- Gallery refresh: `cargo xtask gallery` (writes `docs/gallery-assets/*.png`
  from `templates/` — never edit the images by hand)
- Benchmarks: `cargo run --release -p xtask -- bench` (writes
  `docs/benchmarks.md`; hardware-disclosed, median + p10/p90, research/Q)
- Gates: `python3 <disciplined-coding>/scripts/check_diff.py`,
  `arch_gate.py`, `stop_gate.py --check` from the repo root (config:
  `disciplined.yaml`)

## MCP agent surface

`quillon mcp` serves nine tools over stdio — `validate_document`,
`preview_frames`, `diff_documents`, `list_assets`, `list_fonts`,
`probe_media`, `render_frame`, `render_document`, `render_status` —
plus the schema resource (stdio only — a hosted HTTP endpoint is a
separate service, research/I §2). The server lives in
`crates/quillon-cli/src/mcp.rs`; render machinery is shared with the CLI via
`crates/quillon-cli/src/render_doc.rs` (same-engine guarantee). Register it:
`claude mcp add --scope user quillon -- quillon mcp --root <dir>`. Agent
usage follows the cheapest-first loop in `skills/quillon/SKILL.md`;
E2E coverage is `crates/quillon-cli/tests/mcp_e2e.rs`.

## Architecture rules (myquillonplan.md §3, §6)

- One-way dependencies: `quillon-core` imports no internal crate; `render`,
  `media`, `audio`, `cache` import only `core`; `cli` imports `core`,
  `render`, `media`, `cache`, `audio`. No cycles. `arch_gate.py` enforces
  this against `disciplined.yaml` (the `audio` layer entry is required —
  it was missing once and the gate could not see the cli→audio edge).
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

## Caption & audio surface (v0.5)

Captions are document-level tracks (`captions` array next to `audio`), not
scene elements: cue timing is document-global and may cross scenes; burn-in
composites over the finished frame (after crossfades). Imports (SRT/VTT via
`quillon-core`, ASS/SSA via rsubs-lib in the CLI) always pass through
`regularize_cues` (own timing regularization — research/H: raw parser output
is never burned) and are clamped to the document duration; `--no-regularize`
keeps source timing exactly. The frame cache keys only the cues active at a
frame's tick, so editing one cue invalidates only its frame range. Whisper
transcription lives behind the default-off `transcribe` feature (whisper-rs +
ureq; model weights download on first use, sha256-pinned) — plan §15 #7.
`quillon audio analyze` measures each scene's LUFS/true peak (ebur128) and
silence ranges. The acceptance tests are
`quillon-audio/tests/transcribe_acceptance.rs` (deterministic click-track)
and `tests/whisper_e2e.rs` (ignored; needs the model).

## Templates, OTIO and encoding (v0.6)

Templates substitute on the JSON tree BEFORE validation
(`apply_template_json`): typed variables (`string|number|color|asset`)
with defaults, `{{name}}` markers in any string, unknown markers and
undeclared CSV columns are named errors; rows that produce an invalid
document exit 2. One shared cache directory serves the whole batch —
scene-subtree keys make identical static frames hit. `quillon otio
import` parses the flat OTIO subset in core (no external OTIO
dependency; relative media URLs stay relative per the §11 confinement,
absolute URLs rebase into --media-dir). `--encode auto` requires BOTH a
compiled-in encoder and its device node (/dev/dri for VAAPI, /dev/nvidia*
for NVENC) — an encoder list is not availability (first-hand
observation recorded in the v0.6 design note). The libav in-process
feature remains a documented deviation: no libav dev headers in this
environment, and unverified code is not written (design note D5).

## Testing

- Every validator boundary has a test (plan §9.5): negative ticks, zero
  durations, `to <= from`, missing assets, hash mismatch, fps not dividing
  the timebase, empty text, opacity outside [0,1].
- Golden hashes: `crates/quillon-render/testdata/golden/hashes.json`. If a
  renderer change shifts pixels intentionally, bump `RENDERER_VERSION` and
  refresh goldens in a separate commit (plan §14).
- A test that has never failed is not evidence; bug fixes come with a
  regression test that failed before the fix.
