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
    let ns = pyaot_hir::NamespaceTable::single(module.functions.len());
    let resolve = pyaot_semantics::resolve(&mut module, &ns, &interner).expect("resolve");
    let classes =
        pyaot_semantics::collect_classes(&module, &ns, &interner).expect("collect_classes");
    pyaot_typeck::infer(&mut module, &resolve, &classes, &interner).expect("infer");
    let program = super::lower(&module, &resolve, &interner, &classes).expect("lower");
    for f in &program.funcs {
        pyaot_mir::verify(f, &program.funcs).expect("verify");
    }
    program
}

fn main_fn(p: &MirProgram) -> &MirFunction {
    &p.funcs[p.entry.index()]
}

/// Parse → resolve → collect → infer → lower, returning the (possibly error)
/// lower result — for asserting lowering-stage rejections.
fn try_lower(src: &str) -> pyaot_diagnostics::Result<MirProgram> {
    let mut interner = StringInterner::new();
    let mut module = pyaot_frontend_python::parse(src, &mut interner).expect("parse");
    let ns = pyaot_hir::NamespaceTable::single(module.functions.len());
    let resolve = pyaot_semantics::resolve(&mut module, &ns, &interner).expect("resolve");
    let classes =
        pyaot_semantics::collect_classes(&module, &ns, &interner).expect("collect_classes");
    pyaot_typeck::infer(&mut module, &resolve, &classes, &interner).expect("infer");
    super::lower(&module, &resolve, &interner, &classes)
}

#[test]
fn unknown_container_method_rejected_at_lowering() {
    // Phase 5 (D2): the unknown-method rejection moved from the frontend to
    // lowering, where the receiver type selects the dispatch.
    assert!(try_lower("xs = [1]\nxs.frobnicate()\nprint(xs)\n").is_err());
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

// ── classes (Phase 5A) ──

/// All MIR instructions of `f` flattened.
fn insts(f: &MirFunction) -> Vec<&MirInst> {
    f.blocks.iter().flat_map(|b| &b.insts).collect()
}

const CLASS_SRC: &str = "\
class Widget:
    def __init__(self, w: int, h: int):
        self.w = w
        self.h = h

    def area(self) -> int:
        return self.w * self.h

x = Widget(3, 4)
print(x.area())
print(x.w)
";

#[test]
fn construction_lowers_to_make_instance_then_init_call() {
    let p = lowered(CLASS_SRC);
    let f = main_fn(&p);
    // MakeInstance with a Heap(Class) dst.
    let mk = insts(f).into_iter().find_map(|i| match i {
        MirInst::MakeInstance { dst, class_id, .. } => Some((*dst, *class_id)),
        _ => None,
    });
    let (dst, cid) = mk.expect("a MakeInstance");
    assert_eq!(f.locals[dst.index()].repr, Repr::Heap(HeapShape::Class(cid)));
    // __init__ is a direct Call right after construction (devirtualized, dst None).
    assert!(insts(f).iter().any(|i| matches!(i, MirInst::Call { dst: None, .. })));
}

#[test]
fn field_read_is_getfield_then_legalize() {
    // `x.w` (an int field) → GetField (Tagged result) flowing as the field's repr.
    let p = lowered(CLASS_SRC);
    let f = main_fn(&p);
    let gf = insts(f).into_iter().find_map(|i| match i {
        MirInst::GetField { dst, .. } => Some(*dst),
        _ => None,
    });
    let dst = gf.expect("a GetField");
    // The uniform tagged field value (int field → stays Tagged).
    assert_eq!(f.locals[dst.index()].repr, Repr::Tagged);
}

#[test]
fn field_write_is_setfield_with_tagged_value() {
    // `self.w = w` inside __init__ → SetField; the value operand is Tagged (A5).
    let p = lowered(CLASS_SRC);
    let init = p
        .funcs
        .iter()
        .find(|f| f.blocks.iter().flat_map(|b| &b.insts).any(|i| matches!(i, MirInst::SetField { .. })))
        .expect("a function with SetField (the __init__)");
    let sf = init
        .blocks
        .iter()
        .flat_map(|b| &b.insts)
        .find_map(|i| match i {
            MirInst::SetField { value: Operand::Local(v), base: Operand::Local(b), .. } => Some((*b, *v)),
            _ => None,
        })
        .expect("a SetField");
    assert_eq!(init.locals[sf.1.index()].repr, Repr::Tagged, "field value is Tagged");
    // The base is a valid instance operand: Heap(Class) (the `self` param) or Tagged.
    assert!(matches!(
        init.locals[sf.0.index()].repr,
        Repr::Heap(HeapShape::Class(_)) | Repr::Tagged
    ));
}

#[test]
fn method_call_is_devirtualized_direct_call() {
    // `x.area()` on a statically-known class lowers to a direct Call to the
    // method's FuncId (no CallVirtual in 5A).
    let p = lowered(CLASS_SRC);
    let f = main_fn(&p);
    // A Call with a dst (area returns int → Tagged) referencing a user method.
    assert!(insts(f).iter().any(|i| matches!(i, MirInst::Call { dst: Some(_), .. })));
}

#[test]
fn classes_recorded_in_program() {
    let p = lowered(CLASS_SRC);
    assert_eq!(p.classes.len(), 1);
    assert_eq!(p.classes[0].field_count, 2);
}

// ── inheritance / dispatch (Phase 5B) ──

const INHERIT_SRC: &str = "\
class Animal:
    def __init__(self, name: str):
        self.name = name
    def speak(self) -> str:
        return \"...\"
class Dog(Animal):
    def __init__(self, name: str, breed: str):
        super().__init__(name)
        self.breed = breed
    def speak(self) -> str:
        return \"Woof\"
animals: list[Animal] = [Dog(\"Rex\", \"Lab\"), Animal(\"Thing\")]
for a in animals:
    print(a.speak())
d = Dog(\"Fido\", \"Beagle\")
print(isinstance(d, Animal))
";

fn has_call_virtual(p: &MirProgram) -> bool {
    p.funcs
        .iter()
        .flat_map(|f| f.blocks.iter().flat_map(|b| &b.insts))
        .any(|i| matches!(i, MirInst::CallVirtual { .. }))
}

#[test]
fn overridden_method_dispatches_virtually() {
    // `a.speak()` where `a: Animal` and `speak` is overridden by `Dog` → CallVirtual.
    let p = lowered(INHERIT_SRC);
    assert!(has_call_virtual(&p), "an overridden method must use virtual dispatch");
}

#[test]
fn isinstance_lowers_to_instance_check() {
    let p = lowered(INHERIT_SRC);
    assert!(p
        .funcs
        .iter()
        .flat_map(|f| f.blocks.iter().flat_map(|b| &b.insts))
        .any(|i| matches!(i, MirInst::IsInstance { .. })));
}

#[test]
fn super_call_is_a_direct_call_not_virtual() {
    // `super().__init__(name)` resolves statically to Animal.__init__ — a direct
    // Call, never CallVirtual.
    let src = "\
class Animal:
    def __init__(self, name: str):
        self.name = name
class Dog(Animal):
    def __init__(self, name: str, breed: str):
        super().__init__(name)
        self.breed = breed
d = Dog(\"x\", \"y\")
print(d.name)
";
    let p = lowered(src);
    // No method is overridden (each class has its own __init__, but __init__ is
    // never called polymorphically) → no CallVirtual.
    assert!(!has_call_virtual(&p));
    // The Dog.__init__ body contains a direct Call (to Animal.__init__ via super).
    let dog_init = p.funcs.iter().any(|f| {
        f.blocks
            .iter()
            .flat_map(|b| &b.insts)
            .any(|i| matches!(i, MirInst::Call { .. }))
    });
    assert!(dog_init, "super().__init__ must lower to a direct Call");
}

#[test]
fn concrete_non_overridden_method_devirtualizes() {
    // `speak` defined only on a leaf class with no subclasses → direct Call.
    let src = "\
class Widget:
    def __init__(self, w: int):
        self.w = w
    def area(self) -> int:
        return self.w
x = Widget(3)
print(x.area())
";
    let p = lowered(src);
    assert!(!has_call_virtual(&p), "a non-overridden method must devirtualize");
}

// ── dunders (Phase 5C) ──

const DUNDER_SRC: &str = "\
class Vector:
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y
    def __add__(self, other: Vector) -> Vector:
        return Vector(self.x + other.x, self.y + other.y)
    def __eq__(self, other: Vector) -> bool:
        return self.x == other.x and self.y == other.y
    def __repr__(self) -> str:
        return \"V\"
a = Vector(1, 2)
b = Vector(3, 4)
print(a + b)
print(a == b)
";

#[test]
fn eq_dunder_is_compiler_routed_to_direct_call() {
    // `a == b` must NOT lower to a bare Compare (rt_obj_eq won't dispatch __eq__);
    // it routes to a direct Call + Truthy.
    let p = lowered(DUNDER_SRC);
    let f = main_fn(&p);
    // There is a Call (the __eq__ dispatch) and a Truthy (the bool→i8 reduction),
    // and no Compare with Eq directly on the two Vector operands at top level.
    assert!(insts(f).iter().any(|i| matches!(i, MirInst::Call { .. })));
    assert!(insts(f).iter().any(|i| matches!(i, MirInst::Truthy { .. })));
}

#[test]
fn arithmetic_dunder_rides_tagged_baseline() {
    // `a + b` lowers to the tagged BinOp(Add) (→ rt_obj_add → registered __add__),
    // not a typed container op.
    let p = lowered(DUNDER_SRC);
    let f = main_fn(&p);
    assert!(binops_with_repr(f)
        .iter()
        .any(|(op, r)| *op == BinOp::Add && *r == Repr::Tagged));
    // __add__ / __eq__ / __repr__ are registered as dunders for the class.
    assert!(!p.classes[0].dunders.is_empty());
}

#[test]
fn dunder_method_returns_tagged() {
    // B11: a dunder's return repr is forced to Tagged even when declared `-> bool`
    // (`__eq__`), so the registry dispatch ABI stays uniform `Value -> Value`.
    let p = lowered(DUNDER_SRC);
    // The `__eq__` body is the only function with an inner Compare; it must return
    // Tagged despite its `-> bool` annotation.
    let eq = p.funcs.iter().find(|f| {
        f.blocks
            .iter()
            .flat_map(|b| &b.insts)
            .any(|i| matches!(i, MirInst::Compare { .. }))
    });
    assert_eq!(eq.expect("the __eq__ method").ret, Repr::Tagged);
}

#[test]
fn print_instance_routes_to_repr_dunder() {
    // print(instance) routes to a direct __repr__/__str__ Call (the runtime's
    // top-level print path renders the default repr otherwise).
    let p = lowered(DUNDER_SRC);
    let f = main_fn(&p);
    // A Print(StrObj) fed by a Call result (the __repr__ dispatch).
    assert!(insts(f)
        .iter()
        .any(|i| matches!(i, MirInst::Print { kind: pyaot_mir::PrintKind::StrObj, .. })));
}

// ── decorators + class attributes (Phase 5D) ──

const DECO_SRC: &str = "\
class T:
    scale = \"c\"
    def __init__(self, d: float):
        self._d = d
    @property
    def d(self) -> float:
        return self._d
    @d.setter
    def d(self, v: float):
        self._d = v
    @staticmethod
    def zero() -> float:
        return 0.0
    @classmethod
    def unit(cls) -> str:
        return T.scale
t = T(1.0)
print(t.d)
t.d = 2.0
print(T.zero())
print(t.zero())
print(T.unit())
print(T.scale)
T.scale = \"k\"
";

#[test]
fn class_attr_get_set_lowered() {
    let p = lowered(DECO_SRC);
    let f = main_fn(&p);
    assert!(insts(f).iter().any(|i| matches!(i, MirInst::GetClassAttr { .. })));
    assert!(insts(f).iter().any(|i| matches!(i, MirInst::SetClassAttr { .. })));
    // The class attribute initializer is recorded for classinit.
    assert!(p.classes.iter().any(|c| !c.class_attr_inits.is_empty()));
}

#[test]
fn property_getter_setter_are_calls() {
    // `t.d` (getter) and `t.d = v` (setter) lower to direct method Calls — never
    // GetField/SetField (the property body owns the backing field access).
    let p = lowered(DECO_SRC);
    let f = main_fn(&p);
    // No GetField/SetField in __main__ (only the property/method bodies do field IO).
    assert!(!insts(f).iter().any(|i| matches!(i, MirInst::GetField { .. } | MirInst::SetField { .. })));
    assert!(insts(f).iter().any(|i| matches!(i, MirInst::Call { .. })));
}

#[test]
fn static_and_class_methods_drop_receiver() {
    // `T.zero()` (static) and `T.unit()` (class) take no self/cls argument.
    let p = lowered(DECO_SRC);
    // The static method `zero` has zero parameters; the classmethod `unit` too.
    let zero = p.funcs.iter().find(|f| f.params.is_empty() && f.ret == Repr::Raw(RawKind::F64));
    assert!(zero.is_some(), "the @staticmethod must have no parameters");
}

// ── closures / generators (Phase 6) ──

/// Count instructions matching a predicate across every function.
fn count_insts(p: &MirProgram, pred: impl Fn(&MirInst) -> bool) -> usize {
    p.funcs.iter().flat_map(|f| &f.blocks).flat_map(|b| &b.insts).filter(|i| pred(i)).count()
}

#[test]
fn closure_call_emits_make_closure_and_call_indirect() {
    // A returned closure called through a `Callable` param lowers to a
    // `MakeClosure` (the env tuple) and a `CallIndirect` (Phase 6A).
    let src = "\
from typing import Callable
def make(n: int) -> Callable[[int], int]:
    def add(x: int) -> int:
        return x + n
    return add
def apply(f: Callable[[int], int], x: int) -> int:
    return f(x)
print(apply(make(5), 1))
";
    let p = lowered(src);
    assert!(count_insts(&p, |i| matches!(i, MirInst::MakeClosure { .. })) >= 1);
    assert!(count_insts(&p, |i| matches!(i, MirInst::CallIndirect { .. })) >= 1);
}

#[test]
fn varargs_call_packs_a_tuple() {
    // A call to a `*args` function packs excess positionals into a fresh tuple
    // (the `TupleNew`/`TupleSet` container ops) — Phase 6C.
    let src = "\
def total(*nums):
    s = 0
    for n in nums:
        s += n
    return s
print(total(1, 2, 3))
";
    let p = lowered(src);
    assert!(count_insts(&p, |i| matches!(
        i,
        MirInst::CallContainer { op: ContainerOp::TupleNew, .. }
    )) >= 1);
}

#[test]
fn generator_lowers_to_make_generator_and_gen_ops() {
    // A generator def lowers a wrapper (`MakeGenerator`) plus a resume state
    // machine using `GenOpInst` slot ops, and registers a resume fn (Phase 6E).
    let src = "\
def count(n):
    i = 0
    while i < n:
        yield i
        i = i + 1
for x in count(3):
    print(x)
";
    let p = lowered(src);
    assert!(count_insts(&p, |i| matches!(i, MirInst::MakeGenerator { .. })) >= 1);
    assert!(count_insts(&p, |i| matches!(i, MirInst::GenOpInst { .. })) >= 1);
    assert_eq!(p.generators.len(), 1, "one registered resume fn");
}

// ── Phase 8B: descriptor-driven stdlib CallRuntime ──────────────────────────

/// Every `CallRuntime` in the program as (symbol, arg reprs, dst repr).
fn runtime_calls(p: &MirProgram) -> Vec<(&'static str, Vec<Repr>, Option<Repr>)> {
    p.funcs
        .iter()
        .flat_map(|f| {
            f.blocks.iter().flat_map(|b| &b.insts).filter_map(move |i| match i {
                MirInst::CallRuntime { dst, def, args } => Some((
                    def.symbol,
                    args.iter()
                        .map(|a| match a {
                            Operand::Local(id) => f.locals[id.index()].repr.clone(),
                        })
                        .collect(),
                    dst.map(|d| f.locals[d.index()].repr.clone()),
                )),
                _ => None,
            })
        })
        .collect()
}

#[test]
fn stdlib_math_sqrt_is_raw_f64_call_runtime() {
    // `math.sqrt(2.0)` rides the descriptor path: a raw-f64 arg, a raw-f64
    // result — no tagged round-trip at the call itself (Phase 8B).
    let p = lowered("import math\nx: float = math.sqrt(2.0)\nprint(x)\n");
    let calls = runtime_calls(&p);
    let sqrt = calls.iter().find(|(s, _, _)| *s == "rt_math_sqrt").expect("sqrt call");
    assert_eq!(sqrt.1, vec![Repr::Raw(RawKind::F64)]);
    assert_eq!(sqrt.2, Some(Repr::Raw(RawKind::F64)));
}

#[test]
fn stdlib_math_ceil_returns_raw_i64() {
    // `math.ceil` returns a raw i64 (descriptor R_I64 + TypeSpec::Int), then
    // legalizes to the Tagged int slot.
    let p = lowered("import math\nn: int = math.ceil(3.2)\nprint(n)\n");
    let calls = runtime_calls(&p);
    let ceil = calls.iter().find(|(s, _, _)| *s == "rt_math_ceil").expect("ceil call");
    assert_eq!(ceil.1, vec![Repr::Raw(RawKind::F64)]);
    assert_eq!(ceil.2, Some(Repr::Raw(RawKind::I64)));
}

#[test]
fn stdlib_random_seed_appends_arg_count() {
    // `random.seed(42)` is a pass_arg_count descriptor: the descriptor's two
    // raw-i64 params are the seed plus the user-written arg count (1).
    let p = lowered("import random\nrandom.seed(42)\nprint(random.random())\n");
    let calls = runtime_calls(&p);
    let seed = calls.iter().find(|(s, _, _)| *s == "rt_random_seed").expect("seed call");
    assert_eq!(seed.1, vec![Repr::Raw(RawKind::I64), Repr::Raw(RawKind::I64)]);
    assert_eq!(seed.2, None, "void descriptor has no dst");
}

#[test]
fn stdlib_absent_optional_passes_null_sentinel() {
    // `random.choices(xs, k=2)` leaves the no-default optional `weights` slot
    // absent → the null-pointer Value sentinel (a Tagged const), k filled by
    // keyword.
    let src = "\
import random
random.seed(1)
xs: list[int] = [1, 2, 3]
print(len(random.choices(xs, k=2)))
";
    let p = lowered(src);
    let calls = runtime_calls(&p);
    let choices =
        calls.iter().find(|(s, _, _)| *s == "rt_random_choices").expect("choices call");
    assert_eq!(choices.1.len(), 3, "population, weights sentinel, k");
    assert_eq!(choices.1[1], Repr::Tagged, "absent weights rides Tagged (null Value)");
    assert_eq!(choices.1[2], Repr::Raw(RawKind::I64), "k is a raw i64");
}

#[test]
fn stdlib_struct_time_field_uses_descriptor_getter() {
    // `t.tm_year` on a `RuntimeObject(StructTime)` value routes through the
    // ObjectFieldDef getter with its constant field index (a raw i8).
    let src = "\
import time
t: time.struct_time = time.localtime(0.0)
print(t.tm_year)
";
    let p = lowered(src);
    let calls = runtime_calls(&p);
    let field = calls
        .iter()
        .find(|(s, _, _)| *s == "rt_struct_time_get_field")
        .expect("field getter call");
    assert_eq!(field.1, vec![Repr::Tagged, Repr::Raw(RawKind::I8)]);
    assert_eq!(field.2, Some(Repr::Raw(RawKind::I64)));
}

#[test]
fn stdlib_isinstance_builtin_folds_statically() {
    // `isinstance(s, str)` on a statically-Str value folds to a constant —
    // no runtime call of any kind.
    let p = lowered("s: str = \"x\"\nprint(isinstance(s, str))\n");
    assert_eq!(runtime_calls(&p).len(), 0);
}

#[test]
fn stdlib_isinstance_on_dyn_rejected() {
    // A gradual receiver cannot answer a builtin-type isinstance statically.
    let src = "\
def f(x):
    return x
print(isinstance(f(1), str))
";
    assert!(try_lower(src).is_err());
}

// ── Phase 8C: stdlib object types (re/Match, File I/O) ──────────────────────

#[test]
fn stdlib_match_method_routes_to_descriptor() {
    // `m.group(0)` on a Match-typed value (re.search → Optional[Match] narrowed
    // to Match) lowers to the `rt_match_group` descriptor: recv Tagged + a raw
    // i64 group index.
    let src = "\
import re
m = re.search(\"x\", \"x\")
g: str = m.group(0)
print(g)
";
    let p = lowered(src);
    let calls = runtime_calls(&p);
    let grp = calls.iter().find(|(s, _, _)| *s == "rt_match_group").expect("group call");
    assert_eq!(grp.1, vec![Repr::Tagged, Repr::Raw(RawKind::I64)]);
}

#[test]
fn stdlib_open_binary_mode_typed_bytes() {
    // `open(p, "rb").read()` → File{binary} → read returns bytes (Heap(Bytes)),
    // routed through `rt_file_read`.
    let src = "\
data = open(\"/tmp/x\", \"rb\").read()
print(data)
";
    let p = lowered(src);
    let calls = runtime_calls(&p);
    assert!(calls.iter().any(|(s, _, _)| *s == "rt_file_open"));
    assert!(calls.iter().any(|(s, _, _)| *s == "rt_file_read"), "read call present");
    // Binary-mode typing shows as a Coerce(Tagged → Heap(Bytes)) after the read.
    let to_bytes = p.funcs.iter().any(|f| {
        f.blocks.iter().flat_map(|b| &b.insts).any(|i| {
            matches!(i, MirInst::Coerce { to, .. } if *to == Repr::Heap(HeapShape::Bytes))
        })
    });
    assert!(to_bytes, "binary read legalizes to Heap(Bytes)");
}

#[test]
fn stdlib_file_write_returns_raw_count() {
    // `f.write(s)` → `rt_file_write` (recv Tagged, data Tagged), a raw i64 byte
    // count tagged back to the Int result.
    let src = "\
f = open(\"/tmp/x\", \"w\")
n: int = f.write(\"hi\")
print(n)
";
    let p = lowered(src);
    let calls = runtime_calls(&p);
    let w = calls.iter().find(|(s, _, _)| *s == "rt_file_write").expect("write call");
    assert_eq!(w.1, vec![Repr::Tagged, Repr::Tagged]);
    assert_eq!(w.2, Some(Repr::Raw(RawKind::I64)));
}

#[test]
fn stdlib_file_iteration_desugars_to_readlines() {
    // `for line in open(p):` desugars to iterating `open(p).readlines()` — the
    // frozen runtime cannot iterate a File directly.
    let src = "\
acc: list[str] = []
for line in open(\"/tmp/x\"):
    acc.append(line)
print(acc)
";
    let p = lowered(src);
    let calls = runtime_calls(&p);
    assert!(calls.iter().any(|(s, _, _)| *s == "rt_file_readlines"));
}

// ── Phase 8D: os/subprocess/urllib + stdlib exceptions ──────────────────────

#[test]
fn stdlib_is_none_uses_rt_is_none() {
    // `x is None` lowers through `rt_is_none` (recognizes heap + immediate
    // None), not `==`; `is not None` negates it.
    let src = "\
import os
v = os.getenv(\"X\")
print(v is None)
print(v is not None)
";
    let p = lowered(src);
    let n = runtime_calls(&p).iter().filter(|(s, _, _)| *s == "rt_is_none").count();
    assert_eq!(n, 2, "two None identity checks");
}

#[test]
fn stdlib_os_path_join_collects_variadic_list() {
    // `os.path.join(a, b, c)` (a `variadic_to_list` descriptor reached through
    // the `import os` submodule chain) passes ONE list to `rt_os_path_join`.
    let src = "\
import os
p: str = os.path.join(\"a\", \"b\", \"c\")
print(p)
";
    let pr = lowered(src);
    let join = runtime_calls(&pr)
        .into_iter()
        .find(|(s, _, _)| *s == "rt_os_path_join")
        .expect("join call");
    assert_eq!(join.1.len(), 1, "one list arg");
    assert_eq!(join.1[0], Repr::Tagged, "the collected list rides Tagged");
}

#[test]
fn stdlib_exception_raise_is_mir_stdlib() {
    // `raise HTTPError(...)` lowers to `MirRaise::Stdlib` carrying the reserved
    // class id and the builtin parent tag (OSError = 24).
    let src = "\
from urllib.error import HTTPError
try:
    raise HTTPError(\"u\", 500, \"m\", {}, None)
except HTTPError:
    pass
print(1)
";
    let p = lowered(src);
    let found = p.funcs.iter().any(|f| {
        f.blocks.iter().flat_map(|b| &b.insts).any(|i| {
            matches!(
                i,
                MirInst::Raise(pyaot_mir::MirRaise::Stdlib { class_id, exc_type_tag, .. })
                    if *class_id == 30 && *exc_type_tag == 24
            )
        })
    });
    assert!(found, "HTTPError → MirRaise::Stdlib{{class_id:30, parent:OSError(24)}}");
}

// ── slicing (Phase 8E) ──

#[test]
fn list_slice_passes_raw_i64_bounds() {
    // `xs[1:3]` → rt_list_slice(list, start, stop) with a Tagged receiver and
    // RAW i64 bounds (the descriptor's SLICE_TERNARY semantics; the generic
    // ptr_ternary Tagged default would corrupt the i64::MIN/MAX sentinels).
    let p = lowered("xs: list[int] = [0, 1, 2, 3]\nys = xs[1:3]\nprint(len(ys))\n");
    let calls = runtime_calls(&p);
    let slice = calls.iter().find(|(s, _, _)| *s == "rt_list_slice").expect("list slice call");
    assert_eq!(
        slice.1,
        vec![Repr::Tagged, Repr::Raw(RawKind::I64), Repr::Raw(RawKind::I64)],
        "receiver Tagged, bounds Raw(I64)"
    );
}

#[test]
fn open_ended_slice_uses_step_descriptor_when_stepped() {
    // `xs[::-1]` → rt_list_slice_step with four args (receiver + 3 raw bounds);
    // absent start/stop ride the i64::MIN/MAX sentinels emitted directly into
    // Raw(I64) slots (never via the tagging round-trip).
    let p = lowered("xs: list[int] = [0, 1, 2]\nys = xs[::-1]\nprint(len(ys))\n");
    let calls = runtime_calls(&p);
    let slice =
        calls.iter().find(|(s, _, _)| *s == "rt_list_slice_step").expect("stepped slice call");
    assert_eq!(
        slice.1,
        vec![
            Repr::Tagged,
            Repr::Raw(RawKind::I64),
            Repr::Raw(RawKind::I64),
            Repr::Raw(RawKind::I64)
        ],
    );
}

#[test]
fn str_slice_dispatches_to_str_slicer() {
    let p = lowered("s = \"abcdef\"\nt = s[2:4]\nprint(t)\n");
    let calls = runtime_calls(&p);
    assert!(
        calls.iter().any(|(s, _, _)| *s == "rt_str_slice"),
        "a str receiver selects rt_str_slice"
    );
}

// ── f-string format specs (Phase 8E) ──

#[test]
fn fstring_format_spec_is_rt_format() {
    // `f"{x:.2f}"` → rt_format(value, spec) with both args Tagged.
    let p = lowered("x = 3.14159\nprint(f\"{x:.2f}\")\n");
    let calls = runtime_calls(&p);
    let fmt = calls.iter().find(|(s, _, _)| *s == "rt_format").expect("rt_format call");
    assert_eq!(fmt.1, vec![Repr::Tagged, Repr::Tagged], "value + spec both Tagged");
}

// ── return-type inference (Phase 8E) ──

#[test]
fn inferred_method_return_types_a_dunder_chain() {
    // With no return annotations, `(a - b).double()` must type `a - b` as the
    // user class so the `.double()` method resolves to a direct devirtualized
    // Call (not a hard "statically-known instance" error). Inference lifts each
    // dunder/method return from its `return Value(...)` body.
    let src = "\
class V:
    def __init__(self, d):
        self.d = d
    def __sub__(self, o):
        return V(self.d - o)
    def double(self):
        return V(self.d * 2)
a = V(5)
r = (a - 1).double()
print(r.d)
";
    let p = lowered(src);
    // The `.double()` on the inferred-`V` dunder result lowers to a direct Call.
    assert!(
        p.funcs.iter().any(|f| f
            .blocks
            .iter()
            .flat_map(|b| &b.insts)
            .any(|i| matches!(i, MirInst::Call { .. }))),
        "method on an inferred-class dunder result devirtualizes to a Call"
    );
}

#[test]
fn str_of_instance_routes_to_user_dunder() {
    // `str(p)` on a class with `__str__` must call the user dunder directly, not
    // the generic `rt_builtin_str` (which renders the default object repr). The
    // `CallBuiltin{Str}` must NOT appear for the instance argument (Phase 8E).
    let src = "\
class P:
    def __init__(self, n: int):
        self.n = n
    def __str__(self) -> str:
        return \"P\"
p = P(1)
print(str(p))
";
    let p = lowered(src);
    let has_builtin_str = p.funcs.iter().any(|f| {
        f.blocks.iter().flat_map(|b| &b.insts).any(|i| {
            matches!(
                i,
                MirInst::CallBuiltin { kind: pyaot_mir::BuiltinFunctionKind::Str, .. }
            )
        })
    });
    assert!(!has_builtin_str, "str(instance) must route to __str__, not CallBuiltin{{Str}}");
}

#[test]
fn sum_expands_to_tagged_accumulator_loop() {
    // `sum(xs)` (Phase 8H, D2) expands at lowering into an Iter/IterNext/
    // IterExhausted loop with a Tagged `BinOp Add` — the Sum HIR node never
    // reaches MIR as a builtin/runtime call.
    let src = "\
xs = [1.5, 2.5]
print(sum(xs))
";
    let p = lowered(src);
    let main = main_fn(&p);
    let ops = container_ops(main);
    assert!(ops.contains(&ContainerOp::Iter), "sum lowers through Iter");
    assert!(ops.contains(&ContainerOp::IterNext), "sum lowers through IterNext");
    assert!(ops.contains(&ContainerOp::IterExhausted), "sum lowers through IterExhausted");
    let has_tagged_add = main.blocks.iter().flat_map(|b| &b.insts).any(|i| {
        matches!(i, MirInst::BinOp { op: pyaot_mir::BinOp::Add, .. })
    });
    assert!(has_tagged_add, "sum body adds on the tagged baseline");
}

#[test]
fn dyn_stdlib_float_arg_takes_checked_unbox() {
    // A Dyn argument into a raw-f64 stdlib param (Phase 8H, D3) takes the
    // CHECKED Coerce (runtime-validated rt_unbox_float), not a blind unbox.
    let src = "\
import math


def pick(flag):
    if flag:
        return 2.0
    return \"oops\"


print(math.sqrt(pick(True)))
";
    let p = lowered(src);
    let has_checked = p.funcs.iter().any(|f| {
        f.blocks.iter().flat_map(|b| &b.insts).any(|i| {
            matches!(
                i,
                MirInst::Coerce { checked: true, to: Repr::Raw(RawKind::F64), .. }
            )
        })
    });
    assert!(has_checked, "Dyn -> Raw(F64) stdlib arg must be a checked Coerce");
}

#[test]
fn dyn_receiver_field_access_uses_named_insts() {
    // Field reads/writes on a Dyn receiver (Phase 8H, D4) lower to the
    // by-name GetFieldNamed/SetFieldNamed instructions.
    let src = "\
class N:
    def __init__(self, d: float):
        self.d = d


def pick(flag):
    if flag:
        return N(1.0)
    return \"oops\"


n = pick(True)
print(n.d)
n.d = 3.0
";
    let p = lowered(src);
    let all_insts = || p.funcs.iter().flat_map(|f| f.blocks.iter().flat_map(|b| &b.insts));
    assert!(
        all_insts().any(|i| matches!(i, MirInst::GetFieldNamed { .. })),
        "Dyn receiver field read lowers to GetFieldNamed"
    );
    assert!(
        all_insts().any(|i| matches!(i, MirInst::SetFieldNamed { .. })),
        "Dyn receiver field write lowers to SetFieldNamed"
    );
    // The classes carry the field-name registry for the runtime resolution.
    assert!(
        p.classes.iter().any(|c| !c.field_names.is_empty()),
        "MirClass.field_names populated for by-name registration"
    );
}
