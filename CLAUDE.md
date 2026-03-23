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
  cli.rs         clap derive structs: Cli, CheckArgs, ListArgs, FixArgs, BaselineArgs, FormatArg
  config.rs      .timebomb.toml loading, CLI overlay merging, glob exclusion, extension filter
  scanner.rs     scan(), scan_file(), scan_content(), build_regex(), is_binary()
  annotation.rs  Annotation struct, Status enum, compute_status()
  output.rs      Terminal / JSON / GitHub Actions formatters
  error.rs       Error enum, Result alias, parse_duration_days()
  blame.rs       git blame integration for --blame enrichment
  hook.rs        Pre-commit hook install / uninstall
  trend.rs       Report snapshot comparison (trend command)
  report.rs      Report JSON generation and writing
  stats.rs       Aggregate stats by owner / tag
  init.rs        timebomb init command
  add.rs         timebomb add command (insert annotations)
  snooze.rs      timebomb snooze command (bump deadlines in-place)
  fix.rs         timebomb fix command (interactive expired annotation resolution)
  diff.rs        Unified diff parsing for --changed mode
  baseline.rs    Baseline save/show/ratchet enforcement
  git.rs         Git helpers (validate_git_ref, changed files, repo detection)
  lib.rs         Public re-exports (makes src/ importable from tests/)

tests/
  scanner_tests.rs    Integration tests against fixture files
  config_tests.rs     Integration tests for config loading and merging
  fix_tests.rs        Integration tests for the fix command
  diff_tests.rs       Integration tests for diff parsing / --changed mode
  baseline_tests.rs   Integration tests for baseline ratchet enforcement
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

Helper regexes used in other modules (`snooze.rs`, `diff.rs`) are cached as `std::sync::LazyLock<Regex>` statics (stable since 1.80) so they are compiled at most once per process.

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
| 1 | One or more expired annotations found (or `--fail-on-warn` triggered, or ratchet exceeded) |
| 2 | Configuration or runtime error |

`list` and `fix` **always** exit 0 — they are informational/interactive. Only `check` uses exit code 1.

### `fix` command — two-pass interactive resolution

`fix` collects all user decisions in a first pass (interactive prompts), then applies them in a second pass bottom-up by descending line number per file. This avoids line-shift bugs that would occur if edits were applied during the prompt loop. It reuses `snooze::snooze_line` for Extend and `remove::remove_line` for Delete.

### `--changed` mode — diff-aware filtering

`check --changed --base <ref>` runs the normal scan, then filters results to annotations whose file+line appear in the changed line ranges returned by `diff::git_changed_line_ranges`. The diff parser (`parse_unified_diff`) is a pure function that takes a `git diff --unified=0` string and returns `HashMap<PathBuf, Vec<RangeInclusive<usize>>>`. Both staged and unstaged diffs are merged.

### Baseline ratchet

`baseline save` writes `.timebomb-baseline.json` with the current expired and expiring-soon counts. `check` loads this file (if present) and calls `check_ratchet` — a pure function returning a `Vec<String>` of violation messages. Four independent checks: `max_expired` ceiling, `max_expiring_soon` ceiling, regression vs. baseline expired, regression vs. baseline expiring-soon. Any violation causes `check` to exit 1.

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

## Performance notes

These were reviewed and addressed. Remaining known cost:

- **`is_binary` runs serially in Phase 1** — every candidate file is opened once for binary detection, then again in Phase 2 via `fs::read`. The double-open is a known tradeoff; moving binary detection into Phase 2 would require reading the full file before knowing whether to skip it.

The following were already fixed:
- Line-level `[` pre-filter in `scan_content` before the regex is applied
- Allocations deferred until after date validation in `scan_content`
- `OnceLock` caches for regexes in `snooze.rs` and `diff.rs`
- Single-pass fold for expired/warning/ok counts in `output.rs`
- Single-buffer file reconstruction in `snooze.rs` and `fix.rs` (no per-line `String` allocs)
- `sort_unstable_by_key` for the final Phase 3 sort
- `HashSet<&str>` instead of `HashSet<String>` for membership lookups in `trend.rs`
- Iterator `.next()` instead of `collect::<Vec<_>>()` for two-field destructure in `blame.rs`
- Char-safe `char_indices().nth(N)` instead of byte-slice truncation in `stats.rs`

---

## Dependencies — rationale

| Crate | Why |
|-------|-----|
| `clap` (derive) | Argument parsing with minimal boilerplate |
| `walkdir` | Reliable, cross-platform recursive directory walking |
| `regex` | Compiled, reusable annotation pattern matching |
| `chrono` | `NaiveDate` arithmetic for expiry calculation |
| `toml` + `serde` | `.timebomb.toml` deserialization |
| `serde_json` | JSON output format and baseline file I/O |
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
- **Do not make `list` or `fix` exit non-zero**. They are informational/interactive only.
- **Do not add language-specific parsers**. The scanner is intentionally language-agnostic — it matches tag patterns anywhere on a line regardless of comment syntax.
- **Do not apply edits top-down in `fix` or `snooze`**. Always apply bottom-up (descending line number) to avoid line-shift bugs when multiple annotations are in the same file.
