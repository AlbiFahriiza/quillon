# Quillon

Video as Code: create videos from scratch with code, and edit existing footage
programmatically. One Rust binary, no browser, no Node, no per-render license.
The interchange format is a schema-validated JSON document — designed so AI
agents can compose and edit video with validated output instead of guessing.

Status: **v0.2 (Motion & Preview) in development.** Scope and architecture:
[`myquillonplan.md`](myquillonplan.md); underlying research in [`research/`](research/).

## What it does

- `quillon validate doc.json` — structural + semantic validation with
  JSON-pointer error paths, plus semantic lint (low contrast, off-canvas
  elements, text past the edge, unreadable scenes).
- `quillon render doc.json -o out.mp4` — deterministic CPU rendering
  (tiny-skia + cosmic-text, linear-light compositing), MP4 via the system
  ffmpeg binary, PNG frame export, and a per-frame sha256 manifest. Frames
  render in parallel (`--workers`) and stream straight into the encoder, so
  memory stays bounded on long videos.
- `quillon render --cache-dir <dir>` — content-addressed frame cache: a
  re-render computes only frames whose scene content changed (edit one
  scene, re-render that scene).
- `quillon preview doc.json` — a localhost filmstrip of the draft
  (half-resolution, same render path as the export) that reloads when the
  document is saved. Serves on 127.0.0.1 only.
- `quillon-core` — the document model, JSON Schema, validation, and builder
  API as a library.

## Build and test

```
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

MP4 rendering requires `ffmpeg` on the PATH (PNG rendering and preview do
not).

## CLI contract

```
quillon validate <doc.json> [--json] [--allow-lint]
quillon render <doc.json> -o <out> [--format png|mp4] [--workers N]
                 [--cache-dir <dir>] [--lufs <target>] [--json]
quillon preview <doc.json> [--port N]
quillon probe <media-file> [--json]
quillon inspect <doc.json> [--json]
quillon mcp [--root <dir>]
quillon caption import <subs.srt|subs.vtt|subs.ass> --doc <doc.json> [-o out.json]
                 [--font <id>] [--size N] [--margin N]
                 [--position bottom|center|top] [--bg #rrggbb] [--no-regularize]
quillon caption export <doc.json> [-o out.srt] [--track N] [--format srt|vtt]
quillon audio analyze <doc.json> [--json] [--silence-db N] [--min-silence-ms N]
```

Exit codes: `0` success · `2` invalid document (errors carry JSON pointers) ·
`3` valid but lint findings (suppress with `--allow-lint`) · `1` other errors.
All subcommands accept `--json` for machine-readable output.

Caption tracks (v0.5) are document-level: cues carry document-timeline ticks
that may cross scene boundaries and burn in over the composited frame with a
per-track style (font, size, position, margin, optional shadow and backing
box). Imports always pass through timing regularization (min/max duration,
reading speed, merge of tiny gaps — configurable, never raw). Whisper
transcription is an optional feature build:

```
cargo build --features transcribe    # adds `quillon caption transcribe`
```

The model (whisper.cpp's tiny.en, sha256-pinned) downloads into the user
cache on first use; default builds pull no C toolchain.

## Agent surface (MCP)

`quillon mcp` serves the agent tool set over stdio (plan §8): validate,
preview frames, diff documents, list assets/fonts, probe media, render a
frame, and start/poll a full render — cheapest first, so most turns cost no
pixels. Register it:
`claude mcp add --scope user quillon -- quillon mcp --root <project-dir>`.
The agent-facing guide is [`skills/quillon/SKILL.md`](skills/quillon/SKILL.md);
the schema is also served as the `quillon://schema/document` resource.

## Document format (v1)

Time is integer ticks (flicks): `705600000` per second, so 24/25/30/50/60 fps
are all exact. Colors are authored as sRGB hex and composited in linear light.
Elements: `text`, `rect`, `image`. Animatable properties: `opacity`,
`translate_x`, `translate_y`, `scale`, `rotate` with keyframes and 10 easing
curves. Scenes transition with `cut` (the default) or `crossfade`, blended in
linear light over the previous scene. Rects fill with a solid color or a
linear `gradient` (stops interpolated in linear light) and take a corner
`radius`; any element can carry a blurred `shadow` and a rectangular `clip`.
`video` elements trim existing footage with `source_range` (keyframe-accurate
decode-forward); audio tracks mix with `gain`, `fades` and optional LUFS
normalization (EBU R128 −23 preset). The full schema ships as [`docs/schema.json`](docs/schema.json)
(regenerated with `cargo xtask export-schema`).

The same document always renders to identical frame hashes on the same
platform — the reproducibility contract covers pixels, not MP4 container
bytes.

## License

MIT OR Apache-2.0. Dependency attributions: [NOTICE](NOTICE).
