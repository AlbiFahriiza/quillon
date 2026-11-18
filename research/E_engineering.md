# E. Engineering Foundations Research — for Quillon

*Research date: 2026-11-17. Direct source reads by the lead agent. Foundation-crate rows verified live against the crates.io API.*

## 1. Workspace layout (source: matklad, "Large Rust Workspaces" — https://matklad.github.io/2021/08/22/large-rust-workspaces.html)

- **Flat layout wins** for 10k–1M LOC projects (reference: rust-analyzer, ~200k lines, 32 crates).
- Root `Cargo.toml` is a **virtual manifest** (`members = ["crates/*"]`) and nothing else. Every crate — including the main binary — sits one level deep under `crates/`. Directory name identical to crate name, prefix and all.
- Never put the main crate at the root (pollutes root, forces `--workspace` flags, breaks consistency).
- Internal crates get `version = "0.0.0"`; crates intended for publishing get isolated in `libs/` so you can verify `libs/` never depends on `crates/`.
- Write all automation as a Rust crate (the **cargo xtask** pattern) rather than Makefiles/shell scripts.
- Rationale in three lines: cargo's namespace is already flat; `ls ./crates` is an instant map; "there are no perfect hierarchies" — trees deteriorate, flat needs no maintenance.

## 2. Hardware encoding reality on Linux (source: https://ffmpeg-cookbook.com/en/articles/hardware-encode-vaapi)

- VAAPI matrix: Intel iGPU H.264 (Sandy Bridge+), H.265 newer gens, AV1 Arc/newer Xe only; AMD H.264/H.265 by VCE/VCN generation, AV1 RDNA3+; **NVIDIA is decode-only under VAAPI — encode goes through NVENC**.
- Two pipelines: software-decode → hw-encode (`-vf 'format=nv12,hwupload' -c:v h264_vaapi`), or full-GPU (`-hwaccel vaapi -hwaccel_output_format vaapi`). Pitfalls: missing `hwupload` (VAAPI encoders only accept GPU-resident frames); **10-bit silently becomes 8-bit** with nv12 (use `format=p010le` + `main10`); multi-GPU render node confusion; Docker needs `--device /dev/dri`; driver variants differ (i965 vs iHD; non-free driver needed for some encode paths); `vainfo` verifies entry points.
- AV1 VAAPI: no `-qp` (silently ignored), quality scale 0–255.
- Fallback rule: VAAPI "usually needs a higher bitrate than libx264" for the same VMAF (mitigate: −1–2 QP or +10–20% bitrate); software wins for max compression, hardware wins for throughput. NVIDIA → NVENC; no device/drivers → software.
- **Implication for Quillon**: hardware encode is an *adapter option* selected by a codec profile, auto-detected with `vainfo`-style probing and clean software fallback — never a hard requirement. Deterministic software rendering (tiny-skia) remains the default path so golden-hash testing stays valid; hardware paths are explicitly marked non-deterministic for pixel-hash purposes.

## 3. Foundation crates (verified via crates.io API, 2026-11-17)

| Crate | Latest | Updated | Total downloads | Role in Quillon |
|---|---|---|---|---|
| serde | 1.0.229 | 2026-07-18 | 1,726,431,803 | document (de)serialization |
| serde_json | 1.0.154 | 2026-10-11 | 1,633,300,649 | document JSON I/O |
| schemars | 1.2.3 | 2026-10-12 | 661,281,510 | derive JSON Schema from document types |
| jsonschema | 0.61.2 | 2026-11-12 | 124,589,631 | validate agent-submitted documents |
| clap | 4.6.8 | 2026-11-16 | 1,350,670,803 | CLI |
| rayon | 1.12.0 | 2026-04-14 | 675,406,410 | frame-parallel rendering |
| tokio | 1.54.1 | 2026-10-21 | 1.21B | preview server / watch / IPC only |
| image | 0.25.10 | 2026-03-10 | 254,907,517 | raster decode/encode |
| tiny-skia | 0.12.0 | 2026-02-02 | 71,919,523 | CPU compositing (deterministic default) |
| cosmic-text | 0.19.0 | 2026-04-22 | 12,604,252 | text shaping/layout/burn-in |
| symphonia | 0.6.1 | 2026-08-13 | 22,233,913 | audio decode |
| rubato | 5.1.0 | 2026-11-12 | 15,819,656 | audio resample |
| ffmpeg-next | 9.1.0 | 2026-11-11 | 9,052,157 | libav: any-codec decode, encode, mux |
| ffmpeg-sidecar | 2.7.0 | 2026-10-18 | 2,315,126 | fallback: drive system ffmpeg binary over pipes |
| rmcp | 3.10.0 | 2026-11-12 | 50,855,726 | official MCP SDK (server feature pulls schemars) |
| rhai | 1.27.0 | 2026-10-21 | 14,979,536 | sandboxed scripting layer (phase 2) |
| insta | 1.49.0 | 2026-10-03 | 130,847,688 | snapshot testing |
| anyhow | 1.0.104 | 2026-07-18 | 1,183,054,984 | error handling (binaries) |
| thiserror | 2.0.21 | 2026-09-23 | 1,875,645,725 | error types (libraries) |
| tracing | 0.1.44 | 2025-12-18 | 1,061,227,237 | structured logging |

## 4. Snapshot / golden testing (source: Rust Project Primer — snapshot testing; docs.rs/martensite-test; general practice)

- **insta** (130M downloads) is the Rust standard for snapshot tests: assert serialized output against committed `.snap` files; `cargo insta review` to accept changes.
- For pixel-accurate video work, snapshot-hashing extends naturally: render frame at tick T → sha256 of the raw RGBA buffer → compare against golden hash list (Kineto's `testdata/golden/hashes.json` pattern; 27/27 corpus, CI-enforced).
- Two-tier determinism contract: (1) **pixel-level golden hashes** for the CPU renderer — byte-identical required; (2) `insta` snapshot tests for document canonicalization (same document → same canonical JSON) and CLI JSON outputs (inspect/probe). MP4 container bytes are explicitly out of scope ("reproducible pixels, not reproducible MP4s").
- Anti-flake rules: no wall-clock in render paths (tick-based); no HashMap iteration order in canonical serialization (BTreeMap / ordered serialization); fonts pinned by hash in the document's asset map.

## 5. License posture (sources: research/A + crates.io metadata)

- Quillon engine: **MIT OR Apache-2.0** dual license (Kineto, video-rs precedent) — maximizes adoption; every competitor comparison page uses Remotion's custom license against it.
- WTFPL deps (ffmpeg-next, ffmpeg-sys-next): legally fine to use and ship; flag in a NOTICE file; the ffmpeg-sidecar fallback keeps a pure-MIT path when linking is unwanted.
- MPL-2.0 deps (symphonia): file-level copyleft, compatible with MIT/Apache distribution as long as MPL files' source remains available — acceptable, note in NOTICE.
- BSD crates (rav1e, tiny-skia, openh264): attribution only, no issue.
- The document format/schema is published under CC-BY or CC0 to decouple ecosystem adoption from the engine license (Clipkit's CKP/1.0 move).
