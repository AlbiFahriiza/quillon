# Quillon

Video as Code: create videos from scratch with code, and edit existing footage
programmatically. One Rust binary, no browser, no Node, no per-render license.
The interchange format is a schema-validated JSON document — designed so AI
agents can compose and edit video with validated output instead of guessing.

Status: **v0.1 (Foundation) in development.** Scope and architecture:
[`myquillonplan.md`](myquillonplan.md); underlying research in [`research/`](research/).

## What v0.1 does

- `quillon validate doc.json` — structural + semantic validation with
  JSON-pointer error paths, plus semantic lint (low contrast, off-canvas
  elements, text past the edge, unreadable scenes).
- `quillon render doc.json -o out.mp4` — deterministic CPU rendering
  (tiny-skia + cosmic-text, linear-light compositing), MP4 via the system
  ffmpeg binary, PNG frame export, and a per-frame sha256 manifest.
- `quillon-core` — the document model, JSON Schema, validation, and builder
  API as a library.

## Build and test

```
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

MP4 rendering requires `ffmpeg` on the PATH (PNG rendering does not).

## CLI contract

```
quillon validate <doc.json> [--json] [--allow-lint]
quillon render <doc.json> -o <out> [--format png|mp4] [--json]
```

Exit codes: `0` success · `2` invalid document (errors carry JSON pointers) ·
`3` valid but lint findings (suppress with `--allow-lint`) · `1` other errors.
All subcommands accept `--json` for machine-readable output.

## Document format (v1)

Time is integer ticks (flicks): `705600000` per second, so 24/25/30/50/60 fps
are all exact. Colors are authored as sRGB hex and composited in linear light.
Elements: `text`, `rect`, `image`. Animatable properties: `opacity`,
`translate_x`, `translate_y`, `scale`, `rotate` with keyframes and 10 easing
curves. The full schema ships as [`docs/schema.json`](docs/schema.json)
(regenerated with `cargo xtask export-schema`).

The same document always renders to identical frame hashes on the same
platform — the reproducibility contract covers pixels, not MP4 container
bytes.

## License

MIT OR Apache-2.0. Dependency attributions: [NOTICE](NOTICE).
