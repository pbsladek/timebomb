# timebomb 💣

A CLI tool that scans source code for **expiring TODO/FIXME annotations** and fails with a non-zero exit code in CI when any annotation's deadline has passed.

It enforces the social contract that temporary code actually gets removed.

---

## The Problem

Every codebase has comments like these:

```rust
// TODO: remove this feature flag once experiment ends
// FIXME: workaround for upstream bug, revert after upgrade
// HACK: temporary patch until we migrate to the new API
```

These comments are well-intentioned, but they never get cleaned up. The experiment ends, the upgrade ships, the migration completes — and the temporary code stays forever.

**timebomb** makes TODOs enforceable by requiring an expiry date. When the date passes, CI fails.

---

## Annotation Format

```
// TODO[2026-06-01]: remove this feature flag once experiment ends
# FIXME[2026-03-15]: workaround for upstream bug, revert after upgrade
-- TODO[2025-12-31]: drop this column after migration completes
```

**Pattern:** `<TAG>[<YYYY-MM-DD>]: <message>`

The tag must be **immediately followed** by `[date]` with no space. This distinguishes timebomb annotations from ordinary `// TODO: fix this` comments — undecorated TODOs are ignored entirely, so you can adopt timebomb gradually without touching your existing comments.

### Optional owner field

```
// TODO[2026-06-01][alice]: remove after migration — alice owns this
# FIXME[2026-03-15][team-backend]: revert after the platform upgrade
```

### Supported tags

By default: `TODO`, `FIXME`, `HACK`, `TEMP`, `REMOVEME`

Tags are matched **case-insensitively**, so `todo`, `Todo`, and `TODO` all work.

### Works in any language

The scanner matches the tag pattern **anywhere on a line**, so it works across all languages without language-specific parsers. The comment prefix (`//`, `#`, `--`, `*`, etc.) is irrelevant.

```rust
// TODO[2026-06-01]: Rust
```
```python
# TODO[2026-06-01]: Python
```
```sql
-- TODO[2026-06-01]: SQL
```
```go
// TODO[2026-06-01]: Go
```
```typescript
// TODO[2026-06-01]: TypeScript
```

---

## Installation

### From source

```bash
git clone https://github.com/yourname/timebomb
cd timebomb
cargo install --path .
```

### Via cargo

```bash
cargo install timebomb
```

### Download a pre-built binary

Check the [Releases](https://github.com/yourname/timebomb/releases) page for pre-built binaries for Linux, macOS, and Windows.

---

## Usage

### `check` — Scan and fail if expired

```bash
# Scan current directory, fail if any annotations have expired
timebomb check

# Scan a specific path
timebomb check ./src

# Also warn on items expiring within 30 days
timebomb check --warn-within 30d

# Fail (exit 1) if any items are in the warning window too
timebomb check --warn-within 30d --fail-on-warn

# Machine-readable JSON output
timebomb check --format json

# GitHub Actions annotation format
timebomb check --format github

# Use a specific config file
timebomb check --config /path/to/.timebomb.toml

# Enrich unowned annotations with the git blame author
timebomb check --blame
```

### `list` — List all annotations

```bash
# List all annotations sorted by expiry date (always exits 0)
timebomb list

# List only expired annotations
timebomb list --expired

# List annotations expiring within 14 days
timebomb list --expiring-soon 14d

# JSON output
timebomb list --format json

# Scan a specific directory
timebomb list ./src

# Enrich unowned annotations with the git blame author
timebomb list --blame
```

### `hook` — Manage the git pre-commit hook

```bash
# Install a timebomb pre-commit hook (prompts for confirmation)
timebomb hook install

# Install without prompting (useful in scripts)
timebomb hook install --yes

# Remove the timebomb block from the pre-commit hook
timebomb hook uninstall --yes

# Operate on a repository at a specific path
timebomb hook install /path/to/repo
```

The hook block appended to (or created in) `.git/hooks/pre-commit`:

```sh
# BEGIN timebomb
timebomb check --since HEAD .
# END timebomb
```

If a `pre-commit` hook already exists, the block is **appended** and the existing content is preserved. Uninstalling removes only the timebomb block; other content is left intact. The hook file is deleted if it becomes empty after removal.

### `trend` — Compare two report snapshots

```bash
# Compare an older report to a newer one
timebomb trend report-2025-01.json report-2025-02.json

# Output as JSON
timebomb trend --format json report-2025-01.json report-2025-02.json

# GitHub Actions format (shows ::error / ::notice annotations)
timebomb trend --format github report-2025-01.json report-2025-02.json
```

`trend` reads two report JSON files produced by `timebomb report` and shows how annotation debt has changed between them:

```
Trend: 2025-01-01T00:00:00Z → 2025-02-01T00:00:00Z

  Expired:       +2
  Expiring soon: -1
  Total:         +1

  Newly expired (2):
    src/auth/login.rs:42  TODO[2025-01-15]  remove legacy oauth flow
    src/db/schema.sql:88  FIXME[2025-01-20]  drop temp_users table

  Resolved (0):
    (none)

  Snoozed (0):
    (none)
```

**Snoozed** annotations were expired in the baseline but now appear in the expiring-soon bucket, meaning the deadline was bumped without resolving the underlying issue.

---

## Exit Codes

| Code | Meaning |
|------|---------|
| `0` | No expired annotations found |
| `1` | One or more expired annotations found (or expiring-soon if `--fail-on-warn`) |
| `2` | Configuration or runtime error |

> **Note:** `timebomb list` always exits `0` regardless of expiry status — it is purely informational. Only `timebomb check` can produce a non-zero exit code.

---

## Output Formats

### Terminal (default)

Color-coded output. Red for expired, yellow for expiring-soon, dimmed for future.

```
EXPIRED  src/auth/login.rs:42           TODO[2026-01-15]      remove legacy oauth flow
WARNING  src/db/schema.sql:108          FIXME[2026-04-01]     drop temp_users table
OK       src/api/handler.rs:77          HACK[2099-01-01]      revisit when platform ships new API

Scanned 142 file(s) · 17 annotation(s) total · 1 expired · 1 expiring soon · 15 ok
```

When `--blame` is passed, the responsible developer is shown after the tag:

```
EXPIRED  src/auth/login.rs:42           TODO[2026-01-15] [~alice]   remove legacy oauth flow
EXPIRED  src/db/schema.sql:108          FIXME[2026-04-01] [bob]     drop temp_users table
```

`[alice]` = explicit owner from the annotation bracket; `[~alice]` = inferred from git blame.

Set the `NO_COLOR` environment variable to disable color output:

```bash
NO_COLOR=1 timebomb check
```

### JSON (`--format json`)

```json
{
  "scanned_files": 142,
  "total_annotations": 17,
  "expired": [
    {
      "file": "src/auth/login.rs",
      "line": 42,
      "tag": "TODO",
      "date": "2026-01-15",
      "owner": null,
      "message": "remove legacy oauth flow",
      "status": "expired"
    }
  ],
  "expiring_soon": [
    {
      "file": "src/db/schema.sql",
      "line": 108,
      "tag": "FIXME",
      "date": "2026-04-01",
      "owner": null,
      "message": "drop temp_users table",
      "status": "expiring_soon"
    }
  ],
  "ok": [...]
}
```

### GitHub Actions (`--format github`)

Produces [workflow commands](https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/workflow-commands-for-github-actions) that appear as inline annotations in the GitHub PR interface:

```
::error file=src/auth/login.rs,line=42::TODO expired on 2026-01-15: remove legacy oauth flow
::warning file=src/db/schema.sql,line=108::FIXME expires on 2026-04-01 (within 30d warning window):  drop temp_users table
```

**Auto-detection:** If the `GITHUB_ACTIONS=true` environment variable is set, timebomb automatically defaults to `--format github` without needing the flag explicitly.

---

## Configuration

Place a `.timebomb.toml` file in your project root:

```toml
# Tags to scan for (default shown)
tags = ["TODO", "FIXME", "HACK", "TEMP", "REMOVEME"]

# Warn (and optionally fail with --fail-on-warn) if an annotation expires
# within this many days. Set to 0 to disable warnings.
warn_within_days = 14

# Paths/globs to exclude from scanning
exclude = [
  "vendor/**",
  "node_modules/**",
  "*.min.js",
  ".git/**",
  "target/**",
  "dist/**",
]

# File extensions to scan (if empty, scans all text files)
extensions = ["rs", "go", "ts", "js", "py", "rb", "java", "sql", "tf", "yaml", "yml"]
```

**Merge order:** Config file values are loaded first, then CLI flags override them.

If no config file is found, built-in defaults are used silently.

### Configuration reference

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `tags` | `[string]` | `["TODO","FIXME","HACK","TEMP","REMOVEME"]` | Tags to scan for |
| `warn_within_days` | integer | `0` | Days before expiry to start warning |
| `exclude` | `[string]` | `["vendor/**","node_modules/**","*.min.js",".git/**"]` | Glob patterns to exclude |
| `extensions` | `[string]` | `["rs","go","ts","js","py","rb","java","sql","tf","yaml","yml"]` | File extensions to scan |

---

## CI Integration

### GitHub Actions

```yaml
- name: Check for expired timebombs
  run: timebomb check --warn-within 14d --format github
```

Since `GITHUB_ACTIONS=true` is set automatically in GitHub Actions, you can also omit `--format github` and it will be detected automatically:

```yaml
- name: Check for expired timebombs
  run: timebomb check --warn-within 14d
```

A full workflow example:

```yaml
name: timebomb

on:
  push:
  pull_request:
  schedule:
    # Run daily at 09:00 UTC so you get alerted even without a push
    - cron: '0 9 * * *'

jobs:
  timebomb:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install timebomb
        run: cargo install timebomb

      - name: Check for expired annotations
        run: timebomb check --warn-within 14d --fail-on-warn
```

### Pre-commit hook

The easiest way to add a pre-commit hook is `timebomb hook install`:

```bash
timebomb hook install --yes
```

This creates (or appends to) `.git/hooks/pre-commit` with a shebang, `set -e`, and a `timebomb check` invocation. To remove it later:

```bash
timebomb hook uninstall --yes
```

Or write the hook manually:

```bash
#!/bin/sh
set -e
timebomb check --warn-within 7d
chmod +x .git/hooks/pre-commit
```

### GitLab CI

```yaml
timebomb:
  stage: lint
  script:
    - timebomb check --warn-within 14d
  allow_failure: false
```

---

## Scanner Behavior

- **Recursive walk:** Walks the directory tree recursively using `walkdir`.
- **Exclude globs:** Paths matching any pattern in `exclude` are skipped entirely.
- **Extension filtering:** Only files with extensions in the `extensions` list are scanned. If the list is empty, all non-binary files are scanned.
- **Binary file detection:** Files are checked for null bytes (`\x00`) in the first 8 KB. If found, the file is skipped silently.
- **Line-by-line scanning:** Each file is scanned line by line using a single compiled regex.
- **Regex pattern:** `(?i)(TODO|FIXME|HACK|TEMP|REMOVEME)\[(\d{4}-\d{2}-\d{2})\](\[([^\]]+)\])?:\s*(.+)`
- **Invalid dates:** An annotation like `TODO[2026-13-45]` (invalid month) emits a warning to stderr but does not crash.
- **Consistent "today":** The current date is derived once at program start and passed through the entire scan, so long runs across midnight are consistent.
- **Sorted output:** Results are sorted by date ascending, so the most urgent items appear first.

---

## Development

### Prerequisites

- Rust 1.70 or later (`rustup install stable`)

### Build

```bash
cargo build
cargo build --release
```

### Test

```bash
cargo test
```

### Lint

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

### Format

```bash
cargo fmt
```

### Run locally

```bash
# Scan src/ with a 14-day warning window
cargo run -- check ./src --warn-within 14d

# List all annotations as JSON
cargo run -- list --format json

# Check and fail on warns too
cargo run -- check --warn-within 30d --fail-on-warn
```

---

## Project Structure

```
timebomb/
├── Cargo.toml
├── .timebomb.toml          # Example configuration file
├── src/
│   ├── lib.rs              # Library root — exposes all modules
│   ├── main.rs             # CLI entrypoint, clap setup, subcommand dispatch
│   ├── cli.rs              # Subcommand definitions (clap derive)
│   ├── config.rs           # .timebomb.toml loading + merging with CLI args
│   ├── scanner.rs          # File walking + line scanning → Vec<Annotation>
│   ├── annotation.rs       # Annotation struct, Status enum (Expired/ExpiringSoon/Ok)
│   ├── output.rs           # Terminal, JSON, GitHub Actions formatters
│   ├── error.rs            # Error types and duration parsing
│   ├── blame.rs            # git blame integration for --blame enrichment
│   ├── hook.rs             # Pre-commit hook install / uninstall
│   ├── trend.rs            # Report snapshot comparison (trend command)
│   ├── report.rs           # Report JSON generation and writing
│   ├── stats.rs            # Aggregate stats by owner / tag
│   ├── init.rs             # timebomb init command
│   ├── add.rs              # timebomb add command (insert annotations)
│   ├── snooze.rs           # timebomb snooze command (bump deadlines)
│   └── git.rs              # Git helpers (changed files, repo detection)
├── tests/
│   ├── scanner_tests.rs    # Integration tests for the scanner
│   ├── config_tests.rs     # Integration tests for config loading
│   └── fixtures/           # Sample files with known annotations
│       ├── sample.rs       # Rust fixture
│       ├── sample.py       # Python fixture
│       └── sample.sql      # SQL fixture
└── README.md
```

---

## FAQ

**Q: Will timebomb break my existing `// TODO: ...` comments?**

No. Plain TODOs without a date bracket are intentionally ignored. Only annotations in the form `TODO[YYYY-MM-DD]:` are matched.

**Q: What if I need more time to fix something?**

Update the date in the annotation. This is intentional — it forces a conscious decision to extend the deadline rather than silently ignoring it.

**Q: Can I use timebomb with pre-existing technical debt?**

Yes. Start by running `timebomb list` to see everything, then use `timebomb check` in CI with a date in the near future. Gradually add timebomb annotations to new temporary code; you don't have to annotate everything at once.

**Q: What if a date is genuinely far in the future (e.g., after a contract period)?**

That's fine. Pick the real date. If it's 3 years away, `timebomb check` will be green until then.

**Q: Can I add custom tags?**

Yes. Set `tags = ["TODO", "FIXME", "MYTEAMTAG"]` in `.timebomb.toml`.

**Q: Does timebomb scan binary files?**

No. It detects binary files by looking for null bytes in the first 8 KB and skips them silently.

**Q: What happens if the date is malformed (e.g. `TODO[2026-13-45]`)?**

timebomb prints a warning to stderr and skips that annotation. It does not crash.

**Q: What does `--blame` do exactly?**

When `--blame` is passed, timebomb runs `git blame --porcelain` on each file that has annotations without an explicit `[owner]` bracket. The commit author for that line is recorded as the inferred owner and shown as `[~author]` in terminal output and as `blamed_owner` in JSON. Explicit `[owner]` values are never overwritten. `--blame` is a no-op on files not tracked by git.

**Q: Is `timebomb hook install` safe if I already have a pre-commit hook?**

Yes. It appends the timebomb block to the existing file without touching any other content. The markers `# BEGIN timebomb` / `# END timebomb` make the block idempotent (installing twice only writes one block) and cleanly removable with `timebomb hook uninstall`.

**Q: How do I track annotation debt over time with `trend`?**

Run `timebomb report --output report-$(date +%Y-%m).json` on a schedule (e.g., weekly CI job) to produce snapshots. Then compare any two snapshots:

```bash
timebomb trend report-2025-01.json report-2025-02.json
```

---

## License

MIT — see [LICENSE](LICENSE) for details.