---
layout: default
title: Fuse Format
---

# Fuse Format

`timebomb` looks for annotations that combine a tag, an expiry date, an optional
owner, and a message.

```text
TODO[2026-06-01]: remove after migration
FIXME[2026-03-15][alice]: replace workaround after upstream release
HACK[2025-12-31][platform]: temporary shim for legacy payloads
```

## Syntax

```text
TAG[YYYY-MM-DD]: message
TAG[YYYY-MM-DD][owner]: message
```

The tag must be immediately followed by the date bracket. Plain comments such as
`TODO: fix this later` are ignored.

## Default Tags

The default trigger set is:

```text
TODO, FIXME, HACK, TEMP, REMOVEME, DEBT, STOPSHIP, WORKAROUND, DEPRECATED, BUG
```

Tags are case-insensitive. The scanner stores matched tags in uppercase.

## Status

| Status | Meaning |
| --- | --- |
| `detonated` | The date is before today. |
| `ticking` | The date is today or within the configured `fuse_days` window. |
| `inert` | The date is outside the warning window. |

The current date is resolved once at startup and then passed through the scan so
long-running scans remain consistent.

## Language Handling

The scanner is language-agnostic. It matches the fuse pattern anywhere on a line
and does not parse source code syntax. These all work:

```rust
// TODO[2026-06-01]: Rust comment
```

```python
# FIXME[2026-06-01][alice]: Python comment
```

```sql
-- HACK[2026-06-01]: SQL comment
```

## Ignoring Existing TODOs

Existing TODO comments without a bracketed date are ignored. This lets teams
adopt `timebomb` incrementally without rewriting all historical comments.

