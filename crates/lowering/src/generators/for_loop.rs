//! For-loop generator pattern detection and resume generation
//!
//! This module handles generators that follow the for-loop pattern:
//! ```python
//! def gen():
//!     for x in iterable:
//!         yield x
//! ```
//! Or with a filter condition:
//! ```python
//! def gen():
//!     for x in iterable:
//!         if cond:
//!             yield x
//! ```

use std::collections::HashMap;

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{BlockId, LocalId, VarId};

use super::{ForLoopGenerator, GeneratorVar};
use crate::context::Lowering;
use crate::utils::get_iterable_info;

impl<'a> Lowering<'a> {
    /// Detect a for-loop generator pattern:
    /// for x in iterable:
    ///     yield expr
    /// or:
    /// for x in iterable:
    ///     if cond:
    ///         yield expr
    pub(super) fn detect_for_loop_generator(
        &self,
        body: &[hir::StmtId],
        hir_module: &hir::Module,
    ) -> Option<ForLoopGenerator> {
        // Body must start with a for loop (at least 1 statement)
        if body.is_empty() {
            return None;
        }

        let stmt = &hir_module.stmts[body[0]];
        let (target_var, iter_expr, for_body) = match &stmt.kind {
            hir::StmtKind::For {
                target, iter, body, ..
            } => (*target, *iter, body),
            _ => return None,
        };

        // For body should contain exactly one statement
        if for_body.len() != 1 {
            return None;
        }

        let first_stmt = &hir_module.stmts[for_body[0]];

        let (yield_expr, filter_cond) = {
            // Case 1: Direct yield statement
            if let hir::StmtKind::Expr(expr_id) = &first_stmt.kind {
                let expr = &hir_module.exprs[*expr_id];
                if let hir::ExprKind::Yield(val) = &expr.kind {
                    (*val, None)
                } else {
                    return None;
                }
            }
            // Case 2: If statement wrapping yield (filter condition)
            else if let hir::StmtKind::If {
                cond,
                then_block,
                else_block,
            } = &first_stmt.kind
            {
                if !else_block.is_empty() || then_block.len() != 1 {
                    return None;
                }
                let yield_stmt = &hir_module.stmts[then_block[0]];
                if let hir::StmtKind::Expr(expr_id) = &yield_stmt.kind {
                    let expr = &hir_module.exprs[*expr_id];
                    if let hir::ExprKind::Yield(val) = &expr.kind {
                        (*val, Some(*cond))
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            } else {
                return None;
            }
        };

        // Collect trailing yield statements after the for-loop
        let mut trailing_yields = Vec::new();
        for &stmt_id in &body[1..] {
            let trailing_stmt = &hir_module.stmts[stmt_id];
            if let hir::StmtKind::Expr(expr_id) = &trailing_stmt.kind {
                let expr = &hir_module.exprs[*expr_id];
                if let hir::ExprKind::Yield(val) = &expr.kind {
                    trailing_yields.push(*val);
                } else {
                    // Non-yield statement after for-loop — can't use this pattern
                    return None;
                }
            } else {
                return None;
            }
        }

        Some(ForLoopGenerator {
            target_var,
            iter_expr,
            yield_expr,
            filter_cond,
            trailing_yields,
        })
    }

    /// Create a resume function for a for-loop generator
    /// Structure:
    /// - State 0: Create iterator (from stored iterable), call next, yield or exhaust
    /// - State 1: Call next on iterator, yield or transition to trailing yields
    /// - State 2..N: Trailing yield states (from `yield from X; yield Y; yield Z`)
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_for_loop_generator_resume(
        &mut self,
        _func: &hir::Function,
        hir_module: &hir::Module,
        gen_vars: &[GeneratorVar],
        var_to_gen_local: &HashMap<VarId, u32>,
        for_gen: ForLoopGenerator,
        gen_param_local: LocalId,
        mut mir_func: mir::Function,
    ) -> Result<mir::Function> {
        let mut next_block_id = 0u32;
        let num_trailing = for_gen.trailing_yields.len();

        // Allocate MIR locals for generator variables
        let mut var_to_mir_local: HashMap<VarId, LocalId> = HashMap::new();
        for gen_var in gen_vars {
            let local_id = self.alloc_and_add_local(gen_var.ty.clone(), &mut mir_func);
            var_to_mir_local.insert(gen_var.var_id, local_id);
        }

        // Determine the element type of the iterable so we can correctly type the
        // loop variable (e.g. Class { .. } when iterating over list[MyClass]).
        // This is used when the yield expression accesses a field on the loop var.
        let iter_expr_ty = self.get_type_of_expr_id(for_gen.iter_expr, hir_module);
        let loop_var_elem_ty = get_iterable_info(&iter_expr_ty)
            .map(|(_kind, ty)| ty)
            .unwrap_or(Type::Any);

        // Also allocate a local for the loop variable (target_var) if not already in gen_vars
        let target_mir_local = if let Some(&existing) = var_to_mir_local.get(&for_gen.target_var) {
            existing
        } else {
            // Use the element type for the local so that InstanceGetField gets the
            // correct pointer-sized allocation (all class instances are heap pointers).
            let local_ty = if matches!(loop_var_elem_ty, Type::Class { .. }) {
                loop_var_elem_ty.clone()
            } else {
                Type::Int
            };
            let local_id = self.alloc_and_add_local(local_ty, &mut mir_func);
            var_to_mir_local.insert(for_gen.target_var, local_id);
            local_id
        };

        // Allocate helper locals
        let state_local = self.alloc_and_add_local(Type::Int, &mut mir_func);
        let cmp_local = self.alloc_and_add_local(Type::Bool, &mut mir_func);
        let exhausted_local = self.alloc_and_add_local(Type::Bool, &mut mir_func);
        let dummy_local = self.alloc_and_add_local(Type::Int, &mut mir_func);

        // Local for the iterator object (loaded from generator state)
        let iter_local =
            self.alloc_and_add_local(Type::Iterator(Box::new(Type::Any)), &mut mir_func);

        // Local for next() result (the value from iterator)
        let next_value_local = self.alloc_and_add_local(Type::Int, &mut mir_func);

        // Local to check if iterator is done
        let iter_done_local = self.alloc_and_add_local(Type::Bool, &mut mir_func);

        // ===== Allocate block IDs =====
        let entry_block_id = BlockId::from(next_block_id);
        next_block_id += 1;
        let dispatch_block_id = BlockId::from(next_block_id);
        next_block_id += 1;
        let mark_exhausted_block_id = BlockId::from(next_block_id);
        next_block_id += 1;
        let state0_block_id = BlockId::from(next_block_id);
        next_block_id += 1;
        let state1_block_id = BlockId::from(next_block_id);
        next_block_id += 1;
        let iter_next_block_id = BlockId::from(next_block_id);
        next_block_id += 1;
        let check_iter_done_block_id = BlockId::from(next_block_id);
        next_block_id += 1;
        let yield_block_id = BlockId::from(next_block_id);
        next_block_id += 1;

        // Allocate block IDs for trailing yield states
        let mut trailing_block_ids = Vec::new();
        for _ in 0..num_trailing {
            trailing_block_ids.push(BlockId::from(next_block_id));
            next_block_id += 1;
        }

        // ===== Entry block: Get state and check if exhausted =====
        let mut entry_block = mir::BasicBlock {
            id: entry_block_id,
            instructions: Vec::new(),
            terminator: mir::Terminator::Unreachable,
        };

        entry_block.instructions.push(mir::Instruction {
            kind: mir::InstructionKind::RuntimeCall {
                dest: state_local,
                func: mir::RuntimeFunc::GeneratorGetState,
                args: vec![mir::Operand::Local(gen_param_local)],
            },
        });

        entry_block.instructions.push(mir::Instruction {
            kind: mir::InstructionKind::RuntimeCall {
                dest: exhausted_local,
                func: mir::RuntimeFunc::GeneratorIsExhausted,
                args: vec![mir::Operand::Local(gen_param_local)],
            },
        });

        entry_block.terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(exhausted_local),
            then_block: mark_exhausted_block_id,
            else_block: dispatch_block_id,
        };
        mir_func.blocks.insert(entry_block_id, entry_block);

        // ===== Dispatch: Chain of state comparisons =====
        // Build state targets: [state0, state1, trailing0, trailing1, ..., mark_exhausted]
        let mut state_targets: Vec<BlockId> = vec![state0_block_id, state1_block_id];
        state_targets.extend_from_slice(&trailing_block_ids);

        // Build dispatch chain: state==0 → state0, state==1 → state1, ...
        let mut current_dispatch_id = dispatch_block_id;
        for (i, &target_id) in state_targets.iter().enumerate() {
            let mut check_block = mir::BasicBlock {
                id: current_dispatch_id,
                instructions: Vec::new(),
                terminator: mir::Terminator::Unreachable,
            };

            check_block.instructions.push(mir::Instruction {
                kind: mir::InstructionKind::BinOp {
                    dest: cmp_local,
                    op: mir::BinOp::Eq,
                    left: mir::Operand::Local(state_local),
                    right: mir::Operand::Constant(mir::Constant::Int(i as i64)),
                },
            });

            let else_block = if i + 1 < state_targets.len() {
                let next_check = BlockId::from(next_block_id);
                next_block_id += 1;
                next_check
            } else {
                // Last state — fall through to mark_exhausted
                mark_exhausted_block_id
            };

            check_block.terminator = mir::Terminator::Branch {
                cond: mir::Operand::Local(cmp_local),
                then_block: target_id,
                else_block,
            };
            mir_func.blocks.insert(current_dispatch_id, check_block);
            current_dispatch_id = else_block;
        }

        // ===== Mark exhausted block =====
        let mut mark_exhausted_block = mir::BasicBlock {
            id: mark_exhausted_block_id,
            instructions: Vec::new(),
            terminator: mir::Terminator::Unreachable,
        };

        mark_exhausted_block.instructions.push(mir::Instruction {
            kind: mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::GeneratorSetExhausted,
                args: vec![mir::Operand::Local(gen_param_local)],
            },
        });

        mark_exhausted_block.terminator =
            mir::Terminator::Return(Some(mir::Operand::Constant(mir::Constant::Int(0))));
        mir_func
            .blocks
            .insert(mark_exhausted_block_id, mark_exhausted_block);

        // ===== State 0: Initialize iterator from stored value =====
        let mut state0_block = mir::BasicBlock {
            id: state0_block_id,
            instructions: Vec::new(),
            terminator: mir::Terminator::Unreachable,
        };

        state0_block.instructions.push(mir::Instruction {
            kind: mir::InstructionKind::RuntimeCall {
                dest: iter_local,
                func: mir::RuntimeFunc::GeneratorGetLocal,
                args: vec![
                    mir::Operand::Local(gen_param_local),
                    mir::Operand::Constant(mir::Constant::Int(0)),
                ],
            },
        });

        state0_block.instructions.push(mir::Instruction {
            kind: mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::GeneratorSetState,
                args: vec![
                    mir::Operand::Local(gen_param_local),
                    mir::Operand::Constant(mir::Constant::Int(1)),
                ],
            },
        });

        state0_block.terminator = mir::Terminator::Goto(iter_next_block_id);
        mir_func.blocks.insert(state0_block_id, state0_block);

        // ===== State 1: Load iterator from generator state =====
        let mut state1_block = mir::BasicBlock {
            id: state1_block_id,
            instructions: Vec::new(),
            terminator: mir::Terminator::Unreachable,
        };

        state1_block.instructions.push(mir::Instruction {
            kind: mir::InstructionKind::RuntimeCall {
                dest: iter_local,
                func: mir::RuntimeFunc::GeneratorGetLocal,
                args: vec![
                    mir::Operand::Local(gen_param_local),
                    mir::Operand::Constant(mir::Constant::Int(0)),
                ],
            },
        });

        state1_block.terminator = mir::Terminator::Goto(iter_next_block_id);
        mir_func.blocks.insert(state1_block_id, state1_block);

        // ===== Iter next block: Call next() on iterator =====
        let mut iter_next_block = mir::BasicBlock {
            id: iter_next_block_id,
            instructions: Vec::new(),
            terminator: mir::Terminator::Unreachable,
        };

        iter_next_block.instructions.push(mir::Instruction {
            kind: mir::InstructionKind::RuntimeCall {
                dest: next_value_local,
                func: mir::RuntimeFunc::IterNextNoExc,
                args: vec![mir::Operand::Local(iter_local)],
            },
        });

        iter_next_block.instructions.push(mir::Instruction {
            kind: mir::InstructionKind::RuntimeCall {
                dest: iter_done_local,
                func: mir::RuntimeFunc::IterIsExhausted,
                args: vec![mir::Operand::Local(iter_local)],
            },
        });

        iter_next_block.terminator = mir::Terminator::Goto(check_iter_done_block_id);
        mir_func.blocks.insert(iter_next_block_id, iter_next_block);

        // ===== Check iter done: Branch on done =====
        // When iterator is done, go to first trailing yield (or mark exhausted if none)
        let iter_done_target = if !trailing_block_ids.is_empty() {
            trailing_block_ids[0]
        } else {
            mark_exhausted_block_id
        };

        #[allow(unused_assignments)]
        let assign_block_id = if for_gen.filter_cond.is_some() {
            let id = BlockId::from(next_block_id);
            next_block_id += 1;
            id
        } else {
            yield_block_id
        };

        let mut check_iter_done_block = mir::BasicBlock {
            id: check_iter_done_block_id,
            instructions: Vec::new(),
            terminator: mir::Terminator::Unreachable,
        };

        check_iter_done_block.terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(iter_done_local),
            then_block: iter_done_target,
            else_block: assign_block_id,
        };
        mir_func
            .blocks
            .insert(check_iter_done_block_id, check_iter_done_block);

        // ===== Build yield block (for the for-loop iteration) =====
        if let Some(filter_cond_id) = for_gen.filter_cond {
            // Filter path: assign → check filter → yield or loop back
            let mut assign_block = mir::BasicBlock {
                id: assign_block_id,
                instructions: Vec::new(),
                terminator: mir::Terminator::Unreachable,
            };

            assign_block.instructions.push(mir::Instruction {
                kind: mir::InstructionKind::Copy {
                    dest: target_mir_local,
                    src: mir::Operand::Local(next_value_local),
                },
            });

            if let Some(&gen_idx) = var_to_gen_local.get(&for_gen.target_var) {
                assign_block.instructions.push(mir::Instruction {
                    kind: mir::InstructionKind::RuntimeCall {
                        dest: dummy_local,
                        func: mir::RuntimeFunc::GeneratorSetLocal,
                        args: vec![
                            mir::Operand::Local(gen_param_local),
                            mir::Operand::Constant(mir::Constant::Int(gen_idx as i64)),
                            mir::Operand::Local(target_mir_local),
                        ],
                    },
                });
            }

            let filter_cond_local = self.alloc_and_add_local(Type::Bool, &mut mir_func);
            let filter_expr = &hir_module.exprs[filter_cond_id];
            let filter_operand = self.lower_simple_expr_for_generator(
                filter_expr,
                hir_module,
                &mut assign_block,
                &mut mir_func,
                &var_to_mir_local,
                target_mir_local,
                for_gen.target_var,
            )?;

            assign_block.instructions.push(mir::Instruction {
                kind: mir::InstructionKind::Copy {
                    dest: filter_cond_local,
                    src: filter_operand,
                },
            });

            assign_block.terminator = mir::Terminator::Branch {
                cond: mir::Operand::Local(filter_cond_local),
                then_block: yield_block_id,
                else_block: iter_next_block_id,
            };
            mir_func.blocks.insert(assign_block_id, assign_block);

            let mut yield_block = mir::BasicBlock {
                id: yield_block_id,
                instructions: Vec::new(),
                terminator: mir::Terminator::Unreachable,
            };

            yield_block.instructions.push(mir::Instruction {
                kind: mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::GeneratorSetLocal,
                    args: vec![
                        mir::Operand::Local(gen_param_local),
                        mir::Operand::Constant(mir::Constant::Int(0)),
                        mir::Operand::Local(iter_local),
                    ],
                },
            });

            let yield_value_operand = if let Some(yield_expr_id) = for_gen.yield_expr {
                let yield_expr = &hir_module.exprs[yield_expr_id];
                self.compute_yield_expr_for_generator(
                    yield_expr,
                    hir_module,
                    &mut yield_block,
                    &mut mir_func,
                    &var_to_mir_local,
                    next_value_local,
                    for_gen.target_var,
                    Some(&loop_var_elem_ty),
                )?
            } else {
                mir::Operand::Constant(mir::Constant::None)
            };

            yield_block.terminator = mir::Terminator::Return(Some(yield_value_operand));
            mir_func.blocks.insert(yield_block_id, yield_block);
        } else {
            // No filter — direct yield
            let mut yield_block = mir::BasicBlock {
                id: yield_block_id,
                instructions: Vec::new(),
                terminator: mir::Terminator::Unreachable,
            };

            yield_block.instructions.push(mir::Instruction {
                kind: mir::InstructionKind::Copy {
                    dest: target_mir_local,
                    src: mir::Operand::Local(next_value_local),
                },
            });

            yield_block.instructions.push(mir::Instruction {
                kind: mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::GeneratorSetLocal,
                    args: vec![
                        mir::Operand::Local(gen_param_local),
                        mir::Operand::Constant(mir::Constant::Int(0)),
                        mir::Operand::Local(iter_local),
                    ],
                },
            });

            let yield_value_operand = if let Some(yield_expr_id) = for_gen.yield_expr {
                let yield_expr = &hir_module.exprs[yield_expr_id];
                self.compute_yield_expr_for_generator(
                    yield_expr,
                    hir_module,
                    &mut yield_block,
                    &mut mir_func,
                    &var_to_mir_local,
                    next_value_local,
                    for_gen.target_var,
                    Some(&loop_var_elem_ty),
                )?
            } else {
                mir::Operand::Constant(mir::Constant::None)
            };

            yield_block.terminator = mir::Terminator::Return(Some(yield_value_operand));
            mir_func.blocks.insert(yield_block_id, yield_block);
        }

        // ===== Trailing yield blocks (from `yield from X; yield Y; yield Z`) =====
        for (i, trailing_yield_expr) in for_gen.trailing_yields.iter().enumerate() {
            let block_id = trailing_block_ids[i];
            let mut trailing_block = mir::BasicBlock {
                id: block_id,
                instructions: Vec::new(),
                terminator: mir::Terminator::Unreachable,
            };

            // Set state to next trailing yield (i+2+1) or to a terminal state
            // NOTE: We never mark exhausted here — rt_iter_next_no_exc checks the
            // exhausted flag AFTER resume returns, so setting exhausted would cause
            // it to discard the yielded value. Instead, set state to a value beyond
            // all valid states so the next call falls through to mark_exhausted.
            let next_state = (i + 2 + 1) as i64; // states: 0=init, 1=iter, 2..=trailing
            trailing_block.instructions.push(mir::Instruction {
                kind: mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::GeneratorSetState,
                    args: vec![
                        mir::Operand::Local(gen_param_local),
                        mir::Operand::Constant(mir::Constant::Int(next_state)),
                    ],
                },
            });

            // Compute and return the yield value
            let value_operand = if let Some(yield_expr_id) = trailing_yield_expr {
                let expr = &hir_module.exprs[*yield_expr_id];
                match &expr.kind {
                    hir::ExprKind::Int(n) => mir::Operand::Constant(mir::Constant::Int(*n)),
                    hir::ExprKind::Float(f) => mir::Operand::Constant(mir::Constant::Float(*f)),
                    hir::ExprKind::Bool(b) => {
                        mir::Operand::Constant(mir::Constant::Int(if *b { 1 } else { 0 }))
                    }
                    hir::ExprKind::None => mir::Operand::Constant(mir::Constant::Int(0)),
                    hir::ExprKind::Var(var_id) => {
                        if let Some(&gen_local_idx) = var_to_gen_local.get(var_id) {
                            let var_local = self.alloc_and_add_local(Type::Int, &mut mir_func);
                            trailing_block.instructions.push(mir::Instruction {
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
            } else {
                mir::Operand::Constant(mir::Constant::Int(0))
            };

            trailing_block.terminator = mir::Terminator::Return(Some(value_operand));
            mir_func.blocks.insert(block_id, trailing_block);
        }

        mir_func.entry_block = entry_block_id;

        Ok(mir_func)
    }
}
