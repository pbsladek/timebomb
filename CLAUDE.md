# timebomb — CLAUDE.md

This file gives Claude (and any other AI coding assistant) the context needed to work effectively in this repository. Read it before making changes.

---

## What this project is

`timebomb` is a Rust CLI tool that scans source code for structured expiry annotations (`TODO[2026-06-01]: ...`) and fails when any deadline has passed. It is designed to enforce the social contract that temporary code actually gets removed.

---

## Build, test, and lint commands

```sh
cargo build                  # compile (dev profile)
cargo build --release        # compile (optimised)
cargo test                   # run all unit + integration tests
cargo test -- --nocapture    # show eprintln! output during tests
cargo clippy -- -D warnings  # lint; CI fails on any warning
cargo fmt                    # format; CI checks with --check
make smoke                   # end-to-end smoke tests against the release binary
```

All five must pass cleanly before any PR is merged.

---

## Project layout

```
src/
  main.rs        CLI entrypoint; resolve_config(); resolve_fuse_arg(); file_matches();
                 status_order(); subcommand dispatch; exit codes
  cli.rs         clap derive structs: Cli, Command, SweepArgs, ManifestArgs, …,
                 FormatArg, SortBy, GroupBy, CompletionsArgs; re-exports clap_complete::Shell
  config.rs      .timebomb.toml loading, CLI overlay merging, glob exclusion, extension filter
  scanner.rs     scan(), scan_file(), scan_content(), build_regex(), is_binary()
  annotation.rs  Fuse struct, Status enum (Detonated/Ticking/Inert), compute_status()
  output.rs      Terminal / JSON / CSV / GitHub Actions formatters;
                 age_col() compact age column; print_csv_list(); write_json_report()
  error.rs       Error enum, Result alias, parse_duration_days()
  blame.rs       git blame integration for --blame enrichment
  hook.rs        Pre-commit tripwire install / uninstall
  trend.rs       Report snapshot comparison (fallout command)
  report.rs      Report JSON generation and writing
  stats.rs       Aggregate stats by owner / tag / month (intel command);
                 compute_stats(), print_stats(), print_stats_month()
  init.rs        timebomb init command
  add.rs         timebomb plant command (insert fuses)
  snooze.rs      timebomb delay command (bump deadlines in-place)
  fix.rs         timebomb defuse command (interactive detonated fuse resolution)
  diff.rs        Unified diff parsing for --since/--changed mode
  baseline.rs    Bunker save/show/ratchet enforcement
  git.rs         Git helpers (validate_git_ref, changed files, repo detection)
  lib.rs         Public re-exports (makes src/ importable from tests/)

tests/
  scanner_tests.rs    Integration tests against fixture files
  config_tests.rs     Integration tests for config loading and merging
  fix_tests.rs        Integration tests for the defuse command
  diff_tests.rs       Integration tests for diff parsing / --since mode
  baseline_tests.rs   Integration tests for bunker ratchet enforcement
  fixtures/           One sample.* file per supported language extension
```

---

## Naming — the bomb theme

Everything in the codebase uses bomb/explosion terminology. Key mappings:

| Concept | Name in code |
|---------|-------------|
| Annotation / TODO comment with a date | **fuse** (`Fuse` struct) |
| Past-due fuse | **detonated** (`Status::Detonated`) |
| Fuse within the warning window | **ticking** (`Status::Ticking`) |
| Fuse safely in the future | **inert** (`Status::Inert`) |
| Number of files scanned | **swept_files** |
| Scan and fail in CI | **sweep** (subcommand) |
| List all fuses | **manifest** (subcommand) |
| Insert a fuse | **plant** (subcommand) |
| Bump a deadline | **delay** (subcommand) |
| Remove a fuse | **disarm** (subcommand) |
| Stats by owner/tag/month | **intel** (subcommand) |
| Pre-commit hook | **tripwire** (subcommand: `set` / `cut`) |
| Compare two snapshots | **fallout** (subcommand) |
| Interactive resolve detonated fuses | **defuse** (subcommand) |
| Baseline ratchet | **bunker** (subcommand: `save` / `show`) |
| Shell completion scripts | **completions** (subcommand) |
| Warning window (days) | **fuse_days** (config key) |
| Max detonated ceiling | **max_detonated** (config key) |
| Max ticking ceiling | **max_ticking** (config key) |

---

## Key architecture decisions

### `today` is injected, never fetched internally

`scan()`, `scan_content()`, and `Fuse::compute_status()` all accept `today: NaiveDate` as a parameter. "Today" is derived once in `main.rs` at startup and threaded through. This makes every test deterministic without mocks or time-travel hacks.

### Regex compiled once

`build_regex(config)` is called once in `scan()` before the walk loop. The resulting `Regex` is `Send + Sync` and is shared (by reference) across all rayon worker threads. Never compile the regex inside `scan_file` or `scan_content`.

Helper regexes used in other modules (`snooze.rs`, `diff.rs`) are cached as `std::sync::OnceLock<Regex>` statics so they are compiled at most once per process.

### Three-phase scan pipeline

`scan()` is structured in three explicit phases:

1. **Serial walk** — `WalkDir` collects candidate `(abs_path, rel_path)` pairs after applying exclude globs, extension filter, and binary detection.
2. **Parallel scan** — `candidates.par_iter().map(scan_file)` via rayon. Each worker reads one file and returns `Vec<Fuse>`. No shared mutable state.
3. **Serial flatten + sort** — flatten the per-file vecs, sort by `NaiveDate` ascending.

If you restructure `scan()`, preserve this boundary so the rayon step stays pure.

### Config merging order

`Config` is resolved in `main.rs` via `resolve_config()`:
1. Look for `--config <file>` (explicit override).
2. Look for `.timebomb.toml` in the scan directory.
3. Fall back to `.timebomb.toml` in CWD.
4. If no file found, use `Config::default()` silently.

CLI flags (e.g. `--fuse`, `--fail-on-ticking`) are applied on top as `CliOverrides` after file loading. CLI always wins over file.

### `--fuse` resolution and `TIMEBOMB_FUSE_DAYS`

All six call sites that construct `CliOverrides::new(fuse, ...)` go through `resolve_fuse_arg(cli_fuse)` first:

```rust
fn resolve_fuse_arg(cli_fuse: Option<String>) -> Option<String> {
    cli_fuse.or_else(|| {
        std::env::var("TIMEBOMB_FUSE_DAYS").ok().map(|v| {
            if v.ends_with('d') { v } else { format!("{}d", v) }
        })
    })
}
```

Priority: `--fuse` CLI flag > `TIMEBOMB_FUSE_DAYS` env var > config file > default (0).

### `--file` filter — three-step path matching

`manifest --file` accepts multiple values. Each is matched via `file_matches(fuse_file, filter)` in `main.rs`:

1. Strip a leading `./` or `.\` (shell tab-completion compatibility).
2. If the filter contains glob metacharacters (`*`, `?`, `[`, `{`), compile and match with `globset`.
3. Otherwise fall back to a component-aware suffix match (`Path::ends_with`).

This means `src/auth.rs`, `./src/auth.rs`, and `src/auth/**` all work transparently.

### Exit codes

| Code | Meaning |
|------|---------|
| 0 | No detonated fuses (or counts within bunker/ceilings) |
| 1 | Detonated fuses found, `--fail-on-ticking` triggered, or ratchet ceiling breached |
| 2 | Configuration or runtime error |

`manifest` and `defuse` **always** exit 0 — they are informational/interactive. Only `sweep` uses exit code 1.

### `defuse` command — two-pass interactive resolution

`defuse` collects all user decisions in a first pass (interactive prompts: extend / delete / skip), then applies them in a second pass bottom-up by descending line number per file. This avoids line-shift bugs. It reuses `snooze::snooze_line` for Extend and `remove::remove_line` for Delete.

### `--since` mode — diff-aware filtering

`sweep --since <ref>` runs the normal scan, then filters results to fuses whose file+line appear in the changed line ranges returned by `diff::git_changed_line_ranges`. The diff parser (`parse_unified_diff`) is a pure function that takes a `git diff --unified=0` string and returns `HashMap<PathBuf, Vec<RangeInclusive<usize>>>`. Both staged and unstaged diffs are merged.

### Bunker ratchet

`bunker save` writes `.timebomb-baseline.json` with the current detonated and ticking counts. `sweep` loads this file (if present) and calls `check_ratchet` — a pure function returning a `Vec<String>` of violation messages. Four independent checks: `max_detonated` ceiling, `max_ticking` ceiling, regression vs. baseline detonated, regression vs. baseline ticking. Any violation causes `sweep` to exit 1.

### `intel --by month`

`compute_stats` in `stats.rs` groups fuses by `fuse.date.format("%Y-%m")` into `MonthRow` entries stored in `by_month: Vec<MonthRow>` on `StatsResult`. Month rows are sorted chronologically (ascending YYYY-MM string sort). The `--by month` arm in main.rs dispatches to `print_stats_month(result, format)` instead of `print_stats`.

### Shell completions

`timebomb completions <shell>` uses `clap_complete::generate` with `Cli::command()` to print a completion script to stdout. `clap_complete::Shell` is re-exported from `cli.rs` as `pub use clap_complete::Shell` so the type is accessible to `main.rs` without an extra import.

---

## Fuse format

```
// TODO[2026-06-01]: message
# FIXME[2026-03-15][alice]: message with owner
```

The regex (built in `Config::fuse_regex_pattern()`):

```
(?i)(TODO|FIXME|HACK|TEMP|REMOVEME|DEBT|STOPSHIP|WORKAROUND|DEPRECATED|BUG)\[(\d{4}-\d{2}-\d{2})\](\[([^\]]+)\])?:\s*(.+)
```

Capture groups: `[1]` tag, `[2]` date, `[4]` optional owner, `[5]` message.

Plain `// TODO: fix this` comments (no bracket-date) are intentionally ignored.

---

## Test fixtures

Fixture files in `tests/fixtures/` use **hardcoded, date-independent values**:

- **Detonated**: dates in 2018–2021 (always in the past)
- **Ticking**: dates in mid-2025 (use a wide `--fuse 30d` window in tests)
- **Inert**: dates in 2088 or 2099 (always in the future)

Never use relative dates like "30 days from today" in fixture files. Tests must not depend on the wall clock. Each fixture file contributes 4 detonated, 1 ticking, and 2 inert fuses (except `sample.rs`, `sample.py`, `sample.sql` which have 6 detonated each).

When adding a new fixture file:
1. Add the extension to `default_extensions()` in `src/config.rs`.
2. Update the `swept_files`, `detonated_count`, and `ticking_count` assertions in `tests/scanner_tests.rs`.

---

## Output formats

Four formats are supported, selected via `--format` or auto-detected:

| Format | Trigger | Commands |
|--------|---------|---------|
| `terminal` | Default; respects `NO_COLOR` env var | all |
| `json` | `--format json` | all |
| `github` | `--format github` or `GITHUB_ACTIONS=true` env var | all |
| `csv` | `--format csv` | `manifest` only; falls back to terminal elsewhere |

- **Terminal**: `DETONATED` (red/bold) / `TICKING` (yellow) / `INERT` (dim). `manifest` adds a compact `age_col` column (`-Xd` overdue, `+Xd` future) between the date and owner fields. `sweep` uses the verbose `days_label` `"(X days overdue)"` form instead.
- **CSV**: `print_csv_list` in `output.rs` writes a header row then one row per fuse. Fields are quoted per RFC 4180 via `csv_field()` if they contain commas, quotes, or newlines.
- **GitHub Actions**: `::error` for detonated, `::warning` for ticking, inert silently skipped.

When `Csv` is passed to a dispatch function that doesn't support it (`print_scan_result`, `print_stats`, `print_trend`), it falls back to the terminal formatter.

---

## CI and releases

CI runs on `ubuntu-24.04`. Jobs: `fmt` → `clippy` → `unit-tests` + `integration-tests` → `smoke-tests` → `self-check` → `release`.

The `release` job uses `googleapis/release-please-action` and only runs on pushes to `main` after `smoke-tests` passes. It reads Conventional Commits to determine the version bump (`fix:` → patch, `feat:` → minor, `feat!:` / `BREAKING CHANGE:` → major), opens a release PR that bumps `Cargo.toml`, and creates the GitHub release on merge.

---

## Performance notes

Known remaining cost:
- **`is_binary` runs serially in Phase 1** — every candidate file is opened once for binary detection, then again in Phase 2 via `fs::read`. The double-open is a known tradeoff.

Already fixed:
- Line-level `[` pre-filter in `scan_content` before the regex is applied
- Allocations deferred until after date validation in `scan_content`
- `OnceLock` caches for regexes in `snooze.rs` and `diff.rs`
- Single-pass fold for detonated/ticking/inert counts in `output.rs`
- Single-buffer file reconstruction in `snooze.rs` and `fix.rs`
- `sort_unstable_by_key` for the final Phase 3 sort
- `HashSet<&str>` instead of `HashSet<String>` for membership lookups in `trend.rs`
- Iterator `.next()` instead of `collect::<Vec<_>>()` for two-field destructure in `blame.rs`
- Char-safe `char_indices().nth(N)` instead of byte-slice truncation in `stats.rs`

---

## Dependencies — rationale

| Crate | Why |
|-------|-----|
| `clap` (derive) | Argument parsing with minimal boilerplate |
| `clap_complete` | Shell completion script generation for bash/zsh/fish/elvish/powershell |
| `walkdir` | Reliable, cross-platform recursive directory walking |
| `regex` | Compiled, reusable fuse pattern matching |
| `chrono` | `NaiveDate` arithmetic for expiry calculation |
| `toml` + `serde` | `.timebomb.toml` deserialization |
| `serde_json` | JSON output format and bunker baseline file I/O |
| `globset` | Fast glob matching for exclude patterns and `--file` filters |
| `colored` | Terminal color; respects `NO_COLOR` automatically |
| `rayon` | Data-parallel file scanning in Phase 2 |
| `tempfile` (dev) | Isolated temp directories in integration tests |

Do not add new dependencies without a clear justification. Prefer extending existing ones.

---

## Things to avoid

- **Do not call `std::process::exit` outside `main.rs`**. Library code must return `Result`; exit codes are resolved only at the top level.
- **Do not fetch the current date inside the scanner**. Always use the injected `today: NaiveDate`.
- **Do not compile the regex inside `scan_file` or `scan_content`**. It must be compiled once in `scan()` and passed by reference.
- **Do not make `manifest` or `defuse` exit non-zero**. They are informational/interactive only.
- **Do not add language-specific parsers**. The scanner is intentionally language-agnostic.
- **Do not apply edits top-down in `defuse` or `delay`**. Always apply bottom-up (descending line number) to avoid line-shift bugs.
- **Do not use old names**. The rename from `Annotation`/`expired`/`check`/`list`/`fix` to `Fuse`/`detonated`/`sweep`/`manifest`/`defuse` is complete. Do not reintroduce the old terminology.
- **Do not bypass `resolve_fuse_arg`**. All `CliOverrides::new(fuse, ...)` calls must go through this helper so `TIMEBOMB_FUSE_DAYS` is respected consistently.
- **Do not use `OutputFormat::Csv` in dispatch functions that don't support it** without providing a terminal fallback. CSV is only meaningful for list output (`manifest`).
