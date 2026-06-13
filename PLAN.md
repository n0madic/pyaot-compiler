# Implementation Plan — broadening the language subset

The compiler reached the plan's original definition of "working" (see below):
Phases 1–9 and the post-Phase-9 hardening backlog (items 1–7) are **all
complete** — the gated `corpus/` + `microgpt.py` diff clean vs CPython, bignum,
table-based zero-cost unwinding + real tracebacks, raw-int specialization
(Phase 3c intervals + interprocedural), MRO-aware lattice joins, cached
`StrObj.char_len`, and the release-safe pre-codegen MIR verifier all landed. That
completed phase log now lives in git history (commits + each crate's `lib.rs`
doc) and the auto-memory; this plan no longer carries it.

What remains is **breadth**: the differential gate is green on its allowlist, but
~17 aspirational `corpus/test_*.py` files exercise valid Python 3 the compiler
does not yet accept. The full, deduplicated inventory of those gaps is the
**[Remaining backlog](#remaining-backlog--broaden-the-language-subset)** below.
The principles and anti-patterns that governed the build still govern every item
in it.

## Definition of "working" (reached)

A `pyaot` binary that:

- compiles the static-Python `corpus/` and runs **`microgpt.py`** (the north-star
  real script) unchanged or with only standard-syntax tweaks;
- produces output **identical to CPython** on every corpus file (differential gate);
- has **arbitrary-precision `int`**;
- links against `runtime` and produces a native executable;
- reaches competitive performance *after* the optimization phase — not before
  (correctness never waits on the optimizer; see Principle 2).

Out of scope (too dynamic for AOT): `eval`/`exec`/`compile`, metaclasses,
`__dict__` mutation, **dynamic** `getattr(obj, name_var)` (a literal-name
`getattr(obj, "x")` is in scope — see backlog §5), `globals()`/`locals()`,
`inspect`, `import *`, runtime class creation.

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

---

## Remaining backlog — broaden the language subset

Every gap below is **valid Python 3** that the current compiler rejects or
crashes on, harvested by probing each failing `corpus/test_*.py` construct-by-
construct (the parser stops at the first error, so whole-file compiles hide
most of them) and confirmed two ways: `python3` accepts the snippet, `pyaot`
does not. None of these widen the "Out of scope" list — they are subset breadth,
not new dynamism.

**Where the work lands.** Syntax/semantic gaps (§1–4, §7–8, §12–14) are
front-half work — `frontend-python` (parse/desugar), `typeck` (constraints), and
`lowering` — gated by Principle 9 and the verifier. Builtin **functions** (§5–6)
and **methods** on builtin/stdlib types (§9–11) follow the declarative
two-file pattern of Principle 8 (`stdlib-defs` descriptor + `runtime` `rt_*`),
unless they need true HOF/representation handling (`map`/`filter`, `type()`).
Keep every gradual/raw seam on the checked-coerce path (PITFALLS A2/A3).

**Already transferred from the previous compiler — use these, don't re-derive.**

- **Corpus: fully absorbed.** Every `examples/test_*.py` of the previous
  compiler exists in `corpus/` (verified file-by-file); three were deliberately
  cleaned of old-compiler workarounds (`test_exceptions.py` `exc_type: int`
  hack, `test_match.py`, `test_stdlib_sys.py`) — the `corpus/` versions are
  authoritative, never re-sync from the old repo.
- **`crates/types/src/dunders.rs`** — the dunder classification tables ported
  name-level: `DunderKind`, `dunder_kind`, `canonical_dunder_name`,
  `reflected_name` (incl. comparison pairs `__lt__`↔`__gt__`, self-reflected
  `__eq__`/`__ne__`). Every backlog item touching operators or dunders (§2
  `@`/`__matmul__`, §9 methods, §11 dunder results) must consume this table
  instead of hardcoding name lists; `typeck` currently hardcodes
  `["__add__", "__radd__"]` in one spot — migrate it onto the table when first
  touching that code. The old `polymorphic_other_type` helper was deliberately
  **not** ported: its blind `Self`-injection into the `other` Union was the
  microgpt `loss=NaN` root cause — type `other` in the solver instead (see the
  module docs).
- **Runtime registries already in the fork.** `vtable.rs` method registry +
  `ops/dunder_dispatch.rs` (FNV-1a name-hash probes) — §5 `hasattr` and §12
  `Protocol`/`runtime_checkable` build on these; no new registry mechanism.
- **Builtin-signature reference.** The old repo's `crates/stdlib-defs/`
  (`../python-compiler-rust`) is the kwargs/signature catalogue to consult
  when authoring §1/§5/§9 descriptors — read-only reference, not a code
  source.

### Highest-leverage first
These few gaps block the most files — close them before the long tail.

| Gap | Files blocked | First error |
|---|---|---|
| ~~**`is`/`is not` against non-`None`**~~ — DONE (backlog §2) | 6 | ~~`is / is not is only supported against None`~~ |
| ~~**`del`** statement (`del d[k]`, `del name`, `del obj.attr`)~~ — DONE (backlog §3) | 4 | ~~`unsupported statement for this milestone`~~ |
| ~~**`*seq` spread into a non-`*args` callee**~~ — DONE (backlog §1) | 3 | ~~`f() takes no *args, cannot spread * into it`~~ |
| ~~**Nested destructuring** `a, (b, c) = …` (assign / `for` / comprehension)~~ — DONE (backlog §4) | 2 | ~~`tuple/list unpacking assignment is not yet supported`~~ |
| ~~**`type()` builtin** (incl. `type(x).__name__`)~~ — DONE (§6); `hash()` still pending | 3 | ~~`builtin Type not supported in Phase 2`~~ |
| ~~**int→float numeric tower through a `float` slot**~~ — DONE for return + annotated-local seams (§8); global/field/param deferred | 2 | ~~`int cannot be returned/assigned to a float slot`~~ |

### 1. Calls & arguments
- ~~**kwargs on indirect/builtin calls**~~ — DONE (Phase 10): `sorted(xs, key=, reverse=)`
  (compiled key loop + `rt_list_sort_by_keys` tandem sort), `dict(a=1,b=2)` /
  `dict(pos, kw=)`, `enumerate(xs, start=1)`; written-order argument staging
  fixed for ALL keyword calls (incl. the pre-existing direct-call/stdlib bug).
  Truly indirect callees (a callee-typed variable) still reject keywords.
- ~~**kwargs on method calls**~~ — DONE (Phase 10): user-class methods
  (defaults / virtual / super / static / classmethod / `**kwargs` leftovers via
  `MethodCall.kwargs` + `pyaot_hir::match_keywords`), `list.sort(key=, reverse=)`.
  The `str.format(name=)`/`str.split(sep=)`/`str.encode(encoding=)`/
  `str.replace(count=)` entries wait on §9 (those methods don't exist yet) —
  the kwargs mechanism is ready for them. Caveats: `.sort(key=K)` desugars by
  method NAME (runtime TypeError guard); virtual calls require identical
  parameter names/defaults across overrides when keywords/defaults are used.
  Constructor kwargs `Cls(x=1)` are nearly free now (match_keywords) — small
  follow-up.
- ~~**`*seq` spread**~~ — DONE (backlog §1): all four sub-cases. A list/tuple
  LITERAL spread flattens at compile time into plain positionals (reusing the
  existing slot-matching path); a runtime sequence (variable / call result /
  comprehension) materializes a fresh `argv` list in WRITTEN order (the iterator
  protocol, so any iterable works), runs an arg-count guard (`TypeError` on
  mismatch), then binds each parameter by position — required slots `argv[i]`,
  defaulted slots `(i < len) ? argv[i] : default`, a `*args` callee's rest =
  `tuple(argv[n_fixed:])`. Covers fixed-arity, mixed `f(1, *seq, 4)`, multiple
  `f(*a, *b, c)`, empty, defaults filled from a short spread, and spread covering
  a varargs callee's leading fixed params. A `float`/`bool` parameter (Raw
  reinterpret, which rejects gradual `Dyn`) is laundered through a `pin_tagged`
  authoritative-typed local. Spread into a **decorated** function fills its
  `(*args, **kwargs)` wrapper's args tuple. Gated by `corpus/p13_spread.py`.
  Entirely front-half (`frontend-python`) — no new runtime/HIR/typeck surface.
  Out of scope here (separate gaps): `**d` spread (next item); spread into a
  method call / class constructor (still a loud error); the `test_decorator_
  factory.py` `plain_deco` shape (its wrapper lacks a `Callable[...]` return
  annotation, so the decorated slot is `Dyn` — a decorator-return-type-inference
  gap, NOT spread). Keyword args are not combined with a runtime `*` spread.
- **`**d` spread into a call** — `f(**{"a":1})` (was Phase 6C out-of-scope).
- **Mutable default parameter** — `def f(x, lst=[])`, `d={}`.
- **Non-literal default** — `def f(count=5+5)`.

### 2. Operators & expressions
- ~~**`is`/`is not` against non-`None`**~~ — DONE: `x is True`, `a is b` lower to
  a dedicated `HirExprKind::Is` → `rt_is` (bit-identity; `None`'s ABI encodings
  normalized via `rt_is_none`, so the `is None` `IsNone` path is untouched and
  never dispatches through `__eq__`). Int/str caching is NOT modeled (the trap
  below). Gated by `corpus/p11_is_identity.py`. Still open: `type(x) is T` (the
  `type()` builtin landed in §6, but a "type object" is its repr StrObj, so `is`
  would compare two distinct StrObjs by pointer — a documented out-of-scope
  divergence, NOT a missing feature) and chained `a is b is c` (rejected via
  `map_cmp`, as before).
- **Walrus `:=`** — in `if`/`while`/nested expressions, everywhere.
- **Matrix-multiply `@` / `__matmul__`**.

### 3. Statements
- ~~**`del`**~~ — DONE: `del d[k]`/`del li[i]`/a class `__delitem__` are runtime
  deletes (`HirStmt::DelItem` → `rt_dict_delete`/`rt_list_delete`/`rt_any_delitem`,
  raising KeyError/IndexError like CPython); `del name`/`del obj.attr` sidestep
  the missing definite-assignment analysis with an **unbound sentinel +
  runtime read-guard** (the CPython NULL-in-fast-locals model): the delete stores
  `Value::UNBOUND` (the `RESERVED_TAG` immediate) into the slot, and a read of a
  *deletable* slot is wrapped in `rt_check_bound` → `UnboundLocalError` (local) /
  `NameError` (global) / `AttributeError` (attr). Correct on all control-flow
  paths with zero CFG analysis, costing a guard only on reads of `del`'d slots.
  Gated by `corpus/p12_del.py`. Out of scope: `del` of a captured/cell variable
  (clear error), `del ClassName.attr`, and `del dq[i]` is wired
  (`rt_any_delitem` → `rt_deque_delete`) but unexercisable until deque
  construction/mutation lands (§10) — covered by a runtime unit test instead.
- **PEP 695 `type X = T`** — `type IntPair = tuple[int, int]`.
- **`X: TypeAlias = T`** (PEP 613) — RHS type rejected as a value.
- **`...` (Ellipsis) as a statement / stub body** — `def f() -> int: ...` (Protocol stubs).

### 4. Unpacking & loop targets
- ~~**Nested destructuring**~~ — DONE: `a, (b, c) = …`, `(m1,m2),(m3,m4) = …`,
  `g, [h, i] = …`, deep nesting, and nested + starred — in assignment, `for`, and
  comprehension/genexpr targets. The whole unpacking pipeline funnels through one
  method `assign_to_target`, which now recurses into a Tuple/List target via
  `lower_unpack_subscript` (each nested element is staged and re-subscripted
  positionally), so all three contexts get nesting from one change; nested
  attribute/subscript leaves reuse the existing `SetAttr`/`SetItem` arms.
  Entirely front-half (`frontend-python`) — no HIR/typeck/lowering surface (nested
  unpacking desugars to plain `Assign` + `Subscript` chains; `subscript_ty` already
  types arbitrary index depth). Gated by `corpus/p14_nested_unpack.py`. Inherited
  limitation (same as flat unpack): an over-long runtime/inner sequence is NOT
  statically rejected (CPython's "too many values to unpack" not raised). The
  aspirational `test_collections_list_tuple.py` / `test_iteration.py` stay off the
  gate on UNRELATED gaps — `test_iteration.py` on the next bullet (bare
  attr/subscript `for`-targets); `test_collections_list_tuple.py` now reaches §9
  `tuple.index()`/`.count()` (its earlier blocker — a tuple SLICE result, typed
  variable-length `tuple[T, ...]` by `slice_ty`, assigned into an annotated
  fixed-arity `tuple[T, …]` slot — is FIXED: the repr-contract check
  (`check_reinterpret`) now admits a `tuple`→`tuple` store when element `Repr`s
  match per index, since fixed/variable arity is one physical `TupleObj`; gated by
  `corpus/p15_tuple_slice_slot.py`).
- **Attribute / subscript as a `for` target** — `for obj.attr in …`, `for lst[i] in …`
  (`bind_for_target` only accepts a `Name` or a sequence pattern; a bare
  `Attribute`/`Subscript` target raises "unsupported for-loop target". This is what
  keeps `test_iteration.py` off the gate after nested destructuring landed.)
- **`range()` for-loop with a non-literal step** — `range(10,0,-(-1))`, a variable step, `range(0,10,len(xs))` (the `list(range(...))` value form already works; the restriction is only in the for-loop desugar).

### 5. Builtins — `undefined name`
`map`, `filter`, `round`, `pow`, `all`, `any`, `id`, `divmod`, `bin`, `hex`,
`oct`, `format`, `ascii`, `getattr` (literal-name), `setattr`, `hasattr`,
`issubclass`, `object` (`object.__new__`), `NotImplemented`.

### 6. Builtins — `Phase 2 codegen not supported`
- ~~**`type()`**~~ — DONE: incl. `type(x).__name__`, `str(type(x))`,
  `print(type(x))`. Gated by `corpus/p17_type_builtin.py`. A "type object" IS its
  repr StrObj (`type(x)` → `rt_builtin_type` → `<class '...'>`); `.__name__` is a
  lowering peephole through `rt_type_name_extract` (same runtime string, bare last
  segment). Out of scope (unprobed divergences): `type(x) is T` / `type(x) is
  type(y)` (pointer-identity on distinct StrObjs; `p11_is_identity.py` already
  defers it) and `repr(type(x))` (would add quotes). `test_builtins.py` /
  `test_classes.py` (the type-blocked files) stay OFF the gate — now blocked by
  unrelated gaps (`pow` §5; the `@` matmul operator §11 respectively).
- **`hash()`** — still pending (`builtin Hash not supported in Phase 2`; needs its
  own probe and a `rt_builtin_hash` codegen arm).

### 7. `isinstance`
- **Tuple of types** — `isinstance(x, (int, str))` (single-type form works).
- **Container targets** — `isinstance(x, list/dict/tuple/set)`.
- **Gradual/`Any` receiver** — "runtime type query on a gradual value is out of scope" (decide: support via a runtime tag query, or keep out-of-scope and document).

### 8. Numeric tower (int↔float)
- ~~An `int`/widened local returned through `-> float`; a literal `return 0` in a `-> float` function; an unannotated mixed `return 1.5 / return 0` inferred as `Any` and rejected at a `float` slot.~~ — DONE for the two `Raw(F64)`-slot seams: **return through `-> float`** and **assignment to an annotated `float` LOCAL** (incl. a `__main__` top-level local). int / bool / gradual `Dyn` are accepted at these seams via a new `allow_numeric_coerce` gate in `check_reinterpret`; the coercion lands at the store as a CHECKED `Tagged → Raw(F64)` unbox (`coerce_value` helper → `rt_unbox_float`, now with a `BigInt` arm for `float(huge_int)` → round-to-nearest, ±inf on overflow). The annotation is a *contract* (CPython keeps the raw int), so the divergence is observable only via repr-print — gated by `corpus/p16_numeric_tower_float.py` (asserts via `==`, prints only float-forced results; covers int/bool returns, a `: float` local from int, a `Dyn` mixed return into a float local, the `2 ** 62` BigInt arm, and a `sum`-over-floats interaction). Untouched: `is_subtype_of` (covariant for generics — a global `int<:float` would unsoundly admit `list[int] <: list[float]`), `numeric_promote`, `raw_uniform`. (bool↔int promotion already worked — this closes int↔float.)
  - **Deferred sub-items** (kept rejected, no liftable corpus needs them):
    - **`float` GLOBAL / FIELD slots** (a genuine cross-function `x: float` global; `self.v: float`). Physically *tagged* slots that unbox on READ via an *unchecked* `UnboxFloat` (stores coerce to plain `Tagged`), so accepting an int there would later misread (PITFALLS A2). Needs a store-side coerce-to-float-then-box so the slot holds a genuine `FloatObj` — a separate, larger change.
    - **Passing int/bool to a `float` PARAMETER** (free-fn / method / ctor / dunder). The method/ctor/dunder arg seams pass `(loc, repr)` without a per-arg `SemTy`, so `needs_check` can't be evaluated without threading types through. Kept rejecting to avoid an accept-then-SEGV.

### 9. Methods on builtin types
- **`int`**: `bit_length`, `bit_count`, `conjugate`, `__index__`.
- **`str`**: `format`, `split`, `rsplit`, `replace`, `lstrip`/`rstrip`, `removeprefix`/`removesuffix`, `expandtabs`, `splitlines`, `partition`/`rpartition`, `rindex`, `encode`, predicates `isdigit`/`isalpha`/`isalnum`/`isspace`/`isupper`/`islower`/`isascii`. (`upper`/`lower`/`strip`/`find`/`title`/`center`/… already work.)
- **`bytes`**: `startswith`, `endswith`, `find`, `rfind`, `count`, `replace`, `split`/`rsplit`, `strip`/`lstrip`/`rstrip`, `upper`/`lower`, `join` — only `.decode()` is supported today.
- **`tuple`**: `index`, `count`.
- **`dict`**: `popitem`, `fromkeys`.
- **`set`**: `issubset`, `issuperset`, `isdisjoint`, `intersection_update`, `difference_update`, `symmetric_difference_update`. (`union`/`intersection`/`|&-^` already work.)

### 10. `collections` module
- **`Counter`** — `undefined symbol rt_make_counter` at link. The runtime fork *already has* `counter.rs` (`rt_make_counter_empty` / `rt_make_counter_from_iter` / `rt_counter_most_common`) — the frontend emits a symbol name that doesn't exist, so this is pure wiring, not runtime work. `.total()`, `.most_common()` likewise.
- **`defaultdict`** — a type passed as the factory (`defaultdict(int)`); subscript-store `dd[k]=v`.
- **`deque`** — all mutating/query methods (`append`/`appendleft`/`pop`/`popleft`/`rotate`/…) and item assignment `dq[i]=v`. (Construction, read, iteration, `list/sum/sorted(dq)` already work.)
- **`OrderedDict`** — `move_to_end`, `popitem`.

### 11. Classes / OOP
- **`@abstractmethod`** and general method decorators.
- **Class decorators** — any `@deco` on a class (incl. `@runtime_checkable`).
- **`object` / `object.__new__(cls)`** and **`NotImplemented`** (see §5).
- **`abs()` on a user class** — the `__abs__` result is not statically typed, so a later `.attr` on it fails.

### 12. Typing / generics
- **Subscripted instance annotation** — `b: Box[int] = …` (the `Generic[T]` base parses; the `Name[T]` *annotation use* fails).
- **`Protocol` base class** — `unknown base class Protocol` (structural subtyping unsupported); also **`Protocol[T]`** subscripted base.
- **`zip()` with 3+ iterables** — exactly two are supported.

### 13. f-strings
- **Dynamic/nested format specs** — `f"{x:.{n}f}"`, `f"{x:{w}d}"`.
- **`!a` conversion** — `f"{x!a}"`, `f"{x=!a}"`. (`!r`/`!s`, debug `=`, static specs already work.)

### 14. Literals
- **Non-UTF-8 bytes literals** — any byte ≥ `\x80`, e.g. `b"\xff"` (this is what fails `test_file_io.py`; ASCII and `\x00`–`\x7f` already work).

---

## Known traps — the previous compiler already shipped every backlog item

Each construct above works in the previous compiler; these notes record where it
got them *wrong first* (every one is backed by a documented fix in its history).
Read the matching note before starting an item.

- **§1 kwargs — two traps.** (a) Python evaluates call arguments left-to-right
  *as written*; desugaring kwargs by reordering into the callee's positional
  order reorders side effects — add a side-effecting-args corpus probe. (b) A
  default expression that captures a free variable is evaluated in the *def*
  scope, once — the previous compiler had an SSA/capture bug exactly there.
- **§1 mutable defaults.** `lst=[]` is evaluated once at def time and shared
  across calls. The naive per-call evaluation diffs clean on everything except
  the aliasing probe (`f(1); f(2)` → `[1, 2]`) — put that probe in the corpus
  *first*. Needs a per-default static cell that is a GC root.
- **§2 `is` / `is not`.** Under fixnum tagging all equal small ints are
  bit-identical, while CPython caches only −5..256; identity of value types is
  implementation-defined in CPython anyway. Define `is` as bit-identity
  (heap pointer / fixnum / bool / None), keep corpus probes to the defined
  cases (`is True`, same-object, `type(x) is T`), and do not chase CPython's
  int cache.
- **§4 non-literal `range` step.** The previous compiler shipped
  `sum(range(a, b, step)) == 0` for a negative variable step — the loop desugar
  assumed an ascending direction. Unknown step ⇒ emit the runtime-direction
  comparison (`step > 0 ? i < stop : i > stop`) and `ValueError` on
  `step == 0`.
- **§5 `map`/`filter`.** The single item that birthed PITFALLS A4 in the
  previous compiler (parallel `rt_*_tagged` HOF variants, marker bits; `filter`
  additionally broke on an i8-truthiness callback ABI). Implement as lazy
  iterators over the uniform `fn(Value)→Value` callback first; unboxed-callback
  specialization is a separate, proof-gated item.
- **§5 `getattr`/`setattr`/`hasattr` (literal name).** Desugar in the frontend
  to direct attribute access so the dynamic-`getattr` out-of-scope boundary
  stays syntactic. For `hasattr` on a gradual receiver the previous compiler's
  design is reusable: a name-hash method/field registry probe (existence-only,
  no vtable lookup).
- **§6 `type()`.** DONE. `print(type(x))` / `str(type(x))` give the
  module-qualified `<class '__main__.Foo'>`, `type(x).__name__` the bare name, and
  the default instance repr is module-qualified again — all three from ONE metadata
  source as required: `rt_builtin_type` formats the `<class '...'>` string (builtin
  tag or registered qualname), and `.__name__` runs `rt_type_name_extract` over
  THAT string (a lowering peephole), never a parallel compile-time name table.
- **§8 numeric tower.** `int` is Tagged (fixnum-or-bignum) and `float` is
  `Raw(F64)`, so int→float at a slot is a real `legalize` coercion with a
  bignum arm (precision loss above 2⁵³ matches CPython's `float(int)`) — never
  a noop. On the typeck side make `int ⊔ float = float` a deliberate lattice
  rule; the previous compiler repeatedly leaked these joins to `Any` instead.
- **§9 `str.format` + §13 dynamic f-string specs.** The same mini-language.
  Both must call the one `format-shared::parse_format_spec` engine (a dynamic
  spec becomes a runtime call into it); the previous compiler's repr/format
  drift came from duplicated formatting paths.
- **§10 `deque`.** Its method names collide with `list` (`append`, `pop`, …);
  in the previous compiler that leaked wrong element-type constraints into the
  solver for look-alike receivers. Key §9–§11 method constraints by receiver
  `SemTy`, never by method name alone.
- **§10 `defaultdict(int)`.** A type used as a value. Don't grow first-class
  type objects for this — special-case the builtin factories
  (`int`/`list`/`dict`/`set` → zero-value thunks) plus user functions, as the
  previous compiler did.
- **§11 method/class decorators.** The root of the previous compiler's
  per-function-ABI saga was exactly here: `@property` getters with primitive
  returns got return-ABI-flipped, then needed flags and side-tables to track
  it. The immunizing rule: a function whose identity escapes through *any*
  decorator is address-taken ⇒ uniform Tagged ABI, decided in `typeck`, no
  per-function exceptions.
- **§11 `abs()` / dunder results.** Two lessons: dunder return types must
  enter the solver as ordinary constraints (post-hoc threading caused a
  bound-result SEGV in the previous compiler), and the `other` param of a
  binary dunder must not get `Self` blindly injected into its Union (that was
  the microgpt `loss=NaN` root cause).
- **§7 `isinstance`.** Accepting the syntax is the small half; the value is
  flow-sensitive narrowing in `solve`, and retrofitting narrowing late was a
  documented multi-pass cascade in the previous compiler — wire narrowing in
  together with the syntax. For the gradual receiver: `ObjHeader.type_tag`
  makes the runtime query one load — support it rather than carving an
  out-of-scope hole.
- **§3 `del name`.** `del d[k]` / `del lst[i]` are runtime calls; `del name`
  changes definite-assignment (a later read is `UnboundLocalError`). That is
  typeck/CFG work, not lowering work — scope it that way. (DONE: rather than add
  a definite-assignment CFG pass, the implementation took the unbound-sentinel +
  runtime read-guard route — correct on every path with no flow analysis. See §3.)
- **§14 non-UTF-8 bytes.** Keep `bytes` permanently out of the string
  interner/`StrObj`; one shared pool entry holding a byte ≥ `\x80` breaks the
  codepoint invariant behind `char_len`.

---

## Cross-cutting

- **Differential harness (the spine):** every corpus file is the spec. A feature
  isn't done until its corpus entry diffs clean vs CPython; close a backlog item
  by lifting the relevant `test_*.py` onto the gate allowlist.
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
  `random`/`hashlib`/file I/O, so closing it is mostly `stdlib-defs`
  descriptors (Principle 8) — but only with corpus probes added per module.
  Never declare a module supported on the strength of inherited runtime code
  alone.
