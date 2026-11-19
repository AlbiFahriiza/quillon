---
name: quillon
description: Create and edit video programmatically with Quillon (video as code). Use when the user wants to make a video, motion graphics, a slideshow, captions or edit/trim/concat existing footage from code or via an AI agent. Covers the document JSON format, the cheapest-first agent loop (validate → preview → render), the MCP tool surface, and reproducible frame hashes for self-verification.
---

# Quillon — Video as Code

Quillon renders video from a JSON document: scenes hold elements
(text, rect, image, video), time is integer ticks (705,600,000 per
second), and the same document always produces the same frames. Work
through the MCP server (`quillon mcp`) or the CLI (`quillon validate /
render / probe / inspect`).

## The loop: cheapest first

Never render pixels to answer a question the validator can answer.

1. **Read the schema once**: resource `quillon://schema/document`.
2. **validate_document** — schema + semantics + lint, ~20-150 tokens,
   no pixels. Errors are JSON-pointer paths (`/scenes/0/elements/3`)
   plus a message; fix what they point at. Lint findings (contrast,
   read time, off-canvas) are warnings, not errors.
3. **preview_frames** — draft half-resolution PNGs at chosen ticks
   (~390 tokens/frame). Look before you commit.
4. **render_document** — final MP4/PNG. Returns a `run_id` and the
   estimated frame count immediately; poll **render_status** until
   `done`, then read the manifest (every frame's sha256).

Frame hashes are reproducible: the same document + tick always hashes
the same. To check your edit did not move a frame you cared about,
compare sha256 strings — zero new renders.

## Tools

| Tool | Use |
|------|-----|
| `validate_document` | Schema + semantic errors with pointers, lint findings |
| `preview_frames` | Draft PNGs at up to 16 ticks |
| `diff_documents` | Semantic diff of two documents (JSON-pointer paths) |
| `list_assets` / `list_fonts` | What media exists in the working root |
| `probe_media` | Duration / resolution / fps / codecs of a file |
| `render_frame` | One full-resolution frame + sha256 (visual regression) |
| `render_document` | Start a render; returns `run_id` + estimated frames |
| `render_status` | Poll: `running` / `done` + manifest / `error` |

Register the server (stdio): `claude mcp add --scope user quillon -- quillon mcp --root <project-dir>`

## Minimal document

```json
{
  "v": 1,
  "size": { "w": 1920, "h": 1080 },
  "fps": 30,
  "scenes": [
    {
      "id": "title",
      "duration": 705600000,
      "elements": [
        { "type": "text", "text": "Hello", "font": "body", "x": 960,
          "y": 540, "size": 120, "color": "#ffffff" }
      ]
    }
  ]
}
```

Rules that bite most often: scenes need at least 0.8 s (read-time
lint); assets must exist and be referenced by id; video elements trim
with `source_range` in ticks; audio tracks mix with `gain` and
`fades` (LUFS target −23 is the EBU R128 preset on render). The full
schema is the `quillon://schema/document` resource — trust it, not
your memory of this file.

## Captions (v0.5)

Caption tracks live at the document level (`"captions": [{ "style":
{...}, "cues": [...] }]`, next to `"audio"`): cue timing is
document-global ticks and may cross scenes; cues burn in over the
finished frame with the track's style (font, size, `position` bottom/
center/top, `margin`, optional `shadow` and `bg` box). Fastest path
from existing subtitles — the CLI regularizes timing (min 1 s, max
6 s, ≤18 chars/s, tiny-gap merge; `--no-regularize` keeps source
timing exactly) and clamps cues to the document duration:

```
quillon caption import subs.srt --doc video.json -o video.cap.json
quillon caption import subs.vtt --doc video.json   # .ass/.ssa too
quillon caption export video.cap.json --format srt
```

Then validate + render as usual. `quillon audio analyze video.json
--json` reports per-scene integrated LUFS, true peak (dBTP) and
silence ranges of the mix. Whisper transcription
(`quillon caption transcribe audio.wav --doc video.json`) exists only
in `--features transcribe` builds; model weights download on first
use.

## Errors

Every tool error names the step that failed and, for document
problems, points into the document. `is_error: true` with a
`/path/like/this` means fix the document; everything else is an
operational message. Exit codes (CLI): `2` invalid document, `3` lint
findings, `0` ok, `1` operational failure.
