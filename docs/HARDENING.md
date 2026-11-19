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

## The limits, in one place

- `MAX_CSV_ROWS = 1_000_000` data rows (`quillon-cli/src/csv.rs`)
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
