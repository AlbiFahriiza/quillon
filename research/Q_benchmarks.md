# Q. Benchmark & Metrics Notes — for Quillon

*Research date: 2026-11-17. Search findings; informed by research/D §4 (fframes benchmark honesty).*

## Rules for honest render benchmarks (from fframes' caveats + Meishe methodology article + FPS-Bench)

1. **Never headline a single synthetic scene.** fframes' 31.16× came from 99k one-pixel rects — near-best-case for cache-and-raster, worst-case for a virtual DOM. The author himself calls it "a floor for one synthetic scene, not a general 31×."
2. Report **median + p10/p90 across a corpus** of scenes (motion graphics, text-heavy, video-overlay, caption-burn) — the shape Clipkit used (5 briefs × 3 model tiers × 4 lanes).
3. **Disclose hardware, threads, encoder settings, and whether the encoder was instantiated** (fframes' benchmark excluded the encoder entirely — rendering only).
4. Distinguish **render-only vs end-to-end** (decode+composite+encode+mux) numbers; report both.
5. Distinguish **CPU vs GPU paths** explicitly (fframes' GPU run printed "skipped" — the headline was CPU).
6. For agent-facing latency: report **time-to-first-validated-document** and **time-to-first-preview-frame**, not just full-render time (vidformer's 0.25–0.5s TTP is the metric that matters for interactivity).
7. Publish the benchmark corpus + harness in-repo so anyone can rerun (`cargo xtask bench`).

## Quillon metrics to instrument from day one

| Metric | Definition | Why |
|---|---|---|
| frames/sec (render-only, CPU) | compositor throughput, encoder excluded | comparable to fframes' number |
| end-to-end render time | document → MP4 on disk | user-facing truth |
| time-to-first-preview | `quillon preview` cold start → first frame served | the interactive loop |
| incremental re-render time | edit one scene → only affected frames re-rendered | the cache promise |
| validate cost | document → validation verdict | the ~20-token loop step |
| agent-validity rate | % of agent-authored documents that pass validation first try | the Clipkit-style headline metric |
| reproducibility | same document → identical frame hashes, N runs | the determinism contract |

## VMAF/quality comparisons

- When comparing encoders (libav H.264 vs hardware vs AV1), measure **VMAF at fixed bitrate** + encode speed, not just speed (VAAPI "usually needs a higher bitrate than libx264" for equal VMAF — research/E §2). Full-reference quality measurement tooling (forasoft VQM overview, Jul 2026); `ffmpeg`'s libvmaf filter is the practical measurer.
