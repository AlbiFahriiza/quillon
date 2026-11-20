# Hardening: guards, limits, and how to extend them

Quillon's hardening posture: every external input channel either
returns a value, returns a **named error**, or is rejected by a
**bound** — it never panics, never saturates silently, and never
writes outside the directories the user named. Each guard below ships
with a regression test that failed before the fix (watch-it-fail) and
runs in CI on every platform.

## Where the guards live

| Channel | Guard | Test |
| --- | --- | --- |
| Cache frame files on disk (corrupt/crafted headers) | non-zero dimensions + checked `w*h*4` math; anything else is a cache miss | `quillon-cache` `corrupt_frame_files_are_misses_not_panics` |
| All core parsers (`validate_str`, SRT/VTT, OTIO, template) | deterministic fuzz: 10k seeded inputs in CI, 100k behind `--ignored`; seed printed on panic | `quillon-core/tests/fuzz_smoke.rs` |
| `--out` pattern filled from CSV rows | row values with `/`, `\` or `..` are named errors (no escaping the output directory); empty fill is an error; absolute output dirs the user typed stay legal | `template.rs` tests + CLI `traversal_values_in_the_out_pattern_are_rejected` |
| CSV input | UTF-8 BOM stripped; field > 1 MiB named error; > 1,000,000 rows fails early (checked inside the loop) | `csv.rs` tests |
| OTIO durations | values that overflow the timeline are named errors, not saturated `as i64` casts | `otio.rs` `absurd_durations_are_named_errors_not_saturated_casts` |
| Template semantics | one substitution pass (row values are never re-interpolated); markers tolerate inner whitespace; object keys are never substituted | `template.rs` tests |
| Render misuse | `--workers 0` and a directory as the output path are named errors before anything is written | CLI `zero_workers_...`, `a_directory_as_the_output_path_...` |
| Output writing | `otio import` and `caption export` create the output's parent directory, same contract as `render` | `tests/contract.rs` |
| CLI exit codes | table-locked: 0 success · 1 operational · 2 invalid document · 3 lint findings; `--json` stdout always parses on success | `tests/contract.rs` |
| Scene and document durations (r1 F1) | per-scene and total capped at 24 h in ticks with named errors; `total_duration`/`scene_at` saturate for library use | `validate_pipeline.rs` duration tests |
| Element and clip geometry (r1 F9) | x/y/w/h bounded to ±4x the widest canvas; lint bbox and clip math are overflow-proof (f64/saturating) | `validate_pipeline.rs` geometry tests |
| Subtitle files (r1 F2) | ≤ 5000 cues and ≤ 10 kB per cue at the SRT/VTT/ASS read boundary, before regularization runs | `subtitle.rs` `r1_caps_tests` |
| OTIO ranges (r1 F3) | `start + duration` checked on video and audio paths; overflow is a named error | `otio.rs` `start_plus_duration_overflow_is_a_named_error` |
| MCP renders (r1 F4) | ≤ 2 concurrent renders, registry pruned to the 32 newest (never a live run) | `mcp.rs` |
| Mux temp dir (r1 F5) | unpredictable pid+nanos name; refuses an existing path (symlink pre-planting) | `render_doc.rs` |
| Download and flags (r1 F6, F7) | whisper download has a 30 s connect timeout; `--silence-db` rejects NaN/inf, `--min-silence-ms` bounded 1..=3 600 000 | `cli.rs` flag tests |
| MCP path resolution (r2 R2-1) | caller paths with a root/prefix component or a Windows drive/UNC shape are named errors — `Path::join` would otherwise replace the working root | `mcp.rs` `probe_media_rejects_absolute_paths` |
| Input file size (r2 R2-2) | documents, OTIO timelines and subtitle files enter through a 64 MiB read ceiling, CSVs through 512 MiB; the size is checked before the read, and re-checked after against a file that grew | `read_capped.rs` + `caption.rs` wiring test |
| Decoded audio (r2 R2-3) | the decoder stops appending past 2^31 interleaved samples and names the ceiling instead of allocating without bound | `quillon-audio` `r2_cap_tests` |
| MCP render cap regression (r2 R2-4) | the r1 two-render cap is locked by a test that renders two real documents, rejects the third, waits, and accepts the fourth | `mcp.rs` `concurrent_render_cap_rejects_the_third_run_and_frees_slots` |
| Preview encode failures (r2 R2-5) | a failed draft PNG encode prints to stderr and the sample is skipped — never a silent zero-byte frame | `preview.rs` |
| Whisper staging file (r2 R2-6) | the `.part` download stage carries a pid+nanos suffix (the r1 F5 pattern), so a pre-planted symlink cannot predict it | `transcribe.rs` |
| Installer (r2 R2-7) | the binary lands via a staging copy + rename, so a racing or interrupted install cannot leave a half-written `~/.local/bin/quillon` | `installer.sh` |
| CI supply chain (r2 R2-8) | every workflow `uses:` is pinned to the commit SHA its tag resolved to on 2026-11-20 | `.github/workflows/*.yml` (branch `hardening/r2-workflows`) |
| Subtitle timestamps (r3 F1, F2) | the hours field is capped at 100 (minutes, seconds and milliseconds always were checked) and the ms→tick conversion is checked — a crafted giant-hours file is a named error, not an overflow panic | `subtitle.rs` `r3_timestamp_tests` |
| Zero-fps pipeline start (r3 F3) | `fps == 0` is refused before `frame_tick` divides — a library-built Document can no longer panic the pipeline | `pipeline.rs` `r3_zero_fps_is_a_named_error_not_a_divide_by_zero` |
| Sample conversion and mix size (r3 F4) | tick→sample is computed in i128 (a valid 24 h @ 192 kHz document used to wrap negative) and the flat mix buffer is refused past 2^32 interleaved samples | `quillon-audio` `r3_mix_tests` |

## The limits, in one place

- `MAX_CSV_ROWS = 1_000_000` data rows (`quillon-cli/src/csv.rs`)
- `MAX_TEXT_FILE_BYTES = 64 MiB` for documents, OTIO and subtitle files; `MAX_CSV_FILE_BYTES = 512 MiB` for mail-merge data (`quillon-cli/src/read_capped.rs`, r2 R2-2)
- `MAX_DECODED_SAMPLES = 2^31` interleaved f32 (~8 GiB, ~6 h stereo at 48 kHz) (`quillon-audio/src/lib.rs`, r2 R2-3)
- `MAX_MIX_SAMPLES = 2^32` interleaved f32 (~34 GB stereo) (`quillon-audio/src/lib.rs`, r3 F4)
- `MAX_TIMESTAMP_HOURS = 100` per subtitle timestamp (`quillon-core/src/subtitle.rs`, r3 F1)
- `MAX_SCENE_DURATION_TICKS = MAX_DOCUMENT_DURATION_TICKS = 24 h` (`quillon-core/src/lib.rs`, r1 F1)
- `MAX_ELEMENT_GEOMETRY = ±30 720 px` for element and clip x/y/w/h (r1 F9)
- `MAX_PARSED_CUES = 5_000`, `MAX_CUE_TEXT_BYTES = 10 000` per subtitle file (r1 F2)
- `MAX_FIELD_BYTES = 1 MiB` per CSV field (same file)
- Document bounds (scenes, elements, assets, dimensions, fps, text
  length, …) live in `quillon-core/src/lib.rs` (`MAX_*` constants) and
  are enforced by `validate` with named errors.

Every limit error names the limit and the actual value.

## How to add a guard

1. Find the input channel and write the failing test first: an input
   that would panic, escape, saturate, or hang on today's code. Watch
   it fail (run the test against the pre-fix code).
2. Add the smallest check at the boundary — a named error, a checked
   operation, or a bound constant next to the others.
3. Lock the contract: if it changes an exit code or output shape, add
   a row to `tests/contract.rs`.
4. Extend the fuzz generator (`tests/fuzz_smoke.rs`) with characters or
   shapes that exercise the new channel.
5. Run the battery: `cargo fmt --check && cargo clippy --workspace
   --all-targets -- -D warnings && cargo test --workspace`.

## Non-goals

Guards confine untrusted input (documents, CSV rows, OTIO files,
cache files). They never second-guess paths and values the user typed
on their own command line: an absolute `--out`, a `..` in a pattern
the user wrote, or `--allow-lint` are the user's calls to make.

## The three hardening rounds

| Round | PR / branch | Focus | Closed | Tests |
| --- | --- | --- | --- | --- |
| 1 | PR #1, `hardening/r1` | first audit: durations, geometry, subtitle volume, OTIO sums, MCP render caps, temp dirs, download/flag guards | F1–F9 | 302 → 313 |
| 2 | PR #2, `hardening/r2-core` | areas R1 left untouched: import read ceilings, MCP path confinement, decode caps, render-cap regression, installer/workflow hygiene | R2-1–R2-7 (R2-8 on `hardening/r2-workflows`, blocked on a `workflow`-scoped token) | 313 → 322 |
| 3 | `hardening/r3` | cross-verification of the r1/r2 guards plus the core modules both rounds left alone: timestamp arithmetic, pipeline start, sample conversion, mix size | R3-F1–F4 (R3-F5 documented as defense-in-depth) | 322 → 329 |

Method, fixed across the rounds: a 30-minute timed review with a
Python timer, a master plan committed as the goal, watch-it-fail
regression tests, checkpoint commits per finding, and the full
battery (`cargo test --workspace`, `clippy -D warnings`, `fmt
--check`, unchanged golden hashes) green after every step. The
cross-verification pass in round 3 probed the r1/r2 guards for
bypasses (path shapes, cap arithmetic) and re-ran the 100k-input
fuzz locally; no regression survived.

