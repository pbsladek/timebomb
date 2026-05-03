---
layout: default
title: Output Formats
---

# Output Formats

## Terminal

The default output is human-readable terminal text.

```text
DETONATED src/auth/login.rs:42  TODO[2025-01-15]  -433d  [alice]  remove legacy oauth flow
```

## JSON

```bash
timebomb sweep --format json
timebomb manifest --format json
```

`sweep` JSON groups fuses by status. `manifest` JSON emits a list of matching
fuses.

## CSV

```bash
timebomb manifest --format csv
```

CSV is available for `manifest` and is useful for spreadsheets and lightweight
automation.

## Table

```bash
timebomb manifest --format table
```

Table output is fixed-width and intended for quick terminal scanning.

## GitHub Actions

```bash
timebomb sweep --format github
```

This emits workflow commands:

```text
::error file=src/auth/login.rs,line=42::TODO detonated on 2025-01-15: remove legacy oauth flow
```

When `GITHUB_ACTIONS=true`, this format is selected automatically.

## Agent Summary

```bash
timebomb sweep --agent-summary
```

```text
timebomb: failed
swept_files: 142
total_fuses: 17
detonated: 2
ticking: 1
inert: 14
next_action:
- fix src/auth/login.rs:42 TODO[2025-01-15][alice]: remove legacy oauth flow
```

This format is stable and compact so agents can paste it into reports.

## Fix Plan JSON

```bash
timebomb sweep --fix-plan json
```

```json
{
  "status": "failed",
  "actions": [
    {
      "kind": "review_detonated",
      "file": "src/auth/login.rs",
      "line": 42,
      "target": "src/auth/login.rs:42",
      "tag": "TODO",
      "date": "2025-01-15",
      "owner": "alice",
      "status": "detonated",
      "message": "remove legacy oauth flow",
      "command": "timebomb delay src/auth/login.rs:42 --date YYYY-MM-DD --reason \"...\""
    }
  ]
}
```

The fix plan is non-mutating. It gives agents a task list without editing files.

