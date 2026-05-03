---
layout: default
title: CI and Automation
---

# CI and Automation

## GitHub Actions

```yaml
name: timebomb
on:
  push:
  pull_request:
  schedule:
    - cron: '0 9 * * *'

jobs:
  timebomb:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd # v6
      - name: Install timebomb
        run: |
          curl -sSL https://github.com/pbsladek/timebomb/releases/latest/download/timebomb-linux-x86_64 \
            -o /usr/local/bin/timebomb
          chmod +x /usr/local/bin/timebomb
      - run: timebomb sweep --fuse 14d --fail-on-ticking
```

GitHub Actions output is selected automatically when `GITHUB_ACTIONS=true`.

## Changed Lines Only

```bash
timebomb sweep --changed --base main
```

Use this when a CI job should only fail on fuses added or touched by the current
change.

## Since a Git Ref

```bash
timebomb sweep --since HEAD
```

This restricts scanning to files changed relative to the given ref.

## Agent-Friendly CI Output

```bash
timebomb sweep --agent-summary
timebomb sweep --fix-plan json
```

Use `--agent-summary` when a bot or agent should report the failure in a compact
text block. Use `--fix-plan json` when an agent should parse specific remediation
actions.

## Pre-commit Hook

```bash
timebomb tripwire set --yes
```

The hook checks changed files before commit:

```sh
timebomb sweep --since HEAD .
```

## Ratchet Enforcement

For repositories with existing fuse debt:

```bash
timebomb bunker save
timebomb sweep
```

Commit `.timebomb-baseline.json`. Future sweeps fail only when detonated or
ticking counts grow beyond the baseline or configured ceilings.

