#!/usr/bin/env bash
# Run all benchmarks and print a clean, human-readable summary table.
#
# Usage:
#   ./benches/bench.sh                  # run + print table (default: 5s per bench)
#   ./benches/bench.sh --time 30        # run with 30s measurement time per bench
#   ./benches/bench.sh --no-run         # reformat last saved results without re-running

set -euo pipefail

RESULTS=/tmp/timebomb_bench_results.txt
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BENCH_TIME=""

# ── Parse arguments ────────────────────────────────────────────────────────────
NO_RUN=false
while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-run)
            NO_RUN=true
            shift
            ;;
        --time)
            BENCH_TIME="${2:?--time requires a value (seconds)}"
            shift 2
            ;;
        *)
            echo "Unknown argument: $1" >&2
            echo "Usage: $0 [--no-run] [--time <seconds>]" >&2
            exit 1
            ;;
    esac
done

cd "$REPO_ROOT"

# ── Run benchmarks unless --no-run is passed ──────────────────────────────────
if [[ "$NO_RUN" == false ]]; then
    if [[ -n "$BENCH_TIME" ]]; then
        echo "Building and running benchmarks (${BENCH_TIME}s per benchmark)…"
    else
        echo "Building and running benchmarks (this takes ~5 minutes)…"
    fi
    echo ""

    # Criterion CLI flags override the programmatic config in scanner_bench.rs.
    # --bench-time sets measurement time; --warm-up-time scales proportionally.
    CRITERION_ARGS=""
    if [[ -n "$BENCH_TIME" ]]; then
        WARMUP=$(( BENCH_TIME / 5 < 1 ? 1 : BENCH_TIME / 5 ))
        CRITERION_ARGS="-- --bench-time ${BENCH_TIME} --warm-up-time ${WARMUP}"
    fi

    # Stdout = criterion timing  |  Stderr = compilation progress (shown on terminal)
    # shellcheck disable=SC2086
    cargo bench $CRITERION_ARGS >"$RESULTS"
    echo ""
    echo "Results saved to $RESULTS"
    echo ""
fi

# ── Parse and format ──────────────────────────────────────────────────────────
python3 "$SCRIPT_DIR/bench_format.py" "$RESULTS"
