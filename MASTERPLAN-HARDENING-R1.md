# Quillon Hardening MasterPlan — Round 1 (2026-11-20)

Repo: AlbiFahriiza/quillon @ c96cff6 (branch main). Operator: tools
engineer under disciplined-coding 2.2 (Panoptes). This plan is the goal
contract for Round 1; Rounds 2 and 3 iterate with the same prompt.

## Scope

Harden the beta against hostile and malformed input on every external
channel. Non-goals: no refactors, no new dependencies, no feature work,
no changes to the document format's legal surface beyond new bounds
(each bound is itself a format change, accepted deliberately and
documented in STABILITY terms).

## Verification baseline (taken before any edit)

- `cargo build --workspace` OK
- `cargo test --workspace` 302 passed, 0 failed
- `cargo clippy --workspace --all-targets -- -D warnings` OK
- `cargo fmt --all -- --check` OK

Every step below must re-run all four after its edits. A step is done
only when all four are green AND its regression tests are observed
failing before the fix (watch-it-fail) and passing after.

## Findings driving the steps (verified by repro or code read)

| ID | Finding | Evidence |
| --- | --- | --- |
| F1 | Scene duration unbounded above; `total_duration()` uses plain `.sum()`; 3 scenes of i64::MAX/2 overflow | Repro: `quillon inspect` panics `attempt to add with overflow` (document.rs:538, lint.rs:164-165, audio_cmd.rs:42, preview.rs:146, pipeline.rs render loop) |
| F9 | Element x/y/w/h and clip x/y/w/h unbounded; `Bbox::rect` computes `x + w` unchecked; stretch fit casts i64→u32 (saturating) and can reach `resize(u32::MAX, u32::MAX)` | Repro: `quillon validate` panics at lint.rs:79 with w=h=i64::MAX image element; visuals.rs:159-160 `clip.x + ox + clip.w` same class |
| F2 | Subtitle import parses unbounded cue text and runs `regularize_cues` BEFORE validation | caption_cmd.rs:82-90 order; no caps in subtitle.rs |
| F3 | OTIO `to: from + duration` unchecked adds (otio.rs:174, 222); audio path `.unwrap_or(0)` swallows the named overflow error | code read |
| F4 | MCP `render_document` spawns one OS thread per call with no concurrency cap; `runs` map never pruned | mcp.rs:403-423 |
| F5 | Mux temp dir `/tmp/quillon-mux-<pid>` predictable; preview cache fixed name in shared /tmp | render_doc.rs:246, preview.rs:42 |
| F6 | Whisper model download has no timeout (ureq default) | transcribe.rs:99 |
| F7 | `audio analyze` `--silence-db` accepts NaN/inf; `--min-silence-ms` unbounded | main.rs:221-224 |

## Steps (checkpoint commit per step)

### S1 — Time bounds (F1)

- Add `MAX_SCENE_DURATION_TICKS` (24 h expressed in flicks — a real
  ceiling for scene-level work) and `MAX_DOCUMENT_DURATION_TICKS`
  (24 h; same value, the document total).
- `validate_scene`: duration must be `1..=MAX_SCENE_DURATION_TICKS`.
- `validate_document`: total duration (checked sum) within bound.
- `Document::total_duration()` and `scene_at()`: checked adds; on the
  impossible post-validation path, saturate rather than panic (defense
  in depth, documented).
- Regression tests: doc with 3 huge scenes → named error, exit 2 (CLI);
  inspect on bounded-huge doc must not panic; unit tests for the
  validator bounds.
- Checkpoint commit: `hardening(r1): bound scene and document durations`

### S2 — Geometry bounds (F9)

- Add `MAX_ELEMENT_GEOMETRY` (i64) = canvas_max*4 (30720). Validate
  element x, y, w, h and clip x, y, w, h within
  `-MAX_ELEMENT_GEOMETRY..=MAX_ELEMENT_GEOMETRY` (w,h also >= 1 as today).
- `Bbox::rect`: compute in f64 (lint is heuristic; no panic possible on
  bounded input, but f64 makes the class unreachable).
- Regression tests: huge w/h rect/image/video and clip → named errors;
  the F9 repro doc now exits 2 with pointers, never panics.
- Checkpoint commit: `hardening(r1): bound element and clip geometry`

### S3 — Subtitle input caps (F2)

- `parse_srt`/`parse_vtt`: cap cues per file (MAX_CUES_PER_TRACK = 5000,
  matching the validator's per-track cap) and cue text length
  (MAX_TEXT_LEN = 10_000, matching text elements). Named errors with
  line numbers.
- ASS parser (caption_cmd): same caps after rsubs-lib parse.
- Regression tests: SRT with 5001 cues → named error; cue with 10_001
  chars → named error.
- Checkpoint commit: `hardening(r1): cap subtitle input at the boundary`

### S4 — OTIO checked arithmetic (F3)

- `to: from.checked_add(duration)` → named error on overflow.
- Audio start/duration: propagate the error instead of `.unwrap_or(0)`.
- Regression test: OTIO with start near i64::MAX and positive duration
  → named error, not a wrapped `to`.
- Checkpoint commit: `hardening(r1): checked tick arithmetic in otio import`

### S5 — MCP render caps (F4)

- `MAX_CONCURRENT_MCP_RENDERS` (2): a counting semaphore-ish atomic
  check before spawning; excess calls get a named tool error telling
  the agent to poll existing runs.
- Prune `runs`: keep the 32 most recent entries after insert.
- Regression test (unit-level on the registry): 40 inserts → 32 kept.
- Checkpoint commit: `hardening(r1): cap concurrent MCP renders and prune the run registry`

### S6 — Temp dir hardening (F5)

- Mux temp dir: random suffix from process entropy
  (`std::process::id()` + a SystemTime nanos suffix), reject
  pre-existing dir (create fresh; on collision, error).
- Regression test: dir already exists → named error (unit).
- Checkpoint commit: `hardening(r1): unpredictable mux temp dir`

### S7 — Network + flag hardening (F6, F7)

- ureq agent with 30 s connect timeout for the model download.
- `--silence-db`: reject non-finite values; `--min-silence-ms`:
  reject 0..=1_000_000_000 bound violation.
- Regression tests: unit tests for the flag validators.
- Checkpoint commit: `hardening(r1): bound download time and analyze flags`

### S8 — Docs and contract lock

- Update docs/HARDENING.md: new guards table rows, the limits list.
- Extend tests/contract.rs only if any exit code changed (none planned).
- Extend the fuzz generator with huge duration/geometry numbers.
- Checkpoint commit: `hardening(r1): document the r1 guards`

## Verification rules (apply after every step)

1. `cargo test --workspace` — compare against the 302-test baseline;
   no test deleted or loosened; new failures are mine to fix.
2. `cargo clippy --workspace --all-targets -- -D warnings`.
3. `cargo fmt --all -- --check`.
4. The step's regression test must have been observed failing before
   the fix (revert-check or pre-fix run) and passing after.
5. `git diff` read line-by-line: nothing out of scope, no debug output.
6. Golden hashes unchanged (no renderer edits in this round).

## Definition of done (Round 1)

All eight checkpoint commits on a branch, battery green, both repro
docs (`overflow.json`, `stretch2.json`) now producing named errors with
exit 2 and no panic anywhere in the battery, docs updated, findings
list cross-checked. Push to main only with the user's go-ahead
(Panoptes: commits/pushes need explicit approval — the task says
"checkpoint commit", so commits are authorized, push is not).
