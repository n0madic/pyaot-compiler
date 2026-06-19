//! Differential harness — the Phase-1 gate.
//!
//! For each file in [`PHASE_CORPUS`] (an explicit **allowlist**, NOT a glob, so
//! the full-feature corpus files cannot break this phase's gate): compile it with
//! `pyaot`, run the resulting executable, run the same file under `python3`, and
//! compare the two stdouts per the entry's [`DiffMode`]. CPython is the live
//! oracle (no `.expected` fixtures).

use std::path::{Path, PathBuf};
use std::process::Command;

/// Entries whose intermediate output is inherently run-dependent (live
/// timestamps), compared in self-checking mode instead of byte-diff: both
/// runs must exit 0 and end with the same final line — the file's own
/// "…passed!" marker, which only prints when every internal assert held.
const SELF_CHECKING: &[&str] = &[
    "test_stdlib_time.py",
    "test_file_io_core.py",
    "test_stdlib_subprocess.py",
];

/// The phase spec entries — an explicit allowlist that grows one feature at a
/// time. Each file's compiled stdout must match CPython byte-for-byte
/// (except the [`SELF_CHECKING`] few).
const PHASE_CORPUS: &[&str] = &[
    "test_hello.py",
    "p2_scalars_print.py",
    "p2_expr.py",
    "p2_control.py",
    "p2_gc_stress.py",
    "p2_funcs.py",
    "test_main.py",
    "p2_bignum.py",
    "p3_numeric.py",
    // Phase 3c — raw-int loop specialization via typeck's interval proof: a
    // narrowed induction variable + derived i*3 % k / i*3 // k running raw, the
    // negative-operand floor correction (srem/sdiv → Python //, %), and the
    // proof correctly REFUSING (a doubling-to-bignum accumulator and a
    // collatz-shaped while stay tagged → bignum-safe and byte-exact).
    "p3c_raw_int_loops.py",
    // Phase 3c interprocedural (PLAN backlog #7, Part A) — raw-int proof across
    // direct call edges: bounded args make a callee's params + return Raw(I64)
    // (the bench_exc_hotpath shape, incl. inside a `try` → Tail trampoline seam),
    // while an address-taken callback, a per-position unbounded arg (a bignum),
    // and a recursive bounded function correctly stay tagged. Mis-specializing
    // any of those would untag a heap BigInt as garbage — a clean run is the
    // soundness proof.
    "p3c_interproc_raw.py",
    // Phase 4A — container literals, indexed read/write, len/in, operators.
    "p4_literals.py",
    "p4_subscript.py",
    "p4_operators.py",
    // Phase 4B — general for-loop + iterator protocol + tuple unpacking.
    "p4_for_iter.py",
    "p4_unpack.py",
    // Phase 4C — comprehensions + iteration builtins.
    "p4_comprehensions.py",
    "p4_iter_builtins.py",
    // Phase 4D — focused container methods.
    "p4_methods.py",
    // Phase 4 — cross-feature integration + GC soak (B5/B15).
    "p4_integration.py",
    "p4_gc_stress.py",
    // Phase 5A — core classes: fields, methods, construction.
    "p5_class_basic.py",
    // Phase 5B — inheritance, super(), C3 MRO, virtual dispatch, isinstance.
    "p5_inherit.py",
    // MRO-aware nominal joins in the lattice (PLAN item 4): unannotated sibling
    // joins resolve to the nearest common C3 ancestor instead of a Union.
    "p5_mro_join.py",
    // Phase 5C — dunders: arithmetic / comparison / conversion / container.
    "p5_dunder_arith.py",
    "p5_dunder_container.py",
    // Phase 5D — decorators (@staticmethod/@classmethod/@property) + class attrs.
    "p5_decorators.py",
    // Phase 5E — generics: TypeVar / Generic[T] / typed instantiation.
    "p5_generics.py",
    // Phase 5 — class-instance-graph GC soak (uniform-tagged field tracing).
    "p5_gc_stress.py",
    // Phase 6A — closures, lambdas, functions as values.
    "p6_closures.py",
    "p6_lambda_hof.py",
    // Phase 6B — nonlocal / global.
    "p6_nonlocal_global.py",
    // Phase 6C — defaults, keyword args, *args / **kwargs.
    "p6_varargs.py",
    "p6_defaults_kwargs.py",
    // Backlog §1 — mutable / computed parameter defaults (top-level functions).
    "p36_mutable_defaults.py",
    // Backlog §1 — `**dict` spread into a direct call (literal + runtime).
    "p37_kwargs_spread.py",
    // test_functions.py lift, Phase 1 — `rt_unbox_bool` (third checked-unbox
    // shape, Tagged -> Raw(I8)): a Dyn value into an annotated `: bool` slot.
    "p38_unbox_bool.py",
    // test_functions.py lift, Phase 2 (b1) — closure/lambda values typed
    // `Callable(sig)`: a lambda or returned closure bound and called by value.
    "p39_closure_values.py",
    // Uniform value-call carrying KEYWORDS into a kwonly / `**kwargs` closure:
    // the call site builds a keyword dict (named + `**d` merge), and the closure's
    // uniform thunk normalizes the null `__kwargs__` sentinel (no-keyword common
    // path) to an empty dict, so `**kwargs` inspection / kwonly binding is sound.
    "p40_value_call_kwargs.py",
    // Runtime callable guard on the uniform value-call path: a non-closure `Dyn`
    // callee (int / str / None / a DATA tuple — distinct `Closure` vs `Tuple` tag)
    // raises `TypeError` instead of crashing on a bad slot-0 read; a real closure
    // through the same `Dyn` path still calls. Pins the closure/tuple tag split.
    "p41_call_guard.py",
    // Phase 6D — user decorators (functions).
    "p6_decorators.py",
    // Phase 6E — generators, send/close, generator expressions, GC soak.
    "p6_generators.py",
    "p6_send_close.py",
    "p6_genexpr.py",
    "p6_gc_stress.py",
    // Phase 7A — raise + try/except (builtin exceptions).
    "p7_raise_tryexcept.py",
    "test_multi_except.py",
    // Phase 7B — finally/else, raise-from chaining, instance surface.
    "p7_finally.py",
    "test_traceback.py",
    // Phase 7C — custom exception classes.
    "p7_custom_exc.py",
    // Phase 7D — context managers.
    "p7_with.py",
    "test_exceptions.py",
    // Phase 7E — structural match.
    "p7_match.py",
    "test_match.py",
    // Phase 7 — raise/catch GC soak (shadow-stack unwind + rooted `as e`).
    "p7_gc_stress.py",
    // Phase 8A — module-level global scoping (annotated globals stay typed even
    // when written inside functions). No imports — exercises the global
    // infrastructure the import machinery promotes module bindings onto.
    "test_global_scoping.py",
    // Phase 8A — multi-module: `import` / `from … import` (incl. relative),
    // packages, cross-module functions / classes / constants / generators. The
    // `math_utils.py` / `genmod.py` / `mypackage/**` trees are fixtures (search
    // path = the source's directory), never gate entries themselves.
    "test_import.py",
    "test_packages.py",
    // Phase 8A — package re-export: a package `__init__.py` that publishes names
    // it imported from a submodule (`from .circle import Circle`). The canonical
    // facade — `from shapes import Circle` and `import shapes; shapes.Circle`.
    "test_reexport.py",
    // Phase 8B — the descriptor-driven stdlib `CallRuntime` path: math (raw
    // f64/i64 ABI + constants), random (CPython-exact MT19937, kwargs,
    // pass_arg_count, absent-optional sentinel), sys (attr getters, argv/path
    // singletons), time (struct_time fields; self-checking — live timestamps).
    "test_stdlib_math.py",
    "test_stdlib_random.py",
    "test_stdlib_sys.py",
    "test_stdlib_time.py",
    // Phase 8C — stdlib object types: re/Match methods (group/span via the
    // object-type registry), json (dumps/loads + dump/load to a File), and
    // File I/O (open/read/write/with/modes/encoding/iteration, io.StringIO/
    // BytesIO). `test_file_io_core.py` is self-checking — it writes /tmp paths.
    "test_stdlib_re.py",
    "test_stdlib_json.py",
    "test_file_io_core.py",
    // §14 — full file I/O suite (binary mode round-trips, `r+`/`w+`/`a+`, file
    // iteration, StringIO/BytesIO). Its one blocker was the non-UTF-8 `bytes`
    // literal `b"\x00\x01\x02\xff"`: byte literals shared the UTF-8 `String`
    // interner, so `b"\xff"` errored at lower time. The interner now stores raw
    // byte blobs (`intern_bytes`/`resolve_bytes`), so non-UTF-8 literals round-trip
    // intact (runtime was already byte-clean: `rt_make_bytes` = raw memcpy). The
    // output is deterministic (fixed /tmp paths, self-cleaning) → byte-diffable.
    "test_file_io.py",
    // Phase 8D — os / os.path (submodule-chain folding + variadic join), the
    // `os.environ` dict attr, subprocess.run (CompletedProcess fields;
    // self-checking — subprocess stdout is bytes in CPython, str here), and
    // urllib (parse + ParseResult/Request fields, plus the urllib.error
    // exception hierarchy raised/caught by id and by builtin parent). The
    // network urlopen/urlretrieve sections are excluded (non-deterministic).
    "test_stdlib_os.py",
    "test_stdlib_subprocess.py",
    "test_stdlib_urllib_core.py",
    // itertools.chain regression: chain folds its variadic iterables into one
    // list (VARIADIC_TO_LIST) and the runtime iter()-wraps each element lazily.
    // Formerly chain dropped every arg past the first and passed the first
    // iterable where a list-of-iterators was expected -> null deref / SIGSEGV
    // (the crash test_generators.py's `itertools.chain` section hit). Covers
    // lists, generators, mixed kinds, empty/single/nested chain, and chain fed
    // to list/tuple/sum/for/comprehension.
    "test_stdlib_itertools.py",
    // Phase 8E — language gaps for real scripts: f-string format specs, slicing
    // (list/str/tuple, negative/stepped/open-ended), str.join / list.index, the
    // tuple `()` parameter default + `__slots__`, and the cross-function
    // return-type inference that keeps unannotated dunder/method results typed.
    "p8e_language.py",
    // Phase 8F — the capstone: Karpathy's microgpt (autograd `Value` with 12
    // dunders, multi-head attention, Adam training, temperature sampling) on real
    // stdlib (math.log/exp, random.gauss/shuffle/choices — MT19937 + libm match
    // CPython bit-for-bit). A small model config keeps it byte-exact yet fast
    // under the debug runtime. Exercises nearly every front-half feature at once.
    "microgpt.py",
    // Phase 8 seam-safety regressions: the stdlib/container seam used to pass a
    // mismatched heap shape (str/tuple/generator to `join`, a heap-valued
    // `dict.get` miss, a str-keyed `json.loads` subscript) or a bare None to the
    // frozen runtime, which dereferenced it without a guard — SEGVs + silent
    // wrong values. Also `list.index(missing)` now raises `ValueError`.
    "p8g_seam_safety.py",
    // Phase 8H stage A — stdlib edge-case parity: urlencode str()-ifies non-str
    // values (was a tagged-int deref SEGV), posixpath-exact basename/dirname,
    // quote's safe="/" default, json.dumps ensure_ascii, slice step=0 ->
    // ValueError, int/bool with float presentation types, and the CPython
    // __str__ of HTTPError/URLError.
    "p8h_stdlib_edges.py",
    // Phase 8H stage B — codepoint-correct string model: len/subscript/slices/
    // step-slices/iteration/reversed walk Unicode codepoints (not bytes),
    // upper/lower/title/capitalize/swapcase use full char case mappings, and
    // center/ljust/rjust/zfill widths count characters.
    "p8h_unicode.py",
    // Phase 8H stage C — front-half: module-level lambda defaults via the
    // synthetic-def desugar (known-callee kwargs/defaults for free), `for line
    // in f:` over a File VARIABLE (lowering expands Iter(File) through
    // rt_file_readlines), and os.environ writes through rt_os_environ_set
    // (visible to getenv / environ reads; a plain SetItem wrote into a fresh
    // snapshot and was lost).
    "p8h_lang.py",
    // Phase 8H D1 — element-type inference from container pushes: comprehension
    // results and append/add/insert/extend/setitem-built containers solve to
    // precise element types (list[float], dict[int, float], …) instead of
    // pinning `…[Dyn]`, keeping downstream numeric code specialized.
    "p8h_comp_elem.py",
    // Phase 8H D2 — sum() through the typed HIR node: numeric promotion
    // (int/float/bool), inferred __add__/__radd__ returns for class elements,
    // generator arguments materialized as list comprehensions, and a single
    // Tagged-accumulator loop expanded at lowering.
    "p8h_sum.py",
    // Phase 8H D3 — checked Dyn->Raw unbox at stdlib raw-ABI boundaries:
    // gradual/numeric args reach math.* raw params through rt_unbox_float /
    // rt_unbox_int (TypeError on a bad tag) while proven types keep the
    // unchecked fast path.
    "p8h_checked_unbox.py",
    // Phase 8H D3 (extended) — checked-unbox seams beyond the basics:
    // Optional/None into raw-f64 (TypeError, not a null-deref), Dyn container
    // elements and dict.get misses, raw-i64 params (gcd/comb/factorial/perm)
    // fed from Dyn/bool/str sources, and chained Dyn producers. Also pins the
    // Raw-uniformity guard on element slots: `[2.25, 16, True]` stays tagged
    // (list[Dyn]) instead of a `Raw(F64)` slot blindly unboxing a tagged int.
    "p8h_checked_unbox2.py",
    // Phase 8H D4 — by-name field access on a Dyn receiver: fields resolve at
    // runtime through the FIELD_NAME_REGISTRY (rt_getattr_name/rt_setattr_name,
    // AttributeError on a miss/non-instance); sum() over class elements rides
    // the inferred __add__ returns. Method calls on Dyn stay a loud error.
    "p8h_dyn_attr.py",
    // Phase 9 — GC root-set narrowing via liveness (B15 -> real dataflow):
    // strs consumed before vs live across allocation loops, the uses(I) rule
    // (argument of an allocating call), the TryEnter handler rule (pre-try
    // value read in the handler), generator locals across yields, and bignum
    // promotion as an allocating tagged BinOp. Run under the gc_stress
    // runtime to surface any missed root as a use-after-free.
    "p9_root_narrowing_gc_stress.py",
    // Runtime ShadowFrame audit — zip over fresh-element sources (string /
    // generator / enumerate): the zip nexts must root already-obtained items
    // across the remaining inner nexts and the result-tuple allocation
    // (internal ShadowFrame, the rt_list_from_iter pattern). Crashes the
    // unfixed runtime under gc_stress.
    "p9_zip_fresh_elems_gc_stress.py",
    // B10 — field-type inference as solver variables: unannotated fields are
    // joined over every module-wide write (the autograd pattern:
    // `child.grad = child.grad + local * out.grad` through non-self
    // receivers), mixed int/float writes demote the field to Dyn instead of
    // rejecting, a Dyn-receiver write demotes by name (it lowers to
    // SetFieldNamed, which can hit any class), a subclass write feeds the
    // base class's variable, and annotated fields stay authoritative.
    "b10_field_inference.py",
    // Phase 10 — kwargs evaluation order (PLAN §1 trap): keyword-call argument
    // side effects run left-to-right AS WRITTEN (pass-1 staging), never in
    // parameter-slot order, across defaults / kw-only / *args / **kwargs.
    "p10_kwargs_evalorder.py",
    // Phase 10 — kwargs on builtins: sorted(key=, reverse=) via the compiled
    // key loop + rt_list_sort_by_keys tandem sort (stability incl. reverse),
    // enumerate(start=), dict(a=1) / dict(pos, kw=) incl. dict-copy routing,
    // and the kwargs × closure interaction probe.
    "p10_kwargs_builtins.py",
    // Phase 10 — kwargs on methods: user classes (defaults / written-order
    // side effects / **kwargs leftovers), virtual dispatch + super() with
    // keywords (identical-override-defaults precondition), classmethod /
    // staticmethod, and the full list.sort(key=, reverse=) matrix incl.
    // stability.
    "p10_kwargs_methods.py",
    // Unblocked by Phase 10 kwargs (sorted(key=)/list.sort(key=) were its
    // first blocker): builtins as first-class values incl. keyed sorting.
    "test_builtin_first_class.py",
    // Backlog §2 — `is` / `is not` against non-None operands (bit-identity via
    // rt_is): bool/None singletons, same-object / distinct-object / alias for
    // class instances and containers, `is not`, and identity inside if/while
    // guards combined with and/or/not. Int/str caching is NOT modeled (§2 trap).
    "p11_is_identity.py",
    // Backlog §3 — the `del` statement: `del d[k]` (+ missing → KeyError),
    // `del li[i]` (+ negative / OOB → IndexError), a class `__delitem__` (with
    // `del self.data[i]` on an attribute base), `del name` local (rebind-ok and
    // read → UnboundLocalError), module-global `del g` (read → NameError),
    // `del obj.attr` (read → AttributeError, then reassign), and `del` inside
    // if/loop bodies + multi-target `del a, b`.
    "p12_del.py",
    // Backlog §1 — `*seq` spread into a non-`*args` callee. A list/tuple LITERAL
    // spread flattens at compile time; a runtime sequence (variable / call
    // result / comprehension) materializes an argv list, length-checks it, and
    // binds each parameter by position. Covers fixed-arity / mixed plain+spread /
    // multiple spreads / empty spread, str (gradual-heap) and float/bool (Raw,
    // laundered through a pin_tagged typed slot) params, defaults filled from a
    // short spread, `*args` callees (spread covering fixed + rest), a decorated
    // callee's (*args, **kwargs) wrapper, and interaction probes: comprehension
    // source, spread in a loop, and left-to-right call-arg evaluation order.
    "p13_spread.py",
    // Backlog §4 — nested destructuring (`a, (b, c) = …`, `(m1,m2),(m3,m4) = …`,
    // `g, [h, i] = …`) across assignment / for-loop / comprehension / gen-expr.
    // `assign_to_target` recurses into a Tuple/List target through
    // `lower_unpack_subscript`: each nested element is staged and re-subscripted
    // positionally, so deeper nesting and the for/comp paths fall out of the same
    // machinery. Covers literal + runtime (variable/call) RHS, mixed tuple/list,
    // nested attribute/subscript leaves, nested + starred (assignment), and
    // interaction probes crossing nesting with comprehension element-type
    // inference + sum() and with `*seq` spread (§13). (Both
    // `test_collections_list_tuple.py` and `test_iteration.py` — once blocked on
    // unrelated gaps — are now LIFTED below.)
    "p14_nested_unpack.py",
    // Tuple slice (`t[a:b]` → variable-length `tuple[T, ...]`) assigned into an
    // annotated fixed-arity `tuple[T, …]` slot. The repr-contract check now admits
    // a `tuple` → `tuple` store when element `Repr`s match per index (a fixed and a
    // variable tuple are one physical `TupleObj`); `len()` reads the real runtime
    // length, not the annotated arity. Probes Tagged-int / Raw-f64 / Heap-str
    // element families + iterate/unpack interaction probes.
    "p15_tuple_slice_slot.py",
    // int→float numeric tower through a float slot (PLAN §8). An int/bool/gradual
    // value into a `-> float` return or an annotated `: float` local is a real
    // (checked) coercion — `rt_unbox_float` (with a bignum arm), not a noop. The
    // annotation is a contract (CPython keeps the raw int), so the probe asserts
    // via `==` and prints only float-forced results to keep the divergence
    // unobservable. Covers int/bool returns, a `: float` local from int, a Dyn
    // mixed return into a float local, the BigInt arm (`2 ** 62`), and a
    // `sum`-over-floats interaction.
    "p16_numeric_tower_float.py",
    // `type()` builtin incl. `type(x).__name__` (PLAN §6). A pyaot "type object"
    // IS its repr string: `type(x)` → `rt_builtin_type` → `<class '...'>` (builtins
    // via the value tag, user instances via the registered module-qualified
    // qualname), so `str(type(x))` / `print(type(x))` / `==`-on-name all fall out.
    // `type(<1 arg>).__name__` is a lowering peephole through `rt_type_name_extract`
    // — the SAME runtime string, last dotted segment — never a parallel
    // compile-time name table (the §6 one-source trap). Probes every builtin tag,
    // a user class (qualified `<class '__main__.Widget'>` vs bare `Widget` from one
    // source), and interaction with f-strings / `==` / a bound var. Divergences
    // `type(x) is T` and `repr(type(x))` are out of scope and unprobed.
    "p17_type_builtin.py",
    // Scalar / value builtins (PLAN §5): `pow`, `divmod`, `all`, `any`, `id`,
    // `round`, `bin`, `hex`, `oct`. Recognized by name in the frontend (like
    // sum/min/max), unshadowed-gated. Two shapes: pure desugar (`pow` → `**`,
    // `divmod` → staged `(a//b, a%b)`, `all`/`any` → a truthiness-short-circuit
    // iterator loop) and declarative `CallRuntime` (`id` wraps `rt_id_obj`;
    // `round` → `rt_builtin_round` banker's via decimal formatting; `bin`/`hex`/
    // `oct` → BIGNUM-AWARE `rt_builtin_*` taking a TAGGED Value — B16). Probes
    // negative-operand `divmod` sign (B1), banker's half-even incl. `2.675`,
    // `id`↔`is` consistency, `bin(2**100)`/`hex(2**100)` (the B16 gate), and
    // f-string / unpack / arithmetic interactions. Out of scope: 1-/3-arg pow.
    "p18_scalar_builtins.py",
    // Consolidated core-types/operators suite (514 lines, no imports). Its sole
    // §5 blocker was `round` — closed by p18, so it now byte-matches CPython
    // end-to-end and is lifted onto the gate.
    "test_core_types.py",
    // §9 str methods (runtime-ready batch): split/rsplit/splitlines, replace,
    // lstrip/rstrip, removeprefix/removesuffix, expandtabs, partition/
    // rpartition, encode, rindex, and ASCII predicates is{digit,alpha,alnum,
    // space,upper,lower,ascii} — declarative `StrPlan` wiring (no codegen edit)
    // of runtime fns whose impls + descriptors already existed. `maxsplit`/
    // `tabsize` ride a RAW i64 slot (descriptors retyped — B16); an explicit
    // `None` sep/chars lowers to the null "default" sentinel (not NONE_TAG).
    // Multi-byte (Cyrillic / café) inputs exercise the codepoint char_len
    // recount paths. Limits (unprobed): positional-only, no replace `count`,
    // no splitlines `keepends`, encode ignores encoding, no find/index
    // start/end, predicates are ASCII-only.
    "p19_str_methods.py",
    // §9 bytes methods (runtime-ready batch): startswith/endswith, find/rfind,
    // count, replace, split/rsplit, strip/lstrip/rstrip, upper/lower, join (+ the
    // pre-existing decode). A bytes receiver routes to `lower_bytes_method`, the
    // exact sibling of `lower_str_method` — a declarative `BytesPlan` table → the
    // shared `emit_seq_method` (no codegen edit; runtime fn resolved by symbol).
    // `maxsplit` rides a RAW i64 slot (B16); find/rfind use dedicated 2-arg fns
    // (no op_tag); the split family returns `list[bytes]`. Non-ASCII content
    // (b"\xc3\xa9", valid UTF-8) exercises the byte-accurate paths. Limits
    // (unprobed): positional-only, no replace `count`, strip family no `chars`,
    // no find start/end, decode ignores encoding, upper/lower ASCII-only.
    "p20_bytes_methods.py",
    // §9 tuple/set/dict container methods (ContainerMethod path): tuple
    // index/count, set issubset/issuperset/isdisjoint + the three *_update +
    // symmetric_difference (new-set), list.remove, dict popitem. Dispatch keys on
    // the receiver `SemTy` via `MethodRecv` (never the method name alone —
    // `index`/`count`/`update`/`remove` are shared, the §10 trap), routed to a
    // new `ContainerOp` per method (hir op + codegen `d()` + lowering `MethodRecv`
    // arm + typeck `method_ty`). Comparisons ride a proven `Raw(I8)` (B13);
    // updates + list.remove mutate in place (None; remove raises ValueError on
    // miss); the `popitem` 2-tuple stays Tagged (GC-rootable, B5) and typed `Dyn`
    // so `k, v = d.popitem()` unpacks through the gradual seam. ValueError
    // (tuple.index/list.remove miss) / KeyError (empty popitem) are the spec; set
    // contents print via `sorted(list(s))` for determinism.
    "p21_container_methods.py",
    // Backlog §4 (finish "Unpacking & loop targets"): attribute/subscript `for`-loop
    // targets + a non-literal/computed `range()` step. `bind_for_target` now
    // delegates to `assign_to_target`, so an attr (`for o.a in …` → SetAttr) or
    // subscript (`for l[i] in …` → SetItem) leaf binds each iteration via the same
    // path nested destructuring uses (DRY, no new HIR/typeck surface). `lower_for`
    // takes the Phase-3c raw-i64 fast path ONLY for a simple-`Name` target with a
    // compile-time-literal step (`range_step_is_literal`); everything else — a
    // computed/variable step, an attr/subscript target — takes the general iterator
    // path driving the runtime `RangeIter` (correct direction + a `step == 0`
    // `ValueError` guard added to `rt_iter_range`). Probes the §4 trap (a negative
    // VARIABLE step descends, NOT `sum == 0`), step=0 in both the for-loop and value
    // forms, and a tuple-unpack regression-guard.
    "p22_loop_targets.py",
    // Standalone `iter()` builtin + `isinstance()` against container builtins.
    // `iter(iterable)` builds a runtime iterator via the same `ContainerOp::Iter` →
    // `rt_iter_value` the for-loop drives (wired next to `next`, recognized by name);
    // `next(it)` consumes it via the raising `rt_iter_next` (StopIteration on
    // exhaustion). `isinstance(x, list|dict|set|tuple)` extends the builtin-isinstance
    // static fold to match container targets by KIND (element types are irrelevant),
    // alongside the existing `str|int|float|bool|bytes`. Probes iter/next over every
    // iterable kind + StopIteration, positive/cross-kind/primitive isinstance, the
    // element-type-agnostic property, and the `*rest`-is-a-list unpack usage.
    "p23_iter_isinstance.py",
    // `functools.reduce(func, iterable[, initial])` — a higher-order builtin
    // desugared in the frontend to a compiled accumulator loop calling
    // `func(acc, elem)` (mirroring sum/min/max/all/any), NOT the raw-ABI
    // `rt_reduce` callback path (the PITFALLS A4 anti-pattern — the descriptor's
    // `rt_reduce` ABI never matched the 3-arg stdlib dispatch, so the previous
    // generic-dispatch fallthrough SIGSEGV'd). The callable rides the ordinary
    // indirect-call machinery (lambda / capturing lambda / named def). Probes
    // sum/product/subtraction (left-fold order), with/without initial, single
    // element, empty+initial vs empty→TypeError, a capturing lambda, range/tuple/
    // str/list-heap accumulators (GC-rooted), and reduce() inside a function.
    "p24_reduce.py",
    // Lexicographic tuple ordering in min/max/sorted + dynamic sequence concat.
    // `rt_obj_cmp` (the min/max fold) and `sorted`/`compare_list_elements` now
    // route a `Tuple` operand to the lexicographic `tuple_cmp_ordering` (recursing
    // element-wise for nested tuples), instead of raising TypeError (min/max) or
    // comparing by pointer address (sorted). `rt_obj_add` (the gradual `+` path —
    // two `Dyn` operands, e.g. inside an untyped-param function) now handles
    // `list + list` / `tuple + tuple` / `bytes + bytes` via the existing
    // `rt_*_concat` (statically-typed concat already worked). Probes min/max/sorted
    // over (nested) tuples, tie-breaking, direct `<`/`<=` operators, dynamic concat
    // of every sequence kind + numeric/str regression guards, and the
    // mismatched-type (`list + tuple`) TypeError. (Runtime contract change.)
    "p25_tuple_cmp_seq_concat.py",
    // Walrus / named expression `:=` (PEP 572, §2). `lower_named_expr` evaluates the
    // value once, binds the (bare-name) target in the containing scope via the
    // ordinary write/read place machinery (local / cell / promoted global), and
    // yields the assigned value — so a name bound in an `if`/`while`/comprehension
    // test is visible afterward. Also regression-guards `rt_obj_pos`: unary `+` on a
    // bool now yields an int (`+True == 1`), mirroring `-True == -1`. Probes walrus
    // in if/while(re-eval)/function/comprehension(leak)/ternary/nested/statement +
    // module-global promotion.
    "p26_walrus.py",
    // Matrix-multiply `@` / `__matmul__` (PEP 465, §2). No built-in numeric `@`, so
    // `a @ b` lowers to `BinOp::MatMul` → tagged `rt_obj_matmul`, which dispatches the
    // user `__matmul__`/`__rmatmul__` dunder (or raises TypeError) — the same
    // runtime-dunder path as `+`/`*`. typeck types the result as `__matmul__`'s
    // declared return (so attr access on a matrix product resolves). Probes a scalar
    // dot-product, an instance-returning matmul + chained/attr-access, `__rmatmul__`
    // (int @ instance), `@=` (falls back to `__matmul__`), matmul over a loop, and the
    // TypeError for int@int / no-dunder objects. (Runtime contract: new rt_obj_matmul.)
    "p27_matmul.py",
    // `map`/`filter` builtins (§5) — the next HOFs after `reduce`. Both are a PURE
    // FRONTEND desugar (`lower_map`/`lower_filter`) to an EAGER compiled loop that
    // calls the callback per element through the ordinary uniform-tagged
    // indirect-call machinery, materializes into a `list`, and wraps it in
    // `iter(...)` so `for`/`list`/`next`/`sum` consume it:
    //   map(f, xs) ~= iter([f(x) for x in xs]); filter(f, xs) ~= iter([x for x in
    //   xs if f(x)]); filter(None, xs) ~= iter([x for x in xs if x]).
    // This deliberately AVOIDS the runtime `rt_map_new`/`rt_filter_new`/
    // `IteratorKind::Map/Filter` lazy-iterator HOF machinery — the PITFALLS A4
    // anti-pattern (a parallel calling convention with hand-encoded captures and an
    // `i8` predicate ABI). Builtin callbacks (`map(str, …)`/`map(len, …)`) resolve
    // through normal `Symbol`-dispatch with no extra code. `f` is staged ONCE
    // (CPython single function evaluation); the eager-vs-lazy side-effect timing is
    // observationally invisible on the finite/pure corpus (the `lower_sum`/`reduce`
    // materialization precedent). Single-iterable only — multi-iterable `map` needs
    // `zip` (§12). (Runtime contract evolved: `rt_list_eq`/`rt_tuple_eq` now compare
    // elements via the full `rt_obj_eq` instead of the hashable-key
    // `eq_hashable_obj`, so NESTED non-hashable elements — `[[1]] == [[1]]` — compare
    // by value; the probe's `filter(None, list-elements)` case surfaced that
    // pre-existing latent bug.)
    "p28_map_filter.py",
    // §5/§9/§13 — the `format()` mini-language (full). All four surfaces collapse
    // onto ONE node: f-string fields, `format(v[,spec])`, `"...".format(...)`, and
    // dynamic specs (`f"{x:.{n}f}"`) all desugar in the FRONTEND to the same
    // `FormatValue { value, spec }` (`spec` is now an expr) that `rt_format`
    // (PEP-3101 engine in `format-shared`) already backs — no new runtime parser.
    // `"...".format` on a literal receiver parses to literal `StrLit`s + per-field
    // `FormatValue` joined by `+` (the f-string tail), binding fields to pos/kw
    // args at compile time (auto/manual/keyword + mix, `{{`/`}}`, static specs).
    // Runtime contract evolved (Principle 8): `rt_format` gained a class-instance
    // arm (`__format__`, else `object.__format__` → empty-spec `str(self)` via a
    // new `try_str_dunder`); `ascii` is now a first-class builtin
    // (`rt_builtin_ascii` → the value-level ascii dispatcher) wiring `!a` and
    // `ascii()`; and `format_bool` was corrected (bool inherits `int.__format__`,
    // so `f"{True:5}"` == "    1", not " True" — the test file's stale assertion
    // was fixed to the CPython oracle). `test_format_spec.py` crosses format with
    // f-strings × user classes × functions × dynamic specs; `p29_format.py` covers
    // the `.format`/`!a` shapes.
    "test_format_spec.py",
    "p29_format.py",
    // Consolidated strings suite (LIFTED). PEP-501 debug `=` f-strings
    // (`f"{x=}"`, `f"{x=!a}"`, verbatim-expression / whitespace / spec variants)
    // already worked (rustpython-parser expands `=` into literal-text +
    // `FormattedValue`). The real blockers were `str.join` over a non-list
    // iterable: it returned `Dyn` (the str-method typing arm lacked `join`), so a
    // chained `",".join(s).split(",")` saw a gradual receiver — now typed `str`;
    // and `",".join(deque(...))`, which needed deque construction-from-iterable
    // (the front-half routed `deque(it)` through a maxlen-only `rt_make_deque`,
    // dropping the elements → a wired `DEQUE_FROM_ITER` intercept + `rt_iter_deque`
    // in the generic iter dispatcher). Byte-matches CPython end-to-end.
    "test_strings.py",
    // §5 introspection builtins (`getattr`/`setattr`/`hasattr`/`issubclass`) —
    // all collapse onto existing machinery (ZERO runtime changes): getattr/setattr
    // are frontend desugars onto the `Attribute` read / `SetAttr` write (static
    // `GetField`/`SetField` for a concrete receiver), and hasattr/issubclass fold
    // to a compile-time `Const::Bool` (from the receiver's `ClassInfo` /
    // `ClassTable::is_subclass` C3-MRO check) exactly like `IsInstanceBuiltin`.
    // Crosses the four with class inheritance (Animal→Dog/Cat), concrete-instance
    // round-trips, and already-green features (f-strings, arithmetic). Scope
    // limits (clean errors): dynamic getattr (non-literal name), getattr 3-arg
    // default, hasattr on a Dyn receiver, issubclass with a builtin-type / tuple
    // arg. (One of the four blockers that, together with p31/p32, lifted
    // `test_builtins.py` — below.)
    "p30_introspection.py",
    // Multi-iterable `zip` (3+ iterables), §12. The runtime already had
    // `rt_zip3_new`/`rt_zipn_new` + the Zip3/ZipN iterator objects; only the
    // front-half was wired for 2. Now `zip(a,b,c,…)` (N≥3) lowers to a fresh
    // runtime list of the N `iter()`-wrapped sources + `rt_zipn_new(list, count)`
    // (one new `ContainerOp::ZipN`, ABI `[Val, Idx]`), and typeck infers the
    // element as a fixed-arity `tuple[…]` (one type per iterable) so
    // `list(zip(xs, ys, zs))` types as `list[tuple[X,Y,Z]]` and fills an
    // annotated container slot. The 2-iterable `rt_zip_new` path is unchanged.
    // Covers 3/4/5-arity, shortest-wins, direct for-unpacking, and the 2-arity
    // regression. (Closed `test_builtins.py`'s line-1324 blocker.)
    "p31_zip_multi.py",
    // §9 int / bool methods: `bit_length` / `bit_count` / `conjugate` /
    // `__index__`. `bit_*` route to BIGNUM-AWARE runtime counts (the pre-existing
    // `rt_int_bit_*` were rewired from a raw-i64 ABI to a tagged `Value` that
    // `classify_num`-splits fixnum vs heap `BigInt`); `conjugate`/`__index__`
    // return the receiver's int value via the new `rt_int_index` (bool→int 0/1,
    // bignum preserved), so a bool receiver is Int-typed (no i8/i64 verifier
    // clash). Covers fixnum/bignum/bool receivers, loop-var receivers, and a
    // comprehension/f-string cross. (Closed `test_builtins.py`'s line-60 blocker.)
    "p32_int_methods.py",
    // Zero-arg type conversions `int()`/`float()`/`bool()`/`str()` → their
    // defaults (`0`/`0.0`/`False`/`""`). `int`/`float`/`bool` fold to a constant
    // in lowering; `str()` folds to a `""` literal in the FRONTEND (interning
    // lives there — lowering's interner is immutable). A no-arg call must never
    // reach the unary `rt_builtin_*` (arity-mismatched, invalid IR). Covers the
    // empty-string shape (len/concat/join/truthiness), arithmetic use, the
    // with-args forms (no regression), and the unshadowed gate.
    "p33_zero_arg_conversions.py",
    // `isinstance(x, (A, B, ...))` tuple-of-types (§7) — pure frontend desugar to
    // an `or` of the existing per-element checks (`IsInstance` runtime / user
    // classes, `IsInstanceBuiltin` static fold / builtins), over a receiver
    // evaluated ONCE; nested type-tuples flatten, the empty tuple is `False`. No
    // new HIR node, no runtime/typeck/lowering change. Crosses builtin-only,
    // container-kind, user-class, MIXED user+builtin, nested, and empty tuples
    // with a side-effecting receiver (single-eval check) and use under
    // `if`/`and`/`or`/comprehension filters.
    "p34_isinstance_tuple.py",
    // `collections.Counter` (§10) — pure front-half WIRING over the pre-existing
    // `counter.rs` runtime (the `Counter` shares `DictObj` layout under
    // `TypeTagKind::Counter`), plus the runtime additions a differential-correct
    // Counter needs: `rt_counter_get` (missing key → 0, not KeyError), a
    // CPython-faithful `Counter({...})` repr in most-common order, the dict-family
    // seam guard (`Dict`/`DefaultDict`/`Counter`), and the Counter tag wired into
    // the generic `rt_obj_contains` / `rt_iter_value` / `rt_is_truthy` / len
    // dispatchers. Construction picks `rt_make_counter_empty` vs
    // `rt_make_counter_from_iter` by arity (the runtime normalizes any iterable to
    // an iterator); the result is typed `RuntimeObject(Counter)` so
    // `.most_common()/.total()/.update()/.subtract()` dispatch, and `Counter` is an
    // annotatable param/return type. Covers construction/repr, subscript (incl.
    // missing→0 and `+=`), len/`in`/iteration, keys/values/items, the methods,
    // truthiness (`not`/`if`/`bool`), and Counter through annotated functions.
    // `Counter(mapping)`, Counter arithmetic, and `.elements()` are out of scope.
    "p35_counter.py",
    // Comprehensive builtins suite (1633 lines) — LIFTED. Its blocker chain fell
    // across every phase as each fix unmasked the next: `issubclass`/`getattr`/
    // `hasattr`/`setattr` (semantics, p30) → multi-iterable `zip` into a typed
    // `list[tuple[…]]` slot (typeck, p31) → int methods (lowering, p32) → the
    // `hash()` builtin (codegen: a `K::Hash` arm wiring the pre-existing
    // `rt_builtin_hash`, plus `hash(None)` → CPython 3.12's `0xFCA86420` instead
    // of 0) → zero-arg `int()`/`float()`/`bool()` (folded to their default
    // constants, never an arity-mismatched `rt_builtin_*` call) → two-arg
    // `int(str, base)` (routed to `rt_str_to_int_with_base`, whose descriptor was
    // corrected to `[Tagged, Raw]` so the base rides a raw i64). Byte-matches
    // CPython end-to-end (hash/id values are never printed — only determinism and
    // fixed cases like `hash(True)==1` are asserted).
    "test_builtins.py",
    // Consolidated iteration/comprehension suite (LIFTED): its blockers fell in
    // sequence — attribute/subscript `for`-targets (p22), the standalone `iter()`
    // builtin + container `isinstance` (p23), `functools.reduce` (p24), and finally
    // lexicographic `min`/`max` over tuple-yielding gen-exprs (p25). Now byte-matches
    // CPython end-to-end.
    "test_iteration.py",
    // Consolidated control-flow suite (LIFTED): its sole remaining blocker was the
    // walrus operator `:=` (§2, closed by p26); a `+True`-yields-int divergence in
    // the same file rode along the `rt_obj_pos` fix. Byte-matches CPython end-to-end.
    "test_control_flow.py",
    // Consolidated list/tuple suite (lifted): tuple.index/count was its first §9
    // blocker (closed by p21's ContainerMethod path) and `list.remove()` its last
    // (now wired → `rt_list_remove`, ValueError on miss). Byte-matches CPython
    // end-to-end.
    "test_collections_list_tuple.py",
    // Consolidated dict/set/bytes suite (LIFTED): closed FOUR blockers, all
    // front-half wiring to an already-complete runtime. (1) set algebra
    // OPERATORS `|`/`&`/`-`/`^` (never gated before — they lowered to the
    // numeric `rt_obj_bitor`/… which TypeError at runtime) now route through
    // `try_container_binop` to the typed `Set*` ops. (2) `dict | dict` →
    // `ContainerOp::DictMerge` (`rt_dict_merge`, PEP 584). (3) `dict |=` is a
    // TRUE in-place merge via the new `BinOp::IOr` (frontend maps `|=`→`IOr`;
    // `rt_obj_ior` mutates `dict`/`set` in place and returns the same object so
    // the `x = x | y` rebind preserves aliases; numeric `|=` delegates to
    // `rt_obj_bitor`). (4) `d.fromkeys(keys[, value])` → `rt_dict_fromkeys`
    // (receiver discarded). Also fixed pre-existing `bytes(n)` zero-fill and
    // `bytes(str[, enc])` constructors (lowering routed every `bytes(...)` to
    // `rt_make_bytes_from_list`, SEGV on a non-list arg) → new `BytesZero` /
    // `BytesFromStr` ops over the existing runtime makers; and pinned the
    // hash-randomized set print to `sorted()`. Byte-matches CPython end-to-end
    // (debug + release).
    "test_collections_dict_set_bytes.py",
    // `collections` §10 — `defaultdict`, `deque` mutation/subscript, and
    // `OrderedDict`. Pure front-half WIRING over the pre-existing, byte-clean
    // runtime (`defaultdict.rs`, `deque.rs`, `rt_dict_move_to_end`,
    // `rt_dict_popitem_ordered`). A `defaultdict` is a new built-in generic base
    // (`BUILTIN_DEFAULTDICT_CLASS_ID`) whose repr is honestly `Heap(Dict(K, V))`,
    // so `dict_kv()` treats it as a dict everywhere (store, del, `.get`/`.keys`/
    // `.values`, `==`, iter) with zero new arms; the ONE divergence — the
    // auto-inserting subscript-read — is keyed on `is_defaultdict` before the
    // generic dict-read path. The factory Name (`int`/`list`/…) maps to a raw tag
    // (never lowered as a value — the old `undefined name 'set'` bug) that both
    // the frontend and typeck decode to the typed `V`. `deque` grows `dq[i]` /
    // `dq[i] = v` / `del dq[i]`, `maxlen` construction, and the `in` ring walk;
    // `OrderedDict.move_to_end`/`popitem(last)` extend the dict method surface.
    // Byte-matches CPython end-to-end (debug + release).
    "test_collections.py",
    // Verified-clean lifts (compile + runtime-diff MATCH at ~0 code): PEP 563
    // `from __future__ import annotations` (string-form annotations ignored at
    // runtime), a GC smoke test, the generator surface, and print() formatting/
    // sep/end/flush. Locks in working behavior. (`test_stdlib_urllib.py` is
    // network — stays in NET_TESTS, not lifted.)
    "test_future_annotations.py",
    "test_gc_simple.py",
    "test_generators.py",
    "test_print_output.py",
    // Consolidated functions suite (LIFTED): its last out-of-scope root was a
    // genuinely-`Dyn` callee that is a native closure (a curried `chain(1)(2)(3)`
    // whose intermediate returns widen to `Dyn`; an unannotated decorator's
    // `func()`). The uniform value-call convention closes it — every closure's
    // slot 0 is one arity-generic `(args, kwargs) → Value` thunk, so a `Dyn`
    // callee is callable through the single indirect ABI (the precise `Callable`
    // typing is now only a devirtualization hint). A pre-existing inliner bug
    // (a value-returning callee's bare-`return`/fall-off left the call's `dst`
    // stale under -O) rode along its `_test_mixed_value_void` probe and is fixed.
    // Byte-matches CPython end-to-end (debug + release).
    "test_functions.py",
    // Decorator factories (parameterized decorators, stacked decorators, a
    // wrapper whose decorated slot widens to `Dyn`). LIFTED by the uniform
    // value-call convention: the `plain_deco` shape — a wrapper without a
    // `Callable[...]` return annotation, so the decorated callable is `Dyn` —
    // now calls through the single arity-generic indirect ABI instead of
    // rejecting. Byte-matches CPython end-to-end.
    "test_decorator_factory.py",
    // §5 — class decorators: a side-effecting decorator that returns the class
    // unchanged + the parameterized factory form + stacking. The "class value"
    // is the class-id int (the `@classmethod`/`object.__new__` convention); the
    // class name stays bound to its id via the static `class_map`, so `C(...)`
    // still constructs. Class-replacing / class-as-stored-value decorators stay
    // out of scope (classes aren't first-class values).
    "test_class_decorators.py",
    // Gradual-completeness method dispatch — a `Dyn`/`Union`-typed receiver
    // calls methods at run time via the unified `rt_obj_method` (the CPython
    // `type(obj).method` model): container methods (`.append`/`.get`/`.add`/…
    // on a heterogeneous dict's `Dyn` values) route to the typed `rt_*` family
    // (Phase A); user methods on a genuinely-`Dyn` instance (a heterogeneous
    // list's elements) route through each method's uniform thunk, incl.
    // inherited (self coerces C→B), overridden, default + keyword args (Phase
    // B). Inference precision stays a pure performance lever (Invariant 2).
    "test_gradual_methods.py",
    // Lazy user-class iterator protocol — `for x in inst` / `iter()` / `next()`
    // over a class defining `__iter__`/`__next__`. The for-loop drives a runtime
    // `IteratorObj{kind=Instance}` whose `IterNext` calls the class's compiled
    // `<iternext>` thunk (`try: return self.__next__() except StopIteration:
    // return UNBOUND`), bridging the user iterator's exception protocol to the
    // runtime's exhausted-flag protocol: self-iterator, separate iterator,
    // inherited `__next__` (reuses the base thunk), empty / break-early, and a
    // non-`StopIteration` raise propagating out.
    "p42_iter_protocol.py",
    // Heterogeneous-numeric tuple iteration: `for x in (1.5, 1)` must not infer
    // the element type as `float` via the numeric tower. The runtime tuple holds
    // each element tagged (boxed float AND tagged int) and the iterator yields
    // Tagged, so a `Raw(F64)` element type raw-unboxes the tagged int as a
    // FloatObj pointer (SIGSEGV). `iter_elem_ty` now routes the tuple-element
    // fold through the Raw-uniformity guard — mixed numeric tuples iterate as
    // Tagged, homogeneous ones stay precise. (Autograd-accumulation regression.)
    "p43_hetero_tuple_iter.py",
    // Numeric tower int->float at the remaining slot seams (PLAN §8, closed): a
    // `float` PARAMETER (free-fn / method positional + keyword / constructor), a
    // `float` GLOBAL, and a `float` FIELD written from an int/bool. The param
    // seam takes lowering's checked `coerce_value` (`rt_unbox_float`); the
    // global/field seams take `box_float_for_slot` (checked unbox then `BoxFloat`
    // to a genuine `FloatObj`, keeping the slot's unchecked read sound, A2).
    // Divergence-safe: asserts via `==` (3 == 3.0) and prints only float-forced
    // results. Includes the bignum->float arm (2**62) and an int->float-global
    // feeding a float-param free function. Sibling of p16 (return / local seams).
    "p44_numeric_tower_seams.py",
    // In-method instance-field annotations as field-type contracts (PLAN §8
    // follow-up): `self.<name>: T = v` (and the bare `self.<name>: T`) inside a
    // method declares the field's type like a class-level `name: T` — a frontend
    // pre-scan collects them into the class's field annotations before typeck. A
    // `float` field fed an int routes through the §8 SetField box. Covers
    // __init__ + non-__init__ + nested-block annotations, the no-value
    // declaration form, a float-field→float-param interaction, and the bignum
    // arm. Divergence-safe (`==` + float-forced prints), sibling of p44.
    "p45_in_method_field_annot.py",
    // The full class-feature corpus (3749 lines), now byte-exact 95/95 end-to-
    // end. Caps the OOP/dispatch cluster above: `__init__`/`__slots__`,
    // inheritance + C3 MRO, dunders (arithmetic/compare/`__lt__` sort ordering,
    // `__bool__`/`__len__`, `__index__`, reflected), the lazy iterator protocol,
    // heterogeneous-tuple iteration, gradual `Dyn`-receiver dispatch, default
    // `object` repr, and `sorted`/`list.sort`/`min`/`max` over instances via
    // `__lt__`. The standing regression guard for class semantics.
    "test_classes.py",
    // Generics + type system (PLAN §3 + §12). Monomorphization (already working:
    // generic free functions, PEP 695 `def f[V]` / `class C[K]`, `Generic[T]`,
    // TypeVar constraints/bounds, `Literal[...]`) plus the §3 frontend/Protocol
    // plumbing: `...` stub bodies, `type X = T` / `X: TypeAlias = T` aliases,
    // `@runtime_checkable Protocol` (erased to `Dyn`, gradual `rt_obj_method`
    // dispatch), subscripted instance annotations (`IntWrapper[int]`), and
    // structural `isinstance(obj, P)` via `rt_obj_has_method`.
    "test_generics.py",
    "test_types_system.py",
    // §7 — `isinstance` on a gradual / `Any` receiver with flow-sensitive
    // narrowing: `def f(data: Any)` then `if isinstance(data, str): len(data)`
    // (the runtime tag query + branch narrowing), plus the always-True/always-False
    // statically-typed cases. The dead-code-warning regression guard.
    "test_dead_code_warnings.py",
];

/// Network-dependent entries, run (self-checking) ONLY when `PYAOT_NET_TESTS` is
/// set — `test_stdlib_urllib.py` exercises the live `urlopen`/`urlretrieve` paths
/// its `_core` sibling excludes. It is offline-safe: every network section is
/// wrapped in `try/except IOError`, so a connection failure skips to the same
/// final "All urllib tests passed!" line on both pyaot and CPython.
const NET_TESTS: &[&str] = &["test_stdlib_urllib.py"];

#[test]
fn phase_corpus_matches_cpython() {
    let pyaot = PathBuf::from(env!("CARGO_BIN_EXE_pyaot"));
    let target_dir = pyaot
        .parent()
        .expect("pyaot binary has a parent target dir")
        .to_path_buf();
    let runtime_lib = ensure_runtime_lib(&target_dir);
    let corpus_dir = workspace_root().join("corpus");
    let out_dir = std::env::temp_dir().join("pyaot_phase1");
    std::fs::create_dir_all(&out_dir).expect("create temp out dir");

    // The default gate is PHASE_CORPUS; `PYAOT_NET_TESTS` adds the live-network
    // entries (self-checking).
    let mut entries: Vec<&str> = PHASE_CORPUS.to_vec();
    if std::env::var_os("PYAOT_NET_TESTS").is_some() {
        entries.extend_from_slice(NET_TESTS);
    }

    for entry in &entries {
        let source = corpus_dir.join(entry);
        assert!(
            source.exists(),
            "corpus file {} does not exist",
            source.display()
        );

        let exe = out_dir.join(Path::new(entry).file_stem().unwrap());

        // ── Compile with pyaot. ──
        let compile = Command::new(&pyaot)
            .arg(&source)
            .arg("-o")
            .arg(&exe)
            .arg("--runtime-lib")
            .arg(&runtime_lib)
            .output()
            .expect("failed to spawn pyaot");
        assert!(
            compile.status.success(),
            "pyaot failed to compile {}:\n--- stdout ---\n{}\n--- stderr ---\n{}",
            entry,
            String::from_utf8_lossy(&compile.stdout),
            String::from_utf8_lossy(&compile.stderr),
        );

        // ── Run the compiled executable. ──
        let compiled = Command::new(&exe)
            .output()
            .unwrap_or_else(|e| panic!("failed to run compiled {}: {e}", exe.display()));
        assert!(
            compiled.status.success(),
            "compiled {} exited with failure: {:?}\nstderr:\n{}",
            entry,
            compiled.status,
            String::from_utf8_lossy(&compiled.stderr),
        );

        // ── Run the CPython oracle live. ──
        let oracle = Command::new("python3")
            .arg(&source)
            .output()
            .expect("failed to spawn python3");
        assert!(
            oracle.status.success(),
            "python3 failed on {}:\n{}",
            entry,
            String::from_utf8_lossy(&oracle.stderr),
        );

        // ── Compare stdout: byte-for-byte, or final-line for self-checking
        // entries (both already exited 0 above). ──
        if SELF_CHECKING.contains(entry) || NET_TESTS.contains(entry) {
            let last = |out: &[u8]| -> String {
                String::from_utf8_lossy(out)
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or_default()
                    .to_string()
            };
            assert_eq!(
                last(&compiled.stdout),
                last(&oracle.stdout),
                "final self-check line mismatch for {entry} (pyaot vs CPython)",
            );
        } else {
            assert_eq!(
                String::from_utf8_lossy(&compiled.stdout),
                String::from_utf8_lossy(&oracle.stdout),
                "stdout mismatch for {entry} (pyaot vs CPython)",
            );
        }
    }
}

/// Locate (and rebuild) the runtime staticlib next to the `pyaot` binary,
/// matching the test's build profile. The runtime is **not** a Cargo
/// dependency of the CLI (it's linked from a `.a`), so `cargo test` alone does
/// not produce it — build it here so the gate is self-contained (PITFALLS B9).
///
/// `cargo build` runs UNCONDITIONALLY (it is incremental — a no-op when the
/// runtime sources are unchanged, a fraction of a second). Returning a stale
/// `.a` just because it exists silently links yesterday's runtime: a runtime
/// fix would not take effect and could SIGSEGV at link-clean compile time
/// (this bit the Phase-10 `rt_sorted` ABI change).
fn ensure_runtime_lib(target_dir: &Path) -> PathBuf {
    let lib = target_dir.join("libpyaot_runtime.a");

    // The profile is the parent dir's name (e.g. `debug` / `release`).
    let profile = target_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("debug");

    let mut cmd = Command::new(env!("CARGO"));
    cmd.arg("build").arg("-p").arg("pyaot-runtime");
    if profile == "release" {
        cmd.arg("--release");
    }
    let build = cmd
        .output()
        .expect("failed to spawn cargo build for runtime");
    assert!(
        build.status.success(),
        "failed to build pyaot-runtime staticlib:\n{}",
        String::from_utf8_lossy(&build.stderr),
    );
    assert!(
        lib.exists(),
        "runtime staticlib still missing after build: {}",
        lib.display()
    );
    lib
}

/// Workspace root = two levels up from this crate's manifest dir (`crates/cli`).
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("canonicalize workspace root")
}
