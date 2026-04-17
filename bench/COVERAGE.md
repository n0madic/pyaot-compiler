# Phase-0 Coverage Audit

Baseline coverage report for the pyaot workspace, per
`ARCHITECTURE_REFACTOR.md` § 0.2. Re-run with:

```bash
cargo llvm-cov --workspace --html
```

Numbers below are from `cargo llvm-cov --workspace --tests --summary-only`
captured **2026-04-17** on the baseline machine documented in
`bench/BASELINE.md`.

## Workspace totals

| Metric   | Coverage |
|----------|----------|
| Region   | 57.96 %  |
| Function | 54.73 %  |
| Line     | 57.61 %  |

The sub-70 % totals are driven entirely by the `runtime` crate, which is a
**staticlib** linked into Python executables that are invoked as subprocesses
by integration tests. Those subprocesses are not profile-instrumented by
`cargo llvm-cov`, so the runtime reports 0 % for any module exercised only
at runtime. This is an instrumentation artifact, not a testing gap —
runtime modules ship with their own `#[cfg(test)]` units *and* the
examples/ integration suite exercises every runtime path via compiled
executables. Phase-0 non-negotiable below only gates on compiler-side
crates.

## Phase-1 gate — compiler-side modules below 70 %

The spec says Phase 1 must not be bottlenecked on missing tests. Every
module in this table must reach ≥ 70 % region **and** line coverage before
Phase 1 begins, or be explicitly marked "TODO-blocked" with a linked issue.

### `lowering/` — real gaps

| File                                                  | Region | Line   | Status |
|-------------------------------------------------------|--------|--------|--------|
| `expressions/access/method/tuple.rs`                  |  0.00% |  0.00% | TODO — `tuple.index()`/`tuple.count()` blocked by `rt_obj_eq` raw-int handling (noted in `examples/test_collections_dict_set_bytes.py:420`). Unblock + add tests. |
| `expressions/builtins/print.rs`                       | 52.21% | 50.51% | Missing tests for type-tagged print paths beyond int/str. |
| `expressions/builtins/file.rs`                        | 63.55% | 65.00% | Missing tests for `open()` mode edge cases. |
| `expressions/builtins/iteration/composite.rs`         | 64.86% | 63.77% | Missing tests for fused-iteration corner cases. |
| `expressions/calls/args.rs`                           | 34.19% | 42.68% | Missing tests for varargs/kwargs expansion edge cases. |
| `expressions/mod.rs`                                  | 68.58% | 60.71% | Dispatcher-level gaps; typically filled by extending existing test families. |
| `generators/utils.rs`                                 | 28.30% | 41.24% | Needs coverage of heap-capture emission paths. |
| `statements/loops/enumerate.rs`                       | 63.23% | 67.35% | Missing tests for enumerate start/step variants. |
| `statements/loops/range.rs`                           | 71.14% | 64.77% | Line coverage dip on negative-step & zero-step range branches. |
| `statements/loops/starred_unpacking.rs`               | 58.60% | 60.74% | Missing tests for nested starred patterns. |
| `type_planning/lambda_inference.rs`                   | 65.05% | 64.71% | Missing tests for lambda default-value inference. |
| `type_planning/ni_analysis.rs`                        | 39.30% | 44.19% | Missing tests for nested-instance analysis corner cases. |
| `type_planning/validate.rs`                           | 78.57% | 59.02% | Line coverage dip on rarely-triggered validation branches. |
| `utils.rs`                                            | 70.67% | 66.07% | Line-coverage gap on helper utilities. |

### `optimizer/` — real gaps

| File                          | Region | Line   | Status |
|-------------------------------|--------|--------|--------|
| `constfold/mod.rs`            | 62.18% | 56.03% | Missing tests for propagation fix-point on complex chains. |
| `constfold/propagate.rs`      | 56.68% | 53.97% | Missing tests for per-instruction propagation arms. |
| `dce/mod.rs`                  | 46.72% | 49.38% | Missing tests for exception-instruction liveness. |
| `inline/mod.rs`               | 66.67% | 77.42% | Region-only dip (line coverage already ≥ 70 %). |
| `inline/remap.rs`             | 33.53% | 36.21% | Missing tests for terminator/operand remapping arms. |

### `types/` — real gaps

| File                 | Region | Line   | Status |
|----------------------|--------|--------|--------|
| `types/src/lib.rs`   | 63.73% | 69.09% | Missing tests for rare `Type` variants; line coverage borderline. |

## Modules above 70 % (no action required)

Every compiler-side module not listed above already meets the Phase-0 gate.
See the full table in `target/llvm-cov/html/` after running the command
above.

## Runtime crate

`runtime/*` is intentionally omitted from the gate — its coverage must be
measured via the compiled-binary integration suite (`cargo test -p pyaot
--test runtime`), not `cargo llvm-cov`. A follow-up change will instrument
the compiled test binaries with `LLVM_PROFILE_FILE` so runtime coverage
shows up in the report; that work is tracked separately and is not a
Phase-1 blocker.

## Action for Phase-1 kickoff

Before the first Phase-1 PR lands, every row in the "real gaps" tables
above must be either:

1. Raised to ≥ 70 % region **and** line coverage via new tests in
   `examples/test_*.py` or `crates/*/src/tests.rs`, or
2. Explicitly marked "TODO-blocked: <reason>" with a tracking issue, if a
   known bug prevents tests from being added (e.g., `tuple.rs`).

Re-run `cargo llvm-cov --workspace --html` and update this file in the
same PR that raises the coverage.
