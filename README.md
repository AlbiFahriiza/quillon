# Quillon

Video as Code: create videos from scratch with code, and edit existing
footage programmatically. One Rust binary, no browser, no Node, no
per-render license. The interchange format is a schema-validated JSON
document — designed so AI agents can compose and edit video with
validated output instead of guessing.

Status: **1.0.0-beta.1** — feature-complete for the 1.0 scope, in beta
for the stability freeze (see [docs/STABILITY.md](docs/STABILITY.md)).
The plan and research trail live in
[`myquillonplan.md`](myquillonplan.md) and [`research/`](research/).

## Install

Prebuilt binaries (Linux x86_64/aarch64, macOS x86_64/Apple silicon,
Windows x86_64) ship per release, checksummed, with a curl-installer:

```
curl -fsSL https://github.com/AlbiFahriiza/quillon/releases/latest/download/installer.sh | sh
```

MP4 rendering uses the system `ffmpeg` at runtime (not bundled):
`apt-get install ffmpeg` / `brew install ffmpeg` /
`choco install ffmpeg`.

From source (Rust 1.85+):

```
git clone https://github.com/AlbiFahriiza/quillon && cd quillon
cargo build --release -p quillon-cli --bin quillon
```

## The 60-second tour

A document is plain JSON — canvas, assets, scenes of elements:

```
{
  "v": 1, "size": { "w": 1280, "h": 720 }, "fps": 30, "bg": "#0b1020",
  "assets": { "body": { "type": "font", "src": "assets/DejaVuSans-ascii.ttf" } },
  "scenes": [
    { "id": "title", "duration": 2116800000, "elements": [
      { "type": "text", "text": "Quillon", "font": "body",
        "x": 640, "y": 250, "size": 96.0, "color": "#eaf0ff" }
    ]}
  ]
}
```

```
quillon validate doc.json          # schema + semantics + assets + lint, JSON-pointer errors
quillon render doc.json -o out.mp4 # deterministic CPU render (MP4 via ffmpeg)
quillon render doc.json -o frames/ # or PNG frames + per-frame sha256 manifest
```

What renders is deterministic: the same document and version produce
byte-identical frames, pinned by golden hash tests and re-checked by
the [benchmark harness](docs/benchmarks.md) on every run.

## Command surface

| Command | What it does |
|---|---|
| `quillon validate <doc>` | Schema, semantics, asset probe, then lint (exit 3 = lint findings, `--allow-lint` to accept) |
| `quillon render <doc> -o out.mp4` | Render to MP4 (or PNG frames); parallel workers, optional content-addressed `--cache-dir`, audio loudness `--lufs`, encoder choice `--encode auto\|software\|<name>` |
| `quillon render <doc> --data rows.csv -o 'out/{{name}}.mp4'` | Mail-merge batch: one video per CSV row, shared frame cache across rows |
| `quillon preview <doc>` | Localhost filmstrip of the draft, reloads on save |
| `quillon probe <file>` | Duration, resolution, fps, codecs of a media file |
| `quillon inspect <doc>` | Scenes, timing, assets, audio mix summary |
| `quillon caption import subs.srt --doc doc.json` | Attach subtitles as a burn-in caption track (SRT/VTT/ASS in) |
| `quillon caption export doc.json -o out.vtt` | Write a caption track back out (SRT/VTT) |
| `quillon caption transcribe audio.wav --doc doc.json` | Whisper transcription into a caption track (optional `transcribe` build feature) |
| `quillon audio analyze <doc>` | Integrated loudness (LUFS), true peak, silence per scene |
| `quillon otio import timeline.otio -o doc.json` | Import an OpenTimelineIO subset timeline |
| `quillon mcp` | The agent server: 9 tools (validate/preview/render/diff/probe/…) + the document schema over stdio (register: `claude mcp add --scope user quillon -- quillon mcp --root <dir>`) |

Every command takes `--json` for machine-readable output. The
exit-code contract is locked by tests: `0` success, `1` operational
failure, `2` invalid document, `3` lint findings.

## For AI agents

The JSON Schema (`docs/schema.json`) is the contract: generate a
document, validate, iterate on JSON-pointer errors, render. The MCP
server exposes exactly that loop plus preview and diff tools — same
engine as the CLI, one binary. Start with
[AGENTS.md](AGENTS.md) and [`skills/quillon/SKILL.md`](skills/quillon/SKILL.md).

## Documentation

- [Guide](docs/guide.md) — concepts: documents, validation, rendering, cache, template data, OTIO, captions/audio
- [Template gallery](docs/gallery.md) — five documents across the feature lanes, rendered by `cargo xtask gallery`
- [Benchmarks](docs/benchmarks.md) — honest corpus numbers, rerunnable via `cargo xtask bench`
- [Stability policy](docs/STABILITY.md) — what is frozen, what may move during beta
- [Hardening](docs/HARDENING.md) — the guards, the limits, how to add one
- [Schema](docs/schema.json) — the machine-readable document contract

## Development

```
cargo build && cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
cargo xtask export-schema   # regenerate docs/schema.json
cargo xtask gallery         # regenerate docs/gallery-assets/*.png
cargo xtask bench           # regenerate docs/benchmarks.md
```

License: MIT OR Apache-2.0 ([NOTICE](NOTICE) for the runtime
dependency boundary). CI runs the full battery on Linux, macOS, and
Windows.
