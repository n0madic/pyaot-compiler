//! Tests for devirtualization pass

use indexmap::IndexMap;
use pyaot_mir::{
    BasicBlock, Constant, Function, FunctionKind, Instruction, InstructionKind, Local, Module,
    Operand, Terminator, VtableEntry, VtableInfo,
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
        abi_immutable: false,
        is_var_local: false,
        mir_ty: None,
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
        kind: FunctionKind::Regular,
        name: "caller".to_string(),
        params: caller_params,
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
        abi_immutable: false,
        is_var_local: false,
        mir_ty: None,
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
        abi_immutable: false,
        is_var_local: false,
        mir_ty: None,
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
        abi_immutable: false,
        is_var_local: false,
        mir_ty: None,
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
        abi_immutable: false,
        is_var_local: false,
        mir_ty: None,
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

/// S3.3b.1: receiver typed `Type::Generic { base: user_class_id, args }`
/// resolves the same as `Type::Class { class_id }` — the args are stripped
/// to find the vtable, but the call still devirts.
#[test]
fn test_devirtualize_generic_user_class_receiver() {
    let class_id = ClassId::from(7u32);
    let method_func_id = FuncId::from(1u32);
    let slot = 0;
    let receiver_type = Type::Generic {
        base: class_id,
        args: vec![Type::Int],
    };

    let obj_local = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: receiver_type.clone(),
        abi_immutable: false,
        is_var_local: false,
        mir_ty: None,
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
        receiver_type,
    );

    super::devirtualize(&mut module);

    let func = module.functions.get(&FuncId::from(0u32)).unwrap();
    let block = func.blocks.values().next().unwrap();
    let inst = &block.instructions[0];

    match &inst.kind {
        InstructionKind::CallDirect { func, args, .. } => {
            assert_eq!(*func, method_func_id);
            // self prepended + 1 user arg.
            assert_eq!(args.len(), 2);
            assert_eq!(args[0], Operand::Local(LocalId::from(0u32)));
        }
        other => panic!(
            "Expected CallDirect after Generic-receiver devirt, got {:?}",
            other
        ),
    }
}

/// `Type::Generic` whose `base` is a builtin container id (e.g. list) does
/// NOT have a vtable in the module — devirt must leave the CallVirtual
/// untouched (no panic, no spurious resolution).
#[test]
fn test_devirtualize_generic_builtin_no_vtable() {
    use pyaot_types::builtin_classes::BUILTIN_LIST_CLASS_ID;

    let class_id = ClassId::from(99u32); // unrelated user class with a vtable
    let method_func_id = FuncId::from(1u32);
    let slot = 0;

    // Receiver is a list[int] — Type::Generic with builtin list base. No vtable
    // for BUILTIN_LIST_CLASS_ID exists in the module's vtables vec.
    let receiver_type = Type::Generic {
        base: BUILTIN_LIST_CLASS_ID,
        args: vec![Type::Int],
    };

    let obj_local = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: receiver_type,
        abi_immutable: false,
        is_var_local: false,
        mir_ty: None,
    };
    let dest_local = make_local(1, Type::Int);

    let instructions = vec![InstructionKind::CallVirtual {
        dest: LocalId::from(1u32),
        obj: Operand::Local(LocalId::from(0u32)),
        slot,
        args: vec![],
    }];

    // Stub class_type for the unrelated vtable entry — irrelevant to the test.
    let mut interner = StringInterner::new();
    let stub_class_type = make_class_type(&mut interner, class_id, "Other");
    let mut module = make_module_with_vtable(
        instructions,
        vec![obj_local, dest_local],
        vec![],
        class_id,
        method_func_id,
        slot,
        stub_class_type,
    );

    super::devirtualize(&mut module);

    let func = module.functions.get(&FuncId::from(0u32)).unwrap();
    let block = func.blocks.values().next().unwrap();
    let inst = &block.instructions[0];

    // Builtin-list receiver has no matching vtable → CallVirtual stays.
    assert!(matches!(&inst.kind, InstructionKind::CallVirtual { .. }));
}
