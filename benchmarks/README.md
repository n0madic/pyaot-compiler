# Benchmarks

Performance harness for Phase 9. **Not** part of the differential gate
(`PHASE_CORPUS`) — correctness is still validated on every run by diffing each
bench's stdout against CPython, but timing lives here, outside `cargo test`,
because wall-clock measurements in test runners flake and would pollute the
correctness gate.

## Method

`benchmarks/run.sh [label]`:

1. `cargo build --release -p pyaot-cli -p pyaot-runtime` (release — the debug
   runtime carries assertion overhead and is not representative).
2. For each bench: compile with `pyaot`, run once, **diff stdout against
   `python3`** — a bench with a wrong answer is invalid and is reported as
   `OUTPUT MISMATCH` instead of a time.
3. Time both binaries with `hyperfine --warmup 2 --min-runs 10` (fallback:
   `_timer.py`, same warmup/run counts, mean wall-clock).
4. Append a markdown table (bench | pyaot | cpython | ratio) to `results.md`
   stamped with the date and git revision.

Every bench prints an exact checksum (integers, or floats whose IEEE repr is
deterministic) so the CPython diff is byte-for-byte.

## The benches

| bench | what it measures |
|---|---|
| `bench_int_loop` | fixnum arithmetic in tight loops (collatz, iterative fib) |
| `bench_float_kernel` | annotated float kernel → `Raw(F64)` specialization (mandelbrot) |
| `bench_calls` | microgpt proxy: dunder calls + closures + per-iteration allocation — the inliner target |
| `bench_str` | str(int), join, count/find, case mapping, slicing |
| `bench_containers` | list append/iterate, dict insert/get, membership |
| `bench_exc_hotpath` | hot loop inside a never-raising `try` — has_try memory-backing tax (B17) + cold-block effect |
| `microgpt` | `corpus/microgpt.py` end-to-end (the real workload) |

## Targets (ratified against the Phase-8 baseline in `results.md`)

Ratified after the baseline run — the pre-baseline aspirational numbers
(≥10x CPython on scalar loops) assumed unboxed loop arithmetic, but the
tagged-baseline design routes every `int` op through `rt_obj_*` tag dispatch
(bignum-safe by Invariant 2), which CPython's specialized interpreter loop
matches. Beating it ≥10x needs proof-gated raw-int loop specialization —
future work, not a Phase 9 gate.

- `microgpt` (the real workload): **beat CPython** and ≥ 25% faster than the
  Phase-8 pyaot baseline. *Met:* 1.06 s → 0.041 s (26x vs baseline,
  1.1x vs CPython).
- `bench_float_kernel`: ≥ 2x CPython (annotated floats stay `Raw(F64)`).
  *Met:* 2.2x.
- `bench_calls`: ≥ 15% faster than the Phase-8 pyaot baseline (the
  inliner's lever). *Met:* +17%, 1.2x CPython.
- Tagged-dispatch-bound benches (`bench_int_loop`, `bench_str`,
  `bench_containers`, `bench_exc_hotpath`): no regression vs baseline.
  *Met:* all within noise of baseline.
- Regression rule: no bench may degrade > 5% on any subsequent step.
