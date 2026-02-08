//! Generic generator resume function generation
//!
//! This module handles generating the "resume" function for generators that
//! don't match the while-loop or for-loop patterns. It implements a generic
//! state machine that dispatches to different yield points based on state.

use std::collections::HashMap;

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{BlockId, FuncId, VarId};

use super::GeneratorVar;
use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Create the generator resume function
    /// This implements the state machine
    pub(super) fn create_generator_resume(
        &mut self,
        func: &hir::Function,
        hir_module: &hir::Module,
        gen_vars: &[GeneratorVar],
    ) -> Result<mir::Function> {
        // Reset local ID counter for this function
        self.reset_local_id();

        let _func_name = self.resolve(func.name).to_string();
        let resume_name = format!("{}$resume", self.resolve(func.name));

        // Resume function takes generator object as first parameter
        let gen_param_local = self.alloc_local_id();
        let params = vec![mir::Local {
            id: gen_param_local,
            name: None,
            ty: Type::Str, // Use Str as pointer placeholder (maps to I64)
            is_gc_root: false,
        }];

        // Return type is an i64 (either a boxed value or raw int)
        let return_type = Type::Int;

        // Create a new FuncId for the resume function
        let resume_func_id = FuncId(func.id.0 + 10000);

        let mut mir_func = mir::Function::new(
            resume_func_id,
            resume_name,
            params.clone(),
            return_type.clone(),
        );

        // Add parameters to locals
        for param in &params {
            mir_func.add_local(param.clone());
        }

        // Build a map from VarId to generator local index for ALL variables
        let var_to_gen_local: HashMap<VarId, u32> = gen_vars
            .iter()
            .map(|v| (v.var_id, v.gen_local_idx))
            .collect();

        // Check if this is a while-loop generator pattern:
        // [init_stmts...] while cond: yield val; [update_stmts...]
        if let Some(while_gen) = self.detect_while_loop_generator(&func.body, hir_module) {
            return self.create_while_loop_generator_resume(
                func,
                hir_module,
                gen_vars,
                &var_to_gen_local,
                while_gen,
                gen_param_local,
                mir_func,
            );
        }

        // Check if this is a for-loop generator pattern:
        // for x in iterable: yield expr
        if let Some(for_gen) = self.detect_for_loop_generator(&func.body, hir_module) {
            return self.create_for_loop_generator_resume(
                func,
                hir_module,
                gen_vars,
                &var_to_gen_local,
                for_gen,
                gen_param_local,
                mir_func,
            );
        }

        // For non-while/for generators, use the existing sequential approach
        // Collect yield information from the function body (with assignment targets)
        let yield_infos = self.collect_yield_info(&func.body, hir_module);
        let actual_yield_count = yield_infos.len();

        // Merge yield target variables into var_to_gen_local
        let mut var_to_gen_local = var_to_gen_local;
        let base_idx = gen_vars.len() as u32;
        for (i, info) in yield_infos.iter().enumerate() {
            if let Some(target) = info.assignment_target {
                var_to_gen_local
                    .entry(target)
                    .or_insert_with(|| base_idx + i as u32);
            }
        }

        // Block counter
        let mut next_block_id = 0u32;

        // Entry block: get state and dispatch
        let entry_block_id = BlockId::from(next_block_id);
        next_block_id += 1;
        let mut entry_block = mir::BasicBlock {
            id: entry_block_id,
            instructions: Vec::new(),
            terminator: mir::Terminator::Unreachable,
        };

        // Get current state
        let state_local = self.alloc_and_add_local(Type::Int, &mut mir_func);

        entry_block.instructions.push(mir::Instruction {
            kind: mir::InstructionKind::RuntimeCall {
                dest: state_local,
                func: mir::RuntimeFunc::GeneratorGetState,
                args: vec![mir::Operand::Local(gen_param_local)],
            },
        });

        // Check if exhausted
        let exhausted_local = self.alloc_and_add_local(Type::Bool, &mut mir_func);

        entry_block.instructions.push(mir::Instruction {
            kind: mir::InstructionKind::RuntimeCall {
                dest: exhausted_local,
                func: mir::RuntimeFunc::GeneratorIsExhausted,
                args: vec![mir::Operand::Local(gen_param_local)],
            },
        });

        // Exhausted block: return sentinel value (0) - runtime will handle StopIteration
        let exhausted_block_id = BlockId::from(next_block_id);
        next_block_id += 1;
        let exhausted_block = mir::BasicBlock {
            id: exhausted_block_id,
            instructions: vec![],
            terminator: mir::Terminator::Return(Some(mir::Operand::Constant(mir::Constant::Int(
                0,
            )))),
        };
        mir_func.blocks.insert(exhausted_block_id, exhausted_block);

        // Dispatch block: switch on state
        let dispatch_block_id = BlockId::from(next_block_id);
        next_block_id += 1;
        let mut dispatch_block = mir::BasicBlock {
            id: dispatch_block_id,
            instructions: Vec::new(),
            terminator: mir::Terminator::Unreachable,
        };

        // Create state blocks for each yield + one for exhausted
        let mut state_blocks = Vec::new();
        for _ in 0..=actual_yield_count {
            state_blocks.push(BlockId::from(next_block_id));
            next_block_id += 1;
        }

        // Branch: if exhausted goto exhausted_block, else goto dispatch
        entry_block.terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(exhausted_local),
            then_block: exhausted_block_id,
            else_block: dispatch_block_id,
        };
        mir_func.blocks.insert(entry_block_id, entry_block);

        // Create a temp local for comparison results
        let cmp_local = self.alloc_and_add_local(Type::Bool, &mut mir_func);

        // Dispatch block: chain of if-else for each state
        if actual_yield_count == 0 {
            // No yields - immediately exhausted
            dispatch_block.terminator = mir::Terminator::Goto(state_blocks[0]);
        } else {
            // For state 0, check if state == 0
            dispatch_block.instructions.push(mir::Instruction {
                kind: mir::InstructionKind::BinOp {
                    dest: cmp_local,
                    op: mir::BinOp::Eq,
                    left: mir::Operand::Local(state_local),
                    right: mir::Operand::Constant(mir::Constant::Int(0)),
                },
            });
            let else_block = if actual_yield_count > 1 {
                let next_dispatch = BlockId::from(next_block_id);
                next_block_id += 1;
                next_dispatch
            } else {
                state_blocks[1]
            };
            dispatch_block.terminator = mir::Terminator::Branch {
                cond: mir::Operand::Local(cmp_local),
                then_block: state_blocks[0],
                else_block,
            };
        }
        mir_func.blocks.insert(dispatch_block_id, dispatch_block);

        // Create additional dispatch blocks for states 1, 2, ...
        let mut current_else_block = if actual_yield_count > 1 {
            Some(BlockId::from(next_block_id - 1))
        } else {
            None
        };

        for state_idx in 1..actual_yield_count {
            if let Some(else_block_id) = current_else_block {
                let mut check_block = mir::BasicBlock {
                    id: else_block_id,
                    instructions: Vec::new(),
                    terminator: mir::Terminator::Unreachable,
                };

                check_block.instructions.push(mir::Instruction {
                    kind: mir::InstructionKind::BinOp {
                        dest: cmp_local,
                        op: mir::BinOp::Eq,
                        left: mir::Operand::Local(state_local),
                        right: mir::Operand::Constant(mir::Constant::Int(state_idx as i64)),
                    },
                });

                let next_else = if state_idx + 1 < actual_yield_count {
                    let b = BlockId::from(next_block_id);
                    next_block_id += 1;
                    current_else_block = Some(b);
                    b
                } else {
                    current_else_block = None;
                    state_blocks[actual_yield_count]
                };

                check_block.terminator = mir::Terminator::Branch {
                    cond: mir::Operand::Local(cmp_local),
                    then_block: state_blocks[state_idx],
                    else_block: next_else,
                };
                mir_func.blocks.insert(else_block_id, check_block);
            }
        }

        // Allocate a local for the sent value (used for states > 0)
        let sent_value_local = self.alloc_and_add_local(Type::Int, &mut mir_func);

        // Create state blocks that yield values
        for (i, yield_info) in yield_infos.iter().enumerate() {
            let state_block_id = state_blocks[i];
            let mut state_block = mir::BasicBlock {
                id: state_block_id,
                instructions: Vec::new(),
                terminator: mir::Terminator::Unreachable,
            };

            // For states > 0: load the sent value and store if there was an assignment target
            if i > 0 {
                let prev_yield = &yield_infos[i - 1];
                if let Some(target) = prev_yield.assignment_target {
                    if let Some(&gen_local_idx) = var_to_gen_local.get(&target) {
                        state_block.instructions.push(mir::Instruction {
                            kind: mir::InstructionKind::RuntimeCall {
                                dest: sent_value_local,
                                func: mir::RuntimeFunc::GeneratorGetSentValue,
                                args: vec![mir::Operand::Local(gen_param_local)],
                            },
                        });

                        state_block.instructions.push(mir::Instruction {
                            kind: mir::InstructionKind::RuntimeCall {
                                dest: state_local,
                                func: mir::RuntimeFunc::GeneratorSetLocal,
                                args: vec![
                                    mir::Operand::Local(gen_param_local),
                                    mir::Operand::Constant(mir::Constant::Int(
                                        gen_local_idx as i64,
                                    )),
                                    mir::Operand::Local(sent_value_local),
                                ],
                            },
                        });
                    }
                }
            }

            // Lower the yield value expression
            let value_operand = match &yield_info.yield_value {
                Some(expr_id) => {
                    let expr = &hir_module.exprs[*expr_id];
                    match &expr.kind {
                        hir::ExprKind::Int(n) => mir::Operand::Constant(mir::Constant::Int(*n)),
                        hir::ExprKind::Float(f) => mir::Operand::Constant(mir::Constant::Float(*f)),
                        hir::ExprKind::Bool(b) => {
                            mir::Operand::Constant(mir::Constant::Int(if *b { 1 } else { 0 }))
                        }
                        hir::ExprKind::None => mir::Operand::Constant(mir::Constant::Int(0)),
                        hir::ExprKind::Var(var_id) => {
                            // Load from generator locals (includes params and assigned vars)
                            if let Some(&gen_local_idx) = var_to_gen_local.get(var_id) {
                                let var_local = self.alloc_and_add_local(Type::Int, &mut mir_func);
                                state_block.instructions.push(mir::Instruction {
                                    kind: mir::InstructionKind::RuntimeCall {
                                        dest: var_local,
                                        func: mir::RuntimeFunc::GeneratorGetLocal,
                                        args: vec![
                                            mir::Operand::Local(gen_param_local),
                                            mir::Operand::Constant(mir::Constant::Int(
                                                gen_local_idx as i64,
                                            )),
                                        ],
                                    },
                                });
                                mir::Operand::Local(var_local)
                            } else {
                                mir::Operand::Constant(mir::Constant::Int(0))
                            }
                        }
                        _ => mir::Operand::Constant(mir::Constant::Int(0)),
                    }
                }
                None => mir::Operand::Constant(mir::Constant::Int(0)),
            };

            // Set next state
            let next_state = (i + 1) as i64;
            state_block.instructions.push(mir::Instruction {
                kind: mir::InstructionKind::RuntimeCall {
                    dest: state_local,
                    func: mir::RuntimeFunc::GeneratorSetState,
                    args: vec![
                        mir::Operand::Local(gen_param_local),
                        mir::Operand::Constant(mir::Constant::Int(next_state)),
                    ],
                },
            });

            // Return the yielded value
            state_block.terminator = mir::Terminator::Return(Some(value_operand));

            mir_func.blocks.insert(state_block_id, state_block);
        }

        // Final state block: set exhausted
        let final_state_id = state_blocks[actual_yield_count];
        let mut final_state_block = mir::BasicBlock {
            id: final_state_id,
            instructions: Vec::new(),
            terminator: mir::Terminator::Unreachable,
        };

        final_state_block.instructions.push(mir::Instruction {
            kind: mir::InstructionKind::RuntimeCall {
                dest: state_local,
                func: mir::RuntimeFunc::GeneratorSetExhausted,
                args: vec![mir::Operand::Local(gen_param_local)],
            },
        });

        // Return 0 (sentinel) - runtime will raise StopIteration
        final_state_block.terminator =
            mir::Terminator::Return(Some(mir::Operand::Constant(mir::Constant::Int(0))));
        mir_func.blocks.insert(final_state_id, final_state_block);

        mir_func.entry_block = entry_block_id;

        Ok(mir_func)
    }
}
