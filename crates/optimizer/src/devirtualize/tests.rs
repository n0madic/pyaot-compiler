//! Tests for devirtualization pass

use indexmap::IndexMap;
use pyaot_mir::{
    BasicBlock, Constant, Function, Instruction, InstructionKind, Local, Module, Operand,
    Terminator, VtableEntry, VtableInfo,
};
use pyaot_types::Type;
use pyaot_utils::{BlockId, ClassId, FuncId, LocalId, StringInterner};

fn make_class_type(interner: &mut StringInterner, class_id: ClassId, name: &str) -> Type {
    Type::Class {
        class_id,
        name: interner.intern(name),
    }
}

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

/// Build a module with a caller function, a callee function, and a vtable.
fn make_module_with_vtable(
    caller_instructions: Vec<InstructionKind>,
    caller_locals: Vec<Local>,
    caller_params: Vec<Local>,
    class_id: ClassId,
    method_func_id: FuncId,
    slot: usize,
    callee_self_type: Type,
) -> Module {
    let caller_id = FuncId::from(0u32);
    let block_id = BlockId::from(0u32);

    let mut local_map = IndexMap::new();
    for l in &caller_locals {
        local_map.insert(l.id, l.clone());
    }

    let mut blocks = IndexMap::new();
    blocks.insert(
        block_id,
        BasicBlock {
            id: block_id,
            instructions: caller_instructions.into_iter().map(make_inst).collect(),
            terminator: Terminator::Return(None),
        },
    );

    let caller = Function {
        id: caller_id,
        name: "caller".to_string(),
        params: caller_params,
        return_type: Type::None,
        locals: local_map,
        blocks,
        entry_block: block_id,
        span: None,
        is_ssa: false,
        dom_tree_cache: std::cell::OnceCell::new(),
    };

    // Create a stub callee function
    let callee = Function::new(
        method_func_id,
        "Foo$bar".to_string(),
        vec![make_local(100, callee_self_type)],
        Type::Int,
        None,
    );

    let mut module = Module::new();
    module.add_function(caller);
    module.add_function(callee);

    // Add vtable
    module.vtables.push(VtableInfo {
        class_id,
        entries: vec![VtableEntry {
            slot,
            name_hash: 0,
            method_func_id,
        }],
    });

    module
}

#[test]
fn test_devirtualize_known_class() {
    let mut interner = StringInterner::new();
    let class_id = ClassId::from(1u32);
    let method_func_id = FuncId::from(1u32);
    let slot = 0;
    let class_type = make_class_type(&mut interner, class_id, "Foo");

    // obj (local 0) has type Class { class_id: 1 }
    let obj_local = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: class_type.clone(),
        is_gc_root: true,
    };
    let dest_local = make_local(1, Type::Int);

    let instructions = vec![InstructionKind::CallVirtual {
        dest: LocalId::from(1u32),
        obj: Operand::Local(LocalId::from(0u32)),
        slot,
        args: vec![Operand::Constant(Constant::Int(42))],
    }];

    let mut module = make_module_with_vtable(
        instructions,
        vec![obj_local, dest_local],
        vec![],
        class_id,
        method_func_id,
        slot,
        class_type,
    );

    super::devirtualize(&mut module);

    let func = module.functions.get(&FuncId::from(0u32)).unwrap();
    let block = func.blocks.values().next().unwrap();
    let inst = &block.instructions[0];

    match &inst.kind {
        InstructionKind::CallDirect { dest, func, args } => {
            assert_eq!(*dest, LocalId::from(1u32));
            assert_eq!(*func, method_func_id);
            // args should be [obj, 42]
            assert_eq!(args.len(), 2);
            assert_eq!(args[0], Operand::Local(LocalId::from(0u32)));
            assert_eq!(args[1], Operand::Constant(Constant::Int(42)));
        }
        other => panic!("Expected CallDirect, got {:?}", other),
    }
}

#[test]
fn test_skip_unknown_type() {
    let mut interner = StringInterner::new();
    let class_id = ClassId::from(1u32);
    let method_func_id = FuncId::from(1u32);
    let slot = 0;
    let class_type = make_class_type(&mut interner, class_id, "Foo");

    // obj (local 0) has type Any — cannot devirtualize
    let obj_local = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: Type::Any,
        is_gc_root: true,
    };
    let dest_local = make_local(1, Type::Int);

    let instructions = vec![InstructionKind::CallVirtual {
        dest: LocalId::from(1u32),
        obj: Operand::Local(LocalId::from(0u32)),
        slot,
        args: vec![],
    }];

    let mut module = make_module_with_vtable(
        instructions,
        vec![obj_local, dest_local],
        vec![],
        class_id,
        method_func_id,
        slot,
        class_type,
    );

    super::devirtualize(&mut module);

    let func = module.functions.get(&FuncId::from(0u32)).unwrap();
    let block = func.blocks.values().next().unwrap();
    let inst = &block.instructions[0];

    // Should remain CallVirtual
    assert!(matches!(&inst.kind, InstructionKind::CallVirtual { .. }));
}

#[test]
fn test_skip_missing_vtable_slot() {
    let mut interner = StringInterner::new();
    let class_id = ClassId::from(1u32);
    let method_func_id = FuncId::from(1u32);
    let class_type = make_class_type(&mut interner, class_id, "Foo");

    // Vtable has slot 0, but instruction references slot 5
    let obj_local = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: class_type.clone(),
        is_gc_root: true,
    };
    let dest_local = make_local(1, Type::Int);

    let instructions = vec![InstructionKind::CallVirtual {
        dest: LocalId::from(1u32),
        obj: Operand::Local(LocalId::from(0u32)),
        slot: 5, // Not in vtable
        args: vec![],
    }];

    let mut module = make_module_with_vtable(
        instructions,
        vec![obj_local, dest_local],
        vec![],
        class_id,
        method_func_id,
        0, // vtable only has slot 0
        class_type,
    );

    super::devirtualize(&mut module);

    let func = module.functions.get(&FuncId::from(0u32)).unwrap();
    let block = func.blocks.values().next().unwrap();
    let inst = &block.instructions[0];

    // Should remain CallVirtual
    assert!(matches!(&inst.kind, InstructionKind::CallVirtual { .. }));
}

#[test]
fn test_devirtualize_obj_in_params() {
    let mut interner = StringInterner::new();
    let class_id = ClassId::from(2u32);
    let method_func_id = FuncId::from(1u32);
    let slot = 0;
    let class_type = make_class_type(&mut interner, class_id, "Bar");

    // obj is a function parameter, not a local
    let obj_param = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: class_type.clone(),
        is_gc_root: true,
    };
    let dest_local = make_local(1, Type::Int);

    let instructions = vec![InstructionKind::CallVirtual {
        dest: LocalId::from(1u32),
        obj: Operand::Local(LocalId::from(0u32)),
        slot,
        args: vec![],
    }];

    let mut module = make_module_with_vtable(
        instructions,
        vec![dest_local], // obj NOT in locals
        vec![obj_param],  // obj IS in params
        class_id,
        method_func_id,
        slot,
        class_type,
    );

    super::devirtualize(&mut module);

    let func = module.functions.get(&FuncId::from(0u32)).unwrap();
    let block = func.blocks.values().next().unwrap();
    let inst = &block.instructions[0];

    match &inst.kind {
        InstructionKind::CallDirect { func, args, .. } => {
            assert_eq!(*func, method_func_id);
            assert_eq!(args.len(), 1); // just self, no extra args
            assert_eq!(args[0], Operand::Local(LocalId::from(0u32)));
        }
        other => panic!("Expected CallDirect, got {:?}", other),
    }
}

#[test]
fn test_no_vtables_is_noop() {
    let func = Function::new(
        FuncId::from(0u32),
        "test".to_string(),
        vec![],
        Type::None,
        None,
    );
    let mut module = Module::new();
    module.add_function(func);

    // No vtables — should return immediately
    super::devirtualize(&mut module);

    assert_eq!(module.functions.len(), 1);
}
