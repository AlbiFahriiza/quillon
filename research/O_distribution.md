# O. Distribution Standards Notes — for Quillon

*Research date: 2026-11-17. Search findings; cross-checked with research/C §6.*

## Agent Skills (the 2026 standard)

- Agent Skills is the open format: a **self-contained directory centered on a `SKILL.md`** with YAML frontmatter (name + description) plus optional scripts/templates. Adopted by Anthropic (Claude Code, spec open-sourced Dec 2025), then OpenAI (ChatGPT + Codex CLI, reported Dec 2025), and supported across Cursor, Copilot, Antigravity and others by Sep 2026 (getclaudeskills.com coverage).
- Installers: `npx skills add <owner/repo>` (vercel-labs/skills, Node ≥22.20) is the emerging npm-like installer; `npx skills find` indexes skills.sh. VS Code documents Agent Skills natively.
- The MCP SDK itself absorbed the standard: rmcp ships a `skills-fs` feature (cap-std + yaml_serde) for sandboxed-filesystem skill access (verified from rmcp's feature flags, research/C §1).

## Quillon distribution plan (final shape for the plan doc)

1. **One Rust binary** = CLI (`quillon render|check|preview|inspect|probe|audio|snapshot`) + stdio MCP server (`quillon mcp`, rmcp, stdio-only per research/I §2).
2. **cargo-dist releases** (0.34.0 verified active): 5-target matrix, checksummed GitHub Release artifacts, installer script (research/L §2).
3. **Skills package**: `skills/quillon/SKILL.md` in-repo (frontmatter + the validate→preview→render loop + schema pointers), installable via `npx skills add AlbiFahriiza/quillon`.
4. **AGENTS.md** in repo root — universal agent context (the 2026 convention; Next.js ships one via codemod, Red Hat documents the standard).
5. **crates.io**: `quillon` (CLI) + `quillon-core` (document model) etc. — names verified free (research/N).
6. **Schema** published: `quillon://schema/document` MCP resource + `docs/schema.json` in repo, CC-BY.

## Why this exact stack (evidence chain)

- Kineto/Clipkit prove binary+MCP+skills is the 2026 adoption pattern for AI-native media tools (research/C §6, research/D §2).
- stdio-only keeps the security posture honest (one user, local, OS trust — archestra rules, research/I §2).
- AGENTS.md + SKILL.md make the tool teachable to agents without the agent reading our whole repo.
