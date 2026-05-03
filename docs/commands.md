---
layout: default
title: Commands
---

# Commands

## `sweep`

Scan a path and fail when detonated fuses are found.

```bash
timebomb sweep
timebomb sweep ./src
timebomb sweep --fuse 30d
timebomb sweep --fuse 30d --fail-on-ticking
timebomb sweep --format json
timebomb sweep --format github
timebomb sweep --since HEAD
timebomb sweep --changed --base main
timebomb sweep --owner alice
timebomb sweep --tag FIXME
timebomb sweep --message oauth
timebomb sweep --no-inert
timebomb sweep --summary
timebomb sweep --agent-summary
timebomb sweep --fix-plan json
timebomb sweep --output report.json
timebomb sweep --max-detonated 0
timebomb sweep --max-ticking 5
```

Exit codes:

- `0`: clean, or counts are within configured limits.
- `1`: detonated fuses, fail-on-ticking violation, or ratchet violation.
- `2`: configuration or runtime error.

## `manifest`

List matching fuses without failing.

```bash
timebomb manifest
timebomb manifest --detonated
timebomb manifest --ticking 14d
timebomb manifest --format json
timebomb manifest --format csv
timebomb manifest --format table
timebomb manifest --owner alice
timebomb manifest --tag TODO
timebomb manifest --message oauth
timebomb manifest --owner-missing --blame
timebomb manifest --path-only
timebomb manifest --file src/auth.rs
timebomb manifest --file "src/auth/**"
timebomb manifest --between 2026-01-01 2026-06-30
timebomb manifest --sort date
timebomb manifest --sort file
timebomb manifest --sort owner
timebomb manifest --sort status
timebomb manifest --next 10
timebomb manifest --count
```

`manifest` always exits `0` unless there is a runtime error.

## `armory`

Show the most urgent active fuses.

```bash
timebomb armory
timebomb armory --oldest
timebomb armory --count
timebomb armory --json
timebomb armory --limit 5
timebomb armory --owner alice
timebomb armory --tag FIXME
timebomb armory --message oauth
timebomb armory --fuse 14d
```

Detonated fuses are ranked before ticking fuses.

## `explain`

Focus on one fuse at `FILE:LINE`.

```bash
timebomb explain src/auth/login.rs:42
timebomb explain src/auth/login.rs:42 --path .
timebomb explain src/auth/login.rs:42 --blame
```

This is useful for agents that receive one failing CI annotation and need a
small remediation menu.

## `plant`

Insert a new fuse.

```bash
timebomb plant src/auth/login.rs:42 "remove after migration" --date 2026-06-01
timebomb plant src/auth/login.rs:42 "remove after migration" --in-days 90
timebomb plant src/auth.rs "remove oauth" --search legacy_auth --tag FIXME --owner alice --yes
```

## `delay`

Bump an existing fuse deadline.

```bash
timebomb delay src/auth/login.rs:42 --date 2026-09-01
timebomb delay src/auth/login.rs:42 --in-days 30 --reason "blocked on upstream fix"
```

## `disarm`

Remove a fuse.

```bash
timebomb disarm src/auth/login.rs:42
timebomb disarm --all-detonated
timebomb disarm --all-detonated --yes
```

## `defuse`

Interactively resolve detonated fuses.

```bash
timebomb defuse
timebomb defuse ./src
```

For each detonated fuse, choose extend, delete, or skip. File edits are applied
bottom-up per file so line numbers do not shift while multiple fuses are edited.

## `intel`

Count fuses by owner, tag, or expiry month.

```bash
timebomb intel
timebomb intel --by owner
timebomb intel --by tag
timebomb intel --by month
timebomb intel --by tag --format json
timebomb intel --message oauth
```

## `tripwire`

Manage a git pre-commit hook.

```bash
timebomb tripwire set --yes
timebomb tripwire cut --yes
```

The hook runs:

```sh
timebomb sweep --since HEAD .
```

## `fallout`

Compare two JSON report snapshots.

```bash
timebomb fallout report-jan.json report-feb.json
timebomb fallout --format json report-jan.json report-feb.json
```

## `bunker`

Save and enforce a baseline of detonated and ticking counts.

```bash
timebomb bunker save
timebomb bunker show
```

## `completions`

Print shell completion scripts.

```bash
timebomb completions bash
timebomb completions zsh
timebomb completions fish
```

