//! Tests for function inlining

use pyaot_mir::{
    BinOp, Constant, Function, Instruction, InstructionKind, Local, Module, Operand, Terminator,
};
use pyaot_types::Type;
use pyaot_utils::{FuncId, LocalId};

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

    let mut func = Function::new(func_id, "caller".to_string(), vec![], Type::Int);

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
        },
        Instruction {
            kind: InstructionKind::Const {
                dest: y.id,
                value: Constant::Int(20),
            },
        },
        Instruction {
            kind: InstructionKind::CallDirect {
                dest: result.id,
                func: add_func_id,
                args: vec![Operand::Local(x.id), Operand::Local(y.id)],
            },
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

    let mut func = Function::new(fac_id, "fac".to_string(), vec![param_n.clone()], Type::Int);

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

    let mut fac_func = Function::new(fac_id, "fac".to_string(), vec![param_n.clone()], Type::Int);
    fac_func.locals.insert(result.id, result.clone());

    let entry = fac_func.blocks.get_mut(&fac_func.entry_block).unwrap();
    entry.instructions = vec![Instruction {
        kind: InstructionKind::CallDirect {
            dest: result.id,
            func: fac_id,
            args: vec![Operand::Local(param_n.id)],
        },
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

    let mut caller_func = Function::new(caller_id, "caller".to_string(), vec![], Type::Int);
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
        },
        Instruction {
            kind: InstructionKind::CallDirect {
                dest: res.id,
                func: fac_id,
                args: vec![Operand::Local(x.id)],
            },
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
    );
    gen_func.locals.insert(result.id, result.clone());

    let entry = gen_func.blocks.get_mut(&gen_func.entry_block).unwrap();
    entry.instructions = vec![Instruction {
        kind: InstructionKind::Copy {
            dest: result.id,
            src: Operand::Local(param.id),
        },
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

    let mut caller_func = Function::new(caller_id, "caller".to_string(), vec![], Type::Int);
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
        },
        Instruction {
            kind: InstructionKind::CallDirect {
                dest: res.id,
                func: gen_id,
                args: vec![Operand::Local(x.id)],
            },
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
