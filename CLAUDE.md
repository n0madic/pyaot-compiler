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

- **The substrate is frozen; the front-half is built fresh.** The substrate
  crates (`core-defs`, `runtime`, `stdlib-defs`, …) are a sealed dependency —
  don't retype or casually edit them. The front-half crates (`types`, `hir`,
  `typeck`, `mir`, `lowering`, `optimizer`, `codegen-cranelift`,
  `frontend-python`, `cli`) are implemented from the design in this repo
  (ARCHITECTURE.md + each crate's `lib.rs` doc), not transcribed from any prior
  implementation. Reach for established algorithms (constraint solving, C3 MRO,
  standard optimizer passes) on their own merits.
- **Do not reintroduce the anti-patterns in `PITFALLS.md`.** They are why the
  invariants exist.
- **The runtime is a frozen contract.** Its `Value`-level ABI and `rt_*`
  signatures are the seam the whole compiler targets. Extend it only with a
  deliberate, documented reason (bignum is the one planned extension) — never to
  paper over a front-half bug.
- `#![forbid(unsafe_code)]` in every compiler crate; only `runtime` uses unsafe.
- After any change: `cargo check --workspace --exclude pyaot-runtime`, and
  `cargo build -p pyaot-runtime` if the runtime was touched.

## Build / test

```bash
cargo check --workspace --exclude pyaot-runtime   # fast front-half check
cargo build -p pyaot-runtime                       # frozen runtime staticlib
cargo build --workspace                            # full
```

## Crate map

| Crate | Role | State |
|---|---|---|
| core-defs, format-shared, utils, diagnostics, linker, stdlib-defs, runtime | frozen substrate + contract | sealed |
| types | `SemTy` + `Repr` + `repr_of` + lattice | **implemented** |
| hir, semantics, typeck, mir, lowering, optimizer, codegen-cranelift, frontend-python, cli | front-half | scaffolds |
