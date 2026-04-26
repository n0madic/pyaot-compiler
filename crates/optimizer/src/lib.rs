//! MIR Optimizer
//!
//! Provides optimization passes for MIR before codegen.
//! Implements devirtualization, property flattening, function inlining,
//! constant folding & propagation, peephole simplification, and dead code elimination.
//!
//! Each pass implements the `OptimizationPass` trait and is orchestrated
//! by the `PassManager`, which handles fixpoint iteration for passes that
//! require it.

#![forbid(unsafe_code)]

pub mod abi_repair;
pub mod call_graph;
pub mod constfold;
pub mod dce;
pub mod devirtualize;
pub mod flatten_properties;
pub mod inline;
pub mod pass;
pub mod peephole;
pub mod type_inference;

use pyaot_mir::Module;
use pyaot_utils::StringInterner;

pub use pass::{build_pass_pipeline, OptimizationPass, PassManager};

/// Configuration for optimization passes
#[derive(Debug, Clone)]
pub struct OptimizeConfig {
    /// Enable devirtualization (replace virtual calls with direct calls)
    pub devirtualize: bool,
    /// Enable property flattening (inline trivial @property getters)
    pub flatten_properties: bool,
    /// Enable function inlining
    pub inline: bool,
    /// Maximum instruction count for inlining consideration
    pub inline_threshold: usize,
    /// Enable dead code elimination
    pub dce: bool,
    /// Enable constant folding and propagation
    pub constfold: bool,
}

impl Default for OptimizeConfig {
    fn default() -> Self {
        Self {
            devirtualize: true,
            flatten_properties: true,
            inline: true,
            inline_threshold: 50,
            dce: true,
            constfold: true,
        }
    }
}

/// Run all enabled optimization passes on the MIR module.
///
/// Pass order: devirtualize → flatten_properties → inline → constfold → peephole → dce
/// - Devirtualization converts virtual calls to direct calls when receiver type is known
/// - Property flattening inlines trivial getters as field accesses
/// - Inlining exposes constant expressions across function boundaries
/// - Constant folding simplifies expressions and branches
/// - Peephole simplifies local patterns (identity ops, strength reduction, box/unbox)
/// - DCE cleans up dead code left by earlier passes
pub fn optimize_module(
    module: &mut Module,
    config: &OptimizeConfig,
    interner: &mut StringInterner,
) {
    let mut pm = build_pass_pipeline(config);
    pm.run(module, interner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use pyaot_mir::{
        BasicBlock, ClassMetadata, Constant, Function, Instruction, InstructionKind, Local,
        Operand, Terminator, VtableEntry, VtableInfo,
    };
    use pyaot_types::Type;
    use pyaot_utils::{BlockId, ClassId, FuncId, LocalId, StringInterner};

    fn make_local(id: u32, ty: Type) -> Local {
        Local {
            id: LocalId::from(id),
            name: None,
            ty: ty.clone(),
            is_gc_root: ty.is_heap(),
        }
    }

    fn make_inst(kind: InstructionKind) -> Instruction {
        Instruction { kind, span: None }
    }

    fn make_func(
        func_id: u32,
        name: &str,
        params: Vec<Local>,
        return_type: Type,
        locals: Vec<Local>,
        instructions: Vec<InstructionKind>,
    ) -> Function {
        let block_id = BlockId::from(0u32);
        let mut local_map = IndexMap::new();
        for local in locals {
            local_map.insert(local.id, local);
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
            id: FuncId::from(func_id),
            name: name.to_string(),
            params,
            return_type,
            locals: local_map,
            blocks,
            entry_block: block_id,
            span: None,
            is_ssa: true,
            dom_tree_cache: std::cell::OnceCell::new(),
        }
    }

    #[test]
    fn abi_repair_materializes_direct_call_coercions_before_codegen() {
        let callee_id = FuncId::from(1u32);
        let float_caller_id = FuncId::from(2u32);
        let int_caller_id = FuncId::from(3u32);
        let param = make_local(0, Type::Any);

        let callee = make_func(
            1,
            "callee",
            vec![param.clone()],
            Type::None,
            vec![param],
            vec![],
        );
        let float_caller = make_func(
            2,
            "float_caller",
            vec![],
            Type::None,
            vec![make_local(0, Type::Float), make_local(1, Type::None)],
            vec![InstructionKind::CallDirect {
                dest: LocalId::from(1u32),
                func: callee_id,
                args: vec![Operand::Local(LocalId::from(0u32))],
            }],
        );
        let caller = make_func(
            3,
            "int_caller",
            vec![],
            Type::None,
            vec![make_local(0, Type::Int), make_local(1, Type::None)],
            vec![InstructionKind::CallDirect {
                dest: LocalId::from(1u32),
                func: callee_id,
                args: vec![Operand::Local(LocalId::from(0u32))],
            }],
        );

        let mut module = Module::new();
        module.add_function(callee);
        module.add_function(float_caller);
        module.add_function(caller);

        crate::type_inference::analyze_and_materialize_types(&mut module);
        crate::abi_repair::repair_mir_abi_from_types(&mut module)
            .expect("ABI repair should succeed");

        let caller = module
            .functions
            .get(&int_caller_id)
            .expect("int caller exists");
        let block = caller
            .blocks
            .get(&caller.entry_block)
            .expect("entry block exists");
        assert_eq!(block.instructions.len(), 2);
        match &block.instructions[0].kind {
            InstructionKind::IntToFloat { .. } => {}
            other => panic!("expected IntToFloat before direct call, got {other:?}"),
        }
        match &block.instructions[1].kind {
            InstructionKind::CallDirect { args, .. } => {
                assert!(matches!(args[0], Operand::Local(_)));
            }
            other => panic!("expected repaired direct call, got {other:?}"),
        }

        let float_caller = module
            .functions
            .get(&float_caller_id)
            .expect("float caller exists");
        let float_block = float_caller
            .blocks
            .get(&float_caller.entry_block)
            .expect("entry block exists");
        assert_eq!(float_block.instructions.len(), 1);
    }

    #[test]
    fn abi_repair_rewrites_indirect_calls_to_repaired_internal_abi() {
        let callee_id = FuncId::from(10u32);
        let caller_id = FuncId::from(11u32);

        let callee = make_func(
            10,
            "callee",
            vec![make_local(0, Type::Any)],
            Type::None,
            vec![make_local(0, Type::Any)],
            vec![],
        );
        let direct_caller = make_func(
            12,
            "direct_caller",
            vec![],
            Type::None,
            vec![make_local(0, Type::None)],
            vec![InstructionKind::CallDirect {
                dest: LocalId::from(0u32),
                func: callee_id,
                args: vec![Operand::Constant(Constant::Int(7))],
            }],
        );
        let caller = make_func(
            11,
            "caller",
            vec![],
            Type::None,
            vec![
                make_local(0, Type::Any),
                make_local(1, Type::Bool),
                make_local(2, Type::None),
            ],
            vec![
                InstructionKind::FuncAddr {
                    dest: LocalId::from(0u32),
                    func: callee_id,
                },
                InstructionKind::Call {
                    dest: LocalId::from(2u32),
                    func: Operand::Local(LocalId::from(0u32)),
                    args: vec![Operand::Local(LocalId::from(1u32))],
                },
            ],
        );

        let mut module = Module::new();
        module.add_function(callee);
        module.add_function(direct_caller);
        module.add_function(caller);

        crate::type_inference::analyze_and_materialize_types(&mut module);
        crate::abi_repair::repair_mir_abi_from_types(&mut module)
            .expect("ABI repair should succeed");

        let caller = module.functions.get(&caller_id).expect("caller exists");
        let block = caller
            .blocks
            .get(&caller.entry_block)
            .expect("entry block exists");
        assert_eq!(block.instructions.len(), 3);
        assert!(matches!(
            block.instructions[0].kind,
            InstructionKind::FuncAddr {
                dest,
                func
            } if dest == LocalId::from(0u32) && func == callee_id
        ));
        assert!(matches!(
            block.instructions[1].kind,
            InstructionKind::BoolToInt {
                dest,
                src: Operand::Local(local)
            } if dest == LocalId::from(3u32) && local == LocalId::from(1u32)
        ));
        match &block.instructions[2].kind {
            InstructionKind::CallDirect { func, args, .. } => {
                assert_eq!(*func, callee_id);
                assert!(matches!(args[0], Operand::Local(local) if local == LocalId::from(3u32)));
            }
            other => panic!("expected repaired indirect call, got {other:?}"),
        }
    }

    #[test]
    fn abi_repair_rewrites_virtual_calls_to_repaired_internal_abi() {
        let class_id = ClassId::from(30u32);
        let class_name = pyaot_utils::StringInterner::default().intern("Receiver");
        let class_ty = Type::Class {
            class_id,
            name: class_name,
        };
        let method_id = FuncId::from(31u32);
        let caller_id = FuncId::from(32u32);

        let method = make_func(
            31,
            "Receiver$m",
            vec![make_local(0, class_ty.clone()), make_local(1, Type::Any)],
            Type::None,
            vec![make_local(0, class_ty.clone()), make_local(1, Type::Any)],
            vec![],
        );
        let direct_caller = make_func(
            33,
            "direct_caller",
            vec![],
            Type::None,
            vec![make_local(0, class_ty.clone()), make_local(1, Type::None)],
            vec![InstructionKind::CallDirect {
                dest: LocalId::from(1u32),
                func: method_id,
                args: vec![
                    Operand::Local(LocalId::from(0u32)),
                    Operand::Constant(Constant::Int(9)),
                ],
            }],
        );
        let caller = make_func(
            32,
            "caller",
            vec![],
            Type::None,
            vec![
                make_local(0, class_ty.clone()),
                make_local(1, Type::Bool),
                make_local(2, Type::None),
            ],
            vec![InstructionKind::CallVirtual {
                dest: LocalId::from(2u32),
                obj: Operand::Local(LocalId::from(0u32)),
                slot: 0,
                args: vec![Operand::Local(LocalId::from(1u32))],
            }],
        );

        let mut module = Module::new();
        module.add_function(method);
        module.add_function(direct_caller);
        module.add_function(caller);
        module.vtables.push(VtableInfo {
            class_id,
            entries: vec![VtableEntry {
                slot: 0,
                name_hash: 0,
                method_func_id: method_id,
            }],
        });

        crate::type_inference::analyze_and_materialize_types(&mut module);
        crate::abi_repair::repair_mir_abi_from_types(&mut module)
            .expect("ABI repair should succeed");

        let caller = module.functions.get(&caller_id).expect("caller exists");
        let block = caller
            .blocks
            .get(&caller.entry_block)
            .expect("entry block exists");
        assert_eq!(block.instructions.len(), 2);
        assert!(matches!(
            block.instructions[0].kind,
            InstructionKind::BoolToInt {
                dest,
                src: Operand::Local(local)
            } if dest == LocalId::from(3u32) && local == LocalId::from(1u32)
        ));
        match &block.instructions[1].kind {
            InstructionKind::CallDirect { func, args, .. } => {
                assert_eq!(*func, method_id);
                assert!(matches!(args[0], Operand::Local(local) if local == LocalId::from(0u32)));
                assert!(matches!(args[1], Operand::Local(local) if local == LocalId::from(3u32)));
            }
            other => panic!("expected repaired virtual call, got {other:?}"),
        }
    }

    #[test]
    fn abi_repair_rejects_divergent_internal_indirect_sites() {
        let caller_id = FuncId::from(40u32);

        let mut callee_int = make_func(
            41,
            "takes_int",
            vec![],
            Type::Int,
            vec![make_local(0, Type::Int)],
            vec![InstructionKind::Copy {
                dest: LocalId::from(0u32),
                src: Operand::Constant(Constant::Int(1)),
            }],
        );
        let mut callee_float = make_func(
            42,
            "takes_float",
            vec![],
            Type::Float,
            vec![make_local(0, Type::Float)],
            vec![InstructionKind::Copy {
                dest: LocalId::from(0u32),
                src: Operand::Constant(Constant::Float(1.0)),
            }],
        );
        callee_int.block_mut(callee_int.entry_block).terminator =
            Terminator::Return(Some(Operand::Local(LocalId::from(0u32))));
        callee_float.block_mut(callee_float.entry_block).terminator =
            Terminator::Return(Some(Operand::Local(LocalId::from(0u32))));
        let caller = make_func(
            40,
            "caller",
            vec![],
            Type::None,
            vec![
                make_local(0, Type::Any),
                make_local(1, Type::Any),
                make_local(2, Type::Any),
                make_local(3, Type::Any),
            ],
            vec![
                InstructionKind::FuncAddr {
                    dest: LocalId::from(0u32),
                    func: FuncId::from(41u32),
                },
                InstructionKind::FuncAddr {
                    dest: LocalId::from(1u32),
                    func: FuncId::from(42u32),
                },
                InstructionKind::Phi {
                    dest: LocalId::from(2u32),
                    sources: vec![
                        (BlockId::from(0u32), Operand::Local(LocalId::from(0u32))),
                        (BlockId::from(0u32), Operand::Local(LocalId::from(1u32))),
                    ],
                },
                InstructionKind::Call {
                    dest: LocalId::from(3u32),
                    func: Operand::Local(LocalId::from(2u32)),
                    args: vec![],
                },
            ],
        );

        let mut module = Module::new();
        module.add_function(callee_int);
        module.add_function(callee_float);
        module.add_function(caller);

        crate::type_inference::analyze_and_materialize_types(&mut module);
        assert_eq!(
            module.functions[&FuncId::from(41u32)].return_type,
            Type::Int
        );
        assert_eq!(
            module.functions[&FuncId::from(42u32)].return_type,
            Type::Float
        );
        let graph = crate::call_graph::CallGraph::build(&module);
        assert_eq!(graph.callees[&caller_id].len(), 2);
        let err = crate::abi_repair::repair_mir_abi_from_types(&mut module)
            .expect_err("divergent higher-order sites should be rejected");
        let msg = format!("{err}");
        assert!(msg.contains("divergent ABI"), "unexpected error: {msg}");
    }

    #[test]
    fn abi_repair_rewrites_internal_named_calls_to_direct() {
        let callee_id = FuncId::from(60u32);
        let caller_id = FuncId::from(61u32);

        let callee = make_func(
            60,
            "__module_other_callee",
            vec![make_local(0, Type::Int)],
            Type::None,
            vec![make_local(0, Type::Int)],
            vec![],
        );
        let caller = make_func(
            61,
            "caller",
            vec![],
            Type::None,
            vec![make_local(0, Type::Bool), make_local(1, Type::None)],
            vec![InstructionKind::CallNamed {
                dest: LocalId::from(1u32),
                name: "__module_other_callee".to_string(),
                args: vec![Operand::Local(LocalId::from(0u32))],
            }],
        );

        let mut module = Module::new();
        module.add_function(callee);
        module.add_function(caller);

        crate::type_inference::analyze_and_materialize_types(&mut module);
        crate::abi_repair::repair_mir_abi_from_types(&mut module)
            .expect("ABI repair should resolve internal named calls");

        let caller = module.functions.get(&caller_id).expect("caller exists");
        let block = caller
            .blocks
            .get(&caller.entry_block)
            .expect("entry block exists");
        assert_eq!(block.instructions.len(), 2);
        assert!(matches!(
            block.instructions[0].kind,
            InstructionKind::BoolToInt {
                dest,
                src: Operand::Local(local)
            } if dest == LocalId::from(2u32) && local == LocalId::from(0u32)
        ));
        match &block.instructions[1].kind {
            InstructionKind::CallDirect { func, args, .. } => {
                assert_eq!(*func, callee_id);
                assert!(matches!(args[0], Operand::Local(local) if local == LocalId::from(2u32)));
            }
            other => panic!("expected CallNamed to rewrite to CallDirect, got {other:?}"),
        }
    }

    #[test]
    fn abi_repair_field_write_passes_value_through_for_int_field() {
        let mut interner = StringInterner::default();
        let class_name = interner.intern("BoxedField");
        let field_name = interner.intern("data");
        let class_id = ClassId::from(50u32);

        let mut init = make_func(
            50,
            "BoxedField$__init__",
            vec![make_local(
                0,
                Type::Class {
                    class_id,
                    name: class_name,
                },
            )],
            Type::None,
            vec![
                make_local(
                    0,
                    Type::Class {
                        class_id,
                        name: class_name,
                    },
                ),
                make_local(1, Type::Any),
                make_local(2, Type::None),
            ],
            vec![InstructionKind::RuntimeCall {
                dest: LocalId::from(2u32),
                func: pyaot_mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_INSTANCE_SET_FIELD,
                ),
                args: vec![
                    Operand::Local(LocalId::from(0u32)),
                    Operand::Constant(Constant::Int(0)),
                    Operand::Local(LocalId::from(1u32)),
                ],
            }],
        );
        init.params.push(make_local(1, Type::Any));

        let mut module = Module::new();
        module.add_function(init);
        module.class_info.insert(
            class_id,
            ClassMetadata {
                class_id,
                init_func_id: Some(FuncId::from(50u32)),
                field_offsets: IndexMap::from([(field_name, 0usize)]),
                field_types: IndexMap::from([(field_name, Type::Int)]),
                base_class: None,
                is_protocol: false,
            },
        );

        crate::abi_repair::repair_mir_abi_from_types(&mut module)
            .expect("field-write repair should succeed");

        // §F.7c: `InstanceObj.fields` stores uniform tagged `Value` words.
        // Field-write paths coerce primitives to `HeapAny` (via Value-tag
        // wrapping) instead of unboxing — value flows through to
        // `rt_instance_set_field` as already-tagged bits.
        let init = &module.functions[&FuncId::from(50u32)];
        let block = &init.blocks[&init.entry_block];
        // Source operand has type `Any` here, which already matches the
        // HeapAny ABI shape — no extra wrap/unwrap is emitted.
        assert_eq!(block.instructions.len(), 1);
        match &block.instructions[0].kind {
            InstructionKind::RuntimeCall {
                func: pyaot_mir::RuntimeFunc::Call(def),
                args,
                ..
            } => {
                assert!(std::ptr::eq(
                    *def,
                    &pyaot_core_defs::runtime_func_def::RT_INSTANCE_SET_FIELD
                ));
                assert!(matches!(args[2], Operand::Local(local) if local == LocalId::from(1u32)));
            }
            other => panic!("expected pass-through field write, got {other:?}"),
        }
    }
}
