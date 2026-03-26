//! Integration tests for constant folding & propagation

use indexmap::IndexMap;
use pyaot_mir::{
    BasicBlock, BinOp, Constant, Function, Instruction, InstructionKind, Local, Module, Operand,
    Terminator, UnOp,
};
use pyaot_types::Type;
use pyaot_utils::{BlockId, FuncId, LocalId, StringInterner};

fn make_local(id: u32, ty: Type) -> Local {
    Local {
        id: LocalId::from(id),
        name: None,
        ty,
        is_gc_root: false,
    }
}

fn make_instruction(kind: InstructionKind) -> Instruction {
    Instruction { kind, span: None }
}

/// Helper: create a minimal function with one block and given instructions.
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
            instructions: instructions.into_iter().map(make_instruction).collect(),
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

#[test]
fn test_fold_int_addition() {
    // x = 2 + 3  →  x = 5
    let locals = vec![make_local(0, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(0u32),
        op: BinOp::Add,
        left: Operand::Constant(Constant::Int(2)),
        right: Operand::Constant(Constant::Int(3)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    let mut interner = StringInterner::new();
    super::fold_constants(&mut module, &mut interner);

    let insts = get_instructions(&module);
    assert_eq!(insts.len(), 1);
    match &insts[0].kind {
        InstructionKind::Const {
            value: Constant::Int(5),
            ..
        } => {}
        other => panic!("Expected Const(Int(5)), got {:?}", other),
    }
}

#[test]
fn test_propagate_and_fold() {
    // _0 = 10
    // _1 = 20
    // _2 = _0 + _1  →  _2 = 30
    let locals = vec![
        make_local(0, Type::Int),
        make_local(1, Type::Int),
        make_local(2, Type::Int),
    ];
    let instructions = vec![
        InstructionKind::Const {
            dest: LocalId::from(0u32),
            value: Constant::Int(10),
        },
        InstructionKind::Const {
            dest: LocalId::from(1u32),
            value: Constant::Int(20),
        },
        InstructionKind::BinOp {
            dest: LocalId::from(2u32),
            op: BinOp::Add,
            left: Operand::Local(LocalId::from(0u32)),
            right: Operand::Local(LocalId::from(1u32)),
        },
    ];

    let mut module = make_module(make_func(locals, instructions));
    let mut interner = StringInterner::new();
    super::fold_constants(&mut module, &mut interner);

    let insts = get_instructions(&module);
    // After propagation + folding: all three are Const instructions
    // _0 = 10, _1 = 20, _2 = 30
    assert_eq!(insts.len(), 3);
    match &insts[2].kind {
        InstructionKind::Const {
            value: Constant::Int(30),
            ..
        } => {}
        other => panic!("Expected Const(Int(30)), got {:?}", other),
    }
}

#[test]
fn test_transitive_propagation() {
    // _0 = 5
    // _1 = _0  (copy)
    // _2 = _1 + _1  →  _2 = 10
    let locals = vec![
        make_local(0, Type::Int),
        make_local(1, Type::Int),
        make_local(2, Type::Int),
    ];
    let instructions = vec![
        InstructionKind::Const {
            dest: LocalId::from(0u32),
            value: Constant::Int(5),
        },
        InstructionKind::Copy {
            dest: LocalId::from(1u32),
            src: Operand::Local(LocalId::from(0u32)),
        },
        InstructionKind::BinOp {
            dest: LocalId::from(2u32),
            op: BinOp::Add,
            left: Operand::Local(LocalId::from(1u32)),
            right: Operand::Local(LocalId::from(1u32)),
        },
    ];

    let mut module = make_module(make_func(locals, instructions));
    let mut interner = StringInterner::new();
    super::fold_constants(&mut module, &mut interner);

    let insts = get_instructions(&module);
    assert_eq!(insts.len(), 3);
    // _1 = Copy(Const(5)) → Const(5)  (copy of constant folded)
    match &insts[1].kind {
        InstructionKind::Const {
            value: Constant::Int(5),
            ..
        } => {}
        other => panic!("Expected _1 = Const(5), got {:?}", other),
    }
    // _2 = 5 + 5 = 10
    match &insts[2].kind {
        InstructionKind::Const {
            value: Constant::Int(10),
            ..
        } => {}
        other => panic!("Expected _2 = Const(10), got {:?}", other),
    }
}

#[test]
fn test_fold_float_multiplication() {
    let locals = vec![make_local(0, Type::Float)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(0u32),
        op: BinOp::Mul,
        left: Operand::Constant(Constant::Float(2.5)),
        right: Operand::Constant(Constant::Float(4.0)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    let mut interner = StringInterner::new();
    super::fold_constants(&mut module, &mut interner);

    let insts = get_instructions(&module);
    match &insts[0].kind {
        InstructionKind::Const {
            value: Constant::Float(v),
            ..
        } => assert_eq!(*v, 10.0),
        other => panic!("Expected Const(Float(10.0)), got {:?}", other),
    }
}

#[test]
fn test_fold_string_concatenation() {
    let mut interner = StringInterner::new();
    let hello = interner.intern("hello");
    let world = interner.intern(" world");

    let locals = vec![make_local(0, Type::Str)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(0u32),
        op: BinOp::Add,
        left: Operand::Constant(Constant::Str(hello)),
        right: Operand::Constant(Constant::Str(world)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    super::fold_constants(&mut module, &mut interner);

    let insts = get_instructions(&module);
    match &insts[0].kind {
        InstructionKind::Const {
            value: Constant::Str(s),
            ..
        } => assert_eq!(interner.resolve(*s), "hello world"),
        other => panic!("Expected Const(Str(\"hello world\")), got {:?}", other),
    }
}

#[test]
fn test_fold_comparison() {
    let locals = vec![make_local(0, Type::Bool)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(0u32),
        op: BinOp::Lt,
        left: Operand::Constant(Constant::Int(3)),
        right: Operand::Constant(Constant::Int(5)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    let mut interner = StringInterner::new();
    super::fold_constants(&mut module, &mut interner);

    let insts = get_instructions(&module);
    match &insts[0].kind {
        InstructionKind::Const {
            value: Constant::Bool(true),
            ..
        } => {}
        other => panic!("Expected Const(Bool(true)), got {:?}", other),
    }
}

#[test]
fn test_fold_unary_negation() {
    let locals = vec![make_local(0, Type::Int)];
    let instructions = vec![InstructionKind::UnOp {
        dest: LocalId::from(0u32),
        op: UnOp::Neg,
        operand: Operand::Constant(Constant::Int(42)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    let mut interner = StringInterner::new();
    super::fold_constants(&mut module, &mut interner);

    let insts = get_instructions(&module);
    match &insts[0].kind {
        InstructionKind::Const {
            value: Constant::Int(-42),
            ..
        } => {}
        other => panic!("Expected Const(Int(-42)), got {:?}", other),
    }
}

#[test]
fn test_fold_bool_to_int_conversion() {
    let locals = vec![make_local(0, Type::Int)];
    let instructions = vec![InstructionKind::BoolToInt {
        dest: LocalId::from(0u32),
        src: Operand::Constant(Constant::Bool(true)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    let mut interner = StringInterner::new();
    super::fold_constants(&mut module, &mut interner);

    let insts = get_instructions(&module);
    match &insts[0].kind {
        InstructionKind::Const {
            value: Constant::Int(1),
            ..
        } => {}
        other => panic!("Expected Const(Int(1)), got {:?}", other),
    }
}

#[test]
fn test_no_fold_on_overflow() {
    // i64::MAX + 1 should NOT fold
    let locals = vec![make_local(0, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(0u32),
        op: BinOp::Add,
        left: Operand::Constant(Constant::Int(i64::MAX)),
        right: Operand::Constant(Constant::Int(1)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    let mut interner = StringInterner::new();
    super::fold_constants(&mut module, &mut interner);

    let insts = get_instructions(&module);
    // Should remain as BinOp
    assert!(matches!(&insts[0].kind, InstructionKind::BinOp { .. }));
}

#[test]
fn test_no_fold_div_by_zero() {
    let locals = vec![make_local(0, Type::Int)];
    let instructions = vec![InstructionKind::BinOp {
        dest: LocalId::from(0u32),
        op: BinOp::FloorDiv,
        left: Operand::Constant(Constant::Int(10)),
        right: Operand::Constant(Constant::Int(0)),
    }];

    let mut module = make_module(make_func(locals, instructions));
    let mut interner = StringInterner::new();
    super::fold_constants(&mut module, &mut interner);

    let insts = get_instructions(&module);
    assert!(matches!(&insts[0].kind, InstructionKind::BinOp { .. }));
}

#[test]
fn test_constant_branch_simplification() {
    // Block 0: branch on True → should become Goto(then_block)
    let locals = vec![make_local(0, Type::Bool)];

    let block0 = BlockId::from(0u32);
    let block1 = BlockId::from(1u32);
    let block2 = BlockId::from(2u32);

    let func_id = FuncId::from(0u32);
    let mut blocks = IndexMap::new();
    blocks.insert(
        block0,
        BasicBlock {
            id: block0,
            instructions: vec![make_instruction(InstructionKind::Const {
                dest: LocalId::from(0u32),
                value: Constant::Bool(true),
            })],
            terminator: Terminator::Branch {
                cond: Operand::Local(LocalId::from(0u32)),
                then_block: block1,
                else_block: block2,
            },
        },
    );
    blocks.insert(
        block1,
        BasicBlock {
            id: block1,
            instructions: vec![],
            terminator: Terminator::Return(None),
        },
    );
    blocks.insert(
        block2,
        BasicBlock {
            id: block2,
            instructions: vec![],
            terminator: Terminator::Return(None),
        },
    );

    let func = Function {
        id: func_id,
        name: "test".to_string(),
        params: vec![],
        return_type: Type::None,
        locals: {
            let mut m = IndexMap::new();
            m.insert(LocalId::from(0u32), locals[0].clone());
            m
        },
        blocks,
        entry_block: block0,
        span: None,
    };

    let mut module = make_module(func);
    let mut interner = StringInterner::new();
    super::fold_constants(&mut module, &mut interner);

    let func = module.functions.values().next().unwrap();
    let term = &func.blocks.get(&block0).unwrap().terminator;
    match term {
        Terminator::Goto(target) => assert_eq!(*target, block1),
        other => panic!("Expected Goto(block1), got {:?}", other),
    }
}
