//! Tests for peephole optimizations

use indexmap::IndexMap;
use pyaot_mir::{
    BasicBlock, BinOp, Constant, Function, Instruction, InstructionKind, Local, Module, Operand,
    RuntimeFunc, Terminator, UnOp,
};
use pyaot_types::Type;
use pyaot_utils::{BlockId, FuncId, LocalId};

fn make_local(id: u32, ty: Type) -> Local {
    Local {
        id: LocalId::from(id),
        name: None,
        ty,
        is_gc_root: false,
    }
}

fn make_inst(kind: InstructionKind) -> Instruction {
    Instruction { kind, span: None }
}

fn make_func(locals: Vec<Local>, instructions: Vec<InstructionKind>) -> Function {
    let func_id = FuncId::from(0u32);
    let block_id = BlockId::from(0u32);

    let mut local_map = IndexMap::new();
    for l in &locals {
        local_map.insert(l.id, l.clone());
    }

    let mut blocks = IndexMap::new();
    blocks.insert(
        block_id,
        BasicBlock {
            id: block_id,
            instructions: instructions.into_iter().map(make_inst).collect(),
            terminator: Terminator::Return(None),
        },
    );

    Function {
        id: func_id,
        name: "test".to_string(),
        params: vec![],
        return_type: Type::None,
        locals: local_map,
        blocks,
        entry_block: block_id,
        span: None,
        is_ssa: false,
        dom_tree_cache: std::cell::OnceCell::new(),
    }
}

fn make_module(func: Function) -> Module {
    let mut module = Module::new();
    module.add_function(func);
    module
}

fn get_instructions(module: &Module) -> &[Instruction] {
    let func = module.functions.values().next().unwrap();
    let block = func.blocks.values().next().unwrap();
    &block.instructions
}

// ==================== Identity elimination ====================

#[test]
fn test_add_zero_right() {
    // _1 = _0 + 0  →  _1 = _0
    let locals = vec![make_local(0, Type::Int), make_local(1, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(1u32),
        op: BinOp::Add,
        left: Operand::Local(LocalId::from(0u32)),
        right: Operand::Constant(Constant::Int(0)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    match &insts[0].kind {
        InstructionKind::Copy { dest, src } => {
            assert_eq!(*dest, LocalId::from(1u32));
            assert!(matches!(src, Operand::Local(id) if *id == LocalId::from(0u32)));
        }
        other => panic!("Expected Copy, got {:?}", other),
    }
}

#[test]
fn test_add_zero_left() {
    // _1 = 0 + _0  →  _1 = _0
    let locals = vec![make_local(0, Type::Int), make_local(1, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(1u32),
        op: BinOp::Add,
        left: Operand::Constant(Constant::Int(0)),
        right: Operand::Local(LocalId::from(0u32)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    assert!(matches!(&insts[0].kind, InstructionKind::Copy { .. }));
}

#[test]
fn test_mul_one() {
    // _1 = _0 * 1  →  _1 = _0
    let locals = vec![make_local(0, Type::Int), make_local(1, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(1u32),
        op: BinOp::Mul,
        left: Operand::Local(LocalId::from(0u32)),
        right: Operand::Constant(Constant::Int(1)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    assert!(matches!(&insts[0].kind, InstructionKind::Copy { .. }));
}

// ==================== Zero/absorbing ====================

#[test]
fn test_mul_zero() {
    // _1 = _0 * 0  →  _1 = 0
    let locals = vec![make_local(0, Type::Int), make_local(1, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(1u32),
        op: BinOp::Mul,
        left: Operand::Local(LocalId::from(0u32)),
        right: Operand::Constant(Constant::Int(0)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    match &insts[0].kind {
        InstructionKind::Const {
            value: Constant::Int(0),
            ..
        } => {}
        other => panic!("Expected Const(Int(0)), got {:?}", other),
    }
}

#[test]
fn test_bitand_zero() {
    // _1 = _0 & 0  →  _1 = 0
    let locals = vec![make_local(0, Type::Int), make_local(1, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(1u32),
        op: BinOp::BitAnd,
        left: Operand::Local(LocalId::from(0u32)),
        right: Operand::Constant(Constant::Int(0)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    assert!(matches!(
        &insts[0].kind,
        InstructionKind::Const {
            value: Constant::Int(0),
            ..
        }
    ));
}

// ==================== Strength reduction ====================

#[test]
fn test_mul_two_strength_reduction() {
    // _1 = _0 * 2  →  _1 = _0 + _0
    let locals = vec![make_local(0, Type::Int), make_local(1, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(1u32),
        op: BinOp::Mul,
        left: Operand::Local(LocalId::from(0u32)),
        right: Operand::Constant(Constant::Int(2)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    match &insts[0].kind {
        InstructionKind::BinOp {
            op: BinOp::Add,
            left,
            right,
            ..
        } => {
            assert!(matches!(left, Operand::Local(id) if *id == LocalId::from(0u32)));
            assert!(matches!(right, Operand::Local(id) if *id == LocalId::from(0u32)));
        }
        other => panic!("Expected Add(_0, _0), got {:?}", other),
    }
}

#[test]
fn test_mul_power_of_two() {
    // _1 = _0 * 8  →  _1 = _0 << 3
    let locals = vec![make_local(0, Type::Int), make_local(1, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(1u32),
        op: BinOp::Mul,
        left: Operand::Local(LocalId::from(0u32)),
        right: Operand::Constant(Constant::Int(8)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    match &insts[0].kind {
        InstructionKind::BinOp {
            op: BinOp::LShift,
            right: Operand::Constant(Constant::Int(3)),
            ..
        } => {}
        other => panic!("Expected LShift by 3, got {:?}", other),
    }
}

#[test]
fn test_floordiv_power_of_two() {
    // _1 = _0 // 4  →  _1 = _0 >> 2
    let locals = vec![make_local(0, Type::Int), make_local(1, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(1u32),
        op: BinOp::FloorDiv,
        left: Operand::Local(LocalId::from(0u32)),
        right: Operand::Constant(Constant::Int(4)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    match &insts[0].kind {
        InstructionKind::BinOp {
            op: BinOp::RShift,
            right: Operand::Constant(Constant::Int(2)),
            ..
        } => {}
        other => panic!("Expected RShift by 2, got {:?}", other),
    }
}

// ==================== Power patterns ====================

#[test]
fn test_pow_zero() {
    // _1 = _0 ** 0  →  _1 = 1
    let locals = vec![make_local(0, Type::Int), make_local(1, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(1u32),
        op: BinOp::Pow,
        left: Operand::Local(LocalId::from(0u32)),
        right: Operand::Constant(Constant::Int(0)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    assert!(matches!(
        &insts[0].kind,
        InstructionKind::Const {
            value: Constant::Int(1),
            ..
        }
    ));
}

#[test]
fn test_pow_two() {
    // _1 = _0 ** 2  →  _1 = _0 * _0
    let locals = vec![make_local(0, Type::Int), make_local(1, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(1u32),
        op: BinOp::Pow,
        left: Operand::Local(LocalId::from(0u32)),
        right: Operand::Constant(Constant::Int(2)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    match &insts[0].kind {
        InstructionKind::BinOp { op: BinOp::Mul, .. } => {}
        other => panic!("Expected Mul, got {:?}", other),
    }
}

// ==================== Same operand ====================

#[test]
fn test_sub_self() {
    // _1 = _0 - _0  →  _1 = 0
    let locals = vec![make_local(0, Type::Int), make_local(1, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(1u32),
        op: BinOp::Sub,
        left: Operand::Local(LocalId::from(0u32)),
        right: Operand::Local(LocalId::from(0u32)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    assert!(matches!(
        &insts[0].kind,
        InstructionKind::Const {
            value: Constant::Int(0),
            ..
        }
    ));
}

#[test]
fn test_xor_self() {
    // _1 = _0 ^ _0  →  _1 = 0
    let locals = vec![make_local(0, Type::Int), make_local(1, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(1u32),
        op: BinOp::BitXor,
        left: Operand::Local(LocalId::from(0u32)),
        right: Operand::Local(LocalId::from(0u32)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    assert!(matches!(
        &insts[0].kind,
        InstructionKind::Const {
            value: Constant::Int(0),
            ..
        }
    ));
}

// ==================== Pair patterns ====================

#[test]
fn test_box_unbox_elimination() {
    // _1 = BoxInt(_0)
    // _2 = UnboxInt(_1)  →  _2 = Copy(_0)
    let locals = vec![
        make_local(0, Type::Int),
        make_local(1, Type::Int),
        make_local(2, Type::Int),
    ];
    let instructions = vec![
        InstructionKind::RuntimeCall {
            dest: LocalId::from(1u32),
            func: RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_BOX_INT),
            args: vec![Operand::Local(LocalId::from(0u32))],
        },
        InstructionKind::RuntimeCall {
            dest: LocalId::from(2u32),
            func: RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_UNBOX_INT),
            args: vec![Operand::Local(LocalId::from(1u32))],
        },
    ];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    // First instruction unchanged (BoxInt)
    assert!(matches!(
        &insts[0].kind,
        InstructionKind::RuntimeCall { .. }
    ));
    // Second instruction replaced with Copy
    match &insts[1].kind {
        InstructionKind::Copy { dest, src } => {
            assert_eq!(*dest, LocalId::from(2u32));
            assert!(matches!(src, Operand::Local(id) if *id == LocalId::from(0u32)));
        }
        other => panic!("Expected Copy, got {:?}", other),
    }
}

#[test]
fn test_double_negation() {
    // _1 = -_0
    // _2 = -_1  →  _2 = Copy(_0)
    let locals = vec![
        make_local(0, Type::Int),
        make_local(1, Type::Int),
        make_local(2, Type::Int),
    ];
    let instructions = vec![
        InstructionKind::UnOp {
            dest: LocalId::from(1u32),
            op: UnOp::Neg,
            operand: Operand::Local(LocalId::from(0u32)),
        },
        InstructionKind::UnOp {
            dest: LocalId::from(2u32),
            op: UnOp::Neg,
            operand: Operand::Local(LocalId::from(1u32)),
        },
    ];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    match &insts[1].kind {
        InstructionKind::Copy { dest, src } => {
            assert_eq!(*dest, LocalId::from(2u32));
            assert!(matches!(src, Operand::Local(id) if *id == LocalId::from(0u32)));
        }
        other => panic!("Expected Copy, got {:?}", other),
    }
}

#[test]
fn test_bitcast_roundtrip() {
    // _1 = FloatBits(_0)
    // _2 = IntBitsToFloat(_1)  →  _2 = Copy(_0)
    let locals = vec![
        make_local(0, Type::Float),
        make_local(1, Type::Int),
        make_local(2, Type::Float),
    ];
    let instructions = vec![
        InstructionKind::FloatBits {
            dest: LocalId::from(1u32),
            src: Operand::Local(LocalId::from(0u32)),
        },
        InstructionKind::IntBitsToFloat {
            dest: LocalId::from(2u32),
            src: Operand::Local(LocalId::from(1u32)),
        },
    ];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    match &insts[1].kind {
        InstructionKind::Copy { dest, src } => {
            assert_eq!(*dest, LocalId::from(2u32));
            assert!(matches!(src, Operand::Local(id) if *id == LocalId::from(0u32)));
        }
        other => panic!("Expected Copy, got {:?}", other),
    }
}

// ==================== Float identity ====================

#[test]
fn test_float_mul_one() {
    // _1 = _0 * 1.0  →  _1 = _0
    let locals = vec![make_local(0, Type::Float), make_local(1, Type::Float)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(1u32),
        op: BinOp::Mul,
        left: Operand::Local(LocalId::from(0u32)),
        right: Operand::Constant(Constant::Float(1.0)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    assert!(matches!(&insts[0].kind, InstructionKind::Copy { .. }));
}

// ==================== No-op cases (should NOT transform) ====================

#[test]
fn test_no_transform_mul_three() {
    // _1 = _0 * 3  →  should stay as-is (3 is not a power of 2)
    let locals = vec![make_local(0, Type::Int), make_local(1, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(1u32),
        op: BinOp::Mul,
        left: Operand::Local(LocalId::from(0u32)),
        right: Operand::Constant(Constant::Int(3)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    assert!(matches!(
        &insts[0].kind,
        InstructionKind::BinOp { op: BinOp::Mul, .. }
    ));
}

// ==================== Idempotent same-operand (SSA-aware) ====================

/// `x & x → x` (bitwise idempotent). Under SSA, LocalId equality is
/// sufficient for value equality since each local has a single def.
#[test]
fn test_bitand_same_operand_folds_to_copy() {
    let locals = vec![make_local(0, Type::Int), make_local(1, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(1u32),
        op: BinOp::BitAnd,
        left: Operand::Local(LocalId::from(0u32)),
        right: Operand::Local(LocalId::from(0u32)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    match &insts[0].kind {
        InstructionKind::Copy {
            src: Operand::Local(id),
            ..
        } => assert_eq!(id.0, 0),
        other => panic!("Expected Copy(Local(0)), got {:?}", other),
    }
}

/// `x | x → x` (bitwise idempotent).
#[test]
fn test_bitor_same_operand_folds_to_copy() {
    let locals = vec![make_local(0, Type::Int), make_local(1, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(1u32),
        op: BinOp::BitOr,
        left: Operand::Local(LocalId::from(0u32)),
        right: Operand::Local(LocalId::from(0u32)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    match &insts[0].kind {
        InstructionKind::Copy {
            src: Operand::Local(id),
            ..
        } => assert_eq!(id.0, 0),
        other => panic!("Expected Copy(Local(0)), got {:?}", other),
    }
}

/// Different operands must NOT fire the same-operand rule: `a & b` stays.
#[test]
fn test_bitand_different_operands_stays() {
    let locals = vec![
        make_local(0, Type::Int),
        make_local(1, Type::Int),
        make_local(2, Type::Int),
    ];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(2u32),
        op: BinOp::BitAnd,
        left: Operand::Local(LocalId::from(0u32)),
        right: Operand::Local(LocalId::from(1u32)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::run_peephole(&mut module);

    let insts = get_instructions(&module);
    assert!(matches!(
        &insts[0].kind,
        InstructionKind::BinOp {
            op: BinOp::BitAnd,
            ..
        }
    ));
}
