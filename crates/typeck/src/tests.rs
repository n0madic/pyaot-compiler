//! Inferred-type unit tests: drive real Python through parse → resolve → infer
//! and assert the solver's `SemTy` for representative locals and expressions.

use pyaot_hir::{HirExprKind, HirFunction, HirModule};
use pyaot_types::SemTy;
use pyaot_utils::StringInterner;

use super::infer;

/// Parse `src`, resolve names, run inference; return the typed module + interner.
fn typed(src: &str) -> (HirModule, StringInterner) {
    let mut interner = StringInterner::new();
    let mut module = pyaot_frontend_python::parse(src, &mut interner).expect("parse");
    let ns = pyaot_hir::NamespaceTable::single(module.functions.len());
    let resolve = pyaot_semantics::resolve(&mut module, &ns, &interner).expect("resolve");
    let mut classes =
        pyaot_semantics::collect_classes(&module, &ns, &interner).expect("collect_classes");
    infer(&mut module, &resolve, &mut classes, &interner).expect("infer");
    (module, interner)
}

/// Parse + resolve, then return the `infer` result (for testing rejections).
fn try_infer(src: &str) -> pyaot_diagnostics::Result<()> {
    let mut interner = StringInterner::new();
    let mut module = pyaot_frontend_python::parse(src, &mut interner).expect("parse");
    let ns = pyaot_hir::NamespaceTable::single(module.functions.len());
    let resolve = pyaot_semantics::resolve(&mut module, &ns, &interner).expect("resolve");
    let mut classes =
        pyaot_semantics::collect_classes(&module, &ns, &interner).expect("collect_classes");
    infer(&mut module, &resolve, &mut classes, &interner)
}

/// The synthetic `__main__` function (module body).
fn main_fn(module: &HirModule) -> &HirFunction {
    module.function(module.main)
}

/// Inferred type of the named local in `func`.
fn local_ty(func: &HirFunction, interner: &StringInterner, name: &str) -> SemTy {
    func.locals
        .iter()
        .find(|l| interner.resolve(l.name) == name)
        .unwrap_or_else(|| panic!("no local named {name}"))
        .ty
        .clone()
}

#[test]
fn infers_unannotated_float_accumulator() {
    // The plan's milestone-3a marker: an unannotated float loop accumulator must
    // infer `Float` despite the cyclic `acc = acc + 1.5` dependency.
    let (m, i) = typed("acc = 0.0\nfor n in range(3):\n    acc = acc + 1.5\nprint(acc)\n");
    assert_eq!(local_ty(main_fn(&m), &i, "acc"), SemTy::Float);
}

#[test]
fn infers_unannotated_int_accumulator() {
    let (m, i) = typed("total = 0\nfor n in range(5):\n    total = total + n\nprint(total)\n");
    assert_eq!(local_ty(main_fn(&m), &i, "total"), SemTy::Int);
    // The range cursor / loop variable are int as well.
    assert_eq!(local_ty(main_fn(&m), &i, "n"), SemTy::Int);
}

#[test]
fn infers_bool_local() {
    let (m, i) = typed("flag = True\nflag = 1 < 2\nprint(flag)\n");
    assert_eq!(local_ty(main_fn(&m), &i, "flag"), SemTy::Bool);
}

#[test]
fn infers_str_local() {
    let (m, i) = typed("s = \"a\"\ns = \"bc\"\nprint(s)\n");
    assert_eq!(local_ty(main_fn(&m), &i, "s"), SemTy::Str);
}

#[test]
fn mixed_numeric_local_stays_tagged() {
    // A slot assigned both an int and a float cannot take a single `Raw(F64)`
    // representation soundly — the solver must fall back to `Dyn` (→ Tagged),
    // never a collapsed `Float` (PITFALLS A2/B6).
    let (m, i) = typed("x = 5\nx = 2.5\nprint(x)\n");
    assert_eq!(local_ty(main_fn(&m), &i, "x"), SemTy::Dyn);
}

#[test]
fn true_division_is_float() {
    // `7 / 2 == 3.5`: true division yields `float` even for int operands.
    let (m, _) = typed("print(7 / 2)\n");
    let f = main_fn(&m);
    let has_float_div = f.exprs.iter().any(|(_, e)| {
        matches!(
            e.kind,
            HirExprKind::BinOp {
                op: pyaot_hir::BinOp::Div,
                ..
            }
        ) && e.ty == SemTy::Float
    });
    assert!(has_float_div, "true-division expr should infer Float");
}

#[test]
fn arithmetic_promotes_int_and_float() {
    let (m, _) = typed("print(3 + 1.5)\n");
    let f = main_fn(&m);
    let add = f
        .exprs
        .iter()
        .find(|(_, e)| {
            matches!(
                e.kind,
                HirExprKind::BinOp {
                    op: pyaot_hir::BinOp::Add,
                    ..
                }
            )
        })
        .expect("add expr");
    assert_eq!(add.1.ty, SemTy::Float);
}

#[test]
fn bitwise_on_ints_is_int() {
    let (m, _) = typed("print(5 & 3)\n");
    let f = main_fn(&m);
    let band = f
        .exprs
        .iter()
        .find(|(_, e)| {
            matches!(
                e.kind,
                HirExprKind::BinOp {
                    op: pyaot_hir::BinOp::BitAnd,
                    ..
                }
            )
        })
        .expect("bitand expr");
    assert_eq!(band.1.ty, SemTy::Int);
}

#[test]
fn call_result_takes_callee_return_type() {
    let src = "def add(a: int, b: int) -> int:\n    return a + b\n\n\nr = add(1, 2)\nprint(r)\n";
    let (m, i) = typed(src);
    assert_eq!(local_ty(main_fn(&m), &i, "r"), SemTy::Int);
}

#[test]
fn builtin_call_result_types() {
    let (m, i) = typed("a = abs(-5)\nb = float(7)\nc = str(42)\nd = len(\"hi\")\n");
    let f = main_fn(&m);
    assert_eq!(local_ty(f, &i, "a"), SemTy::Int);
    assert_eq!(local_ty(f, &i, "b"), SemTy::Float);
    assert_eq!(local_ty(f, &i, "c"), SemTy::Str);
    assert_eq!(local_ty(f, &i, "d"), SemTy::Int);
}

#[test]
fn annotation_is_authoritative() {
    // An explicit annotation drives the type even when the value would infer
    // differently; inference must not override it.
    let (m, i) = typed("b: float = 2.5\nprint(b)\n");
    assert_eq!(local_ty(main_fn(&m), &i, "b"), SemTy::Float);
}

// ── unboxed-slot boundary checks (reject-don't-crash, PITFALLS A2) ──

#[test]
fn accepts_int_into_float_parameter() {
    // §8 numeric tower (param seam closed): `poly(3)` for `def poly(a: float)` is
    // a real (checked) coercion at the param boundary — lowering's `coerce_value`
    // emits `rt_unbox_float` (int→f64, bignum arm), so the `Raw(F64)` param holds
    // a genuine f64. CPython keeps the raw int; pyaot coerces (annotation-as-
    // contract, observable only via repr-print). A `bool` arg coerces the same way.
    assert!(
        try_infer("def poly(a: float) -> float:\n    return a + 1.0\n\n\nprint(poly(3))\n").is_ok(),
        "int into a float parameter is a checked coercion (§8)"
    );
    assert!(
        try_infer("def poly(a: float) -> float:\n    return a + 1.0\n\n\nprint(poly(True))\n")
            .is_ok(),
        "bool into a float parameter is a checked coercion (§8)"
    );
}

#[test]
fn rejects_int_into_bool_parameter() {
    // §8 numeric tower is asymmetric: the bool seam stays Dyn-ONLY. A statically
    // `int` arg into a `bool` param must stay REJECTED — a contract-coerced
    // `bool(3)` is `True`, but `3 == True` is `False`, so silently `int→bool`
    // would diverge from CPython *observably* (not just via repr-print, unlike
    // int→float). The `eff = allow_coerce` widening must not over-admit this.
    let src = "def flag(a: bool) -> bool:\n    return a\n\n\nprint(flag(3))\n";
    assert!(
        try_infer(src).is_err(),
        "static int into a bool parameter must stay rejected (bool seam is Dyn-only)"
    );
    // A matching `bool` arg still compiles.
    assert!(try_infer("def flag(a: bool) -> bool:\n    return a\n\n\nprint(flag(True))\n").is_ok());
}

#[test]
fn accepts_float_into_float_parameter() {
    let src = "def poly(a: float) -> float:\n    return a + 1.0\n\n\nprint(poly(3.0))\n";
    assert!(
        try_infer(src).is_ok(),
        "float into a float parameter must compile"
    );
}

#[test]
fn accepts_int_into_float_local() {
    // §8 numeric tower: an `int` into an annotated `: float` LOCAL is a real
    // (checked) coercion at the store, not a contract violation — the slot is a
    // genuine `Raw(F64)` register, so the int boxes to `Tagged` then takes the
    // checked `rt_unbox_float` unbox to a real f64. (CPython keeps the raw int;
    // pyaot coerces to `5.0` — the annotation-as-contract divergence, observable
    // only via repr-print.) Holds for both a `__main__` top-level local and a
    // function-body local.
    assert!(try_infer("x: float = 5\nprint(x + 0.0)\n").is_ok());
    assert!(
        try_infer("def g() -> float:\n    y: float = 7\n    return y + 0.0\n\n\nprint(g())\n")
            .is_ok()
    );
}

#[test]
fn accepts_int_returned_as_float() {
    // §8 numeric tower: an `int`/`bool` through a `-> float` return is a real
    // (checked) coercion at the return terminator, not a contract violation.
    assert!(try_infer("def f() -> float:\n    return 5\n\n\nprint(f())\n").is_ok());
    assert!(try_infer("def b() -> float:\n    return True\n\n\nprint(b())\n").is_ok());
}

#[test]
fn accepts_int_into_float_global_and_field() {
    // §8 numeric tower (global / field seams closed): a `float` global / field is
    // a tagged slot read back via an unchecked `UnboxFloat`, so lowering's
    // `box_float_for_slot` coerces the int to a genuine f64 then re-boxes it to a
    // `FloatObj` at the store — the slot's read stays sound (A2). A genuine
    // cross-function `float` GLOBAL (`x` is read inside `f`, so it lowers to a
    // tagged `GlobalSet` slot, not a `Raw(F64)` `__main__` local):
    assert!(
        try_infer("x: float = 5\ndef f() -> float:\n    return x + 1.0\n\n\nprint(f())\n").is_ok()
    );
    // A `float` FIELD written from an int (here via a `float` ctor param, the
    // store-side box):
    assert!(try_infer(
        "class C:\n    def __init__(self, v: float):\n        self.v = v\n\n\nprint(C(5).v + 0.5)\n"
    )
    .is_ok());
}

// ── typed-heap boundary checks (Phase 4: `TaggedToHeap` is reinterpret-by-type
//    too, so a concrete non-container into a container/str slot is rejected
//    loudly — symmetric with the float/bool contract — while `Dyn` passes) ──

#[test]
fn rejects_concrete_nonmatch_into_typed_heap_slot() {
    // `f(5)` for a `list[int]` param SIGSEGVs without this guard (CPython raises a
    // clean TypeError); reject it at compile time instead.
    assert!(
        try_infer("def f(a: list[int]) -> int:\n    return len(a)\n\n\nprint(f(5))\n").is_err()
    );
    // Annotated local / return / str slots get the same contract.
    assert!(try_infer("x: list[int] = 5\nprint(x)\n").is_err());
    assert!(try_infer("s: str = 42\nprint(s)\n").is_err());
    assert!(try_infer("def h() -> dict:\n    return 7\n\n\nprint(h())\n").is_err());
}

#[test]
fn accepts_matching_and_gradual_into_typed_heap_slot() {
    // A matching container value, the empty-literal bootstrap, and a gradual `Dyn`
    // value (a future runtime guard) all compile.
    assert!(try_infer("x: list[int] = [1, 2, 3]\nprint(x)\n").is_ok());
    assert!(try_infer("x: list[int] = []\nprint(len(x))\n").is_ok());
    assert!(try_infer(
        "def pick(flag):\n    if flag:\n        return [1]\n    return [2]\n\n\nx: list = pick(True)\nprint(x)\n"
    )
    .is_ok());
}

// ── containers (Phase 4) ──

#[test]
fn infers_list_literal_element_type() {
    let (m, i) = typed("xs = [1, 2, 3]\nprint(xs)\n");
    assert_eq!(local_ty(main_fn(&m), &i, "xs"), SemTy::list_of(SemTy::Int));
}

#[test]
fn list_literal_joins_heterogeneous_elements() {
    // `[1, 2.0]` must NOT numeric-promote the element slot to `float`: the
    // stored tagged int would be blindly unboxed as an f64 on read (PITFALLS
    // A2). The Raw-uniformity guard demotes the slot to `Dyn` (tagged
    // elements) — CPython semantics keep `xs[0]` an int.
    let (m, i) = typed("xs = [1, 2.0]\nprint(xs)\n");
    assert_eq!(local_ty(main_fn(&m), &i, "xs"), SemTy::list_of(SemTy::Dyn));
}

#[test]
fn infers_dict_and_tuple_literal_types() {
    let (m, i) = typed("d = {\"a\": 1}\nt = (1, \"two\")\nprint(d)\nprint(t)\n");
    let f = main_fn(&m);
    assert_eq!(local_ty(f, &i, "d"), SemTy::dict_of(SemTy::Str, SemTy::Int));
    assert_eq!(
        local_ty(f, &i, "t"),
        SemTy::tuple_of(vec![SemTy::Int, SemTy::Str])
    );
}

#[test]
fn subscript_read_takes_element_type() {
    let (m, i) = typed("xs = [10, 20]\ny = xs[0]\nprint(y)\n");
    assert_eq!(local_ty(main_fn(&m), &i, "y"), SemTy::Int);
}

#[test]
fn len_and_membership_result_types() {
    let (m, i) = typed("xs = [1, 2]\nn = len(xs)\nf = 1 in xs\nprint(n)\nprint(f)\n");
    let f = main_fn(&m);
    assert_eq!(local_ty(f, &i, "n"), SemTy::Int);
    assert_eq!(local_ty(f, &i, "f"), SemTy::Bool);
}

#[test]
fn empty_literal_bootstrap_seeds_element_type() {
    // PITFALLS B4: `x: list[int] = []` must materialize the empty literal's type
    // as the annotated container type (so lowering picks tagged element slots),
    // not the `list[Never]` it solves to in isolation.
    let (m, i) = typed("x: list[int] = []\nprint(x)\n");
    let f = main_fn(&m);
    assert_eq!(local_ty(f, &i, "x"), SemTy::list_of(SemTy::Int));
    // The `[]` literal expr itself now carries the seeded type.
    let empty = f
        .exprs
        .iter()
        .find(|(_, e)| matches!(e.kind, HirExprKind::ListLit { ref elems } if elems.is_empty()))
        .expect("empty list literal");
    assert_eq!(empty.1.ty, SemTy::list_of(SemTy::Int));
}

#[test]
fn comprehension_and_builtins_result_types() {
    let (m, i) = typed(
        "a = [x for x in range(3)]\nb = sorted([3, 1])\nc = sum([1, 2])\nprint(a)\nprint(b)\nprint(c)\n",
    );
    let f = main_fn(&m);
    // Comprehension / sorted produce lists; sum of ints is int.
    assert!(local_ty(f, &i, "a").list_elem().is_some());
    assert!(local_ty(f, &i, "b").list_elem().is_some());
    assert_eq!(local_ty(f, &i, "c"), SemTy::Int);
}

#[test]
fn enumerate_and_zip_yield_tuple_iterators() {
    let (m, i) = typed("e = enumerate([1, 2])\nz = zip([1], [2])\nprint(e)\nprint(z)\n");
    let f = main_fn(&m);
    assert!(matches!(local_ty(f, &i, "e"), SemTy::Iterator(_)));
    assert!(matches!(local_ty(f, &i, "z"), SemTy::Iterator(_)));
}

#[test]
fn container_method_result_types() {
    let (m, i) = typed(
        "xs = [1, 2, 3]\np = xs.pop()\nn = xs.count(2)\nc = xs.copy()\nprint(p)\nprint(n)\nprint(c)\n",
    );
    let f = main_fn(&m);
    assert_eq!(local_ty(f, &i, "p"), SemTy::Int); // list[int].pop() → int
    assert_eq!(local_ty(f, &i, "n"), SemTy::Int); // .count() → int
    assert!(local_ty(f, &i, "c").list_elem().is_some()); // .copy() → list
}

#[test]
fn dict_view_method_result_types() {
    let (m, i) = typed("d = {\"a\": 1}\nk = d.keys()\nv = d.values()\nprint(k)\nprint(v)\n");
    let f = main_fn(&m);
    assert!(local_ty(f, &i, "k").list_elem().is_some());
    assert!(local_ty(f, &i, "v").list_elem().is_some());
}

#[test]
fn unknown_method_parses_and_infers_dyn() {
    // Phase 5 (D2): the frontend no longer rejects unknown method names — they
    // become a `MethodCall` carrying the name. A non-container method on a
    // container receiver infers `Dyn` here; the *rejection* moves to lowering
    // (where the receiver type selects the dispatch). Parse + infer must succeed.
    let parses = |src: &str| {
        let mut interner = StringInterner::new();
        pyaot_frontend_python::parse(src, &mut interner).is_ok()
    };
    assert!(parses("xs = [1]\nxs.frobnicate()\nprint(xs)\n"));
    assert!(try_infer("xs = [1]\nxs.frobnicate()\nprint(xs)\n").is_ok());
}

#[test]
fn general_for_infers_element_type() {
    // Iterating a `list[int]` makes the loop variable `int` (via the iterator
    // element type flowing through `Iter` → `IterNext`).
    let (m, i) = typed("for x in [10, 20, 30]:\n    print(x)\n");
    assert_eq!(local_ty(main_fn(&m), &i, "x"), SemTy::Int);
}

#[test]
fn rejects_unpack_arity_mismatch() {
    // A literal-sequence unpack with the wrong arity is a parse-time error.
    let parses = |src: &str| {
        let mut interner = StringInterner::new();
        pyaot_frontend_python::parse(src, &mut interner).is_ok()
    };
    assert!(!parses("a, b = 1, 2, 3\nprint(a)\n"));
    assert!(!parses("a, b, c = [1, 2]\nprint(a)\n"));
    // A correctly-sized unpack still compiles.
    assert!(parses("a, b = 1, 2\nprint(a)\nprint(b)\n"));
}

#[test]
fn unannotated_empty_literal_stays_never_element() {
    // No annotation → `list[Never]` (→ tagged elements at lowering). Still correct.
    let (m, i) = typed("x = []\nprint(x)\n");
    assert_eq!(local_ty(main_fn(&m), &i, "x"), SemTy::list_of(SemTy::Never));
}

#[test]
fn accepts_tagged_float_value_into_float_local() {
    // A float-*typed* value that lowers to a tagged boxed float (true division,
    // a `float()` call) is a legitimate UnboxFloat — it must NOT be rejected.
    assert!(try_infer("x: float = 7.0 / 2.0\nprint(x)\n").is_ok());
    assert!(try_infer("y: float = float(3)\nprint(y)\n").is_ok());
}

// ── classes (Phase 5A) ──

const WIDGET_SRC: &str = "\
class Widget:
    def __init__(self, w: int, h: int):
        self.w = w
        self.h = h

    def area(self) -> int:
        return self.w * self.h

x = Widget(3, 4)
a = x.area()
ww = x.w
print(a)
print(ww)
";

#[test]
fn class_construction_infers_class_type() {
    let (m, i) = typed(WIDGET_SRC);
    let f = main_fn(&m);
    match local_ty(f, &i, "x") {
        SemTy::Class { name, .. } => assert_eq!(i.resolve(name), "Widget"),
        other => panic!("expected `x: Widget`, got {other:?}"),
    }
}

#[test]
fn class_method_call_takes_declared_return() {
    // `a = x.area()` → the method's declared `-> int`.
    let (m, i) = typed(WIDGET_SRC);
    assert_eq!(local_ty(main_fn(&m), &i, "a"), SemTy::Int);
}

#[test]
fn class_attribute_read_takes_field_type() {
    // `ww = x.w` → the field's best-effort type (`self.w = w` of param `w: int`).
    let (m, i) = typed(WIDGET_SRC);
    assert_eq!(local_ty(main_fn(&m), &i, "ww"), SemTy::Int);
}

#[test]
fn in_method_field_annotation_is_a_type_contract() {
    // `self.x: float = v` (v an int param) inside a method declares a `float`
    // FIELD — the in-method annotation is honored as a field-type contract,
    // exactly like a class-level `x: float`. A read of the
    // field is then `float`, NOT the `int` best-effort type of the written value;
    // this distinguishes the contract (the implemented level 2) from a merely
    // decorative annotation (level 1, which would leave the field `int`). The
    // int→float store itself is the §8 SetField numeric-tower seam.
    let src = "\
class Box:
    def __init__(self, v: int) -> None:
        self.x: float = v

    def get(self) -> float:
        return self.x


b = Box(5)
val = b.x
print(val)
print(b.get())
";
    let (m, i) = typed(src);
    assert_eq!(
        local_ty(main_fn(&m), &i, "val"),
        SemTy::Float,
        "in-method `self.x: float` must type the field as a float contract"
    );
}

#[test]
fn bare_in_method_field_annotation_declares_field_type() {
    // A bare `self.x: float` (no value) is a no-op store in CPython, but here it
    // still DECLARES the field type — the field is read back as `float` even
    // though the only write is an int. Proves the no-value branch of the
    // in-method annotation scan registers the contract.
    let src = "\
class Box:
    def __init__(self, v: int) -> None:
        self.x: float
        self.x = v

    def get(self) -> float:
        return self.x


b = Box(5)
val = b.x
print(val)
print(b.get())
";
    let (m, i) = typed(src);
    assert_eq!(local_ty(main_fn(&m), &i, "val"), SemTy::Float);
}

#[test]
fn class_typed_return_annotation_resolves() {
    // `def make() -> Widget` resolves the class-name annotation to `Class`.
    let src = "\
class Widget:
    def __init__(self, w: int):
        self.w = w

def make(w: int) -> Widget:
    return Widget(w)

g = make(5)
print(g.w)
";
    let (m, i) = typed(src);
    match local_ty(main_fn(&m), &i, "g") {
        SemTy::Class { name, .. } => assert_eq!(i.resolve(name), "Widget"),
        other => panic!("expected `g: Widget`, got {other:?}"),
    }
}

#[test]
fn float_field_round_trips_through_uniform_storage() {
    // A `float` field reads back via UnboxFloat from the uniform tagged slot; the
    // write of a matching float value must pass the repr-boundary contract.
    let src = "\
class P:
    def __init__(self, x: float):
        self.x = x

    def get(self) -> float:
        return self.x

p = P(1.5)
v = p.get()
print(v)
";
    assert!(try_infer(src).is_ok());
    let (m, i) = typed(src);
    assert_eq!(local_ty(main_fn(&m), &i, "v"), SemTy::Float);
}

// ── dunders (Phase 5C) ──

const VECTOR_SRC: &str = "\
class Vector:
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y
    def __add__(self, other: Vector) -> Vector:
        return Vector(self.x + other.x, self.y + other.y)
    def __mul__(self, k: int) -> Vector:
        return Vector(self.x * k, self.y * k)
    def __eq__(self, other: Vector) -> bool:
        return self.x == other.x and self.y == other.y
a = Vector(1, 2)
b = Vector(3, 4)
s = a + b
m = a * 3
e = a == b
";

#[test]
fn class_binop_takes_dunder_return_type() {
    // `a + b` / `a * 3` type as the dunder's declared return (`Vector`), so field
    // access on the result is statically resolvable.
    let (md, i) = typed(VECTOR_SRC);
    let f = main_fn(&md);
    for name in ["s", "m"] {
        match local_ty(f, &i, name) {
            SemTy::Class { name: n, .. } => assert_eq!(i.resolve(n), "Vector"),
            other => panic!("expected `{name}: Vector`, got {other:?}"),
        }
    }
}

#[test]
fn class_eq_is_bool() {
    let (md, i) = typed(VECTOR_SRC);
    assert_eq!(local_ty(main_fn(&md), &i, "e"), SemTy::Bool);
}

#[test]
fn class_getitem_takes_dunder_return_type() {
    let src = "\
class Box:
    def __init__(self, data: list[int]):
        self.data = data
    def __getitem__(self, i: int) -> int:
        return self.data[i]
b = Box([1, 2, 3])
v = b[0]
";
    let (md, i) = typed(src);
    assert_eq!(local_ty(main_fn(&md), &i, "v"), SemTy::Int);
}

// ── generics (Phase 5E) ──

const GENERIC_SRC: &str = "\
from typing import TypeVar, Generic
T = TypeVar(\"T\")
class Box(Generic[T]):
    def __init__(self, v: T):
        self.value = v
    def get(self) -> T:
        return self.value
bi = Box[int](1)
gi = bi.get()
bs = Box[str](\"x\")
gs = bs.get()
bare = Box(5)
gd = bare.get()
";

#[test]
fn generic_method_substitutes_type_arg() {
    // `Box[int].get()` substitutes T↦int; `Box[str].get()` → str.
    let (m, i) = typed(GENERIC_SRC);
    let f = main_fn(&m);
    assert_eq!(local_ty(f, &i, "gi"), SemTy::Int);
    assert_eq!(local_ty(f, &i, "gs"), SemTy::Str);
}

#[test]
fn bare_generic_erases_type_var_to_dyn() {
    // A bare `Box(5)` (no type args) leaves no residual `Var`: `get()` erases to
    // `Dyn` (→ Tagged), never a representation-less type variable.
    let (m, i) = typed(GENERIC_SRC);
    let gd = local_ty(main_fn(&m), &i, "gd");
    assert!(
        !gd.contains_var(),
        "no residual type variable may survive materialize"
    );
    assert_eq!(gd, SemTy::Dyn);
}

#[test]
fn generic_instance_type_is_generic() {
    let (m, i) = typed(GENERIC_SRC);
    match local_ty(main_fn(&m), &i, "bi") {
        SemTy::Generic { args, .. } => assert_eq!(args, vec![SemTy::Int]),
        other => panic!("expected `bi: Generic`, got {other:?}"),
    }
}

// ── reinterpret-boundary on call forms (Phase 5 review fix #1) ──

#[test]
fn accepts_int_into_float_method_arg() {
    // §8 numeric tower (method-param seam closed): `C().scaled(3)` is a checked
    // int→float coercion at the method param (lowering's `build_call_operands`
    // routes the `Pos`/`Kw` slot through `coerce_value` → `rt_unbox_float`).
    let src = "\
class C:
    def scaled(self, a: float) -> float:
        return a * 2.0
print(C().scaled(3))
";
    assert!(try_infer(src).is_ok());
    // A keyword arg into the same `float` param coerces identically.
    let kw = "\
class C:
    def scaled(self, a: float) -> float:
        return a * 2.0
print(C().scaled(a=3))
";
    assert!(try_infer(kw).is_ok());
    // The matching-type call still type-checks.
    let ok = "\
class C:
    def scaled(self, a: float) -> float:
        return a * 2.0
print(C().scaled(3.0))
";
    assert!(try_infer(ok).is_ok());
}

#[test]
fn accepts_int_into_float_super_and_static_args() {
    // §8 numeric tower (every direct-call seam closed): int→float coercion is now
    // a checked unbox at the param boundary, uniform across super() / static /
    // generic construction (all funnel through `build_call_operands` /
    // `lower_construct`).
    // super() arg into a float param.
    let sup = "\
class A:
    def __init__(self, a: float):
        self.a = a
class B(A):
    def __init__(self, a: float):
        super().__init__(3)
print(B(1.0).a)
";
    assert!(try_infer(sup).is_ok());
    // @staticmethod arg into a float param.
    let st = "\
class C:
    @staticmethod
    def f(a: float) -> float:
        return a
print(C.f(3))
";
    assert!(try_infer(st).is_ok());
    // generic-construction arg into a float param.
    let gen = "\
from typing import TypeVar, Generic
T = TypeVar(\"T\")
class P(Generic[T]):
    def __init__(self, v: T, scale: float):
        self.v = v
        self.scale = scale
p = P[int](1, 2)
print(p.scale)
";
    assert!(try_infer(gen).is_ok());
}

// ── closures / callables (Phase 6) ──

#[test]
fn make_closure_infers_callable() {
    // A returned nested function value is a `Callable` (drives `Repr::Closure`).
    let src = "\
from typing import Callable
def make() -> Callable[[int], int]:
    def add(x: int) -> int:
        return x + 1
    return add
f = make()
print(f(1))
";
    // The `f` local holds the closure value → Callable.
    let (m, i) = typed(src);
    assert!(matches!(local_ty(main_fn(&m), &i, "f"), SemTy::Callable(_)));
}

#[test]
fn accepts_calling_dyn_value() {
    // Uniform value-call convention: calling a genuinely-`Dyn` value is ADMITTED
    // (Principle 2 — inference is no longer required for correctness). The
    // closure's slot 0 is the arity-generic uniform thunk; a non-callable `Dyn`
    // raises `TypeError` at run time (the runtime callable guard), not at compile
    // time.
    let src = "\
def f(g):
    return g()
print(f(0))
";
    assert!(try_infer(src).is_ok());
}

#[test]
fn accepts_callable_sig_mismatch_uniform() {
    // Every closure shares the ONE uniform repr, so a closure of ANY signature
    // fits a `Callable[...]` slot — the precise signature is a devirtualization
    // hint, not an ABI contract. Arity is bound/checked at run time by the thunk,
    // never statically.
    let src = "\
from typing import Callable
def takes(f: Callable[[int], int]) -> int:
    return f(1)
def two(a: int, b: int) -> int:
    return a + b
print(takes(two))
";
    assert!(try_infer(src).is_ok());
}

#[test]
fn indirect_call_has_no_static_arg_guard() {
    // The uniform value-call path packs args into a tuple and binds them in the
    // thunk at run time (with the Phase-1 checked unbox), so there is no static
    // per-argument reinterpret boundary at an indirect call site — this type-checks
    // (the `int` 3 is checked-unboxed into the `float` param at run time).
    let src = "\
from typing import Callable
def apply(f: Callable[[float], float]) -> float:
    return f(3)
print(apply(lambda x: x))
";
    assert!(try_infer(src).is_ok());
}

#[test]
fn stdlib_raw_param_admits_gradual_arg_via_checked_unbox() {
    // Phase 8H, D3: a gradual argument at a raw-ABI Float param is ADMITTED —
    // lowering emits the CHECKED `rt_unbox_float` (TypeError on a bad tag)
    // instead of a blind reinterpret. A statically NON-numeric argument is
    // still a loud compile error.
    let ok = "\
import math
def f(x):
    return x
print(math.sqrt(f(2)))
";
    assert!(try_infer(ok).is_ok());
    let bad = "\
import math
print(math.sqrt(\"nope\"))
";
    assert!(try_infer(bad).is_err());
}

// ── cross-function return / global variables of the constraint system ──

#[test]
fn call_result_takes_inferred_callee_return() {
    // An UNANNOTATED callee's return type is a variable of the system: the
    // caller's `x = a()` must converge to the callee's inferred `Int`.
    let (m, i) = typed("def a():\n    return 1\n\nx = a()\nprint(x)\n");
    assert_eq!(local_ty(main_fn(&m), &i, "x"), SemTy::Int);
}

#[test]
fn mixed_numeric_return_demotes_to_dyn() {
    // `return 2.25` / `return 16`: the numeric tower joins to Float, but a
    // `Raw(F64)` return ABI would blindly unbox the tagged int return — the
    // Raw-uniformity guard demotes the inferred return to Dyn (tagged).
    let src = "\
def f(flag: bool):
    if flag:
        return 2.25
    return 16

x = f(False)
print(x)
";
    let (m, i) = typed(src);
    assert_eq!(local_ty(main_fn(&m), &i, "x"), SemTy::Dyn);
}

#[test]
fn mixed_numeric_append_demotes_element_slot() {
    // `xs = [2.25]; xs.append(16)`: the element slot must NOT promote to
    // `float` — the pushed tagged int would be blindly unboxed on read.
    let src = "\
xs = [2.25]
xs.append(16)
print(xs[1])
";
    let (m, i) = typed(src);
    assert_eq!(local_ty(main_fn(&m), &i, "xs"), SemTy::list_of(SemTy::Dyn));
}

#[test]
fn mixed_numeric_dict_value_demotes_slot() {
    // `{1: 2.25}` then `d[2] = 7`: the value slot demotes to Dyn, the int
    // key slot stays uniform Int.
    let src = "\
d = {1: 2.25}
d[2] = 7
print(d[2])
";
    let (m, i) = typed(src);
    assert_eq!(
        local_ty(main_fn(&m), &i, "d"),
        SemTy::dict_of(SemTy::Int, SemTy::Dyn)
    );
}

#[test]
fn recursive_inferred_return_converges() {
    // Self-recursion: `fact`'s return feeds its own body through `ret_ty`;
    // the rounds climb Never → Int and settle.
    let src = "\
def fact(n: int):
    if n <= 1:
        return 1
    return n * fact(n - 1)

x = fact(5)
print(x)
";
    let (m, i) = typed(src);
    assert_eq!(local_ty(main_fn(&m), &i, "x"), SemTy::Int);
}

#[test]
fn mutually_recursive_inferred_returns_converge() {
    // Two unannotated functions whose returns depend on each other must still
    // converge — they are two variables of ONE system, not nested fixpoints.
    let src = "\
def is_even(n: int):
    if n == 0:
        return True
    return is_odd(n - 1)

def is_odd(n: int):
    if n == 0:
        return False
    return is_even(n - 1)

x = is_even(4)
print(x)
";
    let (m, i) = typed(src);
    assert_eq!(local_ty(main_fn(&m), &i, "x"), SemTy::Bool);
}

#[test]
fn unbounded_recursive_container_return_widens_to_dyn() {
    // `ret = list[ret]` climbs the lattice's infinite container spine; there is
    // no iteration cap anymore, so termination relies on the WIDEN_LIMIT
    // widening cutting the variable to `Dyn`. `infer` must terminate and the
    // caller's result must be the (correct, tagged) `Dyn`.
    let src = "\
def f():
    return [f()]

x = f()
print(len(x))
";
    let (m, i) = typed(src);
    assert_eq!(local_ty(main_fn(&m), &i, "x"), SemTy::Dyn);
}

#[test]
fn self_referential_container_local_terminates() {
    // `x = [x]` is the self-recursive constraint `x ⊒ list[x]`: without the
    // per-expr widening the inner sweep would climb list[list[…]] forever.
    // Infer must terminate, with the slot cut to the (correct, tagged) `Dyn`.
    let (m, i) = typed("x = []\nx = [x]\nprint(x)\n");
    assert_eq!(local_ty(main_fn(&m), &i, "x"), SemTy::Dyn);
}

#[test]
fn self_referential_cell_terminates() {
    // The same unbounded spine through a closure cell: the cell content is
    // re-joined from `cell_writes` every sweep iteration.
    let src = "\
def outer():
    x = []
    x = [x]
    def inner():
        return x
    print(inner())

outer()
";
    assert!(try_infer(src).is_ok());
}

#[test]
fn self_referential_global_terminates() {
    // The same unbounded spine through a promoted global slot (`__main__`'s
    // own reads join `global_writes` live inside its sweep).
    let src = "\
def reader():
    return x

x = []
x = [x]
print(reader())
";
    assert!(try_infer(src).is_ok());
}

#[test]
fn ret_chain_propagates_through_dirty_marking() {
    // a → b → c: each round moves one more return variable, so dirty-marking
    // must transitively re-solve exactly the readers — b after c moves, a
    // after b, finally `__main__` — and still converge `x` to Int.
    let src = "\
def c():
    return 1

def b():
    return c()

def a():
    return b()

x = a()
print(x)
";
    let (m, i) = typed(src);
    assert_eq!(local_ty(main_fn(&m), &i, "x"), SemTy::Int);
}

#[test]
fn rising_global_does_not_poison_float_seam() {
    // A global fed by an unannotated callee whose return climbs across rounds:
    // while `k`'s return is still unknown, `g`'s slot must ride bottom
    // (`Never`), never a transient `Dyn` that sticks and makes
    // `check_reinterpret` reject the `float`-parameter seam. The from-bottom
    // re-solve guarantees the converged slot is `Float` and this compiles.
    let src = "\
def k():
    return 2.5

def f(a: float):
    print(a)

def caller():
    f(g)

g = k()
caller()
";
    let (m, i) = typed(src);
    // The promoted slot converged to Float: the GlobalGet feeding the float
    // parameter is typed Float (not Dyn), in every reader.
    let caller = m
        .functions
        .iter()
        .find(|f| i.resolve(f.name) == "caller")
        .expect("caller function");
    let gget = caller
        .exprs
        .iter()
        .find(|(_, e)| matches!(e.kind, HirExprKind::GlobalGet { .. }))
        .expect("GlobalGet in caller");
    assert_eq!(gget.1.ty, SemTy::Float);
}

#[test]
fn stdlib_call_types_from_descriptor() {
    // `math.sqrt` types as Float, `math.ceil` as Int — straight from the
    // descriptors' return TypeSpecs, so annotated slots accept them.
    let src = "\
import math
a: float = math.sqrt(4.0)
b: int = math.ceil(3.2)
print(a, b)
";
    assert!(try_infer(src).is_ok());
}

#[test]
fn container_element_types_infer_from_pushes() {
    // Phase 8H, D1: `acc = []` + `acc.append(<float>)` solves the local to
    // list[float] — pushes constrain the element type.
    let src = "\
acc = []
for i in range(3):
    acc.append(i * 0.5)
print(acc[1])
";
    let mut interner = StringInterner::new();
    let mut module = pyaot_frontend_python::parse(src, &mut interner).expect("parse");
    let ns = pyaot_hir::NamespaceTable::single(module.functions.len());
    let resolve = pyaot_semantics::resolve(&mut module, &ns, &interner).expect("resolve");
    let mut classes =
        pyaot_semantics::collect_classes(&module, &ns, &interner).expect("collect_classes");
    infer(&mut module, &resolve, &mut classes, &interner).expect("infer");
    let main = main_fn(&module);
    assert!(
        main.locals
            .iter()
            .any(|l| l.ty == SemTy::list_of(SemTy::Float)),
        "append-built list solves to list[float], got {:?}",
        main.locals.iter().map(|l| l.ty.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn sum_types_from_numeric_promotion() {
    // Phase 8H, D2: sum over floats solves Float; over ints solves Int — so
    // annotated consumers accept the results without casts.
    let src = "\
a: float = sum([0.5, 1.5])
b: int = sum([1, 2, 3])
print(a, b)
";
    assert!(try_infer(src).is_ok());
}

#[test]
fn comp_element_type_infers_from_pushes() {
    // Phase 8H, D1: a list comprehension's element type comes from the
    // desugared pushes, so the result feeds an annotated list[float] slot.
    let src = "\
xs: list[float] = [i * 0.5 for i in range(4)]
print(xs[2])
";
    assert!(try_infer(src).is_ok());
}

// ── B10: field-type inference as solver variables ──

#[test]
fn field_climbs_to_float_through_self_referential_write() {
    // `grad` is written `0.0` in __init__ and `o.grad = o.grad + 1.5` through
    // a NON-self receiver — the variable must climb Never → Float across
    // rounds (the read feeds the write's own contribution).
    let src = "\
class V:
    def __init__(self):
        self.grad = 0.0
        self.other = self

    def step(self):
        o = self.other
        o.grad = o.grad + 1.5


x = V()
x.step()
g = x.grad
print(g)
";
    let (m, i) = typed(src);
    assert_eq!(local_ty(main_fn(&m), &i, "g"), SemTy::Float);
}

#[test]
fn mixed_numeric_field_writes_demote_to_dyn() {
    // Float and int writes into one field: the Raw-uniformity guard demotes
    // the field to Dyn (tagged) — and the program still infers cleanly
    // (today's best-effort Float field would loudly reject the int write).
    let src = "\
class M:
    def __init__(self, flag: bool):
        if flag:
            self.v = 1.5
        else:
            self.v = 7


m = M(True)
y = m.v
print(y)
";
    let (m, i) = typed(src);
    assert_eq!(local_ty(main_fn(&m), &i, "y"), SemTy::Dyn);
}

#[test]
fn dyn_receiver_write_demotes_field() {
    // A write through a Dyn receiver goes by NAME at runtime (SetFieldNamed
    // can hit any class with that field name) — every same-named field
    // variable demotes to Dyn.
    let src = "\
class A:
    def __init__(self):
        self.w = 1.5


def poke(x):
    x.w = \"s\"


a = A()
poke(a)
z = a.w
print(z)
";
    let (m, i) = typed(src);
    assert_eq!(local_ty(main_fn(&m), &i, "z"), SemTy::Dyn);
}

#[test]
fn annotated_field_stays_authoritative() {
    // A class-level `name: T` annotation is a constant of the system — no
    // solver variable, reads keep the declared type.
    let src = "\
class T:
    lbl: str

    def __init__(self):
        self.lbl = \"x\"


t = T()
s = t.lbl
print(s)
";
    let (m, i) = typed(src);
    assert_eq!(local_ty(main_fn(&m), &i, "s"), SemTy::Str);
}

#[test]
fn subclass_write_feeds_base_class_variable() {
    // `self.t = ...` in a subclass method writes the field DEFINED by the
    // base — the contribution lands in the base's variable, and a read
    // through the subclass resolves to it.
    let src = "\
class B:
    def __init__(self):
        self.t = 0.0


class D(B):
    def bump(self):
        self.t = self.t + 1.5


d = D()
d.bump()
u = d.t
print(u)
";
    let (m, i) = typed(src);
    assert_eq!(local_ty(main_fn(&m), &i, "u"), SemTy::Float);
}

#[test]
fn generic_class_fields_keep_static_path() {
    // Fields defined by a generic class get NO solver variable (their types
    // mention type params; apply_subst keeps the static path) — inference
    // must still complete cleanly.
    let src = "\
class Box[T]:
    def __init__(self, item: T):
        self.item = item

    def get(self) -> T:
        return self.item


b = Box(5)
print(b.get())
";
    assert!(try_infer(src).is_ok());
}
