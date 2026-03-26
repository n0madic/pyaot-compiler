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
| Arithmetic Intensive (Fibonacci) | 197ms | 1512ms | **7.7x faster** | Computation |
| Arithmetic (Mixed) | 11ms | 112ms | **10.5x faster** | Computation |
| Primes | 1.9ms | 17ms | **9.0x faster** | Computation |
| Matrix Multiply | 4.0ms | 42ms | **10.6x faster** | Computation |
| Function Calls | 2.7ms | 26ms | **9.6x faster** | Control Flow |
| Classes | 2.1ms | 18ms | **8.6x faster** | OOP |
| List Intensive | 3.8ms | 30ms | **7.9x faster** | Collections |
| List Operations | 2.1ms | 16ms | **7.8x faster** | Collections |
| String Operations | 34ms | 59ms | **1.8x faster** | Collections |
| Dictionary Operations | 2.2ms | 17ms | **7.6x faster** | Collections |

### Heavy Allocation Benchmarks (100K iterations)

| Benchmark | Compiled | CPython | Speedup |
|-----------|----------|---------|---------|
| Class instances + method calls | 3.7ms | 29ms | **7.8x faster** |
| Dict 100K insertions + lookups | 6.4ms | 25ms | **3.8x faster** |
| String creation + concatenation | 5.0ms | 19ms | **3.7x faster** |
| List append + iteration | 2.2ms | 20ms | **9.0x faster** |

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
2. **Slab allocator** — Bump-pointer allocation from 4KB pages for objects ≤64 bytes (IntObj, FloatObj, StrObj, ListObj, DictObj, InstanceObj). Replaces system malloc (~50ns) with O(1) bump (~3ns). Free-list recycling on sweep.
3. **Eliminated Vec tracking for small objects** — Slab-allocated objects are swept by iterating slab pages directly, avoiding `Vec::push` per allocation and `Vec::retain` per sweep cycle
4. **Small string optimization** — String allocation sizes rounded to slab classes (24/32/48/64 bytes), ensuring most strings use slab bump allocation
5. **Pointer equality fast path** — `eq_hashable_obj` checks `a == b` first, catching interned strings and pooled integers
6. **Optimized string comparison** — Replaced byte-by-byte loops with `slice` comparison (uses SIMD `memcmp`)
7. **Prior optimizations** — Timsort, triangular probing, SplitMix64 hashing, Boyer-Moore-Horspool string search, StringBuilder

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
