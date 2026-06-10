# Pitfalls — traps in AOT-compiling Python, and how we avoid them

Hard-won knowledge about what bites when you compile a typed subset of Python 3
ahead-of-time. Read before building any front-half crate. Each entry is framed
forward: **the trap**, **why it bites**, **how this architecture avoids it**.

Two kinds of pitfall: **(A) architecture-level** — the deep ones the six
invariants exist to prevent; and **(B) concrete semantics/runtime gotchas** — the
rake that bites at implementation time no matter how clean the architecture. The
frozen `runtime` already handles most of (B) correctly; the danger is the
front-half failing to *cooperate* with it.

---

## A. Architecture-level traps (why the invariants exist)

### A1. Conflating semantic type with physical representation
**Trap:** one type enum answers both "what Python type is this?" and "how is it
stored in a slot/register?". You then need an `Any`-that-might-be-raw-or-pointer,
its sibling `HeapAny`, a second physical type field bolted on later, and an
`Option` sentinel whose `None` means two different things.
**Why it bites:** a slot's representation depends on *which producer wrote it* —
unknowable locally. Every downstream pass must re-derive it and they disagree.
Removing the ambiguity later is a multi-release migration that never finishes.
**Avoided by:** Invariant 1 — `SemTy` and `Repr` are separate; `Repr` is total
(never `Option`); gradual unknown is `SemTy::Dyn` whose `Repr` is always `Tagged`.

### A2. Making inference precision load-bearing for correctness
**Trap:** because type == representation, an imprecise inferred type yields a
wrong representation, which is a miscompile — SIGSEGV, `OverflowError` on
pointer-shaped ints, or `loss=NaN` from reading a `FloatObj` pointer as an f64.
**Why it bites:** inference can never be perfect, so the compiler is perpetually
one inference gap away from a memory-safety bug. Every new feature is a new way
to be imprecise and crash.
**Avoided by:** Invariant 2 — `Repr::Tagged` is always correct, and `repr_of`
gives a safe representation for every `SemTy`. Imprecise inference yields *slower*
code, never *wrong* code. Precision is a performance lever you can improve
post-hoc with zero correctness risk.

### A3. Inference as a fixpoint of mutually-dependent passes
**Trap:** type information is computed by N monotone passes (refine containers,
prescan params, propagate captures, re-run nested returns, …) run to a fixpoint;
correctness depends on which passes are *inside* the loop and how deep it iterates.
**Why it bites:** ordering and iteration depth are tuned empirically against test
failures; there are recursion gaps (a construct the harvester doesn't recurse
into); a "clean rewrite" of this shape tends to *grow* rather than shrink and
still diverge from the original on edge cases.
**Avoided by:** Invariant 4 — inference is ONE algorithm: collect constraints →
solve (union-find / worklist over the lattice) → materialize. No pass-fixpoint.
Cross-instance field widening and polymorphic dunders are *constraints*, not
extra passes.

**Test before adding a pass (so this trap is never re-opened by accident).** The
smell is **not** the *number* of passes — a forward pipeline of many
single-purpose passes (`parse → resolve → typeck → lower → verify → optimize →
codegen`) is normal, healthy, universal compiler architecture. The smell is two
specific things:

- **(i) Feedback / iteration.** Does the new pass feed information *back* into a
  pass that already ran, so the two must be re-run to converge? That re-creates
  the fixpoint. A *terminal, forward* pass (runs once, never revisited) does not.
- **(ii) Responsibility duplication.** Does the new pass *re-derive* something an
  existing pass already computes — a second source of truth for the same fact
  that can drift? (This is the same drift Invariant 1 and §A5 fight.)

If a new step is **(a)** a single forward walk, **(b)** with a distinct
responsibility, and **(c)** with no feedback into an earlier pass, add it freely
regardless of count. Example: `typeck::check_repr_boundaries` (the int→`float`
boundary check) is a single read-only validation that runs *once after* the infer
worklist, changes nothing, and feeds nothing back — so it lives behind the one
`infer` entry point (`infer` = derive **then** validate = bidirectional type
checking), not as a new pipeline pass. That is fine.

The one hard rule: **never split `infer` itself back into N interdependent
sub-passes.** Type *derivation* stays one monotone worklist. Everything around it
may be as many forward, single-purpose passes as the work needs.

### A4. Deciding the calling convention ad hoc, per function
**Trap:** whether a function takes/returns boxed-or-raw, tagged-or-untagged, is
decided case-by-case and recorded in per-function flags, a per-local "ABI is
immutable" flag, and marker bits stashed in a function pointer's high bits; a
separate "ABI repair" stage runs twice to reconcile call sites.
**Why it bites:** the flags interact combinatorially and drift; some ABIs are
*fundamentally* incompatible (e.g. a predicate returning a raw `i8` truthiness
value cannot also carry a boxed-`Value` return-flip — `false` boxes to a nonzero
low byte and reads as truthy). You discover the incompatibility only when a
specific higher-order call corrupts.
**Avoided by:** Invariant 3 — a function's ABI is a deterministic function of its
parameters' `Repr`. HOF callbacks are uniform `fn(Value)→Value` at the baseline;
unboxed call paths are a proven optimization, not an escape hatch. No flags, no
marker bits, no repair stage.

### A5. Boxing/representation logic leaking into every layer
**Trap:** "dict values are always boxed, so insert an unbox after every dict get",
"box raw ints before this iterator", "this runtime result must land in a GC slot"
— dozens of such rules scattered across lowering, the optimizer, and the runtime.
GC tracing needs per-slot side-tables (heap-field masks, element-tag arrays) to
know which words are pointers.
**Why it bites:** every new operation must remember every rule; miss one and the
GC traces a raw int as a pointer (crash) or arithmetic reads a pointer as an int.
The side-tables are a second source of truth that drifts from the real layout.
**Avoided by:** Invariant 3 + 5 — all coercions go through one `legalize` pass
(`coerce(have, need)`); container storage is uniformly tagged `Value` (no
side-tables); GC-rootness is `Repr::is_gc_root()`, derived, not stored.

### A6. 61-bit "tagged" int as the only integer
**Trap:** to fit an int in a tagged word you give it ~61 bits and silently wrap on
overflow.
**Why it bites:** it isn't Python — `2**100`, large factorials, and many hashes
break. The breakage is silent (wrong number), the worst kind.
**Avoided by:** `int` is arbitrary precision: a fixnum fast-path plus a heap
`BigInt` (`Repr::Heap(HeapShape::BigInt)`) it promotes to on overflow. `Raw(I64)`
is chosen only when a range proof guarantees no overflow.

---

## B. Concrete semantics / runtime gotchas

The frozen runtime already implements the correct behavior for most of these. The
trap is the *front-half* not cooperating. "Cooperate by" = what lowering/typeck
must do.

### B1. CPython floor-division and modulo signs
`-7 // 2 == -4` and `-7 % 2 == 1` in Python (floor / sign-of-divisor), vs Rust's
truncate / sign-of-dividend. **Cooperate by:** routing `int//int` and `int%int`
through the runtime ops (which apply the branchless `(r ^ b) < 0` adjustment) —
never emit a raw machine `div`/`rem` for Python semantics.

### B2. `longjmp` skips Rust destructors → leaks
Exception unwinding via `longjmp` does not run `Drop`, so a `format!()` `String`
built just before a raise leaks. **Cooperate by:** using the runtime's owned-
message raise path (ownership transferred before the jump) for dynamic messages;
byte-string literals for static messages.

### B3. `setjmp` must be called *directly* from generated code
If `setjmp` is wrapped in a Rust function, by the time `longjmp` fires that
frame is dead → SIGILL. It *appears* to work in release (LTO inlines the wrapper)
and breaks in debug. **Cooperate by:** emitting the `setjmp` call directly in
generated code at the handler site.

### B4. Empty container literals have no element type
`[]` / `{}` infer no element type and default to a heap-element assumption; later
appending ints makes the GC trace raw ints as pointers → SIGSEGV. **Cooperate
by:** propagating the *expected* type into empty literals during `typeck`
(`x: list[int] = []`), so the element representation is known before any store.

### B5. Tagged-`Value` runtime results may carry a heap pointer
Results of tag-dispatched ops (`rt_obj_add`, generator resume, …) can be a boxed
float or a class instance — a real heap pointer. If such a result lands in a
primitive-typed (non-GC-rooted) slot, the next collection frees it underfoot.
**Cooperate by:** giving any tagged-`Value` result a `Repr` that
`is_gc_root()` (i.e. `Tagged`/`Heap`), then unboxing *into* a primitive only via
`legalize` — never widen-narrow it through a non-rooted slot.

### B6. Numeric-tower collapse when building unions
Constructing a union via the lattice `join` applies `Int ⊔ Float = Float`, so a
runtime-distinguishable `Union[Int, Float]` collapses to `Float`; a downstream
narrowing then treats the lone `Float` as an "unbox me" hint and dereferences a
tagged int. **Cooperate by:** building runtime-distinguishable unions *directly*
(not via `join`) where members must stay distinct — `meet`/`minus` in
`lattice.rs` already do this; preserve the property anywhere you assemble unions.

### B7. HOF callback ABI: builtins vs compiled functions
A `map`/`filter` callback that is a builtin (`str`, `int`) expects a boxed
`*Obj`, while a Cranelift-compiled user function receives raw native values. Mix
them up and the GC traces raw ints. **Cooperate by:** keeping HOF callbacks on
the uniform tagged `fn(Value)→Value` baseline; specialize to raw only when the
callback's `Repr` is statically known on both sides.

### B8. Dict keys rely on consistent string interning
Fast dict lookup uses pointer-equality on interned strings before byte compare.
**Cooperate by:** interning compile-time string literals and dict keys through
the runtime's interner so equal keys are pointer-equal.

### B9. Stale runtime staticlib
The compiler links a prebuilt `libpyaot_runtime.a`; it is not auto-rebuilt.
Editing the runtime and rebuilding only the compiler links a stale archive →
ABI-mismatch segfaults. **Cooperate by:** `cargo build --workspace` (or rebuild
`pyaot-runtime`) whenever the runtime changes.

### B10. Cross-instance / call-site class-field types
A field assigned different types across methods, or whose type is only evident
from constructor call-site arguments, needs cross-site inference — otherwise it
keeps a too-narrow type and stores corrupt. **Cooperate by:** modeling field
types as constraints joined across *all* writes (including call-site arg types),
solved once — not inferred from `__init__` alone.

### B11. Polymorphic dunder `other` parameter
Giving an unannotated binary-dunder `other` a union that includes `Self` and then
distributing an operation through the union's class arm can synthesize a bogus
nested type (a notorious `loss=NaN` source). **Cooperate by:** treating dunder
dispatch as constraints over the tagged baseline; do not fabricate
representation-bearing types from the polymorphic seed.

### B12. Validate pointer alignment before dereferencing a tag
Runtime helpers that read an object's type tag can receive non-object values
(function pointers from closures have 4-byte code alignment, not 8). **Cooperate
by:** never handing a non-`Heap` `Repr` value to an API that dereferences it as
an object; the runtime guards alignment, but the front-half should not rely on
that guard for correctness.

### B13. min/max over heap-element iterables
`min`/`max` of a `list[str]`/class instances must return the *element* type and
compare via tagged dispatch — not raw pointer comparison (which orders by
address, not value). **Cooperate by:** typing the result as the element type and
selecting a tag-dispatched comparison when elements aren't a concrete primitive.

### B14. Single-threaded runtime statics
The runtime uses `UnsafeCell`-backed global state (GC, interner, registries) with
no locking — valid only because there is no threading. **Cooperate by:** keeping
threading/async out of scope; any future concurrency needs runtime synchronization
designed in, not bolted on.

### B15. The GC-rootless leaf path is conditionally correct
The shadow-stack leaf optimization (a function with no live-across-allocation
roots emits no `ShadowFrame`, `nroots == 0`) is correct — and tempting to leave in
place because the first programs (`print("hello")`, where the one string is
created and immediately consumed with nothing allocating between) genuinely don't
need a frame. **Why it bites:** the boundary is silent and easy to cross. The
moment *any* `is_gc_root()` local is live across an allocating call, the rootless
path is a use-after-free waiting for the next collection — and "allocating call"
includes the non-obvious ones: **bignum promotion** (`x = 2**100` boxes a heap
`BigInt`), a second string/container literal while the first is still live, and
**any function call** that may allocate. **Cooperate by:** deriving the root set
from `locals[i].repr.is_gc_root()` (never a stored flag) and emitting
`gc_push`/`gc_pop` + storing each live root into `frame.roots` whenever such a
local survives an allocating call; keep the `nroots == 0` fast-path strictly for
functions that provably have none. Do not let "it worked in Phase 1" justify
skipping the frame in Phase 2.

### B16. Bitwise / shift unboxing a possibly-bignum int
**Trap:** `&`/`|`/`^`/`<<`/`>>` look like pure machine-word ops, so it is tempting
to lower their operands as `Raw(I64)` (`UntagInt` → `band`/`bor`/`bxor`/shift →
`TagInt`). For a fixnum that is fine; but an `int` slot is `Tagged` and may
*dynamically* hold a heap `BigInt` pointer. `UntagInt` (`sshr 3`) on that pointer
yields garbage — and it is **silent**: `Tagged → Raw(I64)` is a legal coercion the
verifier accepts (it cannot know the runtime value is a bignum), so `x = 2**100;
x & 1` neither errors nor computes correctly. This is exactly the
"silent-wrong-via-premature-unboxing" Invariant 2 exists to prevent (cf. A2).
**Why it bites:** the literal form `2**100 & 1` *does* fail to compile
(`Heap(BigInt) → Raw(I64)` is absent from the coercion table), so the gap reads as
"only edge cases" — but a tagged variable holding the same bignum slips straight
through. **Avoided by:** routing bitwise/shift through the tagged-`Value`
baseline like arithmetic — `rt_obj_bitand`/`bitor`/`bitxor`/`lshift`/`rshift`
dispatch on the tag (fixnum fast path + `num-bigint`, demote on fit). A range-proven
`Raw(I64)` fast path is a deliberate Phase-3 optimization gated on a proof that the
operands cannot be bignum — never the default.

### B17. Raising Cranelift `opt_level` re-opens the setjmp soundness argument
**Trap:** the Phase-7 memory-backing of locals in `has_try` functions is sound
*today* because codegen runs at `opt_level=none` (the default — no flag is set):
every read is a `stack_load`, every write a `stack_store`, and nothing is
scheduled or forwarded across the direct `setjmp` call. Turning on
`opt_level=speed[_and_size]` (the obvious Phase-9 knob) enables Cranelift's
e-graph optimizations, and the argument then silently starts depending on
Cranelift's alias analysis treating *every call* as a memory clobber — if a
store-to-load forwarding ever ran past the `setjmp` call into the handler edge,
the handler would observe the pre-`try` value of a local reassigned inside the
body (the classic setjmp-clobber miscompile, in its memory form). **Why it
bites:** nothing fails at the moment the flag flips — the divergence appears
only on the exceptional path of an optimized build, the exact debug-vs-release
tell B3 warns about. **Avoided by:** treating the clobber assumption as a
*tested* invariant, not a given: the corpus pins it (`test_exceptions.py`
variable-preservation section; `p7_raise_tryexcept.py` `preserve()` covers
Raw-spilled floats and rooted strs/ints), so re-run the differential gate in
release *with the new opt level* before shipping Phase 9 — and if it breaks,
the fallbacks are per-frame `volatile`-style loads via `MemFlags` or fencing
the setjmp block boundary, never "hope the regalloc behaves".

---

## How to use this file

When implementing a front-half feature, scan Part B for anything it touches
(arithmetic → B1/B6; containers → B4/B5/B7; classes → B10/B11; exceptions →
B2/B3/B17). When tempted to add a flag, side-table, or special case to make a type
"work", stop — that is the smell Part A warns about; fix the representation or the
constraint instead.
