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
~13 aspirational `corpus/test_*.py` files exercise valid Python 3 the compiler
does not yet accept. (The §9 str-method batch lifted four already-clean files —
`test_future_annotations.py`, `test_gc_simple.py`, `test_generators.py`,
`test_print_output.py` — onto the gate at ~0 code, locking in working behavior.)
The full, deduplicated inventory of those gaps is the
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
| ~~**Attr/subscript `for`-targets + non-literal `range()` step**~~ — DONE (backlog §4) | 1 | ~~`unsupported for-loop target` / `range() step must be an integer literal`~~ |
| ~~**`type()` builtin** (incl. `type(x).__name__`)~~ — DONE (§6); ~~`hash()`~~ — DONE (§6) | 3 | ~~`builtin Type not supported in Phase 2`~~ |
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
  `str.split`/`str.encode`/`str.replace` now exist (§9) but are **positional-
  only** — the kwargs gate rejects `s.split(sep=",")` with a clean diagnostic
  (no keyword params on non-class methods except `.sort`). `str.format(name=)` is
  now handled out-of-band: a literal-receiver `"...".format(...)` desugars in the
  frontend (§5/§9/§13), binding its keyword fields to args at compile time, so the
  keyword-less method gate is never reached. The kwargs mechanism is ready when
  other surfaces grow keyword parameters. Caveats: `.sort(key=K)` desugars by
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
- ~~**`**d` spread into a call**~~ — DONE (backlog §1): `f(**{"a":1})`, `f(a=1, **d)`,
  `f(**make_dict())`. Entirely front-half (`lower_direct_known_call`): a `**{literal}`
  dict (string-literal keys) flattens into keyword args at compile time (reusing the
  slot-matching path, with CPython `got multiple values` duplicate detection); a
  non-literal `**d` (variable / call result) is evaluated ONCE and each named
  parameter bound from it at run time — `dict[name]` (`DictGet`) for a required param,
  `dict.get(name, default)` (`DictGetDefault`) for a defaulted one. Combining a runtime
  `*` spread with `**`, and a runtime `**d` into a `**kwargs` callee, stay out of scope
  (loud errors); `**` into a method call / constructor is still rejected. Documented
  gaps (the static callee shape can't see a runtime dict's contents): an unexpected key
  in the dict, and a key colliding with an explicit positional/keyword, are not
  diagnosed at run time. Gated by `corpus/p37_kwargs_spread.py`.
- ~~**Mutable default parameter**~~ — DONE (backlog §1): `def f(x, lst=[])`, `d={}`.
- ~~**Non-literal default**~~ — DONE (backlog §1): `def f(count=5+5)`. Both realized as a
  synthetic GC-rooted promoted-global slot per non-literal default of a **top-level**
  `def`: evaluated ONCE at the def's module-init position (CPython def-time
  once-evaluation, in module scope so no captures) and read — shared — at every
  defaulted call, so a mutable default is the SAME object reused (`f(1); f(2)` → shared
  list). `ParamDefault::{Const,Slot}` in `hir`; the slot read coerces into the param
  repr at the existing fill seam (no new pass/Repr/coercion site). Non-literal defaults
  on nested defs / methods / decorated defs / generators / lambdas are a clean error
  (top-level-only — a process-global slot cannot hold a per-closure-instance capture).
  Gated by `corpus/p36_mutable_defaults.py`.
- ~~**`Dyn → : bool` annotated-slot checked unbox**~~ — DONE (test_functions.py lift,
  Phase 1): `rt_unbox_bool` completes the checked-unbox family as the third shape
  (`Tagged → Raw(I8)`, B18-sanctioned — the guard B18 demands). A gradual value into a
  `: bool` local takes the CHECKED unbox (TypeError on a non-bool tag) instead of a
  blind `UntagBool`. `new_checked`/the MIR verifier/`coerce_value`/the `Assign` gate
  admit `Raw(I8)`; `check_reinterpret` adds a **Dyn-only** bool arm (admitting
  `Int`/`Bool` would diverge from CPython — `3 == True` is `False`). Gated by
  `corpus/p38_unbox_bool.py` (success path; the wrong-shape guard can't byte-match
  CPython at an annotation seam — CPython ignores annotations — so it rides a runtime
  unit test).
- ~~**Closure/lambda VALUES typed `Callable(sig)`**~~ — DONE (test_functions.py lift,
  Phase 2 b1): a lambda / nested `def` used as a value carries its static
  `Callable(sig)` (visible param/return reprs from the lowered function) instead of
  `Dyn`, so binding it to a name and calling that name rides the existing native
  `CallIndirect` ABI (no new runtime; `closure_sem_ty` mirrors `repr_of`, and the
  raw-int proofs are gated off for any address-taken function, so the closure ABI stays
  the Tagged baseline). Gated by `corpus/p39_closure_values.py`.
- ~~**`corpus/test_functions.py` stays OFF.**~~ — LIFTED by the **uniform value-call
  convention** (the sound root fix for `Dyn` callees). Its last out-of-scope root was
  a genuinely-`Dyn` callee that is a *native* closure (a curried chain
  `chain(1)(2)(3)` whose intermediate return widens to `Dyn`; an unannotated
  decorator's `func()` and its slot). The prior two value-call mechanisms (the
  Phase-6D decorator generic `(tuple, dict)` thunk and the Phase-6A typed
  top-level-fn-value thunk) are replaced by **one**: every function that can become a
  closure value gets an **arity-generic uniform entry** `F.<uniform>(env, args_tuple,
  kwargs_dict) → Value` as its slot 0, `repr_of(Callable) → Closure(GENERIC_SIG)` for
  ALL closures, and **every** indirect call routes through that single ABI
  (`lower_indirect_call` packs the positional args into a tuple + null kwargs;
  `CallValue` pre-packs a `*seq`/`**dict` forward). The uniform thunk does the
  runtime arg→param bind (positional / defaults / `*args` / the Phase-1 **checked**
  float/bool unbox via `bind_arg_checked`) then makes ONE **direct** call to `F`
  (specialized native ABI — the hot path is untouched). So a fixed-arity native
  closure is bound correctly, not corrupted; Principle 2 becomes literally true for
  callables (the precise `Callable` sig demotes to an optional devirtualization
  *hint*); and the closure-leaked-through-`Dyn` hole closes. A non-callable `Dyn`
  callee raises `TypeError` at run time (`rt_call_check` guards slot 0; a
  statically-known non-callable is still a loud compile error). Closures carry a
  **distinct `TypeTagKind::Closure`** (same `TupleObj` layout, allocated by
  `rt_make_closure`; GC traces it like a tuple), so a *data* tuple is never
  mistaken for a callable — `rt_call_check` accepts only the `Closure` tag, closing
  the former `(1, 2)()` / `Dyn`-holding-a-data-tuple SEGV (it was newly reachable
  once `Dyn` callees stopped being a compile error). Gated by
  `corpus/p41_call_guard.py`. A pre-existing
  inliner bug rode along the file's `_test_mixed_value_void` probe — a value-returning
  callee's bare-`return`/fall-off (`Return(None)`) left the call's `dst` STALE under
  `-O` instead of writing the default return value; fixed in `optimizer::inline`
  (`emit_default_ret`, mirroring codegen's `default_ret`). Byte-matches CPython
  end-to-end (debug + release).
  - ~~Deferred: a value call *carrying* keyword args into a keyword-only / `**kwargs`
    closure.~~ — DONE: `lower_indirect_or_unknown_call` now accepts `name=value` and
    `**d` on a value-position call (`build_indirect_kwargs` builds the keyword dict
    from named entries + `**d` merges in source order → the `kwargs` slot of
    `CallValue`); the callee's uniform thunk **normalizes** the null `__kwargs__`
    sentinel (no-keyword common path → no alloc) into a fresh empty dict via
    `kd = {}; kd.update(__kwargs__)` (`rt_dict_update` is null-tolerant), so kwonly
    binding (`dict.get`/`dict[]`) and `**kwargs` forwarding never dereference null.
    Keywords reach the closure's keyword-only / `**kwargs` params; binding a
    *positional* param BY keyword through a value call stays out of scope (the
    positional slots are still matched positionally — a loud arity error, not a
    crash). Gated by `corpus/p40_value_call_kwargs.py`.
  - ~~Devirtualization of monomorphic indirect calls back to the specialized ABI is
    a separate additive optimizer pass.~~ — DONE (the deferred Phase E): the new
    `optimizer::devirt` pass (registered right after `inline::Inline`) rewrites a
    monomorphic value-position `CallIndirect` — one whose callee resolves through
    single-def bit-identity `Coerce` links to a `MakeClosure { func: thunk }` over a
    *simple positional* uniform thunk (exactly one `Call` to the specialized `F`, no
    varargs/`**kwargs`/`Dict*` machinery) — back to a direct `Call(F, [env?, args…])`,
    recovering the positional args from the now-dead args-tuple builder (non-escape
    proven by read-counts) and re-boxing F's non-`Tagged` return. It restores the
    pre-uniform specialized call (`Call{step,[env,v,w]}` for `bench_calls`), recovering
    the ~16% value-call regression (`bench_calls` 0.658s → 0.583s, 1.41x → 1.57x; see
    `benchmarks/results.md`). Strictly additive: any unmet precondition leaves the
    `CallIndirect` on the always-correct uniform path; no new ABI, no marker bits (A4);
    the MIR verifier (debug+release) + the byte-exact differential gate are the backstop.
    A companion `inline` change keeps it from inlining INTO uniform thunks (every
    `MakeClosure` target) — a thunk is the thin COLD-fallback shim, and inlining its
    callee `F` into it both bloats that fallback and severs the thunk→`F` link `devirt`
    needs. `CallValue` (spread/keyword) and varargs/kwargs/default closures stay on the
    uniform path (v1 scope).
  - Still deferred: the heap-arg seam (`list`/`str` param of a `Dyn` callee) keeps
    the existing `TaggedToHeap` trust.

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
- ~~**Walrus `:=`**~~ — DONE: `(target := value)` (PEP 572) lowers in the frontend
  (`lower_named_expr`) — evaluate `value` once, bind the bare-name `target` in the
  CONTAINING scope through the ordinary write/read place machinery (local / captured
  cell / promoted module-global via `resolve_write_place`), and yield the assigned
  value. So a name bound in an `if`/`while`/comprehension test is visible after the
  statement, exactly as CPython (the comprehension walrus leaks to the enclosing
  scope; `freevars` already recognized `NamedExpr` targets for closure capture). No
  new HIR/typeck surface. A `+True`-yields-int divergence (`rt_obj_pos` returned the
  bool unchanged; now promotes to int like `rt_obj_neg`) rode along. Gated by
  `corpus/p26_walrus.py`; **`test_control_flow.py` is now LIFTED** (walrus was its
  sole remaining blocker).
- ~~**Matrix-multiply `@` / `__matmul__`**~~ — DONE: no built-in numeric `@`, so
  `a @ b` lowers to a new `BinOp::MatMul` → tagged `rt_obj_matmul`, which dispatches
  the user `__matmul__`/`__rmatmul__` dunder (or raises `TypeError`) — the SAME
  runtime-dunder path as `+`/`*` (`rt_obj_add`/`rt_obj_mul`), no per-op frontend
  dispatch. typeck types the result as `__matmul__`'s declared return (via
  `class_dunder_ret`), so attribute access on a matrix product resolves; non-class
  operands type to `Dyn`. `@=` falls back to `__matmul__` (the convention `+=` uses
  for `__add__`; in-place `__imatmul__` is the same pre-existing gap as `__iadd__`).
  Threaded through both `BinOp` enums (hir/mir), `map_binop`, codegen dispatch +
  `rt_obj_matmul` decl, the interval/may-raise/constfold matches, and `FNV_MATMUL`/
  `FNV_RMATMUL`. Gated by `corpus/p27_matmul.py`. This was §2's last operator gap.

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
  `test_iteration.py` is now **LIFTED** — its blockers fell in sequence:
  attribute/subscript `for`-targets (`corpus/p22`), the standalone `iter()` builtin
  + container `isinstance` (`p23`), `functools.reduce` (`p24`), and finally
  lexicographic `min`/`max`/`sorted` over tuples + dynamic `list`/`tuple`/`bytes`
  concatenation (`p25`, a runtime fidelity fix — `rt_obj_cmp`/`compare_list_elements`
  now route a `Tuple` operand to the lexicographic `tuple_cmp_ordering`, and
  `rt_obj_add` handles same-type sequence concat through the gradual `+` path).
  `test_collections_list_tuple.py`
  is now **LIFTED** (§9: its earlier blocker — a tuple SLICE result, typed
  variable-length `tuple[T, ...]` by `slice_ty`, assigned into an annotated
  fixed-arity `tuple[T, …]` slot — was FIXED via the repr-contract check
  (`check_reinterpret`) admitting a `tuple`→`tuple` store when element `Repr`s
  match per index, `corpus/p15_tuple_slice_slot.py`; then `tuple.index`/`.count`
  and finally `list.remove` closed the remaining blockers — see §9).
- ~~**Attribute / subscript as a `for` target**~~ — DONE: `for obj.attr in …`
  (→ `SetAttr`), `for lst[i] in …` (→ `SetItem`), and mixed tuple targets
  (`for p.x, p.y in …`). `bind_for_target` now delegates the supported shapes to
  `assign_to_target` — byte-identical on `Name`/`Tuple`/`List`, and the
  attribute/subscript leaves reuse the existing `SetAttr`/`SetItem` arms (the same
  path nested destructuring uses). Entirely front-half, no new HIR/typeck surface.
  Gated by `corpus/p22_loop_targets.py`.
- ~~**`range()` for-loop with a non-literal step**~~ — DONE: `range(10,0,-(-1))`, a
  variable step, computed `range(0,10,1+1)`. `lower_for` takes the Phase-3c raw-i64
  fast path ONLY for a simple-`Name` target with a compile-time-literal step
  (`range_step_is_literal`); everything else routes to the general iterator path,
  which drives the runtime `RangeIter` (correct direction at runtime). The runtime
  `rt_iter_range` now raises `ValueError: range() arg 3 must not be zero` on
  `step == 0` (CPython fidelity — fixes both the for-loop general path and the value
  form `list(range(0,5,0))`). The §4 trap (a negative VARIABLE step must NOT collapse
  to `sum == 0`) is handled by reusing the proven `RangeIter` direction logic, not a
  hand-emitted compile-time branch. Gated by `corpus/p22_loop_targets.py`.

### 5. Builtins — `undefined name`
- ~~**`pow`, `divmod`, `all`, `any`, `id`, `round`, `bin`, `hex`, `oct`**~~ —
  DONE. Recognized by name in the frontend (like `sum`/`min`/`max`), gated on the
  name being UNSHADOWED. Two shapes: **pure desugar** (`pow` → `**`/`BinOp::Pow`,
  bignum- & numeric-tower-correct incl. negative-exponent→float; `divmod` → a
  staged `(a // b, a % b)` 2-tuple, CPython floor/sign via `rt_obj_floordiv`/
  `mod`, B1; `all`/`any` → an iterator loop with a truthiness short-circuit,
  empty→seed, result `Bool`) and **declarative `CallRuntime`** (`id` wraps the
  existing `rt_id_obj` → `Raw(I64)` address, never a GC root; `round` →
  `rt_builtin_round` banker's, round-half-to-even via decimal formatting so
  `round(2.675,2)==2.67`, presence-of-`ndigits` switches int↔float result;
  `bin`/`hex`/`oct` → BIGNUM-AWARE `rt_builtin_bin`/`hex`/`oct` taking a TAGGED
  `Value` — never the raw-`i64` `rt_int_to_*` formatters — so `bin(2**100)` is
  exact, **PITFALLS B16**). Descriptors in `stdlib-defs/src/modules/builtins.rs`
  (bare builtins, no module registry). Gated by `corpus/p18_scalar_builtins.py`;
  the lift `corpus/test_core_types.py` (its sole §5 blocker was `round`) is now
  on the gate too. **Out of scope** (unprobed): 1-arg `pow(x)` and the 3-arg
  modular `pow(a,b,m)` (both `parse_error`); negative-`ndigits` correctness for
  `round` (naive scaling) and the |float|>i64 → bignum corner (implemented via
  `BigInt::from_f64`, unprobed).
- ~~**`functools.reduce`**~~ — DONE: a higher-order builtin, but desugared in the
  frontend to a compiled accumulator loop calling `func(acc, elem)` each iteration
  (mirroring sum/min/max/all/any), NOT the raw-ABI `rt_reduce` callback path (the
  PITFALLS A4 anti-pattern — and the substrate's `rt_reduce` 6-arg ABI never matched
  the 3-arg generic stdlib dispatch, so the descriptor fallthrough SIGSEGV'd). The
  reduction callable rides the ordinary indirect-call machinery (lambda / capturing
  lambda / named def). Seeds from `initial` if given, else the first element (empty
  without initial → `TypeError`, CPython wording). Intercepted in both the bare
  (`from functools import reduce`) and qualified (`functools.reduce`) dispatch via
  `is_reduce_def`. Gated by `corpus/p24_reduce.py`. This shows `map`/`filter` should
  follow the SAME lazy-iterator/compiled-loop shape, never the `rt_*_tagged` HOF
  variants of the previous compiler.
- ~~**`iter()`**~~ — DONE: the standalone 1-arg `iter(iterable)` builds a runtime
  iterator object via the same `ContainerOp::Iter` → `rt_iter_value` the for-loop
  drives (so a File iterable routes through `rt_file_readlines` in lowering too);
  `next(it)` (already wired) consumes it via the raising `rt_iter_next`
  (StopIteration on exhaustion). Wired next to `next`/`sum`/`set` (recognized by
  name; shadowing unsupported). The 2-arg sentinel form `iter(callable, sentinel)`
  is out of scope. Gated by `corpus/p23_iter_isinstance.py`.
- ~~**`map`/`filter`**~~ — DONE: the next HOFs after `reduce`, following the SAME
  shape. PURE FRONTEND desugar (`lower_map`/`lower_filter`) to an EAGER compiled
  loop calling the callback per element through the ordinary uniform-tagged
  indirect-call machinery, materializing into a `list`, then wrapping it in
  `iter(...)` so `for`/`list`/`next`/`sum` consume it:
  `map(f, xs) ~= iter([f(x) for x in xs])`,
  `filter(f, xs) ~= iter([x for x in xs if f(x)])`,
  `filter(None, xs) ~= iter([x for x in xs if x])` (element truthiness). This
  AVOIDS the runtime `rt_map_new`/`rt_filter_new`/`IteratorKind::Map/Filter`
  lazy-iterator HOF machinery — the PITFALLS A4 anti-pattern (parallel calling
  convention, marker bits, `i8` predicate ABI). Builtin callbacks
  (`map(str, …)`/`map(len, …)`/`map(abs, …)`) resolve through normal
  `Symbol`-dispatch with NO extra code (the `min(…, key=len)` mechanism). `f` is
  staged ONCE (CPython single function evaluation); the eager-vs-lazy side-effect
  timing is observationally invisible on the finite/pure corpus (the
  `lower_sum`/`reduce` materialization precedent). Intercepted in the
  UNSHADOWED-name builtin block, so a user `map = …` binding wins. **Scope limit**:
  single-iterable only — multi-iterable `map(f, xs, ys)` needs `zip` (§12). Gated
  by `corpus/p28_map_filter.py`. **Runtime contract evolved**: the probe's
  `filter(None, list-elements)` case surfaced a pre-existing latent bug —
  `rt_list_eq`/`rt_tuple_eq` compared elements via the hashable-key
  `eq_hashable_obj`, which falls back to POINTER identity for non-hashable types,
  so `[[1]] == [[1]]` was wrongly `False`; both now compare elements via the full
  structural `rt_obj_eq` (with a CPython `x is y or x == y` identity
  short-circuit), so nested lists/dicts/sets compare by value.
- ~~**`format`/`ascii`**~~ — DONE (the full PEP-3101 mini-language, §5/§9/§13 in
  one stroke). `format(v[,spec])`, `str.format()`, f-string fields, and dynamic
  specs (`f"{x:.{n}f}"`) ALL desugar in the FRONTEND to one node —
  `FormatValue { value, spec }` (`spec` is now an `Idx<HirExpr>`, not an interned
  literal) — backed by the existing `rt_format` (the `format-shared` PEP-3101
  engine). No new runtime parser. `"...".format(...)` on a literal receiver parses
  to literal `StrLit`s + per-field `FormatValue` joined by `+` (the f-string tail),
  binding fields to pos/kw args at compile time. `ascii` is now a first-class
  builtin (`rt_builtin_ascii` → the value-level ascii dispatcher), wiring both the
  `ascii()` builtin and the f-string `!a` conversion. Runtime contract evolved
  (Principle 8): `rt_format` gained a class-instance arm (user `__format__`, else
  `object.__format__` → empty-spec `str(self)` via a new `try_str_dunder`); and
  `format_bool` was corrected to CPython (bool inherits `int.__format__`, so a
  non-empty spec formats the int 1/0 — `f"{True:5}"` == "    1", NOT " True"; the
  test file's stale assertion was fixed to the live oracle). Gated by the lifted
  `corpus/test_format_spec.py` + `corpus/p29_format.py`. See §9/§13 below.
- ~~**`getattr`/`setattr`/`hasattr`/`issubclass`**~~ — DONE (the §5 introspection
  set, ZERO runtime changes — all collapse onto existing machinery, exactly the
  `isinstance` template). `getattr(o,"x")` ≡ `o.x` and `setattr(o,"x",v)` ≡
  `o.x=v` are pure FRONTEND desugars onto the existing `Attribute` read /
  `SetAttr` write (static `GetField`/`SetField` for a concrete receiver; a `Dyn`
  receiver rides the gradual `GetFieldNamed`/`SetFieldNamed` →
  `rt_getattr_name`/`rt_setattr_name` path for free). `hasattr(o,"x")` and
  `issubclass(A,B)` are two new compile-time-`Bool` HIR nodes (`HasAttr`,
  `IsSubclass`) folded in `lowering` to `Const::Bool` — `hasattr` from the
  receiver's `ClassInfo` (field/method/property/static-/class-method/class-attr),
  `issubclass` via `ClassTable::is_subclass` (the C3-MRO check) — just like
  `IsInstanceBuiltin`. Unshadowed-gated (a user `def getattr(...)` still wins).
  Scope limits (clean compile errors): dynamic `getattr(o, name_var)`
  (non-literal name), `getattr` 3-arg default, `hasattr` on a `Dyn`/non-class
  receiver, `issubclass` with a builtin-type (`issubclass(bool,int)`) or tuple
  second arg. Gated by `corpus/p30_introspection.py`. One of the four blockers
  that, together with the multi-`zip` (§12), int-method (§9), and `hash`/zero-arg-
  `int`/`int(str,base)` (§6) fixes, finally **LIFTED `test_builtins.py`** onto the
  gate (its blocker chain fell across every phase, each fix unmasking the next).
- **Still pending**: `object` (`object.__new__`), `NotImplemented` (these belong
  to the class-OOP cluster that `test_classes.py` needs — gated by
  `@abstractmethod`, not part of the introspection step).

### 6. Builtins — `Phase 2 codegen not supported`
- ~~**`type()`**~~ — DONE: incl. `type(x).__name__`, `str(type(x))`,
  `print(type(x))`. Gated by `corpus/p17_type_builtin.py`. A "type object" IS its
  repr StrObj (`type(x)` → `rt_builtin_type` → `<class '...'>`); `.__name__` is a
  lowering peephole through `rt_type_name_extract` (same runtime string, bare last
  segment). Out of scope (unprobed divergences): `type(x) is T` / `type(x) is
  type(y)` (pointer-identity on distinct StrObjs; `p11_is_identity.py` already
  defers it) and `repr(type(x))` (would add quotes). `test_builtins.py` is now
  **LIFTED** (all its blockers closed — see below); `test_classes.py` stays OFF
  (the `@abstractmethod` wall, §11).
- ~~**`hash()`**~~ — DONE: a `K::Hash` codegen arm wiring the pre-existing
  `rt_builtin_hash` (which already returns the right tagged-int hash for
  int/bool/str/float/tuple via `rt_hash_*`). One fix: `hash(None)` now returns
  CPython 3.12's fixed `0xFCA86420` (the builtin must be non-zero; the dict-key
  hashing path `hash_hashable_obj` keeps its own 0 for bucket placement). The
  `builtin_fn` match is now exhaustive (all 12 kinds wired). Gated via
  `test_builtins.py`. (Bignum `hash()` still raises "unhashable" — a pre-existing
  limit, not exercised; CPython's `_PyHASH_MODULUS` folding is a later add.)
- ~~**zero-arg `int()`/`float()`/`bool()`/`str()`**~~ — DONE: folded to their
  default constants (`0`/`0.0`/`False`/`""`), never an arity-mismatched unary
  `rt_builtin_*` call (which built invalid Cranelift IR). `int`/`float`/`bool`
  fold in lowering; `str()` folds to a `""` literal in the FRONTEND (the mutable
  interner lives there — lowering's is immutable), unshadowed-gated so a user
  `str` binding still wins. Other zero-arg builtins get a clean error, not
  invalid IR. Gated by `corpus/p33_zero_arg_conversions.py` (+ `test_builtins.py`
  for int/float/bool).
- ~~**two-arg `int(str, base)`**~~ — DONE: routed to the (pre-existing)
  `rt_str_to_int_with_base`, whose descriptor was corrected from `binary_to_i64`
  (`[Tagged, Tagged]`) to `[Tagged, Raw]` so the base rides a raw i64, not its
  tagged bits. Handles `0x`/`0b`/`0o` prefixes. Gated via `test_builtins.py`.

### 7. `isinstance`
- ~~**Tuple of types** — `isinstance(x, (int, str))`~~ — DONE: a pure frontend
  desugar to an `or` of the existing per-element checks — `IsInstance` (runtime,
  user classes) and `IsInstanceBuiltin` (static fold, builtins) — over a receiver
  staged ONCE (CPython single-eval), combined with the same short-circuit CFG as
  `lower_boolop`. Nested type-tuples flatten recursively; the empty tuple is
  `False`. ZERO new HIR / typeck / lowering / runtime change. A builtin-type
  element crossed with a `Dyn`/`Union` receiver raises the same loud "requires a
  statically-typed value" error as the single-type builtin fold (gradual receiver
  is out of scope, below); a tuple element that is not a class / builtin-type
  *name* is a clean error (matching single-type strictness). Gated by
  `corpus/p34_isinstance_tuple.py`.
- ~~**Container targets** — `isinstance(x, list/dict/tuple/set)`~~ — DONE: the
  builtin-isinstance static fold now matches container targets by KIND (element
  types are irrelevant to isinstance — a `list[int]` value satisfies
  `isinstance(x, list)`; a fixed `tuple[A,B]` and a variable `tuple[T,...]` are both
  `tuple`), alongside the existing `str|int|float|bool|bytes`. Frontend maps
  `list`/`dict`/`set`/`tuple` to a canonical Dyn-element target; `lower_isinstance_builtin`
  compares via the `list_elem`/`dict_kv`/`set_elem`/`tuple_elems`/`tuple_var_elem`
  accessors. Gated by `corpus/p23_iter_isinstance.py`.
- **Gradual/`Any` receiver** — "runtime type query on a gradual value is out of scope" (decide: support via a runtime tag query, or keep out-of-scope and document).

### 8. Numeric tower (int↔float)
- ~~An `int`/widened local returned through `-> float`; a literal `return 0` in a `-> float` function; an unannotated mixed `return 1.5 / return 0` inferred as `Any` and rejected at a `float` slot.~~ — DONE for the two `Raw(F64)`-slot seams: **return through `-> float`** and **assignment to an annotated `float` LOCAL** (incl. a `__main__` top-level local). int / bool / gradual `Dyn` are accepted at these seams via a new `allow_numeric_coerce` gate in `check_reinterpret`; the coercion lands at the store as a CHECKED `Tagged → Raw(F64)` unbox (`coerce_value` helper → `rt_unbox_float`, now with a `BigInt` arm for `float(huge_int)` → round-to-nearest, ±inf on overflow). The annotation is a *contract* (CPython keeps the raw int), so the divergence is observable only via repr-print — gated by `corpus/p16_numeric_tower_float.py` (asserts via `==`, prints only float-forced results; covers int/bool returns, a `: float` local from int, a `Dyn` mixed return into a float local, the `2 ** 62` BigInt arm, and a `sum`-over-floats interaction). Untouched: `is_subtype_of` (covariant for generics — a global `int<:float` would unsoundly admit `list[int] <: list[float]`), `numeric_promote`, `raw_uniform`. (bool↔int promotion already worked — this closes int↔float.)
  - **Deferred sub-items** (kept rejected, no liftable corpus needs them):
    - **`float` GLOBAL / FIELD slots** (a genuine cross-function `x: float` global; `self.v: float`). Physically *tagged* slots that unbox on READ via an *unchecked* `UnboxFloat` (stores coerce to plain `Tagged`), so accepting an int there would later misread (PITFALLS A2). Needs a store-side coerce-to-float-then-box so the slot holds a genuine `FloatObj` — a separate, larger change.
    - **Passing int/bool to a `float` PARAMETER** (free-fn / method / ctor / dunder). The method/ctor/dunder arg seams pass `(loc, repr)` without a per-arg `SemTy`, so `needs_check` can't be evaluated without threading types through. Kept rejecting to avoid an accept-then-SEGV.

### 9. Methods on builtin types
- ~~**`int`**: `bit_length`, `bit_count`, `conjugate`, `__index__`~~ — DONE.
  Dispatched on an int/bool receiver in `lower_method_call` (`lower_int_method`),
  typed `→ Int`. The pre-existing `rt_int_bit_length`/`rt_int_bit_count` were
  rewired from a raw-`i64` ABI to a tagged `Value` that `classify_num`-splits
  fixnum vs heap `BigInt` (the B16 hazard `bin`/`hex`/`oct` solved the same way —
  `BigInt::bits()` / `BigUint::count_ones()`). `conjugate`/`__index__` return the
  receiver's int value via the new `rt_int_index` (bool → int 0/1, bignum
  preserved), so a bool receiver is **Int-typed** — avoiding the i8-vs-i64
  verifier clash a naive bool pass-through would hit. Gated by
  `corpus/p32_int_methods.py` (+ the lifted `test_builtins.py`).
- **`str`**: ~~`split`, `rsplit`, `splitlines`, `replace`, `lstrip`/`rstrip`,
  `removeprefix`/`removesuffix`, `expandtabs`, `partition`/`rpartition`,
  `rindex`, `encode`, predicates `isdigit`/`isalpha`/`isalnum`/`isspace`/
  `isupper`/`islower`/`isascii`~~ — DONE (§9 runtime-ready batch:
  `corpus/p19_str_methods.py`). Declarative `StrPlan` wiring of runtime fns
  whose impls + core-defs descriptors already existed; `maxsplit`/`tabsize`
  retyped to a RAW i64 MIR slot (B16); an explicit `None` sep/chars lowers to
  the null "default" sentinel (not `NONE_TAG`, which the runtime would
  mis-deref). **Scope limits (unprobed):** positional-only (the kwargs gate
  rejects `s.split(sep=",")`); `replace` has no `count` (runtime is 2-arg);
  `splitlines` no `keepends`; `encode` ignores encoding/errors (always UTF-8);
  `find`/`index`/`rindex` take no `start`/`end`; predicates are **ASCII-only**
  (`is_ascii_*` — `"café".isalpha()` → `False` here vs CPython `True`).
  ~~`format`~~ is now DONE (the §5/§9/§13 mini-language — a literal-receiver
  `"...".format(...)` frontend desugar onto the shared `FormatValue`/`rt_format`
  path). (`upper`/`lower`/`strip`/`find`/`title`/`center`/`zfill`/`join`/…
  already worked.)
- **`bytes`**: ~~`startswith`, `endswith`, `find`, `rfind`, `count`, `replace`,
  `split`/`rsplit`, `strip`/`lstrip`/`rstrip`, `upper`/`lower`, `join`~~ — DONE
  (§9 runtime-ready batch: `corpus/p20_bytes_methods.py`). A bytes receiver
  routes to `lower_bytes_method`, the **exact sibling of `lower_str_method`** — a
  declarative `BytesPlan` table → the shared `emit_seq_method` (extracted from
  `lower_str_method` in this batch; no codegen edit — the runtime fn resolves by
  symbol). `maxsplit` rides a RAW i64 slot (B16, accepted by the `new`-default
  Raw slot — no descriptor retype, unlike str). `find`/`rfind` use dedicated
  2-arg runtime fns (no op_tag, unlike str's shared `rt_str_search`); the split
  family returns `list[bytes]`. **Scope limits (unprobed):** positional-only;
  `replace` has no `count`; the strip family takes **no `chars`** (whitespace
  only — the runtime is `ptr_unary`); `find`/`rfind` take no `start`/`end`;
  `decode` ignores its encoding (always UTF-8); `upper`/`lower` are ASCII-only
  (non-ASCII bytes pass through, matching CPython). The `in` operator on
  bytes-in-bytes (`b"a" in b"banana"`, subsequence membership) is **now wired**
  too — a runtime fix adding a bytes-needle branch to `rt_bytes_contains_value`
  (empty needle ⇒ True, like CPython); covered by `p20`.
- **`tuple`**: ~~`index`, `count`~~ — DONE (`corpus/p21_container_methods.py`).
  The `ContainerMethod::Index`/`Count` names now dispatch on a tuple receiver via
  the new `MethodRecv::Tuple` → `ContainerOp::TupleIndexOf`/`TupleCount`
  (value-comparing `rt_tuple_index`/`count`, B13; `index` miss → `ValueError`).
- **`dict`**: ~~`popitem`~~ — DONE (`corpus/p21_container_methods.py`):
  `ContainerOp::DictPopitem` → `rt_dict_popitem`, a fresh `(k, v)` 2-tuple (LIFO,
  matches CPython 3.7+; empty → `KeyError`). The tuple is a `Value`/`Tagged`
  GC-rootable result (B5), typed `Dyn`, so `k, v = d.popitem()` unpacks through
  the gradual seam (like `str.partition`). **`fromkeys` still pending** —
  deliberately OUT: a **classmethod** (`dict.fromkeys(keys, v)`), and even its
  instance form `d.fromkeys(keys, v)` does NOT fit the recv-first `MethodRecv`
  path — `rt_dict_fromkeys(keys, value)` **drops the receiver** entirely (the
  dict's contents are irrelevant), so it needs a distinct dispatch, not the
  uniform `(recv, args…)` ContainerOp signature.
- **`set`**: ~~`issubset`, `issuperset`, `isdisjoint`, `intersection_update`,
  `difference_update`, `symmetric_difference_update`~~ — DONE
  (`corpus/p21_container_methods.py`): comparisons → `ContainerOp::SetIsSubset`/
  `SetIsSuperset`/`SetIsDisjoint` (value-comparing `rt_set_*` → proven `Raw(I8)`
  bool, B13); the three `*_update` → `ContainerOp::Set{Intersection,Difference,
  SymmetricDifference}Update` (mutate in place via the void `rt_set_*_update`,
  None result). The new-set `symmetric_difference` (non-`update`) is **also
  wired** now → `ContainerOp::SetSymmetricDifference` → `rt_set_symmetric_
  difference` (Heap result). (`union`/`intersection`/`difference`/`|&-^` already
  worked.)
- **`list.remove(x)`** (not a §9-listed bullet, but the last lift blocker) — wired
  via `MethodRecv::List` → `ContainerOp::ListRemove` → `rt_list_remove`. Runtime
  fix: `rt_list_remove` now **raises `ValueError` on a miss** (was a silent
  no-op returning 0 — a CPython divergence); the i8 result is discarded (a
  `None`-returning mutation). Covered by `p21` + the lift below.
- **Lift status:** `test_collections_list_tuple.py` is **LIFTED** onto the gate —
  tuple.index/count was its first §9 blocker and `list.remove()` its last, both
  now closed; byte-matches CPython end-to-end.
  `test_collections_dict_set_bytes.py` advanced past `set.symmetric_difference()`
  (now wired) but is **NOT lifted** — it next hits `dict.fromkeys()` (the deferred
  classmethod: `rt_dict_fromkeys(keys, value)` drops the receiver, so it does NOT
  fit the recv-first `MethodRecv` path) and the `dict | / |=` merge operators
  (operator-level, a distinct feature beyond "methods on builtin types").

### 10. `collections` module
- ~~**`Counter`**~~ — DONE. Front-half WIRING over the pre-existing `counter.rs`
  (`Counter` shares `DictObj` layout under `TypeTagKind::Counter`), plus the
  runtime additions a differential-correct Counter needs (the original "pure
  wiring" estimate was short — `RuntimeObject`s aren't dict-iterable, subscript,
  repr, or truthy out of the box):
  - **Construction**: frontend intercept (mirrors the `reduce` intercept on the
    `COUNTER_NEW` sentinel `runtime_name`) picks `rt_make_counter_empty` (0-arg)
    vs `rt_make_counter_from_iter` (1-arg) and types the result
    `RuntimeObject(Counter)`. The three counting entry points
    (`from_iter`/`update`/`subtract`) now normalize any iterable to an iterator
    internally via `rt_iter_value_dyn` (so `c.update("ab")` works, not just
    iterators).
  - **Subscript**: `c[k]` → new `rt_counter_get` (missing key → boxed `0`, no
    KeyError, no insert); `c[k] = v` / `c[k] += n` → `rt_dict_set` (intercepts in
    `lower_subscript` / `lower_setitem` on the Counter tag).
  - **dict-family seam**: `len` / `in` / iteration / `keys`/`values`/`items` /
    truthiness all route through the generic tag-dispatched runtime
    (`rt_obj_len` already had it; added `Counter` to `rt_obj_contains`,
    `rt_iter_value_dyn`, `rt_is_truthy`, `rt_builtin_len`/bool). A new
    `debug_assert_dict_family!` guard replaces the `== Dict` seam guard on the
    `rt_dict_*` primitives the family shares.
  - **repr**: `Counter({...})` in most-common order (count desc, stable ties) via
    a new `counter_repr_string`, shared by stdout print and `str()`/`repr()`.
  - **methods**: `.most_common()/.total()/.update()/.subtract()` dispatch via the
    object-type registry; `keys`/`values`/`items` added there → `rt_dict_*`.
    `most_common()` uses an `i64::MIN` no-arg sentinel (= all) distinct from an
    explicit `most_common(0)`/`(-1)` (both `[]`) — fixed `lower_runtime_object_method`
    to emit a method's DECLARED optional default instead of a hardcoded `0` (also
    fixes latent `deque.rotate()`/`OrderedDict.popitem()` defaults).
  - **annotation**: `Counter` is an annotatable param/return type (import-gated in
    `named_annotation`; a user `class Counter` still wins).
  - Out of scope (documented, clean): `Counter(mapping)` / `Counter(**kwargs)`
    (would count keys), Counter arithmetic (`c1 + c2`, `&`, `|`), `.elements()`.
  - Gated by `corpus/p35_counter.py` (byte-matches CPython, debug **and** release).
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
- ~~**`zip()` with 3+ iterables**~~ — DONE. The runtime already had
  `rt_zip3_new`/`rt_zipn_new` + the Zip3/ZipN iterator objects (kind-dispatched
  `rt_iter_next`); only the front-half was wired for 2. `zip(a,b,c,…)` (N≥3) now
  lowers to a fresh runtime list of the N `iter()`-wrapped sources +
  `rt_zipn_new(list, count)` (one new `ContainerOp::ZipN`, ABI `[Val, Idx]`,
  Heap result), and typeck infers the element as a fixed-arity `tuple[…]` (one
  type per iterable), so `list(zip(xs, ys, zs))` types as `list[tuple[X,Y,Z]]`
  and fills an annotated container slot. The 2-iterable `rt_zip_new` path is
  unchanged. Gated by `corpus/p31_zip_multi.py`. (No runtime change — the
  substrate was already complete.)

### 13. f-strings
- ~~**Dynamic/nested format specs** — `f"{x:.{n}f}"`, `f"{x:{w}d}"`.~~ DONE — the
  field's `:spec` is itself a `JoinedStr`, so a dynamic spec lowers through the
  ordinary f-string concat (a literal spec collapses to a `StrLit`). See §5
  `format`.
- ~~**`!a` conversion** — `f"{x!a}"`.~~ DONE — wraps the value in an `ascii()`
  call (now a first-class builtin), exactly like `!r`→`repr()`. Also fixed: a bare
  `f"{p}"` now routes a class instance to `__format__`/`__str__` (was `str(x)`,
  which skipped `__format__`). Still pending: the PEP-501 debug `=` self-documenting
  f-string (`f"{x=}"`, `f"{x=!a}"`) — blocks `test_strings.py`.

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
- ~~**§1 mutable defaults.**~~ DONE. `lst=[]` is evaluated once at def time and
  shared across calls. The naive per-call evaluation diffs clean on everything
  except the aliasing probe (`f(1); f(2)` → `[1, 2]`) — that probe is now the
  first assert in `corpus/p36_mutable_defaults.py`. Realized as a per-default
  GC-rooted promoted-global slot (`ParamDefault::Slot`), set once at the def's
  module-init position and read shared at every defaulted call.
- **§2 `is` / `is not`.** Under fixnum tagging all equal small ints are
  bit-identical, while CPython caches only −5..256; identity of value types is
  implementation-defined in CPython anyway. Define `is` as bit-identity
  (heap pointer / fixnum / bool / None), keep corpus probes to the defined
  cases (`is True`, same-object, `type(x) is T`), and do not chase CPython's
  int cache.
- **§4 non-literal `range` step.** — DONE. The previous compiler shipped
  `sum(range(a, b, step)) == 0` for a negative variable step — the loop desugar
  assumed an ascending direction. Fix: route a non-literal/computed step to the
  general iterator path over the runtime `RangeIter`, whose next/exhausted logic
  already decides direction at runtime (verified for negative steps), instead of a
  hand-emitted compile-time direction branch; `rt_iter_range` raises `ValueError`
  on `step == 0`. Gated by `corpus/p22_loop_targets.py`.
- ~~**§5 `map`/`filter`.**~~ DONE (see the §5 Builtins entry above). The single
  item that birthed PITFALLS A4 in the previous compiler (parallel `rt_*_tagged`
  HOF variants, marker bits; `filter` additionally broke on an i8-truthiness
  callback ABI). Implemented NOT as runtime lazy iterators but as a pure frontend
  EAGER desugar to a compiled list-materializing loop wrapped in `iter(...)`
  (`lower_map`/`lower_filter`) — the same `reduce` shape, even simpler since it
  needs no runtime callback machinery at all. The per-element call rides the
  uniform tagged indirect-call/`Symbol`-dispatch path, so builtin callbacks
  (`map(str, …)`) work with no extra code. Gated by `corpus/p28_map_filter.py`.
- ~~**§5 `getattr`/`setattr`/`hasattr` (literal name) + `issubclass`.**~~ DONE
  (see the §5 Builtins entry above). Desugared in the frontend to direct
  attribute access (`getattr`/`setattr` → `Attribute`/`SetAttr`) plus two
  compile-time-`Bool` fold nodes (`HasAttr`/`IsSubclass`) — the `isinstance`
  template, ZERO runtime changes. The dynamic-`getattr` out-of-scope boundary
  stays syntactic (a non-literal name is a clean error). `hasattr` on a gradual
  receiver is still a loud error (the name-hash method/field registry probe is a
  later add when a corpus needs it). Gated by `corpus/p30_introspection.py`.
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
- ~~**§9 `str.format` + §13 dynamic f-string specs.**~~ DONE — the same
  mini-language, and the decisive simplification held: all four surfaces
  (`format()`, `str.format()`, f-string fields, dynamic specs) desugar in the
  FRONTEND to ONE `FormatValue { value, spec }` node, so there is exactly ONE
  formatting path — the existing `rt_format` over `format-shared::parse_format_spec`.
  No duplicated formatter, no repr/format drift. (See §5 `format` / §13.)
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
