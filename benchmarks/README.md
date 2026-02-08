# Benchmark Suite

Performance benchmarks comparing the pyaot compiler with CPython.

## Quick Start

```bash
# From the benchmarks directory
./run_benchmarks.sh
```

This will:
1. Compile each benchmark program with pyaot (release mode)
2. Run the compiled version and measure execution time
3. Run the same program with CPython and measure execution time
4. Compare and display speedup/slowdown

## Prerequisites

- Rust toolchain (for building pyaot)
- CPython 3.x (for comparison)
- `time` command (for measurement)

The benchmark script will automatically build pyaot if needed.

## Benchmark Programs

### Computation Benchmarks

- **`bench_arithmetic_intensive.py`** - Deep recursion (fibonacci)
- **`bench_arithmetic.py`** - Mixed arithmetic and recursion
- **`bench_primes.py`** - Prime number computation
- **`bench_matrix_multiply.py`** - Nested loops (matrix multiply simulation)

### Collection Benchmarks

- **`bench_list_intensive.py`** - Large list with repeated iteration
- **`bench_list_ops.py`** - List append, slice, filter
- **`bench_string_ops.py`** - String concatenation, case, search
- **`bench_dict_ops.py`** - Dictionary insert, lookup, iteration

### Other Benchmarks

- **`bench_function_calls.py`** - Function call overhead
- **`bench_classes.py`** - Object creation and method calls

## Results

See [BENCHMARK_RESULTS.md](BENCHMARK_RESULTS.md) for detailed analysis.

**Summary:**
- **Computation-heavy code**: Up to 7.76x faster than CPython
- **Collection operations**: Currently 3-13x slower (optimization opportunity)

## Adding New Benchmarks

1. Create a new `bench_*.py` file in this directory
2. Add it to the `benchmarks` array in `run_benchmarks.sh`
3. Run `./run_benchmarks.sh` to include it in the suite

**Benchmark Guidelines:**
- Use type annotations (required by pyaot)
- Make workloads substantial enough that startup cost is not dominant
- Verify output correctness matches CPython
- Focus on one performance characteristic per benchmark

## Script Options

The `run_benchmarks.sh` script automatically:
- Builds pyaot in release mode if needed
- Compiles with proper runtime library path
- Uses `/usr/bin/time` for accurate measurement
- Handles different locale decimal formats (`,` vs `.`)
- Displays results with color coding (green=faster, red=slower)

## Notes

- Times are "real" (wall-clock) time from the `time` command
- Each benchmark runs once (no averaging) - add iterations internally for stable results
- Compilation time is not included in measurements
- Results may vary based on system load and other factors
