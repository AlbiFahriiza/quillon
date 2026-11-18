# G. Schema & Cache Design Notes — for Quillon

*Research date: 2026-11-17. Direct reads by the lead agent.*

## 1. Content-addressed cache-key design (source: mattpocock/course-video-manager issue #1574, https://github.com/mattpocock/course-video-manager/issues/1574)

- **Key formula**: `namespace + hash(all output-determining inputs) + version constant for non-input changes`.
- The hashed field set *defines the equivalence class* — anything outside it will not dedup. Content-addressed naming gives idempotency for free (same input → one render; orphans are garbage-collected, never invalidated).
- **Two independent invalidation axes**: content (hash of semantic inputs) and renderer version (a constant bumped when rendering logic changes with identical inputs). Per-layer version constants keep invalidation local — bumping the overlay renderer does not nuke the export-level cache, and vice versa.
- Tradeoffs to avoid: coarse granularity (whole-file caching only), blunt global version keys (invalidate everything on any renderer change), namespace-scoped dedup that blocks cross-project reuse.
- **Quillon application**: frame cache key = `hash(document-subtree relevant to frame) + renderer version + asset content hash`. Per-frame keys give natural incremental re-render: edit scene 3 → only frames whose key changed re-render. Asset hash (sha256 of source file) means touching a source video invalidates only frames that use it.

## 2. Editly spec — what to keep and what to fix (source: https://github.com/mifi/editly/blob/master/examples/videos.json5)

- Top-level: `width`, `height`, `outPath`, `defaults` (global transition + font defaults), `clips[]`. Each clip has `layers[]` stacked bottom-to-top; video layers carry `path`, `cutFrom`/`cutTo` (trim in seconds), `resizeMode` (`contain`, `contain-blur`, `stretch`, `cover`). Title layers support `\n` multiline.
- **Keep**: clips→layers mental model (maps cleanly to scenes→elements); global `defaults` block (cuts document size for template families); fit modes as a closed enum; trim expressed as source-range on the layer.
- **Fix**: seconds as floats (Quillon uses flicks everywhere — exact); no explicit audio model in the spec (Quillon: audio as first-class tracks with gain/fades); transitions bolted on per-clip (Quillon: transition is a property of the scene boundary, rendered deterministically); JSON5 parsing quirks (Quillon: strict JSON for interchange; comments live in a sidecar `.jsonc` authoring format converted on save).

## 3. Schema-error design lessons (sources: research/C §2–3, research/D §2)

- Validation errors must be **JSON-path-qualified and actionable** (`elements.0.layer: Required` — Clipkit's format). This single property is credited with the 100% vs 87% agent-validity gap vs code-first tools.
- Separate **structural validation** (jsonschema: shape, types, ranges) from **semantic lint** (check_document-style: contrast, off-canvas, read-time at 300 wpm, overlaps) — different costs, different outputs, both machine-readable.
- Lint rules exit nonzero on genuine problems (CI-gateable) but return findings as data, so agents can fix without human reading.
- Canonicalization: same semantic document → same canonical JSON bytes (ordered keys, no whitespace noise) — makes git diffs and agent edit-verification meaningful.

## 4. Video source editing — the decode surface (synthesis from research/B + ffmpeg docs)

- Decoding arbitrary input codecs (H.264/H.265/AV1/VP9 + AAC/Opus/MP3 containers) is a solved problem **only via libav** (ffmpeg-next 9.1.0). Pure-Rust decode exists for subsets (Wedeo H.264 ~30k LoC) but is not production-grade.
- Seek accuracy: for frame-accurate cuts, decode from the previous keyframe to the exact tick — the document's `source_range` must be tick-exact, and the decoder adapter must expose "nearest keyframe ≤ T" semantics.
- Resize modes (`contain`, `cover`, `stretch`, `contain-blur`) are a closed enum over the compositor, independent of decode.
- Audio for edited clips: decode via symphonia where codec allows, libav otherwise; resample to the document's audio rate (rubato), mix in tick domain.

## 5. Preview loop design (synthesis from research/D §1, §4)

- `quillon preview`: local HTTP server (tokio + notify file-watch) serving a filmstrip page; re-render only frames whose cache keys changed on document save; page auto-reloads.
- Draft renders: half-resolution, single-scene, skip audio — target <1s (fframes `render --draft` precedent: ~1s).
- The preview loop and the final render share the exact same `render_tick(tick) -> RGBA` path — different only in resolution and encoder (same-engine guarantee kills the "preview ≠ export" failure class).
