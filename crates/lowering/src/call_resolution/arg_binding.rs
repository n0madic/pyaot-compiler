//! Positional argument lowering and list/tuple unpacking.

use super::ParamClassification;
use crate::context::Lowering;
use crate::expressions::ExpandedArg;
use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::LocalId;

impl<'a> Lowering<'a> {
    /// Lower all positional arguments, handling runtime unpacking.
    ///
    /// Returns a vector of operands representing the lowered positional args.
    pub(crate) fn lower_positional_args(
        &mut self,
        positional: &[ExpandedArg],
        params: &ParamClassification<'_>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<Vec<mir::Operand>> {
        let mut all_positional = Vec::new();

        let mut positional_index = 0usize;
        for arg in positional {
            match arg {
                ExpandedArg::Regular(expr_id) => {
                    let arg_expr = &hir_module.exprs[*expr_id];

                    // Bidirectional: propagate parameter type into argument expression
                    let expected = params
                        .regular
                        .get(positional_index)
                        .and_then(|p| p.ty.clone());
                    let operand =
                        self.lower_expr_expecting(arg_expr, expected, hir_module, mir_func)?;

                    all_positional.push(operand);
                    positional_index += 1;
                }
                ExpandedArg::RuntimeUnpackTuple(expr_id) => {
                    self.lower_runtime_tuple_unpack(
                        *expr_id,
                        hir_module,
                        mir_func,
                        &mut all_positional,
                    )?;
                }
                ExpandedArg::RuntimeUnpackList(expr_id) => {
                    self.lower_runtime_list_unpack(
                        *expr_id,
                        &all_positional,
                        params,
                        hir_module,
                        mir_func,
                    )?
                    .into_iter()
                    .for_each(|op| all_positional.push(op));
                }
            }
        }

        Ok(all_positional)
    }

    /// Unpack a tuple at runtime and add elements to positional args.
    fn lower_runtime_tuple_unpack(
        &mut self,
        expr_id: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
        all_positional: &mut Vec<mir::Operand>,
    ) -> Result<()> {
        let tuple_expr = &hir_module.exprs[expr_id];
        let tuple_type = self.get_type_of_expr_id(expr_id, hir_module);
        let tuple_operand = self.lower_expr(tuple_expr, hir_module, mir_func)?;

        if let Type::Tuple(elem_types) = tuple_type {
            for (i, elem_type) in elem_types.iter().enumerate() {
                let get_func = Self::tuple_get_func(elem_type);
                let elem_local = self.emit_runtime_call(
                    get_func,
                    vec![
                        tuple_operand.clone(),
                        mir::Operand::Constant(mir::Constant::Int(i as i64)),
                    ],
                    elem_type.clone(),
                    mir_func,
                );
                all_positional.push(mir::Operand::Local(elem_local));
            }
        } else {
            // Not a tuple - pass as-is
            all_positional.push(tuple_operand);
        }

        Ok(())
    }

    /// Unpack a list at runtime, handling varargs and default values.
    ///
    /// Returns a vector of operands extracted from the list.
    fn lower_runtime_list_unpack(
        &mut self,
        expr_id: hir::ExprId,
        already_processed: &[mir::Operand],
        params: &ParamClassification<'_>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<Vec<mir::Operand>> {
        let list_expr = &hir_module.exprs[expr_id];
        let list_type = self.get_type_of_expr_id(expr_id, hir_module);
        let list_operand = self.lower_expr(list_expr, hir_module, mir_func)?;

        let Type::List(elem_type) = list_type else {
            // Not a list - return as-is
            return Ok(vec![list_operand]);
        };

        let remaining_params = params.regular.len().saturating_sub(already_processed.len());
        let has_varargs = params.vararg.is_some();

        // Emit ListLen runtime call
        let len_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_LEN),
            vec![list_operand.clone()],
            Type::Int,
            mir_func,
        );

        if has_varargs {
            self.lower_list_unpack_with_varargs(
                &list_operand,
                len_local,
                remaining_params,
                &elem_type,
                mir_func,
            )
        } else {
            self.lower_list_unpack_fixed(
                &list_operand,
                len_local,
                already_processed.len(),
                params,
                &elem_type,
                hir_module,
                mir_func,
            )
        }
    }

    /// Unpack list elements when function has *args (flexible unpacking).
    fn lower_list_unpack_with_varargs(
        &mut self,
        list_operand: &mir::Operand,
        len_local: LocalId,
        remaining_params: usize,
        elem_type: &Type,
        mir_func: &mut mir::Function,
    ) -> Result<Vec<mir::Operand>> {
        // Validate: list_len >= remaining_params
        let required_local = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::Const {
            dest: required_local,
            value: mir::Constant::Int(remaining_params as i64),
        });

        let cmp_local = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: cmp_local,
            op: mir::BinOp::GtE,
            left: mir::Operand::Local(len_local),
            right: mir::Operand::Local(required_local),
        });

        // Create assertion blocks
        let fail_bb = self.new_block();
        let continue_bb = self.new_block();
        let fail_id = fail_bb.id;
        let continue_id = continue_bb.id;

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(cmp_local),
            then_block: continue_id,
            else_block: fail_id,
        };

        // Fail block
        self.push_block(fail_bb);
        self.emit_list_unpack_assertion_failure(remaining_params, None, mir_func);

        // Continue block
        self.push_block(continue_bb);

        // Extract elements for regular params
        let mut extracted = Vec::new();
        for i in 0..remaining_params {
            let elem_local = self.extract_list_element(list_operand, i, elem_type, mir_func);
            extracted.push(mir::Operand::Local(elem_local));
        }

        // Build varargs tuple from remaining elements
        let tail_to_tuple_func = match elem_type {
            Type::Float => mir::RuntimeFunc::Call(
                &pyaot_core_defs::runtime_func_def::RT_LIST_TAIL_TO_TUPLE_FLOAT,
            ),
            Type::Bool => mir::RuntimeFunc::Call(
                &pyaot_core_defs::runtime_func_def::RT_LIST_TAIL_TO_TUPLE_BOOL,
            ),
            _ => mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_TAIL_TO_TUPLE),
        };

        let varargs_tuple_local =
            self.alloc_gc_local(Type::Tuple(vec![elem_type.clone()]), mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: varargs_tuple_local,
            func: tail_to_tuple_func,
            args: vec![
                list_operand.clone(),
                mir::Operand::Constant(mir::Constant::Int(remaining_params as i64)),
            ],
        });

        self.set_pending_varargs(varargs_tuple_local);

        Ok(extracted)
    }

    /// Unpack list elements when function has no *args (fixed unpacking).
    fn lower_list_unpack_fixed(
        &mut self,
        list_operand: &mir::Operand,
        len_local: LocalId,
        already_filled: usize,
        params: &ParamClassification<'_>,
        elem_type: &Type,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<Vec<mir::Operand>> {
        let remaining_params = params.regular.len().saturating_sub(already_filled);
        let required_count = params
            .regular
            .iter()
            .skip(already_filled)
            .filter(|p| p.default.is_none())
            .count();

        // Validate: required_count <= list_len <= remaining_params
        self.emit_list_length_validation(len_local, required_count, remaining_params, mir_func);

        // Extract elements conditionally
        let mut extracted = Vec::new();
        for i in 0..remaining_params {
            let param = &params.regular[already_filled + i];
            let has_default = param.default.is_some();

            if has_default {
                let elem_local = self.extract_list_element_with_default(
                    list_operand,
                    len_local,
                    i,
                    param
                        .default
                        .expect("parameter must have default value when has_default is true"),
                    elem_type,
                    hir_module,
                    mir_func,
                )?;
                extracted.push(mir::Operand::Local(elem_local));
            } else {
                let elem_local = self.extract_list_element(list_operand, i, elem_type, mir_func);
                extracted.push(mir::Operand::Local(elem_local));
            }
        }

        Ok(extracted)
    }

    /// Extract a single element from a list at runtime.
    pub(super) fn extract_list_element(
        &mut self,
        list_operand: &mir::Operand,
        index: usize,
        elem_type: &Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        let (get_func, needs_unbox) = match elem_type {
            Type::Int => (
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET_INT),
                false,
            ),
            Type::Float => (
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET_FLOAT),
                false,
            ),
            Type::Bool => (
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET),
                true,
            ),
            _ => (
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET),
                false,
            ),
        };

        if needs_unbox {
            let boxed_local = self.emit_runtime_call(
                get_func,
                vec![
                    list_operand.clone(),
                    mir::Operand::Constant(mir::Constant::Int(index as i64)),
                ],
                Type::HeapAny,
                mir_func,
            );

            let elem_local = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_UNBOX_BOOL),
                vec![mir::Operand::Local(boxed_local)],
                elem_type.clone(),
                mir_func,
            );
            elem_local
        } else {
            let elem_local = self.emit_runtime_call(
                get_func,
                vec![
                    list_operand.clone(),
                    mir::Operand::Constant(mir::Constant::Int(index as i64)),
                ],
                elem_type.clone(),
                mir_func,
            );
            elem_local
        }
    }

    /// Extract a list element with fallback to default value if out of bounds.
    fn extract_list_element_with_default(
        &mut self,
        list_operand: &mir::Operand,
        len_local: LocalId,
        index: usize,
        default_id: hir::ExprId,
        elem_type: &Type,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<LocalId> {
        // Check if index < list_len
        let idx_local = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::Const {
            dest: idx_local,
            value: mir::Constant::Int(index as i64),
        });

        let in_bounds_local = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: in_bounds_local,
            op: mir::BinOp::Lt,
            left: mir::Operand::Local(idx_local),
            right: mir::Operand::Local(len_local),
        });

        // Create blocks
        let extract_bb = self.new_block();
        let default_bb = self.new_block();
        let merge_bb = self.new_block();
        let extract_id = extract_bb.id;
        let default_id_block = default_bb.id;
        let merge_id = merge_bb.id;

        let elem_local = self.alloc_and_add_local(elem_type.clone(), mir_func);

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(in_bounds_local),
            then_block: extract_id,
            else_block: default_id_block,
        };

        // Extract block
        self.push_block(extract_bb);

        let extracted = self.extract_list_element(list_operand, index, elem_type, mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: elem_local,
            src: mir::Operand::Local(extracted),
        });
        self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);

        // Default block
        self.push_block(default_bb);

        let default_expr = &hir_module.exprs[default_id];
        let default_operand = self.lower_expr(default_expr, hir_module, mir_func)?;
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: elem_local,
            src: default_operand,
        });
        self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);

        // Merge block
        self.push_block(merge_bb);

        Ok(elem_local)
    }

    /// Emit list length validation for fixed unpacking.
    fn emit_list_length_validation(
        &mut self,
        len_local: LocalId,
        required_count: usize,
        remaining_params: usize,
        mir_func: &mut mir::Function,
    ) {
        // Check: list_len >= required_count
        let min_required = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::Const {
            dest: min_required,
            value: mir::Constant::Int(required_count as i64),
        });

        let cmp_min = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: cmp_min,
            op: mir::BinOp::GtE,
            left: mir::Operand::Local(len_local),
            right: mir::Operand::Local(min_required),
        });

        // Check: list_len <= remaining_params
        let max_allowed = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::Const {
            dest: max_allowed,
            value: mir::Constant::Int(remaining_params as i64),
        });

        let cmp_max = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: cmp_max,
            op: mir::BinOp::LtE,
            left: mir::Operand::Local(len_local),
            right: mir::Operand::Local(max_allowed),
        });

        // Combine: cmp_min && cmp_max
        let cmp_local = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: cmp_local,
            op: mir::BinOp::And,
            left: mir::Operand::Local(cmp_min),
            right: mir::Operand::Local(cmp_max),
        });

        let fail_bb = self.new_block();
        let continue_bb = self.new_block();
        let fail_id = fail_bb.id;
        let continue_id = continue_bb.id;

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(cmp_local),
            then_block: continue_id,
            else_block: fail_id,
        };

        // Fail block
        self.push_block(fail_bb);
        self.emit_list_unpack_assertion_failure(required_count, Some(remaining_params), mir_func);

        // Continue block
        self.push_block(continue_bb);
    }

    /// Emit assertion failure for list unpacking.
    pub(super) fn emit_list_unpack_assertion_failure(
        &mut self,
        required: usize,
        max: Option<usize>,
        mir_func: &mut mir::Function,
    ) {
        let msg = if let Some(max_val) = max {
            if required == max_val {
                format!(
                    "list unpacking: expected {} elements for function parameters",
                    required
                )
            } else {
                format!(
                    "list unpacking: expected {}-{} elements for function parameters",
                    required, max_val
                )
            }
        } else {
            format!(
                "list unpacking: expected at least {} elements for function parameters",
                required
            )
        };

        let msg_str = self.intern(&msg);
        let msg_operand = mir::Operand::Constant(mir::Constant::Str(msg_str));

        // Emit the assertion fail instruction. AssertFail never returns,
        // but we need a dest local for the instruction format.
        let dummy_local = self.alloc_and_add_local(Type::None, mir_func);
        self.current_block_mut()
            .instructions
            .push(mir::Instruction {
                kind: mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::AssertFail,
                    args: vec![msg_operand],
                },
                span: None,
            });
        self.current_block_mut().terminator = mir::Terminator::Unreachable;
    }
}
