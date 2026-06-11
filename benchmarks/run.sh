#!/bin/bash
# Benchmark harness: compile each bench with the release pyaot, validate its
# stdout against CPython (a bench with a wrong answer is invalid), then time
# pyaot vs python3 (hyperfine when available, _timer.py otherwise) and append
# a markdown table to results.md.
#
# Usage: benchmarks/run.sh [label]
#   label — optional tag for the results table header (default: git describe).

set -u -o pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$SCRIPT_DIR")"
OUT_DIR="${TMPDIR:-/tmp}/pyaot_bench"
RESULTS="$SCRIPT_DIR/results.md"
LABEL="${1:-}"

WARMUP=2
MIN_RUNS=10

BENCHES=(
    "$SCRIPT_DIR/bench_int_loop.py"
    "$SCRIPT_DIR/bench_float_kernel.py"
    "$SCRIPT_DIR/bench_calls.py"
    "$SCRIPT_DIR/bench_str.py"
    "$SCRIPT_DIR/bench_containers.py"
    "$SCRIPT_DIR/bench_exc_hotpath.py"
    "$ROOT/corpus/microgpt.py"
)

echo "== Building release pyaot + runtime =="
cargo build --release -p pyaot-cli -p pyaot-runtime --manifest-path "$ROOT/Cargo.toml" || exit 1

PYAOT="$ROOT/target/release/pyaot"
RUNTIME_LIB="$ROOT/target/release/libpyaot_runtime.a"
mkdir -p "$OUT_DIR"

# time_cmd <cmd...> — echo mean seconds for a command.
time_cmd() {
    if command -v hyperfine >/dev/null 2>&1; then
        local json="$OUT_DIR/hyperfine.json"
        hyperfine --warmup "$WARMUP" --min-runs "$MIN_RUNS" --export-json "$json" \
            --style none "$(printf '%q ' "$@")" >/dev/null || return 1
        python3 -c "import json,sys; print(f\"{json.load(open(sys.argv[1]))['results'][0]['mean']:.6f}\")" "$json"
    else
        python3 "$SCRIPT_DIR/_timer.py" "$WARMUP" "$MIN_RUNS" "$@"
    fi
}

GIT_REV="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
DATE="$(date '+%Y-%m-%d %H:%M')"
ROWS=()
FAILED=0

for src in "${BENCHES[@]}"; do
    name="$(basename "$src" .py)"
    exe="$OUT_DIR/$name"
    echo "== $name =="

    if ! "$PYAOT" "$src" -o "$exe" --runtime-lib "$RUNTIME_LIB"; then
        echo "  COMPILE FAILED"; FAILED=1
        ROWS+=("| $name | COMPILE FAILED | — | — |")
        continue
    fi

    # Validate: the bench result must match CPython, else the timing is moot.
    "$exe" > "$OUT_DIR/$name.pyaot.out" 2>"$OUT_DIR/$name.pyaot.err" || {
        echo "  RUN FAILED (see $OUT_DIR/$name.pyaot.err)"; FAILED=1
        ROWS+=("| $name | RUN FAILED | — | — |")
        continue
    }
    python3 "$src" > "$OUT_DIR/$name.cpython.out" || {
        echo "  python3 FAILED"; FAILED=1
        ROWS+=("| $name | CPYTHON FAILED | — | — |")
        continue
    }
    if ! diff -q "$OUT_DIR/$name.pyaot.out" "$OUT_DIR/$name.cpython.out" >/dev/null; then
        echo "  OUTPUT MISMATCH vs CPython — bench invalid"
        diff "$OUT_DIR/$name.pyaot.out" "$OUT_DIR/$name.cpython.out" | head -10
        FAILED=1
        ROWS+=("| $name | OUTPUT MISMATCH | — | — |")
        continue
    fi

    t_pyaot="$(time_cmd "$exe")" || { echo "  timing failed"; FAILED=1; continue; }
    t_py="$(time_cmd python3 "$src")" || { echo "  timing failed"; FAILED=1; continue; }
    ratio="$(python3 -c "import sys; print(f'{float(sys.argv[2])/float(sys.argv[1]):.2f}x')" "$t_pyaot" "$t_py")"
    echo "  pyaot ${t_pyaot}s | cpython ${t_py}s | ${ratio} faster"
    ROWS+=("| $name | ${t_pyaot}s | ${t_py}s | $ratio |")
done

{
    echo ""
    echo "## ${DATE} — ${GIT_REV}${LABEL:+ — $LABEL}"
    echo ""
    echo "| bench | pyaot | cpython | ratio (cpython/pyaot) |"
    echo "|---|---|---|---|"
    for row in "${ROWS[@]}"; do echo "$row"; done
} >> "$RESULTS"

echo ""
echo "Results appended to $RESULTS"
exit $FAILED
