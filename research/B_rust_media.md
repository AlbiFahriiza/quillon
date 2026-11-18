# B. Rust Media Ecosystem Map — for Quillon (VIDEO AS CODE)

Research date: 2026-11-17. Delegated to Explore subagent (gated: see .disciplined/delegation.json). Every crate entry below was verified against the crates.io API (`https://crates.io/api/v1/crates/NAME`) or docs.rs during this session. Download counts are total lifetime; "recent" = recent downloads from the API. Non-existent crates are explicitly marked.

## 1. Video Encode/Decode

| Crate | Latest ver (rel. date) | Downloads (total / recent) | License | Verdict |
|---|---|---|---|---|
| rav1e | 0.8.1 (2025-06-16) | 61.8M / 21.5M | BSD-2-Clause | Viable, slow-moving |
| ffmpeg-next | 9.1.0 (2026-11-11) | 9.05M / 2.6M | WTFPL | Actively maintained |
| ffmpeg-sys-next | 9.1.1 (2026-11-11) | 9.4M / 2.7M | WTFPL | Active (FFI layer) |
| ffmpeg-the-third | 6.0.0+ffmpeg-9.0 (2026-08-09) | 165K / 59K | WTFPL | Maintained fork |
| ffmpeg-sidecar | 2.7.0 (2026-10-18) | 2.3M / 663K | MIT | Active; wraps ffmpeg binary |
| openh264 | 0.9.10 (2026-11-10) | 2.3M / 1.6M | BSD-2-Clause | Active; H.264 encode/decode |
| x264 | 0.5.0 (2022-12-04) | 180K / 80K | MIT | Stale ~4 yrs |
| x264-next | 0.4.2 (2022-11-08) | 2.8K / 4 | MIT | Deprecated ("in favor of the x264 crate") |
| x265 | 0.1.1 (2023-01-07) | 2.2K / 4 | MIT | Dead |
| vpx | 0.3.1 (2015-08-02) | 13.9K / 370 | MPL-2.0 | Dead (10 yrs) |
| hwencoder | — | — | — | DOES NOT EXIST on crates.io |
| gpu-video | 0.5.0 (2026-11-10) | 6.2K | MIT | Vulkan Video via wgpu, early |
| vk-video | 0.3.1 (2026-04-27) | 3.5K | MIT | DEPRECATED, renamed to gpu-video |
| pixelforge | 0.12.1 (2026-11-15) | 247 | BSD-2-Clause | Vulkan H.264/H.265/AV1, very early |
| gstreamer / gstreamer-video | 0.25.4 (2026-09-21) | 13.3M / 8.8M | MIT OR Apache-2.0 | Active; requires GStreamer runtime |

Key findings:

- **rav1e** is real and huge (61.8M downloads) but releases slowly — 0.8.1 is the newest (June 2025). Fine as an AV1 encode option; not a general solution.
- **Pure-Rust H.264/HEVC encode does not exist in usable form.** The x264 crate hasn't released since 2022; x265 is a 2K-download toy; `hwencoder` doesn't exist. openh264 (BSD, active, 1.6M recent downloads) is the only maintained in-process H.264 option, but OpenH264's encoder quality/perf is below x264.
- **ffmpeg-next** released 9.1.0 six days before this research (2026-11-11) — this is the actively maintained libav binding and what most of the ecosystem builds on. License is WTFPL (fine for use; note for compliance review).
- **ffmpeg-sidecar** (MIT, 663K recent downloads) is the "shell out to a system ffmpeg binary over pipes" approach — zero build pain, full codec coverage, at the cost of a runtime dependency.

## 2. Container Muxing

| Crate | Latest ver (rel. date) | Downloads (total / recent) | License | Verdict |
|---|---|---|---|---|
| mp4 | 0.14.0 (2023-08-01) | 16.0M / 3.8M | MIT | Stale (3+ yrs no release) but hugely used (alfg/mp4-rust) |
| matroska | 0.30.1 (2026-04-15) | 349K / 68K | MIT/Apache-2.0 | Active but **metadata parsing only** — not a muxer |
| webm | 2.2.1 (2026-08-25) | 594K / 16K | MPL-2.0 | Active-ish; muxes VP8/VP9/AV1+Opus/Vorbis into WebM |
| mux | 0.1.1 (2016-04-21) | 7.2K | MIT | Dead (9 yrs) |
| mkv | 0.0.8 (2021-08-31) | 14K | — | Stale |
| matroska-demux, mkv-demuxer | — | — | — | DO NOT EXIST on crates.io |

Finding: there is **no maintained pure-Rust MP4 muxer**; `mp4` 0.14.0 (16M downloads) is the de-facto standard and last shipped Aug 2023 — it works (used by live-streaming stacks) but receives minimal maintenance. The realistic muxing story is: let libav (ffmpeg-next) or the ffmpeg binary do muxing; treat pure-Rust muxing as a later optimization.

## 3. Compositing / 2D Rendering

| Crate | Latest ver (rel. date) | Downloads (total / recent) | License | CPU/GPU | Verdict |
|---|---|---|---|---|---|
| image | 0.25.10 (2026-03-10) | 254.9M / 83.9M | MIT OR Apache-2.0 | CPU | Foundation; decode/encode raster |
| tiny-skia | 0.12.0 (2026-02-02) | 71.9M / 29.5M | BSD-3-Clause | CPU | Mature, linebender, excellent |
| skia-safe | 0.157.0 (2026-11-03) | 5.3M / 1.9M | MIT | CPU+GPU | Very active; binds C++ Skia (large build) |
| vello | 0.12.0 (2026-11-11) | 1.2M / 488K | Apache-2.0 OR MIT | GPU (wgpu compute) | Active, still pre-stable |
| wgpu | 30.0.2 (2026-11-12) | 51.5M / 19.7M | MIT OR Apache-2.0 | GPU | Extremely active standard |
| piet | 0.8.0 (2025-08-29) | 2.2M / 464K | Apache-2.0 OR MIT | abstraction | Low activity; backend-agnostic layer |
| glyphon | 0.12.0 (2026-07-09) | 1.7M | — | GPU (wgpu) | wgpu text renderer, niche |

Finding: the practical choice is **tiny-skia (CPU, deterministic, no GPU dependency, trivially parallelizable per frame with rayon) vs skia-safe (what fframes and OpenCat actually use for GPU rendering)**. vello is the promising pure-Rust GPU future but not yet at 1.0 maturity. For a headless video renderer, CPU tiny-skia is the safe default; skia-safe is the proven "pro" path (fframes runs it on Metal/Vulkan).

## 4. Text / Font Rendering

| Crate | Latest ver (rel. date) | Downloads (total / recent) | License | Role |
|---|---|---|---|---|
| ttf-parser | 0.25.1 (2024-11-29) | 140.0M / 42.9M | MIT OR Apache-2.0 | Font parsing primitive |
| ab_glyph | 0.2.32 (2025-09-28) | 55.0M / 15.7M | Apache-2.0 | Pure-Rust rasterizer + layout |
| rustybuzz | 0.20.1 (2024-11-12) | 41.4M / 14.5M | MIT | HarfBuzz shaping, ported to Rust |
| fontdb | 0.24.0 (2026-07-29) | 47.1M / 17.9M | MIT | System font discovery/DB |
| cosmic-text | 0.19.0 (2026-04-22) | 12.6M / 5.2M | MIT OR Apache-2.0 | Full stack: rustybuzz shaping + fontdb + layout + fallback |

Finding: **cosmic-text is the assembled answer** — shaping via rustybuzz, font discovery via fontdb, font fallback, and line layout in one crate. It is what a video tool should use for burned-in titles/captions: render its glyph layouts onto the compositing surface. rustybuzz itself hasn't released since Nov 2024 but is complete and stable. fframes inherits text from Skia — viable; cosmic-text is the pure-Rust, no-C++ path.

## 5. Audio

| Crate | Latest ver (rel. date) | Downloads (total / recent) | License | Role |
|---|---|---|---|---|
| symphonia | 0.6.1 (2026-08-13) | 22.2M / 11.6M | MPL-2.0 | Pure-Rust decode (WAV/MP3/FLAC/OGG/AAC...) |
| rubato | 5.1.0 (2026-11-12) | 15.8M / 6.3M | MIT OR Apache-2.0 | Resampling, very active |
| cpal | 0.18.2 (2026-08-16) | 29.4M / 11.1M | Apache-2.0 | Audio output (playback only) |
| rodio | 0.23.0 (2026-10-11) | 14.3M / 4.1M | MIT OR Apache-2.0 | Playback on top of cpal |

Finding: for an offline renderer, the story is **symphonia (decode) + rubato (resample to a common rate) + custom mixing math (sum + gain + fades) — all pure Rust**. cpal/rodio are for preview playback only, not needed for render. AAC-encode for MP4 output remains a gap in pure Rust — plan on ffmpeg/libav or opus-in-webm.

## 6. Existing Rust Video Projects (what to learn from)

- **fframes — github.com/dmtrKovalenko/fframes**: 3,128 stars, pushed 2026-11-15, MIT. "Programmatic video rendering framework that is actually fast" — the closest existing thing to Quillon's ambition. Rust-native DSL: `svgr!` macro composes each frame as an SVG tree, `timeline!` macro does keyframes/easing; Skia GPU renderer (Metal/Vulkan), lyon tessellation, three-d for 3D, Shadertoy-style shaders. Media via statically-linked FFmpeg (`ffmpeg-sys-fframes`, WTFPL) with x264/x265/vpx/opus opt-in features. Claims 30x faster than Remotion. Crates on crates.io: fframes 1.4.2-rc.11 (2026-11-15, 53.9K downloads), fframes_skia_renderer (24.4K), svgr-macro (56.2K). Agent-oriented distribution (`npx skills add`). **This is the benchmark competitor.**
- **FilmCraft — github.com/storytold/filmcraft**: 18.5k stars, 4,143 commits, MIT/Apache-2.0, self-described alpha. Pure-Rust NLE: no FFmpeg at all — own codec implementations, own MP4/MOV/MKV/WebM muxing, symphonia for audio, wgpu compositor, egui UI, EBU R128 loudness. Proof that full-Rust video is possible at huge engineering cost. Exposes 650+ commands for AI agents via CLI/JSON/MCP.
- **vidformer — github.com/ixlab/vidformer**: 71 stars, Apache-2.0, Rust core + Python frontend (drop-in `cv2` replacement). Academic (ACM MM '26, arXiv 2601.17221, OSU IX Lab). Declarative frame-spec recording + lazy/parallel render + on-demand streaming; 2-3x faster than cv2, 0.25-0.5s playback start. **The best research reference for Quillon's instant-preview architecture.**
- **video-rs — github.com/oddity-ai/video-rs**: 429 stars; crate 0.12.0 (2026-09-09), 507K downloads, MIT/Apache-2.0. ffmpeg-next-based readers/writers/encoders with ndarray frames. Maintainers building sibling "rave" to drop the ffmpeg dependency.
- **ReelForge — github.com/sergii-ziborov/ReelForge**: v0.2.0, MIT. Fluent clip-graph API, JSON render plans, SRT/WebVTT/ASS burn-in, NVENC/QSV/AMF helpers — **shells out to ffmpeg/ffprobe on PATH** (explicitly does not link libav). A direct feature checklist for Quillon.
- **OpenCat — github.com/ZhouXiaolin/opencat**: 72 stars, pushed 2026-11-15. JSONL/XML declarative compositions, Tailwind-style layouts, Skia GPU rendering, "built for agents".
- **gpu-video — github.com/AdrianEddy/gpu-video**: crate 0.5.0 (2026-11-10), 6.2K downloads, MIT. Vulkan Video decode/encode with wgpu integration — early but exactly the hardware-codec gap in Rust.
- Also exists: awesome-video-as-code curated list (github.com/voidforall/awesome-video-as-code, updated Nov 2026).

## 7. CLI + Async (confirmed standards)

- **clap 4.6.8** — released 2026-11-16, 1.35B downloads, 292.8M recent, MIT OR Apache-2.0. Standard, extremely active.
- **tokio 1.54.1** — released 2026-10-21, 1.21B downloads, 313.6M recent, MIT. Standard. Note: a render pipeline is mostly CPU (rayon) or GPU work; tokio is for the preview server / file watching / IPC, not frame loops.

## 8. Scripting Layer for the Video DSL

| Option | Latest ver (rel.) | Downloads (total/recent) | Startup cost | Sandboxability | Hot reload |
|---|---|---|---|---|---|
| rhai | 1.27.0 (2026-10-21) | 15.0M / 4.7M | Milliseconds; interpreter, no FFI | Built for it — no I/O unless you register functions; pure Rust, memory-safe | First-class: documented pattern (rhai.rs/book/patterns/hot-reload.html) — re-entrant engine, swap `Rc<RefCell<AST>>`, full reload or function-level hot-patch |
| mlua (Lua 5.4/LuaJIT/Luau) | 0.12.2 (2026-10-03) | 9.0M / 2.9M | Tiny (LuaJIT/Luau are small C VMs) | `Lua::new()` loads only a safe stdlib subset; Luau mode adds `Lua::sandbox()`; C FFI surface requires care | Simple: reload chunk in fresh VM; state loss is the usual caveat |
| deno_core (V8 JS/TS) | 0.413.0 (2026-10-21) | 10.4M / 3.3M | Heavy — V8 is tens of MB and slow to init; snapshots mitigate | Excellent (V8 isolate model, permission capabilities) | Excellent for JS devs; huge binary + build complexity, churns fast (0.x versioning) |
| rquickjs (QuickJS) | 0.14.0 (2026-09-18) | 7.6M / 4.0M | Low (QuickJS is ~KBs, fast init) | Good (no stdlib by default, interpreter-only) | Re-eval in fresh context; simple |
| rlua | 0.20.1 (2024-07-20) | 814K / 42K | — | — | Superseded — mlua is its successor; don't adopt |
| Rust-native macro DSL | — | — | None (compile-time) | Perfect (it's your code) | `cargo watch` recompile ~seconds; no runtime reload — this is fframes' model |

Recommendation: **rhai** for the hot-reloadable composition DSL (pure Rust, purpose-built sandboxing, documented hot-reload, one dep — matches Quillon's "video as code + live preview" loop), with a JSON/serde render-plan as the interchange format. Offer a Rust-native macro front-end later for power users (fframes model). deno_core only if browser-compatible JS becomes a hard requirement — its startup weight fights the sub-second-preview goal.

## 9. Build-vs-Buy Matrix for Quillon

| Capability | Recommendation | Choice |
|---|---|---|
| H.264/HEVC encode, any-codec decode, muxing | BUY (libav) | ffmpeg-next 9.1.0 (active) as engine; ffmpeg-sidecar as fallback/shell mode |
| AV1 encode (opt-in) | BUY | rav1e 0.8.1 |
| H.264 in-process, no-libav profile | BUY | openh264 0.9.10 |
| MP4/WebM muxing | BUY (via libav) | mp4 0.14.0 stale; webm 2.2.1 for WebM-only; long-term: fork or libav-only muxing |
| 2D compositing | BUY | tiny-skia 0.12.0 (CPU default); skia-safe later for GPU parity with fframes |
| Raster I/O (PNG/JPEG frames, textures) | BUY | image 0.25.10 |
| Text shaping/layout/burn-in | BUY | cosmic-text 0.19.0 (+ its rustybuzz/fontdb) |
| Audio decode | BUY | symphonia 0.6.1 |
| Audio resample | BUY | rubato 5.1.0 |
| Audio mixing, gain, fades, timeline math | BUILD | trivial DSP, own the semantics |
| Frame-accurate timeline/clip graph, keyframes, easing | BUILD | this is the product (learn from fframes timeline!, ReelForge clip graph, vidformer spec-lifting) |
| Incremental/parallel frame rendering + preview cache | BUILD | core differentiator (vidformer paper is the reference) |
| CLI | BUY | clap 4.6.8 |
| Async (watch/serve/IPC) | BUY | tokio 1.54.1 (+ rayon for frame parallelism) |
| Scripting DSL | BUY | rhai 1.27.0; serde JSON render-plan interchange |
| Hardware encode/decode (NVENC/VAAPI/Vulkan Video) | DEFER | gpu-video 0.5.0 / pixelforge too early (247-6K downloads); route via libav hwaccel for now |
| GStreamer pipeline | SKIP | gstreamer-rs 0.25.4 is solid but drags a C runtime; wrong fit for a code-first tool |

## 10. Explicit Warnings

- **Do not adopt**: x264 (stale 2022), x264-next (deprecated), x265 (dead), vpx (2015), mux (2016), mkv (2021), rlua (superseded by mlua), vk-video (renamed to gpu-video). **Do not search for**: hwencoder, matroska-demux, mkv-demuxer — none exist.
- **matroska** crate parses metadata only; it will not mux your output.
- ffmpeg-next / ffmpeg-the-third are WTFPL — legally usable, but flag for license review if Quillon's policy expects MIT/Apache.
- The mp4 crate's 3-year release gap is the biggest supply-chain risk in the pure-Rust path; plan for forking or libav-only muxing.

Sources: crates.io API (all crate rows, fetched 2026-11-17), docs.rs/mlua (Lua::sandbox), rhai.rs/book (hot-reload pattern), deno.com blog (V8 snapshots), github.com repos for fframes, vidformer, FilmCraft, video-rs, ReelForge, vidr, OpenCat, gpu-video, script-bench-rs (via WebFetch), Phoronix (rav1e 0.8), arXiv 2601.17221 (vidformer paper).
