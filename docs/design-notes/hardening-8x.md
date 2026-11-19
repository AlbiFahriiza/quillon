# Design Note: Hardening 8× (post-v0.6)

Session goal: close the failure classes that v0.1–v0.6 features opened.
Not new features — guards with names, and a regression test for every
guard. Zero new dependencies (P3). Baseline at start: battery green
(287 tests, clippy 0, fmt 0) at `ae795f6`.

Non-goals: no new rendering capability, no format/schema change, no
refactor of the render pipeline, no changes to cache layout, no new
dependencies.

Rationale for the main choice (guards in our own code over a sanitizer
or fuzzing crate): every input channel already flows through Quillon's
own parsers (CSV, OTIO, template are first-party by design from v0.6),
so named guards at those boundaries give precise, testable errors and
keep the dependency surface at zero; a sanitizer crate would only
cover the one path (out pattern) and pull in maintenance debt for the
rest.

## Attack surface (read from the real code, 2026-11-19)

- `template.rs::fill_pattern` + `batch.rs:80-86` — row values flow
  **unsanitized** into the output path: `name = "../../evil"` with
  pattern `out/{{name}}.mp4` writes outside the output directory
  (`create_dir_all` even builds the escape directories). Asset `src`
  has §11 confinement; the **out pattern has none**.
- `csv.rs::parse_rows` — no row-count or field-size bound; a 1 GB
  quoted field or 10 M rows parse into memory before any validation
  runs. A UTF-8 BOM silently corrupts the first header name.
- `otio.rs` — tick conversion multiplies by timebase before an `as`
  cast; an absurd rate/value can overflow (silent clamp today).
- panic surface (`unwrap/expect/panic/unreachable` outside tests):
  cache 25, audio 13, mcp 11, builder 9, draft 8, encode 7 hits
  (counts include test modules — Step 1 classifies for real).
- Exit codes and `--json` output shape are conventions in `main.rs`
  with no table test locking them.

## Invariants (must hold after every step)

1. Every guard produces a **named error** naming the limit and the
   actual value (no bare "invalid input").
2. No valid workload regresses: the 1000-row template acceptance must
   stay green and fast (cache hits unchanged).
3. Substitution stays **before** validation (P1): guards on row values
   never bypass the schema pipeline.
4. Zero new dependencies; NOTICE unchanged.
5. Every step lands with its regression test already failing on the
   pre-fix code path (test first, watch it fail, then fix).

## Steps

### D1 — Panic audit (production paths only)
Classify every `unwrap/expect/panic/unreachable` hit as TEST (fine) or
PRODUCTION (must become a `Result` with a named error, or carry an
invariant comment). Files in order of hits: cache, audio, mcp, builder,
draft, encode, decoder, subtitle, caption. Only production hits move;
smallest diff, no refactors.

### D2 — Deterministic fuzz property (no new deps)
Seeded PRNG (xorshift) generating (a) arbitrary JSON trees valid and
malformed, (b) arbitrary byte strings as subtitle/CSV/OTIO input.
Property: `validate_str`, `parse` (srt/vtt), `import_otio`,
`apply_template_json`, `parse_rows` either return or return Err —
never panic, never allocate unboundedly (input capped at 64 KB per
iteration). 10k iterations in CI, 100k behind `#[ignore]` for local
runs. Seed printed on failure.

### D3 — Path traversal guard on `fill_pattern`
`fill_pattern` rejects a resolved value containing `/`, `\`, or `..`
when it lands in the **pattern** (out path), naming the variable and
the row. Values in the document tree stay free (§11 confines assets;
text is text). Empty result after fill and absolute paths are named
errors too. Regression test: the `../../evil` attack, the absolute
path `/tmp/evil`, the `a\b` and `..` values, and the empty-after-fill
case.

### D4 — Bounded inputs (DOS guard)
`csv::parse_rows` gets `MAX_CSV_ROWS` (1_000_000) and
`MAX_FIELD_BYTES` (1 MB) with named errors (row/column context).
UTF-8 BOM is stripped before parsing. `otio.rs` tick math gets
`checked_mul/add` with a named overflow error (no silent `as` clamp).
Document MAX_* constants already exist and stay as-is.

### D5 — Template semantics locked by tests
Non-recursion (a row value containing `{{name}}` stays literal),
marker in key position, whitespace markers (`{{ name }}` — currently
trimmed, lock it), unterminated marker stays literal, defaults used
for missing columns. Most already behave; the tests make it permanent.

### D6 — Render/encode guards
`--workers 0` (from a user typo) → named error, not a silent default.
Batch render error text names the row and the codec when ffmpeg dies.
Out pattern that fills to an empty string or a directory → named error
before any ffmpeg spawn.

### D7 — Exit-code & `--json` contract table test
One test file: (command, input, expected exit, stdout must parse as
JSON when `--json`) for success, operational failure, invalid
document, and each new guard error. Locks the CLI contract the MCP
server and scripts depend on.

### D8 — Verification suite + docs
`docs/HARDENING.md`: the guard list, the limits, how to add a guard.
Final battery + coverage (floor 80, current 88.57%) + 1000-row
acceptance rerun + gates + CI.

## Rollback

Each step is one commit; any regression reverts that commit alone.
No data migrations, no format changes — v0.6 documents and caches stay
valid (guards only reject inputs v0.6 would have crashed on or
mis-written).
