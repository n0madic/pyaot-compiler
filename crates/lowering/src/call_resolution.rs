//! Call argument resolution
//!
//! This module handles the complex logic of resolving positional and keyword
//! arguments against function parameters, including:
//! - Runtime *args and **kwargs unpacking
//! - Default parameter handling
//! - Keyword-only parameters
//! - Building varargs tuples and kwargs dicts
#![allow(clippy::too_many_arguments)]

use crate::context::Lowering;
use crate::expressions::ExpandedArg;
use indexmap::IndexMap;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{self as hir, ParamKind};
use pyaot_mir::{self as mir, ValueKind};
use pyaot_types::Type;
use pyaot_utils::{InternedString, LocalId};

/// Result of matching keyword arguments to parameters.
/// Contains (kwonly_resolved, extra_keywords).
pub(crate) type KwargsMatchResult = (
    Vec<Option<mir::Operand>>,
    IndexMap<InternedString, mir::Operand>,
);

/// Parameters classified by their kind for easier handling.
pub(crate) struct ParamClassification<'a> {
    pub regular: Vec<&'a hir::Param>,
    pub vararg: Option<&'a hir::Param>,
    pub kwonly: Vec<&'a hir::Param>,
    pub kwarg: Option<&'a hir::Param>,
}

impl<'a> ParamClassification<'a> {
    /// Classify parameters by their kind.
    pub fn from_params(params: &'a [hir::Param]) -> Self {
        let mut regular = Vec::new();
        let mut vararg = None;
        let mut kwonly = Vec::new();
        let mut kwarg = None;

        for param in params {
            match param.kind {
                ParamKind::Regular => regular.push(param),
                ParamKind::VarPositional => vararg = Some(param),
                ParamKind::KeywordOnly => kwonly.push(param),
                ParamKind::VarKeyword => kwarg = Some(param),
            }
        }

        Self {
            regular,
            vararg,
            kwonly,
            kwarg,
        }
    }
}

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
        let tuple_type = self.get_expr_type(tuple_expr, hir_module);
        let tuple_operand = self.lower_expr(tuple_expr, hir_module, mir_func)?;

        if let Type::Tuple(elem_types) = tuple_type {
            for (i, elem_type) in elem_types.iter().enumerate() {
                let elem_local = self.alloc_and_add_local(elem_type.clone(), mir_func);
                let get_func = Self::tuple_get_func(elem_type);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: elem_local,
                    func: get_func,
                    args: vec![
                        tuple_operand.clone(),
                        mir::Operand::Constant(mir::Constant::Int(i as i64)),
                    ],
                });
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
        let list_type = self.get_expr_type(list_expr, hir_module);
        let list_operand = self.lower_expr(list_expr, hir_module, mir_func)?;

        let Type::List(elem_type) = list_type else {
            // Not a list - return as-is
            return Ok(vec![list_operand]);
        };

        let remaining_params = params.regular.len().saturating_sub(already_processed.len());
        let has_varargs = params.vararg.is_some();

        // Emit ListLen runtime call
        let len_local = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: len_local,
            func: mir::RuntimeFunc::ListLen,
            args: vec![list_operand.clone()],
        });

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
            Type::Float => mir::RuntimeFunc::ListTailToTupleFloat,
            Type::Bool => mir::RuntimeFunc::ListTailToTupleBool,
            _ => mir::RuntimeFunc::ListTailToTuple,
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
    fn extract_list_element(
        &mut self,
        list_operand: &mir::Operand,
        index: usize,
        elem_type: &Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        let (get_func, needs_unbox) = match elem_type {
            Type::Int => (mir::RuntimeFunc::ListGetInt, false),
            Type::Float => (mir::RuntimeFunc::ListGetFloat, false),
            Type::Bool => (mir::RuntimeFunc::ListGet, true),
            _ => (mir::RuntimeFunc::ListGet, false),
        };

        if needs_unbox {
            let boxed_local = self.alloc_and_add_local(Type::HeapAny, mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: boxed_local,
                func: get_func,
                args: vec![
                    list_operand.clone(),
                    mir::Operand::Constant(mir::Constant::Int(index as i64)),
                ],
            });

            let elem_local = self.alloc_and_add_local(elem_type.clone(), mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: elem_local,
                func: mir::RuntimeFunc::UnboxBool,
                args: vec![mir::Operand::Local(boxed_local)],
            });
            elem_local
        } else {
            let elem_local = self.alloc_and_add_local(elem_type.clone(), mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: elem_local,
                func: get_func,
                args: vec![
                    list_operand.clone(),
                    mir::Operand::Constant(mir::Constant::Int(index as i64)),
                ],
            });
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
    fn emit_list_unpack_assertion_failure(
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

    /// Match positional arguments to regular parameters.
    ///
    /// Returns (resolved params, extra positional args).
    pub(crate) fn match_positional_to_params(
        &self,
        all_positional: Vec<mir::Operand>,
        num_regular_params: usize,
    ) -> (Vec<Option<mir::Operand>>, Vec<mir::Operand>) {
        let mut resolved: Vec<Option<mir::Operand>> = vec![None; num_regular_params];
        let mut extra_positional = Vec::new();

        for (i, operand) in all_positional.into_iter().enumerate() {
            if i < num_regular_params {
                resolved[i] = Some(operand);
            } else {
                extra_positional.push(operand);
            }
        }

        (resolved, extra_positional)
    }

    /// Match keyword arguments to parameters.
    ///
    /// Returns (updated resolved, kwonly_resolved, extra_keywords).
    pub(crate) fn match_kwargs_to_params(
        &mut self,
        kwargs: &[hir::KeywordArg],
        regular_params: &[&hir::Param],
        kwonly_params: &[&hir::Param],
        resolved: &mut [Option<mir::Operand>],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<KwargsMatchResult> {
        let mut kwonly_resolved: Vec<Option<mir::Operand>> = vec![None; kwonly_params.len()];
        let mut extra_keywords = IndexMap::new();

        for kwarg in kwargs {
            let kwarg_name = self.resolve(kwarg.name);

            // Try regular params first
            let param_idx = regular_params
                .iter()
                .position(|p| self.resolve(p.name) == kwarg_name);

            if let Some(idx) = param_idx {
                if resolved[idx].is_some() {
                    return Err(CompilerError::duplicate_keyword_argument(
                        kwarg_name.to_string(),
                        kwarg.span,
                    ));
                }
                let arg_expr = &hir_module.exprs[kwarg.value];
                resolved[idx] = Some(self.lower_expr(arg_expr, hir_module, mir_func)?);
            } else {
                // Try kwonly params
                let kwonly_idx = kwonly_params
                    .iter()
                    .position(|p| self.resolve(p.name) == kwarg_name);

                if let Some(idx) = kwonly_idx {
                    if kwonly_resolved[idx].is_some() {
                        return Err(CompilerError::duplicate_keyword_argument(
                            kwarg_name.to_string(),
                            kwarg.span,
                        ));
                    }
                    let arg_expr = &hir_module.exprs[kwarg.value];
                    kwonly_resolved[idx] = Some(self.lower_expr(arg_expr, hir_module, mir_func)?);
                } else {
                    // Extra kwarg
                    let arg_expr = &hir_module.exprs[kwarg.value];
                    extra_keywords
                        .insert(kwarg.name, self.lower_expr(arg_expr, hir_module, mir_func)?);
                }
            }
        }

        Ok((kwonly_resolved, extra_keywords))
    }

    /// Fill defaults for missing parameters.
    ///
    /// If `target_func_id` is provided, mutable defaults (list, dict, set, class instances)
    /// are loaded from global storage instead of being re-evaluated. This implements Python's
    /// semantics where mutable defaults are evaluated once at function definition time.
    ///
    /// The `param_index_offset` adjusts the lookup index when the params slice doesn't start
    /// at index 0 in the original function. For example, when calling `__init__`, the `self`
    /// parameter is skipped when resolving user arguments, but `default_value_slots` uses
    /// indices relative to the original function parameters (including `self`).
    pub(crate) fn fill_param_defaults(
        &mut self,
        resolved: &mut [Option<mir::Operand>],
        params: &[&hir::Param],
        target_func_id: Option<pyaot_utils::FuncId>,
        param_index_offset: usize,
        call_span: pyaot_utils::Span,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        for (i, param) in params.iter().enumerate() {
            if resolved[i].is_none() {
                if let Some(default_id) = param.default {
                    // Check if this parameter has a stored mutable default
                    // Adjust index by offset when looking up stored mutable defaults
                    let stored_slot = target_func_id
                        .and_then(|fid| self.get_default_slot(&(fid, i + param_index_offset)));

                    if let Some(slot) = stored_slot {
                        // Load the pre-evaluated default from global storage
                        let default_local = self.alloc_gc_local(Type::Any, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: default_local,
                            func: mir::RuntimeFunc::GlobalGet(ValueKind::Ptr),
                            args: vec![mir::Operand::Constant(mir::Constant::Int(slot as i64))],
                        });
                        resolved[i] = Some(mir::Operand::Local(default_local));
                    } else {
                        // Evaluate the default expression (immutable types or unknown func)
                        let default_expr = &hir_module.exprs[default_id];
                        resolved[i] = Some(self.lower_expr(default_expr, hir_module, mir_func)?);
                    }
                } else {
                    let param_name = self.resolve(param.name).to_string();
                    return Err(CompilerError::missing_required_argument(
                        param_name, call_span,
                    ));
                }
            }
        }
        Ok(())
    }

    /// Build varargs tuple from extra positional arguments.
    pub(crate) fn build_varargs_tuple(
        &mut self,
        extra_positional: Vec<mir::Operand>,
        vararg_param: &hir::Param,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        // Get element type from vararg_param.ty
        let elem_type = vararg_param
            .ty
            .as_ref()
            .and_then(|t| match t {
                Type::Tuple(types) if !types.is_empty() => Some(types[0].clone()),
                _ => None,
            })
            .unwrap_or(Type::Any);

        // Collect per-operand types for correct elem_tag inference
        let operand_types: Vec<Type> = extra_positional
            .iter()
            .map(|op| self.operand_type(op, mir_func))
            .collect();

        // If elem_type is Any (no annotation), infer from actual operand types.
        let elem_type = if elem_type == Type::Any
            && !extra_positional.is_empty()
            && operand_types.iter().all(|t| *t == Type::Int)
        {
            Type::Int
        } else {
            elem_type
        };

        // Check for pre-built varargs from list unpacking
        if let Some(pre_built) = self.take_pending_varargs() {
            if extra_positional.is_empty() {
                pre_built
            } else {
                self.create_combined_varargs_tuple(
                    &extra_positional,
                    pre_built,
                    &elem_type,
                    mir_func,
                )
            }
        } else {
            self.create_tuple_from_operands_typed(
                &extra_positional,
                &elem_type,
                Some(&operand_types),
                mir_func,
            )
        }
    }

    /// Build kwargs dict from extra keyword arguments.
    pub(crate) fn build_kwargs_dict(
        &mut self,
        extra_keywords: IndexMap<InternedString, mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        // Check for pre-built remaining dict from runtime unpacking
        if let Some((remaining_dict, _)) = self.take_pending_kwargs() {
            if extra_keywords.is_empty() {
                mir::Operand::Local(remaining_dict)
            } else {
                // Merge extra_keywords into remaining dict
                for (key_name, value_op) in &extra_keywords {
                    let key_local = self.alloc_gc_local(Type::Str, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: key_local,
                        func: mir::RuntimeFunc::MakeStr,
                        args: vec![mir::Operand::Constant(mir::Constant::Str(*key_name))],
                    });

                    let dummy_local = self.alloc_and_add_local(
                        Type::Dict(Box::new(Type::Str), Box::new(Type::Any)),
                        mir_func,
                    );
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: dummy_local,
                        func: mir::RuntimeFunc::DictSet,
                        args: vec![
                            mir::Operand::Local(remaining_dict),
                            mir::Operand::Local(key_local),
                            value_op.clone(),
                        ],
                    });
                }
                mir::Operand::Local(remaining_dict)
            }
        } else {
            let dict_local = self.create_dict_from_keywords(&extra_keywords, mir_func);
            mir::Operand::Local(dict_local)
        }
    }

    /// Process runtime **kwargs dict extraction for parameters.
    pub(crate) fn process_runtime_kwargs_dict(
        &mut self,
        dict_local: LocalId,
        value_type: Type,
        regular_params: &[&hir::Param],
        kwonly_params: &[&hir::Param],
        resolved: &mut [Option<mir::Operand>],
        kwonly_resolved: &mut [Option<mir::Operand>],
        has_kwarg_param: bool,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<Vec<LocalId>> {
        let mut consumed_keys = Vec::new();

        // Process both regular and kwonly params using unified helper
        self.extract_params_from_dict(
            regular_params,
            resolved,
            dict_local,
            &value_type,
            &mut consumed_keys,
            hir_module,
            mir_func,
        )?;

        self.extract_params_from_dict(
            kwonly_params,
            kwonly_resolved,
            dict_local,
            &value_type,
            &mut consumed_keys,
            hir_module,
            mir_func,
        )?;

        // Build remaining dict for **kwargs if needed
        if has_kwarg_param {
            let remaining_dict = self.alloc_gc_local(
                Type::Dict(Box::new(Type::Str), Box::new(value_type.clone())),
                mir_func,
            );
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: remaining_dict,
                func: mir::RuntimeFunc::DictCopy,
                args: vec![mir::Operand::Local(dict_local)],
            });

            // Remove consumed keys
            for key_local in &consumed_keys {
                let dummy = self.alloc_and_add_local(Type::HeapAny, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy,
                    func: mir::RuntimeFunc::DictPop,
                    args: vec![
                        mir::Operand::Local(remaining_dict),
                        mir::Operand::Local(*key_local),
                    ],
                });
            }

            self.set_pending_kwargs(remaining_dict, value_type);
        }

        Ok(consumed_keys)
    }

    /// Extract parameter values from a runtime kwargs dict.
    ///
    /// Iterates over parameters, extracting values from the dict for those that
    /// haven't been resolved yet. Updates `resolved` in place and appends
    /// consumed key locals to `consumed_keys`.
    fn extract_params_from_dict(
        &mut self,
        params: &[&hir::Param],
        resolved: &mut [Option<mir::Operand>],
        dict_local: LocalId,
        value_type: &Type,
        consumed_keys: &mut Vec<LocalId>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        for (i, param) in params.iter().enumerate() {
            if resolved[i].is_some() {
                continue;
            }

            if let Some((result_local, key)) =
                self.extract_param_from_dict(dict_local, param, value_type, hir_module, mir_func)?
            {
                resolved[i] = Some(mir::Operand::Local(result_local));
                consumed_keys.push(key);
            }
        }
        Ok(())
    }

    /// Extract a parameter value from a runtime dict.
    fn extract_param_from_dict(
        &mut self,
        dict_local: LocalId,
        param: &hir::Param,
        value_type: &Type,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<Option<(LocalId, LocalId)>> {
        let param_name_str = self.resolve(param.name).to_string();
        let key_interned = self.intern(&param_name_str);
        let key_local = self.alloc_gc_local(Type::Str, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: key_local,
            func: mir::RuntimeFunc::MakeStr,
            args: vec![mir::Operand::Constant(mir::Constant::Str(key_interned))],
        });

        // Check if dict contains key
        let contains_local = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: contains_local,
            func: mir::RuntimeFunc::DictContains,
            args: vec![
                mir::Operand::Local(dict_local),
                mir::Operand::Local(key_local),
            ],
        });

        // Create blocks
        let has_key_bb = self.new_block();
        let no_key_bb = self.new_block();
        let merge_bb = self.new_block();
        let has_key_id = has_key_bb.id;
        let no_key_id = no_key_bb.id;
        let merge_id = merge_bb.id;

        let param_type = param.ty.clone().unwrap_or_else(|| value_type.clone());
        let result_local = self.alloc_and_add_local(param_type.clone(), mir_func);

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(contains_local),
            then_block: has_key_id,
            else_block: no_key_id,
        };

        // Has-key block
        self.push_block(has_key_bb);

        // DictGet returns a boxed pointer for primitive values; use HeapAny to represent
        // the intermediate boxed pointer. For heap types, it returns the pointer directly.
        let dict_value_type = match &param_type {
            Type::Int | Type::Float | Type::Bool => Type::HeapAny, // boxed pointer
            _ => param_type.clone(),                               // direct pointer
        };
        let dict_value = self.alloc_and_add_local(dict_value_type, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: dict_value,
            func: mir::RuntimeFunc::DictGet,
            args: vec![
                mir::Operand::Local(dict_local),
                mir::Operand::Local(key_local),
            ],
        });

        let param_value = self.convert_dict_value_for_param(
            mir::Operand::Local(dict_value),
            &param_type,
            mir_func,
        );

        self.emit_instruction(mir::InstructionKind::Copy {
            dest: result_local,
            src: param_value,
        });
        self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);

        // No-key block
        self.push_block(no_key_bb);

        if let Some(default_id) = param.default {
            let default_expr = &hir_module.exprs[default_id];
            let default_operand = self.lower_expr(default_expr, hir_module, mir_func)?;
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: result_local,
                src: default_operand,
            });
            self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);
        } else {
            // Required param - emit assertion failure
            let dummy_local = self.alloc_and_add_local(Type::None, mir_func);
            let msg = format!("missing required keyword argument '{}'", param_name_str);
            let msg_str = self.intern(&msg);
            let msg_operand = mir::Operand::Constant(mir::Constant::Str(msg_str));
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::AssertFail,
                args: vec![msg_operand],
            });
            self.current_block_mut().terminator = mir::Terminator::Unreachable;
        }

        // Merge block
        self.push_block(merge_bb);

        Ok(Some((result_local, key_local)))
    }
}
