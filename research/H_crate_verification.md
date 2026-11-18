# H. Supporting-Crate Verification Log — for Quillon

*Research date: 2026-11-17. All rows verified live via the crates.io API by the lead agent.*

## Subtitle crates (for caption import/export)

| Crate | Latest | Updated | Downloads | Verdict |
|---|---|---|---|---|
| subtitler | 2.13.0 | 2026-10-30 | 6,682 | Active (recent release); parse/manipulate/generate, multi-format |
| rsubs-lib | 0.4.0 | 2026-10-07 | 35,169 | Active; SRT/SSA/ASS/VTT parse-modify-convert |
| subparse | 0.7.0 | 2021-04-11 | 50,438 | Stale 5 yrs; SRT/SSA/ASS/IDX/SUB common interface |
| ass_parser | 0.2.3 | 2025-04-11 | small | ASS-only, niche |
| subtitle | 1.0.3 | 2021-03-28 | 4,236 | Dead |
| srt | 0.1.0 | 2017-03-10 | 6,386 | Dead |
| oxideav-ass / oxideav-subtitle | — | 2026-11-06 | — | Pure-Rust ASS codec with semantic round-trip (github.com/OxideAV/oxideav-subtitle); promising, watch |

Decision input: SRT is trivial enough (~50 lines) to parse/serialize ourselves; VTT similar. For ASS (styled karaoke), rsubs-lib or oxideav-ass are the candidates. The caption pipeline's timing regularization (min duration, merge/split) is ours regardless — raw parser output is never burned directly.

## GPU compositing notes (search findings, 2026-11-17)

- wgpu 30.0.2 remains the GPU standard (51.5M downloads, research/B). Image-filter ops (blur, bloom, SSAO) are pixel/fragment-shader territory; compute shaders win for scatter-type ops (research: computergraphics.stackexchange.com/questions/54). WGSL compute passes for procedural 2D content are proven (jarl-game.com 2D grass, Aug 2026).
- fframes' approach (research/D §4) remains the pragmatic reference: hand the scene tree to a mature rasterizer (Skia) instead of writing custom GPU kernels. wgpu/vello is the pure-Rust future path (vello 0.12.0, pre-stable — research/B).
- Quillon v0.x: CPU tiny-skia is the deterministic baseline; GPU is a later adapter behind the same `render_tick` contract, parity-tested against CPU output with a tolerance, explicitly excluded from golden-hash guarantees.

## Render-farm / scale-out pattern (search findings)

- Cloud render farms (3sfarm, GarageFarm, AWS Deadline Cloud) all share: queue + fleet of workers + shared storage — the architecture Quillon's cache design must be *compatible with* (stateless workers + content-addressed shared cache = horizontal scaling) without shipping any cloud dependency itself.
- Quillon application: the render manifest (per-frame hash list) plus content-addressed cache keys mean a fleet of stateless `quillon render` workers on one shared cache directory/S3-compatible store parallelize with zero coordination — each worker claims frame ranges by key. This is a v1.x concern; the v0.x requirement is only that the cache layout stays shareable (no machine-local state in keys).

## persona → feature mapping confirmed (no change to research/F)

- The subtitle + GPU + farm findings do not change the persona table; they firm up: captions (whisper-rs + own timing regularization + rsubs-lib for ASS), GPU as adapter, farm as cache-layout constraint.
