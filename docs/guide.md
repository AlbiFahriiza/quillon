# Quillon guide

Everything here describes what the binary actually does — commands
shown are real and tested by the CI battery. When in doubt, the
contract is [`docs/schema.json`](schema.json) and `quillon --help`.

## The document

A Quillon document is one JSON file: a canvas (`size`, `fps`, `bg`),
`assets` (fonts, images, audio, video), `scenes` holding `elements`
(text, rect, image, video), optional `captions`, and optional
`variables` for mail-merge.

- **Time is ticks.** One second is 705,600,000 ticks — divisible by
  every common frame rate, so cuts land exactly on frames. Scene
  `duration` and every animation key time are ticks.
- **Assets are relative to the document.** `src` paths resolve against
  the document's own directory and are confined there: a document
  cannot read files outside it.
- **Elements stack bottom-to-top** in scene order. Text needs a font
  asset key; rects can carry gradients, corner radius, shadows, and
  clipping; image/video elements take `fit` (`cover`, `contain`,
  `fill`).
- **Motion is keyframe tracks.** `anim` on an element animates
  `opacity`, `translate_x`, `translate_y` between keys, each key with
  its own easing (`out_cubic`, `out_back`, ...). Scenes compose with
  cuts or `crossfade` transitions via `transition_in`.

Validate before anything else:

```
quillon validate doc.json
```

Errors carry a JSON pointer to the offending node, a code, and a
human message — the same shape `--json` emits for agents. Exit code
`2` means the document is invalid; `3` means it is valid but the
linter has findings (low contrast, off-canvas elements, text past the
edge, unreadable scenes); `--allow-lint` accepts the findings.

## Rendering

```
quillon render doc.json -o out.mp4            # MP4 via system ffmpeg
quillon render doc.json -o frames/            # PNG frames + manifest.json
```

- **Deterministic frames.** The same document + version + CPU render
  path produces byte-identical frames; a per-frame sha256 manifest is
  written next to the output. The golden-hash suite in CI pins this.
- **Parallel workers** (`--workers N`, default one per core) stream
  frames straight into the encoder, so memory stays bounded on long
  videos.
- **Content-addressed cache** (`--cache-dir dir`): frames are keyed by
  their scene subtree, so re-rendering after an edit computes only
  frames whose content changed. Edit one scene, re-render that scene.
- **Audio**: the mix can be normalized to integrated loudness with
  `--lufs` (broadcast preset: `-23`); `quillon audio analyze` reports
  LUFS, true peak, and per-scene silence without rendering.
- **Encoders**: `--encode software` (libx264, default),
  `--encode auto` (hardware VAAPI/NVENC when available, with an
  explicit fallback), or any ffmpeg encoder by name. Encoded bytes are
  not part of the determinism contract — frames are.

## Mail-merge batches

Declare `variables` in the document, fill them per CSV row:

```
quillon render template.json --data rows.csv -o 'out/{{name}}.mp4'
```

Every row is substituted, validated as a full document, and rendered;
identical static frames hit the shared cache across all rows. Types
are checked at substitution (`string`, `number`, `color`, `asset`).
Row values filling the output pattern cannot escape the output
directory (path separators and `..` are named errors); CSV inputs are
bounded (1 MiB per field, one million rows) with named errors past
the limits.

## OpenTimelineIO import

```
quillon otio import timeline.otio -o doc.json
```

A subset timeline (stacks, video tracks, clips, external references,
source ranges) imports with exact frame math; rates that do not divide
the timebase are rejected with the nearest legal fps. `subset_warnings`
reports what the subset does not cover.

## Captions and transcription

```
quillon caption import subs.srt --doc doc.json -o captioned.json
quillon render captioned.json -o out.mp4     # burn-in at render time
quillon caption export captioned.json -o out.vtt
```

SRT/VTT/ASS import attaches a caption track (one style, timed cues,
sorted and non-overlapping — enforced by validation). Timing
regularization fixes overlapping or too-short cues on import;
`--no-regularize` keeps source timing exactly. With the optional
`transcribe` build feature, `quillon caption transcribe audio.wav
--doc doc.json` runs whisper into a regularized caption track.

## The agent loop

Agents work cheapest-first: draft JSON → `validate` (its verdict is a
few hundred bytes, sub-millisecond) → fix pointer errors → `preview`
frames → `render`. The MCP server (`quillon mcp`) serves nine tools
over stdio — `validate_document`, `preview_frames`, `diff_documents`,
`list_assets`, `list_fonts`, `probe_media`, `render_frame`,
`render_document`, `render_status` — plus the schema as a resource.
Register it:

```
claude mcp add --scope user quillon -- quillon mcp --root <dir>
```

## Guardrails

Inputs that a naive pipeline would crash on are named errors instead:
corrupt cache entries become cache misses, absurd OTIO durations are
rejected rather than saturated, every limit error names the limit and
the actual value. The full list and how to extend it:
[`HARDENING.md`](HARDENING.md).
