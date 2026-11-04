# KNOWN_BUGS.md — review backlog

Findings from a full per-crate code review (2026-11, one reviewer per crate,
most confirmed by compiling a repro with `pyaot … --run` and diffing against
CPython 3.14). This file is the working backlog: tick an item (`[x]`) in the
same commit that fixes it, with a regression case added to the `corpus/`.
Remove the file once everything is fixed or moved to `COMPILER_STATUS.md`.

Legend: **C** critical (segfault / UAF / silent miscompile), **H** high,
**M** medium, **L** low. `✓` = confirmed by a repro.

Two root causes dominate:
- **Root A** — inferred types become *unchecked* `Raw`/`Heap` reprs, so
  inference precision decides correctness (violates invariant 2).
- **Root B** — members resolved on the receiver's *static* class, ignoring
  subclass overrides.

---

## Phase 0 — safety net

- [x] Differential oracle: `PYAOT_PYTHON` selects the interpreter (corpus needs
      3.14 / PEP 649); `PYTHONCOERCECLOCALE=0` for both sides (PEP 538 adds
      `LC_CTYPE` to CPython's `os.environ`).
- [ ] Differential gate runs every corpus entry at `--opt-level none` AND the
      default level (Truthy/devirt/inliner bugs diverge between levels).
- [ ] GC-stress run of the corpus (`RUSTFLAGS="--cfg gc_stress_test"` runtime).
- [ ] MIR verifier: `Return` operand vs `f.ret`; reject `Return(None)` from
      non-void (`mir/src/verify.rs:401`).
- [ ] MIR verifier: `params.len() <= locals.len()` and `locals[i].repr ==
      params[i]` (`verify.rs:370`).
- [ ] MIR verifier: `CallRuntime` dst vs `def.return_semantic()` — Raw must be
      `Raw(I64)`, Tagged/Heap must be GC roots (`verify.rs:965`).
- [ ] MIR verifier: `Heap(shape)` arg slot must get matching shape or checked
      coercion (`verify.rs:937`); `ContainerResult::Heap` shape (`:1028`).
- [ ] MIR verifier panics on `MakeClosure` of a zero-param callee
      (`verify.rs:707`, `params[1..]`). ✓
- [ ] MIR verifier: range-check program FuncIds (entry, generators, vtables,
      thunks); reject a block that is its own handler.

## Phase 1 — GC / memory safety

- [ ] **C** `Truthy::may_allocate`/`has_side_effects` return false but
      `rt_is_truthy` runs user `__bool__`/`__len__` → unrooted locals freed;
      DCE drops `__bool__` side effects (`mir/src/lib.rs:505`, `:648`). ✓
- [ ] **M** `GetField`/`SetField` with a `Tagged` base can raise (allocates) —
      treat as allocating/side-effecting (`mir/src/lib.rs:521`, `:662`).
- [ ] **C** `copy.copy`/`copy.deepcopy` deref tagged immediates
      (`runtime/src/copy.rs:16`, `:208`): `copy.deepcopy([[1,2]])` segfaults. ✓
- [ ] **M** deepcopy dict branch: `new_key` unrooted across value copy
      (`copy.rs:344`); copy of deque/defaultdict/Counter returns the original
      (`copy.rs:178`, `:499`). ✓ (aliasing)
- [ ] **C** sort comparators deref tagged immediates on mixed kinds
      (`runtime/src/sorted.rs:55`, `list/mutation.rs:433`):
      `sorted([1,0.5,2])`, `(1,2)<(0.5,2)` segfault; `list.sort` has no
      Tuple/Bytes arm (sorts by address); BigInts ordered by address. Merge the
      two comparators. ✓
- [ ] **C** `sorted()` result list unrooted during `stable_sort` while a user
      `__lt__` allocates (`sorted.rs:196`, `:230`). ✓
- [ ] **C** `deque.rotate` only moves `head` (`runtime/src/deque.rs:371`);
      `pop`/`popleft` leave stale pointers in vacated slots. ✓
- [ ] **H** `StringIO.truncate()` past end reads out of bounds
      (`runtime/src/stringio.rs:299`). ✓
- [ ] **M** StringIO positions are byte offsets, not char offsets → invalid
      UTF-8 strings (`stringio.rs:176`). ✓
- [ ] **H** `random.choices`: negative `k`, short `weights`, int weights all
      segfault (`runtime/src/random.rs:333`). ✓
- [ ] **M** `rt_exc_instance_str` allocates under `EXCEPTION_STATE.borrow_mut`;
      GC's `try_borrow` silently skips exception roots on conflict
      (`exceptions/ffi.rs:467`).
- [ ] **L** `rt_obj_eq`/`obj_cmp_ordering` deref `non_null` without `is_ptr`
      when the other side is null (`ops/comparison.rs:44`).
- [ ] **L** `rt_make_list`/`list_insert` don't check `alloc_zeroed` null;
      `Layout.expect` panics inside `extern "C"`.

## Phase 2 — Root A: unchecked Raw/Heap from inference

- [ ] **C** container element type inferred only from writes on the local
      itself — aliases, params, closures, globals, fields, sub-containers,
      `setdefault`/`update`/slice-assign are missed → `Raw(F64)` read of an int
      segfaults (`typeck/src/lib.rs:1604`, `:1738`). ✓
- [ ] **C** `list[float]` holding an int (`[1.0, 2]`, `append(3)`) stored
      int-tagged, read via unchecked `UnboxFloat` (codegen `lib.rs:2679`;
      fix in lowering/legalize). ✓
- [ ] **C** `raw_uniform` guards only top level; nested slots join
      int⊔float=float; container `+`/`|` use the plain join
      (`typeck/src/lib.rs:647`, `:2674`, `:2728`; `types/src/lattice.rs:383`). ✓
- [ ] **C** devirt `make_arg_coerce` drops the checked Tagged→Heap guard
      (`optimizer/src/devirt.rs:548`). ✓
- [ ] **M** lowering uses unchecked `coerce` for raw params/indices
      (`lowering/src/lib.rs:2443`, `:2353`, `:4652`); `Optional[int]` slice /
      islice bound untagged as garbage (`:5064`). ✓
- [ ] **H** `Bool op Bool` arithmetic typed Bool → `True+True` prints False
      (`typeck/src/lib.rs:2674`, `:2666`). ✓
- [ ] **M** `bool & | ^ bool` typed Int (`typeck:2723`). ✓
- [ ] **H** `Dyn / x` typed Float ignores `__truediv__` (`typeck:2698`). ✓
- [ ] **M** `Compare` always Bool even for user dunders returning other types
      (`typeck:1942`). ✓
- [ ] **H** Raw(I64)→Tagged `TagInt` unchecked: `math.factorial(20)` wraps,
      `factorial(25)` OverflowError (`stdlib-defs/src/modules/math.rs:121`,
      codegen `lib.rs:2684`). ✓
- [ ] **H** `abs()` truncates out-of-fixnum results; no BigInt arm in
      `abs`/`float` (`runtime/src/builtins.rs:267`, `:200`). ✓
- [ ] **M** bignum arg to raw-Int stdlib param → codegen error "illegal
      coercion Heap(BigInt) -> Raw(I64)" (`math.gcd(3e18-ish ints)`). ✓
- [ ] **L** `sum(list[float])` on empty list typed Float → `0.0` (`typeck:1987`). ✓

## Phase 3 — Root B: static dispatch ignores overrides

- [ ] **C** `lower_virtual_call` checks override arity only for kw/short
      calls → SIGSEGV with an extra defaulted param (`lowering:4430`). ✓
- [ ] **C** method/dunder/property result type from static class only
      (`typeck:2245`, `:2407`, `:2373`) → `1e-323`, segfaults. ✓
- [ ] **H** `concrete_dunder` → `None` when overridden → falls to runtime op
      that doesn't dispatch instances: `len(b)==0`, wrong `str(b)`, `b<b`
      TypeError (`lowering:4555`). ✓
- [ ] **H** properties, class attrs, `getattr` fold resolved on static class
      (`lowering:3206`, `:3407`, `:3310`). ✓
- [ ] **H** `inst.classattr = v` writes the class slot (`lowering:3424`). ✓
- [ ] **M** `inst.sm()`/`inst.cm()` bind statically (`lowering:4308`). ✓
- [ ] **M** `cls` in `@classmethod` folded to defining class
      (`frontend-python/src/lower/expressions.rs:36`, `lower/calls.rs:75`). ✓
- [ ] **M** redefined method: first definition wins (`semantics/src/lib.rs:344`,
      `frontend lower/classes.rs:841`). ✓
- [ ] **M** static/class/property/class-attr not inherited
      (`semantics:362`); COMPILER_STATUS rows marked ✅. ✓
- [ ] **H** `exception_base` only via first user parent (`semantics:407`). ✓
- [ ] **H** runtime registers one parent per class → `isinstance(C(), B)` false
      for `class C(A, B)` (`runtime/src/vtable.rs:222`, `:80`). ✓
- [ ] **H** duplicate top-level def/class / def rebound by assignment resolved
      statically with no diagnostic (`frontend lower/program.rs:750`). ✓
- [ ] **L** `poke(b: Base): b.y = 3` (field only on Sub) rejected by semantics.

## Phase 4 — evaluation order, scopes, statements

- [ ] **H** later operand emitting statements runs before an earlier lazy
      operand: `f() + (g() or 2)`, `x + (x := 5)`, `g(x, (x := 5))`
      (`frontend lower/expressions.rs:580` and call/subscript/compare). ✓
- [ ] **H** aug-assign reads `x` after RHS side effects; attr base evaluated
      twice (`frontend lower/statements.rs:847`, `:865`). ✓
- [ ] **M** assignment: target base/index evaluated before RHS
      (`lowering:3469`, `:3413`, `:3447`, `:990`, `:1059`;
      `frontend statements.rs:308`). ✓
- [ ] **H** `+=`/`*=` not in-place (list aliasing, tuple `+=`, `__iadd__`)
      (`frontend statements.rs:836`; `types/src/dunders.rs` lacks `__i*__`). ✓
- [ ] **H** no definite-assignment: unbound locals read as None, no
      UnboundLocalError (`frontend statements.rs:491`). ✓
- [ ] **H** `except E as e` permanently rebinds `e` in the flat scope
      (`frontend lower/exceptions.rs:310`). ✓
- [ ] **M** inliner: callee locals not reinitialized per inlined call
      (`optimizer/src/inline.rs:265`). ✓
- [ ] **H** devirt `single_def` doesn't check dominance → conditionally bound
      closure devirtualized (`optimizer/src/devirt.rs:173`). ✓
- [ ] **C** genexpr reading an enclosing local resolves it as a global
      (`frontend lower/generators.rs:10`, `freevars.rs:425`). ✓
- [ ] **L** lambda in comprehension capturing the loop var: "no cell binding"
      (`frontend lower/comprehensions.rs:100`). ✓
- [ ] **H** sequence pattern only checks `len()` (str matches `[a,b]`, dict →
      KeyError) (`frontend lower/patterns.rs:153`). ✓
- [ ] **M** `__exit__` gets `(e, e, None)` instead of `(type(e), e, tb)`
      (`frontend lower/exceptions.rs:646`). ✓
- [ ] **M** starred element in unpack RHS goes down literal fast path
      (`frontend statements.rs:288`). ✓
- [ ] **M** `print(file=sys.stderr)` sticky global switch; a raising `__str__`
      leaves stdout redirected (`lowering:1283`). ✓
- [ ] **L** module-level `return`, `global x` after use accepted; `C(x=1)`
      misleading error; generator diagnostics use `Span::dummy()`.

## Phase 5 — runtime / stdlib semantics

- [ ] **H** exception hierarchy flat: no parent column, `except OSError` /
      `LookupError` / `RuntimeError` miss subclasses; IOError not alias
      (`core-defs/src/exceptions.rs:168`, `runtime/src/vtable.rs:289`). ✓
- [ ] **H** `handling_exception` is a single slot, not a stack — bare `raise`
      after an inner handled exception fails (`exceptions/ffi.rs:110`). ✓
- [ ] **M** `super().__init__(msg)` in Exception subclass doesn't compile
      (`lowering:4364`). ✓
- [ ] **H** BigInt hash/eq by pointer in dict/set/list.index; `hash(bigint)`
      raises (`runtime/src/hash_table_utils.rs:46`, `:243`; `builtins.rs:350`). ✓
- [ ] **H** instance keys ignore `__hash__`/`__eq__`; `hash(obj)` without
      `__hash__` raises (`hash_table_utils.rs:96`). ✓
- [ ] **M** int/float mixed ops via lossy f64; `1.0 // 0.1` = 10.0;
      `10**400/10**399` nan (`runtime/src/bigint.rs:163`, `:146`, `:339`;
      `ops/comparison.rs:116`). ✓
- [ ] **M** str/bytes `repr` always single quotes (`conversions/repr.rs:19`,
      `print.rs:66`). ✓
- [ ] **M** strip/split whitespace only ` \t\n\r` (`string/trim.rs:31`,
      `slice_utils.rs:47`). ✓
- [ ] **M** Dyn `str % x`, `[1]*3`, `tuple * int` raise TypeError
      (`ops/arithmetic.rs:400`, `:269`; `lowering:5780`). ✓
- [ ] **H** json: key order sorted (no `preserve_order`), big ints truncated,
      NaN, non-str keys (`runtime/Cargo.toml:35`, `json.rs:137`). ✓
- [ ] **M** `random.sample` algorithm differs from CPython; BigInt seed
      (`random.rs:465`). ✓
- [ ] **M** `isinstance(x, dict)` false for defaultdict/Counter
      (`runtime/src/instance.rs:312`). ✓
- [ ] **M** sentinel defaults: `time.gmtime(-1.0)`, `randrange(step=0)`,
      `log(x, nan)` (`stdlib-defs time.rs:63`, `random.rs:99`, `math.rs:133`). ✓
- [ ] **M** `OrderedDict(iterable)` declared as `(capacity: Int)`
      (`stdlib-defs collections.rs:21`). ✓
- [ ] **L** `dict.keys()/values()/items()` are snapshots, not views
      (`typeck:2858`). ✓
- [ ] **M** `re.sub` template escapes; `match.group(n)` out of range → None.
- [ ] **M** format-shared: float with no type code, `g` exponent before
      rounding, `-0.0` sign, `NaN` case, grouping/`0` flag
      (`format-shared/src/lib.rs:398`, `:417`, `:137`, `:386`). ✓
- [ ] **M** typeck rejects `Dyn` for `TypeSpec::Str/Bool` stdlib params
      (`os.path.exists(p)` untyped) (`typeck:1131`). ✓
- [ ] **M** typeck underflow panic: `def m(*args)` method (`typeck:1000`). ✓
- [ ] **L** `type(e).__name__` prints `object`; `repr(e)` via Dyn.

## Phase 6 — CLI, linker, lattice, tracebacks

- [ ] **H** `--run` executes bare name via `$PATH` (`cli/src/main.rs:356`);
      signal exit reported as 1. ✓
- [ ] **H** output path can overwrite source (no `.py`) / collide with `.o`;
      `foo.o` deleted (`cli/src/main.rs:324`). ✓
- [ ] **H** lattice prune removes both mutually-subtyped members → `Never`
      (`dict`⇄`defaultdict`, `Iterator[Dyn]`) (`types/src/lattice.rs:314`). ✓
- [ ] **M** `Bool ⊔ Int = Int` loses bool identity (`lattice.rs:116`). ✓
- [ ] **M** `SemTy::optional` builds non-canonical unions (`types/src/sem.rs:179`).
- [ ] **M** diagnostics in imported modules rendered against entry file —
      `Span` has no file id (`cli/src/main.rs:228`, `utils/src/span.rs`). ✓
- [ ] **M** srcloc base wrap → `line 0` (`codegen-cranelift/src/lib.rs:2094`,
      use `ensure_base_srcloc`). ✓
- [ ] **M** traceback names (`C.m`, `inner#0`, `<resume>`, thunk frames)
      differ from CPython (`codegen lib.rs:828`); inlined frames erased
      (`optimizer/src/inline.rs:254`). ✓
- [ ] **M** linker: package archives after runtime archive (GNU ld single
      pass); missing runtime lib silently skipped; Windows/other OS branches
      broken (`linker/src/lib.rs:59`).

## Phase 7 — performance & cleanup

- [ ] Raw(I64) add/sub/mul compile to `rt_*_int` calls instead of inline ops
      (`codegen lib.rs:2720`).
- [ ] Raw(F64) compare/div/neg box both operands (`lowering:5656`, `:6139`).
- [ ] Quadratic: liveness worklist + per-alloc local scan
      (`mir/src/liveness.rs:96`); peephole `resolve_target` (`peephole.rs:83`);
      devirt `single_def` clones (`devirt.rs:173`); inline size recompute +
      env var per refusal (`inline.rs:131`); no FuncRef/DataId cache
      (`codegen lib.rs:1981`); linear generator dispatch (`:943`).
- [ ] Set tombstone accounting (`runtime/src/set/ops.rs:31`).
- [ ] `iconst` not masked for I8/I32; unchecked int tag shift of literals
      (`codegen lib.rs:2572`, `:1254`).
- [ ] Dead/wrong descriptors: 273 unused `RT_*` (11 name missing symbols),
      `RT_BYTES_STARTS/ENDS_WITH` declared i8 vs i64, collections/functools
      sentinel descriptors (`core-defs/src/runtime_func_def.rs`,
      `stdlib-defs collections.rs`, `functools.rs`).
- [ ] `Coercion::HeapToTagged` never produced (`mir/src/lib.rs:1197`).
- [ ] Stale docs: optimizer crate header, core-defs ranges, runtime
      `FIRST_USER_CLASS_ID`, `verify_call_container` doc placement.
- [ ] Duplication: arg-tuple / kwargs-dict packing in lowering
      (`:1893`, `:3933`, `:4183`); `materialize_const` vs `lower_const`.
- [ ] COMPILER_STATUS: downgrade in-place dunders, MI/C3 at runtime,
      inherited static/class/property, `super()` to 🟡 until fixed.
