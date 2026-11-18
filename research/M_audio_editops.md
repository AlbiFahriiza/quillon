# M. Audio & Edit-Ops Engineering Notes — for Quillon

*Research date: 2026-11-17. Search findings + crates.io verification by the lead agent.*

## 1. Loudness (source: EBU Tech; ebur128 crate)

- EBU R 128: normalize to **−23 LUFS ±0.2 LU** (±1 LU live) — the broadcast standard; streaming platforms use their own (e.g. −14 LUFS common for social platforms, but that is platform policy, not law — expose as a preset, not a hardcoded value).
- **ebur128 crate VERIFIED**: 0.1.11, updated 2026-10-26, 1.27M downloads — the Rust port of libebur128 (originally ported by the coaxion.net blog, Sep 2020). Active. This is the measurement half of `quillon audio analyze` (LUFS integrated, true peak) and the normalization feedback loop.
- Quillon audio pipeline: decode (symphonia/libav) → resample to common rate (rubato) → mix (own DSP: sum + gain + fades in tick domain) → measure (ebur128) → gain-adjust to target LUFS → encode.

## 2. Frame-accurate seek (sources: FFmpeg wiki Seeking; SO 69422502; dev.to frame-accurate trim, Aug 2026)

- The only way to get accurate seeking is to **seek to the nearest earlier keyframe and decode forward** to the target frame. libx264's default GOP size is 250 — a keyframe can be up to ~10s away at 24fps.
- `-c copy` trims can only cut on keyframes — a 12.4s trim starts wherever the previous keyframe sits. Frame-accurate cuts therefore require re-encode of at least the boundary GOPs.
- Quillon edit-op contract: `source_range { from_tick, to_tick }` decodes from keyframe ≤ from_tick forward; the decoder adapter exposes nearest-keyframe semantics; cuts are re-encoded by default with a documented `--smart-copy` later optimization (copy mid-GOP-aligned segments, re-encode only boundary GOPs).

## 3. Audio-video sync invariants (systems-design applied)

- Audio samples and video ticks must share the document timebase: audio resampled to the document's audio rate; every mix decision happens in tick domain; drift is an invariant violation that CI tests (render a click-track document, assert frame/sample alignment).
- Failure-first answers: missing audio stream → document error at validation time (not render time); variable-framerate sources → probed and normalized at import (probe_media reports it); corrupted trailing packets → decode errors surface with tick context in the render manifest.

## 4. Edit operations list for v0.x (grounded in Treza verbs + Editly layers + SO/ffmpeg mechanics)

| Op | Semantics | Notes |
|---|---|---|
| trim | source_range cut | keyframe-accurate via decode-forward |
| concat | sequence of clips on a track | gap handling: error or explicit freeze, chosen in document |
| overlay | element stack over clip range | compositor, not decoder |
| retime | speed factor per range | audio resample + pitch preserve flag (rubato) |
| mute/duck | gain envelope over range | tick-domain envelopes |
| caption burn-in | text elements from SRT/VTT/ASS or transcript | cosmic-text rendering; timing regularization ours |
| reframe | crop/scale to target aspect (9:16, 1:1, 16:9) | fit-mode enum reused from research/G §2 |
| freeze | hold frame across range | single decode + repeat |

All ops are **document-level** (data, validated) — never imperative scripts. This is the "editing existing footage as a first-class citizen" gap from research/A §9.5.
