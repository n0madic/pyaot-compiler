# pyaot benchmark harness

Phase-0 regression-detection infrastructure. Every benchmark here feeds into
`BASELINE.md`, which each subsequent refactor phase measures against.

## Layout

```
bench/
├── Cargo.toml              # cargo crate definition
├── BASELINE.md             # committed reference numbers (one column / phase)
├── README.md               # this file
├── benches/
│   └── pyaot_bench.rs      # Criterion harness — drives every *.py source
└── py/
    ├── int_arith.py        # integer hot loop
    ├── float_arith.py      # float hot loop
    ├── polymorphic.py      # Value-class dunder dispatch (microgpt-style)
    ├── containers.py       # list + dict alloc/iterate/mutate
    ├── strings.py          # intern-hit + concat
    ├── generators.py       # gen-expr + enumerate fusion + nested comp
    ├── exceptions.py       # non-raising + raising try/except
    ├── gc_stress.py        # allocation-heavy; triggers mark-sweep
    ├── classes.py          # monomorphic + polymorphic method dispatch
    ├── closures.py         # closure capture + reduce
    └── startup.py          # smoke test; measures launch overhead
```

## Running

### Prerequisites

The harness invokes the release pyaot binary directly — it does **not**
drive cargo. Build the toolchain up front:

```bash
cargo build --workspace --release
```

This produces `target/release/pyaot` and the runtime static library that
the linker needs.

### Full suite

```bash
cargo bench -p pyaot-bench
```

For each `bench/py/<name>.py` source, Criterion emits three groups:

- `compile::<name>`      — compiler + linker throughput only.
- `run::<name>`          — pre-compiled binary invocation only.
- `fresh_launch::<name>` — compile + immediate first launch of the
  freshly linked binary.

Results land under `target/criterion/`. Human-readable summaries are
maintained in `BASELINE.md`; the raw Criterion JSON is **not** committed.

### Compare against the committed baseline

Criterion's `--save-baseline` / `--baseline` flags drive comparisons:

```bash
# After a refactor, capture a named baseline
cargo bench -p pyaot-bench -- --save-baseline phase-1

# Later, compare against it
cargo bench -p pyaot-bench -- --baseline phase-1
```

The second invocation prints per-benchmark deltas.

- `run::*` is the hot-runtime acceptance metric. Regressions > 3 % must
  be flagged in the associated phase-acceptance PR.
- `compile::*` is the compiler-throughput acceptance metric. Regressions
  > 10 % must be flagged in the associated phase-acceptance PR.
- `fresh_launch::*` is **diagnostic only**. On macOS it is dominated by
  path-sensitive first-launch cost for a freshly linked executable, so it
  is useful for spotting launch regressions but should not block phase
  acceptance on its own.

### Binary size & max RSS

The Criterion harness only records wall-clock time. Binary size and peak RSS
are captured out-of-band during baseline runs — they're cheap enough to do
by hand:

```bash
# Binary size (macOS / Linux)
stat -f%z /tmp/pyaot-bench/int_arith      # macOS
stat -c%s /tmp/pyaot-bench/int_arith      # Linux

# Peak RSS
/usr/bin/time -l /tmp/pyaot-bench/int_arith   # macOS — "maximum resident set size"
/usr/bin/time -v /tmp/pyaot-bench/int_arith   # Linux — "Maximum resident set size (kbytes)"
```

The harness leaves the compiled binaries in `$TMPDIR/pyaot-bench/` after
a run, so these commands can be scripted on top of a fresh
`cargo bench -p pyaot-bench`.

## Stability

Each Criterion group uses `sample_size(10)` and `measurement_time(15–30s)`.
`run::*` and `compile::*` should stay low-noise on a quiesced machine.
`fresh_launch::*` is intentionally more variable because it captures
platform launch effects on a newly linked executable; compare it as a
trend line, not as a hard gate.

If a `run::*` or `compile::*` benchmark consistently reports > 3 % spread,
profile it in isolation before committing a new baseline column — a noisy
baseline erodes the regression-detection signal for every downstream phase.

## Adding a benchmark

1. Drop a new `*.py` file under `bench/py/`.
2. Make it type-check and run with the current pyaot; exit 0 and print a
   short summary line (see existing files for the pattern).
3. Rebuild (`cargo build --workspace --release`) and rerun `cargo bench
   -p pyaot-bench` — the harness auto-discovers new files.
4. Add the row to every table in `BASELINE.md` with a `TBD` placeholder,
   then run the baseline recipe to fill in the Phase-0 column.
