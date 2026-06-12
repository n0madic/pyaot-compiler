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
- links against `runtime` and produces a native executable;
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

7. **Build the front-half fresh from the design; the substrate is a stable
   base.** Front-half crates are implemented from ARCHITECTURE.md + each crate's
   `lib.rs`, using established algorithms (constraint solving, C3 MRO, standard
   optimizer passes) on their own merits. The substrate (`runtime`, `core-defs`,
   …) is a stable dependency, edited deliberately rather than rewritten.

8. **The runtime contract evolves deliberately with the compiler.** Its `Value`
   ABI + `rt_*` signatures are the seam the whole compiler targets, so changes
   are deliberate, never casual. When compiler development requires a runtime
   change — fixing a runtime bug, a new `rt_*`/ABI, a layout extension — make
   it, document it as a contract change, and back it with corpus coverage
   (precedents: bignum, `StrObj.char_len`). What stays forbidden is editing the
   runtime to paper over a front-half bug or to dodge front-half work.

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
against the runtime, and the **differential harness** (compile each
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

## Post-Phase-9 hardening backlog — lessons from the previous compiler

Phase 9 closed the plan's definition of "working". What remains is a ranked
backlog of items where the *previous* compiler is known to have gone wrong
after this exact point — each entry names the old failure it guards against.
None of these are gates; all of them are the places to be deliberate.

1. **Raw-int loop specialization must be a `typeck` proof, never an optimizer
   demotion pass.** The biggest remaining performance lever (`bench_int_loop`
   0.47x, `bench_containers` 0.74x vs CPython) is Principle 3's narrowing (a):
   `Tagged` int → `Raw(I64)` under a range/no-overflow proof. The old compiler
   attempted this as post-lowering `mir_ty` narrowing in the optimizer four
   times; three attempts caused mass regressions (198+ unbox mismatches) and
   the surviving one needed a producer-proof side-set plus an `abi_immutable`
   guard flag — both PITFALLS Part A smells. The proof (loop-bound range
   analysis) belongs in `typeck` *before* lowering, so `legalize` emits the
   raw representation natively and the verifier sees consistent `Repr` from
   the start. If the implementation ever needs a "repair" sweep after the
   fact, that is the signal the design is wrong — stop and move the proof
   earlier.

2. **Promote the MIR verifier to release builds at the final pre-codegen
   boundary.** Today the verifier is `#[cfg(debug_assertions)]`-only
   (`cli/main.rs`, `optimizer/lib.rs`); release builds compile with zero
   representation checking. The old compiler started the same way and ended
   with hard-error verification in *both* build profiles after release-only
   miscompiles slipped through (its Stage G.1). Per-boundary verification can
   stay debug-only; one mandatory hard-error pass at final-pre-codegen is
   cheap (linear in MIR) and catches optimizer bugs where they are introduced
   rather than as corpus SEGVs.

3. **Defence-in-depth at the proof-trusted `Tagged → Heap` seam.** The
   `TaggedToHeap` coercion is a bit-identical no-op justified entirely by a
   `typeck` proof — which means any future inference bug surfaces as a SEGV
   in the runtime, not as a `TypeError`. This already happened once
   (the Phase 8B–8F gradual-seam SEGV family: `join` on a non-list,
   `urlencode` with non-str values, `environ.get` miss) and was fixed
   correctly via checked coercions. Two cheap guards remain worth adding:
   (a) a debug-runtime tag assert in the hot `rt_*` shape-dereferencing
   entry points (the old compiler's `rt_*_abi` guards exist in the substrate
   for exactly this reason — extend the pattern to the stdlib seam);
   (b) keep every *new* gradual admission at a raw/heap ABI boundary on the
   checked-`Coerce` path — never widen the verifier's two legal checked
   shapes without a matching runtime guard.

4. **Land MRO-aware nominal subtyping in the lattice before anything depends
   on equality-only class joins.** `lattice.rs` still compares classes by
   `id1 == id2` (TODO in-tree). In the old compiler the equivalent gap — class
   joins collapsing to `Union`/`Any` instead of the common base — seeded the
   entire class-field-widening cascade (six-pass harvester fixes, per-function
   overlays). The C3 linearization already computed in `semantics` is the
   single source of truth; wire `join(Class(a), Class(b))` to the nearest
   common MRO ancestor while the consumer surface is still small.

   **Status: DONE.** Every lattice operation now takes a `ClassHierarchy` env
   (implemented by `ClassTable`, so the MRO data still lives only in `hir`):
   `Class(a) <: Class(b)` iff `b ∈ mro(a)`, and union canonicalization merges
   class members to their nearest common C3 ancestor (commutativity-guarded
   under multiple inheritance; no-common-ancestor pairs still form a `Union`).
   typeck's `nominal_subtype` shim is gone — the lattice covers its cases.
   Corpus: `p5_mro_join.py` (unannotated sibling joins, diamond, base-typed
   virtual dispatch).

5. **String performance work must not re-open the byte/char model.**
   `rt_str_len_int` and slicing are codepoint-correct but O(n) per call
   (`bench_str` 0.36x). The acceptable fix is a cached char-length (and/or an
   is-ASCII bit) in `StrObj` — a deliberate substrate extension under
   Principle 8, like bignum. The unacceptable fix is any fast path that
   reverts an operation to byte indexing: the old compiler shipped byte-`len`
   / byte-slices / char-`s[i]` simultaneously, and the three-way inconsistency
   was worse than either consistent model.

   **Status: DONE.** `StrObj` carries a cached `char_len` (codepoint count by
   the non-continuation-byte rule — `char_len == len` ⟺ ASCII, so no separate
   bit), filled at every allocation site (greedily via `count_codepoints`,
   arithmetically for concat/mul/slice/join/strip/remove-fix) and guarded by
   a `str_alloc_size` helper + `offset_of!(StrObj, data)` const assert against
   under-allocation, plus a debug validator in `rt_str_len_int`. `len()` is
   O(1); subscript/slice/slice-step/find/rfind take byte==char shortcuts only
   under *proven* ASCII and a single walk otherwise (no offsets `Vec` for
   plain slices); align ops early-return on the cached count. Observable
   semantics unchanged (corpus byte-exact); `bench_str` 0.36x → 0.50x.

6. **Exception hot path (0.15x) waits for table-based unwinding — not for a
   faster setjmp.** The deferred Phase 7 follow-up (zero-cost unwinding +
   real tracebacks) is the only fix that pays. Caching or hoisting setjmp
   frames breaks the two documented constraints that already bit once
   (PITFALLS B2 owned-message leaks, B3 dead-frame longjmp) for a constant
   factor on a path that needs an asymptotic change.

   **Status: DONE (unwinding).** setjmp is gone: protected regions are
   static `MirBlock::handler` annotations; every raising call inside one is
   a Cranelift `try_call` routed through a `CallConv::Tail` trampoline
   (mandatory — see the rewritten PITFALLS B3), and codegen bakes a
   PC→handler table the runtime unwinder binary-searches at raise time
   (frame-pointer walk + SP/FP-restoring resume stub; GC shadow frames are
   pruned address-wise via `gc::unwind_below`). The happy path of a `try`
   emits ZERO runtime calls and the `has_try` memory-backing of locals is
   deleted — handler edges are ordinary CFG edges the regalloc understands.
   `bench_exc_hotpath` 0.19x → 0.40x, now at parity with the same loop's
   non-try tagged-arithmetic gap (`bench_int_loop` 0.46x) — the exception
   tax itself is gone; the residual is raw-int specialization territory.
   **Real tracebacks: DONE.** The frontend emits `HirStmt::Line` markers
   (per statement + at every block head) from a per-module `LineMap`;
   lowering threads them as `MirInst::LineMarker` (DCE-surviving, excluded
   from inline size accounting) and codegen turns them into Cranelift
   srclocs. A second baked table (`pyaot_tb_table`: per function — relocated
   base address, code size, display name, file path, `[start,end)→line`
   ranges from `get_srclocs_sorted()`) is registered via
   `rt_tb_register_table`. Every raise snapshots the FP-chain's return PCs
   (generated frames only — runtime/trampoline frames are not in the table);
   resolution to `File "…", line N, in fn` happens lazily when an unhandled
   exception prints, in CPython's frame format, `<module>` for module
   bodies, per-module files for imports. Gated by
   `crates/cli/tests/traceback.rs` (the differential corpus requires exit 0,
   so unhandled output cannot be corpus-pinned). Documented divergences from
   CPython: no source-line echo / `^^^` anchors (no embedded source text),
   bare-`raise` keeps the original capture instead of appending the re-raise
   site, and inlined callees collapse into their caller's frame (keeping the
   innermost line) — compile with `--opt-level none` for full frame
   fidelity.

7. **Benchmark-gap pressure is the named adversary of Part A.** Every
   side-table, marker bit, and parallel `rt_*_tagged` variant in the old
   compiler was born as a quick win against a benchmark or a failing corpus
   entry. The remaining gaps (`int_loop`, `str`, `containers`,
   `exc_hotpath`) will generate exactly that pressure. The existing rule
   stands and is restated here at the point of maximum temptation: before
   adding a flag, side-table, or special case to win a benchmark, re-read
   PITFALLS Part A — fix the representation or the constraint instead.

   **Status: DONE — every gap closed the disciplined way, with zero new flags /
   side-tables / marker bits / `rt_*_tagged` variants.**
   - `containers` — already closed by Phase 3c (`bench_containers` 1.75x).
   - `exc_hotpath` — closed by **interprocedural raw-int specialization** (fix
     the *constraint*): the Phase-3c terminal interval analysis in
     `crates/typeck/src/intervals.rs` is now whole-program. Proven-bounded `int`
     values flow across **direct** call edges, so a specializable free
     function's params and return become `Raw(I64)` instead of `Tagged` (the ABI
     follows the `Repr` deterministically — no codegen edit, no ABI flag). It
     stays A3-safe: runs after the SemTy solver converges + materializes, writes
     only `raw_int_ok` / `ret_raw_int` representation flags, never feeds back
     into `SemTy`. A function is specializable only when its address is never
     taken (no `MakeClosure`, generator, or `ClassTable` method/static/class/
     property slot), so every call site is a direct `Call` and the unchecked
     `Tagged → Raw(I64)` arg untag is sound (an eligible arg is `≤ 2^48 < 2^60`
     → a fixnum, never a heap `BigInt`). `bench_exc_hotpath` 0.41x → 0.82x —
     past the non-try raw-int loop's gap, the exception tax AND the tagged-call
     tax both gone. `safe_div`'s MIR signature is `(Raw(I64), Raw(I64)) ->
     Raw(I64)`. Corpus: `p3c_interproc_raw.py` (the minimized bench shape; an
     address-taken callback + a per-position unbounded bignum arg + a recursive
     bounded function all correctly staying tagged — a mis-specialization would
     untag a heap `BigInt` as garbage, so a clean run is the soundness proof).
   - `str` — case-conversion family (`upper`/`lower`/`title`/`capitalize`/
     `swapcase`) given a byte-wise ASCII fast path gated on item #5's cached
     `char_len == len ⟺ ASCII` invariant (no new field, no re-opening the
     byte/char model): pure-ASCII strings skip the UTF-8 decode + char iteration
     + intermediate `String`, falling through to the Unicode path otherwise
     (`"straße".upper()` → `"STRASSE"` stays correct). `bench_str` 0.51x →
     1.05x. The dominant residual — 20000 build-phase `str(i)`+concat
     allocations — is **inherent to a non-SSO heap-string representation**;
     closing it would require an SSO/arena substrate change, exactly the big
     risky machinery #7 warns against. Accepted as the documented residual.
   - `int_loop` — **irreducible under Part A; documented, not hacked** (see
     PITFALLS A7). Collatz `n`/`steps` and fib `a`/`b` are accumulators with no
     static magnitude bound, unbounded in any sound interval domain. The only
     closers are a representation-ambiguous speculative deopt-to-bignum (the
     forbidden "could be raw or pointer" type) or a raise-on-overflow that breaks
     Python's arbitrary-precision `int` semantics — both forbidden by Part A.
     `bench_int_loop` stays ~0.51x and is correct to leave open.

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
