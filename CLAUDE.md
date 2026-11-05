# CLAUDE.md

Guidance for Claude Code working on `pyaot-compiler`.

## What this is

Static AOT compiler for a **typed subset of Python 3** → native (Cranelift).
Read `ARCHITECTURE.md` (the design) and `PITFALLS.md` (the traps) first.

## The non-negotiable invariants

Every change answers to these (full rationale in `ARCHITECTURE.md`; the failure
modes they prevent are catalogued in `PITFALLS.md`).

1. **`SemTy` and `Repr` never merge.** Semantic type vs physical representation
   are two types in `crates/types`. `Repr` is mandatory (by value, never
   `Option`). Gradual unknown is `SemTy::Dyn` (repr always `Tagged`).
2. **Tagged is the always-correct default.** `Raw` / typed `Heap` are
   optimizations proven by `typeck`, never an unsafe default. So inference
   precision is a performance lever, not a correctness requirement.
3. **One calling convention, one coercion pass.** ABI derives from parameter
   `Repr`. All boxing/coercion goes through `lowering::legalize` — nowhere else.
   No per-function ABI flags, no separate ABI-repair stage.
4. **Inference is one constraint algorithm in `typeck`, finished before
   lowering.** Never a fixpoint of mutually recursive passes; never let inference
   leak into `lowering`.
5. **The MIR verifier runs in debug at every pass boundary.** GC-rootness is
   derived from `Repr`, never a stored flag. Optimizer passes read `Repr`, never
   inference-internal state.
6. **The `corpus/` is the spec.** New features must match CPython output on the
   corpus (differential test).

## Working discipline

- **The substrate is a stable base; the front-half is built fresh.** The
  substrate crates (`core-defs`, `runtime`, `stdlib-defs`, …) are a stable
  dependency — don't retype or casually rewrite them. The front-half crates
  (`types`, `hir`, `typeck`, `mir`, `lowering`, `optimizer`,
  `codegen-cranelift`, `frontend-python`, `cli`) are implemented from the
  design in this repo (ARCHITECTURE.md + each crate's `lib.rs` doc), not
  transcribed from any prior implementation. Reach for established algorithms
  (constraint solving, C3 MRO, standard optimizer passes) on their own merits.
- **Do not reintroduce the anti-patterns in `PITFALLS.md`.** They are why the
  invariants exist.
- **The runtime is a stable contract that evolves with the compiler.** Its
  `Value`-level ABI and `rt_*` signatures are the seam the whole compiler
  targets, so changes to it are deliberate, not casual. When compiler
  development requires a runtime change — fixing a runtime bug, a new
  `rt_*`/ABI, a layout extension (precedents: bignum, `StrObj.char_len`) —
  make it, document it as a contract change, and back it with corpus
  coverage. The one thing that stays forbidden is papering over a front-half
  bug in the runtime instead of fixing the front-half.
- `#![forbid(unsafe_code)]` in every compiler crate; only `runtime` uses unsafe.
- **Keep `COMPILER_STATUS.md` current.** It is the Python-feature coverage map
  (statements, types, operators, builtins, stdlib, …) keyed to the `corpus/`
  differential gate. Whenever a change adds, completes, or drops support for a
  Python feature — a new builtin, stdlib surface, syntax form, type, or a
  feature moving ❌→🟡→✅ — update the relevant row(s) in the same commit. A
  feature only earns ✅ once it is in `PHASE_CORPUS` and byte-exact vs CPython.
- After any change: `cargo check --workspace --exclude pyaot-runtime`, and
  `cargo build -p pyaot-runtime` if the runtime was touched.

## Build / test

```bash
cargo check --workspace --exclude pyaot-runtime   # fast front-half check
cargo build -p pyaot-runtime                       # runtime staticlib
cargo build --workspace                            # full
```

## Running examples quickly

`--run` compiles and immediately executes, propagating the exit code; `-o` is
optional (defaults to the input stem). Build the runtime staticlib once first
(`cargo build -p pyaot-runtime`) so the linker finds it.

```bash
cargo run -p pyaot-cli -- corpus/microgpt.py --run   # compile + run in one step
# or with the built binary:
target/debug/pyaot corpus/microgpt.py --run -v       # -v prints each stage
```
