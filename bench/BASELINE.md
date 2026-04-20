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
| Measurement date   | Phase 0: 2026-04-17 (`run` / `fresh_launch`), 2026-04-20 (`compile` backfill); Phase 1 acceptance: 2026-04-20 |
| Measurement mode   | Phase 0 `run` / `fresh_launch`: `cargo bench -- --quick`; Phase 0 `compile`: `cargo bench -p pyaot-bench --bench pyaot_bench compile:: -- --save-baseline phase0-compile-backfill`; Phase 1 acceptance: full-sample `cargo bench -p pyaot-bench --bench pyaot_bench {compile,run,fresh_launch}::` |

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
- If a metric's collection semantics change materially, the newly accepted
  phase column becomes the forward reference for subsequent phases. This
  happened in Phase 1 when the old `end_to_end` signal was split into
  `compile::*` and `fresh_launch::*`.
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
| int_arith     | 46.541  | 48.439  |         |         | compiler + linker only |
| float_arith   | 46.662  | 49.027  |         |         | compiler + linker only |
| polymorphic   | 48.636  | 49.394  |         |         | compiler + linker only |
| containers    | 45.795  | 47.532  |         |         | compiler + linker only |
| strings       | 48.175  | 48.579  |         |         | compiler + linker only |
| generators    | 48.243  | 49.478  |         |         | compiler + linker only |
| exceptions    | 47.415  | 46.983  |         |         | compiler + linker only |
| gc_stress     | 47.454  | 48.698  |         |         | compiler + linker only |
| classes       | 47.329  | 45.889  |         |         | compiler + linker only |
| closures      | 46.847  | 47.514  |         |         | compiler + linker only |
| startup       | 48.352  | 47.861  |         |         | compiler + linker only |

**Phase 0 backfill (2026-04-20, full Criterion sample)**:
captured with
`cargo bench -p pyaot-bench --bench pyaot_bench compile:: -- --save-baseline phase0-compile-backfill`.
This replaced the earlier manual triage snapshot and is now the committed
reference for `compile::*`.

**Phase 1 accepted snapshot (2026-04-20, split harness, full sample)**:
captured with `cargo bench -p pyaot-bench --bench pyaot_bench compile::`.
The Phase 1 column above records the accepted full-sample medians on the
formal-close `HEAD`. Compiler-throughput acceptance passes against the
backfilled Phase-0 baseline: all 11 `compile::*` groups remained inside
the ±10% gate.

### `run` — pre-compiled execution wall time (ms, median)

| Benchmark     | Phase 0 | Phase 1 | Phase 2 | Phase 3 | Notes |
|---------------|---------|---------|---------|---------|-------|
| int_arith     |  15.35  |  18.29  |         |         | `for i in range(10_000_000): total += i` |
| float_arith   |   2.89  |   3.58  |         |         | `total += float(i) * 0.5` over 1M |
| polymorphic   |  11.21  |  13.26  |         |         | Value class `__add__`/`__mul__`, 200k iters |
| containers    |   8.61  |  12.32  |         |         | list append/index/mutate + dict insert/lookup/iter, N=100k |
| strings       |   2.71  |   3.93  |         |         | intern hit-path + 2k concat |
| generators    |  13.08  |  16.49  |         |         | gen-expr `sum`, enumerate fusion, nested comp |
| exceptions    | 115.95  | 142.52  |         |         | 500k try/except non-raising + 500k with 4 raises |
| gc_stress     |   6.37  |   8.93  |         |         | 200 chains × 1000 Node instances |
| classes       |   8.75  |  12.61  |         |         | Point.norm + polymorphic Shape.area, 200k each |
| closures      |   2.13  |   2.68  |         |         | closure capture + comprehension reduce |
| startup       |   1.72  |   2.13  |         |         | single `print`, measures binary launch |

**Phase 1 preliminary (2026-04-18, post-pruned-SSA)**: captured with
`cargo bench -p pyaot-bench --bench pyaot_bench -- --quick` after
S1.17-pruned-SSA landed. Criterion's t-test reports "No change in
performance detected" for all benchmarks. Raw median deltas are
within ±10% of Phase 0, with `generators` (+12.5%) and `exceptions`
(+6.3%) the largest outliers — both expected to fall inside a proper
10-sample / 30s window (the `--quick` variance on those benchmarks is
±8–15%). `startup` shows a +90% jump but that's dominated by the new
SSA rename walks on the imported runtime crates; absolute time is
still sub-4ms. This note is retained for historical context; the
accepted Phase 1 column above was later replaced by the 2026-04-20
split-harness full-sample snapshot.

**Phase 1 acceptance sweep (2026-04-20, full sample, not promoted to the
Phase 1 column)**: `cargo bench -p pyaot-bench` was run without
`--quick`. That full-suite run initially appeared to fail the ±3%
runtime gate: `containers` = `9.0683 ms` (+5.3%), `strings` =
`2.8221 ms` (+4.1%), `generators` = `13.578 ms` (+3.8%), and
`startup` = `1.7777 ms` (+3.4%) exceeded the threshold. Follow-up
isolated reruns after the harness split showed representative outliers
(`containers`, `strings`, `gc_stress`) returning to near-baseline hot-run
numbers, so these full-suite deltas are now treated as pre-triage suite
noise rather than as confirmed runtime regressions. This note remains as
historical triage context; the Phase 1 column above now records the
accepted 2026-04-20 split-harness full-sample snapshot.

**Phase 1 accepted snapshot (2026-04-20, split harness, full sample)**:
captured with `cargo bench -p pyaot-bench --bench pyaot_bench run::`.
The Phase 1 column above is the accepted runtime reference for future
phases. The same full-sample rerun still sat materially above the
historical Phase-0 quick-captured `run` column, but on 2026-04-20 that
gap was explicitly accepted as a re-baselining event rather than left as
a Phase 1 blocker. Future runtime comparisons should therefore use the
accepted Phase 1 column above unless the harness changes again.

### `fresh_launch` — compile + immediate first launch wall time (ms, median)

| Benchmark     | Phase 0 | Phase 1 | Phase 2 | Phase 3 |
|---------------|---------|---------|---------|---------|
| int_arith     | 199.54  | 357.30  |         |         |
| float_arith   | 178.75  | 330.51  |         |         |
| polymorphic   | 184.73  | 367.28  |         |         |
| containers    | 186.25  | 327.61  |         |         |
| strings       | 171.90  | 349.45  |         |         |
| generators    | 200.53  | 374.12  |         |         |
| exceptions    | 337.39  | 441.22  |         |         |
| gc_stress     | 181.82  | 350.60  |         |         |
| classes       | 183.99  | 335.09  |         |         |
| closures      | 178.21  | 323.67  |         |         |
| startup       | 172.36  | 365.12  |         |         |

**Phase 1 fresh_launch (2026-04-18, post-pruned-SSA)**: the original
Phase-0 harness named this metric `end_to_end`, but post-2026-04-20
triage it is understood as "compile and immediately launch the freshly
linked executable". The original Phase-0 number is preserved here under
the corrected name, while the early 2026-04-18 quick Phase-1 snapshot
is retained only in this note for historical context. At the time, the
launch-heavy metric was interpreted as compile-phase throughput and
appeared within ±6% of
Phase 0 across every benchmark —
`exceptions` is the biggest outlier at +6.1%, every other benchmark
is within ±3%. The earlier 50-85% regression documented against the
S1.6e "always place Phi" design was fully recovered by restoring the
classical single-def shortcut gated on actual dominance (pruned SSA):
only single-def locals whose def does NOT dominate every use
(match-lowering's elements_bb pattern) run the iterated dominance
frontier computation.

**Phase 1 accepted snapshot (2026-04-20, split harness, full sample)**:
captured with
`cargo bench -p pyaot-bench --bench pyaot_bench fresh_launch::`.
The Phase 1 column above records the accepted diagnostic launch snapshot
after the harness split. This metric remains non-blocking and is kept as
an informational trend line for macOS first-launch cost, not as a phase
acceptance gate.

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
- As of formal Phase 1 close on 2026-04-20, the accepted split-harness
  full-sample snapshot is recorded in the Phase 1 columns above.
  `compile::*` passed against the backfilled Phase-0 baseline; `run::*`
  and `fresh_launch::*` now use the accepted Phase-1 column as the
  forward reference for later phases unless the harness changes again.
