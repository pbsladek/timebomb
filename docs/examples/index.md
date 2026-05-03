---
layout: default
title: Examples
---

# timebomb Examples

These examples show practical ways to use `timebomb` in real repositories.

## 1. Fail CI on Expired Cleanup Work

```bash
timebomb sweep .
```

Use this as the default gate. If any fuse has a date before today, the command
exits `1`.

## 2. Warn Before a Deadline

```bash
timebomb sweep --fuse 14d
```

This still fails only on detonated fuses, but it prints fuses that will expire in
the next 14 days.

## 3. Fail Before a Deadline in Release Branches

```bash
timebomb sweep --fuse 30d --fail-on-ticking
```

Use this on release branches where upcoming deadlines should block the build.

## 4. Check Only Changed Lines in Pull Requests

```bash
timebomb sweep --changed --base main --format github
```

This is useful when a project has existing fuse debt but wants to block new or
touched expired fuses.

## 5. Get an Agent-Friendly Failure Summary

```bash
timebomb sweep --agent-summary
```

Agents can paste this output directly into a task report because it is compact
and deterministic.

## 6. Give an Agent a JSON Remediation Plan

```bash
timebomb sweep --fix-plan json
```

The JSON output lists active fuses and suggested commands without editing files.

## 7. Focus on One Failing Annotation

```bash
timebomb explain src/auth/login.rs:42 --blame
```

Use this when CI reports one failing line and an agent needs context plus safe
next actions.

## 8. Create a Debt Ratchet

```bash
timebomb bunker save
git add .timebomb-baseline.json
git commit -m "chore: save timebomb baseline"
```

Future `timebomb sweep` runs fail when detonated or ticking counts grow beyond
the saved baseline.

## 9. Export a Report for Trend Tracking

```bash
timebomb sweep --format json --output reports/timebomb-current.json || true
timebomb fallout reports/timebomb-last.json reports/timebomb-current.json
```

Use this to track newly detonated, resolved, and delayed fuses across snapshots.

## 10. Add a Pre-commit Hook

```bash
timebomb tripwire set --yes
```

The hook runs `timebomb sweep --since HEAD .` before each commit.

## Example Fuse Annotations

```rust
// TODO[2026-06-01][auth]: remove legacy oauth fallback
```

```python
# FIXME[2026-03-15][ml]: delete temporary prompt template after evals
```

```sql
-- HACK[2026-01-31][data]: keep old column until dashboard migration finishes
```

