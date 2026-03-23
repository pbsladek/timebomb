# timebomb

[![CI](https://github.com/pbsladek/timebomb/actions/workflows/ci.yml/badge.svg)](https://github.com/pbsladek/timebomb/actions/workflows/ci.yml)

`timebomb` scans source code for structured expiry annotations and exits non-zero in CI when any deadline has passed.

The problem it solves: `// TODO: remove this after the migration` gets written with good intentions and stays forever. `timebomb` makes the deadline explicit and machine-enforceable. When the date passes, the build fails — forcing a fix or a conscious decision to extend the deadline.

---

## Fuse format

```
// TODO[2026-06-01]: remove this feature flag once the experiment ends
# FIXME[2026-03-15][alice]: workaround for upstream bug, revert after upgrade
-- HACK[2025-12-31]: temporary shim, drop this column after migration
```

**Syntax:** `TAG[YYYY-MM-DD]: message` or `TAG[YYYY-MM-DD][owner]: message`

The tag must be immediately followed by `[date]` with no space. Plain `// TODO: fix this` comments (no bracket-date) are ignored entirely, so you can adopt timebomb incrementally without touching existing annotations.

The scanner is language-agnostic — it matches the pattern anywhere on a line regardless of comment syntax (`//`, `#`, `--`, `;;`, `%`, `*`, anything). No language-specific parsers.

### Default triggers

`TODO`, `FIXME`, `HACK`, `TEMP`, `REMOVEME`, `DEBT`, `STOPSHIP`, `WORKAROUND`, `DEPRECATED`, `BUG`

Tags are matched case-insensitively. The full set is configurable via `.timebomb.toml`.

### Fuse status

Each fuse is classified relative to the current date, which is derived once at startup and threaded through the entire scan (so long runs across midnight are consistent):

| Status | Condition |
|--------|-----------|
| `detonated` | Date is in the past |
| `ticking` | Date is within the `fuse_days` warning window |
| `inert` | Date is beyond the warning window |

---

## Installation

```bash
cargo install timebomb
```

Or from source:

```bash
git clone https://github.com/pbsladek/timebomb
cd timebomb
cargo install --path .
```

---

## Commands

### `sweep` — scan and detonate in CI

```bash
timebomb sweep                          # scan current directory
timebomb sweep ./src                    # scan a specific path
timebomb sweep --fuse 30d               # also flag fuses ticking within 30 days
timebomb sweep --fuse 30d --fail-on-ticking  # exit 1 on ticking fuses too
timebomb sweep --since HEAD             # only check fuses on lines changed since HEAD
timebomb sweep --blame                  # enrich unowned fuses via git blame
timebomb sweep --format json            # machine-readable output
timebomb sweep --format github          # GitHub Actions workflow commands
```

`sweep` is the only command that exits non-zero. All other commands are informational and always exit 0.

### `manifest` — list all fuses

```bash
timebomb manifest                       # all fuses, sorted by date ascending
timebomb manifest --detonated           # only detonated
timebomb manifest --ticking 14d         # only ticking within 14 days
timebomb manifest --format json
timebomb manifest --blame
```

### `defuse` — interactively resolve detonated fuses

```bash
timebomb defuse                         # walk through each detonated fuse
timebomb defuse ./src
```

For each detonated fuse, `defuse` prompts:

```
DETONATED src/auth/login.rs:42  TODO[2025-01-15]: remove legacy oauth flow

  [e] Extend to new date
  [d] Delete line
  [s] Skip

Choice:
```

**Extend** prompts for a new date and rewrites the annotation in-place. **Delete** removes the line. Files are updated in a single bottom-up pass per file to avoid line-shift bugs.

### `plant` — insert a new fuse

```bash
timebomb plant src/auth/login.rs:42 "remove after migration" --date 2026-06-01
timebomb plant src/auth/login.rs:42 "remove after migration" --in-days 90
timebomb plant src/auth.rs "remove oauth" --search legacy_auth --tag FIXME --owner alice --yes
```

### `delay` — bump a deadline

```bash
timebomb delay src/auth/login.rs:42 --date 2026-09-01
timebomb delay src/auth/login.rs:42 --in-days 30 --reason "blocked on upstream fix"
```

### `disarm` — remove a fuse

```bash
timebomb disarm src/auth/login.rs:42
timebomb disarm --all-detonated         # remove every detonated fuse in the scan path
timebomb disarm --all-detonated --yes   # skip confirmation
```

### `intel` — breakdown by owner or tag

```bash
timebomb intel                          # count fuses grouped by owner and tag
timebomb intel --by owner
timebomb intel --by tag --format json
```

### `tripwire` — manage the git pre-commit hook

```bash
timebomb tripwire set --yes             # append timebomb block to .git/hooks/pre-commit
timebomb tripwire cut --yes             # remove only the timebomb block; leave other content intact
```

The hook block written by `tripwire set`:

```sh
# BEGIN timebomb
timebomb sweep --since HEAD .
# END timebomb
```

Installing twice is idempotent. Cutting removes only the marked block; if the file becomes empty it is deleted.

### `fallout` — compare two report snapshots

```bash
timebomb fallout report-jan.json report-feb.json
timebomb fallout --format json report-jan.json report-feb.json
```

Reads two JSON reports produced by `timebomb sweep --format json` and shows how fuse debt changed between them — newly detonated, resolved, and delayed (deadline bumped without fixing).

### `bunker` — ratchet enforcement

```bash
timebomb bunker save                    # snapshot current detonated/ticking counts
timebomb bunker show                    # compare live counts to the saved baseline
```

`bunker save` writes `.timebomb-baseline.json`:

```json
{
  "generated_at": "2026-03-22T10:00:00Z",
  "detonated": 3,
  "ticking": 5
}
```

When this file exists, `timebomb sweep` automatically loads it and exits 1 if the current detonated or ticking count exceeds the baseline — preventing debt from growing while not requiring everything to be fixed at once.

Hard ceilings can also be set in `.timebomb.toml` independently of the baseline file:

```toml
max_detonated = 0
max_ticking = 5
```

---

## Output formats

### Terminal (default)

```
DETONATED  src/auth/login.rs:42       TODO[2026-01-15]    remove legacy oauth flow
TICKING    src/db/schema.sql:108      FIXME[2026-04-01]   drop temp_users table
INERT      src/api/handler.rs:77      HACK[2099-01-01]    revisit when platform ships

Swept 142 file(s) · 17 fuse(s) · 1 detonated · 1 ticking · 15 inert
```

With `--blame`, unowned fuses show the git blame author as `[~name]`. Explicit `[owner]` brackets are shown as-is and are never overwritten.

Respects `NO_COLOR`.

### JSON (`--format json`)

```json
{
  "swept_files": 142,
  "total_fuses": 17,
  "detonated": [
    {
      "file": "src/auth/login.rs",
      "line": 42,
      "tag": "TODO",
      "date": "2026-01-15",
      "owner": null,
      "message": "remove legacy oauth flow",
      "status": "detonated"
    }
  ],
  "ticking": [...],
  "inert": [...]
}
```

### GitHub Actions (`--format github`)

Emits [workflow commands](https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/workflow-commands-for-github-actions) that appear as inline PR annotations:

```
::error file=src/auth/login.rs,line=42::TODO detonated on 2026-01-15: remove legacy oauth flow
::warning file=src/db/schema.sql,line=108::FIXME ticking until 2026-04-01: drop temp_users table
```

Auto-detected when `GITHUB_ACTIONS=true` is set.

---

## Configuration

`.timebomb.toml` in the project root:

```toml
# Tags to scan for
triggers = ["TODO", "FIXME", "HACK", "TEMP", "REMOVEME", "DEBT", "STOPSHIP", "WORKAROUND", "DEPRECATED", "BUG"]

# Flag fuses expiring within this many days as ticking (0 = disabled)
fuse_days = 14

# Glob patterns to exclude from scanning
exclude = [
  "vendor/**",
  "node_modules/**",
  "*.min.js",
  ".git/**",
]

# File extensions to scan. If empty, all non-binary files are scanned.
extensions = ["rs", "go", "ts", "js", "py", "rb", "java", "sql", "tf", "yaml", "yml"]

# Ratchet ceilings: sweep fails if live count exceeds these values.
max_detonated = 0
max_ticking = 5
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `triggers` | `[string]` | see above | Tags to match (case-insensitive) |
| `fuse_days` | integer | `0` | Days before expiry to enter `ticking` status |
| `exclude` | `[string]` | `["vendor/**","node_modules/**","*.min.js",".git/**"]` | Glob exclusions |
| `extensions` | `[string]` | see defaults | Extensions to scan; empty means all non-binary |
| `max_detonated` | integer | — | Hard ceiling; `sweep` exits 1 if exceeded |
| `max_ticking` | integer | — | Hard ceiling; `sweep` exits 1 if exceeded |

CLI flags override config file values. If no config file is found, built-in defaults apply silently.

---

## CI integration

### GitHub Actions

```yaml
name: timebomb
on:
  push:
  pull_request:
  schedule:
    - cron: '0 9 * * *'   # daily sweep even without a push

jobs:
  timebomb:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v4
      - run: cargo install timebomb
      - run: timebomb sweep --fuse 14d --fail-on-ticking
```

`--format github` is inferred automatically from `GITHUB_ACTIONS=true`, so workflow command annotations appear in the PR diff without any extra flags.

### GitLab CI

```yaml
timebomb:
  stage: lint
  script:
    - timebomb sweep --fuse 14d
```

### Pre-commit hook

```bash
timebomb tripwire set --yes
```

Or manually in `.git/hooks/pre-commit`:

```sh
#!/bin/sh
set -e
timebomb sweep --since HEAD .
```

---

## Releases

Releases are automated via [release-please](https://github.com/googleapis/release-please). Every merge to `main` is inspected for [Conventional Commits](https://www.conventionalcommits.org/):

| Commit type | Version bump |
|-------------|-------------|
| `fix:` | patch |
| `feat:` | minor |
| `feat!:` or `BREAKING CHANGE:` footer | major |

release-please opens a release PR that bumps `Cargo.toml` and drafts the changelog. Merging that PR creates the git tag and GitHub release automatically.

---

## Scanner behavior

- **Walk:** Recursive directory walk via `walkdir`. Symlinks are not followed.
- **Exclusions:** Paths matching any `exclude` glob are skipped before opening files.
- **Extension filter:** Only files whose extension matches the `extensions` list are scanned. An empty list disables the filter.
- **Binary detection:** The first 8 KB of each candidate file is checked for null bytes (`\x00`). Files containing any are skipped silently.
- **Parallel scan:** After the serial walk phase collects candidates, files are scanned in parallel via `rayon`. The compiled regex is shared across all worker threads.
- **Invalid dates:** A fuse with an unparseable date (e.g. `TODO[2026-13-45]`) emits a warning to stderr and is skipped; the scan continues.
- **Sort:** Results are sorted by date ascending so the most urgent fuses appear first.

---

## Exit codes

| Code | Meaning |
|------|---------|
| `0` | Clean — no detonated fuses (or counts within baseline/ceilings) |
| `1` | Detonated fuses found, ticking threshold exceeded with `--fail-on-ticking`, or ratchet ceiling breached |
| `2` | Configuration or runtime error |

---

## Development

Requires Rust 1.80+.

```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

---

## License

MIT — see [LICENSE](LICENSE) for details.
