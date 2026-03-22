# timebomb — CLAUDE.md

This file gives Claude (and any other AI coding assistant) the context needed to work effectively in this repository. Read it before making changes.

---

## What this project is

`timebomb` is a Rust CLI tool that scans source code for structured expiry annotations (`TODO[2026-06-01]: ...`) and fails with a non-zero exit code in CI when any deadline has passed. It is designed to enforce the social contract that temporary code actually gets removed.

---

## Build, test, and lint commands

```sh
cargo build                  # compile (dev profile)
cargo build --release        # compile (optimised)
cargo test                   # run all unit + integration tests
cargo test -- --nocapture    # show eprintln! output during tests
cargo clippy -- -D warnings  # lint; CI fails on any warning
cargo fmt                    # format; CI checks with --check
```

All four commands must pass cleanly before any PR is merged. There is no separate script — just `cargo`.

---

## Project layout

```
src/
  main.rs        CLI entrypoint; resolve_config(); subcommand dispatch; exit codes
  cli.rs         clap derive structs: Cli, CheckArgs, ListArgs, FormatArg
  config.rs      .timebomb.toml loading, CLI overlay merging, glob exclusion, extension filter
  scanner.rs     scan(), scan_file(), scan_content(), build_regex(), is_binary()
  annotation.rs  Annotation struct, Status enum, compute_status()
  output.rs      Terminal / JSON / GitHub Actions formatters
  error.rs       Error enum, Result alias, parse_duration_days()
  lib.rs         Public re-exports (makes src/ importable from tests/)

tests/
  scanner_tests.rs    Integration tests against fixture files
  config_tests.rs     Integration tests for config loading and merging
  fixtures/
    sample.rs         Rust source with known mix of expired/expiring-soon/future annotations
    sample.py         Python source, same mix
    sample.sql        SQL source, same mix
```

---

## Key architecture decisions

### `today` is injected, never fetched internally

`scan()`, `scan_content()`, and `Annotation::compute_status()` all accept `today: NaiveDate` as a parameter. "Today" is derived once in `main.rs` at startup and threaded through. This makes every test deterministic without mocks or time-travel hacks.

### Regex compiled once

`build_regex(config)` is called once in `scan()` before the walk loop. The resulting `Regex` is `Send + Sync` and is shared (by reference) across all rayon worker threads. Never compile the regex inside `scan_file` or `scan_content`.

### Three-phase scan pipeline

`scan()` is structured in three explicit phases:

1. **Serial walk** — `WalkDir` collects candidate `(abs_path, rel_path)` pairs after applying exclude globs, extension filter, and binary detection.
2. **Parallel scan** — `candidates.par_iter().map(scan_file)` via rayon. Each worker reads one file and returns `Vec<Annotation>`. No shared mutable state.
3. **Serial flatten + sort** — flatten the per-file vecs, sort by `NaiveDate` ascending.

If you restructure `scan()`, preserve this boundary so the rayon step stays pure.

### Config merging order

`Config` is resolved in `main.rs` via `resolve_config()`:
1. Look for `--config <file>` (explicit override).
2. Look for `.timebomb.toml` in the scan directory.
3. Fall back to `.timebomb.toml` in CWD.
4. If no file found, use `Config::default()` silently.

CLI flags (e.g. `--warn-within`, `--fail-on-warn`) are applied on top as `CliOverrides` after file loading. CLI always wins over file.

### Exit codes

| Code | Meaning |
|------|---------|
| 0 | No expired annotations |
| 1 | One or more expired annotations found (or `--fail-on-warn` triggered) |
| 2 | Configuration or runtime error |

`list` **always** exits 0 — it is purely informational. Only `check` uses exit code 1.

---

## Annotation format

```
// TODO[2026-06-01]: message
# FIXME[2026-03-15][alice]: message with owner
```

The regex (built in `Config::annotation_regex_pattern()`):

```
(?i)(TODO|FIXME|HACK|TEMP|REMOVEME)\[(\d{4}-\d{2}-\d{2})\](\[([^\]]+)\])?:\s*(.+)
```

Capture groups: `[1]` tag, `[2]` date, `[4]` optional owner, `[5]` message.

Plain `// TODO: fix this` comments (no bracket-date) are intentionally ignored.

---

## Test fixtures

Fixture files in `tests/fixtures/` use **hardcoded, date-independent values**:

- **Expired**: dates in 2018–2021 (always in the past)
- **Expiring-soon**: dates in mid-2025 (treat as "recently expired" in tests; use a wide `warn_within` window)
- **Future / OK**: dates in 2088 or 2099 (always in the future)

Never use relative dates like "30 days from today" in fixture files. Tests must not depend on the wall clock.

---

## Output formats

Three formats are supported, selected via `--format` or auto-detected:

| Format | Trigger |
|--------|---------|
| `terminal` | Default; respects `NO_COLOR` env var |
| `json` | `--format json` |
| `github` | `--format github` or `GITHUB_ACTIONS=true` env var |

GitHub Actions format emits `::error` and `::warning` annotation lines. Terminal format uses `colored` for red/yellow/green status prefixes.

---

## Known performance considerations

Two performance reviews were conducted by agent review. Key findings to be aware of:

1. **`is_binary` runs serially in Phase 1**, meaning every candidate file is opened twice (once for binary check, once in `scan_file`). This serialises what could be parallel I/O. A future improvement is to move binary detection into Phase 2 inside `scan_file` by reading raw bytes with `std::fs::read`, checking for null bytes, then decoding — eliminating the double open.

2. **Missing line-level pre-filter in `scan_content`**: `regex.captures_iter()` is called on every line even when the line cannot possibly match (no `[` character). Adding `if !line.contains('[') { continue; }` before the regex call eliminates the regex engine overhead on the ~99.9% of lines that have no annotation.

3. **Allocations before date validation**: In `scan_content`, `tag.to_uppercase()`, `message.to_string()`, and `owner.to_string()` are called before `NaiveDate::parse_from_str`. On invalid-date lines these allocations are immediately discarded. Moving date parsing above the string allocations makes the error path alloc-free.

4. **`sort_by_key` vs `sort_unstable_by_key`**: The final sort in Phase 3 uses stable sort. Since there is no meaningful tiebreaker for equal dates, `sort_unstable_by_key` is a free speed improvement.

5. **`BufReader` in `is_binary`**: Wraps a single 8 KB read — the `BufReader` adds overhead without benefit. A plain `File::read` into a stack buffer is sufficient.

These are documented for awareness; they are not yet fixed. Address them in priority order (2 → 3 → 1 → 4 → 5) if performance becomes a bottleneck.

---

## Dependencies — rationale

| Crate | Why |
|-------|-----|
| `clap` (derive) | Argument parsing with minimal boilerplate |
| `walkdir` | Reliable, cross-platform recursive directory walking |
| `regex` | Compiled, reusable annotation pattern matching |
| `chrono` | `NaiveDate` arithmetic for expiry calculation |
| `toml` + `serde` | `.timebomb.toml` deserialization |
| `serde_json` | JSON output format |
| `globset` | Fast glob matching for exclude patterns |
| `colored` | Terminal color; respects `NO_COLOR` automatically |
| `rayon` | Data-parallel file scanning in Phase 2 |
| `tempfile` (dev) | Isolated temp directories in integration tests |

Do not add new dependencies without a clear justification. Prefer extending existing ones.

---

## Things to avoid

- **Do not call `std::process::exit` outside `main.rs`**. Library code must return `Result`; exit codes are resolved only at the top level.
- **Do not fetch the current date inside the scanner**. Always use the injected `today: NaiveDate`.
- **Do not compile the regex inside `scan_file` or `scan_content`**. It must be compiled once in `scan()` and passed by reference.
- **Do not make `list` exit non-zero**. It is informational only.
- **Do not add language-specific parsers**. The scanner is intentionally language-agnostic — it matches tag patterns anywhere on a line regardless of comment syntax.
```

Ignoring the model's response since it was prompted directly.