//! Lowering-shape unit tests: drive real Python through parse → resolve → infer
//! → lower and assert representation specialization the differential gate cannot
//! observe (e.g. unboxed `Raw(F64)` float arithmetic vs the tagged baseline).

use pyaot_mir::{BinOp, Coercion, MirFunction, MirInst, MirProgram, Operand};
use pyaot_types::{RawKind, Repr};
use pyaot_utils::StringInterner;

/// Parse, resolve, infer, and lower `src`; return the lowered program. Also runs
/// the MIR verifier so the assertions only ever see well-formed MIR.
fn lowered(src: &str) -> MirProgram {
    let mut interner = StringInterner::new();
    let mut module = pyaot_frontend_python::parse(src, &mut interner).expect("parse");
    let resolve = pyaot_semantics::resolve(&mut module, &interner).expect("resolve");
    pyaot_typeck::infer(&mut module, &resolve).expect("infer");
    let program = super::lower(&module, &resolve, &interner).expect("lower");
    for f in &program.funcs {
        pyaot_mir::verify(f, &program.funcs).expect("verify");
    }
    program
}

fn main_fn(p: &MirProgram) -> &MirFunction {
    &p.funcs[p.entry.index()]
}

/// Every `BinOp` in `f` paired with the declared `Repr` of its `dst` local.
fn binops_with_repr(f: &MirFunction) -> Vec<(BinOp, Repr)> {
    f.blocks
        .iter()
        .flat_map(|b| &b.insts)
        .filter_map(|i| match i {
            MirInst::BinOp { dst, op, .. } => Some((*op, f.locals[dst.index()].repr.clone())),
            _ => None,
        })
        .collect()
}

/// Does `f` contain any float-boxing coercion (`Raw(F64)` → `Tagged`)?
fn has_box_float(f: &MirFunction) -> bool {
    f.blocks.iter().flat_map(|b| &b.insts).any(|i| {
        matches!(i, MirInst::Coerce { from, to, .. }
                 if pyaot_mir::classify_coercion(from, to) == Some(Coercion::BoxFloat))
    })
}

#[test]
fn float_add_sub_mul_lower_unboxed() {
    // `+ - *` over floats must lower to Raw(F64) BinOps with NO float boxing.
    let f64 = Repr::Raw(RawKind::F64);
    for src in ["print(3.0 + 1.5)\n", "print(3.0 - 1.5)\n", "print(3.0 * 1.5)\n"] {
        let p = lowered(src);
        let f = main_fn(&p);
        let binops = binops_with_repr(f);
        assert!(
            binops.iter().any(|(_, r)| *r == f64),
            "{src:?} should produce a Raw(F64) BinOp, got {binops:?}"
        );
        assert!(!has_box_float(f), "{src:?} must not box floats");
    }
}

#[test]
fn float_accumulator_loop_is_unboxed() {
    // `acc = acc + 1.5` across a loop: the add is Raw(F64), no boxing in the body.
    let p = lowered("acc = 0.0\nfor i in range(5):\n    acc = acc + 1.5\nprint(acc)\n");
    let f = main_fn(&p);
    assert!(binops_with_repr(f).iter().any(|(op, r)| *op == BinOp::Add
        && *r == Repr::Raw(RawKind::F64)));
    assert!(!has_box_float(f), "float accumulation must not box");
}

#[test]
fn float_division_stays_tagged() {
    // `/` is NOT specialized (CPython `x/0.0` raises; we keep tagged semantics).
    let p = lowered("print(7.0 / 2.0)\n");
    let f = main_fn(&p);
    let divs: Vec<_> = binops_with_repr(f).into_iter().filter(|(op, _)| *op == BinOp::Div).collect();
    assert_eq!(divs.len(), 1, "expected one Div");
    assert_eq!(divs[0].1, Repr::Tagged, "float division must stay tagged");
}

#[test]
fn int_arithmetic_stays_tagged() {
    // Plain int arithmetic remains tagged (bignum-safe) — no Raw specialization
    // in 3b; the proof-gated raw int path is 3c (range cursors only).
    let p = lowered("print(1 + 2 * 3)\n");
    let f = main_fn(&p);
    for (op, r) in binops_with_repr(f) {
        assert_eq!(r, Repr::Tagged, "int BinOp {op:?} should be tagged");
    }
}

#[test]
fn mixed_int_float_add_stays_tagged() {
    // Only when BOTH operands are statically float does the raw path fire; mixed
    // int/float falls back to the tagged baseline (the runtime promotes).
    let p = lowered("print(3 + 1.5)\n");
    let f = main_fn(&p);
    let adds: Vec<_> = binops_with_repr(f).into_iter().filter(|(op, _)| *op == BinOp::Add).collect();
    assert!(adds.iter().any(|(_, r)| *r == Repr::Tagged), "mixed add must be tagged");
}

/// Count locals declared with `repr` in `f`.
fn locals_with_repr(f: &MirFunction, repr: &Repr) -> usize {
    f.locals.iter().filter(|l| l.repr == *repr).count()
}

/// `Repr` of a `Compare` instruction's left operand (its operand representation).
fn compare_operand_reprs(f: &MirFunction) -> Vec<Repr> {
    f.blocks
        .iter()
        .flat_map(|b| &b.insts)
        .filter_map(|i| match i {
            MirInst::Compare { l: Operand::Local(id), .. } => Some(f.locals[id.index()].repr.clone()),
        _ => None,
        })
        .collect()
}

#[test]
fn literal_bounded_range_cursor_is_raw_i64() {
    // The plan's milestone-3c marker: a `range(lo, hi, step)` loop with integer
    // literal bounds narrows its cursor + stop slot to Raw(I64), runs the guard
    // as a raw `Compare`, and increments with a raw `Add`.
    let i64r = Repr::Raw(RawKind::I64);
    let p = lowered("total = 0\nfor i in range(1, 6):\n    total = total + i\nprint(total)\n");
    let f = main_fn(&p);
    // The cursor + stop slots (and the raw arithmetic temporaries they spawn).
    assert!(locals_with_repr(f, &i64r) >= 2, "cursor and stop should be Raw(I64)");
    // The loop guard compares raw i64 operands.
    assert!(
        compare_operand_reprs(f).contains(&i64r),
        "loop guard should compare Raw(I64) cursors"
    );
    // The cursor increment is a raw i64 Add.
    assert!(
        binops_with_repr(f).iter().any(|(op, r)| *op == BinOp::Add && *r == i64r),
        "cursor increment should be a Raw(I64) Add"
    );
    // The accumulator stays tagged (bignum-safe).
    assert!(
        binops_with_repr(f).iter().any(|(op, r)| *op == BinOp::Add && *r == Repr::Tagged),
        "the accumulator add must stay tagged"
    );
}

#[test]
fn range_with_negative_literal_step_is_raw_i64() {
    let i64r = Repr::Raw(RawKind::I64);
    let p = lowered("for d in range(10, 0, -2):\n    print(d)\n");
    let f = main_fn(&p);
    assert!(locals_with_repr(f, &i64r) >= 2);
    assert!(compare_operand_reprs(f).contains(&i64r));
}

#[test]
fn non_literal_range_stays_tagged() {
    // A non-literal bound (`range(1, n)` with `n` a variable) is not range-proven,
    // so the cursor must stay tagged — soundness over completeness (PITFALLS A6).
    let i64r = Repr::Raw(RawKind::I64);
    let p = lowered("n = 6\nfor i in range(1, n):\n    print(i)\n");
    let f = main_fn(&p);
    assert_eq!(locals_with_repr(f, &i64r), 0, "non-literal range cursor must stay tagged");
    // And the guard compares tagged operands.
    assert!(compare_operand_reprs(f).iter().all(|r| *r == Repr::Tagged));
}
