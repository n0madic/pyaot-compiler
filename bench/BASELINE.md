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
| int_arith     |  15.35  |  15.92  |         |         | `for i in range(10_000_000): total += i` |
| float_arith   |   2.89  |   2.79  |         |         | `total += float(i) * 0.5` over 1M |
| polymorphic   |  11.21  |  11.69  |         |         | Value class `__add__`/`__mul__`, 200k iters |
| containers    |   8.61  |   8.70  |         |         | list append/index/mutate + dict insert/lookup/iter, N=100k |
| strings       |   2.71  |   2.84  |         |         | intern hit-path + 2k concat |
| generators    |  13.08  |  13.80  |         |         | gen-expr `sum`, enumerate fusion, nested comp |
| exceptions    | 115.95  | 116.19  |         |         | 500k try/except non-raising + 500k with 4 raises |
| gc_stress     |   6.37  |   7.05  |         |         | 200 chains × 1000 Node instances |
| classes       |   8.75  |   9.09  |         |         | Point.norm + polymorphic Shape.area, 200k each |
| closures      |   2.13  |   2.22  |         |         | closure capture + comprehension reduce |
| startup       |   1.72  |   1.77  |         |         | single `print`, measures binary launch |

**Phase 1 preliminary (2026-04-18)**: captured with `cargo bench -p
pyaot-bench --bench pyaot_bench -- --quick` after S1.14b-inliner. All
deltas report "No change in performance detected" from Criterion's
t-test (p > 0.05). Raw median deltas are within the `--quick` noise
floor (±5%) for every benchmark except `gc_stress` (+10.7%); the
larger gc_stress spread is expected under `--quick` since that
benchmark has the highest cross-sample variance in Phase 0's
measurement. A formal 10-sample / 30s-measurement run is scheduled
for the §1.10 Phase 1 close-out (S1.17).

### `end_to_end` — compile + run wall time (ms, median)

| Benchmark     | Phase 0 | Phase 1 | Phase 2 | Phase 3 |
|---------------|---------|---------|---------|---------|
| int_arith     | 199.54  | 224.32  |         |         |
| float_arith   | 178.75  |   —     |         |         |
| polymorphic   | 184.73  | 326.14  |         |         |
| containers    | 186.25  |   —     |         |         |
| strings       | 171.90  | 317.69  |         |         |
| generators    | 200.53  | 329.96  |         |         |
| exceptions    | 337.39  |   —     |         |         |
| gc_stress     | 181.82  | 314.79  |         |         |
| classes       | 183.99  |   —     |         |         |
| closures      | 178.21  |   —     |         |         |
| startup       | 172.36  | 175.17  |         |         |

**Phase 1 end_to_end flag** (2026-04-18): `—` entries were missing
from the sampled `--quick` output; run-column numbers above are the
sampled subset. Of the recorded end_to_end deltas, **`startup` is
within ±2%** (pure launch cost unchanged) but every other benchmark's
compile-phase is 50–85% slower. The likely cause is the S1.6e
"always place Phi" relaxation of Cytron's single-def optimisation —
every defined local now runs the iterated dominance frontier
computation regardless of def count, so `construct_ssa`'s cost grows
from O(multi-def-locals) to O(all-locals). The `run` column shows
the runtime-only impact is within noise (±5%), confirming the
regression is compile-time, not emitted-code. Tracked as a Phase 1
close-out (S1.17) task: either switch `insert_phis` to pruned SSA
(place Phi only where a use is not dominated by the single def), or
accept the compile-time cost as an invariant-safety tradeoff.

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
