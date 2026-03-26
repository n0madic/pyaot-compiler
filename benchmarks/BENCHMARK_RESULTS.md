# Benchmark Results: pyaot vs CPython

Comparison of the Rust-based Python AOT compiler (pyaot) with CPython 3.

**Test Environment:**
- Platform: macOS (Darwin 25.3.0)
- Architecture: ARM64 (Apple Silicon)
- Compiler: pyaot (release build with Cranelift backend)
- Interpreter: CPython 3 (system default)
- Timing: `time.perf_counter()` (millisecond precision), best of 5 runs after warm-up

## Summary

| Benchmark | Compiled | CPython | Speedup | Category |
|-----------|----------|---------|---------|----------|
| Arithmetic Intensive (Fibonacci) | 180ms | 1539ms | **8.6x faster** | Computation |
| Arithmetic (Mixed) | 11ms | 114ms | **10.4x faster** | Computation |
| Primes | 2.0ms | 17ms | **8.5x faster** | Computation |
| Matrix Multiply | 4.3ms | 42ms | **9.9x faster** | Computation |
| Function Calls | 2.7ms | 24ms | **9.1x faster** | Control Flow |
| Classes | 2.1ms | 17ms | **8.1x faster** | OOP |
| List Intensive | 4.3ms | 31ms | **7.2x faster** | Collections |
| List Operations | 1.9ms | 16ms | **8.4x faster** | Collections |
| String Operations | 34ms | 60ms | **1.7x faster** | Collections |
| Dictionary Operations | 3.0ms | 17ms | **5.7x faster** | Collections |

## Key Findings

### Strengths

**1. Computation (8-10x faster)**
- Deep recursion, arithmetic loops, and prime computation all show ~8-10x speedup
- Cranelift generates efficient native code for tight loops and recursion
- Static typing enables register allocation and instruction selection optimization

**2. Control Flow & OOP (8-9x faster)**
- Function calls: 9.1x faster — no interpreter dispatch overhead
- Class instances and method dispatch: 8.1x faster — vtable dispatch is 2 loads + indirect call
- Lock-free runtime eliminates synchronization overhead on all hot paths

**3. Collections (1.7-8.4x faster)**
- List operations: 8.4x faster with CPython-compatible Timsort, elem_tag specialization
- Dictionary operations: 5.7x faster with SplitMix64 hashing, triangular probing
- String operations: 1.7x faster with Boyer-Moore-Horspool search, StringBuilder

## Performance History

### Before optimization (baseline)
All benchmarks used `Mutex`/`RwLock` on every hot-path operation (gc_push/gc_pop per function call, gc_alloc per allocation, boxing pools, string interning, globals, vtable). This added ~0.25s of synchronization overhead to every benchmark despite being a single-threaded program.

| Benchmark | Before | After | Improvement |
|-----------|--------|-------|-------------|
| Primes | 11x slower | **8.5x faster** | ~94x |
| Function Calls | 7.8x slower | **9.1x faster** | ~71x |
| Classes | 9.3x slower | **8.1x faster** | ~75x |
| List Operations | 9.0x slower | **8.4x faster** | ~76x |
| Dict Operations | 14x slower | **5.7x faster** | ~80x |
| String Operations | 6.6x slower | **1.7x faster** | ~11x |

### Optimizations Applied
1. **Lock-free runtime** — Replaced 10+ `Mutex`/`RwLock` statics with `UnsafeCell`/`AtomicPtr` (safe for single-threaded AOT-compiled Python)
2. **Pointer equality fast path** — `eq_hashable_obj` checks `a == b` first, catching interned strings and pooled integers
3. **Optimized string comparison** — Replaced byte-by-byte loops with `slice` comparison (uses SIMD `memcmp`)
4. **Prior optimizations** — Timsort, triangular probing, SplitMix64 hashing, Boyer-Moore-Horspool string search, StringBuilder

## Important Note: First-Run Overhead

On macOS, newly compiled binaries incur a one-time ~250ms Gatekeeper/code signing verification penalty on first execution. This does NOT affect subsequent runs. Benchmarks above use warm-up runs to exclude this OS-level overhead.

## Benchmark Descriptions

### Arithmetic Intensive
Computes `fibonacci(32)` ten times using recursive implementation. Pure computation with minimal runtime calls.

### Arithmetic (Mixed)
Integer arithmetic (1M iterations) plus `fibonacci(30)`. Mix of tight loops and recursion.

### Primes
Finds all prime numbers up to 10,000 using trial division. Moderate computation with branching.

### Matrix Multiply
Triple-nested loop simulating 100x100 matrix multiplication. Pure arithmetic with loop overhead.

### Function Calls
100K iterations calling small functions (add/multiply). Tests function call overhead.

### Classes
Creates 10K class instances and calls methods. Tests object allocation and method dispatch.

### List Intensive
Creates 100K element list, then iterates 10 times summing elements. Tests list iteration.

### List Operations
List creation, iteration, slicing, and filtering (10K elements). Tests diverse list operations.

### String Operations
String concatenation, case conversion, and searching (10K iterations). Tests string runtime.

### Dictionary Operations
Dict creation (10K entries), lookup, and iteration. Tests hash table runtime.

---

*Updated: 2026-03-26*
*Compiler: pyaot (Rust + Cranelift)*
*Interpreter: CPython 3*
