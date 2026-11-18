# S. Remaining Verification & Risk Register — for Quillon

*Research date: 2026-11-17. Final verification pass + consolidated risk register for the plan.*

## Additional verifications this session

- **kineto-core** (the closest document-format sibling): max 0.2.0, updated 2026-10-28, **243 total downloads** (crates.io API). Small but active; validates that the "document → deterministic video" niche has room — nobody owns it at scale.
- **vidformer paper** (arXiv 2601.17221 abstract re-read): evaluation = "diverse annotation workloads" (dataset names not in abstract; full text needed for baselines), full-render 2–3× faster, time-to-playback 0.25–0.5s (~400×), enables conversational LLM video querying. Architecture claims: declarative lift + transparent render optimization + JIT VOD serving. Internal mechanics (symbolic execution, spec recording) are in the full text — we verified them separately from the GitHub README (research/D §1), so the plan cites both.
- **Cross-compilation reality** (cargo-dist issue #74, blog.urth.org, SO): Linux ARM/x86_64 cross-compile from a single runner is the classic pain; the standard answer is a **GitHub Actions matrix with native runners per target** (ubuntu ×2, macos ×2, windows ×1) — which is exactly what cargo-dist generates. No exotic tooling needed; keep `cross` as a documented local alternative.

## Consolidated risk register (for the plan)

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| 1 | ffmpeg-next maintenance-only mode stalls (no API improvements ever merged) | Medium | Medium | Contain it in one adapter crate behind our own trait; rsmpeg/ez-ffmpeg are documented alternates (research/K) |
| 2 | libav linking pain hurts adoption (build failures on user machines) | High | High | Default = sidecar mode (system ffmpeg binary, zero linking); libav is an opt-in feature; prebuilt binaries via cargo-dist |
| 3 | Pure-Rust MP4 muxer gap (mp4 crate stale 3 yrs) | Certain (already true) | Low | Mux via libav/ffmpeg in both modes; pure-Rust mux is a non-goal for v0.x (research/B §2) |
| 4 | Frame-accurate cuts require re-encode (slow path vs `-c copy`) | Certain (physics of GOPs) | Medium | Documented contract: keyframe-accurate default, `--smart-copy` later (research/M §2) |
| 5 | GPU nondeterminism breaks golden-hash testing | Certain if GPU used | Medium | GPU is an adapter excluded from hash guarantees; CPU tiny-skia is the deterministic baseline (research/H §2) |
| 6 | Whisper timing drift produces bad captions | Medium | Medium | Own timing-regularization layer; never burn raw Whisper output (research/F §5) |
| 7 | Scope creep (Remotion parity is a decade of work) | High | Fatal | Phased plan with hard non-goals per phase; "editing existing footage + document-first agent loop" is the wedge, not CSS parity |
| 8 | WTFPL deps spook corporate adopters | Low | Medium | Sidecar mode is pure MIT; NOTICE file documents everything (research/P) |
| 9 | Solo-maintainer crates in the graph (ez-ffmpeg etc.) | Medium | Low | Only adopted if at all as watch-list references; core deps are linebender/serde/clap-class orgs |
| 10 | oneOf validation errors confuse agents | Certain (measured, research/J) | Medium | Our own ValidationError with JSON-pointer paths; serde-first validation pipeline |
| 11 | "FilmCraft" research claim unverified | Already caught | Low | Excluded from the plan; serves as a reminder that subagent claims get re-verified |
| 12 | Name collision (note apps named Quillon) | Low | Low | Different domain; crates.io name free (research/N); trademark check flagged as a gap if commercializing |

## What we could NOT verify this session (stated gaps for the plan)

- Neutral market-size figure for video-as-code tooling (only vendor numbers exist — research/F §1).
- vidformer's exact baseline systems and datasets (abstract-level only; full PDF not parsed).
- USPTO trademark clearance for "Quillon" (out of scope; flagged).
- Kineto/Clipkit performance claims beyond what their own pages publish (no third-party reproduction found).
