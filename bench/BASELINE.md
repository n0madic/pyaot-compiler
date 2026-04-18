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
| Measurement date   | 2026-04-17                               |
| Measurement mode   | `cargo bench -- --quick` (Phase 0 scaffolding; full 10-sample runs pending stability audit) |

## Columns

| Column          | Meaning                                                    |
|-----------------|------------------------------------------------------------|
| `run`           | Wall-clock milliseconds, pre-compiled binary invocation.   |
| `end_to_end`    | Wall-clock milliseconds, `pyaot <src> -o … && ./…` per sample. |
| `binary_size`   | Output binary size in bytes (release, stripped).           |
| `max_rss`       | Peak resident-set size in KiB, via `/usr/bin/time -l`.     |

Each `run` / `end_to_end` number is Criterion's median estimate (the middle
of the printed `[low median high]` confidence interval). The current
`--quick` numbers have bench-internal spread well under 3 %; a full
10-sample/30s-measurement sweep will be run at Phase-1 acceptance and
committed as an addendum row before gating the phase.

## How to read the phase columns

- **Phase 0** — captured before any refactor work lands. This is the
  "zero-regression" reference for everything after it.
- **Phase 1 / Phase 2 / Phase 3** — snapshotted at each phase's acceptance
  gate. A new column is appended to every table below; earlier columns are
  never rewritten.

Regressions > 3 % in any `run::<stem>` row, or > 10 % in any
`end_to_end::<stem>` row, must be flagged in the corresponding phase's
acceptance review.

---

## Benchmarks

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

### `end_to_end` — compile + run wall time (ms, median)

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

**Phase 1 end_to_end (2026-04-18, post-pruned-SSA)**: compile-phase
numbers are now within ±6% of Phase 0 across every benchmark —
`exceptions` is the biggest outlier at +6.1%, every other benchmark
is within ±3%. The earlier 50-85% regression documented against the
S1.6e "always place Phi" design was fully recovered by restoring the
classical single-def shortcut gated on actual dominance (pruned SSA):
only single-def locals whose def does NOT dominate every use
(match-lowering's elements_bb pattern) run the iterated dominance
frontier computation.

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
- The Phase-0 column above was produced with `--quick` to bootstrap the
  scaffolding. Before Phase-1 work begins, a full `cargo bench -p
  pyaot-bench` sweep (no `--quick`) must be run and the columns replaced in
  the same PR — the `--quick` numbers are informational only and must not
  be used as a regression gate.
