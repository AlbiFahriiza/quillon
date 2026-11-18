# P. FFmpeg Licensing Compliance — for Quillon

*Research date: 2026-11-17. Source: https://www.ffmpeg.org/legal.html (official). Not legal advice.*

## The rules that bind Quillon

- FFmpeg is **LGPL 2.1+ by default**; GPL parts activate only via `--enable-gpl` or GPL-only external libs; `--enable-nonfree` is off the LGPL path entirely.
- **libx264 is GPL-only** — a build that compiles it in makes the whole FFmpeg build GPL. (This is why Quillon's libav profile encodes H.264 via the LGPL `libavcodec` built-in encoder, with x264 only through the optional sidecar/system-ffmpeg path where the user supplies their own binary.)
- LGPL binary distribution obligations: **dynamic linking**, ship the **matching FFmpeg source** on the same server as binaries, include a `changes.diff` and configure notes, attribution statements on download pages/about box, no obfuscated DLL names, no reverse-engineering bans in any EULA.

## Quillon compliance plan (v0.x concrete steps)

1. The prebuilt binaries we distribute (cargo-dist artifacts) link **system FFmpeg libraries dynamically** when present; the default `ffmpeg-sidecar` mode drives the user's own `ffmpeg` binary — in both cases **we never distribute GPL/nonfree FFmpeg code**.
2. Static-libav builds (like fframes' approach) are **not distributed by us**; they remain a documented local-build option for users who accept the obligations (`features = ["libav-static"]` → user builds it themselves).
3. Attribution file (`NOTICE`) + download-page statement: "This software uses code of FFmpeg licensed under the LGPLv2.1" + source links (https://ffmpeg.org/download.html + our build notes).
4. The engine itself stays **MIT OR Apache-2.0** — our license does not claim FFmpeg code; LGPL parts stay dynamically linked at the edges (media adapter crate only).
5. Patent caveat acknowledged (MPEG LA): we distribute an open-source tool, not a codec product; commercial users get pointed at the official page's disclaimer.

## License posture summary (consolidated from research/E §5)

| Component | License | Distribution mode |
|---|---|---|
| Quillon engine + CLI + MCP | MIT OR Apache-2.0 | source + prebuilt binaries |
| Document schema | CC-BY | published standalone |
| ffmpeg-sidecar (MIT) | MIT | bundled (pure Rust) |
| FFmpeg (user's binary / system libs) | LGPL 2.1+ | never bundled; attribution only |
| whisper.cpp via whisper-rs | MIT | optional feature, weights runtime-download |
| symphonia | MPL-2.0 | source availability (file-level copyleft, fine) |
| tiny-skia / rav1e / openh264 | BSD | attribution |
| ffmpeg-next bindings | WTFPL | usage fine, noted in NOTICE |
