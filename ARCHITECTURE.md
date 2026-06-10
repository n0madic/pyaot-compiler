# Architecture

`pyaot-compiler` is a **static AOT compiler for a typed subset of Python 3** →
native executables via Cranelift.

Target: compile real, idiomatic Python scripts (e.g. `microgpt.py`) with no or
minimal changes *within standard Python syntax*. Consciously **out of scope**
(too dynamic for AOT): `eval`/`exec`/`compile`, metaclasses, `__dict__` mutation,
dynamic `getattr(obj, name_var)`, `globals()`/`locals()`, `inspect`,
`import *`, runtime class creation. `int` is **arbitrary precision** (bignum).

## The seam: a frozen runtime contract

The compiler's type/IR layer is entirely *upstream* of the runtime. The runtime
only ever sees `Value` and `rt_*` calls — it is agnostic to the compiler's type
system. So the architectural seam is the **runtime's `Value`-level ABI**: it is
frozen as a contract, and everything above it is built to target it.

The freeze is a default discipline, not an absolute prohibition: it holds only
as long as it serves the plan. When fully realizing a planned feature genuinely
requires a runtime change — as bignum did — the plan wins, and the seam is
extended deliberately (a new `rt_*`/ABI, documented as such, with corpus
coverage). The freeze exists to keep front-half bugs from being papered over in
the runtime, not to block the plan.

| Layer | Crates | Status |
|---|---|---|
| **Frozen substrate + contract** | `core-defs`, `format-shared`, `utils`, `diagnostics`, `linker`, `stdlib-defs`, `runtime` | sealed; treated as a fixed dependency |
| **Compiler front-half** | `types`, `hir`, `semantics`, `typeck`, `mir`, `lowering`, `optimizer`, `codegen-cranelift`, `frontend-python`, `cli` | built fresh from this design |

## The six invariants (the constitution)

Every design choice answers to these. Each names the anti-pattern it exists to
prevent.

1. **Two separate type layers; representation is mandatory.**
   [`SemTy`](crates/types/src/sem.rs) (semantic / Python-level) and
   [`Repr`](crates/types/src/repr.rs) (physical) never merge. `Repr` is a field
   *by value*, never `Option`. Gradual "dynamic" is the explicit `SemTy::Dyn`
   whose `Repr` is always `Tagged`. *Anti-pattern prevented:* a single type that
   conflates "what Python type" with "how it's stored", forcing a
   representation-ambiguous `Any`/`HeapAny` distinction and a dual
   logical/physical field with an optional, dual-meaning sentinel.

2. **One uniform tagged substrate; specialization is opt-in.**
   Correctness is always reachable with `Repr::Tagged`. Unboxed `Raw` / typed
   `Heap` are *optimizations* `typeck` proves safe — never a default that can
   corrupt memory. There is no "representation cliff".

3. **One calling convention; one coercion-insertion pass.**
   A function's ABI is a deterministic function of its parameters' `Repr`. All
   box/unbox/tag/coerce ops are inserted by
   [`lowering::legalize`](crates/lowering/src/lib.rs) and nowhere else.
   *Anti-pattern prevented:* per-function ABI flags, argument-marker bits, and a
   separate after-the-fact ABI-repair stage.

4. **One constraint-based inference, finished before lowering.**
   [`typeck`](crates/typeck/src/lib.rs) is collect → solve → materialize, one
   algorithm. *Anti-pattern prevented:* a fixpoint of mutually-dependent monotone
   inference passes whose ordering and iteration depth are tuned empirically.

5. **Typed IR + verifier from commit #1.**
   [`mir::verify`](crates/mir/src/lib.rs) runs in debug at *every* pass boundary.
   GC-rootness is `Repr::is_gc_root()`, derived — never a stored, drift-prone flag.

6. **CPython behavior is an executable spec.**
   The [`corpus/`](corpus) `.py` files run under both CPython and `pyaot`; output
   is diffed automatically. Compatibility knowledge lives in tests, not in notes.

## Why precision is decoupled from correctness

Because `repr_of` gives a *safe* representation for every `SemTy` and
`Repr::Tagged` is always correct, a weak `typeck` produces *slower* code, never
*wrong* code. This is the central enabler: a working (slow) compiler can ship on
minimal inference, and inference precision can grow afterwards with no
correctness risk. (The failure mode this avoids: a design where `type ==
representation`, so any inference imprecision becomes a miscompile.)

## Pipeline

```
source ─▶ frontend-python ─▶ HIR ─▶ semantics ─▶ typeck ─▶ lowering (+legalize)
       ─▶ MIR (verify) ─▶ optimizer (verify) ─▶ codegen-cranelift ─▶ linker ─▶ exe
```

## Reaching real scripts (`microgpt.py`)

A principled `SemTy` + constraint solver handles cross-instance class-field
inference (e.g. autograd `child.grad += …`) and polymorphic dunders as ordinary
unification, on top of an always-correct tagged baseline. Real scripts
additionally need bignum (invariant scope) and broad stdlib coverage (the frozen
`stdlib-defs` + `runtime`).

## Status

Skeleton. Substrate crates build; front-half crates are typed scaffolds. The
`types` crate (`SemTy` / `Repr` / `repr_of` / lattice) is implemented — it is the
load-bearing architectural artifact. See each crate's `lib.rs` for its
responsibility, and [PLAN.md](PLAN.md) for the phased roadmap.
