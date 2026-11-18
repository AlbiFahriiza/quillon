# T. Color Management Notes — for Quillon

*Research date: 2026-11-17. Search findings: Blender 5.3 manual (color management), fractolog.com alpha-blending correctness (Jul 2024), r/vfx thread, Adobe AE color docs.*

## The rule

- **Compositing math must happen in linear light.** Alpha blending on gamma-encoded (sRGB) values produces dark fringes and incorrect transparency — the classic "why do my crossfades look muddy" bug (fractolog: "for correct blending, the main scene must render/blend in linear, then convert once at output").
- Blender's guidance is the industry answer: "Rendering and compositing is best done in a scene linear color space"; output conversion (to sRGB or display space) happens once, at the end.
- VFX practice: footage is converted to linear before compositing because transparency arithmetic assumes linear channels.

## Quillon application

- Internal pipeline: decode → **to linear (f32)** → composite (blend, transforms, gradients in linear) → **convert once to output space** (sRGB/8-bit for the encoder, or p010le for 10-bit paths).
- Document colors are authored as sRGB hex (what users see in pickers) and converted to linear at load; the document stores the authoring space explicitly (`color_space: "srgb"` field, so Rec.709/HLR can be added later without a format break).
- Filters/effects that are perceptual (opacity fades can be perceptual in some engines) get an explicit `interpolation: "linear" | "perceptual"` field on the animation key — perceptual fades match how NLEs behave; linear is physically correct. Default: linear (deterministic + documented).
- tiny-skia operates on the buffers we hand it; correctness of the space is our adapter's contract, and a golden-hash test covers a crossfade to catch gamma-space regressions (this is exactly the class of bug that passes schema validation and looks wrong on screen — a check_document lint "crossfade:compositing-space" is cheap insurance).
- 10-bit: VAAPI nv12 silently downconverts 10-bit to 8-bit (research/E §2) — the document's `bit_depth` field makes that explicit at render planning time instead of a silent quality loss.
