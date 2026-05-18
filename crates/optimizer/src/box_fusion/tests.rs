//! Tests for the Float box/unbox fusion pass.

use indexmap::IndexMap;
use pyaot_core_defs::runtime_func_def::{RT_BOX_FLOAT, RT_UNBOX_FLOAT};
use pyaot_mir::{
    BasicBlock, Function, FunctionKind, Instruction, InstructionKind, Local, Module, Operand,
    RuntimeFunc, Terminator,
};
use pyaot_types::Type;
use pyaot_utils::{BlockId, FuncId, LocalId};

fn make_local(id: u32, ty: Type) -> Local {
    Local {
        id: LocalId::from(id),
        name: None,
        ty,
        is_gc_root: false,
        abi_immutable: false,
        mir_ty: None,
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
        kind: FunctionKind::Regular,
        name: "test".to_string(),
        params: vec![],
        return_type: Type::None,
        locals: local_map,
        blocks,
        entry_block: block_id,
        span: None,
        is_ssa: false,
        is_generic_template: false,
        typevar_params: Vec::new(),
        wrapper_fn_ptr_capture_index: None,
        phase4_return_abi_flipped: false,
        phase4_original_return_type: None,
        dom_tree_cache: std::cell::OnceCell::new(),
        signature: None,
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
fn box_float_then_unbox_float_collapses_to_copy() {
    // _1 = BoxValue(_0, Float)
    // _2 = UnboxValue(_1, Float)  →  _2 = Copy(_0)
    let locals = vec![
        make_local(0, Type::Float),
        make_local(1, Type::Any),
        make_local(2, Type::Float),
    ];
    let instructions = vec![
        InstructionKind::BoxValue {
            dest: LocalId::from(1u32),
            src: Operand::Local(LocalId::from(0u32)),
            src_type: Type::Float,
        },
        InstructionKind::UnboxValue {
            dest: LocalId::from(2u32),
            src: Operand::Local(LocalId::from(1u32)),
            dest_type: Type::Float,
        },
    ];

    let mut module = make_module(make_func(locals, instructions));
    super::run_box_fusion(&mut module);

    let insts = get_instructions(&module);
    assert!(matches!(
        &insts[0].kind,
        InstructionKind::BoxValue {
            src_type: Type::Float,
            ..
        }
    ));
    match &insts[1].kind {
        InstructionKind::Copy { dest, src } => {
            assert_eq!(*dest, LocalId::from(2u32));
            assert!(matches!(src, Operand::Local(id) if *id == LocalId::from(0u32)));
        }
        other => panic!("Expected Copy, got {:?}", other),
    }
}

#[test]
fn unbox_float_then_box_float_collapses_to_copy() {
    // _1 = UnboxValue(_0, Float)
    // _2 = BoxValue(_1, Float)  →  _2 = Copy(_0)
    let locals = vec![
        make_local(0, Type::Any),
        make_local(1, Type::Float),
        make_local(2, Type::Any),
    ];
    let instructions = vec![
        InstructionKind::UnboxValue {
            dest: LocalId::from(1u32),
            src: Operand::Local(LocalId::from(0u32)),
            dest_type: Type::Float,
        },
        InstructionKind::BoxValue {
            dest: LocalId::from(2u32),
            src: Operand::Local(LocalId::from(1u32)),
            src_type: Type::Float,
        },
    ];

    let mut module = make_module(make_func(locals, instructions));
    super::run_box_fusion(&mut module);

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
fn box_float_mir_then_rt_unbox_float_collapses() {
    // _1 = BoxValue(_0, Float)
    // _2 = rt_unbox_float(_1)  →  _2 = Copy(_0)
    let locals = vec![
        make_local(0, Type::Float),
        make_local(1, Type::Any),
        make_local(2, Type::Float),
    ];
    let instructions = vec![
        InstructionKind::BoxValue {
            dest: LocalId::from(1u32),
            src: Operand::Local(LocalId::from(0u32)),
            src_type: Type::Float,
        },
        InstructionKind::RuntimeCall {
            dest: LocalId::from(2u32),
            func: RuntimeFunc::Call(&RT_UNBOX_FLOAT),
            args: vec![Operand::Local(LocalId::from(1u32))],
        },
    ];

    let mut module = make_module(make_func(locals, instructions));
    super::run_box_fusion(&mut module);

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
fn rt_box_float_then_unbox_float_mir_collapses() {
    // _1 = rt_box_float(_0)
    // _2 = UnboxValue(_1, Float)  →  _2 = Copy(_0)
    let locals = vec![
        make_local(0, Type::Float),
        make_local(1, Type::Any),
        make_local(2, Type::Float),
    ];
    let instructions = vec![
        InstructionKind::RuntimeCall {
            dest: LocalId::from(1u32),
            func: RuntimeFunc::Call(&RT_BOX_FLOAT),
            args: vec![Operand::Local(LocalId::from(0u32))],
        },
        InstructionKind::UnboxValue {
            dest: LocalId::from(2u32),
            src: Operand::Local(LocalId::from(1u32)),
            dest_type: Type::Float,
        },
    ];

    let mut module = make_module(make_func(locals, instructions));
    super::run_box_fusion(&mut module);

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
fn unbox_float_mir_then_rt_box_float_collapses() {
    // _1 = UnboxValue(_0, Float)
    // _2 = rt_box_float(_1)  →  _2 = Copy(_0)
    let locals = vec![
        make_local(0, Type::Any),
        make_local(1, Type::Float),
        make_local(2, Type::Any),
    ];
    let instructions = vec![
        InstructionKind::UnboxValue {
            dest: LocalId::from(1u32),
            src: Operand::Local(LocalId::from(0u32)),
            dest_type: Type::Float,
        },
        InstructionKind::RuntimeCall {
            dest: LocalId::from(2u32),
            func: RuntimeFunc::Call(&RT_BOX_FLOAT),
            args: vec![Operand::Local(LocalId::from(1u32))],
        },
    ];

    let mut module = make_module(make_func(locals, instructions));
    super::run_box_fusion(&mut module);

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
fn rt_unbox_float_then_box_float_mir_collapses() {
    // _1 = rt_unbox_float(_0)
    // _2 = BoxValue(_1, Float)  →  _2 = Copy(_0)
    let locals = vec![
        make_local(0, Type::Any),
        make_local(1, Type::Float),
        make_local(2, Type::Any),
    ];
    let instructions = vec![
        InstructionKind::RuntimeCall {
            dest: LocalId::from(1u32),
            func: RuntimeFunc::Call(&RT_UNBOX_FLOAT),
            args: vec![Operand::Local(LocalId::from(0u32))],
        },
        InstructionKind::BoxValue {
            dest: LocalId::from(2u32),
            src: Operand::Local(LocalId::from(1u32)),
            src_type: Type::Float,
        },
    ];

    let mut module = make_module(make_func(locals, instructions));
    super::run_box_fusion(&mut module);

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
fn non_adjacent_pairs_are_not_collapsed() {
    // _1 = BoxValue(_0, Float)
    // _2 = Copy(_1)              <-- separator, blocks fusion
    // _3 = UnboxValue(_2, Float)
    let locals = vec![
        make_local(0, Type::Float),
        make_local(1, Type::Any),
        make_local(2, Type::Any),
        make_local(3, Type::Float),
    ];
    let instructions = vec![
        InstructionKind::BoxValue {
            dest: LocalId::from(1u32),
            src: Operand::Local(LocalId::from(0u32)),
            src_type: Type::Float,
        },
        InstructionKind::Copy {
            dest: LocalId::from(2u32),
            src: Operand::Local(LocalId::from(1u32)),
        },
        InstructionKind::UnboxValue {
            dest: LocalId::from(3u32),
            src: Operand::Local(LocalId::from(2u32)),
            dest_type: Type::Float,
        },
    ];

    let mut module = make_module(make_func(locals, instructions));
    super::run_box_fusion(&mut module);

    let insts = get_instructions(&module);
    assert_eq!(insts.len(), 3);
    assert!(matches!(&insts[0].kind, InstructionKind::BoxValue { .. }));
    assert!(matches!(&insts[1].kind, InstructionKind::Copy { .. }));
    assert!(matches!(&insts[2].kind, InstructionKind::UnboxValue { .. }));
}

#[test]
fn pair_with_unrelated_inner_local_is_not_collapsed() {
    // _2 = BoxValue(_0, Float)   <-- produces _2
    // _3 = UnboxValue(_1, Float) <-- consumes _1, NOT _2
    let locals = vec![
        make_local(0, Type::Float),
        make_local(1, Type::Any),
        make_local(2, Type::Any),
        make_local(3, Type::Float),
    ];
    let instructions = vec![
        InstructionKind::BoxValue {
            dest: LocalId::from(2u32),
            src: Operand::Local(LocalId::from(0u32)),
            src_type: Type::Float,
        },
        InstructionKind::UnboxValue {
            dest: LocalId::from(3u32),
            src: Operand::Local(LocalId::from(1u32)),
            dest_type: Type::Float,
        },
    ];

    let mut module = make_module(make_func(locals, instructions));
    super::run_box_fusion(&mut module);

    let insts = get_instructions(&module);
    assert!(matches!(&insts[0].kind, InstructionKind::BoxValue { .. }));
    assert!(matches!(&insts[1].kind, InstructionKind::UnboxValue { .. }));
}
