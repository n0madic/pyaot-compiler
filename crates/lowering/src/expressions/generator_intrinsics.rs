//! Lowering for generator intrinsic expressions
//!
//! Each `GeneratorIntrinsic` variant maps 1:1 to a runtime function call.
//! These intrinsics are created by the generator desugaring pass and are
//! never present in the original HIR from the frontend.

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a generator intrinsic expression to a MIR RuntimeCall.
    pub(crate) fn lower_generator_intrinsic(
        &mut self,
        intrinsic: &hir::GeneratorIntrinsic,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        use pyaot_core_defs::runtime_func_def::*;

        match intrinsic {
            hir::GeneratorIntrinsic::Create {
                func_id,
                num_locals,
            } => {
                let dest = self.emit_runtime_call_gc(
                    mir::RuntimeFunc::Call(&RT_MAKE_GENERATOR),
                    vec![
                        mir::Operand::Constant(mir::Constant::Int(*func_id as i64)),
                        mir::Operand::Constant(mir::Constant::Int(*num_locals as i64)),
                    ],
                    Type::Iterator(Box::new(Type::Any)),
                    mir_func,
                );
                Ok(mir::Operand::Local(dest))
            }

            hir::GeneratorIntrinsic::GetState(gen_expr_id) => {
                let gen_op =
                    self.lower_expr(&hir_module.exprs[*gen_expr_id], hir_module, mir_func)?;
                let dest = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest,
                    func: mir::RuntimeFunc::Call(&RT_GENERATOR_GET_STATE),
                    args: vec![gen_op],
                });
                Ok(mir::Operand::Local(dest))
            }

            hir::GeneratorIntrinsic::SetState { gen, state } => {
                let gen_op = self.lower_expr(&hir_module.exprs[*gen], hir_module, mir_func)?;
                let dest = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest,
                    func: mir::RuntimeFunc::Call(&RT_GENERATOR_SET_STATE),
                    args: vec![gen_op, mir::Operand::Constant(mir::Constant::Int(*state))],
                });
                Ok(mir::Operand::Local(dest))
            }

            hir::GeneratorIntrinsic::GetLocal { gen, idx } => {
                let gen_op = self.lower_expr(&hir_module.exprs[*gen], hir_module, mir_func)?;
                let dest = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest,
                    func: mir::RuntimeFunc::Call(&RT_GENERATOR_GET_LOCAL),
                    args: vec![
                        gen_op,
                        mir::Operand::Constant(mir::Constant::Int(*idx as i64)),
                    ],
                });
                Ok(mir::Operand::Local(dest))
            }

            hir::GeneratorIntrinsic::SetLocal { gen, idx, value } => {
                let gen_op = self.lower_expr(&hir_module.exprs[*gen], hir_module, mir_func)?;
                let val_op = self.lower_expr(&hir_module.exprs[*value], hir_module, mir_func)?;
                let dest = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest,
                    func: mir::RuntimeFunc::Call(&RT_GENERATOR_SET_LOCAL),
                    args: vec![
                        gen_op,
                        mir::Operand::Constant(mir::Constant::Int(*idx as i64)),
                        val_op,
                    ],
                });
                Ok(mir::Operand::Local(dest))
            }

            hir::GeneratorIntrinsic::SetLocalType { gen, idx, type_tag } => {
                let gen_op = self.lower_expr(&hir_module.exprs[*gen], hir_module, mir_func)?;
                let dest = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest,
                    func: mir::RuntimeFunc::Call(&RT_GENERATOR_SET_LOCAL_TYPE),
                    args: vec![
                        gen_op,
                        mir::Operand::Constant(mir::Constant::Int(*idx as i64)),
                        mir::Operand::Constant(mir::Constant::Int(*type_tag as i64)),
                    ],
                });
                Ok(mir::Operand::Local(dest))
            }

            hir::GeneratorIntrinsic::SetExhausted(gen_expr_id) => {
                let gen_op =
                    self.lower_expr(&hir_module.exprs[*gen_expr_id], hir_module, mir_func)?;
                let dest = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest,
                    func: mir::RuntimeFunc::Call(&RT_GENERATOR_SET_EXHAUSTED),
                    args: vec![gen_op],
                });
                Ok(mir::Operand::Local(dest))
            }

            hir::GeneratorIntrinsic::IsExhausted(gen_expr_id) => {
                let gen_op =
                    self.lower_expr(&hir_module.exprs[*gen_expr_id], hir_module, mir_func)?;
                let dest = self.alloc_and_add_local(Type::Bool, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest,
                    func: mir::RuntimeFunc::Call(&RT_GENERATOR_IS_EXHAUSTED),
                    args: vec![gen_op],
                });
                Ok(mir::Operand::Local(dest))
            }

            hir::GeneratorIntrinsic::GetSentValue(gen_expr_id) => {
                let gen_op =
                    self.lower_expr(&hir_module.exprs[*gen_expr_id], hir_module, mir_func)?;
                let dest = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest,
                    func: mir::RuntimeFunc::Call(&RT_GENERATOR_GET_SENT_VALUE),
                    args: vec![gen_op],
                });
                Ok(mir::Operand::Local(dest))
            }

            hir::GeneratorIntrinsic::IterNextNoExc(iter_expr_id) => {
                let iter_op =
                    self.lower_expr(&hir_module.exprs[*iter_expr_id], hir_module, mir_func)?;
                let dest = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest,
                    func: mir::RuntimeFunc::Call(&RT_ITER_NEXT_NO_EXC),
                    args: vec![iter_op],
                });
                Ok(mir::Operand::Local(dest))
            }

            hir::GeneratorIntrinsic::IterIsExhausted(iter_expr_id) => {
                let iter_op =
                    self.lower_expr(&hir_module.exprs[*iter_expr_id], hir_module, mir_func)?;
                let dest = self.alloc_and_add_local(Type::Bool, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest,
                    func: mir::RuntimeFunc::Call(&RT_ITER_IS_EXHAUSTED),
                    args: vec![iter_op],
                });
                Ok(mir::Operand::Local(dest))
            }
        }
    }
}
