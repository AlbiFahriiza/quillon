# F. Market & Personas Research — for Quillon

*Research date: 2026-11-17. Direct reads by the lead agent. Vendor-sourced numbers flagged as such.*

## 1. Who buys video-as-code today (source: https://fluxnote.io/guides/programmatic-video-vs-ai-generation — AI video vendor, numbers are theirs, treat as directional)

- **Programmatic video buyers**: developers (React/TS). "Mail merge for video" — dynamic data (customer name, product photos, prices) injected into templates; thousands of unique videos from one structure.
- **Named use cases**: personalized sales outreach, dynamic real-estate listings, weekly market-report videos, strict brand guidelines ("pixel-perfect" control), mass personalization (name, location, profile photo).
- **Cost structure (vendor numbers)**: template setup 30–50 developer-hours ($3k–6k at $75–150/hr); render APIs like Shotstack $49/mo for 1,250 rendered minutes (Q1 2026), $499+/mo at volume; no-code template platforms (Plainly, Bannerbear) still need technical integration for render at scale.
- **AI generation buyers**: marketers/teams without developers; time-to-first-video <5 min vs weeks; $10–60/mo SaaS tiers; strengths in content repurposing (article → TikTok/Reels/Shorts); 2026 limitations: temporal consistency (clothing color changing mid-shot) and specific-element control.
- **Quillon wedge**: these two lanes exclude each other today. An open Rust engine with an agent-native surface is the first tool that serves BOTH: developers get code + determinism, non-developers get an agent that emits validated documents.

## 2. Use-case categories seen across research (Remotion showcase categories: products, campaigns, launch videos, tutorials, creative tools; Blings "Spotify Wrapped for your company"; Kapil619/SpotifyWrap-Remotion)

- Year-in-review / wrapped-style personalization (company metrics → shareable video)
- Data-driven report videos (weekly market reports, changelogs — Kineto dogfoods changelog videos)
- Personalized outreach at scale (sales, onboarding)
- Template marketplaces / creative tools (Remotion template ecosystem; lobehub skills marketplace lists remotion-templates)
- Explainers & motion graphics (Motion Canvas's niche)
- Podcast clip factories (Treza's proof chain: transcribe → pick moment → reframe 9:16 → captions → badge → publish)

## 3. Personas for Quillon (synthesis)

| Persona | Needs | Quillon answer |
|---|---|---|
| **Dev (Rust/TS)** — builds video features into products | library crates, no runtime tax, deterministic builds, CI-friendly | `quillon-core` library + golden-hash testing + cargo workspace |
| **AI agent** (Claude/Codex/Cursor) driving the tool | schema-first, cheapest-first loop, machine-readable everything, no code execution | JSON document + schemars schema + MCP tools + validate→preview→render |
| **Content ops / marketer** — no code, has an agent | agent does the editing; human reviews previews | `quillon preview` filmstrip + templates + one-command renders |
| **Editor with existing footage** — cut/caption/repost | frame-accurate edit ops on real media, captions from audio | edit ops on source clips + whisper-rs transcription + SRT/WebVTT burn-in |

## 4. Preview GUI reality check (verified via crates.io, 2026-11-17)

- **egui 0.37.0** (2026-10-15, 31.9M downloads) / **eframe 0.37.0** (25.2M) — active, mature; an **egui-timeline-widget** crate exists (lib.rs, "customizable timeline widget for media applications", Sep 2025).
- **notify 9.0.0-rc.9** (2026-11-15, 200M downloads) — file watching for live reload (note: rc channel).
- FilmCraft reportedly uses egui for its NLE UI (UNVERIFIED — FilmCraft itself unverified, see research/C caveat); fframes ships a native preview window with filmstrip scrubbing (verified via its repo/article).
- Decision input: Quillon v0.x ships `quillon preview` as a **local web page** (serve filmstrip + frame PNGs over localhost, auto-reload on document change) — no GUI toolkit dependency, works over SSH, renders in the user's browser. An egui native preview is a later optional crate. This keeps the core binary GUI-free and CI-safe.

## 5. Cross-check: whisper-rs for captions (verified: 0.16.0, 2026-03-12, 2.1M downloads, https://crates.io/api/v1/crates/whisper-rs)

- whisper-rs is the maintained whisper.cpp binding; word-level timestamps available (whisper.cpp token-level timing; "whisper timestamped" approaches exist in other languages).
- Known issue class: subtitle timing drift (ggml-org/whisper.cpp#3623) — plan for post-hoc timing regularization (min/max caption duration, merge/split rules) in the caption pipeline, not raw Whisper output.
- Model weights are a runtime download (license: Whisper models MIT; whisper.cpp MIT) — ship as an optional feature, never bundled into the core binary.

## 6. What the research does NOT establish (honest gaps)

- No neutral market-size number for "video-as-code" tooling exists; vendor pages are the only cost/scale sources found. Do not put a market-size claim in the plan without a neutral citation.
- Remotion's showcase page loads cards via JS; named-company production stories were not directly captured this session. Known named adopters from other pages: Spotify Wrapped-style recreations are community templates, not Spotify itself.
- "FilmCraft" (18.5k-star pure-Rust NLE) reported by subagent B but not found by subagent C — treat as unverified; the plan must not depend on it.
