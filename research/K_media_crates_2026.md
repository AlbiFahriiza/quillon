# K. Media Crate Landscape Cross-Check (2026) — for Quillon

*Research date: 2026-11-17. Source: https://dev.to/yeauty/the-state-of-media-processing-in-rust-2026-what-each-crate-actually-covers-2k2c (Jul 2026), crate rows re-verified live via the crates.io API by the lead agent.*

## The 2026 coverage map (source article + our verification)

| Crate | Decode | Encode | Mux | Filter | crates.io status (verified 2026-11-17) |
|---|---|---|---|---|---|
| ffmpeg binary via std::process | ✅ | ✅ | ✅ | ✅ | n/a — community default answer |
| ffmpeg-sidecar | ✅ raw RGB iterator | ❌ | ❌ | ❌ | 2.7.0, 2026-10-18, 2.3M total |
| ffmpeg-next | ✅ full libav | ✅ | ✅ | ✅ | 9.1.0, 2026-11-11, 9.05M total |
| rsmpeg | ✅ libav 6/7/8 | ✅ | ✅ | ✅ | 0.18.0+ffmpeg.8.0, 2025-08-24, 335K — ~14 months without release |
| video-rs | ✅ RGB ndarrays | ✅ ndarray→mp4 | partial | basic + seek_to_frame | 0.12.0, 2026-09-09, 507K — "video only, no audio API" |
| symphonia + rubato | ✅ audio only | ❌ | demux | ❌ | 0.6.1 / 5.1.0 — clean pure-Rust audio path |
| ez-ffmpeg | ✅ FrameExtractor | ✅ VideoWriter frame-push | ✅ | ✅ FrameFilter | 0.19.3, 2026-10-06, 251K — new, single maintainer |
| ac-ffmpeg | ✅ | ✅ | ✅ | packet-level | no release in 14+ months (article) — skip |

## Converged community advice (article)

- "Just run the ffmpeg binary" remains the default thread answer — most reliable, fewest pitfalls — when output is a file and the recipe exists.
- The moment frames/samples must land in process memory (feeding compositing, real-time, logic interleaved with decode), the CLI tax starts (stdout parsing, temp WAVs, mismatched parameters) → in-process libav.
- "Pick by task, not by tribe."
- No single crate covers everything; pure-Rust codec wall stands (symphonia: HE-AAC/Opus unfinished, no AC-3, video out of scope).

## Warnings that matter for Quillon

- **ffmpeg-next is in maintenance-only mode by policy** — "Any PR to improve existing API is unlikely to be merged"; third fork in lineage. Still the foundation of choice (fresh 9.1.0 release, 2.6M recent downloads).
- **ez-ffmpeg** (frame-push VideoWriter + FrameFilter, all three jobs in-process) is the lowest-migration path but: links libav via ffmpeg-next, single maintainer (bus factor), experimental APIs.
- **rsmpeg** fresher low-level wrapper but ~14 months quiet (our verification: last release 2025-08-24 — confirms the article).
- One shipped public API in the wild was annotated "might trigger undefined behavior" — safe wrappers ≠ safety proof; keep unsafe auditing in dependency review.
- whisper-rs: archived on GitHub but actively maintained on Codeberg — not dead (matches our research/F §5).

## Decision for Quillon (updates research/B §9)

- **Decode/encode/mux engine**: ffmpeg-next 9.1.0 (activity beats rsmpeg's quiet period; API pain is contained inside one adapter crate `quillon-media`).
- **Fallback mode**: `ffmpeg-sidecar` adapter behind the same trait for users who cannot link libav (pure-MIT path, binary on PATH).
- **Audio pure-Rust path**: symphonia + rubato where codecs allow (WAV/MP3/FLAC/OGG), libav otherwise.
- **ez-ffmpeg**: watch list, not a dependency (bus factor + experimental APIs); its FrameExtractor/VideoWriter shapes are good API references for our adapter trait.
- **video-rs**: not adopted (video-only, "use with caution" self-assessment); its ndarray API is a useful contrast, nothing more.
