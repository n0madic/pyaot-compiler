//! Tests for the raw-local demotion pass (Phase 5 cross-block box/unbox
//! cancellation).

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

fn make_func_blocks(
    locals: Vec<Local>,
    blocks: Vec<(BlockId, Vec<InstructionKind>, Terminator)>,
    is_ssa: bool,
) -> Function {
    let func_id = FuncId::from(0u32);
    let mut local_map = IndexMap::new();
    for l in &locals {
        local_map.insert(l.id, l.clone());
    }

    let mut block_map = IndexMap::new();
    let entry_block = blocks[0].0;
    for (block_id, instructions, terminator) in blocks {
        block_map.insert(
            block_id,
            BasicBlock {
                id: block_id,
                instructions: instructions.into_iter().map(make_inst).collect(),
                terminator,
            },
        );
    }

    Function {
        id: func_id,
        kind: FunctionKind::Regular,
        name: "test".to_string(),
        params: vec![],
        return_type: Type::None,
        locals: local_map,
        blocks: block_map,
        entry_block,
        span: None,
        is_ssa,
        is_generic_template: false,
        typevar_params: Vec::new(),
        wrapper_fn_ptr_capture_index: None,
        phase4_return_abi_flipped: false,
        phase4_original_return_type: None,
        dom_tree_cache: std::cell::OnceCell::new(),
        signature: None,
    }
}

fn make_func(locals: Vec<Local>, instructions: Vec<InstructionKind>, is_ssa: bool) -> Function {
    let block_id = BlockId::from(0u32);
    make_func_blocks(
        locals,
        vec![(block_id, instructions, Terminator::Return(None))],
        is_ssa,
    )
}

fn make_module(func: Function) -> Module {
    let mut module = Module::new();
    module.add_function(func);
    module
}

fn get_instructions(module: &Module, block_id: BlockId) -> &[Instruction] {
    let func = module.functions.values().next().unwrap();
    &func.blocks[&block_id].instructions
}

#[test]
fn cross_block_box_int_unbox_int_collapses() {
    // BB0:
    //   _1 = BoxValue(_0, Int)
    //   br BB1
    // BB1:
    //   _2 = UnboxValue(_1, Int)  →  _2 = Copy(_0)
    let bb0 = BlockId::from(0u32);
    let bb1 = BlockId::from(1u32);
    let locals = vec![
        make_local(0, Type::Int),
        make_local(1, Type::Any),
        make_local(2, Type::Int),
    ];
    let blocks = vec![
        (
            bb0,
            vec![InstructionKind::BoxValue {
                dest: LocalId::from(1u32),
                src: Operand::Local(LocalId::from(0u32)),
                src_type: Type::Int,
            }],
            Terminator::Goto(bb1),
        ),
        (
            bb1,
            vec![InstructionKind::UnboxValue {
                dest: LocalId::from(2u32),
                src: Operand::Local(LocalId::from(1u32)),
                dest_type: Type::Int,
            }],
            Terminator::Return(None),
        ),
    ];

    let mut module = make_module(make_func_blocks(locals, blocks, true));
    super::raw_demotion_once(&mut module);

    let bb1_insts = get_instructions(&module, bb1);
    match &bb1_insts[0].kind {
        InstructionKind::Copy { dest, src } => {
            assert_eq!(*dest, LocalId::from(2u32));
            assert!(matches!(src, Operand::Local(id) if *id == LocalId::from(0u32)));
        }
        other => panic!("Expected Copy, got {:?}", other),
    }
}

#[test]
fn cross_block_box_float_unbox_float_collapses() {
    let bb0 = BlockId::from(0u32);
    let bb1 = BlockId::from(1u32);
    let locals = vec![
        make_local(0, Type::Float),
        make_local(1, Type::Any),
        make_local(2, Type::Float),
    ];
    let blocks = vec![
        (
            bb0,
            vec![InstructionKind::BoxValue {
                dest: LocalId::from(1u32),
                src: Operand::Local(LocalId::from(0u32)),
                src_type: Type::Float,
            }],
            Terminator::Goto(bb1),
        ),
        (
            bb1,
            vec![InstructionKind::UnboxValue {
                dest: LocalId::from(2u32),
                src: Operand::Local(LocalId::from(1u32)),
                dest_type: Type::Float,
            }],
            Terminator::Return(None),
        ),
    ];

    let mut module = make_module(make_func_blocks(locals, blocks, true));
    super::raw_demotion_once(&mut module);

    let bb1_insts = get_instructions(&module, bb1);
    match &bb1_insts[0].kind {
        InstructionKind::Copy { dest, src } => {
            assert_eq!(*dest, LocalId::from(2u32));
            assert!(matches!(src, Operand::Local(id) if *id == LocalId::from(0u32)));
        }
        other => panic!("Expected Copy, got {:?}", other),
    }
}

#[test]
fn rt_box_float_then_rt_unbox_float_cross_block_collapses() {
    let bb0 = BlockId::from(0u32);
    let bb1 = BlockId::from(1u32);
    let locals = vec![
        make_local(0, Type::Float),
        make_local(1, Type::Any),
        make_local(2, Type::Float),
    ];
    let blocks = vec![
        (
            bb0,
            vec![InstructionKind::RuntimeCall {
                dest: LocalId::from(1u32),
                func: RuntimeFunc::Call(&RT_BOX_FLOAT),
                args: vec![Operand::Local(LocalId::from(0u32))],
            }],
            Terminator::Goto(bb1),
        ),
        (
            bb1,
            vec![InstructionKind::RuntimeCall {
                dest: LocalId::from(2u32),
                func: RuntimeFunc::Call(&RT_UNBOX_FLOAT),
                args: vec![Operand::Local(LocalId::from(1u32))],
            }],
            Terminator::Return(None),
        ),
    ];

    let mut module = make_module(make_func_blocks(locals, blocks, true));
    super::raw_demotion_once(&mut module);

    let bb1_insts = get_instructions(&module, bb1);
    match &bb1_insts[0].kind {
        InstructionKind::Copy { dest, src } => {
            assert_eq!(*dest, LocalId::from(2u32));
            assert!(matches!(src, Operand::Local(id) if *id == LocalId::from(0u32)));
        }
        other => panic!("Expected Copy, got {:?}", other),
    }
}

#[test]
fn box_with_other_uses_keeps_box_but_rewrites_unbox() {
    // The box has TWO uses: a tagged-consumer (mock CallNamed) and an
    // UnboxValue. The unbox must be rewritten regardless; the box
    // survives because the tagged consumer still needs it.
    let locals = vec![
        make_local(0, Type::Int),
        make_local(1, Type::Any),
        make_local(2, Type::None),
        make_local(3, Type::Int),
    ];
    let instructions = vec![
        InstructionKind::BoxValue {
            dest: LocalId::from(1u32),
            src: Operand::Local(LocalId::from(0u32)),
            src_type: Type::Int,
        },
        InstructionKind::CallNamed {
            dest: LocalId::from(2u32),
            name: "external_taker".to_string(),
            args: vec![Operand::Local(LocalId::from(1u32))],
        },
        InstructionKind::UnboxValue {
            dest: LocalId::from(3u32),
            src: Operand::Local(LocalId::from(1u32)),
            dest_type: Type::Int,
        },
    ];

    let mut module = make_module(make_func(locals, instructions, true));
    super::raw_demotion_once(&mut module);

    let bb0 = BlockId::from(0u32);
    let insts = get_instructions(&module, bb0);
    // BoxValue and CallNamed unchanged
    assert!(matches!(&insts[0].kind, InstructionKind::BoxValue { .. }));
    assert!(matches!(&insts[1].kind, InstructionKind::CallNamed { .. }));
    // UnboxValue rewritten to Copy
    match &insts[2].kind {
        InstructionKind::Copy { dest, src } => {
            assert_eq!(*dest, LocalId::from(3u32));
            assert!(matches!(src, Operand::Local(id) if *id == LocalId::from(0u32)));
        }
        other => panic!("Expected Copy, got {:?}", other),
    }
}

#[test]
fn type_mismatch_box_int_unbox_bool_is_not_rewritten() {
    // Malformed-MIR-like situation: BoxValue Int but UnboxValue Bool.
    // Type filter must reject — leave both alone.
    let locals = vec![
        make_local(0, Type::Int),
        make_local(1, Type::Any),
        make_local(2, Type::Bool),
    ];
    let instructions = vec![
        InstructionKind::BoxValue {
            dest: LocalId::from(1u32),
            src: Operand::Local(LocalId::from(0u32)),
            src_type: Type::Int,
        },
        InstructionKind::UnboxValue {
            dest: LocalId::from(2u32),
            src: Operand::Local(LocalId::from(1u32)),
            dest_type: Type::Bool,
        },
    ];

    let mut module = make_module(make_func(locals, instructions, true));
    super::raw_demotion_once(&mut module);

    let bb0 = BlockId::from(0u32);
    let insts = get_instructions(&module, bb0);
    assert!(matches!(&insts[0].kind, InstructionKind::BoxValue { .. }));
    assert!(matches!(&insts[1].kind, InstructionKind::UnboxValue { .. }));
}

#[test]
fn box_int_then_rt_unbox_float_is_not_rewritten() {
    // rt_unbox_float is tag-dispatching; cross-type rewrite would give
    // wrong semantics (Int-tagged input would be coerced to f64 by
    // runtime, not bit-equal).
    let locals = vec![
        make_local(0, Type::Int),
        make_local(1, Type::Any),
        make_local(2, Type::Float),
    ];
    let instructions = vec![
        InstructionKind::BoxValue {
            dest: LocalId::from(1u32),
            src: Operand::Local(LocalId::from(0u32)),
            src_type: Type::Int,
        },
        InstructionKind::RuntimeCall {
            dest: LocalId::from(2u32),
            func: RuntimeFunc::Call(&RT_UNBOX_FLOAT),
            args: vec![Operand::Local(LocalId::from(1u32))],
        },
    ];

    let mut module = make_module(make_func(locals, instructions, true));
    super::raw_demotion_once(&mut module);

    let bb0 = BlockId::from(0u32);
    let insts = get_instructions(&module, bb0);
    assert!(matches!(&insts[0].kind, InstructionKind::BoxValue { .. }));
    assert!(matches!(
        &insts[1].kind,
        InstructionKind::RuntimeCall { .. }
    ));
}

#[test]
fn non_ssa_function_is_skipped() {
    // The pass must not run on non-SSA functions: producer map relies on
    // single-def discipline.
    let locals = vec![
        make_local(0, Type::Int),
        make_local(1, Type::Any),
        make_local(2, Type::Int),
    ];
    let instructions = vec![
        InstructionKind::BoxValue {
            dest: LocalId::from(1u32),
            src: Operand::Local(LocalId::from(0u32)),
            src_type: Type::Int,
        },
        InstructionKind::UnboxValue {
            dest: LocalId::from(2u32),
            src: Operand::Local(LocalId::from(1u32)),
            dest_type: Type::Int,
        },
    ];

    let mut module = make_module(make_func(locals, instructions, false));
    let changed = super::raw_demotion_once(&mut module);
    assert!(!changed, "non-SSA function must be skipped");

    let bb0 = BlockId::from(0u32);
    let insts = get_instructions(&module, bb0);
    assert!(matches!(&insts[0].kind, InstructionKind::BoxValue { .. }));
    assert!(matches!(&insts[1].kind, InstructionKind::UnboxValue { .. }));
}

#[test]
fn unbox_with_constant_source_is_not_rewritten() {
    // UnboxValue { src: Constant(...) } has no producer to look up.
    let locals = vec![make_local(0, Type::Int)];
    let instructions = vec![InstructionKind::UnboxValue {
        dest: LocalId::from(0u32),
        src: Operand::Constant(pyaot_mir::Constant::Int(42)),
        dest_type: Type::Int,
    }];

    let mut module = make_module(make_func(locals, instructions, true));
    let changed = super::raw_demotion_once(&mut module);
    assert!(!changed);
}

#[test]
fn unbox_float_then_box_float_collapses_to_copy() {
    // _1 = UnboxValue(_0, Float)
    // _2 = BoxValue(_1, Float)  →  _2 = Copy(_0)
    //
    // Reverse direction (U→B), inherited from the legacy box_fusion pass.
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

    let mut module = make_module(make_func(locals, instructions, true));
    super::raw_demotion_once(&mut module);

    let bb0 = BlockId::from(0u32);
    let insts = get_instructions(&module, bb0);
    match &insts[1].kind {
        InstructionKind::Copy { dest, src } => {
            assert_eq!(*dest, LocalId::from(2u32));
            assert!(matches!(src, Operand::Local(id) if *id == LocalId::from(0u32)));
        }
        other => panic!("Expected Copy, got {:?}", other),
    }
}

#[test]
fn box_float_mir_then_rt_unbox_float_mixed_form_collapses() {
    // _1 = BoxValue(_0, Float)
    // _2 = rt_unbox_float(_1)  →  _2 = Copy(_0)
    //
    // Mixed MIR/runtime form, inherited from legacy box_fusion.
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

    let mut module = make_module(make_func(locals, instructions, true));
    super::raw_demotion_once(&mut module);

    let bb0 = BlockId::from(0u32);
    let insts = get_instructions(&module, bb0);
    match &insts[1].kind {
        InstructionKind::Copy { dest, src } => {
            assert_eq!(*dest, LocalId::from(2u32));
            assert!(matches!(src, Operand::Local(id) if *id == LocalId::from(0u32)));
        }
        other => panic!("Expected Copy, got {:?}", other),
    }
}

#[test]
fn rt_unbox_float_then_box_float_mir_mixed_form_collapses() {
    // _1 = rt_unbox_float(_0)
    // _2 = BoxValue(_1, Float)  →  _2 = Copy(_0)
    //
    // Reverse direction across mixed runtime/MIR forms.
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

    let mut module = make_module(make_func(locals, instructions, true));
    super::raw_demotion_once(&mut module);

    let bb0 = BlockId::from(0u32);
    let insts = get_instructions(&module, bb0);
    match &insts[1].kind {
        InstructionKind::Copy { dest, src } => {
            assert_eq!(*dest, LocalId::from(2u32));
            assert!(matches!(src, Operand::Local(id) if *id == LocalId::from(0u32)));
        }
        other => panic!("Expected Copy, got {:?}", other),
    }
}

#[test]
fn box_then_copy_then_unbox_chain_collapses() {
    // _1 = BoxValue(_0, Int)
    // _2 = Copy(_1)               <-- producer alias through Copy
    // _3 = UnboxValue(_2, Int)    →  _3 = Copy(_0)
    //
    // The intermediate Copy used to defeat both peephole (non-adjacent)
    // and box_fusion (non-adjacent). The unified pass propagates producer
    // identity through Copy chains in `collect_producers`.
    let locals = vec![
        make_local(0, Type::Int),
        make_local(1, Type::Any),
        make_local(2, Type::Any),
        make_local(3, Type::Int),
    ];
    let instructions = vec![
        InstructionKind::BoxValue {
            dest: LocalId::from(1u32),
            src: Operand::Local(LocalId::from(0u32)),
            src_type: Type::Int,
        },
        InstructionKind::Copy {
            dest: LocalId::from(2u32),
            src: Operand::Local(LocalId::from(1u32)),
        },
        InstructionKind::UnboxValue {
            dest: LocalId::from(3u32),
            src: Operand::Local(LocalId::from(2u32)),
            dest_type: Type::Int,
        },
    ];

    let mut module = make_module(make_func(locals, instructions, true));
    super::raw_demotion_once(&mut module);

    let bb0 = BlockId::from(0u32);
    let insts = get_instructions(&module, bb0);
    match &insts[2].kind {
        InstructionKind::Copy { dest, src } => {
            assert_eq!(*dest, LocalId::from(3u32));
            assert!(matches!(src, Operand::Local(id) if *id == LocalId::from(0u32)));
        }
        other => panic!("Expected Copy, got {:?}", other),
    }
}

#[test]
fn copy_rewrite_propagates_mir_ty_from_source_local() {
    // Single-block adjacent BoxValue / UnboxValue. After rewrite the dest
    // local's `ty` and `mir_ty` must mirror the raw source's metadata,
    // not the box product type — load-bearing for float autograd
    // (microgpt) per the legacy `box_fusion` Phase 3f audit.
    let mut src = make_local(0, Type::Float);
    src.mir_ty = Some(pyaot_mir::MirType::raw_f64());
    let locals = vec![
        src,
        make_local(1, Type::Any), // box dest — tagged
        {
            // Unbox dest — pretend it was allocated as f64 already; we
            // assert it ends up with mir_ty = Raw(F64) after rewrite.
            let mut l = make_local(2, Type::Any);
            l.mir_ty = Some(pyaot_mir::MirType::Tagged);
            l
        },
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

    let mut module = make_module(make_func(locals, instructions, true));
    super::raw_demotion_once(&mut module);

    let func = module.functions.values().next().unwrap();
    let dest_local = func.locals.get(&LocalId::from(2u32)).unwrap();
    assert_eq!(dest_local.ty, Type::Float);
    assert!(
        matches!(
            dest_local.mir_ty.as_ref(),
            Some(pyaot_mir::MirType::Raw(pyaot_mir::RawKind::F64))
        ),
        "dest mir_ty must inherit Raw(F64) from src, got {:?}",
        dest_local.mir_ty
    );
}

#[test]
fn multiple_unbox_uses_of_same_box_are_all_rewritten() {
    let locals = vec![
        make_local(0, Type::Int),
        make_local(1, Type::Any),
        make_local(2, Type::Int),
        make_local(3, Type::Int),
    ];
    let instructions = vec![
        InstructionKind::BoxValue {
            dest: LocalId::from(1u32),
            src: Operand::Local(LocalId::from(0u32)),
            src_type: Type::Int,
        },
        InstructionKind::UnboxValue {
            dest: LocalId::from(2u32),
            src: Operand::Local(LocalId::from(1u32)),
            dest_type: Type::Int,
        },
        InstructionKind::UnboxValue {
            dest: LocalId::from(3u32),
            src: Operand::Local(LocalId::from(1u32)),
            dest_type: Type::Int,
        },
    ];

    let mut module = make_module(make_func(locals, instructions, true));
    super::raw_demotion_once(&mut module);

    let bb0 = BlockId::from(0u32);
    let insts = get_instructions(&module, bb0);
    // Both UnboxValues become Copy(_0)
    for (i, inst) in insts.iter().enumerate().skip(1).take(2) {
        match &inst.kind {
            InstructionKind::Copy { src, .. } => {
                assert!(matches!(src, Operand::Local(id) if *id == LocalId::from(0u32)));
            }
            other => panic!("Expected Copy at index {i}, got {:?}", other),
        }
    }
}
