---
layout: default
title: timebomb
---

# timebomb

`timebomb` scans source code for deadline-tagged TODO-style annotations and
fails when the deadline has passed. It is designed for CI, agents, and teams
that want temporary code to have explicit review dates.

## Start Here

```bash
cargo install timebomb-cli --locked
timebomb sweep .
```

Use this annotation format in any text source file:

```text
TODO[2026-06-01][alice]: remove this feature flag after rollout
```

When the date is in the past, `timebomb sweep` exits with code `1`.

## Documentation

- [Installation](installation.md)
- [Fuse format](fuse-format.md)
- [Commands](commands.md)
- [Configuration](configuration.md)
- [Output formats](output-formats.md)
- [CI and automation](ci.md)
- [Examples](examples/)
- [Design: architecture](design/architecture.md)
- [Design: safety and exit codes](design/safety-and-exit-codes.md)

## Examples

Real-world usage examples are available on the [Examples](examples/) page. The
most useful automation scenarios are also summarized in [CI and automation](ci.md).
