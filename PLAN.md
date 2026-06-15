# Implementation Plan — broadening the language subset

The compiler reached the original definition of "working": Phases 1–9 + the
post-Phase-9 hardening backlog are complete (the gated `corpus/` + `microgpt.py`
diff clean vs CPython; bignum; table-based zero-cost unwinding + real tracebacks;
raw-int specialization; MRO-aware lattice joins; cached `StrObj.char_len`; the
release-safe pre-codegen MIR verifier; the uniform value-call convention + its
`devirt` recovery pass). That completed phase log lives in git history (commits +
each crate's `lib.rs` doc) and the auto-memory.

What remains is **breadth**: the differential gate is green on its allowlist, but
**7** aspirational `corpus/test_*.py` files still exercise valid Python 3 the
compiler does not yet accept. None of the gaps below widen the "Out of scope"
list — they are subset breadth, not new dynamism.

## What still fails (the gate allowlist is everything *not* here)

Each row is the **first** error `pyaot` hits on that file (the parser/typeck stops
at the first gap, so a file usually hides more behind the one shown). Verified two
ways: `python3` accepts the construct, `pyaot` does not. The "§" links the
[remaining backlog](#remaining-backlog) item that closes it.

| `corpus/test_*.py` | First blocker (pyaot diagnostic) | What's needed to lift it | § |
|---|---|---|---|
| `test_classes.py` | `getattr() default is out of scope` (3-arg `getattr(o,"x",-1)`) | 3-arg `getattr` default; then `@abstractmethod` / method + class decorators / `object.__new__` / `NotImplemented` | §5, §11 |
| `test_collections.py` | `undefined name 'set'` (`defaultdict(set)` — a type used as a factory value) | `defaultdict(factory)` + `dd[k]=v`; `deque` mutators; `OrderedDict` | §10 |
| `test_collections_dict_set_bytes.py` | `unsupported method .fromkeys()` | `dict.fromkeys()` classmethod (drops the receiver) + the `dict\|` / `\|=` merge operators | §9 |
| `test_dead_code_warnings.py` | `isinstance() … requires a statically-typed value` | `isinstance(x, T)` on a gradual/`Any` receiver — a runtime tag query + flow narrowing | §7 |
| `test_file_io.py` | `non-UTF-8 bytes literals are out of scope` | non-UTF-8 `bytes` literals (`b"\xff"`, any byte ≥ `\x80`) | §14 |
| `test_generics.py` | `unsupported statement` (`type IntPair[V] = …`) | PEP 695 `type` aliases (incl. generic); subscripted instance annotation `Box[int]`; `Protocol` | §3, §12 |
| `test_types_system.py` | `unsupported statement` (`type IntSet = set[int]`) | PEP 695 `type` aliases; `X: TypeAlias = T`; `Protocol` | §3, §12 |

`test_stdlib_urllib.py` is **not** a feature gap — it exercises the live
`urlopen`/`urlretrieve` network paths and runs (self-checking) only under
`PYAOT_NET_TESTS` (the offline `test_stdlib_urllib_core.py` sibling is on the
gate).

---

## Guiding principles

These are *why* features are built the way they are, and they bind every backlog
item below.

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

9. **Differential testing is the spine.** The corpus-vs-CPython harness gates
   every feature. A backlog item is not done until its construct diffs clean vs
   CPython on the corpus (lift the aspirational `test_*.py` files onto the gate
   allowlist as their gaps close).

### Anti-patterns to avoid (do not reintroduce)

A fixpoint-of-passes inference • per-function ABI flags / argument-marker bits /
a separate ABI-repair stage • a dual logical/physical type field with an optional,
dual-meaning sentinel • GC per-slot side-table tag masks • a 61-bit tagged int •
any representation-ambiguous "could be raw or pointer" type. Full catalogue with
rationale in **[PITFALLS.md](PITFALLS.md)** — Part A.

### Where the work lands

Syntax/semantic gaps (§3, §7–8, §11–12, §14) are front-half work —
`frontend-python` (parse/desugar), `typeck` (constraints), and `lowering` —
gated by Principle 9 and the verifier. Builtin **functions** (§5) and **methods**
on builtin/stdlib types (§9–10) follow the declarative two-file pattern of
Principle 8 (`stdlib-defs` descriptor + `runtime` `rt_*`), unless they need true
HOF/representation handling. Keep every gradual/raw seam on the checked-coerce
path (PITFALLS A2/A3).

### Reference material — use these, don't re-derive

- **Corpus: fully absorbed.** Every `examples/test_*.py` of the previous
  compiler exists in `corpus/`; three were deliberately cleaned of old-compiler
  workarounds (`test_exceptions.py` `exc_type: int` hack, `test_match.py`,
  `test_stdlib_sys.py`) — the `corpus/` versions are authoritative.
- **`crates/types/src/dunders.rs`** — the dunder classification tables
  (`DunderKind`, `dunder_kind`, `canonical_dunder_name`, `reflected_name`).
  Every backlog item touching operators or dunders (§9 methods, §11 dunder
  results) consumes this table instead of hardcoding name lists. The old
  `polymorphic_other_type` helper was deliberately **not** ported (its blind
  `Self`-injection into the `other` Union was the microgpt `loss=NaN` root cause —
  type `other` in the solver instead).
- **Runtime registries already in the fork.** `vtable.rs` method registry +
  `ops/dunder_dispatch.rs` (FNV-1a name-hash probes) — §12 `Protocol` builds on
  these; no new registry mechanism.
- **Builtin-signature reference.** The old repo's `crates/stdlib-defs/`
  (`../python-compiler-rust`) is the kwargs/signature catalogue to consult when
  authoring §5/§9 descriptors — read-only reference, not a code source.

---

## Remaining backlog

Only the *open* items remain below; everything completed has been removed (it
lives in git history + auto-memory). Section numbers are kept stable so the
`test_*.py` table and the PITFALLS notes still reference them.

### 3. Statements
- **PEP 695 `type X = T`** (and generic `type X[V] = T`) — `type IntPair = tuple[int, int]`.
  Blocks `test_generics.py`, `test_types_system.py`.
- **`X: TypeAlias = T`** (PEP 613) — RHS type currently rejected as a value.
- **`...` (Ellipsis) as a statement / stub body** — `def f() -> int: ...` (Protocol stubs).

### 5. Builtins
- **3-arg `getattr(o, "x", default)`** — the literal-name 2-arg form is done; the
  default arg is rejected (`getattr() default is out of scope`). Blocks
  `test_classes.py`. (Dynamic `getattr(o, name_var)` stays out of scope.)
- **`object` / `object.__new__(cls)`** and **`NotImplemented`** — part of the
  class-OOP cluster `test_classes.py` needs (gated by `@abstractmethod`, §11).

### 7. `isinstance`
- **Gradual / `Any` receiver** — `isinstance(any_value, str)`. Currently a loud
  error ("a runtime type query on a gradual value is out of scope"). Blocks
  `test_dead_code_warnings.py`. Decide: support via a runtime `ObjHeader.type_tag`
  query + flow-sensitive narrowing in `solve`, or keep out-of-scope and document.
  (Tuple-of-types and concrete/container-target `isinstance` are already done.)

### 8. Numeric tower (int↔float) — remaining seams
The `-> float` return seam and the annotated-`float`-LOCAL seam are done (checked
`Tagged → Raw(F64)` unbox via `coerce_value`/`rt_unbox_float`). Still rejected:
- **`float` GLOBAL / FIELD slots** — a cross-function `x: float` global, `self.v:
  float`. These are physically *tagged* slots that unbox on READ via an *unchecked*
  `UnboxFloat` (stores coerce to plain `Tagged`), so accepting an int there would
  later misread (PITFALLS A2). Needs a store-side coerce-to-float-then-box so the
  slot holds a genuine `FloatObj` — a separate, larger change.
- **Passing int/bool to a `float` PARAMETER** (free-fn / method / ctor / dunder).
  The method/ctor/dunder arg seams pass `(loc, repr)` without a per-arg `SemTy`, so
  `needs_check` can't be evaluated without threading types through. Kept rejecting
  to avoid an accept-then-SEGV.

### 9. Methods on builtin types — remaining
- **`dict.fromkeys(keys, v)`** — a **classmethod** whose `rt_dict_fromkeys(keys,
  value)` **drops the receiver**, so it does NOT fit the recv-first `MethodRecv`
  ContainerOp signature; needs a distinct dispatch. Blocks
  `test_collections_dict_set_bytes.py`.
- **`dict | dict` / `dict |= dict` merge operators** (PEP 584) — operator-level,
  beyond "methods on builtin types"; the other blocker on
  `test_collections_dict_set_bytes.py`.
- Documented scope limits on the shipped str/bytes batches (unprobed, not
  blocking): predicates are ASCII-only, `replace` has no `count`, `find`/`index`
  take no `start`/`end`, `encode`/`decode` ignore the encoding.

### 10. `collections` module — remaining
- **`defaultdict`** — a type passed as the factory (`defaultdict(int)`,
  `defaultdict(set)`) and subscript-store `dd[k]=v`. Blocks `test_collections.py`.
  (Special-case the builtin factories `int`/`list`/`dict`/`set` → zero-value
  thunks plus user functions — don't grow first-class type objects.)
- **`deque`** — all mutating/query methods (`append`/`appendleft`/`pop`/`popleft`/
  `rotate`/…), item assignment `dq[i]=v`, and the bounded-deque `maxlen` argument
  (`deque(it, n)`). Construction-from-iterable (`deque([1,2,3])`), read, repr, and
  iteration (`for`/`list`/`sum`/`",".join` — through the generic `rt_iter_deque`
  seam) now work; `del dq[i]` is wired but unexercisable until mutation lands.
- **`OrderedDict`** — `move_to_end`, `popitem`.

### 11. Classes / OOP (all open)
- **`@abstractmethod`** and general method decorators.
- **Class decorators** — any `@deco` on a class (incl. `@runtime_checkable`).
- **`object` / `object.__new__(cls)`** and **`NotImplemented`** (see §5).
- **`abs()` on a user class** — the `__abs__` result is not statically typed, so a
  later `.attr` on it fails.

These together block `test_classes.py`.

### 12. Typing / generics — remaining
- **Subscripted instance annotation** — `b: Box[int] = …` (the `Generic[T]` base
  parses; the `Name[T]` *annotation use* fails). Blocks `test_generics.py`.
- **`Protocol` base class** — `unknown base class Protocol` (structural subtyping
  unsupported); also subscripted **`Protocol[T]`**. (Multi-iterable `zip` is done.)

### 14. Literals
- **Non-UTF-8 bytes literals** — any byte ≥ `\x80`, e.g. `b"\xff"` (blocks
  `test_file_io.py`; ASCII and `\x00`–`\x7f` already work).

### 1. Calls & arguments — residual
- **Heap-arg seam** (`list`/`str` param of a genuinely-`Dyn` callee) keeps the
  existing `TaggedToHeap` trust — a deferred precision note, not a blocker.

---

## Known traps — read the matching note before starting an item

Each construct below works in the previous compiler; these notes record where it
got it *wrong first*. Only the traps for **open** items remain (the rest moved to
git history with their fixes).

- **§7 `isinstance` (gradual receiver).** Accepting the syntax is the small half;
  the value is flow-sensitive narrowing in `solve`, and retrofitting narrowing
  late was a documented multi-pass cascade in the previous compiler — wire
  narrowing in *together with* the syntax. For the gradual receiver:
  `ObjHeader.type_tag` makes the runtime query one load — support it rather than
  carving an out-of-scope hole.
- **§8 numeric tower.** `int` is Tagged (fixnum-or-bignum) and `float` is
  `Raw(F64)`, so int→float at a slot is a real `legalize` coercion with a bignum
  arm (precision loss above 2⁵³ matches CPython's `float(int)`) — never a noop. On
  the typeck side make `int ⊔ float = float` a deliberate lattice rule; the
  previous compiler repeatedly leaked these joins to `Any` instead. (The remaining
  seams are GLOBAL/FIELD slots and `float` PARAMETERS — see §8.)
- **§10 `deque`.** Its method names collide with `list` (`append`, `pop`, …); in
  the previous compiler that leaked wrong element-type constraints into the solver
  for look-alike receivers. Key §9–§11 method constraints by receiver `SemTy`,
  never by method name alone.
- **§10 `defaultdict(int)`.** A type used as a value. Don't grow first-class type
  objects for this — special-case the builtin factories
  (`int`/`list`/`dict`/`set` → zero-value thunks) plus user functions.
- **§11 method/class decorators.** The root of the previous compiler's
  per-function-ABI saga: `@property` getters with primitive returns got
  return-ABI-flipped, then needed flags and side-tables to track it. The
  immunizing rule: a function whose identity escapes through *any* decorator is
  address-taken ⇒ uniform Tagged ABI, decided in `typeck`, no per-function
  exceptions. (The uniform value-call convention already realizes this for value
  calls — keep class/method decorators on the same single ABI.)
- **§11 `abs()` / dunder results.** Two lessons: dunder return types must enter the
  solver as ordinary constraints (post-hoc threading caused a bound-result SEGV in
  the previous compiler), and the `other` param of a binary dunder must not get
  `Self` blindly injected into its Union (the microgpt `loss=NaN` root cause).
- **§14 non-UTF-8 bytes.** Keep `bytes` permanently out of the string
  interner/`StrObj`; one shared pool entry holding a byte ≥ `\x80` breaks the
  codepoint invariant behind `char_len`.

---

## Cross-cutting

- **Differential harness (the spine):** every corpus file is the spec. A feature
  isn't done until its corpus entry diffs clean vs CPython; close a backlog item
  by lifting the relevant `test_*.py` onto the gate allowlist
  (`crates/cli/tests/differential.rs::PHASE_CORPUS`).
- **Verifier discipline:** the MIR verifier is a debug-build invariant at every
  pass boundary, plus one mandatory release-safe pass at final pre-codegen.
- **Specialization is always optional:** if a representation optimization is
  unsure, it must fall back to `Tagged` / safe `repr_of` — never guess.
- **Before adding a flag/side-table/special-case:** stop and re-read PITFALLS
  Part A — that is the smell. Fix the representation or the constraint instead.
- **Interaction probes:** nearly every late bug in the previous compiler was a
  *pair* of features (kwargs × closure capture, dunder × Union, decorator ×
  varargs), not a single feature. When closing a backlog item, add at least one
  corpus probe that crosses it with an already-green feature.
- **Stdlib breadth is the layer after this backlog.** The runtime fork already
  carries the previous compiler's `rt_*` surface for `json`/`re`/`os`/`time`/
  `random`/`hashlib`/file I/O, so closing it is mostly `stdlib-defs` descriptors
  (Principle 8) — but only with corpus probes added per module. Never declare a
  module supported on the strength of inherited runtime code alone.
