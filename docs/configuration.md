---
layout: default
title: Configuration
---

# Configuration

`timebomb` reads `.timebomb.toml` from the scan root when present. If none is
found there, it falls back to `.timebomb.toml` in the current working directory.

```toml
triggers = ["TODO", "FIXME", "HACK", "TEMP", "REMOVEME", "DEBT", "STOPSHIP", "WORKAROUND", "DEPRECATED", "BUG"]
fuse_days = 14
exclude = [
  "vendor/**",
  "node_modules/**",
  "*.min.js",
  ".git/**",
]
extensions = ["rs", "go", "ts", "js", "py", "rb", "java", "sql", "tf", "yaml", "yml"]
max_detonated = 0
max_ticking = 5
```

## Fields

| Key | Type | Description |
| --- | --- | --- |
| `triggers` | array of strings | Tags that should be scanned. |
| `fuse_days` | integer | Days before expiry to classify a fuse as ticking. |
| `exclude` | array of globs | Paths excluded before files are opened. |
| `extensions` | array of strings | Extensions to scan. An empty list scans all non-binary files. |
| `max_detonated` | integer | Hard ceiling for detonated fuses. |
| `max_ticking` | integer | Hard ceiling for ticking fuses. |

CLI flags override config values when both are present.

## Environment Variables

| Variable | Description |
| --- | --- |
| `TIMEBOMB_FUSE_DAYS` | Default warning window, such as `14` or `14d`. |
| `NO_COLOR` | Disable terminal color output. |
| `GITHUB_ACTIONS` | When `true`, selects GitHub Actions output by default. |

## Baseline Ratchet

`timebomb bunker save` writes `.timebomb-baseline.json` with current detonated
and ticking counts. Later `sweep` runs compare live counts against the baseline
and fail if the counts increase.

```bash
timebomb bunker save
timebomb sweep
```

Use this when a codebase has existing debt and you want to prevent growth before
driving the count to zero.

