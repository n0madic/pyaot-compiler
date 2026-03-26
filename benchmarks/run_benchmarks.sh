#!/bin/bash

# Benchmark runner script
# Compares performance of compiled Python vs CPython
# Uses high-precision timing with warm-up to avoid first-run overhead

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

# Compile all benchmarks first
echo -e "${BLUE}Compiling all benchmarks...${NC}"
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

executables=()
for benchmark in "${benchmarks[@]}"; do
    IFS=':' read -r name file <<< "$benchmark"
    executable="$SCRIPT_DIR/bench_$name"
    "$PYAOT" "$SCRIPT_DIR/$file" -o "$executable" --runtime-lib "$RUNTIME_LIB" 2>&1 | grep -v "^$" || true
    if [ -f "$executable" ]; then
        # Warm-up run (macOS Gatekeeper check)
        "$executable" > /dev/null 2>&1
        executables+=("$name:$executable:$SCRIPT_DIR/$file")
    else
        echo -e "${RED}Failed to compile $file${NC}"
    fi
done
echo ""

# Run benchmarks with high-precision timing
echo -e "${BLUE}Running benchmarks (best of 5 runs, after warm-up)...${NC}"
echo ""

printf "${BLUE}%-28s %10s %10s %12s${NC}\n" "Benchmark" "Compiled" "CPython" "Speedup"
echo "--------------------------------------------------------------"

for entry in "${executables[@]}"; do
    IFS=':' read -r name executable file <<< "$entry"

    # High-precision timing via Python
    result=$(python3 -c "
import subprocess, time

# Compiled version (5 runs, take 2nd best)
times_c = []
for _ in range(5):
    start = time.perf_counter()
    subprocess.run(['$executable'], capture_output=True)
    times_c.append((time.perf_counter() - start) * 1000)
times_c.sort()
compiled = times_c[1]

# CPython version (5 runs, take 2nd best)
times_p = []
for _ in range(5):
    start = time.perf_counter()
    subprocess.run(['python3', '$file'], capture_output=True)
    times_p.append((time.perf_counter() - start) * 1000)
times_p.sort()
cpython = times_p[1]

ratio = cpython / compiled if compiled > 0 else 0
if ratio >= 1:
    speedup = f'{ratio:.1f}x faster'
else:
    speedup = f'{1/ratio:.1f}x slower'

if compiled >= 1000:
    c_str = f'{compiled/1000:.2f}s'
elif compiled >= 100:
    c_str = f'{compiled:.0f}ms'
else:
    c_str = f'{compiled:.1f}ms'

if cpython >= 1000:
    p_str = f'{cpython/1000:.2f}s'
elif cpython >= 100:
    p_str = f'{cpython:.0f}ms'
else:
    p_str = f'{cpython:.1f}ms'

print(f'{c_str}|{p_str}|{speedup}|{ratio}')
" 2>/dev/null)

    compiled_str=$(echo "$result" | cut -d'|' -f1)
    cpython_str=$(echo "$result" | cut -d'|' -f2)
    speedup=$(echo "$result" | cut -d'|' -f3)
    ratio=$(echo "$result" | cut -d'|' -f4)

    if echo "$speedup" | grep -q "faster"; then
        color="${GREEN}"
    else
        color="${RED}"
    fi

    printf "%-28s %10s %10s ${color}%12s${NC}\n" "$name" "$compiled_str" "$cpython_str" "$speedup"
done

echo ""

# Cleanup
for entry in "${executables[@]}"; do
    IFS=':' read -r name executable file <<< "$entry"
    rm -f "$executable"
done

echo -e "${GREEN}All benchmarks completed!${NC}"
