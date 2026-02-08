#!/bin/bash

# Benchmark runner script
# Compares performance of compiled Python vs CPython

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
PYAOT="$PROJECT_ROOT/target/release/pyaot"
RUNTIME_LIB="$PROJECT_ROOT/target/release/libpyaot_runtime.a"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo "========================================"
echo "Python Compiler Benchmark Suite"
echo "========================================"
echo ""

# Check if pyaot is built
if [ ! -f "$PYAOT" ]; then
    echo -e "${YELLOW}Building pyaot in release mode...${NC}"
    cd "$PROJECT_ROOT"
    cargo build --release --workspace
    echo ""
fi

# Verify runtime library exists
if [ ! -f "$RUNTIME_LIB" ]; then
    echo -e "${RED}Error: Runtime library not found at $RUNTIME_LIB${NC}"
    echo "Please run: cargo build --release -p pyaot-runtime"
    exit 1
fi

# Function to run a single benchmark
run_benchmark() {
    local name=$1
    local file=$2

    echo -e "${BLUE}Benchmark: $name${NC}"
    echo "----------------------------------------"

    # Compile with pyaot
    local executable="$SCRIPT_DIR/bench_$name"
    echo "Compiling..."
    "$PYAOT" "$file" -o "$executable" --runtime-lib "$RUNTIME_LIB" 2>&1 | grep -v "^$" || true

    if [ ! -f "$executable" ]; then
        echo -e "${RED}Compilation failed${NC}"
        return 1
    fi

    # Run compiled version
    echo -e "\n${GREEN}Compiled (pyaot):${NC}"
    /usr/bin/time -p "$executable" 2>&1 | tee /tmp/pyaot_output.txt
    local pyaot_time=$(grep "^real" /tmp/pyaot_output.txt | awk '{print $2}')

    # Run CPython version
    echo -e "\n${YELLOW}CPython 3:${NC}"
    /usr/bin/time -p python3 "$file" 2>&1 | tee /tmp/cpython_output.txt
    local cpython_time=$(grep "^real" /tmp/cpython_output.txt | awk '{print $2}')

    # Calculate speedup using Python (more portable than bc)
    echo -e "\n${GREEN}Results:${NC}"
    echo "  Compiled: ${pyaot_time}s"
    echo "  CPython:  ${cpython_time}s"

    # Use Python for calculation to handle different locales
    local speedup=$(python3 -c "
import sys
try:
    pyaot = float('${pyaot_time}'.replace(',', '.'))
    cpython = float('${cpython_time}'.replace(',', '.'))
    if pyaot > 0:
        ratio = cpython / pyaot
        if ratio >= 1:
            print(f'{ratio:.2f}x faster')
        else:
            print(f'{1/ratio:.2f}x slower')
    else:
        print('N/A')
except:
    print('N/A')
" 2>/dev/null || echo "N/A")

    if [[ "$speedup" == *"faster"* ]]; then
        echo -e "  ${GREEN}Speedup:  $speedup${NC}"
    elif [[ "$speedup" == *"slower"* ]]; then
        echo -e "  ${RED}Speedup:  $speedup${NC}"
    else
        echo "  Speedup:  $speedup"
    fi

    # Cleanup
    rm -f "$executable"
    echo ""
}

# Run all benchmarks
benchmarks=(
    "arithmetic_intensive:bench_arithmetic_intensive.py"
    "matrix_multiply:bench_matrix_multiply.py"
    "primes:bench_primes.py"
    "arithmetic:bench_arithmetic.py"
    "function_calls:bench_function_calls.py"
    "classes:bench_classes.py"
    "list_intensive:bench_list_intensive.py"
    "list_ops:bench_list_ops.py"
    "string_ops:bench_string_ops.py"
    "dict_ops:bench_dict_ops.py"
)

echo -e "${BLUE}Running benchmarks...${NC}"
echo ""

for benchmark in "${benchmarks[@]}"; do
    IFS=':' read -r name file <<< "$benchmark"
    run_benchmark "$name" "$SCRIPT_DIR/$file"
    echo "========================================"
    echo ""
done

echo -e "${GREEN}All benchmarks completed!${NC}"
