//! Tests for property flattening pass

use indexmap::IndexMap;
use pyaot_core_defs::runtime_func_def::RT_INSTANCE_GET_FIELD;
use pyaot_mir::{
    BasicBlock, Constant, Function, Instruction, InstructionKind, Local, Module, Operand,
    RuntimeFunc, Terminator,
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

/// Create a trivial getter function: `return self.field[offset]`
fn make_trivial_getter(func_id: FuncId, offset: i64, self_type: Type) -> Function {
    let self_param = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: self_type,
        is_gc_root: true,
    };
    let dest_local = make_local(1, Type::Int);
    let block_id = BlockId::from(0u32);

    let mut locals = IndexMap::new();
    locals.insert(dest_local.id, dest_local.clone());

    let mut blocks = IndexMap::new();
    blocks.insert(
        block_id,
        BasicBlock {
            id: block_id,
            instructions: vec![make_inst(InstructionKind::RuntimeCall {
                dest: LocalId::from(1u32),
                func: RuntimeFunc::Call(&RT_INSTANCE_GET_FIELD),
                args: vec![
                    Operand::Local(LocalId::from(0u32)),
                    Operand::Constant(Constant::Int(offset)),
                ],
            })],
            terminator: Terminator::Return(Some(Operand::Local(LocalId::from(1u32)))),
        },
    );

    Function {
        id: func_id,
        name: "Foo$x".to_string(),
        params: vec![self_param],
        return_type: Type::Int,
        locals,
        blocks,
        entry_block: block_id,
        span: None,
    }
}

/// Create a caller function that calls getter_id with obj
fn make_caller_with_call(caller_id: FuncId, getter_id: FuncId, obj_type: Type) -> Function {
    let obj_local = Local {
        id: LocalId::from(10u32),
        name: None,
        ty: obj_type,
        is_gc_root: true,
    };
    let dest_local = make_local(11, Type::Int);
    let block_id = BlockId::from(0u32);

    let mut locals = IndexMap::new();
    locals.insert(obj_local.id, obj_local.clone());
    locals.insert(dest_local.id, dest_local.clone());

    let mut blocks = IndexMap::new();
    blocks.insert(
        block_id,
        BasicBlock {
            id: block_id,
            instructions: vec![make_inst(InstructionKind::CallDirect {
                dest: LocalId::from(11u32),
                func: getter_id,
                args: vec![Operand::Local(LocalId::from(10u32))],
            })],
            terminator: Terminator::Return(Some(Operand::Local(LocalId::from(11u32)))),
        },
    );

    Function {
        id: caller_id,
        name: "main".to_string(),
        params: vec![],
        return_type: Type::Int,
        locals,
        blocks,
        entry_block: block_id,
        span: None,
    }
}

#[test]
fn test_flatten_trivial_getter() {
    let mut interner = StringInterner::new();
    let class_type = make_class_type(&mut interner, ClassId::from(1u32), "Foo");
    let getter_id = FuncId::from(1u32);
    let caller_id = FuncId::from(0u32);
    let offset = 3;

    let getter = make_trivial_getter(getter_id, offset, class_type.clone());
    let caller = make_caller_with_call(caller_id, getter_id, class_type);

    let mut module = Module::new();
    module.add_function(caller);
    module.add_function(getter);

    super::flatten_property_getters(&mut module);

    let func = module.functions.get(&caller_id).unwrap();
    let block = func.blocks.values().next().unwrap();
    let inst = &block.instructions[0];

    match &inst.kind {
        InstructionKind::RuntimeCall { dest, func, args } => {
            assert_eq!(*dest, LocalId::from(11u32));
            assert!(
                matches!(func, RuntimeFunc::Call(def) if def.symbol == RT_INSTANCE_GET_FIELD.symbol)
            );
            assert_eq!(args.len(), 2);
            assert_eq!(args[0], Operand::Local(LocalId::from(10u32)));
            assert_eq!(args[1], Operand::Constant(Constant::Int(offset)));
        }
        other => panic!("Expected RuntimeCall(InstanceGetField), got {:?}", other),
    }
}

#[test]
fn test_skip_non_trivial_getter_multiple_blocks() {
    let mut interner = StringInterner::new();
    let class_type = make_class_type(&mut interner, ClassId::from(1u32), "Foo");
    let getter_id = FuncId::from(1u32);
    let caller_id = FuncId::from(0u32);

    // Getter with 2 blocks — not trivial
    let self_param = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: class_type.clone(),
        is_gc_root: true,
    };
    let dest_local = make_local(1, Type::Int);
    let block0 = BlockId::from(0u32);
    let block1 = BlockId::from(1u32);

    let mut locals = IndexMap::new();
    locals.insert(dest_local.id, dest_local.clone());

    let mut blocks = IndexMap::new();
    blocks.insert(
        block0,
        BasicBlock {
            id: block0,
            instructions: vec![],
            terminator: Terminator::Goto(block1),
        },
    );
    blocks.insert(
        block1,
        BasicBlock {
            id: block1,
            instructions: vec![make_inst(InstructionKind::RuntimeCall {
                dest: LocalId::from(1u32),
                func: RuntimeFunc::Call(&RT_INSTANCE_GET_FIELD),
                args: vec![
                    Operand::Local(LocalId::from(0u32)),
                    Operand::Constant(Constant::Int(0)),
                ],
            })],
            terminator: Terminator::Return(Some(Operand::Local(LocalId::from(1u32)))),
        },
    );

    let getter = Function {
        id: getter_id,
        name: "Foo$x".to_string(),
        params: vec![self_param],
        return_type: Type::Int,
        locals,
        blocks,
        entry_block: block0,
        span: None,
    };

    let caller = make_caller_with_call(caller_id, getter_id, class_type);

    let mut module = Module::new();
    module.add_function(caller);
    module.add_function(getter);

    super::flatten_property_getters(&mut module);

    // Should remain CallDirect since getter has multiple blocks
    let func = module.functions.get(&caller_id).unwrap();
    let block = func.blocks.values().next().unwrap();
    let inst = &block.instructions[0];
    assert!(matches!(&inst.kind, InstructionKind::CallDirect { .. }));
}

#[test]
fn test_skip_non_trivial_getter_multiple_instructions() {
    let mut interner = StringInterner::new();
    let class_type = make_class_type(&mut interner, ClassId::from(1u32), "Foo");
    let getter_id = FuncId::from(1u32);
    let caller_id = FuncId::from(0u32);

    let self_param = Local {
        id: LocalId::from(0u32),
        name: None,
        ty: class_type.clone(),
        is_gc_root: true,
    };
    let dest_local = make_local(1, Type::Int);
    let extra_local = make_local(2, Type::Int);
    let block_id = BlockId::from(0u32);

    let mut locals = IndexMap::new();
    locals.insert(dest_local.id, dest_local.clone());
    locals.insert(extra_local.id, extra_local.clone());

    let mut blocks = IndexMap::new();
    blocks.insert(
        block_id,
        BasicBlock {
            id: block_id,
            instructions: vec![
                // Two instructions — not trivial
                make_inst(InstructionKind::RuntimeCall {
                    dest: LocalId::from(1u32),
                    func: RuntimeFunc::Call(&RT_INSTANCE_GET_FIELD),
                    args: vec![
                        Operand::Local(LocalId::from(0u32)),
                        Operand::Constant(Constant::Int(0)),
                    ],
                }),
                make_inst(InstructionKind::Const {
                    dest: LocalId::from(2u32),
                    value: Constant::Int(1),
                }),
            ],
            terminator: Terminator::Return(Some(Operand::Local(LocalId::from(1u32)))),
        },
    );

    let getter = Function {
        id: getter_id,
        name: "Foo$x".to_string(),
        params: vec![self_param],
        return_type: Type::Int,
        locals,
        blocks,
        entry_block: block_id,
        span: None,
    };

    let caller = make_caller_with_call(caller_id, getter_id, class_type);

    let mut module = Module::new();
    module.add_function(caller);
    module.add_function(getter);

    super::flatten_property_getters(&mut module);

    // Should remain CallDirect
    let func = module.functions.get(&caller_id).unwrap();
    let block = func.blocks.values().next().unwrap();
    let inst = &block.instructions[0];
    assert!(matches!(&inst.kind, InstructionKind::CallDirect { .. }));
}

#[test]
fn test_skip_call_with_multiple_args() {
    let mut interner = StringInterner::new();
    let class_type = make_class_type(&mut interner, ClassId::from(1u32), "Foo");
    let getter_id = FuncId::from(1u32);
    let caller_id = FuncId::from(0u32);

    let getter = make_trivial_getter(getter_id, 0, class_type.clone());

    // Caller passes 2 args — not a property getter call
    let obj_local = Local {
        id: LocalId::from(10u32),
        name: None,
        ty: class_type,
        is_gc_root: true,
    };
    let dest_local = make_local(11, Type::Int);
    let block_id = BlockId::from(0u32);

    let mut locals = IndexMap::new();
    locals.insert(obj_local.id, obj_local.clone());
    locals.insert(dest_local.id, dest_local.clone());

    let mut blocks = IndexMap::new();
    blocks.insert(
        block_id,
        BasicBlock {
            id: block_id,
            instructions: vec![make_inst(InstructionKind::CallDirect {
                dest: LocalId::from(11u32),
                func: getter_id,
                args: vec![
                    Operand::Local(LocalId::from(10u32)),
                    Operand::Constant(Constant::Int(99)),
                ],
            })],
            terminator: Terminator::Return(None),
        },
    );

    let caller = Function {
        id: caller_id,
        name: "main".to_string(),
        params: vec![],
        return_type: Type::None,
        locals,
        blocks,
        entry_block: block_id,
        span: None,
    };

    let mut module = Module::new();
    module.add_function(caller);
    module.add_function(getter);

    super::flatten_property_getters(&mut module);

    // Should remain CallDirect because args.len() != 1
    let func = module.functions.get(&caller_id).unwrap();
    let block = func.blocks.values().next().unwrap();
    let inst = &block.instructions[0];
    assert!(matches!(&inst.kind, InstructionKind::CallDirect { .. }));
}

#[test]
fn test_no_trivial_getters_is_noop() {
    let func = Function::new(FuncId::from(0u32), "test".to_string(), vec![], Type::None);
    let mut module = Module::new();
    module.add_function(func);

    super::flatten_property_getters(&mut module);

    assert_eq!(module.functions.len(), 1);
}
