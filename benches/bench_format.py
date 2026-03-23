#!/usr/bin/env python3
"""Parse criterion benchmark output and print a human-readable summary table.

Usage:
    python3 bench_format.py <results_file>
"""

import sys
import re

# Criterion stdout format (one block per benchmark):
#   benchmark_name
#                           time:   [lo  median  hi]
#                           thrpt:  [lo  median  hi]   (optional)
#                           change: ...
#   Found N outliers ...

TIME_RE = re.compile(
    r"^\s+time:\s+\[([0-9.]+)\s+([µmn]?s)\s+([0-9.]+)\s+([µmn]?s)\s+([0-9.]+)\s+([µmn]?s)\]"
)
THRPT_RE = re.compile(
    r"^\s+thrpt:\s+\[([0-9.]+)\s+(\S+)\s+([0-9.]+)\s+(\S+)\s+([0-9.]+)\s+(\S+)\]"
)

SKIP_PREFIXES = (
    " ", "test ", "running ", "Bench", "Gnuplot", "Found", "Compiling", "Finished", "Running",
)


def to_ns(value: float, unit: str) -> float:
    return {"ns": 1, "µs": 1_000, "ms": 1_000_000, "s": 1_000_000_000}.get(unit, 1) * value


def fmt_time(ns: float) -> str:
    if ns < 1_000:
        return f"{ns:.1f} ns"
    elif ns < 1_000_000:
        return f"{ns / 1_000:.2f} µs"
    elif ns < 1_000_000_000:
        return f"{ns / 1_000_000:.2f} ms"
    else:
        return f"{ns / 1_000_000_000:.3f} s"


def parse(lines: list[str]) -> list[tuple[str, float, str]]:
    benchmarks: list[tuple[str, float, str]] = []  # (name, median_ns, thrpt_str)
    current_name: str | None = None

    for line in lines:
        stripped = line.rstrip()

        # A benchmark name line: no leading spaces, not a status/progress line
        if stripped and not any(stripped.startswith(p) for p in SKIP_PREFIXES):
            inline_time = TIME_RE.search(stripped)
            if inline_time:
                name = stripped[: stripped.index("time:")].strip()
                median_ns = to_ns(float(inline_time.group(3)), inline_time.group(4))
                benchmarks.append((name, median_ns, ""))
                current_name = None
            else:
                current_name = stripped
            continue

        m_time = TIME_RE.match(stripped)
        if m_time and current_name is not None:
            median_ns = to_ns(float(m_time.group(3)), m_time.group(4))
            benchmarks.append((current_name, median_ns, ""))
            current_name = None
            continue

        m_thrpt = THRPT_RE.match(stripped)
        if m_thrpt and benchmarks:
            name, ns, _ = benchmarks[-1]
            thrpt = f"{float(m_thrpt.group(3)):.2f} {m_thrpt.group(4)}"
            benchmarks[-1] = (name, ns, thrpt)

    return benchmarks


def print_table(benchmarks: list[tuple[str, float, str]]) -> None:
    col_name = max(len(b[0]) for b in benchmarks) + 2
    col_time = 14
    col_thrpt = 20

    header = f"{'Benchmark':<{col_name}}  {'Median time':>{col_time}}  {'Throughput':<{col_thrpt}}"
    sep = "─" * len(header)

    print(sep)
    print(header)
    print(sep)

    prev_group: str | None = None
    for name, ns, thrpt in benchmarks:
        group = name.split("/")[0] if "/" in name else name
        if prev_group is not None and group != prev_group:
            print()
        prev_group = group

        time_str = fmt_time(ns)
        thrpt_str = thrpt if thrpt else "—"
        print(f"{name:<{col_name}}  {time_str:>{col_time}}  {thrpt_str:<{col_thrpt}}")

    print(sep)
    print(f"  {len(benchmarks)} benchmarks")


def main() -> None:
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} <results_file>", file=sys.stderr)
        sys.exit(2)

    results_file = sys.argv[1]

    try:
        with open(results_file) as f:
            lines = f.readlines()
    except FileNotFoundError:
        print(f"No results file found at {results_file}. Run without --no-run first.")
        sys.exit(1)

    benchmarks = parse(lines)

    if not benchmarks:
        print("No benchmark results found. Did the bench run complete successfully?")
        sys.exit(1)

    print_table(benchmarks)


if __name__ == "__main__":
    main()
