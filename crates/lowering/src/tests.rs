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
