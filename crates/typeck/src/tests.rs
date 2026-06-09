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
    let resolve = pyaot_semantics::resolve(&mut module, &interner).expect("resolve");
    infer(&mut module, &resolve).expect("infer");
    (module, interner)
}

/// Parse + resolve, then return the `infer` result (for testing rejections).
fn try_infer(src: &str) -> pyaot_diagnostics::Result<()> {
    let mut interner = StringInterner::new();
    let mut module = pyaot_frontend_python::parse(src, &mut interner).expect("parse");
    let resolve = pyaot_semantics::resolve(&mut module, &interner).expect("resolve");
    infer(&mut module, &resolve)
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
        matches!(e.kind, HirExprKind::BinOp { op: pyaot_hir::BinOp::Div, .. })
            && e.ty == SemTy::Float
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
        .find(|(_, e)| matches!(e.kind, HirExprKind::BinOp { op: pyaot_hir::BinOp::Add, .. }))
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
        .find(|(_, e)| matches!(e.kind, HirExprKind::BinOp { op: pyaot_hir::BinOp::BitAnd, .. }))
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
fn rejects_int_into_float_parameter() {
    // `poly(3)` for `def poly(a: float)` must be a loud type error, NOT an
    // accept-then-SIGSEGV (an annotated float param is unboxed to Raw(F64)).
    let src = "def poly(a: float) -> float:\n    return a + 1.0\n\n\nprint(poly(3))\n";
    assert!(try_infer(src).is_err(), "int into a float parameter must be rejected");
}

#[test]
fn accepts_float_into_float_parameter() {
    let src = "def poly(a: float) -> float:\n    return a + 1.0\n\n\nprint(poly(3.0))\n";
    assert!(try_infer(src).is_ok(), "float into a float parameter must compile");
}

#[test]
fn rejects_int_into_float_local() {
    assert!(try_infer("x: float = 5\nprint(x)\n").is_err());
}

#[test]
fn rejects_int_returned_as_float() {
    assert!(try_infer("def f() -> float:\n    return 5\n\n\nprint(f())\n").is_err());
}

#[test]
fn accepts_tagged_float_value_into_float_local() {
    // A float-*typed* value that lowers to a tagged boxed float (true division,
    // a `float()` call) is a legitimate UnboxFloat — it must NOT be rejected.
    assert!(try_infer("x: float = 7.0 / 2.0\nprint(x)\n").is_ok());
    assert!(try_infer("y: float = float(3)\nprint(y)\n").is_ok());
}
