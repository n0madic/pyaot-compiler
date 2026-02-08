#!/bin/bash

# Run a single benchmark for quick testing

if [ $# -eq 0 ]; then
    echo "Usage: ./bench_one.sh <benchmark_name>"
    echo ""
    echo "Available benchmarks:"
    echo "  arithmetic_intensive"
    echo "  arithmetic"
    echo "  primes"
    echo "  matrix_multiply"
    echo "  function_calls"
    echo "  classes"
    echo "  list_intensive"
    echo "  list_ops"
    echo "  string_ops"
    echo "  dict_ops"
    exit 1
fi

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
PYAOT="$PROJECT_ROOT/target/release/pyaot"
RUNTIME_LIB="$PROJECT_ROOT/target/release/libpyaot_runtime.a"

NAME=$1
FILE="$SCRIPT_DIR/bench_${NAME}.py"

if [ ! -f "$FILE" ]; then
    echo "Error: Benchmark file not found: $FILE"
    exit 1
fi

# Ensure pyaot is built
if [ ! -f "$PYAOT" ]; then
    echo "Building pyaot..."
    cd "$PROJECT_ROOT"
    cargo build --release --workspace
fi

echo "Compiling $NAME..."
EXECUTABLE="$SCRIPT_DIR/bench_${NAME}_test"
"$PYAOT" "$FILE" -o "$EXECUTABLE" --runtime-lib "$RUNTIME_LIB"

echo ""
echo "Running compiled version..."
/usr/bin/time -p "$EXECUTABLE" 2>&1

echo ""
echo "Running CPython version..."
/usr/bin/time -p python3 "$FILE" 2>&1

# Cleanup
rm -f "$EXECUTABLE"
