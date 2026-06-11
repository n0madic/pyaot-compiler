# Implementation Plan — to a working compiler

Strategic plan from the current skeleton to a working state. Deliberately
high-level: the *ideas* to realize, the *order* to realize them in, and the
principles baked into both. Mechanical detail lives in each crate's `lib.rs` and
is decided when that crate is built. The failure modes these choices avoid are
catalogued in **[PITFALLS.md](PITFALLS.md)**.

## Definition of "working"

A `pyaot` binary that:

- compiles the static-Python `corpus/` and runs **`microgpt.py`** (the north-star
  real script) unchanged or with only standard-syntax tweaks;
- produces output **identical to CPython** on every corpus file (differential gate);
- has **arbitrary-precision `int`**;
- links against the frozen `runtime` and produces a native executable;
- reaches competitive performance *after* the optimization phase — not before
  (correctness never waits on the optimizer; see Principle 2).

Out of scope (too dynamic for AOT): `eval`/`exec`/`compile`, metaclasses,
`__dict__` mutation, dynamic `getattr(obj, name_var)`, `globals()`/`locals()`,
`inspect`, `import *`, runtime class creation.

---

## Guiding principles

These are *why* the phases are ordered the way they are.

1. **Vertical slice before breadth.** Get the thinnest program through the
   *entire* pipeline first, then widen feature-by-feature. Building each layer
   deeply in isolation strands you in inference work before a stable end-to-end
   contract exists.

2. **★ Correctness lives on the Tagged baseline; inference precision is a
   performance lever, not a correctness requirement. ★** The central enabler.
   `repr_of` gives a *safe* representation for every `SemTy`, and `Repr::Tagged`
   is always correct, so a weak `typeck` yields *slower* code, never *wrong* code.
   A working (slow) compiler can ship on minimal inference and improve afterwards
   with no correctness risk. (See PITFALLS A2 for the failure mode this inverts.)

3. **Safe representation is free; only two narrowings carry a proof obligation.**
   `repr_of` already maps `Float→Raw(F64)`, `Bool→Raw(I8)`, `Str→Heap`,
   containers/classes→`Heap`. The *only* representation optimizations needing a
   proof: (a) `int`: `Tagged` (fixnum-or-bignum) → `Raw(I64)` under a
   range/no-overflow proof; (b) narrowing `Dyn`/`Union` → a concrete type.

4. **One coercion pass, one verifier.** All box/unbox/tag/untag/numeric-widen
   coercions are inserted by `lowering::legalize` and nowhere else. The MIR
   verifier runs in debug at *every* pass boundary and rejects representation
   mismatch.

5. **Inference is one algorithm: collect → solve → materialize.** A single
   constraint pass over a lattice with union-find. Never a fixpoint of mutually
   recursive monotone passes (PITFALLS A3).

6. **ABI = f(Repr), deterministic.** No per-function ABI flags, no marker bits,
   no ABI-repair stage. HOF callbacks are uniform `fn(Value)→Value` at baseline;
   unboxed call paths are a proven optimization, not an escape hatch (PITFALLS A4).

7. **Build the front-half fresh from the design; the substrate is frozen.** Front-
   half crates are implemented from ARCHITECTURE.md + each crate's `lib.rs`, using
   established algorithms (constraint solving, C3 MRO, standard optimizer passes)
   on their own merits. The substrate (`runtime`, `core-defs`, …) is a sealed
   dependency.

8. **The runtime is a frozen contract — frozen only while the freeze serves this
   plan.** Its `Value` ABI + `rt_*` signatures are the seam, so it is frozen by
   default. The freeze is a discipline, not an absolute prohibition: when fully
   realizing a planned feature genuinely requires a runtime change — as bignum
   did — the plan wins and the runtime is extended deliberately (new `rt_*`/ABI,
   documented, with corpus coverage). Bignum is the precedent, not the only ever
   permitted extension. What stays forbidden is editing the runtime to paper over
   a front-half bug or to dodge front-half work.

9. **Differential testing is the spine.** The corpus-vs-CPython harness is built
   in Phase 1 and gates every subsequent feature.

### Anti-patterns to avoid (do not reintroduce)

A fixpoint-of-passes inference • per-function ABI flags / argument-marker bits /
a separate ABI-repair stage • a dual logical/physical type field with an optional,
dual-meaning sentinel • GC per-slot side-table tag masks • a 61-bit tagged int •
any representation-ambiguous "could be raw or pointer" type. Full catalogue with
rationale in **[PITFALLS.md](PITFALLS.md)** — Part A.

---

## Phases

Each phase ends at a green differential gate over a growing corpus subset. A
phase may ship code that is correct-but-unoptimized (Principle 2).

### Phase 1 — Tracer bullet (pipeline + seam)
**Goal:** `print("hello")` → native exe, output matches CPython.
**Build:** every seam on the trivial case before any breadth — the HIR shape, a
stub `typeck` (annotations + obvious literals; everything else `Dyn`), `lowering`
+ `legalize`, the `mir` verifier, `codegen` → Cranelift, the `linker` call
against the frozen runtime, and the **differential harness** (compile each
`corpus/*.py`, run, diff vs `python3`).
**Gate:** one trivial program compiles, runs, diffs clean.

### Phase 2 — Scalars, control flow, functions
**Goal:** integer/float/bool/None programs with `if`/`while`/`for range`,
annotated functions, recursion. **Bignum lands here** (fixnum fast-path + heap
`BigInt` promotion on overflow; new `rt_int_*` in the runtime — the one planned
extension).
**Build:** arithmetic/comparison/logical/bitwise ops with CPython floor-div/mod
semantics (PITFALLS B1); truthiness; `typeck` still v1 (annotations + local
literal inference, `Dyn` elsewhere). `int` stays `Tagged` (bignum-safe).
**GC rooting becomes mandatory here** — Phase 1's no-shadow-frame leaf path is no
longer always safe: bignum promotion (`2**100`) and function calls both allocate,
so any `is_gc_root()` local live across them must be in a `ShadowFrame`. Emit
`gc_push`/`gc_pop` + root every such local; keep the `nroots == 0` fast-path only
for functions with no live-across-allocation roots (PITFALLS B15).
**Codegen:** the Phase-1 `Vec<Option<Value>>` "one assignment per local" model
breaks under loops/reassignment — switch to Cranelift `Variable`s
(`declare_var`/`def_var`/`use_var`) and a real block-by-block CFG walk with
jump/branch terminators.
**Gate:** scalar + control-flow corpus subset green.

### Phase 3 — Real `typeck` + representation optimization
**Goal:** turn inference from "annotations only" into a real solver, and start
*specializing* representation.
**Build:** full collect → solve → materialize over the lattice; bidirectional
checking; cross-instance class-field inference as constraints (PITFALLS B10); the
two proof-gated narrowings of Principle 3. Performance first appears here — and so
does the proof that precision is decoupled from correctness (the Phase 2 gate must
stay green throughout).
**Gate:** Phase 2 corpus still green; unboxed int/float paths verified; no
representation mismatch escapes the verifier.

### Phase 4 — Containers & iteration
**Goal:** `list`/`dict`/`set`/`tuple`(fixed+var)/`bytes`, comprehensions, the
iterator protocol, slicing.
**Build:** uniform tagged-`Value` element storage from day one (no `elem_tag` /
heap-mask side-tables ever — PITFALLS A5); GC roots derived from
`Repr::is_gc_root`; container element-type narrowing as constraints; empty-
container bootstrap via expected-type propagation in `typeck` (PITFALLS B4);
comprehension & iterator-protocol desugaring in `frontend-python`.
**Gate:** collections corpus subset green; GC soak clean.

### Phase 5 — Classes & dispatch
**Goal:** classes, fields, methods, inheritance, dunders, `super()`,
`@property`/`@staticmethod`/`@classmethod`, generics.
**Build:** **MRO via C3 linearization in `semantics` from the start**, so multiple
inheritance and vtable layout share one authoritative order; name-based vtable +
devirtualization when the receiver type is statically known; dunder dispatch
(arithmetic / comparison / container / conversion) with the CPython reflected rule
and `NotImplemented` fallback (PITFALLS B11); **monomorphization** erases
`SemTy::Var` before codegen.
**Gate:** classes/inheritance/dunders corpus subset green.

### Phase 6 — Closures, generators, decorators, varargs
**Goal:** nested functions, `nonlocal`/`global`, lambdas, `*args`/`**kwargs`
(def + call), generators (`yield`/`yield from`/`send`/`close`), user decorators.
**Build:** cell-based capture with transitive free-variable bubbling; generators
desugared at HIR level into regular functions; closures are plain `Repr::Closure`,
dispatched uniformly (no marker-bit ABI — there is no unboxed/tagged ABI split to
reconcile, per Principle 6).
**Gate:** functions/generators/closures corpus subset green.

### Phase 7 — Exceptions, `with`, `match`
**Goal:** `try/except/else/finally`, `raise` + chaining, full builtin exception
hierarchy + custom exceptions, context managers, structural `match`.
**Build:** setjmp/longjmp exception handling as the pragmatic first implementation
(setjmp called directly from generated code — PITFALLS B3; leak-free owned-message
raising — B2); **evaluate table-based zero-cost unwinding as a follow-up**, since
real tracebacks need it.
**Gate:** exceptions/match/with corpus subset green.

### Phase 8 — Modules, stdlib, real scripts
**Goal:** `import`/`from … import`, packages, cross-module classes/functions, the
stdlib surface, and **`microgpt.py` running end-to-end**.
**Build:** keep the declarative stdlib pattern (add a function = 2 files:
`stdlib-defs` + `runtime`, no lowering/codegen changes); cross-module types
round-trip through interned/placeholder ClassIds resolved before lowering; wire
the package search path.
**Gate:** full corpus + `microgpt.py` diff-clean against CPython.

### Phase 9 — Optimization & polish
**Goal:** competitive native performance and ergonomics.
**Build:** the optimizer passes (devirtualize, flatten-properties, inline,
constfold, peephole, dce, cold-block annotation) as representation-preserving
rewrites over typed MIR (they read `Repr`, never inference state — Principle 6);
binary-size gating (feature-gated runtime, strip, gc-sections); DWARF debug info.
The runtime's slab allocator + shadow-stack leaf optimization are already in place.
**Gate:** performance targets met; size targets met; all prior gates still green.
**Status: DONE** (devirtualize / flatten-properties had already landed in
lowering during Phase 5; benchmarks in `benchmarks/`, results in
`benchmarks/results.md`). Deferred follow-ups, by decision:
- **Full DWARF line tables** (~3-5 days): MIR carries no spans today. The
  sketch: a per-instruction span side-channel threaded `lowering →
  MirInst`, `FunctionBuilder::set_srcloc` per instruction at codegen, then
  a `gimli` `DebugLine` program assembled from the `SourceLoc → (file,
  line)` map and attached through `ObjectProduct::object` before write-out
  (the cg_clif pattern). Until then `--debug` gives symbol names
  (`pyaot_fn_<i>_<py_name>`), readable lldb backtraces and profiles.
- **MIR-level devirtualization pass**: documented stretch item — lowering
  already devirtualizes everything `method_overridden_below` proves, so a
  MIR pass would rewrite ~nothing on the corpus.

---

## Cross-cutting

- **Differential harness (Phase 1, used forever):** every corpus file is the
  spec. A feature isn't done until its corpus entry diffs clean vs CPython.
- **Verifier discipline:** the MIR verifier is a debug-build invariant at every
  pass boundary, from the first MIR ever produced.
- **Specialization is always optional:** if a representation optimization is
  unsure, it must fall back to `Tagged` / safe `repr_of` — never guess.
- **Before adding a flag/side-table/special-case:** stop and re-read PITFALLS
  Part A — that is the smell. Fix the representation or the constraint instead.
