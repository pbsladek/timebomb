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

A scaffolder agent creates new source files, test fixtures, or configuration. It must follow the layout conventions in `CLAUDE.md` and wire new modules into `src/lib.rs` if they need to be accessible from integration tests.

---

## Parallelism guidelines

Agents can be run in parallel when their scopes do not overlap. The safe split boundaries in this project are:

| Agent A scope | Agent B scope | Safe to parallelise? |
|---------------|---------------|----------------------|
| `src/scanner.rs` | `src/output.rs` | Yes |
| `src/config.rs` | `src/annotation.rs` | Yes |
| `tests/scanner_tests.rs` | `tests/config_tests.rs` | Yes |
| Any `src/` file | The same `src/` file | **No** |
| `Cargo.toml` | Any `src/` file | **No** — dependency changes affect compilation |

When in doubt, assign agents to disjoint directories or disjoint files.

---

## Constraints all agents must follow

1. **Read before editing.** Use `read_file` before any `edit_file` call. Never guess at contents.
2. **Preserve the three-phase scan structure.** `scan()` in `scanner.rs` is split into a serial walk phase, a parallel rayon phase, and a serial flatten+sort phase. Do not collapse these.
3. **Do not fetch `today` inside the scanner.** `NaiveDate` for "today" is always injected from `main.rs` and passed through. Tests depend on this.
4. **Do not compile the regex inside `scan_file` or `scan_content`.** It is compiled once in `scan()` and passed by reference.
5. **Do not call `std::process::exit` outside `main.rs`.** All library functions return `Result`.
6. **`list` must always exit 0.** Only `check` uses exit codes 1 and 2.
7. **All 218 tests must continue to pass.** Run `cargo test` and confirm before declaring a task done.
8. **No new dependencies without justification.** See the dependency rationale table in `CLAUDE.md`.

---

## Completed agent reviews

### Review session — scanner performance (two agents, parallel)

**Date:** 2025  
**Trigger:** Rayon parallelism was added to `scan()`; a full performance review was requested before further optimisation work.  
**Agents:**
- Agent 1: reviewed `scan_content` and `build_regex` (the per-line hot path)
- Agent 2: reviewed `scan()` and `scan_file()` (I/O structure and rayon integration)

---

#### Agent 1 findings — `scan_content` hot path

| # | Finding | Severity |
|---|---------|----------|
| 1 | No line-level pre-filter before `captures_iter` — regex runs on every line even when no `[` is present | **High** |
| 2 | `tag.to_uppercase()`, `owner.to_string()`, `message.to_string()` allocate before date is validated; discarded on invalid-date lines | **Medium** |
| 3 | `rel_path.to_path_buf()` inside annotation push clones the same path string once per annotation | **Medium** |
| 4 | `Vec::new()` with no capacity hint — minor reallocation churn for files with many annotations | **Low** |
| 5 | `regex::escape()` not applied to user-supplied tag names — a tag containing regex metacharacters silently breaks the pattern | **Low** (correctness) |
| 6 | `content.lines()` / CRLF handling — correct and efficient, no action needed | **Low** |

**Top recommendation from Agent 1:**  
Add `if !line.contains('[') { continue; }` as the first statement inside the `for (line_idx, line)` loop in `scan_content`. This eliminates regex overhead on ~99.9% of lines in the common case where annotations are rare. A `memchr` byte scan is a further upgrade.

---

#### Agent 2 findings — `scan()` I/O and rayon structure

| # | Finding | Severity |
|---|---------|----------|
| 1 | `is_binary()` runs in the serial Phase 1, causing every candidate file to be opened twice (once for binary check, once in `scan_file`); binary detection should move to Phase 2 | **High** |
| 2 | `rel_path` `PathBuf` is allocated before the early-exit extension and glob filters run, wasting allocations for files that are immediately skipped | **Medium** |
| 3 | `filter_map(|e| e.ok())` silently discards WalkDir permission errors; `skipped_files` count is inaccurate and the user gets no warning | **Medium** (correctness) |
| 4 | `read_to_string` vs `read` + null-byte check + `from_utf8` — combining into one read eliminates the double-open described in Finding 1 | **Low** |
| 5 | `sort_by_key` uses stable sort; `sort_unstable_by_key` is a free upgrade since there is no meaningful tiebreaker for equal dates | **Low** |
| 6 | `flatten().collect()` in Phase 3 allocates without a capacity hint; pre-sizing with `.map(|v| v.len()).sum()` avoids reallocation | **Low** |
| 7 | `BufReader` in `is_binary` wraps a single 8 KB read — adds a heap allocation with no benefit; a plain stack-buffer `File::read` is sufficient | **Low** |

**Top recommendation from Agent 2:**  
Move `is_binary` into Phase 2 by replacing `std::fs::read_to_string` in `scan_file` with `std::fs::read` (raw bytes), checking for null bytes on those bytes, then decoding with `String::from_utf8`. This eliminates the double file open, removes the serial I/O bottleneck, and makes `is_binary` unnecessary as a standalone pre-filter.

---

#### Combined priority order for implementation

| Priority | Change | Severity | Expected gain |
|----------|--------|----------|---------------|
| 1 | Add `if !line.contains('[') { continue; }` pre-filter in `scan_content` | High | Eliminates regex on ~99.9% of lines |
| 2 | Move binary detection into Phase 2; combine with full file read | High | Removes serial I/O bottleneck + double open |
| 3 | Defer `tag`/`owner`/`message` allocations until after date parse succeeds | Medium | Alloc-free invalid-date path |
| 4 | Reorder Phase 1 checks — extension check before `rel_path` allocation | Medium | Fewer allocs for skipped entries |
| 5 | Emit `eprintln!` warning on WalkDir errors instead of silently dropping them | Medium | Correctness / observability |
| 6 | `sort_unstable_by_key` in Phase 3 | Low | Free speed improvement |
| 7 | `Vec::with_capacity(4)` in `scan_content` | Low | Minor — avoids early reallocs |
| 8 | Remove `BufReader` from `is_binary` (or delete function if Finding 2 is done) | Low | One fewer heap alloc |
| 9 | `regex::escape()` on tag names in `annotation_regex_pattern` | Low (correctness) | Safety against unusual tag names |

None of these changes alter the public API or test interfaces. All can be implemented incrementally. Items 1 and 2 are the only ones likely to produce a measurable speedup on a real repository.

---

## How to record a new agent review

When a new agent review session is completed, append a section to the **Completed agent reviews** block above. Include:

- The date and trigger (why the review was requested).
- Which agents ran and what scope each covered.
- A findings table per agent (finding, severity).
- The top recommendation from each agent.
- A combined priority order if multiple agents reviewed related code.

Keep the record permanent — do not delete old reviews. They serve as an audit trail of what was examined and what was deliberately deferred.