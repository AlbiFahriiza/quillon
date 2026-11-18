# C. AI-Agent Integration Research — for Quillon (VIDEO AS CODE)

*Research date: 2026-11-17. Delegated to Explore subagent (gated: see .disciplined/delegation.json). All crate facts pulled live from crates.io API; all claims sourced. UNVERIFIED marks anything that could not be confirmed.*

## 1. MCP (Model Context Protocol) — Architecture Facts

MCP is an open, stateful client-server protocol over JSON-RPC 2.0 with three server primitives — **tools** (model-invoked functions), **resources** (readable data, e.g. schemas), and **prompts** — and two transports: **stdio** (local, typically one client per process) and **Streamable HTTP** (remote, HTTP POST + optional SSE).

- Spec/architecture: https://modelcontextprotocol.io/docs/draft/learn/architecture ; SDK page: https://modelcontextprotocol.io/docs/2026-07-28/sdk
- Primer confirming primitives/transports: https://www.getmaxim.ai/articles/the-mcp-protocol-transports-primitives-and-the-message-lifecycle

**Official Rust SDK = `rmcp` (VERIFIED via crates.io API):**

- Version **3.10.0**, published **2026-11-12**; crate created 2025-03-16; **50,855,726 total downloads, 29,759,830 recent**; 73 versions. https://crates.io/api/v1/crates/rmcp — repo: https://github.com/modelcontextprotocol/rust-sdk (contains `rmcp` + `rmcp-macros`)
- Load-bearing detail from its feature flags: **the `server` feature depends on `schemars`** — the official SDK itself uses schemars to derive tool input schemas. Other features: `transport-child-process`, `transport-io`, `transport-streamable-http-client`, `server-side-http`, `client-side-sse`, `auth` (OAuth2), `macros`, and **`skills-fs`** (cap-std + yaml_serde — sandboxed-filesystem support for the Agent Skills standard, see §6).
- A third-party alternative `rust-mcp-sdk` exists (https://crates.io/crates/rust-mcp-sdk) but `rmcp` is the official one under the modelcontextprotocol org.

**How existing video tools expose MCP:**

*Kineto* (https://github.com/DanielFidalgo/kineto — MIT OR Apache-2.0, Rust, CPU-only via tiny-skia): one binary ships CLI + MCP server; `npx kineto-mcp` or `cargo install kineto`. Its full tool table, with **published token costs**:

| Tool | Cost | Purpose |
|---|---|---|
| `check_document` | **~20 tokens** | "is it correct, and readable?" — no images |
| `preview_document` | **~390 tokens/frame** | how it looks |
| `build_chart` / `compile_session` / `session_append` / `build_changelog` | no pixels | generate documents from data |
| `render_document` / `render_asciicast` / `render_storyboard` | seconds + file | final output |

It also serves **resources**: the full JSON Schema at `kineto://schema/document` and seven example "shot types" at `kineto://example/`. The pipeline is explicitly "cheapest-first" — most agent turns cost no pixels at all.

*Clipkit* (https://www.clipkit.dev — JSON timeline → GPU MP4): MCP tools `create_project`, `add_element`, `edit_element`, `preview_still`, `validate_project`, `open_in_editor`, `render_video`; hosted MCP via Streamable HTTP at `https://www.clipkit.dev/mcp` (no key): `claude mcp add --transport http clipkit https://www.clipkit.dev/mcp`. `preview_still` exists specifically "so agents can see their own work" — i.e., agents need vision-in-the-loop, not just text validation.

*Note*: "FilmCraft" — not found in this subagent's searches (only Kineto, Clipkit, Kinocut https://github.com/kyanitelabs/kinocut, and mcp-video https://github.com/inematds/mcp-video). UNVERIFIED / possibly a hallucination from the research brief. (Parent note: the B-phase subagent reported it with specific star/commit counts; treat as UNVERIFIED and do not rely on it in the plan.)

## 2. JSON Schema + Structured Output

**`schemars` (VERIFIED via crates.io API):** version **1.2.3**, published **2026-10-12**, MIT, by Graham Esau; **661,281,510 total downloads (255.6M recent)**, 98 versions since 2019. `#[derive(JsonSchema)]` on serde types → JSON Schema; default features `derive`+`std`, plus `preserve_order`. https://crates.io/api/v1/crates/schemars

- For *validating* agent-submitted documents at runtime, the `jsonschema` crate (verified: actively maintained, updated 2026-11-12, https://crates.io/api/v1/crates/jsonschema) validates against a schema. Pattern: derive schema once with schemars, validate every incoming document with jsonschema before rendering.

**Why schema-first beats code-first for agents (Clipkit evidence):**

- Clipkit's 60-cell agent benchmark (5 briefs × 3 model tiers × 4 lanes, real agents): **100% valid videos for Clipkit vs 87% for Remotion** — https://www.clipkit.dev/compare/clipkit-vs-remotion. Root cause: "an invalid composition is caught by the validator with actionable errors, e.g. `elements.0.layer: Required`" *before* rendering; Remotion's failure mode is "code that typechecks can still crash the renderer" — the entire 13% gap.
- Token efficiency: Remotion agents used **median 1.63× more authoring tokens** (range 1.32–2.43×); homepage also claims ~40% fewer tokens vs Remotion TSX and ~53% fewer vs HyperFrames HTML over a 13-brief corpus. Render latency in the agent loop: 13.4s vs 33.7s median (compare page) / 3.4s vs 33.7s vs 107.2s native per-cell (homepage — slight page-to-page discrepancy, both cited). Cost per valid video: $0.89 vs $0.82 — parity.
- Corroborating context: OpenAI's structured-outputs docs state only schema-enforced outputs guarantee schema adherence (https://www.honeyhive.ai/learn/structured-output-llm); grammar engines like LLGuidance enforce JSON Schema as a CFG at decode time (https://arxiv.org/html/2606.09395v1). Design implication: publish the document schema so constrained-decoding runtimes can make first-try-valid output the default.

## 3. Agent Verification Loops (validate → preview → render)

All three researched tools converge on the same loop, ordered by cost:

1. **Static check (free/cheap)** — Kineto `check_document` at ~20 tokens catches what's "invisible in the JSON and obvious on screen": text invisible against background, elements animated off-canvas, text past the edge, scenes too short to read at 300 wpm. Exits nonzero on real errors; wired as default-on in its GitHub Action so bad documents fail CI. Clipkit's `validate_project` returns actionable, path-qualified errors (`elements.0.layer: Required`).
2. **Preview frame (~390 tokens/frame)** — Kineto `preview_document`, Clipkit `preview_still`. Vision feedback for the agent at ~1/10th the cost of a full render. Single-frame poster/thumbnail is a first-class output (`kineto scene.json -o poster.png --at 1500`).
3. **Final render (seconds + file)** — only after 1–2 pass.

The economic argument (Clipkit): a babysat 87%-valid pipeline costs more in human time than a 100%-valid one even at per-attempt token parity.

## 4. Sandboxing LLM/Script Output

Kineto's answer is the most radical: **documents are pure data with no execution model** — "a scene is JSON … there is no browser to launch, no Node runtime at render time, and nothing to sandbox; a single binary renders it in a container." Clipkit similarly states "zero model-authored code executing on user infrastructure." (Both from their sites above.)

If Quillon wants a scripting/DSL escape hatch, **Rhai** is the Rust-native fit (VERIFIED: v1.27.0, published 2026-10-21, 14.98M downloads, https://crates.io/api/v1/crates/rhai):

- Sandboxed by design: "a script can never read from outside its own environment"; a non-`mut` Engine is immutable and re-entrant; no filesystem/network unless the host registers functions (https://rhai.rs/book/safety/sandbox.html)
- DoS vectors documented — memory, CPU (infinite loops), stack, overflows, file-hammering — with mitigations (https://rhai.rs/book/safety/); never enable `unchecked` (disables all safety) or `internals` (exposes Rust internals). Panics outside documented conditions are treated as Rhai bugs.
- `mlua` (VERIFIED: 0.12.2, Lua 5.1–5.5/LuaJIT/Luau, 8.97M downloads) is popular but stock Lua is not sandbox-hardened the way Rhai is; Rhai's no_std embedded pedigree is the stronger default for untrusted input.

**Recommended pattern for Quillon (synthesis, consistent with all sources):** make the document format pure JSON (schema-validated, no execution) the primary agent surface; if Rhai scripts are accepted for effects/generators, run them as (a) pure functions `Document → Document` or `t → Element[]` producing JSON, (b) non-mut Engine with only registered drawing/math functions, (c) resource caps + operation limits, (d) output re-validated against the same schemars schema before anything renders. The script produces *data*, never side effects; validation remains the single trust boundary.

## 5. Deterministic / Reproducible Rendering

- **Kineto**: "byte-identical frames across native and WebAssembly" — 27/27 corpus frames, CI-enforced parity gate; golden sha256 hashes of *frame buffers* in `testdata/golden/hashes.json`. Time is integer ticks at 705,600,000/s (flicks) so 24/25/30/50/60 fps are all exact. Honest scope note: "the *frames* are byte-identical. The MP4 container is not — ffmpeg records its own version and thread count. Never promise reproducible MP4 bytes; promise reproducible pixels." (All from https://github.com/DanielFidalgo/kineto)
- **vidformer** (VERIFIED: crate 1.4.0, 2026-02-02, Apache-2.0, 14.5k downloads, https://crates.io/api/v1/crates/vidformer): the arXiv paper (2601.17221, Winecki & Nandi, Jan 2026, https://arxiv.org/abs/2601.17221) describes a cv2-compatible shim that executes scripts against symbolic frames, "recording a declarative specification of each output frame instead of rendering it" — letting the engine "optimize and parallelize rendering" like a database query planner, with just-in-time segment serving (VOD/HLS). Results: 2–3× faster full renders, 0.25–0.5s time-to-playback (~400×). The repo (https://github.com/ixlab/vidformer) even includes a **TLA+ model of the rendering engine** (`misc/tla`). The exact hyphenated term "frame-spec" as a name: UNVERIFIED (concept fully verified).
- Why it matters for agents: determinism turns "render frame 100 twice → identical bytes" into a cheap sha256 comparison, enabling (a) content-addressed frame caching, (b) visual-regression diffs of *documents* without watching videos, (c) golden-frame tests in CI, (d) Kineto's "video as a build artifact" CI loop where every release renders its own release video.

## 6. Distribution — the 2026 Standard for AI-Native Dev Tools

Observed stack, from real deployments:

1. **Rust binary containing both CLI and MCP server** (Kineto: `cargo install kineto` → `kineto` CLI + `kineto-mcp`; registered with `claude mcp add --scope user kineto "$(which kineto-mcp)"`; Kineto advises `--scope user` over project scope).
2. **npx wrapper for zero-install** (`npx kineto-mcp` — platform binaries fetched on first use; npm package kineto-mcp 0.2.0, https://libraries.io/npm/kineto-mcp-darwin-arm64). Clipkit goes further: hosted Streamable-HTTP MCP, no install or key.
3. **Agent Skills**: Anthropic open-sourced the spec Dec 2025; OpenAI adopted it for Codex CLI by Jan 2026 (https://www.termdock.com/en/blog/cross-agent-skills-new-npm). `npx skills add <owner/repo>` (vercel-labs/skills, Node ≥22.20, https://github.com/vercel-labs/skills) is the emerging npm-like installer; `npx skills find` indexes skills.sh. VS Code documents Agent Skills natively (https://code.visualstudio.com/docs/agent-customization/agent-skills). rmcp's `skills-fs` feature (cap-std) shows the MCP SDK itself is absorbing this standard.
4. **AGENTS.md** as the universal repo context file (README-for-agents), now shipped even by frameworks like Next.js via codemod (https://nextjs.org/docs/app/guides/ai-agents; overview: https://developers.redhat.com/articles/2026/07/27/standardize-project-context-agentsmd-and-agent-skills). CLAUDE.md remains a Claude-specific variant.
5. **GitHub Action** for CI usage (Kineto: `DanielFidalgo/kineto@v0.1.4`, checksum-verified binary download, check-on-by-default).

## 7. Recommended Agent-Tool Surface for Quillon

Modeled directly on Kineto's measured costs (~20-token checks, ~390 tokens/frame previews) and Clipkit's tool verbs. Estimated per-turn token costs assume ~3–5 tool-result tokens per check line, ~390 tokens per returned image frame (Kineto's measured figure), plus small JSON echo.

| Tool | Inputs | Outputs | Est. tokens |
|---|---|---|---|
| `validate_document` | document JSON | schema errors with JSON-paths + lint findings (contrast, off-canvas, read-time) | ~20–150 |
| `get_schema` / resource `quillon://schema/document` | — | JSON Schema (schemars-derived) | one-time ~1–3k |
| `preview_frames` | document + tick list (e.g. [0, 120, 480]) | PNG frames + frame sha256s | ~390/frame |
| `diff_documents` | two documents | semantic diff (scenes/elements/timings) | ~50–300 |
| `list_assets` / `list_fonts` | filter | available media/fonts/capsules with ids | ~50 |
| `probe_media` | path/URL | duration, resolution, fps, codecs | ~40 |
| `render_document` | document + format (mp4/webm/png) | file path, frame-count, per-frame hash manifest, render time | ~30 |
| `render_frame` | document + tick | single PNG + sha256 (for visual-regression assertions) | ~390 |
| `build_*` helpers | structured data (changelog, chart data, storyboard) | draft document | free (document-only) |
| `edit_media` | operation list (trim/crop/concat/mute) + input refs | new media ref + diff summary | ~50 |

Design rules derived from the research: cheapest-first pipeline (never render what a check can answer); every tool returns actionable JSON-path errors; previews are the only vision-bearing results; hashes on every frame make agent self-verification ("frame 100 unchanged after my edit") a 0-token string compare. Budget expectation from Clipkit data: ~$0.89/agent-authored video at parity, with JSON documents costing ~1.6× fewer tokens than code-based authoring.

## 8. Distribution Recommendation for Quillon

Ship all four, mirroring the verified 2026 pattern: (1) one Rust binary — CLI (`quillon render/check`) + MCP server (`quillon-mcp`), stdio transport, built on rmcp 3.10 + schemars 1.2.3; (2) an npx wrapper (`npx quillon-mcp`) for zero-install adoption; (3) a skills package installable via `npx skills add quillon/skills` + AGENTS.md in-repo; (4) optional hosted Streamable-HTTP MCP later (Clipkit's funnel model). Open-source the document format + schema (Clipkit open-sources CKP/1.0 as Apache-2.0 precisely so agents/ecosystem can adopt it independent of the engine).

## UNVERIFIED / caveats

- "FilmCraft" project: not found; possibly a misremembering of Kinocut/Clipkit.
- HyperFrames 67% validity: claimed on clipkit.dev homepage; not re-confirmed on the compare page (which only shows Clipkit 100% vs Remotion 87%).
- Clipkit render-time numbers differ between its homepage (3.4s vs 33.7s vs 107.2s native) and compare page (13.4s vs 33.7s agent-loop median) — both cited as published.
- Exact term "frame-spec" as vidformer's API name: UNVERIFIED; the mechanism (declarative per-frame specification recorded instead of rendering) is verified from the paper abstract and repo.
- Rhai's specific setter names (`max_operations` etc.): the sandbox and DoS-vector pages confirm the limits exist but the exact API names were not verified in this pass (they live deeper in the Rhai book).
