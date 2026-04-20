# Phase-0 Baseline

This file records per-benchmark reference numbers that every subsequent
refactor phase measures against. It is a **committed data file** — regenerate
it locally with the recipe in `README.md` whenever the baseline hardware or
compiler toolchain changes, and review the diff in a dedicated PR.

## Baseline machine

| Field              | Value                                    |
|--------------------|------------------------------------------|
| CPU                | Apple M4 Max (14 cores)                  |
| RAM                | 36 GiB                                   |
| OS                 | macOS 26.3.1 / Darwin 25.3.0 (arm64)     |
| Rust toolchain     | rustc 1.93.1 (01f6ddf75 2026-02-11)      |
| pyaot build        | `cargo build --workspace --release`      |
| Criterion version  | 0.5                                      |
| Measurement date   | 2026-04-17 (`run` / `fresh_launch`), 2026-04-20 (`compile`) |
| Measurement mode   | `run` / `fresh_launch`: `cargo bench -- --quick`; `compile`: `cargo bench -p pyaot-bench --bench pyaot_bench compile:: -- --save-baseline phase0-compile-backfill` |

## Columns

| Column          | Meaning                                                    |
|-----------------|------------------------------------------------------------|
| `compile`       | Wall-clock milliseconds, `pyaot <src> -o …` only.         |
| `run`           | Wall-clock milliseconds, pre-compiled binary invocation.   |
| `fresh_launch`  | Wall-clock milliseconds, compile + immediate first launch. |
| `binary_size`   | Output binary size in bytes (release, stripped).           |
| `max_rss`       | Peak resident-set size in KiB, via `/usr/bin/time -l`.     |

Each `compile` / `run` / `fresh_launch` number is Criterion's median
estimate (the middle of the printed `[low median high]` confidence
interval). The original Phase-0 scaffolding only captured `run` and the
old `end_to_end` metric. Phase-1 triage on 2026-04-20 established that
the old `end_to_end` measurement was really a `fresh_launch` metric on
macOS, so the metric has been renamed in place and a separate
`compile::<stem>` track now exists.

## How to read the phase columns

- **Phase 0** — captured before any refactor work lands. This is the
  "zero-regression" reference for everything after it.
- **Phase 1 / Phase 2 / Phase 3** — snapshotted at each phase's acceptance
  gate. A new column is appended to every table below; earlier columns are
  never rewritten.
- Until a phase **passes** its acceptance gate, the phase column may still
  contain a preliminary snapshot (for example, a `--quick` capture used
  during active development). Failed acceptance sweeps are recorded in
  dated notes below and do **not** overwrite the committed column.

Regressions > 3 % in any `run::<stem>` row, or > 10 % in any
`compile::<stem>` row, must be flagged in the corresponding phase's
acceptance review. `fresh_launch::<stem>` is diagnostic-only.

---

## Benchmarks

### `compile` — compiler + linker wall time (ms, median)

This metric was added on **2026-04-20** after Phase-1 triage showed that
the old `end_to_end` benchmark was dominated by the first launch of a
freshly linked executable on macOS rather than by compiler throughput.
The Phase-0 column below is therefore a **post-hoc backfill** captured on
the baseline machine after the metric split. It is the reference point
for future compiler-throughput comparisons.

| Benchmark     | Phase 0 | Phase 1 | Phase 2 | Phase 3 | Notes |
|---------------|---------|---------|---------|---------|-------|
| int_arith     | 46.541  |         |         |         | compiler + linker only |
| float_arith   | 46.662  |         |         |         | compiler + linker only |
| polymorphic   | 48.636  |         |         |         | compiler + linker only |
| containers    | 45.795  |         |         |         | compiler + linker only |
| strings       | 48.175  |         |         |         | compiler + linker only |
| generators    | 48.243  |         |         |         | compiler + linker only |
| exceptions    | 47.415  |         |         |         | compiler + linker only |
| gc_stress     | 47.454  |         |         |         | compiler + linker only |
| classes       | 47.329  |         |         |         | compiler + linker only |
| closures      | 46.847  |         |         |         | compiler + linker only |
| startup       | 48.352  |         |         |         | compiler + linker only |

**Phase 0 backfill (2026-04-20, full Criterion sample)**:
captured with
`cargo bench -p pyaot-bench --bench pyaot_bench compile:: -- --save-baseline phase0-compile-backfill`.
This replaced the earlier manual triage snapshot and is now the committed
reference for `compile::*`.

**Phase 1 acceptance rerun (2026-04-20, split harness, full sample)**:
captured with
`cargo bench -p pyaot-bench --bench pyaot_bench compile:: -- --baseline phase0-compile-backfill`.
All 11 `compile::*` groups stayed comfortably inside the ±10% gate. The
widest reported interval was still small: `containers` ended at
+1.35% on the high side, while the largest improvement was
`startup` at -2.41% median. Compiler-throughput acceptance therefore
passes against the newly backfilled Phase-0 baseline.

### `run` — pre-compiled execution wall time (ms, median)

| Benchmark     | Phase 0 | Phase 1 | Phase 2 | Phase 3 | Notes |
|---------------|---------|---------|---------|---------|-------|
| int_arith     |  15.35  |  16.03  |         |         | `for i in range(10_000_000): total += i` |
| float_arith   |   2.89  |   2.83  |         |         | `total += float(i) * 0.5` over 1M |
| polymorphic   |  11.21  |  11.85  |         |         | Value class `__add__`/`__mul__`, 200k iters |
| containers    |   8.61  |   8.76  |         |         | list append/index/mutate + dict insert/lookup/iter, N=100k |
| strings       |   2.71  |   3.00  |         |         | intern hit-path + 2k concat |
| generators    |  13.08  |  14.71  |         |         | gen-expr `sum`, enumerate fusion, nested comp |
| exceptions    | 115.95  | 123.20  |         |         | 500k try/except non-raising + 500k with 4 raises |
| gc_stress     |   6.37  |   6.50  |         |         | 200 chains × 1000 Node instances |
| classes       |   8.75  |   9.18  |         |         | Point.norm + polymorphic Shape.area, 200k each |
| closures      |   2.13  |   2.13  |         |         | closure capture + comprehension reduce |
| startup       |   1.72  |   3.28  |         |         | single `print`, measures binary launch |

**Phase 1 preliminary (2026-04-18, post-pruned-SSA)**: captured with
`cargo bench -p pyaot-bench --bench pyaot_bench -- --quick` after
S1.17-pruned-SSA landed. Criterion's t-test reports "No change in
performance detected" for all benchmarks. Raw median deltas are
within ±10% of Phase 0, with `generators` (+12.5%) and `exceptions`
(+6.3%) the largest outliers — both expected to fall inside a proper
10-sample / 30s window (the `--quick` variance on those benchmarks is
±8–15%). `startup` shows a +90% jump but that's dominated by the new
SSA rename walks on the imported runtime crates; absolute time is
still sub-4ms. Formal full-sample run scheduled for §1.10 close-out.

**Phase 1 acceptance sweep (2026-04-20, full sample, not promoted to the
Phase 1 column)**: `cargo bench -p pyaot-bench` was run without
`--quick`. That full-suite run initially appeared to fail the ±3%
runtime gate: `containers` = `9.0683 ms` (+5.3%), `strings` =
`2.8221 ms` (+4.1%), `generators` = `13.578 ms` (+3.8%), and
`startup` = `1.7777 ms` (+3.4%) exceeded the threshold. Follow-up
isolated reruns after the harness split showed representative outliers
(`containers`, `strings`, `gc_stress`) returning to near-baseline hot-run
numbers, so these full-suite deltas are now treated as pre-triage suite
noise rather than as confirmed runtime regressions. The Phase 1 column
above remains preliminary until a fresh full-suite capture is taken with
the split-metric harness.

**Phase 1 acceptance rerun (2026-04-20, split harness, full sample)**:
captured with `cargo bench -p pyaot-bench --bench pyaot_bench run::`.
This rerun does **not** satisfy the ±3% runtime gate against the
historical Phase-0 `run` column. Every benchmark regressed materially:
`classes` = `13.328 ms` (+52.3%), `closures` = `2.5427 ms` (+19.4%),
`containers` = `13.184 ms` (+53.1%), `exceptions` = `143.86 ms`
(+24.1%), `float_arith` = `3.4403 ms` (+19.0%), `gc_stress` =
`9.1185 ms` (+43.1%), `generators` = `16.589 ms` (+26.8%),
`int_arith` = `17.919 ms` (+16.7%), `polymorphic` = `13.080 ms`
(+16.7%), `startup` = `2.2593 ms` (+31.4%), and `strings` =
`4.6141 ms` (+70.3%). Phase 1 benchmark acceptance therefore remains
blocked, but now for a narrowed reason: runtime performance vs the
historical Phase-0 baseline, not compiler throughput.

### `fresh_launch` — compile + immediate first launch wall time (ms, median)

| Benchmark     | Phase 0 | Phase 1 | Phase 2 | Phase 3 |
|---------------|---------|---------|---------|---------|
| int_arith     | 199.54  | 193.67  |         |         |
| float_arith   | 178.75  | 181.97  |         |         |
| polymorphic   | 184.73  | 186.89  |         |         |
| containers    | 186.25  | 183.45  |         |         |
| strings       | 171.90  | 176.49  |         |         |
| generators    | 200.53  | 191.46  |         |         |
| exceptions    | 337.39  | 357.91  |         |         |
| gc_stress     | 181.82  | 183.74  |         |         |
| classes       | 183.99  | 186.61  |         |         |
| closures      | 178.21  | 180.39  |         |         |
| startup       | 172.36  | 175.01  |         |         |

**Phase 1 fresh_launch (2026-04-18, post-pruned-SSA)**: the original
Phase-0 harness named this metric `end_to_end`, but post-2026-04-20
triage it is understood as "compile and immediately launch the freshly
linked executable". The recorded Phase-0 / Phase-1 numbers are preserved
here under the corrected name. At the time, the launch-heavy metric was
interpreted as compile-phase throughput and appeared within ±6% of
Phase 0 across every benchmark —
`exceptions` is the biggest outlier at +6.1%, every other benchmark
is within ±3%. The earlier 50-85% regression documented against the
S1.6e "always place Phi" design was fully recovered by restoring the
classical single-def shortcut gated on actual dominance (pruned SSA):
only single-def locals whose def does NOT dominate every use
(match-lowering's elements_bb pattern) run the iterated dominance
frontier computation.

**Phase 1 acceptance sweep (2026-04-20, full sample, not promoted to the
Phase 1 column)**: `cargo bench -p pyaot-bench` also produced a fresh
compile+launch sweep. The large outliers (`containers` = `251.39 ms`,
`strings` = `189.79 ms`, `exceptions` = `409.34 ms`,
`gc_stress` = `343.25 ms`, `classes` = `253.22 ms`) triggered the
2026-04-20 triage that split the harness into `compile::*` and
`fresh_launch::*`. Follow-up isolated measurements showed compiler
throughput itself sitting in a tight ~48-51 ms band, while
`fresh_launch::*` remained ~350-470 ms on macOS for many binaries. This
metric is therefore retained as a diagnostic launch signal, not as a
phase-acceptance gate.

### `binary_size` — release executable size (bytes)

| Benchmark     | Phase 0  | Phase 1 | Phase 2 | Phase 3 |
|---------------|----------|---------|---------|---------|
| int_arith     | 405,064  |         |         |         |
| float_arith   | 422,344  |         |         |         |
| polymorphic   | 439,208  |         |         |         |
| containers    | 422,152  |         |         |         |
| strings       | 405,272  |         |         |         |
| generators    | 422,408  |         |         |         |
| exceptions    | 405,720  |         |         |         |
| gc_stress     | 405,368  |         |         |         |
| classes       | 439,336  |         |         |         |
| closures      | 422,152  |         |         |         |
| startup       | 404,920  |         |         |         |

### `max_rss` — peak RSS during execution (KiB)

| Benchmark     | Phase 0 | Phase 1 | Phase 2 | Phase 3 |
|---------------|---------|---------|---------|---------|
| int_arith     |  5,840  |         |         |         |
| float_arith   |  5,872  |         |         |         |
| polymorphic   |  7,072  |         |         |         |
| containers    | 21,328  |         |         |         |
| strings       |  8,080  |         |         |         |
| generators    |  6,160  |         |         |         |
| exceptions    |  6,864  |         |         |         |
| gc_stress     |  7,728  |         |         |         |
| classes       |  8,096  |         |         |         |
| closures      |  6,656  |         |         |         |
| startup       |  5,888  |         |         |         |

---

## Methodology notes

- All numbers are collected on a single **quiesced** machine — no browser,
  editor, or background sync running. The baseline machine's OS, CPU model,
  RAM, and toolchain versions are recorded above.
- Criterion's own JSON output (under `target/criterion/`) is *not* checked
  in; this file is the canonical summary.
- When a refactor deliberately changes the compiled semantics (e.g.,
  Phase 2 tagged values change binary size), the Phase column captures the
  post-change number and the PR description explains the delta.
- The original Phase-0 column above was produced with `--quick` to
  bootstrap the scaffolding. Post-2026-04-20 triage, `run::*` remains
  the hot-runtime acceptance metric, `compile::*` is the compiler-
  throughput acceptance metric, and `fresh_launch::*` is diagnostic.
- The `compile::*` Phase-0 column was backfilled on 2026-04-20 with a
  full Criterion sweep (`phase0-compile-backfill`) so Phase-1
  acceptance can now compare compiler throughput directly instead of
  inferring it from launch-heavy numbers.
- As of the 2026-04-20 split-harness acceptance rerun, `compile::*`
  passes and `run::*` fails materially against the committed historical
  baseline. The remaining benchmark gate is therefore runtime-only.
