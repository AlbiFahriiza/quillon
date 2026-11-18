# N. Naming & Positioning Check — for Quillon

*Research date: 2026-11-17. Verified live: crates.io API + GitHub search (gh CLI, authenticated).*

## Name availability

- **crates.io `quillon`: NOT FOUND** — the crate name is free (checked 2026-11-17 via `https://crates.io/api/v1/crates/quillon`). This is the publishing name for the engine crates (`quillon`, or `quillon-core` family).
- `quill` crate exists (0.2.0, SVG plotting library, 9K downloads) — different name, different domain; no conflict but do not shorten "quillon" to "quill" in docs to avoid confusion.
- GitHub: no significant project named Quillon in the video/media space. Existing "Quillon" repos are small note-taking apps (alexcj10/Quillon, ★10 — local-first notes with Smart RAG; QuilloNotes Android handwriting app) and inactive repos. No name collision that matters for discoverability.
- No USPTO trademark search was performed (out of scope for this session) — if the project becomes commercial, a proper clearance search is needed. This is a stated gap, not a verified clearance.

## The name itself

"Quillon" is the crossguard of a sword — the part that guards the hand and balances the blade. Decent metaphor for a tool that guards the edit (deterministic, validated) while the user swings the blade (creative code). The landing page repo (quillonw) already uses the tagline "a programmable video studio — code the frame, edit programmatically, render cinema."

## Positioning one-liners (from research/A §11, refined)

- **Quillon: Video as Code. Create from scratch with code. Edit existing footage programmatically. One Rust binary, no browser, no Node.**
- vs Remotion: no React/Chrome/license; parallel native rendering.
- vs Kineto: full video editing (source clips, audio, captions), not just 2D motion graphics.
- vs fframes: document/JSON-first (agent-safe), editing existing footage; not Rust-code-only.
- vs Clipkit: genuinely open engine (MIT/Apache), self-hostable, no minute caps.

## Repo identity (decided)

- GitHub: `AlbiFahriiza/quillon` (created this session).
- crates.io: `quillon` + `quillon-core`, `quillon-media`, `quillon-mcp`... (all free as of today).
- Landing page stays in `AlbiFahriiza/quillonw` (separate repo, existing).
