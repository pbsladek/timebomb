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
```

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

```bash
#!/bin/sh
# .git/hooks/pre-commit
timebomb check --warn-within 7d
```

Make it executable:

```bash
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
│   └── error.rs            # Error types and duration parsing
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

---

## License

MIT — see [LICENSE](LICENSE) for details.