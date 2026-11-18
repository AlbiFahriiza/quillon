# L. CI & Release Engineering Notes — for Quillon

*Research date: 2026-11-17. Search findings + crates.io verification by the lead agent.*

## CI (sources: Swatinem/rust-cache docs, cargo-chef guides, GitHub's Rust tutorial)

- **Swatinem/rust-cache** is the standard GitHub Action for cargo caching (two-key scheme: workspace Cargo.lock hash + job matrix); GitHub-hosted runners ship Rust toolchain preinstalled with a tools cache.
- CI shape for Quillon (informed by research/D §2 Kineto's `just check` mirror):
  1. `cargo fmt --check`
  2. `cargo clippy --workspace --all-targets -- -D warnings`
  3. `cargo test --workspace`
  4. Golden-frame parity gate (render corpus → sha256 compare; research/J-style determinism)
  5. `cargo doc` build (no-dead-links) — publish on tags
- Release artifacts built by cargo-dist in a separate release workflow; docs deployment from `main`.

## Release tooling (verified: cargo-dist 0.34.0, 2026-11-02, 190K downloads — active)

- **cargo-dist** generates a full release workflow: builds binaries for the target matrix, cuts GitHub Releases with checksums, generates an installer script (`curl ... | sh`) — the standard for Rust CLI distribution in 2026.
- Quillon usage plan: `cargo dist init` with targets `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`; tag-triggered; artifacts checksummed (Kineto's GitHub Action verifies checksums on download — same hygiene).
- npx wrapper (`npx quillon-mcp`) downloads the platform binary from the GitHub Release by checksum — the Kineto/Clipkit adoption pattern (research/C §6).

## Repo automation (source: matklad's xtask advice, research/E §1)

- Automation (release-prep, golden-hash refresh, schema export) lives in an `xtask` crate run via `cargo xtask <cmd>`, not Makefiles — keeps the workspace self-contained and cross-platform.

## License/NOTICE checklist for release (from research/E §5)

- Ship `NOTICE` listing: ffmpeg-next/ffmpeg-sys-next (WTFPL — flag for review), symphonia (MPL-2.0), rav1e/tiny-skia/openh264 (BSD), whisper.cpp weights (MIT, runtime download), fonts (bundled fonts must be OFL/APL).
- The document schema is published separately (CC-BY) so ecosystems can adopt it without the engine license.
