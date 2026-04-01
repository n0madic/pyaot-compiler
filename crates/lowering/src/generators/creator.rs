//! Generator creator function generation
//!
//! This module handles generating the "creator" function for generators.
//! The creator function allocates a generator object and initializes it
//! with parameters and the iterator (for for-loop generators).

use std::collections::HashMap;

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{BlockId, LocalId, VarId};

use super::GeneratorVar;
use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Create the generator creator function
    /// This function creates a generator object and returns it
    pub(super) fn create_generator_creator(
        &mut self,
        func: &hir::Function,
        hir_module: &hir::Module,
        gen_vars: &[GeneratorVar],
        num_locals: u32,
    ) -> Result<mir::Function> {
        // Reset local ID counter for this function
        self.reset_local_id();

        let func_name = self.resolve(func.name).to_string();

        // Creator function has same parameters as original
        // Note: We need to pre-allocate params before creating mir_func,
        // so we manually build the param list here
        let mut params = Vec::new();
        let mut param_local_ids = Vec::new();
        for hir_param in &func.params {
            let local_id = self.alloc_local_id();
            let param_ty = hir_param.ty.clone().unwrap_or(Type::Any);
            params.push(mir::Local {
                id: local_id,
                name: None,
                ty: param_ty.clone(),
                is_gc_root: param_ty.is_heap(),
            });
            param_local_ids.push(local_id);
        }

        // Return type is a generator object (pointer)
        let return_type = Type::Iterator(Box::new(Type::Any)); // Generator is an iterator

        let mut mir_func =
            mir::Function::new(func.id, func_name.clone(), params.clone(), return_type);
        mir_func.span = Some(func.span);

        // Add parameters to locals
        for param in &params {
            mir_func.add_local(param.clone());
        }

        // Create entry block
        let entry_block_id = BlockId::from(0u32);
        let mut entry_block = mir::BasicBlock {
            id: entry_block_id,
            instructions: Vec::new(),
            terminator: mir::Terminator::Unreachable,
        };

        // Allocate generator object
        let gen_local =
            self.alloc_and_add_local(Type::Iterator(Box::new(Type::Any)), &mut mir_func);

        // Dummy dest for void calls
        let dummy_local = self.alloc_and_add_local(Type::Int, &mut mir_func);

        // rt_make_generator(func_id, num_locals)
        entry_block.instructions.push(mir::Instruction {
            kind: mir::InstructionKind::RuntimeCall {
                dest: gen_local,
                func: mir::RuntimeFunc::MakeGenerator,
                args: vec![
                    mir::Operand::Constant(mir::Constant::Int(func.id.0 as i64)),
                    mir::Operand::Constant(mir::Constant::Int(num_locals as i64)),
                ],
            },
            span: None,
        });

        // Save all parameters to generator locals
        for gen_var in gen_vars.iter() {
            if gen_var.is_param {
                // Find the parameter's local ID
                let param_idx = func.params.iter().position(|p| p.var == gen_var.var_id);
                if let Some(idx) = param_idx {
                    let param_local = param_local_ids[idx];
                    entry_block.instructions.push(mir::Instruction {
                        kind: mir::InstructionKind::RuntimeCall {
                            dest: dummy_local,
                            func: mir::RuntimeFunc::GeneratorSetLocal,
                            args: vec![
                                mir::Operand::Local(gen_local),
                                mir::Operand::Constant(mir::Constant::Int(
                                    gen_var.gen_local_idx as i64,
                                )),
                                mir::Operand::Local(param_local),
                            ],
                        },
                        span: None,
                    });
                }
            }
        }

        // Initialize local variables assigned before the first yield.
        // Walk body statements until the first yield and emit GeneratorSetLocal
        // for constant assignments (e.g., x: int = 5).
        'init_loop: for stmt_id in &func.body {
            let stmt = &hir_module.stmts[*stmt_id];
            match &stmt.kind {
                hir::StmtKind::Assign { target, value, .. } => {
                    if let Some(gen_var) = gen_vars.iter().find(|v| v.var_id == *target) {
                        let val_expr = &hir_module.exprs[*value];
                        let val_op = match &val_expr.kind {
                            hir::ExprKind::Int(n) => {
                                Some(mir::Operand::Constant(mir::Constant::Int(*n)))
                            }
                            hir::ExprKind::Float(f) => {
                                Some(mir::Operand::Constant(mir::Constant::Float(*f)))
                            }
                            hir::ExprKind::Bool(b) => {
                                Some(mir::Operand::Constant(mir::Constant::Int(if *b {
                                    1
                                } else {
                                    0
                                })))
                            }
                            _ => None,
                        };
                        if let Some(op) = val_op {
                            entry_block.instructions.push(mir::Instruction {
                                kind: mir::InstructionKind::RuntimeCall {
                                    dest: dummy_local,
                                    func: mir::RuntimeFunc::GeneratorSetLocal,
                                    args: vec![
                                        mir::Operand::Local(gen_local),
                                        mir::Operand::Constant(mir::Constant::Int(
                                            gen_var.gen_local_idx as i64,
                                        )),
                                        op,
                                    ],
                                },
                                span: None,
                            });
                        }
                    }
                }
                hir::StmtKind::Expr(expr_id) => {
                    // Check if this is a yield — stop scanning
                    let expr = &hir_module.exprs[*expr_id];
                    if matches!(expr.kind, hir::ExprKind::Yield(_)) {
                        break 'init_loop;
                    }
                }
                _ => {}
            }
        }

        // For for-loop generators, we need to initialize the iterator
        if let Some(for_gen) = self.detect_for_loop_generator(&func.body, hir_module) {
            // Allocate local for iterator
            let iter_local =
                self.alloc_and_add_local(Type::Iterator(Box::new(Type::Any)), &mut mir_func);

            // Lower the iterable expression to get an iterator
            // We need to handle the iterator creation here
            let iter_expr = &hir_module.exprs[for_gen.iter_expr];

            // Build a map from VarId to MIR local for parameters (needed for lowering)
            let mut var_to_mir_local: HashMap<VarId, LocalId> = HashMap::new();
            for (idx, param) in func.params.iter().enumerate() {
                var_to_mir_local.insert(param.var, param_local_ids[idx]);
            }

            // Lower the iterable expression
            // For most cases, this will be a function call (e.g., range_gen(1, 4))
            let iter_operand = self.lower_iter_expr_for_creator(
                iter_expr,
                hir_module,
                &mut entry_block,
                &mut mir_func,
                &var_to_mir_local,
                iter_local,
            )?;

            // Store the iterator in generator local slot 0
            entry_block.instructions.push(mir::Instruction {
                kind: mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::GeneratorSetLocal,
                    args: vec![
                        mir::Operand::Local(gen_local),
                        mir::Operand::Constant(mir::Constant::Int(0)), // Slot 0 for iterator
                        iter_operand,
                    ],
                },
                span: None,
            });

            // Mark slot 0 as a heap pointer so the GC traces the inner iterator.
            // LOCAL_TYPE_PTR = 3; without this the default LOCAL_TYPE_RAW_INT (0)
            // prevents the GC from marking the inner generator, causing use-after-free.
            entry_block.instructions.push(mir::Instruction {
                kind: mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::GeneratorSetLocalType,
                    args: vec![
                        mir::Operand::Local(gen_local),
                        mir::Operand::Constant(mir::Constant::Int(0)), // Slot 0
                        mir::Operand::Constant(mir::Constant::Int(3)), // LOCAL_TYPE_PTR
                    ],
                },
                span: None,
            });
        }

        // Return the generator object
        entry_block.terminator = mir::Terminator::Return(Some(mir::Operand::Local(gen_local)));

        mir_func.blocks.insert(entry_block_id, entry_block);
        mir_func.entry_block = entry_block_id;

        Ok(mir_func)
    }

    /// Lower an iterator expression for the creator function
    /// This handles function calls like range_gen(1, 4) or list literals like [1, 2, 3]
    pub(super) fn lower_iter_expr_for_creator(
        &mut self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        block: &mut mir::BasicBlock,
        mir_func: &mut mir::Function,
        var_to_mir_local: &HashMap<VarId, LocalId>,
        dest_local: LocalId,
    ) -> Result<mir::Operand> {
        match &expr.kind {
            hir::ExprKind::Call {
                func: func_expr,
                args,
                ..
            } => {
                // Handle function calls like range_gen(1, 4)
                let func_ref_expr = &hir_module.exprs[*func_expr];

                // Get the function ID if this is a direct function reference
                let func_id = match &func_ref_expr.kind {
                    hir::ExprKind::FuncRef(fid) => Some(*fid),
                    _ => None,
                };

                // Lower arguments
                let mut arg_operands = Vec::new();
                for arg in args {
                    let arg_id = match arg {
                        hir::CallArg::Regular(id) => id,
                        hir::CallArg::Starred(id) => id,
                    };
                    let arg_expr = &hir_module.exprs[*arg_id];
                    let arg_op = match &arg_expr.kind {
                        hir::ExprKind::Int(n) => mir::Operand::Constant(mir::Constant::Int(*n)),
                        hir::ExprKind::Var(var_id) => {
                            if let Some(&mir_local) = var_to_mir_local.get(var_id) {
                                mir::Operand::Local(mir_local)
                            } else if self.is_global(var_id) {
                                // Global variable — emit GlobalGet
                                let var_type =
                                    self.get_var_type(var_id).cloned().unwrap_or(Type::Int);
                                let runtime_func = self.get_global_get_func(&var_type);
                                let effective_var_id = self.get_effective_var_id(*var_id);
                                let global_local = self.alloc_and_add_local(var_type, mir_func);
                                block.instructions.push(mir::Instruction {
                                    kind: mir::InstructionKind::RuntimeCall {
                                        dest: global_local,
                                        func: runtime_func,
                                        args: vec![mir::Operand::Constant(mir::Constant::Int(
                                            effective_var_id,
                                        ))],
                                    },
                                    span: None,
                                });
                                mir::Operand::Local(global_local)
                            } else {
                                return Err(pyaot_diagnostics::CompilerError::codegen_error_at(
                                    "unresolved variable in generator creator expression",
                                    arg_expr.span,
                                ));
                            }
                        }
                        _ => {
                            return Err(pyaot_diagnostics::CompilerError::codegen_error_at(
                                "unsupported expression in generator creator argument",
                                arg_expr.span,
                            ));
                        }
                    };
                    arg_operands.push(arg_op);
                }

                // Emit the call instruction
                if let Some(fid) = func_id {
                    block.instructions.push(mir::Instruction {
                        kind: mir::InstructionKind::CallDirect {
                            dest: dest_local,
                            func: fid,
                            args: arg_operands,
                        },
                        span: None,
                    });
                }

                Ok(mir::Operand::Local(dest_local))
            }
            hir::ExprKind::List(elements) => {
                // Handle list literals like [1, 2, 3]
                // First create the list.
                // Use Type::Any as the element type so the list can hold any element
                // kind; using Type::Int was incorrect for lists containing heap objects.
                let list_local =
                    self.alloc_and_add_local(Type::List(Box::new(Type::Any)), mir_func);

                // Create empty list (generator list comprehension elements are ints)
                block.instructions.push(mir::Instruction {
                    kind: mir::InstructionKind::RuntimeCall {
                        dest: list_local,
                        func: mir::RuntimeFunc::MakeList,
                        args: vec![
                            mir::Operand::Constant(mir::Constant::Int(elements.len() as i64)),
                            mir::Operand::Constant(mir::Constant::Int(1)), // ELEM_RAW_INT
                        ],
                    },
                    span: None,
                });

                // Add elements
                let dummy = self.alloc_and_add_local(Type::Int, mir_func);

                for elem_id in elements {
                    let elem_expr = &hir_module.exprs[*elem_id];
                    let elem_op = match &elem_expr.kind {
                        hir::ExprKind::Int(n) => mir::Operand::Constant(mir::Constant::Int(*n)),
                        _ => mir::Operand::Constant(mir::Constant::Int(0)),
                    };
                    block.instructions.push(mir::Instruction {
                        kind: mir::InstructionKind::RuntimeCall {
                            dest: dummy,
                            func: mir::RuntimeFunc::ListPush,
                            args: vec![mir::Operand::Local(list_local), elem_op],
                        },
                        span: None,
                    });
                }

                // Create iterator from list
                block.instructions.push(mir::Instruction {
                    kind: mir::InstructionKind::RuntimeCall {
                        dest: dest_local,
                        func: mir::RuntimeFunc::MakeIterator {
                            source: mir::IterSourceKind::List,
                            direction: mir::IterDirection::Forward,
                        },
                        args: vec![mir::Operand::Local(list_local)],
                    },
                    span: None,
                });

                Ok(mir::Operand::Local(dest_local))
            }
            hir::ExprKind::Var(var_id) => {
                // Handle variable references (e.g., iterating over a list variable)
                if let Some(&mir_local) = var_to_mir_local.get(var_id) {
                    // Variable is a parameter — use it directly
                    let var_type = self
                        .get_var_type(var_id)
                        .cloned()
                        .unwrap_or(Type::List(Box::new(Type::Any)));
                    let iter_source = match &var_type {
                        Type::List(_) => mir::IterSourceKind::List,
                        Type::Tuple(_) => mir::IterSourceKind::Tuple,
                        Type::Dict(_, _) => mir::IterSourceKind::Dict,
                        Type::Set(_) => mir::IterSourceKind::Set,
                        Type::Str => mir::IterSourceKind::Str,
                        _ => mir::IterSourceKind::List,
                    };
                    block.instructions.push(mir::Instruction {
                        kind: mir::InstructionKind::RuntimeCall {
                            dest: dest_local,
                            func: mir::RuntimeFunc::MakeIterator {
                                source: iter_source,
                                direction: mir::IterDirection::Forward,
                            },
                            args: vec![mir::Operand::Local(mir_local)],
                        },
                        span: None,
                    });
                    Ok(mir::Operand::Local(dest_local))
                } else if self.is_global(var_id) {
                    // Variable is from enclosing scope (global) — emit GlobalGet
                    let var_type = self
                        .get_var_type(var_id)
                        .cloned()
                        .unwrap_or(Type::List(Box::new(Type::Any)));
                    let runtime_func = self.get_global_get_func(&var_type);
                    let effective_var_id = self.get_effective_var_id(*var_id);

                    let global_local = self.alloc_and_add_local(var_type.clone(), mir_func);
                    block.instructions.push(mir::Instruction {
                        kind: mir::InstructionKind::RuntimeCall {
                            dest: global_local,
                            func: runtime_func,
                            args: vec![mir::Operand::Constant(mir::Constant::Int(
                                effective_var_id,
                            ))],
                        },
                        span: None,
                    });

                    // Create iterator from the fetched global variable
                    let iter_source = match &var_type {
                        Type::List(_) => mir::IterSourceKind::List,
                        Type::Tuple(_) => mir::IterSourceKind::Tuple,
                        Type::Dict(_, _) => mir::IterSourceKind::Dict,
                        Type::Set(_) => mir::IterSourceKind::Set,
                        Type::Str => mir::IterSourceKind::Str,
                        _ => mir::IterSourceKind::List,
                    };
                    block.instructions.push(mir::Instruction {
                        kind: mir::InstructionKind::RuntimeCall {
                            dest: dest_local,
                            func: mir::RuntimeFunc::MakeIterator {
                                source: iter_source,
                                direction: mir::IterDirection::Forward,
                            },
                            args: vec![mir::Operand::Local(global_local)],
                        },
                        span: None,
                    });
                    Ok(mir::Operand::Local(dest_local))
                } else {
                    Err(pyaot_diagnostics::CompilerError::codegen_error_at(
                        "unresolved variable in generator iterable expression",
                        expr.span,
                    ))
                }
            }
            _ => Err(pyaot_diagnostics::CompilerError::codegen_error_at(
                "unsupported expression kind as generator iterable",
                expr.span,
            )),
        }
    }
}
