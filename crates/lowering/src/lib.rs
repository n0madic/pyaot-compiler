//! Lowering from HIR to MIR

#![forbid(unsafe_code)]

mod call_resolution;
mod class_metadata;
mod context;
mod exceptions;
mod expressions;
mod generators;
mod narrowing;
mod runtime_selector;
mod statements;
mod type_dispatch;
mod type_planning;
mod utils;

pub use context::{
    CrossModuleClassInfo, ExportedParam, FuncOrBuiltin, LoweredClassInfo, Lowering, SimpleDefault,
};

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{LocalId, VarId};

/// Extract the first argument from an operands vec, defaulting to None.
fn first_arg_or_none(args: Vec<mir::Operand>) -> mir::Operand {
    args.into_iter()
        .next()
        .unwrap_or(mir::Operand::Constant(mir::Constant::None))
}

/// Whether `ty` is a container type with `Any` element / key / value
/// parameters. Used by the Area E §E.6 prescan consumers to defer to
/// later, more precise type sources (RHS inference, `refined_container_types`,
/// etc.) rather than hard-coding a shape that will be tightened later.
pub(crate) fn is_useless_container_ty(ty: &Type) -> bool {
    match ty {
        Type::List(e) | Type::Set(e) => **e == Type::Any,
        Type::Dict(k, v) | Type::DefaultDict(k, v) => **k == Type::Any && **v == Type::Any,
        _ => false,
    }
}

impl<'a> Lowering<'a> {
    /// Helper to emit a boxing instruction and return the boxed operand.
    fn emit_box_primitive(
        &mut self,
        operand: mir::Operand,
        result_ty: Type,
        runtime_func: mir::RuntimeFunc,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        let args = if matches!(runtime_func, mir::RuntimeFunc::Call(def) if def.params.is_empty()) {
            vec![]
        } else {
            vec![operand]
        };
        let boxed_local = self.emit_runtime_call_gc(runtime_func, args, result_ty, mir_func);
        mir::Operand::Local(boxed_local)
    }

    /// Box a primitive value to a tagged `Value` when needed.
    ///
    /// Primitives (Int, Bool, Float, None) must be box-tagged for storage in
    /// dict keys/values, union-typed variables, and any other context
    /// requiring heap-shaped slots. After §F.2:
    /// - `Int`/`Bool` emit inline `ValueFromInt` / `ValueFromBool` MIR
    ///   instructions (`(x << 3) | TAG`) — no runtime call.
    /// - `Float` boxes via `rt_box_float` (heap-allocated `FloatObj`).
    /// - `None` boxes via `rt_box_none` (singleton `NoneObj`).
    /// - Heap types (Str, List, Dict, Tuple, Set, class instances, etc.)
    ///   are already pointers and pass through unchanged.
    ///
    /// Uses `Type::HeapAny` for the boxed result so callers see a uniform
    /// pointer-shaped local.
    pub(crate) fn box_primitive_if_needed(
        &mut self,
        operand: mir::Operand,
        ty: &Type,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        match ty {
            Type::Int => {
                let dest = self.alloc_stack_local(Type::HeapAny, mir_func);
                self.emit_instruction(mir::InstructionKind::ValueFromInt { dest, src: operand });
                mir::Operand::Local(dest)
            }
            Type::Bool => {
                let dest = self.alloc_stack_local(Type::HeapAny, mir_func);
                self.emit_instruction(mir::InstructionKind::ValueFromBool { dest, src: operand });
                mir::Operand::Local(dest)
            }
            Type::Float => self.emit_box_primitive(
                operand,
                Type::HeapAny,
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_BOX_FLOAT),
                mir_func,
            ),
            Type::None => self.emit_box_primitive(
                operand,
                Type::HeapAny,
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_BOX_NONE),
                mir_func,
            ),
            // All heap types are already object pointers — no boxing needed
            _ => operand,
        }
    }

    /// Unbox a heap-stored value to a primitive type if needed. After §F.2:
    /// - `Int`/`Bool` emit inline `UnwrapValueInt` / `UnwrapValueBool` MIR
    ///   instructions (arithmetic shift) — no runtime call.
    /// - `Float` calls `rt_unbox_float` (heap-boxed FloatObj).
    /// - Other types pass through unchanged.
    pub(crate) fn unbox_if_needed(
        &mut self,
        operand: mir::Operand,
        target_type: &Type,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        match target_type {
            Type::Int => {
                let dest = self.alloc_stack_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::UnwrapValueInt { dest, src: operand });
                mir::Operand::Local(dest)
            }
            Type::Bool => {
                let dest = self.alloc_stack_local(Type::Bool, mir_func);
                self.emit_instruction(mir::InstructionKind::UnwrapValueBool { dest, src: operand });
                mir::Operand::Local(dest)
            }
            Type::Float => {
                let unboxed_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_UNBOX_FLOAT),
                    vec![operand],
                    Type::Float,
                    mir_func,
                );
                mir::Operand::Local(unboxed_local)
            }
            _ => operand,
        }
    }

    /// Emit `rt_list_get(list, index)` with correct typed unwrapping.
    ///
    /// After F.7c BigBang Step 2, `rt_list_get` returns the slot's tagged
    /// `Value` bit-pattern. Int/Bool callers must unwrap; Float callers must
    /// unbox. This helper centralises the dispatch so every list-element
    /// read site stays correct after Step 2.
    pub(crate) fn emit_list_get(
        &mut self,
        list_operand: mir::Operand,
        index_operand: mir::Operand,
        elem_ty: &Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        match elem_ty {
            Type::Int | Type::Bool | Type::Float => {
                let heap_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET),
                    vec![list_operand, index_operand],
                    Type::HeapAny,
                    mir_func,
                );
                match self.unbox_if_needed(mir::Operand::Local(heap_local), elem_ty, mir_func) {
                    mir::Operand::Local(id) => id,
                    _ => heap_local,
                }
            }
            _ => self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET),
                vec![list_operand, index_operand],
                elem_ty.clone(),
                mir_func,
            ),
        }
    }

    /// Get the type of a MIR operand.
    pub(crate) fn operand_type(&self, operand: &mir::Operand, mir_func: &mir::Function) -> Type {
        match operand {
            mir::Operand::Local(id) => mir_func.locals[id].ty.clone(),
            mir::Operand::Constant(c) => match c {
                mir::Constant::Int(_) => Type::Int,
                mir::Constant::Float(_) => Type::Float,
                mir::Constant::Bool(_) => Type::Bool,
                mir::Constant::Str(_) => Type::Str,
                mir::Constant::Bytes(_) => Type::Bytes,
                mir::Constant::None => Type::None,
            },
        }
    }

    /// Prefer the already-lowered MIR operand type when it is concrete; fall
    /// back to the seed/HIR hint only for dynamic `Any`/`HeapAny` cases.
    pub(crate) fn resolved_value_type_hint(
        &self,
        expr_id: hir::ExprId,
        operand: &mir::Operand,
        hir_module: &hir::Module,
        mir_func: &mir::Function,
    ) -> Type {
        let lowered = self.operand_type(operand, mir_func);
        if !matches!(lowered, Type::Any | Type::HeapAny) {
            return lowered;
        }
        self.seed_expr_type(expr_id, hir_module)
    }

    fn get_or_create_local(
        &mut self,
        var_id: VarId,
        var_type: Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        if let Some(local_id) = self.get_var_local(&var_id) {
            local_id
        } else {
            // Priority: refined container types > prescan unified type
            // (Area E §E.6) > per-site var_type. Refined types win so
            // `Dict(Any, Any)` tightened to `Dict(Str, Int)` by the
            // empty-container pass is preserved.
            let prescan = self
                .lowering_seed_info
                .current_local_seed_types
                .get(&var_id)
                .cloned()
                .filter(|ty| !is_useless_container_ty(ty));
            let ty = self
                .lowering_seed_info
                .refined_container_types
                .get(&var_id)
                .cloned()
                .or(prescan)
                .unwrap_or(var_type);
            let local_id = self.alloc_local_id();
            self.insert_var_local(var_id, local_id);
            mir_func.add_local(mir::Local {
                id: local_id,
                name: None,
                ty: ty.clone(),
                is_gc_root: ty.is_heap(),
            });
            local_id
        }
    }

    /// Resolve positional and keyword arguments against function parameters.
    /// Returns operands in the order matching function parameters.
    ///
    /// This is the main entry point for call argument resolution. It delegates to
    /// helper functions in the `call_resolution` module for specific tasks.
    ///
    /// If `target_func_id` is provided, mutable defaults (list, dict, set, class instances)
    /// are loaded from global storage instead of being re-evaluated, implementing Python's
    /// semantics where mutable defaults are evaluated once at function definition time.
    ///
    /// The `param_index_offset` adjusts the lookup index for mutable defaults when `params`
    /// doesn't include all original function parameters. For example, when calling `__init__`,
    /// the `self` parameter is skipped (offset=1) because user arguments don't include `self`,
    /// but `default_value_slots` uses indices relative to the original function parameters.
    #[allow(clippy::too_many_arguments)]
    fn resolve_call_args(
        &mut self,
        positional: &[crate::expressions::ExpandedArg],
        kwargs: &[hir::KeywordArg],
        params: &[hir::Param],
        target_func_id: Option<pyaot_utils::FuncId>,
        param_index_offset: usize,
        call_span: pyaot_utils::Span,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<Vec<mir::Operand>> {
        use crate::call_resolution::ParamClassification;
        use pyaot_diagnostics::CompilerError;

        // Step 1: Classify parameters by kind
        let param_class = ParamClassification::from_params(params);

        // Step 2: Lower all positional arguments (handling runtime unpacking)
        let all_positional =
            self.lower_positional_args(positional, &param_class, hir_module, mir_func)?;

        // Step 3: Match positional arguments to regular parameters
        let (mut resolved, extra_positional) =
            self.match_positional_to_params(all_positional, param_class.regular.len());

        // Step 4: Match keyword arguments to parameters
        let (mut kwonly_resolved, extra_keywords) = self.match_kwargs_to_params(
            kwargs,
            &param_class.regular,
            &param_class.kwonly,
            &mut resolved,
            hir_module,
            mir_func,
        )?;

        // Step 5: Process runtime **kwargs dict if present
        if let Some((dict_local, value_type)) = self.take_pending_kwargs() {
            self.process_runtime_kwargs_dict(
                dict_local,
                value_type,
                &param_class.regular,
                &param_class.kwonly,
                &mut resolved,
                &mut kwonly_resolved,
                param_class.kwarg.is_some(),
                hir_module,
                mir_func,
            )?;
        }

        // Step 6: Fill defaults for missing regular params
        // Pass target_func_id so mutable defaults can be loaded from storage
        self.fill_param_defaults(
            &mut resolved,
            &param_class.regular,
            target_func_id,
            param_index_offset,
            call_span,
            hir_module,
            mir_func,
        )?;

        // Step 7: Fill defaults for missing keyword-only params
        // For keyword-only params, compute their offset relative to the original function params:
        // [skipped params] + [regular params] + [*args if present] + [kwonly params]
        let kwonly_offset = param_index_offset
            + param_class.regular.len()
            + if param_class.vararg.is_some() { 1 } else { 0 };
        self.fill_param_defaults(
            &mut kwonly_resolved,
            &param_class.kwonly,
            target_func_id,
            kwonly_offset,
            call_span,
            hir_module,
            mir_func,
        )?;

        // Step 7.5: Box primitive values passed to Any-typed or Union-typed parameters.
        // Union parameters are boxed pointers at runtime, so primitive args must be boxed.
        //
        // Step 7.6 (inverse direction): when a concrete-primitive parameter (int/float/bool)
        // receives a boxed Union/Any argument (e.g. the result of `rt_obj_mul` flowing into
        // a `V(...)` constructor whose `__init__` expects `x: float`), unbox it. Without
        // this, Cranelift verifier rejects the i64-into-f64 call.
        for (i, operand_opt) in resolved.iter_mut().enumerate() {
            if let Some(operand) = operand_opt {
                if i < param_class.regular.len() {
                    let param = &param_class.regular[i];
                    if matches!(&param.ty, Some(Type::Any) | Some(Type::Union(_))) {
                        let arg_type = self.operand_type(operand, mir_func);
                        *operand =
                            self.box_primitive_if_needed(operand.clone(), &arg_type, mir_func);
                    } else if matches!(
                        &param.ty,
                        Some(Type::Int) | Some(Type::Float) | Some(Type::Bool)
                    ) {
                        let arg_type = self.operand_type(operand, mir_func);
                        if matches!(arg_type, Type::Union(_) | Type::Any | Type::HeapAny) {
                            *operand = self.unbox_if_needed(
                                operand.clone(),
                                param.ty.as_ref().expect("checked by outer match"),
                                mir_func,
                            );
                        }
                    }
                }
            }
        }
        for (i, operand_opt) in kwonly_resolved.iter_mut().enumerate() {
            if let Some(operand) = operand_opt {
                if i < param_class.kwonly.len() {
                    let param = &param_class.kwonly[i];
                    if matches!(&param.ty, Some(Type::Any) | Some(Type::Union(_))) {
                        let arg_type = self.operand_type(operand, mir_func);
                        *operand =
                            self.box_primitive_if_needed(operand.clone(), &arg_type, mir_func);
                    } else if matches!(
                        &param.ty,
                        Some(Type::Int) | Some(Type::Float) | Some(Type::Bool)
                    ) {
                        let arg_type = self.operand_type(operand, mir_func);
                        if matches!(arg_type, Type::Union(_) | Type::Any | Type::HeapAny) {
                            *operand = self.unbox_if_needed(
                                operand.clone(),
                                param.ty.as_ref().expect("checked by outer match"),
                                mir_func,
                            );
                        }
                    }
                }
            }
        }

        // Step 8: Build result starting with regular params
        let mut result: Vec<mir::Operand> = resolved.into_iter().flatten().collect();

        // Step 9: Build *args tuple from extra positional
        if let Some(vararg_param) = param_class.vararg {
            let tuple_local = self.build_varargs_tuple(extra_positional, vararg_param, mir_func);
            result.push(mir::Operand::Local(tuple_local));
        } else if !extra_positional.is_empty() {
            return Err(CompilerError::too_many_positional_arguments(
                param_class.regular.len(),
                positional.len(),
                call_span,
            ));
        }

        // Step 10: Add keyword-only parameters to result
        result.extend(kwonly_resolved.into_iter().flatten());

        // Step 11: Build **kwargs dict from extra keywords
        if param_class.kwarg.is_some() {
            let kwargs_dict = self.build_kwargs_dict(extra_keywords, mir_func);
            result.push(kwargs_dict);
        } else if !extra_keywords.is_empty() {
            let first_extra_name = extra_keywords
                .keys()
                .next()
                .expect("extra keywords must have at least one element");
            let kwarg_name = self.resolve(*first_extra_name).to_string();
            let kwarg_span = kwargs
                .iter()
                .find(|kw| kw.name == *first_extra_name)
                .map(|kw| kw.span)
                .unwrap_or_else(pyaot_utils::Span::dummy);
            return Err(CompilerError::unexpected_keyword_argument(
                kwarg_name, kwarg_span,
            ));
        } else {
            self.clear_pending_kwargs();
        }

        Ok(result)
    }

    /// Create a tuple from a vector of operands with proper element tag handling.
    /// `operand_types`: optional per-operand types for correct boxing when elem_tag is HEAP_OBJ.
    fn create_tuple_from_operands(
        &mut self,
        operands: &[mir::Operand],
        elem_type: &Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        self.create_tuple_from_operands_typed(operands, elem_type, None, mir_func)
    }

    /// Create a tuple with per-operand type information for correct boxing.
    fn create_tuple_from_operands_typed(
        &mut self,
        operands: &[mir::Operand],
        elem_type: &Type,
        operand_types: Option<&[Type]>,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        // After §F.7c: tuples store uniform tagged Values; box every primitive.
        let tuple_local = self.emit_runtime_call_gc(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_TUPLE),
            vec![mir::Operand::Constant(mir::Constant::Int(
                operands.len() as i64
            ))],
            Type::Tuple(vec![elem_type.clone()]),
            mir_func,
        );

        for (i, op) in operands.iter().enumerate() {
            let op_type = operand_types
                .and_then(|types| types.get(i))
                .unwrap_or(elem_type);
            let final_operand = self.box_primitive_if_needed(op.clone(), op_type, mir_func);

            self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_SET),
                vec![
                    mir::Operand::Local(tuple_local),
                    mir::Operand::Constant(mir::Constant::Int(i as i64)),
                    final_operand,
                ],
                Type::Tuple(vec![elem_type.clone()]),
                mir_func,
            );
        }

        tuple_local
    }

    /// Create a combined varargs tuple from extra positional operands + pre-built list tail tuple
    /// Used when calling f(1, 2, *list) where f has *args
    fn create_combined_varargs_tuple(
        &mut self,
        extra_positional: &[mir::Operand],
        list_tail_tuple: LocalId,
        elem_type: &Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        // First, create a tuple from the extra positional operands
        let prefix_tuple = self.create_tuple_from_operands(extra_positional, elem_type, mir_func);

        // Then, concatenate prefix_tuple + list_tail_tuple

        self.emit_runtime_call_gc(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_CONCAT),
            vec![
                mir::Operand::Local(prefix_tuple),
                mir::Operand::Local(list_tail_tuple),
            ],
            Type::Tuple(vec![Type::Any]),
            mir_func,
        )
    }

    /// Create a dict from keyword arguments
    fn create_dict_from_keywords(
        &mut self,
        keywords: &indexmap::IndexMap<pyaot_utils::InternedString, mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        // Emit: MakeDict(capacity)
        let dict_local = self.emit_runtime_call_gc(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_DICT),
            vec![mir::Operand::Constant(mir::Constant::Int(0))],
            Type::Dict(Box::new(Type::Str), Box::new(Type::Any)),
            mir_func,
        );

        // Emit: DictSet for each key-value pair
        for (key_name, value_op) in keywords {
            // key_name is already an InternedString, so we can use it directly
            let key_local = self.emit_runtime_call(
                mir::RuntimeFunc::MakeStr,
                vec![mir::Operand::Constant(mir::Constant::Str(*key_name))],
                Type::Str,
                mir_func,
            );
            self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_SET),
                vec![
                    mir::Operand::Local(dict_local),
                    mir::Operand::Local(key_local),
                    value_op.clone(),
                ],
                Type::Dict(Box::new(Type::Str), Box::new(Type::Any)),
                mir_func,
            );
        }

        dict_local
    }

    /// Convert a value from a dict for a specific parameter type.
    /// Dict values are stored as boxed pointers for GC safety.
    /// Primitive types (int, float, bool) need to be unboxed when retrieved.
    fn convert_dict_value_for_param(
        &mut self,
        dict_value_operand: mir::Operand,
        param_type: &Type,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        // Dict values are stored as boxed pointers for GC safety.
        // Primitive types need to be unboxed when retrieved.
        // Heap types (str, list, etc.) are stored as pointers and can be used directly.
        self.unbox_if_needed(dict_value_operand, param_type, mir_func)
    }
}
