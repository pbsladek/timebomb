---
layout: default
title: Design - Architecture
---

# Design: Architecture

## Goals

`timebomb` is built around a small set of constraints:

- Scan source-like text quickly.
- Keep the scanner language-agnostic.
- Make CI behavior deterministic.
- Keep library code free of process exits.
- Support interactive edits without line-shift bugs.

## Scanner Structure

The scanner has three phases:

1. Serial directory walk.
2. Parallel file scan with Rayon.
3. Serial flattening and sorting.

The directory walk applies cheap path filters. File contents are scanned in
parallel. Results are flattened and sorted by expiry date so the most urgent
fuses appear first.

## Date Handling

The current date is resolved once in `main.rs` and passed through the scanner.
The scanner never fetches "today" internally. This keeps scans consistent across
midnight and makes tests deterministic.

## Regex Handling

The fuse regex is built once per scan from the configured triggers and shared by
scanner workers. Other module-local regexes use static caching where appropriate.

## Output Layer

Output rendering is separated from scanning. Commands prepare a filtered
`ScanResult` or fuse list, then dispatch to terminal, JSON, GitHub Actions, CSV,
table, or agent-focused renderers.

## Editing Commands

Commands that edit files collect decisions first and then apply changes in
descending line-number order per file. This prevents one edit from shifting the
line numbers for later edits in the same file.

