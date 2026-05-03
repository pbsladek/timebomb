---
layout: default
title: Design - Safety and Exit Codes
---

# Design: Safety and Exit Codes

## Exit Code Discipline

Only `main.rs` exits the process. Library modules return `Result` values.

| Code | Meaning |
| --- | --- |
| `0` | Success or informational command completed. |
| `1` | `sweep` found a policy violation. |
| `2` | Configuration or runtime error. |

`manifest`, `armory`, `intel`, `fallout`, `bunker show`, and other
informational commands do not fail just because they found detonated fuses.

## Sweep Failures

`sweep` exits `1` when any of these are true:

- A detonated fuse is present.
- `--fail-on-ticking` is set and a ticking fuse is present.
- `max_detonated` or `max_ticking` is exceeded.
- A saved baseline ratchet is exceeded.

## File Safety

The scanner does not follow symlinks. Files with null bytes are treated as
binary and skipped. Very large files are skipped to avoid unbounded memory use.

## Edit Safety

`defuse`, `delay`, and `disarm --all-detonated` avoid line-shift bugs by
applying edits from the bottom of each file upward.

## Configuration Safety

Unknown or malformed config fields are treated as errors. CLI overrides are
explicit and have higher priority than config file values.

