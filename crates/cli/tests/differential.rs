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
    // inference + sum() and with `*seq` spread (§13). The aspirational
    // `test_collections_list_tuple.py` / `test_iteration.py` stay OFF the gate on
    // UNRELATED gaps (tuple-slice → fixed-arity tuple annotation; bare
    // attribute/subscript `for`-targets) — the nested-unpacking shapes themselves
    // compile clean here.
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
    // end-to-end and is lifted onto the gate. `test_builtins.py` stays OFF (it
    // still needs `map`/`filter`/`format`).
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
    // Verified-clean lifts (compile + runtime-diff MATCH at ~0 code): PEP 563
    // `from __future__ import annotations` (string-form annotations ignored at
    // runtime), a GC smoke test, the generator surface, and print() formatting/
    // sep/end/flush. Locks in working behavior. (`test_stdlib_urllib.py` is
    // network — stays in NET_TESTS, not lifted.)
    "test_future_annotations.py",
    "test_gc_simple.py",
    "test_generators.py",
    "test_print_output.py",
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
