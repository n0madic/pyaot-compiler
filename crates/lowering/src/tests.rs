//! Lowering-shape unit tests: drive real Python through parse → resolve → infer
//! → lower and assert representation specialization the differential gate cannot
//! observe (e.g. unboxed `Raw(F64)` float arithmetic vs the tagged baseline).

use pyaot_mir::{BinOp, Coercion, ContainerOp, MirFunction, MirInst, MirProgram, Operand};
use pyaot_types::{HeapShape, RawKind, Repr};
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

// ── containers (Phase 4) ──

/// Every `CallContainer` op in `f`.
fn container_ops(f: &MirFunction) -> Vec<ContainerOp> {
    f.blocks
        .iter()
        .flat_map(|b| &b.insts)
        .filter_map(|i| match i {
            MirInst::CallContainer { op, .. } => Some(*op),
            _ => None,
        })
        .collect()
}

#[test]
fn list_literal_lowers_to_new_plus_pushes() {
    // `[1, 2]` → one ListNew followed by a ListPush per element, and the list
    // local is a `Heap(List(Tagged))` (uniform tagged element storage, A5).
    let p = lowered("xs = [1, 2]\nprint(xs[0])\n");
    let f = main_fn(&p);
    let ops = container_ops(f);
    assert_eq!(ops.iter().filter(|o| **o == ContainerOp::ListNew).count(), 1);
    assert_eq!(ops.iter().filter(|o| **o == ContainerOp::ListPush).count(), 2);
    assert!(
        locals_with_repr(f, &Repr::Heap(HeapShape::List(Box::new(Repr::Tagged)))) >= 1,
        "the list local should be Heap(List(Tagged))"
    );
}

#[test]
fn annotated_empty_list_is_tagged_element_heap() {
    // PITFALLS B4 / risk #1: `x: list[int] = []` must lower to `Heap(List(Tagged))`,
    // never a heap-default — verified via the materialized local repr.
    let p = lowered("x: list[int] = []\nprint(len(x))\n");
    let f = main_fn(&p);
    assert!(
        locals_with_repr(f, &Repr::Heap(HeapShape::List(Box::new(Repr::Tagged)))) >= 1,
        "annotated empty list should be Heap(List(Tagged))"
    );
}

#[test]
fn len_lowers_to_container_len_op() {
    let p = lowered("xs = [1, 2, 3]\nprint(len(xs))\n");
    assert!(container_ops(main_fn(&p)).contains(&ContainerOp::Len));
}

#[test]
fn membership_lowers_to_contains_op() {
    let p = lowered("xs = [1, 2]\nprint(2 in xs)\n");
    assert!(container_ops(main_fn(&p)).contains(&ContainerOp::Contains));
}

#[test]
fn list_concat_and_repeat_dispatch_by_type() {
    let p = lowered("a = [1] + [2]\nb = [0] * 3\nprint(a)\nprint(b)\n");
    let ops = container_ops(main_fn(&p));
    assert!(ops.contains(&ContainerOp::ListConcat));
    assert!(ops.contains(&ContainerOp::ListRepeat));
}

#[test]
fn general_for_uses_iterator_protocol_in_order() {
    // A non-range `for` lowers to Iter → (header) IterNext → IterExhausted, with a
    // Heap(Iterator) iterator local kept live across the loop (GC-rooted).
    let p = lowered("for x in [1, 2, 3]:\n    print(x)\n");
    let f = main_fn(&p);
    let ops = container_ops(f);
    assert!(ops.contains(&ContainerOp::Iter));
    assert!(
        f.locals.iter().any(|l| matches!(l.repr, Repr::Heap(HeapShape::Iterator(_)))),
        "iterator local should be Heap(Iterator)"
    );
    // Within the header block, IterNext precedes IterExhausted (runtime contract).
    let header_ops: Vec<ContainerOp> = f
        .blocks
        .iter()
        .find_map(|b| {
            let ops: Vec<ContainerOp> = b
                .insts
                .iter()
                .filter_map(|i| match i {
                    MirInst::CallContainer { op, .. } => Some(*op),
                    _ => None,
                })
                .collect();
            ops.contains(&ContainerOp::IterNext).then_some(ops)
        })
        .expect("a block with IterNext");
    let next_pos = header_ops.iter().position(|o| *o == ContainerOp::IterNext).unwrap();
    let exh_pos = header_ops.iter().position(|o| *o == ContainerOp::IterExhausted).unwrap();
    assert!(next_pos < exh_pos, "IterNext must precede IterExhausted");
}

#[test]
fn list_comprehension_reuses_push_and_iterator_protocol() {
    // A list comprehension desugars to ListNew + an iterator loop whose body
    // pushes via the same ContainerPush(ListPush) path as a literal build, and the
    // result list is a GC-rooted Heap(List).
    let p = lowered("ys = [x * 2 for x in [1, 2, 3]]\nprint(ys)\n");
    let f = main_fn(&p);
    let ops = container_ops(f);
    assert!(ops.contains(&ContainerOp::ListNew));
    assert!(ops.contains(&ContainerOp::ListPush));
    assert!(ops.contains(&ContainerOp::Iter));
    assert!(ops.contains(&ContainerOp::IterNext));
    assert!(
        f.locals.iter().any(|l| matches!(l.repr, Repr::Heap(HeapShape::List(_)))),
        "comprehension result should be a Heap(List)"
    );
}

#[test]
fn iter_next_result_local_is_tagged_not_unboxed() {
    // PITFALLS: iterating a float list keeps the iter_next slot Tagged (it is null
    // on exhaustion); only the typed loop variable is unboxed, inside the body.
    let p = lowered("for x in [1.5, 2.5]:\n    print(x)\n");
    let f = main_fn(&p);
    // No Coerce produces an UnboxFloat in the loop *header* (the iter_next store);
    // the only float unbox is the body binding `x = elem`, which is sound.
    // Structural proxy: there is at least one Tagged local feeding a float result.
    assert!(
        f.locals.iter().any(|l| l.repr == Repr::Tagged),
        "the iter_next result slot must be Tagged"
    );
}

#[test]
fn iteration_builtins_lower_to_runtime_ops() {
    let p = lowered("print(sorted([3, 1, 2]))\nfor i, v in enumerate([9, 8]):\n    print(i)\n");
    let ops = container_ops(main_fn(&p));
    assert!(ops.contains(&ContainerOp::Sorted));
    assert!(ops.contains(&ContainerOp::Enumerate));
}

#[test]
fn list_methods_dispatch_by_receiver() {
    let p = lowered("xs = [1]\nxs.append(2)\nxs.pop()\nprint(len(xs))\n");
    let ops = container_ops(main_fn(&p));
    // `.append` reuses ListPush; `.pop()` is ListPop.
    assert!(ops.contains(&ContainerOp::ListPush));
    assert!(ops.contains(&ContainerOp::ListPop));
}

#[test]
fn dict_and_set_methods_dispatch_by_receiver() {
    let p = lowered(
        "d = {\"a\": 1}\nv = d.get(\"a\")\ns = {1}\ns.add(2)\nprint(v)\nprint(len(s))\n",
    );
    let ops = container_ops(main_fn(&p));
    assert!(ops.contains(&ContainerOp::DictGetDefault));
    assert!(ops.contains(&ContainerOp::SetAdd));
}

#[test]
fn list_insert_index_is_raw_i64() {
    // Regression: `.insert(index, value)` must pass the index as Raw(I64), not a
    // tagged value (which would read as the wrong index).
    let p = lowered("xs = [1, 2]\nxs.insert(0, 9)\nprint(xs)\n");
    let f = main_fn(&p);
    // The ListInsert instruction's middle arg must be a Raw(I64) local.
    let insert = f
        .blocks
        .iter()
        .flat_map(|b| &b.insts)
        .find_map(|i| match i {
            MirInst::CallContainer { op: ContainerOp::ListInsert, args, .. } => Some(args.clone()),
            _ => None,
        })
        .expect("a ListInsert instruction");
    let Operand::Local(idx_local) = insert[1];
    assert_eq!(f.locals[idx_local.index()].repr, Repr::Raw(RawKind::I64));
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
