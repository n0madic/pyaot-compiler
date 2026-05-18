//! Tests for dead code elimination

use pyaot_mir::{
    BasicBlock, BinOp, Constant, Function, FunctionKind, Instruction, InstructionKind, Local,
    Module, Operand, RuntimeFunc, Terminator,
};
use pyaot_types::Type;
use pyaot_utils::{BlockId, FuncId, LocalId};

use super::eliminate_dead_code;
use super::liveness::{eliminate_dead_instructions, eliminate_dead_locals};
use super::reachability::eliminate_unreachable_blocks;

fn local(id: u32) -> Local {
    Local {
        id: LocalId::from(id),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
        abi_immutable: false,
        mir_ty: None,
    }
}

fn lid(id: u32) -> LocalId {
    LocalId::from(id)
}

fn bid(id: u32) -> BlockId {
    BlockId::from(id)
}

fn const_int(dest: u32, value: i64) -> Instruction {
    Instruction {
        kind: InstructionKind::Const {
            dest: lid(dest),
            value: Constant::Int(value),
        },
        span: None,
    }
}

fn binop_add(dest: u32, left: u32, right: u32) -> Instruction {
    Instruction {
        kind: InstructionKind::BinOp {
            dest: lid(dest),
            op: BinOp::Add,
            left: Operand::Local(lid(left)),
            right: Operand::Local(lid(right)),
        },
        span: None,
    }
}

fn copy_instr(dest: u32, src: u32) -> Instruction {
    Instruction {
        kind: InstructionKind::Copy {
            dest: lid(dest),
            src: Operand::Local(lid(src)),
        },
        span: None,
    }
}

// ==================== Unreachable Block Elimination ====================

#[test]
fn test_unreachable_block_removed() {
    let mut func = Function::new(
        FuncId::from(0u32),
        "f".to_string(),
        vec![],
        Type::None,
        None,
    );

    // Entry (block 0) → goto block 1. Block 2 is orphaned.
    let entry = func.blocks.get_mut(&bid(0)).unwrap();
    entry.instructions = vec![const_int(0, 42)];
    entry.terminator = Terminator::Goto(bid(1));

    func.blocks.insert(
        bid(1),
        BasicBlock {
            id: bid(1),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Local(lid(0)))),
        },
    );

    func.blocks.insert(
        bid(2),
        BasicBlock {
            id: bid(2),
            instructions: vec![const_int(1, 99)],
            terminator: Terminator::Unreachable,
        },
    );

    assert_eq!(func.blocks.len(), 3);
    assert!(eliminate_unreachable_blocks(&mut func));
    assert_eq!(func.blocks.len(), 2);
    assert!(func.blocks.contains_key(&bid(0)));
    assert!(func.blocks.contains_key(&bid(1)));
    assert!(!func.blocks.contains_key(&bid(2)));
}

#[test]
fn test_all_blocks_reachable_no_change() {
    let mut func = Function::new(
        FuncId::from(0u32),
        "f".to_string(),
        vec![],
        Type::None,
        None,
    );

    let entry = func.blocks.get_mut(&bid(0)).unwrap();
    entry.terminator = Terminator::Branch {
        cond: Operand::Local(lid(0)),
        then_block: bid(1),
        else_block: bid(2),
    };

    func.blocks.insert(
        bid(1),
        BasicBlock {
            id: bid(1),
            instructions: vec![],
            terminator: Terminator::Return(None),
        },
    );

    func.blocks.insert(
        bid(2),
        BasicBlock {
            id: bid(2),
            instructions: vec![],
            terminator: Terminator::Return(None),
        },
    );

    assert!(!eliminate_unreachable_blocks(&mut func));
    assert_eq!(func.blocks.len(), 3);
}

#[test]
fn test_try_setjmp_both_successors_reachable() {
    let mut func = Function::new(
        FuncId::from(0u32),
        "f".to_string(),
        vec![],
        Type::None,
        None,
    );

    let entry = func.blocks.get_mut(&bid(0)).unwrap();
    entry.terminator = Terminator::TrySetjmp {
        frame_local: lid(0),
        try_body: bid(1),
        handler_entry: bid(2),
    };

    func.blocks.insert(
        bid(1),
        BasicBlock {
            id: bid(1),
            instructions: vec![],
            terminator: Terminator::Return(None),
        },
    );

    func.blocks.insert(
        bid(2),
        BasicBlock {
            id: bid(2),
            instructions: vec![],
            terminator: Terminator::Return(None),
        },
    );

    assert!(!eliminate_unreachable_blocks(&mut func));
    assert_eq!(func.blocks.len(), 3);
}

// ==================== Dead Instruction Elimination ====================

#[test]
fn test_dead_const_removed() {
    let mut func = Function::new(
        FuncId::from(0u32),
        "f".to_string(),
        vec![],
        Type::None,
        None,
    );
    func.locals.insert(lid(0), local(0));

    let entry = func.blocks.get_mut(&bid(0)).unwrap();
    // x = 42 but never used; return None
    entry.instructions = vec![const_int(0, 42)];
    entry.terminator = Terminator::Return(None);

    assert!(eliminate_dead_instructions(&mut func));
    assert!(func.blocks[&bid(0)].instructions.is_empty());
}

#[test]
fn test_binop_kept_because_may_raise() {
    // BinOp can raise OverflowError/ZeroDivisionError, so it's not pure
    let mut func = Function::new(
        FuncId::from(0u32),
        "f".to_string(),
        vec![],
        Type::None,
        None,
    );
    func.locals.insert(lid(0), local(0));
    func.locals.insert(lid(1), local(1));
    func.locals.insert(lid(2), local(2));

    let entry = func.blocks.get_mut(&bid(0)).unwrap();
    // x = 1; y = 2; z = x + y; return x
    // z is unused, but BinOp may raise → keep it and its operands
    entry.instructions = vec![const_int(0, 1), const_int(1, 2), binop_add(2, 0, 1)];
    entry.terminator = Terminator::Return(Some(Operand::Local(lid(0))));

    assert!(!eliminate_dead_instructions(&mut func));
    assert_eq!(func.blocks[&bid(0)].instructions.len(), 3);
}

#[test]
fn test_used_instruction_kept() {
    let mut func = Function::new(
        FuncId::from(0u32),
        "f".to_string(),
        vec![],
        Type::None,
        None,
    );
    func.locals.insert(lid(0), local(0));

    let entry = func.blocks.get_mut(&bid(0)).unwrap();
    // x = 42; return x → x is used
    entry.instructions = vec![const_int(0, 42)];
    entry.terminator = Terminator::Return(Some(Operand::Local(lid(0))));

    assert!(!eliminate_dead_instructions(&mut func));
    assert_eq!(func.blocks[&bid(0)].instructions.len(), 1);
}

#[test]
fn test_side_effectful_call_kept_even_if_unused() {
    let mut func = Function::new(
        FuncId::from(0u32),
        "f".to_string(),
        vec![],
        Type::None,
        None,
    );
    func.locals.insert(lid(0), local(0));
    func.locals.insert(lid(1), local(1));

    let entry = func.blocks.get_mut(&bid(0)).unwrap();
    // x = 42; result = call_direct(some_func, x); return x
    // result is unused, but CallDirect has side effects → keep it
    entry.instructions = vec![
        const_int(0, 42),
        Instruction {
            kind: InstructionKind::CallDirect {
                dest: lid(1),
                func: FuncId::from(1u32),
                args: vec![Operand::Local(lid(0))],
            },
            span: None,
        },
    ];
    entry.terminator = Terminator::Return(Some(Operand::Local(lid(0))));

    assert!(!eliminate_dead_instructions(&mut func));
    assert_eq!(func.blocks[&bid(0)].instructions.len(), 2);
}

#[test]
fn test_gc_instructions_preserved() {
    let mut func = Function::new(
        FuncId::from(0u32),
        "f".to_string(),
        vec![],
        Type::None,
        None,
    );
    func.locals.insert(lid(0), local(0));

    let entry = func.blocks.get_mut(&bid(0)).unwrap();
    entry.instructions = vec![
        Instruction {
            kind: InstructionKind::GcPush { frame: lid(0) },
            span: None,
        },
        Instruction {
            kind: InstructionKind::GcPop,
            span: None,
        },
    ];
    entry.terminator = Terminator::Return(None);

    // GcPush/GcPop have no dest and are side-effectful → never candidates for removal
    assert!(!eliminate_dead_instructions(&mut func));
    assert_eq!(func.blocks[&bid(0)].instructions.len(), 2);
}

#[test]
fn test_exception_instructions_preserved() {
    let mut func = Function::new(
        FuncId::from(0u32),
        "f".to_string(),
        vec![],
        Type::None,
        None,
    );
    func.locals.insert(lid(0), local(0));

    let entry = func.blocks.get_mut(&bid(0)).unwrap();
    // ExcHasException writes to dest but is side-effectful → keep even if dest unused
    entry.instructions = vec![Instruction {
        kind: InstructionKind::ExcHasException { dest: lid(0) },
        span: None,
    }];
    entry.terminator = Terminator::Return(None);

    assert!(!eliminate_dead_instructions(&mut func));
    assert_eq!(func.blocks[&bid(0)].instructions.len(), 1);
}

#[test]
fn test_dead_copy_removed() {
    // Simulates post-inlining pattern: Copy { dest: x, src: param } where x is never used
    let mut func = Function::new(
        FuncId::from(0u32),
        "f".to_string(),
        vec![],
        Type::None,
        None,
    );
    func.locals.insert(lid(0), local(0));
    func.locals.insert(lid(1), local(1));

    let entry = func.blocks.get_mut(&bid(0)).unwrap();
    entry.instructions = vec![const_int(0, 10), copy_instr(1, 0)];
    // Only return x (local 0), copy to local 1 is dead
    entry.terminator = Terminator::Return(Some(Operand::Local(lid(0))));

    assert!(eliminate_dead_instructions(&mut func));
    assert_eq!(func.blocks[&bid(0)].instructions.len(), 1);
    match &func.blocks[&bid(0)].instructions[0].kind {
        InstructionKind::Const { dest, .. } => assert_eq!(*dest, lid(0)),
        _ => panic!("expected Const"),
    }
}

// ==================== Dead Local Elimination ====================

#[test]
fn test_unused_locals_cleaned() {
    let mut func = Function::new(
        FuncId::from(0u32),
        "f".to_string(),
        vec![],
        Type::None,
        None,
    );
    func.locals.insert(lid(0), local(0));
    func.locals.insert(lid(1), local(1)); // unused local
    func.locals.insert(lid(2), local(2)); // unused local

    let entry = func.blocks.get_mut(&bid(0)).unwrap();
    entry.instructions = vec![const_int(0, 42)];
    entry.terminator = Terminator::Return(Some(Operand::Local(lid(0))));

    assert!(eliminate_dead_locals(&mut func));
    assert_eq!(func.locals.len(), 1);
    assert!(func.locals.contains_key(&lid(0)));
}

#[test]
fn test_parameter_locals_kept() {
    let param = local(0);
    let mut func = Function::new(
        FuncId::from(0u32),
        "f".to_string(),
        vec![param.clone()],
        Type::Int,
        None,
    );
    func.locals.insert(lid(0), param);

    let entry = func.blocks.get_mut(&bid(0)).unwrap();
    // Parameter is not used in any instruction but should be kept
    entry.terminator = Terminator::Return(None);

    assert!(!eliminate_dead_locals(&mut func));
    assert_eq!(func.locals.len(), 1);
}

// ==================== Cascading / Fixpoint ====================

#[test]
fn test_cascading_dead_code() {
    // x = 5; y = copy(x); z = copy(y); return const 0
    // z is unused → removed. Then y → removed. Then x → removed.
    let mut module = Module::new();
    let mut func = Function::new(FuncId::from(0u32), "f".to_string(), vec![], Type::Int, None);
    func.locals.insert(lid(0), local(0));
    func.locals.insert(lid(1), local(1));
    func.locals.insert(lid(2), local(2));

    let entry = func.blocks.get_mut(&bid(0)).unwrap();
    entry.instructions = vec![const_int(0, 5), copy_instr(1, 0), copy_instr(2, 1)];
    entry.terminator = Terminator::Return(Some(Operand::Constant(Constant::Int(0))));

    module.add_function(func);
    eliminate_dead_code(&mut module);

    let func = &module.functions[&FuncId::from(0u32)];
    assert!(func.blocks[&bid(0)].instructions.is_empty());
    // All dead locals should also be cleaned
    assert!(func.locals.is_empty());
}

// ==================== Integration: unreachable + dead instructions ====================

#[test]
fn test_unreachable_block_with_dead_instructions() {
    let mut module = Module::new();
    let mut func = Function::new(
        FuncId::from(0u32),
        "f".to_string(),
        vec![],
        Type::None,
        None,
    );
    func.locals.insert(lid(0), local(0));
    func.locals.insert(lid(1), local(1));

    // Entry block returns immediately
    let entry = func.blocks.get_mut(&bid(0)).unwrap();
    entry.terminator = Terminator::Return(None);

    // Unreachable block with dead instructions
    func.blocks.insert(
        bid(1),
        BasicBlock {
            id: bid(1),
            instructions: vec![const_int(0, 42), const_int(1, 99)],
            terminator: Terminator::Return(Some(Operand::Local(lid(0)))),
        },
    );

    module.add_function(func);
    eliminate_dead_code(&mut module);

    let func = &module.functions[&FuncId::from(0u32)];
    assert_eq!(func.blocks.len(), 1);
    assert!(func.blocks.contains_key(&bid(0)));
    assert!(func.locals.is_empty());
}

#[test]
fn test_cross_block_liveness() {
    // Block 0: x = 42; goto block 1
    // Block 1: return x
    // x is used in block 1, so instruction in block 0 must be kept
    let mut func = Function::new(FuncId::from(0u32), "f".to_string(), vec![], Type::Int, None);
    func.locals.insert(lid(0), local(0));

    let entry = func.blocks.get_mut(&bid(0)).unwrap();
    entry.instructions = vec![const_int(0, 42)];
    entry.terminator = Terminator::Goto(bid(1));

    func.blocks.insert(
        bid(1),
        BasicBlock {
            id: bid(1),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Local(lid(0)))),
        },
    );

    assert!(!eliminate_dead_instructions(&mut func));
    assert_eq!(func.blocks[&bid(0)].instructions.len(), 1);
}

#[test]
fn test_runtime_call_kept_even_if_unused() {
    let mut func = Function::new(
        FuncId::from(0u32),
        "f".to_string(),
        vec![],
        Type::None,
        None,
    );
    func.locals.insert(lid(0), local(0));

    let entry = func.blocks.get_mut(&bid(0)).unwrap();
    entry.instructions = vec![Instruction {
        kind: InstructionKind::RuntimeCall {
            dest: lid(0),
            func: RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_PRINT_NEWLINE),
            args: vec![],
        },
        span: None,
    }];
    entry.terminator = Terminator::Return(None);

    assert!(!eliminate_dead_instructions(&mut func));
    assert_eq!(func.blocks[&bid(0)].instructions.len(), 1);
}

#[test]
fn test_gc_alloc_kept_even_if_unused() {
    let mut func = Function::new(
        FuncId::from(0u32),
        "f".to_string(),
        vec![],
        Type::None,
        None,
    );
    func.locals.insert(lid(0), local(0));

    let entry = func.blocks.get_mut(&bid(0)).unwrap();
    entry.instructions = vec![Instruction {
        kind: InstructionKind::GcAlloc {
            dest: lid(0),
            ty: Type::Str,
            size: 32,
        },
        span: None,
    }];
    entry.terminator = Terminator::Return(None);

    assert!(!eliminate_dead_instructions(&mut func));
    assert_eq!(func.blocks[&bid(0)].instructions.len(), 1);
}

#[test]
fn test_empty_function_no_panic() {
    let mut module = Module::new();
    let func = Function::new(
        FuncId::from(0u32),
        "f".to_string(),
        vec![],
        Type::None,
        None,
    );
    module.add_function(func);

    // Should not panic on function with only entry block + Unreachable terminator
    eliminate_dead_code(&mut module);
    assert_eq!(module.functions[&FuncId::from(0u32)].blocks.len(), 1);
}

/// Regression: when an unreachable predecessor is removed by
/// `eliminate_unreachable_blocks`, any φ-node in a surviving block that
/// referenced it must drop the stale source. Without this, the SSA invariant
/// checker fires (PhiArityMismatch) on debug builds. Lock-in for the fix
/// in `crates/optimizer/src/dce/reachability.rs`.
#[test]
fn test_phi_sources_pruned_after_unreachable_block_removal() {
    use indexmap::IndexMap;

    let entry = bid(0);
    let dead = bid(1);
    let merge = bid(2);
    let l_entry = lid(10);
    let l_dead = lid(20);
    let phi_dest = lid(30);

    // entry: l_entry = 1; goto merge
    // dead:  l_dead = 2; goto merge   (unreachable from entry)
    // merge: phi_dest = φ((entry, l_entry), (dead, l_dead)); return phi_dest
    let mut blocks = IndexMap::new();
    blocks.insert(
        entry,
        BasicBlock {
            id: entry,
            instructions: vec![const_int(10, 1)],
            terminator: Terminator::Goto(merge),
        },
    );
    blocks.insert(
        dead,
        BasicBlock {
            id: dead,
            instructions: vec![const_int(20, 2)],
            terminator: Terminator::Goto(merge),
        },
    );
    blocks.insert(
        merge,
        BasicBlock {
            id: merge,
            instructions: vec![Instruction {
                kind: InstructionKind::Phi {
                    dest: phi_dest,
                    sources: vec![
                        (entry, Operand::Local(l_entry)),
                        (dead, Operand::Local(l_dead)),
                    ],
                },
                span: None,
            }],
            terminator: Terminator::Return(Some(Operand::Local(phi_dest))),
        },
    );

    let mut locals = IndexMap::new();
    locals.insert(l_entry, local(10));
    locals.insert(l_dead, local(20));
    locals.insert(phi_dest, local(30));

    let mut func = Function {
        id: FuncId::from(0u32),
        kind: FunctionKind::Regular,
        name: "f".to_string(),
        params: Vec::new(),
        return_type: Type::Int,
        locals,
        blocks,
        entry_block: entry,
        span: None,
        is_ssa: true,
        is_generic_template: false,
        typevar_params: Vec::new(),
        wrapper_fn_ptr_capture_index: None,
        phase4_return_abi_flipped: false,
        phase4_original_return_type: None,
        dom_tree_cache: std::cell::OnceCell::new(),
        signature: None,
    };

    let removed = eliminate_unreachable_blocks(&mut func);
    assert!(removed, "unreachable block should be removed");
    // dead block gone.
    assert!(!func.blocks.contains_key(&dead));
    // phi sources only reference surviving predecessors.
    let phi = &func.blocks[&merge].instructions[0];
    let InstructionKind::Phi { sources, .. } = &phi.kind else {
        panic!("expected Phi, got {:?}", phi.kind);
    };
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].0, entry);
}
