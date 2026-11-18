# I. Scale & Transport Decisions — for Quillon

*Research date: 2026-11-17. Direct reads by the lead agent.*

## 1. Parallel render distribution — Remotion Lambda's curve (source: https://www.remotion.dev/docs/lambda/concurrency)

- Distribution model: video → **frame-range chunks** rendered concurrently → rejoined. Concurrency = `frameCount / framesPerLambda`; chunks balanced evenly (`ceil` division, "21 frames over 2 lambdas → 11 + 10").
- Battle-tested defaults worth adapting: concurrency target interpolates **75 → 150** as frame count goes 0 → 18,000 (10 min @ 30fps); **hard floor of 20 frames per worker** (min 5); **hard cap of 200 workers** ("more would result in diminishing returns").
- `concurrency: 1` renders in-process on the orchestrator (v4.0.517+) — the equivalent of Quillon's single-process rayon path.
- Quillon application: a `--jobs N` flag with an auto curve (min(cores * k, 200-equivalent), floor on chunk size so per-worker startup costs amortize); workers are threads (rayon) in v0.x, shareable processes later. The chunk/rejoin model maps directly onto content-addressed frame keys (research/G §1).

## 2. MCP transport choice (source: https://archestra.ai/blog/stdio-vs-streamable-http-mcp)

- Rule: **stdio if exactly one person reaches the server** (local CLI case — the client spawns it, OS-level trust, no listener, dies with the client). **Streamable HTTP with OAuth from day one** the moment a second human/agent reaches it.
- "We'll switch later" fails empirically — the migration needs auth built from scratch, breaks single-process state assumptions, and "almost never happens"; laptop demos end up serving production.
- Never extend a stdio server into shared use: the properties that make it nice locally (no listener, no auth, no shared state) are exactly what kill it when shared.
- Quillon application: `quillon mcp` ships **stdio only** (local, single-user, spawned by the agent client). A hosted Streamable-HTTP endpoint is a separate future service with its own auth — not a config flag on the same binary. This matches Kineto's shape (CLI + stdio MCP in one binary) and keeps the v0.x scope honest.

## 3. Combined decision log for the plan

| Decision | Choice | Source |
|---|---|---|
| Frame distribution | frame-range chunks, balanced; auto concurrency with floor(20 frames/worker) + cap; `--jobs` override | Remotion Lambda concurrency docs |
| Worker model v0.x | rayon threads in-process; `--jobs 1` = orchestrator-only | Remotion Lambda v4.0.517 precedent + matklad/xdtask pragmatism |
| Cache layout | content-addressed, shareable (no machine-local state in keys) | research/G §1 + farm-compatibility (research/H) |
| MCP transport | stdio only in the binary; hosted HTTP = separate service, later | archestra decision rules |
| Preview | localhost web page (tokio + notify), same `render_tick` path | research/G §5 |
| GPU | adapter behind `render_tick`, excluded from golden-hash guarantees | research/H §2 |
| Captions | whisper-rs + own timing regularization + rsubs-lib (ASS) / own SRT+VTT | research/F §5 + research/H subtitle table |
