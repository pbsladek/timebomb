# timebomb — AGENTS.md

This file documents how AI agents should operate in this repository: what roles exist, how to divide work, what conventions to follow, and a record of completed agent reviews.

---

## Agent roles

### `reviewer` — read-only analysis

A reviewer agent reads source files and produces a written findings report. It makes **no code changes**. Its output is a structured review with per-finding severity ratings (Low / Medium / High), descriptions, and concrete recommendations.

Spawn a reviewer when:
- A subsystem has been rewritten and needs a second opinion before changes are committed.
- A performance bottleneck is suspected but not yet located.
- A correctness or safety audit is needed (e.g. error handling, exit code discipline).

### `implementer` — targeted code changes

An implementer agent applies a specific, scoped change to the codebase. It must:
- Read the relevant files before editing them.
- Run `cargo build` after changes to confirm the project compiles.
- Run `cargo test` to confirm no regressions.
- Not change files outside its assigned scope without flagging it first.

Spawn an implementer when a reviewer has identified a concrete, well-scoped fix and a human has approved it.

### `scaffolder` — creates new files or modules

A scaffolder agent creates new source files, test fixtures, or configuration. It must follow the layout conventions in `CLAUDE.md` and:
- Wire new modules into `src/lib.rs` with `pub mod <name>;`.
- Add the new subcommand variant to `src/cli.rs` (clap derive).
- Add the dispatch arm to `src/main.rs`.
- Create a corresponding integration test file in `tests/`.

---

## Parallelism guidelines

Agents can be run in parallel when their scopes do not overlap. The safe split boundaries in this project are:

| Agent A scope | Agent B scope | Safe to parallelise? |
|---------------|---------------|----------------------|
| `src/scanner.rs` | `src/output.rs` | Yes |
| `src/config.rs` | `src/annotation.rs` | Yes |
| `src/fix.rs` | `src/baseline.rs` | Yes |
| `src/diff.rs` | `src/blame.rs` | Yes |
| `src/snooze.rs` | `src/trend.rs` | Yes |
| `tests/scanner_tests.rs` | `tests/config_tests.rs` | Yes |
| `tests/fix_tests.rs` | `tests/baseline_tests.rs` | Yes |
| Any `src/` file | The same `src/` file | **No** |
| `src/cli.rs` | `src/main.rs` | **No** — both must agree on command variants |
| `Cargo.toml` | Any `src/` file | **No** — dependency changes affect compilation |

When in doubt, assign agents to disjoint directories or disjoint files. `src/cli.rs`, `src/main.rs`, and `src/lib.rs` are shared coordination files — only one agent should touch them at a time, or changes must be reconciled carefully afterwards.

---

## Constraints all agents must follow

1. **Read before editing.** Use `read_file` before any `edit_file` call. Never guess at contents.
2. **Preserve the three-phase scan structure.** `scan()` in `scanner.rs` is split into a serial walk phase, a parallel rayon phase, and a serial flatten+sort phase. Do not collapse these.
3. **Do not fetch `today` inside the scanner.** `NaiveDate` for "today" is always injected from `main.rs` and passed through. Tests depend on this.
4. **Do not compile the regex inside `scan_file` or `scan_content`.** It is compiled once in `scan()` and passed by reference. Other module-local regexes are cached via `OnceLock`.
5. **Do not call `std::process::exit` outside `main.rs`.** All library functions return `Result`.
6. **`list` and `fix` must always exit 0.** Only `check` uses exit codes 1 and 2.
7. **All tests must continue to pass.** Run `cargo test` and confirm before declaring a task done. Do not hardcode a specific test count here — it changes as features are added.
8. **No new dependencies without justification.** See the dependency rationale table in `CLAUDE.md`.
9. **Apply file edits bottom-up.** In `fix` and `snooze`, edits to a file must be applied in descending line-number order to avoid line-shift bugs when multiple annotations are modified in the same file.

---

## Completed agent reviews

---

### Review session — scanner performance (two agents, parallel)

**Date:** 2025
**Trigger:** Rayon parallelism was added to `scan()`; a full performance review was requested before further optimisation work.
**Agents:**
- Agent 1: reviewed `scan_content` and `build_regex` (the per-line hot path)
- Agent 2: reviewed `scan()` and `scan_file()` (I/O structure and rayon integration)

#### Agent 1 findings — `scan_content` hot path

| # | Finding | Severity | Status |
|---|---------|----------|--------|
| 1 | No line-level pre-filter before `captures_iter` — regex runs on every line even when no `[` is present | **High** | ✅ Fixed |
| 2 | `tag.to_uppercase()`, `owner.to_string()`, `message.to_string()` allocate before date is validated; discarded on invalid-date lines | **Medium** | ✅ Fixed |
| 3 | `rel_path.to_path_buf()` inside annotation push clones the same path string once per annotation | **Medium** | Open |
| 4 | `Vec::new()` with no capacity hint — minor reallocation churn for files with many annotations | **Low** | Open |
| 5 | `regex::escape()` not applied to user-supplied tag names — a tag containing regex metacharacters silently breaks the pattern | **Low** (correctness) | Open |
| 6 | `content.lines()` / CRLF handling — correct and efficient, no action needed | **Low** | N/A |

**Top recommendation from Agent 1:**
Add `if !line.contains('[') { continue; }` as the first statement inside the `for (line_idx, line)` loop in `scan_content`. This eliminates regex overhead on ~99.9% of lines in the common case where annotations are rare.

#### Agent 2 findings — `scan()` I/O and rayon structure

| # | Finding | Severity | Status |
|---|---------|----------|--------|
| 1 | `is_binary()` runs in the serial Phase 1, causing every candidate file to be opened twice (once for binary check, once in `scan_file`) | **High** | ⚠️ Partially mitigated — `fs::metadata` pre-check removed; double-open remains |
| 2 | `rel_path` `PathBuf` is allocated before the early-exit extension and glob filters run, wasting allocations for files that are immediately skipped | **Medium** | Open |
| 3 | `filter_map(|e| e.ok())` silently discards WalkDir permission errors | **Medium** (correctness) | Open |
| 4 | `read_to_string` vs `read` + null-byte check + `from_utf8` — combining into one read would eliminate the double-open | **Low** | Open |
| 5 | `sort_by_key` uses stable sort; `sort_unstable_by_key` is a free upgrade | **Low** | ✅ Fixed |
| 6 | `flatten().collect()` in Phase 3 allocates without a capacity hint | **Low** | Open |
| 7 | `BufReader` in `is_binary` wraps a single 8 KB read — adds a heap allocation with no benefit | **Low** | Open |

**Top recommendation from Agent 2:**
Move `is_binary` into Phase 2 by replacing `std::fs::read_to_string` in `scan_file` with `std::fs::read` (raw bytes), checking for null bytes on those bytes, then decoding with `String::from_utf8`. This eliminates the double file open and removes the serial I/O bottleneck.

#### Combined priority order

| Priority | Change | Severity | Status |
|----------|--------|----------|--------|
| 1 | Add `if !line.contains('[') { continue; }` pre-filter in `scan_content` | High | ✅ Fixed |
| 2 | Move binary detection into Phase 2; combine with full file read | High | ⚠️ Partial |
| 3 | Defer `tag`/`owner`/`message` allocations until after date parse succeeds | Medium | ✅ Fixed |
| 4 | Reorder Phase 1 checks — extension check before `rel_path` allocation | Medium | Open |
| 5 | Emit `eprintln!` warning on WalkDir errors instead of silently dropping them | Medium | Open |
| 6 | `sort_unstable_by_key` in Phase 3 | Low | ✅ Fixed |
| 7 | `Vec::with_capacity(4)` in `scan_content` | Low | Open |
| 8 | Remove `BufReader` from `is_binary` | Low | Open |
| 9 | `regex::escape()` on tag names in `annotation_regex_pattern` | Low (correctness) | Open |

---

### Implementation session — broad performance pass (three agents, parallel)

**Date:** 2026-03-22
**Trigger:** Performance review above was approved for implementation. Ten fixes were identified across eight source files and split across three parallel implementer agents.
**Agents:**
- Agent A: `src/snooze.rs`, `src/diff.rs`, `src/fix.rs` (Fixes 1, 2, 4, 5)
- Agent B: `src/output.rs`, `src/scanner.rs` (Fixes 6, 8)
- Agent C: `src/trend.rs`, `src/report.rs`, `src/blame.rs`, `src/stats.rs` (Fixes 3, 7, 9, 10)

All three agents ran concurrently in the same working directory (non-overlapping file scopes). Final `cargo test && cargo clippy -- -D warnings && cargo fmt --check` passed clean after a single `cargo fmt` fixup to fold closure style in `output.rs`.

#### Fixes applied

| Fix | File | Change |
|-----|------|--------|
| 1 | `snooze.rs` | `OnceLock<Regex>` cache for the date-bracket regex — compiled at most once per process |
| 2 | `diff.rs` | `OnceLock<Regex>` cache for the hunk-header regex |
| 3 | `trend.rs` | `HashSet<&str>` instead of `HashSet<String>` for `b_all_keys` — no key clones |
| 4 | `fix.rs` | Single pre-allocated `String` buffer for file reconstruction instead of per-line `.to_string()` + `Vec<String>` |
| 5 | `snooze.rs` | Same single-buffer approach in `run_snooze` |
| 6 | `output.rs` | Single-pass `fold` for expired/warning/ok counts — three `Vec` allocations → zero |
| 7 | `report.rs` | Single `for ann in &result.annotations` dispatch in `build_report` — three filter-collect Vecs eliminated |
| 8 | `scanner.rs` | Removed `fs::metadata` pre-check before `fs::read` in both `scan()` Phase 2 and `scan_file()` — 2 syscalls/file → 1 |
| 9 | `blame.rs` | Iterator `.next()` calls instead of `split_whitespace().collect::<Vec<_>>()` for two-field destructure |
| 10 | `stats.rs` | `char_indices().nth(18)` replaces `&name[..18]` byte-slice — correct for multi-byte UTF-8 |

---

### Implementation session — three new features (three agents, parallel, worktree-isolated)

**Date:** 2026-03-22
**Trigger:** Three product features were designed and approved for implementation simultaneously.
**Agents:**
- Agent (worktree `agent-a4e4d679`): `timebomb fix` — interactive expired annotation resolution (`src/fix.rs`, `tests/fix_tests.rs`)
- Agent (worktree `agent-aade10ac`): `timebomb check --changed` — diff-aware filtering (`src/diff.rs`, `tests/diff_tests.rs`)
- Agent (worktree `agent-aff3d16a`): `timebomb baseline` — ratchet enforcement (`src/baseline.rs`, `tests/baseline_tests.rs`)

Each agent ran in an isolated git worktree to prevent file-level conflicts. Shared coordination files (`src/cli.rs`, `src/main.rs`, `src/lib.rs`, `src/config.rs`) were modified by all three agents and merged without conflicts because the changes were additive and non-overlapping.

#### New modules added

| Module | Command | Summary |
|--------|---------|---------|
| `src/fix.rs` | `timebomb fix` | Two-pass interactive loop: collect decisions (extend / delete / skip), then apply bottom-up by descending line per file |
| `src/diff.rs` | `check --changed` | Pure `parse_unified_diff` function + `git_changed_line_ranges` which merges staged and unstaged diffs |
| `src/baseline.rs` | `timebomb baseline save/show` | `Baseline` struct (serde), `load_baseline`, `check_ratchet` (pure, four independent checks), `run_baseline_save`, `run_baseline_show` |

---

## How to record a new agent review

When a new agent review session is completed, append a section to the **Completed agent reviews** block above. Include:

- The date and trigger (why the review was requested).
- Which agents ran and what scope each covered.
- A findings table per agent (finding, severity, status).
- The top recommendation from each agent.
- A combined priority order if multiple agents reviewed related code.

Keep the record permanent — do not delete old reviews. They serve as an audit trail of what was examined and what was deliberately deferred. Update the **Status** column in existing tables as findings are addressed rather than duplicating rows.
