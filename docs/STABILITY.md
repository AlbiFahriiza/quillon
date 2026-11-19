# Stability policy (from v1.0.0-beta.1)

Quillon's stability contract has three layers: the Rust API, the
document schema, and the rendered-output determinism guarantee.

## Rust API (`quillon-core` and friends)

- During beta (`1.0.0-beta.N`), the public API is **additive-only**:
  new items may appear; existing public items keep their signatures.
- A breaking change does not silently land — it bumps to the next
  `-beta.N` (or is deferred to `1.1.0`/`2.0.0` after final) and is
  listed in the release notes.
- Deprecations: a deprecated item is marked `#[deprecated]` with a
  migration note and survives at least one minor release after final
  1.0.
- The publishable surface is `quillon-core` (document model, builder,
  validation, lint, diff, captions, subtitles, OTIO import, templates).
  The other crates are internal composition and their layout may
  change between betas.

## Document schema

- `docs/schema.json` (exported by `cargo xtask export-schema`) is the
  frozen contract; a parity test in CI fails if the served schema and
  the file drift apart.
- Schema version field: `v: 1`. Beta adds nothing and removes nothing;
  any future change follows the migration policy in
  `myquillonplan.md` §14 (additive for minors, a migrator for
  majors).
- Documents that validate today must validate for the whole 1.x line.
  The hardening guards only reject inputs that v0.6 would have
  crashed on or mis-written — never a previously-valid document.

## Render determinism

- The same document + same `quillon` version + CPU render path
  produces byte-identical frames (sha256 manifests); this is what the
  golden tests pin and what the benchmark harness re-checks each run.
- Deliberate renderer fixes bump `RENDERER_VERSION` and refresh
  goldens in a dedicated commit (plan §14) — cache keys include it,
  so caches invalidate instead of lying.

## What is explicitly not stable yet

- The MCP tool surface (additive during beta).
- CLI flags are additive; the exit-code contract (0 success, 1
  operational, 2 invalid document, 3 lint findings) is locked by
  `tests/contract.rs` and will not change in 1.x.
- Encoded MP4 bytes are not part of the determinism contract — frames
  are (encoder versions vary by machine).
