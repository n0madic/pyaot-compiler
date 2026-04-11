//! Tests for function inlining

use pyaot_mir::{
    BasicBlock, BinOp, Constant, Function, Instruction, InstructionKind, Local, Module, Operand,
    Terminator,
};
use pyaot_types::Type;
use pyaot_utils::{BlockId, FuncId, LocalId};

use super::analysis::{CallGraph, FunctionCost, InlineDecision};
use super::{inline_functions, InlineConfig};

/// Create a simple add function: def add(a, b): return a + b
fn create_add_function(func_id: FuncId) -> Function {
    let param_a = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let param_b = Local {
        id: LocalId::from(1u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let result = Local {
        id: LocalId::from(2u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };

    let mut func = Function::new(
        func_id,
        "add".to_string(),
        vec![param_a.clone(), param_b.clone()],
        Type::Int,
        None,
    );

    func.locals.insert(result.id, result.clone());

    // Entry block: result = a + b; return result
    let entry = func.blocks.get_mut(&func.entry_block).unwrap();
    entry.instructions = vec![Instruction {
        kind: InstructionKind::BinOp {
            dest: result.id,
            op: BinOp::Add,
            left: Operand::Local(param_a.id),
            right: Operand::Local(param_b.id),
        },
        span: None,
    }];
    entry.terminator = Terminator::Return(Some(Operand::Local(result.id)));

    func
}

/// Create a caller function that calls add
fn create_caller_function(func_id: FuncId, add_func_id: FuncId) -> Function {
    let x = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let y = Local {
        id: LocalId::from(1u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let result = Local {
        id: LocalId::from(2u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };

    let mut func = Function::new(func_id, "caller".to_string(), vec![], Type::Int, None);

    func.locals.insert(x.id, x.clone());
    func.locals.insert(y.id, y.clone());
    func.locals.insert(result.id, result.clone());

    // Entry block: x = 10; y = 20; result = add(x, y); return result
    let entry = func.blocks.get_mut(&func.entry_block).unwrap();
    entry.instructions = vec![
        Instruction {
            kind: InstructionKind::Const {
                dest: x.id,
                value: Constant::Int(10),
            },
            span: None,
        },
        Instruction {
            kind: InstructionKind::Const {
                dest: y.id,
                value: Constant::Int(20),
            },
            span: None,
        },
        Instruction {
            kind: InstructionKind::CallDirect {
                dest: result.id,
                func: add_func_id,
                args: vec![Operand::Local(x.id), Operand::Local(y.id)],
            },
            span: None,
        },
    ];
    entry.terminator = Terminator::Return(Some(Operand::Local(result.id)));

    func
}

#[test]
fn test_call_graph_build() {
    let add_id = FuncId::from(0u32);
    let caller_id = FuncId::from(1u32);

    let mut module = Module::new();
    module.add_function(create_add_function(add_id));
    module.add_function(create_caller_function(caller_id, add_id));

    let call_graph = CallGraph::build(&module);

    // caller calls add
    assert!(call_graph.callees[&caller_id].contains(&add_id));
    // add is called by caller
    assert!(call_graph.callers[&add_id].contains(&caller_id));
    // add doesn't call anything
    assert!(call_graph.callees[&add_id].is_empty());
}

#[test]
fn test_function_cost_simple() {
    let add_id = FuncId::from(0u32);

    let mut module = Module::new();
    module.add_function(create_add_function(add_id));

    let call_graph = CallGraph::build(&module);
    let cost = FunctionCost::compute(&module.functions[&add_id], &call_graph);

    assert_eq!(cost.instruction_count, 1); // Just the BinOp
    assert_eq!(cost.block_count, 1);
    assert!(!cost.has_gc_roots);
    assert!(!cost.has_exception_handling);
    assert!(!cost.is_recursive);
    assert!(!cost.is_generator);
}

#[test]
fn test_inline_decision_small_function() {
    let add_id = FuncId::from(0u32);

    let mut module = Module::new();
    module.add_function(create_add_function(add_id));

    let call_graph = CallGraph::build(&module);
    let cost = FunctionCost::compute(&module.functions[&add_id], &call_graph);
    let config = InlineConfig::default();

    let decision = cost.should_inline(&config);
    assert_eq!(decision, InlineDecision::Always);
}

#[test]
fn test_recursive_detection() {
    // Create a recursive function: def fac(n): return n * fac(n-1) if n > 1 else 1
    let fac_id = FuncId::from(0u32);

    let param_n = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };

    let mut func = Function::new(
        fac_id,
        "fac".to_string(),
        vec![param_n.clone()],
        Type::Int,
        None,
    );

    // Simplified: just has a CallDirect to itself
    let result = Local {
        id: LocalId::from(1u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    func.locals.insert(result.id, result.clone());

    let entry = func.blocks.get_mut(&func.entry_block).unwrap();
    entry.instructions = vec![Instruction {
        kind: InstructionKind::CallDirect {
            dest: result.id,
            func: fac_id, // Recursive call
            args: vec![Operand::Local(param_n.id)],
        },
        span: None,
    }];
    entry.terminator = Terminator::Return(Some(Operand::Local(result.id)));

    let mut module = Module::new();
    module.add_function(func);

    let call_graph = CallGraph::build(&module);

    assert!(call_graph.is_recursive(fac_id));
}

#[test]
fn test_simple_inline() {
    let add_id = FuncId::from(0u32);
    let caller_id = FuncId::from(1u32);

    let mut module = Module::new();
    module.add_function(create_add_function(add_id));
    module.add_function(create_caller_function(caller_id, add_id));

    // Before inlining: caller has 1 block with CallDirect
    assert_eq!(module.functions[&caller_id].blocks.len(), 1);
    let has_call_before = module.functions[&caller_id].blocks.values().any(|b| {
        b.instructions
            .iter()
            .any(|i| matches!(i.kind, InstructionKind::CallDirect { .. }))
    });
    assert!(has_call_before);

    // Perform inlining
    inline_functions(&mut module, 50);

    // After inlining: caller should have more blocks and no CallDirect to add
    let caller = &module.functions[&caller_id];
    assert!(caller.blocks.len() > 1);

    let has_call_to_add = caller.blocks.values().any(|b| {
        b.instructions
            .iter()
            .any(|i| matches!(i.kind, InstructionKind::CallDirect { func, .. } if func == add_id))
    });
    assert!(!has_call_to_add, "CallDirect to add should be inlined");

    // Should have a BinOp from the inlined add function
    let has_binop = caller.blocks.values().any(|b| {
        b.instructions
            .iter()
            .any(|i| matches!(i.kind, InstructionKind::BinOp { .. }))
    });
    assert!(has_binop, "Inlined code should contain BinOp");
}

#[test]
fn test_no_inline_recursive() {
    let fac_id = FuncId::from(0u32);
    let caller_id = FuncId::from(1u32);

    // Create recursive fac function
    let param_n = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let result = Local {
        id: LocalId::from(1u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };

    let mut fac_func = Function::new(
        fac_id,
        "fac".to_string(),
        vec![param_n.clone()],
        Type::Int,
        None,
    );
    fac_func.locals.insert(result.id, result.clone());

    let entry = fac_func.blocks.get_mut(&fac_func.entry_block).unwrap();
    entry.instructions = vec![Instruction {
        kind: InstructionKind::CallDirect {
            dest: result.id,
            func: fac_id,
            args: vec![Operand::Local(param_n.id)],
        },
        span: None,
    }];
    entry.terminator = Terminator::Return(Some(Operand::Local(result.id)));

    // Create caller that calls fac
    let x = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let res = Local {
        id: LocalId::from(1u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };

    let mut caller_func = Function::new(caller_id, "caller".to_string(), vec![], Type::Int, None);
    caller_func.locals.insert(x.id, x.clone());
    caller_func.locals.insert(res.id, res.clone());

    let entry = caller_func
        .blocks
        .get_mut(&caller_func.entry_block)
        .unwrap();
    entry.instructions = vec![
        Instruction {
            kind: InstructionKind::Const {
                dest: x.id,
                value: Constant::Int(5),
            },
            span: None,
        },
        Instruction {
            kind: InstructionKind::CallDirect {
                dest: res.id,
                func: fac_id,
                args: vec![Operand::Local(x.id)],
            },
            span: None,
        },
    ];
    entry.terminator = Terminator::Return(Some(Operand::Local(res.id)));

    let mut module = Module::new();
    module.add_function(fac_func);
    module.add_function(caller_func);

    // Perform inlining
    inline_functions(&mut module, 50);

    // Recursive function should NOT be inlined
    let caller = &module.functions[&caller_id];
    let has_call_to_fac = caller.blocks.values().any(|b| {
        b.instructions
            .iter()
            .any(|i| matches!(i.kind, InstructionKind::CallDirect { func, .. } if func == fac_id))
    });
    assert!(
        has_call_to_fac,
        "CallDirect to recursive fac should NOT be inlined"
    );
}

#[test]
fn test_generator_not_inlined() {
    let gen_id = FuncId::from(0u32);
    let caller_id = FuncId::from(1u32);

    // Create generator function (name ends with $resume)
    let param = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let result = Local {
        id: LocalId::from(1u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };

    let mut gen_func = Function::new(
        gen_id,
        "gen$resume".to_string(), // Generator suffix
        vec![param.clone()],
        Type::Int,
        None,
    );
    gen_func.locals.insert(result.id, result.clone());

    let entry = gen_func.blocks.get_mut(&gen_func.entry_block).unwrap();
    entry.instructions = vec![Instruction {
        kind: InstructionKind::Copy {
            dest: result.id,
            src: Operand::Local(param.id),
        },
        span: None,
    }];
    entry.terminator = Terminator::Return(Some(Operand::Local(result.id)));

    // Create caller
    let x = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let res = Local {
        id: LocalId::from(1u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };

    let mut caller_func = Function::new(caller_id, "caller".to_string(), vec![], Type::Int, None);
    caller_func.locals.insert(x.id, x.clone());
    caller_func.locals.insert(res.id, res.clone());

    let entry = caller_func
        .blocks
        .get_mut(&caller_func.entry_block)
        .unwrap();
    entry.instructions = vec![
        Instruction {
            kind: InstructionKind::Const {
                dest: x.id,
                value: Constant::Int(5),
            },
            span: None,
        },
        Instruction {
            kind: InstructionKind::CallDirect {
                dest: res.id,
                func: gen_id,
                args: vec![Operand::Local(x.id)],
            },
            span: None,
        },
    ];
    entry.terminator = Terminator::Return(Some(Operand::Local(res.id)));

    let mut module = Module::new();
    module.add_function(gen_func);
    module.add_function(caller_func);

    // Perform inlining
    inline_functions(&mut module, 50);

    // Generator should NOT be inlined
    let caller = &module.functions[&caller_id];
    let has_call_to_gen = caller.blocks.values().any(|b| {
        b.instructions
            .iter()
            .any(|i| matches!(i.kind, InstructionKind::CallDirect { func, .. } if func == gen_id))
    });
    assert!(
        has_call_to_gen,
        "CallDirect to generator should NOT be inlined"
    );
}

#[test]
fn test_inline_config_with_threshold() {
    let config = InlineConfig::with_threshold(100);
    assert_eq!(config.max_inline_size, 100);
    assert_eq!(config.always_inline_threshold, 10); // capped at 10
    assert_eq!(config.max_iterations, 3);

    let config = InlineConfig::with_threshold(5);
    assert_eq!(config.max_inline_size, 5);
    assert_eq!(config.always_inline_threshold, 5); // min(5, 10) = 5
}

#[test]
fn test_multi_block_callee_inlined() {
    // Create a callee with 2 blocks: if a > 0 then return a else return 0
    let callee_id = FuncId::from(0u32);
    let caller_id = FuncId::from(1u32);

    let param_a = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let result = Local {
        id: LocalId::from(1u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };

    let mut callee = Function::new(
        callee_id,
        "clamp_positive".to_string(),
        vec![param_a.clone()],
        Type::Int,
        None,
    );
    callee.locals.insert(result.id, result.clone());

    let then_block_id = BlockId::from(1u32);
    let else_block_id = BlockId::from(2u32);

    // Entry: branch on param_a
    let entry = callee.blocks.get_mut(&callee.entry_block).unwrap();
    entry.terminator = Terminator::Branch {
        cond: Operand::Local(param_a.id),
        then_block: then_block_id,
        else_block: else_block_id,
    };

    // Then block: return a
    callee.blocks.insert(
        then_block_id,
        BasicBlock {
            id: then_block_id,
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Local(param_a.id))),
        },
    );

    // Else block: result = 0; return result
    callee.blocks.insert(
        else_block_id,
        BasicBlock {
            id: else_block_id,
            instructions: vec![Instruction {
                kind: InstructionKind::Const {
                    dest: result.id,
                    value: Constant::Int(0),
                },
                span: None,
            }],
            terminator: Terminator::Return(Some(Operand::Local(result.id))),
        },
    );

    // Create caller that calls callee
    let x = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let res = Local {
        id: LocalId::from(1u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };

    let mut caller_func = Function::new(caller_id, "caller".to_string(), vec![], Type::Int, None);
    caller_func.locals.insert(x.id, x.clone());
    caller_func.locals.insert(res.id, res.clone());

    let entry = caller_func
        .blocks
        .get_mut(&caller_func.entry_block)
        .unwrap();
    entry.instructions = vec![
        Instruction {
            kind: InstructionKind::Const {
                dest: x.id,
                value: Constant::Int(42),
            },
            span: None,
        },
        Instruction {
            kind: InstructionKind::CallDirect {
                dest: res.id,
                func: callee_id,
                args: vec![Operand::Local(x.id)],
            },
            span: None,
        },
    ];
    entry.terminator = Terminator::Return(Some(Operand::Local(res.id)));

    let mut module = Module::new();
    module.add_function(callee);
    module.add_function(caller_func);

    inline_functions(&mut module, 50);

    let caller = &module.functions[&caller_id];
    // Should have inlined: original block + continuation + 3 callee blocks (entry, then, else)
    assert!(
        caller.blocks.len() >= 4,
        "Multi-block callee should produce multiple blocks: got {}",
        caller.blocks.len()
    );
    // No more calls to callee
    let has_call = caller.blocks.values().any(|b| {
        b.instructions.iter().any(
            |i| matches!(i.kind, InstructionKind::CallDirect { func, .. } if func == callee_id),
        )
    });
    assert!(!has_call, "Multi-block callee should be inlined");
}

#[test]
fn test_multiple_call_sites_in_same_function() {
    let add_id = FuncId::from(0u32);
    let caller_id = FuncId::from(1u32);

    let mut module = Module::new();
    module.add_function(create_add_function(add_id));

    // Create caller with two calls to add
    let x = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let y = Local {
        id: LocalId::from(1u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let r1 = Local {
        id: LocalId::from(2u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let r2 = Local {
        id: LocalId::from(3u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };

    let mut caller = Function::new(caller_id, "caller".to_string(), vec![], Type::Int, None);
    caller.locals.insert(x.id, x.clone());
    caller.locals.insert(y.id, y.clone());
    caller.locals.insert(r1.id, r1.clone());
    caller.locals.insert(r2.id, r2.clone());

    let entry = caller.blocks.get_mut(&caller.entry_block).unwrap();
    entry.instructions = vec![
        Instruction {
            kind: InstructionKind::Const {
                dest: x.id,
                value: Constant::Int(1),
            },
            span: None,
        },
        Instruction {
            kind: InstructionKind::Const {
                dest: y.id,
                value: Constant::Int(2),
            },
            span: None,
        },
        Instruction {
            kind: InstructionKind::CallDirect {
                dest: r1.id,
                func: add_id,
                args: vec![Operand::Local(x.id), Operand::Local(y.id)],
            },
            span: None,
        },
        Instruction {
            kind: InstructionKind::CallDirect {
                dest: r2.id,
                func: add_id,
                args: vec![Operand::Local(r1.id), Operand::Local(y.id)],
            },
            span: None,
        },
    ];
    entry.terminator = Terminator::Return(Some(Operand::Local(r2.id)));

    module.add_function(caller);

    inline_functions(&mut module, 50);

    // Both calls should be inlined
    let caller = &module.functions[&caller_id];
    let call_count: usize = caller
        .blocks
        .values()
        .flat_map(|b| b.instructions.iter())
        .filter(|i| matches!(i.kind, InstructionKind::CallDirect { func, .. } if func == add_id))
        .count();
    assert_eq!(call_count, 0, "Both call sites should be inlined");

    // Should have two BinOps from inlined add
    let binop_count: usize = caller
        .blocks
        .values()
        .flat_map(|b| b.instructions.iter())
        .filter(|i| matches!(i.kind, InstructionKind::BinOp { .. }))
        .count();
    assert_eq!(binop_count, 2, "Should have two inlined BinOps");
}

#[test]
fn test_transitive_inlining() {
    // C calls B, B calls A. Both A and B small enough to inline.
    // After iteration 1: A inlined into B. After iteration 2: B (now containing A) inlined into C.
    let a_id = FuncId::from(0u32);
    let b_id = FuncId::from(1u32);
    let c_id = FuncId::from(2u32);

    // Function A: return param + 1
    let a_param = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let a_result = Local {
        id: LocalId::from(1u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let mut func_a = Function::new(
        a_id,
        "a".to_string(),
        vec![a_param.clone()],
        Type::Int,
        None,
    );
    func_a.locals.insert(a_result.id, a_result.clone());
    let entry = func_a.blocks.get_mut(&func_a.entry_block).unwrap();
    entry.instructions = vec![Instruction {
        kind: InstructionKind::BinOp {
            dest: a_result.id,
            op: BinOp::Add,
            left: Operand::Local(a_param.id),
            right: Operand::Constant(Constant::Int(1)),
        },
        span: None,
    }];
    entry.terminator = Terminator::Return(Some(Operand::Local(a_result.id)));

    // Function B: return A(param)
    let b_param = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let b_result = Local {
        id: LocalId::from(1u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let mut func_b = Function::new(
        b_id,
        "b".to_string(),
        vec![b_param.clone()],
        Type::Int,
        None,
    );
    func_b.locals.insert(b_result.id, b_result.clone());
    let entry = func_b.blocks.get_mut(&func_b.entry_block).unwrap();
    entry.instructions = vec![Instruction {
        kind: InstructionKind::CallDirect {
            dest: b_result.id,
            func: a_id,
            args: vec![Operand::Local(b_param.id)],
        },
        span: None,
    }];
    entry.terminator = Terminator::Return(Some(Operand::Local(b_result.id)));

    // Function C: x = 10; return B(x)
    let c_x = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let c_result = Local {
        id: LocalId::from(1u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let mut func_c = Function::new(c_id, "c".to_string(), vec![], Type::Int, None);
    func_c.locals.insert(c_x.id, c_x.clone());
    func_c.locals.insert(c_result.id, c_result.clone());
    let entry = func_c.blocks.get_mut(&func_c.entry_block).unwrap();
    entry.instructions = vec![
        Instruction {
            kind: InstructionKind::Const {
                dest: c_x.id,
                value: Constant::Int(10),
            },
            span: None,
        },
        Instruction {
            kind: InstructionKind::CallDirect {
                dest: c_result.id,
                func: b_id,
                args: vec![Operand::Local(c_x.id)],
            },
            span: None,
        },
    ];
    entry.terminator = Terminator::Return(Some(Operand::Local(c_result.id)));

    let mut module = Module::new();
    module.add_function(func_a);
    module.add_function(func_b);
    module.add_function(func_c);

    inline_functions(&mut module, 50);

    // C should have no calls at all — both B and A transitively inlined
    let func_c = &module.functions[&c_id];
    let has_any_call = func_c.blocks.values().any(|b| {
        b.instructions
            .iter()
            .any(|i| matches!(i.kind, InstructionKind::CallDirect { .. }))
    });
    assert!(
        !has_any_call,
        "Transitive inlining should eliminate all calls in C"
    );

    // Should have the BinOp from A
    let has_binop = func_c.blocks.values().any(|b| {
        b.instructions
            .iter()
            .any(|i| matches!(i.kind, InstructionKind::BinOp { .. }))
    });
    assert!(
        has_binop,
        "C should contain A's BinOp after transitive inlining"
    );
}

#[test]
fn test_consider_decision_medium_function() {
    // Create a function that is medium-sized: above always_inline_threshold but below max_inline_size
    let func_id = FuncId::from(0u32);
    let param = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };

    let mut func = Function::new(
        func_id,
        "medium".to_string(),
        vec![param.clone()],
        Type::Int,
        None,
    );

    // Add 15 instructions (above always_inline_threshold=10, below max_inline_size=50)
    let entry = func.blocks.get_mut(&func.entry_block).unwrap();
    let mut instructions = Vec::new();
    for i in 0..15u32 {
        let local = Local {
            id: LocalId::from(i + 1),
            name: None,
            ty: Type::Int,
            is_gc_root: false,
        };
        func.locals.insert(local.id, local.clone());
        instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: local.id,
                value: Constant::Int(i as i64),
            },
            span: None,
        });
    }
    entry.instructions = instructions;
    entry.terminator = Terminator::Return(Some(Operand::Local(LocalId::from(1u32))));

    let mut module = Module::new();
    module.add_function(func);

    let call_graph = CallGraph::build(&module);
    let cost = FunctionCost::compute(&module.functions[&func_id], &call_graph);
    let config = InlineConfig::default();

    assert_eq!(cost.instruction_count, 15);
    assert_eq!(cost.block_count, 1);
    let decision = cost.should_inline(&config);
    assert_eq!(
        decision,
        InlineDecision::Consider,
        "Medium function should get Consider decision"
    );
}

#[test]
fn test_gc_roots_excluded_from_always_inline() {
    // Create a small function with GC roots — should get Consider, not Always
    let func_id = FuncId::from(0u32);
    let param = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Str,
        is_gc_root: true,
    };
    let result = Local {
        id: LocalId::from(1u32),
        name: None,
        ty: Type::Str,
        is_gc_root: false,
    };

    let mut func = Function::new(
        func_id,
        "gc_func".to_string(),
        vec![param.clone()],
        Type::Str,
        None,
    );
    func.locals.insert(result.id, result.clone());

    let entry = func.blocks.get_mut(&func.entry_block).unwrap();
    entry.instructions = vec![Instruction {
        kind: InstructionKind::Copy {
            dest: result.id,
            src: Operand::Local(param.id),
        },
        span: None,
    }];
    entry.terminator = Terminator::Return(Some(Operand::Local(result.id)));

    let mut module = Module::new();
    module.add_function(func);

    let call_graph = CallGraph::build(&module);
    let cost = FunctionCost::compute(&module.functions[&func_id], &call_graph);
    let config = InlineConfig::default();

    assert!(cost.has_gc_roots);
    let decision = cost.should_inline(&config);
    // Small function with GC roots: should be Consider (not Always, not Never)
    assert_eq!(
        decision,
        InlineDecision::Consider,
        "Small function with GC roots should get Consider, not Always"
    );
}

#[test]
fn test_max_iterations_limits_inlining() {
    // With max_iterations=1, transitive inlining should be limited
    let a_id = FuncId::from(0u32);
    let b_id = FuncId::from(1u32);
    let c_id = FuncId::from(2u32);

    // Function A: leaf
    let a_param = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let a_result = Local {
        id: LocalId::from(1u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let mut func_a = Function::new(
        a_id,
        "a".to_string(),
        vec![a_param.clone()],
        Type::Int,
        None,
    );
    func_a.locals.insert(a_result.id, a_result.clone());
    let entry = func_a.blocks.get_mut(&func_a.entry_block).unwrap();
    entry.instructions = vec![Instruction {
        kind: InstructionKind::Copy {
            dest: a_result.id,
            src: Operand::Local(a_param.id),
        },
        span: None,
    }];
    entry.terminator = Terminator::Return(Some(Operand::Local(a_result.id)));

    // Function B: calls A
    let b_param = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let b_result = Local {
        id: LocalId::from(1u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let mut func_b = Function::new(
        b_id,
        "b".to_string(),
        vec![b_param.clone()],
        Type::Int,
        None,
    );
    func_b.locals.insert(b_result.id, b_result.clone());
    let entry = func_b.blocks.get_mut(&func_b.entry_block).unwrap();
    entry.instructions = vec![Instruction {
        kind: InstructionKind::CallDirect {
            dest: b_result.id,
            func: a_id,
            args: vec![Operand::Local(b_param.id)],
        },
        span: None,
    }];
    entry.terminator = Terminator::Return(Some(Operand::Local(b_result.id)));

    // Function C: calls B
    let c_x = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let c_result = Local {
        id: LocalId::from(1u32),
        name: None,
        ty: Type::Int,
        is_gc_root: false,
    };
    let mut func_c = Function::new(c_id, "c".to_string(), vec![], Type::Int, None);
    func_c.locals.insert(c_x.id, c_x.clone());
    func_c.locals.insert(c_result.id, c_result.clone());
    let entry = func_c.blocks.get_mut(&func_c.entry_block).unwrap();
    entry.instructions = vec![
        Instruction {
            kind: InstructionKind::Const {
                dest: c_x.id,
                value: Constant::Int(10),
            },
            span: None,
        },
        Instruction {
            kind: InstructionKind::CallDirect {
                dest: c_result.id,
                func: b_id,
                args: vec![Operand::Local(c_x.id)],
            },
            span: None,
        },
    ];
    entry.terminator = Terminator::Return(Some(Operand::Local(c_result.id)));

    let mut module = Module::new();
    module.add_function(func_a);
    module.add_function(func_b);
    module.add_function(func_c);

    // Use max_iterations=1 — only one pass
    let config = InlineConfig {
        max_inline_size: 50,
        always_inline_threshold: 10,
        max_iterations: 1,
    };
    super::transform::inline_pass(&mut module, &config);

    // After 1 iteration: A inlined into B, B inlined into C.
    // But B's inlined-into-C version still has CallDirect to A (from before A was inlined into B).
    // So C may still have a call to A.
    let func_c = &module.functions[&c_id];
    let call_to_b: usize = func_c
        .blocks
        .values()
        .flat_map(|b| b.instructions.iter())
        .filter(|i| matches!(i.kind, InstructionKind::CallDirect { func, .. } if func == b_id))
        .count();
    // B should be inlined into C in the first iteration
    assert_eq!(call_to_b, 0, "B should be inlined into C in iteration 1");
}
