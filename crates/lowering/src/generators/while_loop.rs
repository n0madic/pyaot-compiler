//! While-loop generator pattern detection and resume generation
//!
//! This module handles generators that follow the while-loop pattern:
//! ```python
//! def gen():
//!     i = 0
//!     while i < n:
//!         yield i
//!         i = i + 1
//! ```

use std::collections::HashMap;

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{BlockId, LocalId, VarId};

use super::{GeneratorVar, WhileLoopGenerator, YieldSection};
use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Detect if the generator body follows the pattern:
    /// [init_stmts...] while cond: yield val; [update_stmts...]
    pub(super) fn detect_while_loop_generator(
        &self,
        body: &[hir::StmtId],
        hir_module: &hir::Module,
    ) -> Option<WhileLoopGenerator> {
        // Find the while loop
        let mut init_stmts = Vec::new();
        let mut while_stmt_idx = None;

        for (i, stmt_id) in body.iter().enumerate() {
            let stmt = &hir_module.stmts[*stmt_id];
            if matches!(stmt.kind, hir::StmtKind::While { .. }) {
                while_stmt_idx = Some(i);
                break;
            }
            init_stmts.push(*stmt_id);
        }

        let while_idx = while_stmt_idx?;
        let while_stmt_id = body[while_idx];
        let while_stmt = &hir_module.stmts[while_stmt_id];

        let (cond, while_body) = match &while_stmt.kind {
            hir::StmtKind::While { cond, body, .. } => (*cond, body),
            _ => return None,
        };

        // Find all yields in while body and split into sections
        let mut yield_sections = Vec::new();
        let mut current_stmts = Vec::new();

        for stmt_id in while_body {
            let stmt = &hir_module.stmts[*stmt_id];
            match &stmt.kind {
                hir::StmtKind::Expr(expr_id) => {
                    let expr = &hir_module.exprs[*expr_id];
                    if let hir::ExprKind::Yield(val) = &expr.kind {
                        // Found a yield - save current section
                        yield_sections.push(YieldSection {
                            stmts_before: current_stmts.clone(),
                            yield_expr: *val,
                        });
                        current_stmts.clear();
                    } else {
                        current_stmts.push(*stmt_id);
                    }
                }
                _ => {
                    current_stmts.push(*stmt_id);
                }
            }
        }

        // Statements after last yield become update section
        let update_stmts = current_stmts;

        if yield_sections.is_empty() {
            return None;
        }

        // Make sure there's nothing after the while loop
        if while_idx + 1 < body.len() {
            return None;
        }

        Some(WhileLoopGenerator {
            init_stmts,
            cond,
            yield_sections,
            update_stmts,
        })
    }

    /// Create a resume function for a while-loop generator
    /// Structure:
    /// - State 0: execute init, check cond, goto first yield state or exhausted
    /// - State 1..N: execute statements before yield, yield value, set next state
    /// - State N+1: execute update, check cond, loop back to State 1 or exhausted
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_while_loop_generator_resume(
        &mut self,
        _func: &hir::Function,
        hir_module: &hir::Module,
        gen_vars: &[GeneratorVar],
        _var_to_gen_local: &HashMap<VarId, u32>,
        while_gen: WhileLoopGenerator,
        gen_param_local: LocalId,
        mut mir_func: mir::Function,
    ) -> Result<mir::Function> {
        let mut next_block_id = 0u32;
        let num_yields = while_gen.yield_sections.len();

        // Allocate MIR locals for each generator variable
        let mut var_to_mir_local: HashMap<VarId, LocalId> = HashMap::new();
        for gen_var in gen_vars {
            let local_id = self.alloc_and_add_local(gen_var.ty.clone(), &mut mir_func);
            var_to_mir_local.insert(gen_var.var_id, local_id);
        }

        // Allocate helper locals
        let state_local = self.alloc_and_add_local(Type::Int, &mut mir_func);
        let cmp_local = self.alloc_and_add_local(Type::Bool, &mut mir_func);
        let exhausted_local = self.alloc_and_add_local(Type::Bool, &mut mir_func);
        let dummy_local = self.alloc_and_add_local(Type::Int, &mut mir_func);

        // Entry block
        let entry_block_id = BlockId::from(next_block_id);
        next_block_id += 1;
        let mut entry_block = mir::BasicBlock {
            id: entry_block_id,
            instructions: Vec::new(),
            terminator: mir::Terminator::Unreachable,
        };

        // Get state
        entry_block.instructions.push(mir::Instruction {
            kind: mir::InstructionKind::RuntimeCall {
                dest: state_local,
                func: mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_GENERATOR_GET_STATE,
                ),
                args: vec![mir::Operand::Local(gen_param_local)],
            },
            span: None,
        });

        // Check exhausted
        entry_block.instructions.push(mir::Instruction {
            kind: mir::InstructionKind::RuntimeCall {
                dest: exhausted_local,
                func: mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_GENERATOR_IS_EXHAUSTED,
                ),
                args: vec![mir::Operand::Local(gen_param_local)],
            },
            span: None,
        });

        // Exhausted block
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

        // State 0 block (init)
        let state0_block_id = BlockId::from(next_block_id);
        next_block_id += 1;

        // Allocate blocks for each yield state (1..=N)
        let mut yield_state_blocks = Vec::new();
        for _ in 0..num_yields {
            yield_state_blocks.push(BlockId::from(next_block_id));
            next_block_id += 1;
        }

        // Allocate update block (State N+1)
        let update_block_id = BlockId::from(next_block_id);
        next_block_id += 1;

        // Mark exhausted block (shared)
        let mark_exhausted_block_id = BlockId::from(next_block_id);
        next_block_id += 1;
        let mut mark_exhausted_block = mir::BasicBlock {
            id: mark_exhausted_block_id,
            instructions: Vec::new(),
            terminator: mir::Terminator::Unreachable,
        };
        mark_exhausted_block.instructions.push(mir::Instruction {
            kind: mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_GENERATOR_SET_EXHAUSTED,
                ),
                args: vec![mir::Operand::Local(gen_param_local)],
            },
            span: None,
        });
        mark_exhausted_block.terminator =
            mir::Terminator::Return(Some(mir::Operand::Constant(mir::Constant::Int(0))));
        mir_func
            .blocks
            .insert(mark_exhausted_block_id, mark_exhausted_block);

        // Dispatch block - builds a chain of comparisons for all states
        let dispatch_block_id = BlockId::from(next_block_id);
        next_block_id += 1;

        // Build dispatch chain: state==0 ? state0 : (state==1 ? yield1 : (state==2 ? yield2 : ...))
        // We'll use a series of blocks to create this chain
        let mut dispatch_blocks = Vec::new();
        let first_dispatch_id = dispatch_block_id;

        // Create dispatch blocks for each state
        for i in 0..=num_yields {
            let dispatch_id = if i == 0 {
                dispatch_block_id
            } else {
                let id = BlockId::from(next_block_id);
                next_block_id += 1;
                id
            };
            dispatch_blocks.push(dispatch_id);
        }

        // Build dispatch chain
        for i in 0..=num_yields {
            let mut dispatch_block = mir::BasicBlock {
                id: dispatch_blocks[i],
                instructions: Vec::new(),
                terminator: mir::Terminator::Unreachable,
            };

            let state_value = i as i64;
            let target_block = if i == 0 {
                state0_block_id
            } else if i <= num_yields {
                yield_state_blocks[i - 1]
            } else {
                update_block_id
            };

            let next_dispatch = if i < num_yields {
                dispatch_blocks[i + 1]
            } else {
                update_block_id
            };

            dispatch_block.instructions.push(mir::Instruction {
                kind: mir::InstructionKind::BinOp {
                    dest: cmp_local,
                    op: mir::BinOp::Eq,
                    left: mir::Operand::Local(state_local),
                    right: mir::Operand::Constant(mir::Constant::Int(state_value)),
                },
                span: None,
            });

            dispatch_block.terminator = mir::Terminator::Branch {
                cond: mir::Operand::Local(cmp_local),
                then_block: target_block,
                else_block: next_dispatch,
            };

            mir_func.blocks.insert(dispatch_blocks[i], dispatch_block);
        }

        // Entry: if exhausted goto exhausted, else dispatch
        entry_block.terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(exhausted_local),
            then_block: exhausted_block_id,
            else_block: first_dispatch_id,
        };
        mir_func.blocks.insert(entry_block_id, entry_block);

        // ===== State 0: Initialize and check condition =====
        let mut state0_block = mir::BasicBlock {
            id: state0_block_id,
            instructions: Vec::new(),
            terminator: mir::Terminator::Unreachable,
        };

        // Load all parameters from generator locals
        for gen_var in gen_vars {
            if gen_var.is_param {
                if let Some(&mir_local) = var_to_mir_local.get(&gen_var.var_id) {
                    state0_block.instructions.push(mir::Instruction {
                        kind: mir::InstructionKind::RuntimeCall {
                            dest: mir_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_GENERATOR_GET_LOCAL,
                            ),
                            args: vec![
                                mir::Operand::Local(gen_param_local),
                                mir::Operand::Constant(mir::Constant::Int(
                                    gen_var.gen_local_idx as i64,
                                )),
                            ],
                        },
                        span: None,
                    });
                }
            }
        }

        // Execute init statements
        for stmt_id in &while_gen.init_stmts {
            self.lower_simple_stmt_for_generator(
                *stmt_id,
                hir_module,
                &mut state0_block,
                &var_to_mir_local,
            )?;
        }

        // Evaluate condition
        let cond_result_local = self.evaluate_while_condition(
            while_gen.cond,
            hir_module,
            &mut state0_block,
            &mut mir_func,
            &var_to_mir_local,
        )?;

        // Save variables before branching
        for gen_var in gen_vars {
            if let Some(&mir_local) = var_to_mir_local.get(&gen_var.var_id) {
                state0_block.instructions.push(mir::Instruction {
                    kind: mir::InstructionKind::RuntimeCall {
                        dest: dummy_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_GENERATOR_SET_LOCAL,
                        ),
                        args: vec![
                            mir::Operand::Local(gen_param_local),
                            mir::Operand::Constant(mir::Constant::Int(
                                gen_var.gen_local_idx as i64,
                            )),
                            mir::Operand::Local(mir_local),
                        ],
                    },
                    span: None,
                });
            }
        }

        // State 0: if condition true, goto first yield state, else exhausted
        state0_block.terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(cond_result_local),
            then_block: yield_state_blocks[0],
            else_block: mark_exhausted_block_id,
        };
        mir_func.blocks.insert(state0_block_id, state0_block);

        // ===== States 1..N: Each yield state =====
        for (i, section) in while_gen.yield_sections.iter().enumerate() {
            let mut cur_block_id = yield_state_blocks[i];
            let mut yield_block = mir::BasicBlock {
                id: cur_block_id,
                instructions: Vec::new(),
                terminator: mir::Terminator::Unreachable,
            };

            // Load all variables from generator locals
            for gen_var in gen_vars {
                if let Some(&mir_local) = var_to_mir_local.get(&gen_var.var_id) {
                    yield_block.instructions.push(mir::Instruction {
                        kind: mir::InstructionKind::RuntimeCall {
                            dest: mir_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_GENERATOR_GET_LOCAL,
                            ),
                            args: vec![
                                mir::Operand::Local(gen_param_local),
                                mir::Operand::Constant(mir::Constant::Int(
                                    gen_var.gen_local_idx as i64,
                                )),
                            ],
                        },
                        span: None,
                    });
                }
            }

            // Execute statements before this yield
            for stmt_id in &section.stmts_before {
                self.lower_simple_stmt_for_generator(
                    *stmt_id,
                    hir_module,
                    &mut yield_block,
                    &var_to_mir_local,
                )?;
            }

            // Compute yield value
            let yield_value_local = self.alloc_and_add_local(Type::Int, &mut mir_func);

            if let Some(yield_expr_id) = section.yield_expr {
                let yield_expr = &hir_module.exprs[yield_expr_id];
                match &yield_expr.kind {
                    hir::ExprKind::Var(var_id) => {
                        if let Some(&src_mir_local) = var_to_mir_local.get(var_id) {
                            yield_block.instructions.push(mir::Instruction {
                                kind: mir::InstructionKind::Copy {
                                    dest: yield_value_local,
                                    src: mir::Operand::Local(src_mir_local),
                                },
                                span: None,
                            });
                        }
                    }
                    hir::ExprKind::Int(n) => {
                        yield_block.instructions.push(mir::Instruction {
                            kind: mir::InstructionKind::Copy {
                                dest: yield_value_local,
                                src: mir::Operand::Constant(mir::Constant::Int(*n)),
                            },
                            span: None,
                        });
                    }
                    hir::ExprKind::BinOp { left, op, right } => {
                        let left_expr = &hir_module.exprs[*left];
                        let right_expr = &hir_module.exprs[*right];
                        let left_op = self.get_operand_for_expr(left_expr, &var_to_mir_local)?;
                        let right_op = self.get_operand_for_expr(right_expr, &var_to_mir_local)?;
                        let mir_op = match op {
                            hir::BinOp::Add => mir::BinOp::Add,
                            hir::BinOp::Sub => mir::BinOp::Sub,
                            hir::BinOp::Mul => mir::BinOp::Mul,
                            hir::BinOp::Div => mir::BinOp::Div,
                            hir::BinOp::Mod => mir::BinOp::Mod,
                            hir::BinOp::FloorDiv => mir::BinOp::FloorDiv,
                            hir::BinOp::Pow => mir::BinOp::Pow,
                            hir::BinOp::BitAnd => mir::BinOp::BitAnd,
                            hir::BinOp::BitOr => mir::BinOp::BitOr,
                            hir::BinOp::BitXor => mir::BinOp::BitXor,
                            hir::BinOp::LShift => mir::BinOp::LShift,
                            hir::BinOp::RShift => mir::BinOp::RShift,
                            hir::BinOp::MatMul => {
                                return Err(pyaot_diagnostics::CompilerError::type_error(
                                    "@ operator is only supported on classes with __matmul__"
                                        .to_string(),
                                    yield_expr.span,
                                ));
                            }
                        };
                        yield_block.instructions.push(mir::Instruction {
                            kind: mir::InstructionKind::BinOp {
                                dest: yield_value_local,
                                op: mir_op,
                                left: left_op,
                                right: right_op,
                            },
                            span: None,
                        });
                    }
                    hir::ExprKind::IfExpr {
                        cond,
                        then_val,
                        else_val,
                    } => {
                        // Evaluate condition into a bool local
                        let cond_local = self.alloc_and_add_local(Type::Bool, &mut mir_func);
                        let cond_expr = &hir_module.exprs[*cond];

                        if let hir::ExprKind::Compare { left, op, right } = &cond_expr.kind {
                            let l_expr = &hir_module.exprs[*left];
                            let r_expr = &hir_module.exprs[*right];
                            let l_op = self.get_operand_for_expr(l_expr, &var_to_mir_local)?;
                            let r_op = self.get_operand_for_expr(r_expr, &var_to_mir_local)?;
                            let mir_cmp = match op {
                                hir::CmpOp::Lt => mir::BinOp::Lt,
                                hir::CmpOp::LtE => mir::BinOp::LtE,
                                hir::CmpOp::Gt => mir::BinOp::Gt,
                                hir::CmpOp::GtE => mir::BinOp::GtE,
                                hir::CmpOp::Eq => mir::BinOp::Eq,
                                hir::CmpOp::NotEq => mir::BinOp::NotEq,
                                _ => mir::BinOp::Eq,
                            };
                            yield_block.instructions.push(mir::Instruction {
                                kind: mir::InstructionKind::BinOp {
                                    dest: cond_local,
                                    op: mir_cmp,
                                    left: l_op,
                                    right: r_op,
                                },
                                span: None,
                            });
                        } else if let hir::ExprKind::Var(var_id) = &cond_expr.kind {
                            if let Some(&mir_local) = var_to_mir_local.get(var_id) {
                                yield_block.instructions.push(mir::Instruction {
                                    kind: mir::InstructionKind::Copy {
                                        dest: cond_local,
                                        src: mir::Operand::Local(mir_local),
                                    },
                                    span: None,
                                });
                            }
                        } else if let hir::ExprKind::Bool(b) = &cond_expr.kind {
                            yield_block.instructions.push(mir::Instruction {
                                kind: mir::InstructionKind::Copy {
                                    dest: cond_local,
                                    src: mir::Operand::Constant(mir::Constant::Bool(*b)),
                                },
                                span: None,
                            });
                        }

                        // Allocate then / else / merge blocks
                        let then_bb_id = BlockId::from(next_block_id);
                        next_block_id += 1;
                        let else_bb_id = BlockId::from(next_block_id);
                        next_block_id += 1;
                        let merge_bb_id = BlockId::from(next_block_id);
                        next_block_id += 1;

                        // yield_block terminates with Branch on condition
                        yield_block.terminator = mir::Terminator::Branch {
                            cond: mir::Operand::Local(cond_local),
                            then_block: then_bb_id,
                            else_block: else_bb_id,
                        };
                        mir_func.blocks.insert(cur_block_id, yield_block);

                        // Then block: yield_value = then_val → Goto merge
                        let then_expr = &hir_module.exprs[*then_val];
                        let then_op = self.get_operand_for_expr(then_expr, &var_to_mir_local)?;
                        let then_bb = mir::BasicBlock {
                            id: then_bb_id,
                            instructions: vec![mir::Instruction {
                                kind: mir::InstructionKind::Copy {
                                    dest: yield_value_local,
                                    src: then_op,
                                },
                                span: None,
                            }],
                            terminator: mir::Terminator::Goto(merge_bb_id),
                        };
                        mir_func.blocks.insert(then_bb_id, then_bb);

                        // Else block: yield_value = else_val → Goto merge
                        let else_expr = &hir_module.exprs[*else_val];
                        let else_op = self.get_operand_for_expr(else_expr, &var_to_mir_local)?;
                        let else_bb = mir::BasicBlock {
                            id: else_bb_id,
                            instructions: vec![mir::Instruction {
                                kind: mir::InstructionKind::Copy {
                                    dest: yield_value_local,
                                    src: else_op,
                                },
                                span: None,
                            }],
                            terminator: mir::Terminator::Goto(merge_bb_id),
                        };
                        mir_func.blocks.insert(else_bb_id, else_bb);

                        // Replace yield_block with the merge block for continuation
                        yield_block = mir::BasicBlock {
                            id: merge_bb_id,
                            instructions: Vec::new(),
                            terminator: mir::Terminator::Unreachable,
                        };
                        cur_block_id = merge_bb_id;
                    }
                    _ => {
                        yield_block.instructions.push(mir::Instruction {
                            kind: mir::InstructionKind::Copy {
                                dest: yield_value_local,
                                src: mir::Operand::Constant(mir::Constant::Int(0)),
                            },
                            span: None,
                        });
                    }
                }
            } else {
                // yield without value = yield None (represented as 0)
                yield_block.instructions.push(mir::Instruction {
                    kind: mir::InstructionKind::Copy {
                        dest: yield_value_local,
                        src: mir::Operand::Constant(mir::Constant::Int(0)),
                    },
                    span: None,
                });
            }

            // Save all variables to generator locals
            for gen_var in gen_vars {
                if let Some(&mir_local) = var_to_mir_local.get(&gen_var.var_id) {
                    yield_block.instructions.push(mir::Instruction {
                        kind: mir::InstructionKind::RuntimeCall {
                            dest: dummy_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_GENERATOR_SET_LOCAL,
                            ),
                            args: vec![
                                mir::Operand::Local(gen_param_local),
                                mir::Operand::Constant(mir::Constant::Int(
                                    gen_var.gen_local_idx as i64,
                                )),
                                mir::Operand::Local(mir_local),
                            ],
                        },
                        span: None,
                    });
                }
            }

            // Set next state
            let next_state = if i < num_yields - 1 {
                (i + 2) as i64 // Next yield state (states are 0, 1, 2, ..., N, N+1)
            } else {
                (num_yields + 1) as i64 // Update state
            };

            yield_block.instructions.push(mir::Instruction {
                kind: mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_GENERATOR_SET_STATE,
                    ),
                    args: vec![
                        mir::Operand::Local(gen_param_local),
                        mir::Operand::Constant(mir::Constant::Int(next_state)),
                    ],
                },
                span: None,
            });

            // Return yield value
            yield_block.terminator =
                mir::Terminator::Return(Some(mir::Operand::Local(yield_value_local)));
            mir_func.blocks.insert(cur_block_id, yield_block);
        }

        // ===== State N+1: Update and loop back =====
        let mut update_block = mir::BasicBlock {
            id: update_block_id,
            instructions: Vec::new(),
            terminator: mir::Terminator::Unreachable,
        };

        // Load all variables from generator locals
        for gen_var in gen_vars {
            if let Some(&mir_local) = var_to_mir_local.get(&gen_var.var_id) {
                update_block.instructions.push(mir::Instruction {
                    kind: mir::InstructionKind::RuntimeCall {
                        dest: mir_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_GENERATOR_GET_LOCAL,
                        ),
                        args: vec![
                            mir::Operand::Local(gen_param_local),
                            mir::Operand::Constant(mir::Constant::Int(
                                gen_var.gen_local_idx as i64,
                            )),
                        ],
                    },
                    span: None,
                });
            }
        }

        // Execute update statements (after last yield)
        for stmt_id in &while_gen.update_stmts {
            self.lower_simple_stmt_for_generator(
                *stmt_id,
                hir_module,
                &mut update_block,
                &var_to_mir_local,
            )?;
        }

        // Re-evaluate condition
        let cond_result_local2 = self.evaluate_while_condition(
            while_gen.cond,
            hir_module,
            &mut update_block,
            &mut mir_func,
            &var_to_mir_local,
        )?;

        // Save variables
        for gen_var in gen_vars {
            if let Some(&mir_local) = var_to_mir_local.get(&gen_var.var_id) {
                update_block.instructions.push(mir::Instruction {
                    kind: mir::InstructionKind::RuntimeCall {
                        dest: dummy_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_GENERATOR_SET_LOCAL,
                        ),
                        args: vec![
                            mir::Operand::Local(gen_param_local),
                            mir::Operand::Constant(mir::Constant::Int(
                                gen_var.gen_local_idx as i64,
                            )),
                            mir::Operand::Local(mir_local),
                        ],
                    },
                    span: None,
                });
            }
        }

        // CRITICAL: Loop back to first yield state if condition is true
        update_block.terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(cond_result_local2),
            then_block: yield_state_blocks[0], // Jump back to State 1!
            else_block: mark_exhausted_block_id,
        };
        mir_func.blocks.insert(update_block_id, update_block);

        mir_func.entry_block = entry_block_id;

        Ok(mir_func)
    }
}
