# D. Deep-Dive Research Notes — for Quillon (VIDEO AS CODE)

*Research date: 2026-11-17. Direct source reads by the lead agent (not subagent claims). Each section carries its source.*

## 1. vidformer — instant playback architecture (source: https://github.com/ixlab/vidformer, Apache-2.0; paper arXiv 2601.17221)

- **Declarative lifting**: a drop-in import shim (`import vidformer.cv2 as cv2`) substitutes **symbolic frames** for real pixel buffers. Drawing calls don't execute — the shim records a declarative specification of each output frame instead of rendering it. Control flow runs eagerly (Python), rendering is fully deferred.
- **Optimizing executor**: because the complete spec exists before any pixel work, the Rust engine optimizes and parallelizes "just like a relational database optimizes relational queries" — whole-pipeline optimization, not per-call.
- **Formal concurrency model**: the repo ships a **TLA+ model** of the rendering engine (`misc/tla`). Concurrency correctness was specified formally.
- **Just-in-time segment serving**: output served as standard **HLS VOD**; each segment rendered just in time, so playback latency is independent of video length. 0.25–0.5s time-to-playback (~400× vs waiting for a render). Full renders 2–3× faster than cv2.
- **Storage abstraction**: source access via Apache **OpenDAL** — input videos can live in arbitrary storage backends (s3, local, http...).
- **Transferable pattern for Quillon**: three-layer split — symbolic/declarative spec layer (record intent, don't render) → optimizing Rust executor (global knowledge = global optimization) → just-in-time segment server (latency independent of length).

## 2. Kineto — document format + determinism (source: https://github.com/DanielFidalgo/kineto, MIT OR Apache-2.0)

- **Document structure**: `v` (version), `timebase: 705600000` (flicks), `size {w,h}`, `bg`, `assets` (font map), `scenes[]` → `id`, `duration` (ticks), `elements[]`. Five element types: `image`, `text`, `rect`, `path`, `group`. Gradients, corner radius, shadows, clip windows, image fit modes.
- **Animation**: only translate/scale/rotation/opacity animate. Keyframes: `{prop, keys: [{t, v, ease}]}`, ten easing curves. Example tick value: 176400000 = 0.25s.
- **Flicks timebase**: integer ticks at 705,600,000/s — 24/25/30/50/60 fps all divide it exactly; fps becomes "an export hint rather than a commitment"; no float drift.
- **check_document lint** (~20 tokens, no images): text invisible against background (contrast), element animated off-canvas, text past the edge, scene too short to read at 300 wpm. Exit nonzero on real problems. Catches what is "invisible in the JSON and obvious on screen."
- **Reproducibility**: tiny-skia CPU rasterization; golden sha256 hashes of frame buffers in `testdata/golden/hashes.json`; native-vs-wasm byte-identical frame parity, 27/27 corpus, CI-enforced. Scope honesty: "the frames are byte-identical. The MP4 container is not. Never promise reproducible MP4 bytes; promise reproducible pixels."
- **Layering rule (their repo)**: `crates/core` depends on nothing; everything else depends on it, never the reverse.
- **CI**: GitHub Action with `check` default-on (unreadable text fails CI); changelog videos composed from git history; every release's page carries a video rendered by the version being released.
- **build_chart design lesson**: charts are NOT an element type — `build_chart` emits ordinary paths/rects/text, measured margins, so output stays an editable document.

## 3. Treza — MCP video editing patterns (source: https://www.trezalabs.com/blog/mcp-tools-for-video-editing)

- **Core philosophy**: editing over MCP fails when it mimics a human editor — "There is no playhead to drag over a protocol connection and no viewport to scrub." Agents run edits humans already specified; the verbs live in pipelines.
- **~25 tools mapped to production lifecycle**: `list_pipelines`, `create_pipeline`, `list_node_types`/`get_node_type` (learn the graph vocabulary), `estimate_run_cost`, `run_pipeline`, `get_run`, `list_assets`, `import_media`, `edit_asset`, `assemble_video`.
- **Two-call async pattern**: `run_pipeline` returns a run id immediately; agent polls `get_run` until status leaves running. Per-node results — "a failed caption burn reports as a failed caption burn rather than as a missing file."
- **Agent failure modes**: (1) blind visual judgment (picking caption Y without seeing the frame), (2) homeless state (stateless tools force rebuilding the whole decision list every step), (3) silent compounding (forty small calls = forty chances to be slightly wrong), (4) success ≠ quality (a run succeeding says nothing about pacing or caption collisions).
- **Agent-friendly API rules**: separate start-render from read-result; agent can author new chains, not only trigger canned pipelines; failed runs identify the failed step; price before spending; ~a dozen producer-recognizable actions, not a hundred rendering-API parameters; deterministic repeatability (run 100 = run 1).
- **Proof chain** (podcast clip): transcribe → LLM picks strongest 30–60s moment → extract + reframe 9:16 → captions from same transcript → badge overlay → publish. One run_pipeline call.

## 4. fframes — pure-function frames + machine-readable CLI (source: https://ai.satyajit.si/articles/fframes)

- **A frame is a function**: `Video` trait — `render_frame(&self, frame, ctx) -> Svgr`. Frame *i* never depends on *i−1*; no retained scene graph, no mutable state between frames. Deterministic, diffable, snapshot-testable — same index always paints identical bytes.
- **svgr! macro + static-hash caching**: at compile time the macro assigns a `static_hash` to every node whose lexical content is fully known; runtime `RenderCache` keyed by hash holds converted paths/paints/recorded `Picture`s — "replaying a picture is dramatically cheaper than re-traversing the subtree." A cheap runtime fingerprint guards against inherited-color changes (dynamic parent "poisons" descendants).
- **timeline! macro**: `frame.animate(timeline!(...))` — a pure function of `frame.index` returning interpolated transforms. `at 0., animate Transform::translate(0,0) => Transform::translate(200,480), Easing::Linear`.
- **Benchmark honesty**: the 31.16× figure is one synthetic scene (99k one-pixel rects + 1k changing digits, 30 frames) — CPU vs CPU (GPU run printed "skipped"), worst-case for React reconciliation, near-best-case for cache-and-raster. "A floor for one synthetic scene, not a general 31×."
- **GPU story**: Skia on Metal/Vulkan, ~10× over CPU (reported). No custom GPU kernels — hand the tree to a mature rasterizer. libav statically linked (no PNG encode-then-decode handoff, no process boundary per frame).
- **The agent thesis**: "an LLM that reads rather than watches" — CLI turns video into machine-readable artifacts: `inspect` (samples every 0.25s + scene boundaries, missing fonts, clipped text, panics with timestamps, exit code 2), `strip` (contact sheets + onion blends revealing motion paths), `frame` (full-size PNGs), `audio analyze` (LUFS, true peak, clipping, silence per scene), `snapshot` (visual regression with `.diff.png`), `render --draft` (half-size single scene in ~1s).

## 5. OpenTimelineIO in Rust (source: web search results)

- Official Rust bindings: **placeholder only**. `opentimelineio` crate on crates.io is a placeholder (https://crates.io/crates/opentimelineio/0.1.0); vfx-rs working group exists (https://github.com/vfx-rs/opentimelineio-bind, Linux-only, early). No mature pure-Rust OTIO library.
- **Implication for Quillon**: OTIO import/export must either (a) bind the C++ lib (heavy), (b) parse the OTIO JSON directly (OTIO is JSON-serializable — feasible for the subset we need: Timeline → Stack → Track → Clip with source ranges), or (c) defer. Recommendation: (b) — read/write OTIO JSON subset in pure Rust, declared as v0.x "best effort, subset".

## 6. Synthesis — the five load-bearing design decisions for Quillon

1. **Document-first, code-second**: JSON document (schemars-schema-validated) is the interchange format for agents and humans; Rhai scripting later as pure `t → Element[]` generators with output re-validated against the same schema. Evidence: Clipkit 100% vs Remotion 87% agent validity; ~1.6× token savings.
2. **Frames are pure functions of tick**: `render_tick(tick) -> RGBA` deterministic, no inter-frame state. Evidence: fframes (snapshot testing, static-hash caching), Kineto (golden hashes, cross-target parity).
3. **Flicks integer timebase** (705,600,000/s): exact frame math at all common fps. Evidence: Kineto.
4. **Cheapest-first agent loop**: validate (~20 tok) → preview frame (~390 tok) → render (seconds). Async two-call render pattern with run ids. Evidence: Kineto token costs, Treza run_pipeline/get_run.
5. **Machine-readable everything**: every CLI command emits JSON (inspect, probe, audio analyze, snapshot diffs); MCP server + JSON Schema as resources; determinism makes agent self-verification a hash compare. Evidence: fframes CLI design, Kineto MCP resources.
