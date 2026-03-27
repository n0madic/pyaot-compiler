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
mod type_planning;
mod utils;

pub use context::{CrossModuleClassInfo, FuncOrBuiltin, LoweredClassInfo, Lowering};

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

impl<'a> Lowering<'a> {
    /// Helper to emit a boxing instruction and return the boxed operand.
    fn emit_box_primitive(
        &mut self,
        operand: mir::Operand,
        result_ty: Type,
        runtime_func: mir::RuntimeFunc,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        let boxed_local = self.alloc_gc_local(result_ty, mir_func);
        let args = if matches!(runtime_func, mir::RuntimeFunc::BoxNone) {
            vec![]
        } else {
            vec![operand]
        };
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: boxed_local,
            func: runtime_func,
            args,
        });
        mir::Operand::Local(boxed_local)
    }

    /// Box a primitive value to an object pointer when needed.
    ///
    /// Primitives (Int, Bool, Float, None) must be boxed to `*mut Obj` for storage in
    /// dict keys/values, union-typed variables, and any other context requiring heap pointers.
    /// Heap types (Str, List, Dict, Tuple, Set, class instances, etc.) are already pointers
    /// and pass through unchanged.
    ///
    /// Uses `Type::Str` as a proxy for the pointer type since it maps to I64 in Cranelift.
    fn box_primitive_if_needed(
        &mut self,
        operand: mir::Operand,
        ty: &Type,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        match ty {
            Type::Int => {
                self.emit_box_primitive(operand, Type::Str, mir::RuntimeFunc::BoxInt, mir_func)
            }
            Type::Bool => {
                self.emit_box_primitive(operand, Type::Str, mir::RuntimeFunc::BoxBool, mir_func)
            }
            Type::Float => {
                self.emit_box_primitive(operand, Type::Str, mir::RuntimeFunc::BoxFloat, mir_func)
            }
            Type::None => {
                self.emit_box_primitive(operand, Type::Str, mir::RuntimeFunc::BoxNone, mir_func)
            }
            // All heap types are already object pointers — no boxing needed
            _ => operand,
        }
    }

    /// Get the type of a MIR operand
    fn operand_type(&self, operand: &mir::Operand, mir_func: &mir::Function) -> Type {
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

    fn get_or_create_local(
        &mut self,
        var_id: VarId,
        var_type: Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        if let Some(local_id) = self.get_var_local(&var_id) {
            local_id
        } else {
            let local_id = self.alloc_local_id();
            self.insert_var_local(var_id, local_id);
            mir_func.add_local(mir::Local {
                id: local_id,
                name: None,
                ty: var_type.clone(),
                is_gc_root: var_type.is_heap(),
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
        for (i, operand_opt) in resolved.iter_mut().enumerate() {
            if let Some(operand) = operand_opt {
                if i < param_class.regular.len() {
                    let param = &param_class.regular[i];
                    if matches!(&param.ty, Some(Type::Any) | Some(Type::Union(_))) {
                        let arg_type = self.operand_type(operand, mir_func);
                        *operand =
                            self.box_primitive_if_needed(operand.clone(), &arg_type, mir_func);
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
        let tuple_local = self.alloc_gc_local(Type::Tuple(vec![elem_type.clone()]), mir_func);

        // Determine elem_tag based on element types.
        // Use ELEM_RAW_INT (1) when no element needs GC tracing,
        // ELEM_HEAP_OBJ (0) when any element is a heap type.
        let elem_tag: i64 = if *elem_type == Type::Int {
            1 // ELEM_RAW_INT — all elements are ints
        } else if let Some(types) = operand_types {
            // Per-operand types available: check if any needs GC
            if types.iter().any(Type::is_heap) {
                0 // ELEM_HEAP_OBJ
            } else {
                1 // ELEM_RAW_INT — all operands are primitives
            }
        } else {
            0 // ELEM_HEAP_OBJ — unknown types, be safe
        };

        // Emit: MakeTuple(size, elem_tag)
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: tuple_local,
            func: mir::RuntimeFunc::MakeTuple,
            args: vec![
                mir::Operand::Constant(mir::Constant::Int(operands.len() as i64)),
                mir::Operand::Constant(mir::Constant::Int(elem_tag)),
            ],
        });

        // Emit: TupleSet for each element
        for (i, op) in operands.iter().enumerate() {
            // Box primitive values when elem_tag is ELEM_HEAP_OBJ
            let final_operand = if elem_tag == 0 {
                // Use per-operand type if available, otherwise use common elem_type
                let op_type = operand_types
                    .and_then(|types| types.get(i))
                    .unwrap_or(elem_type);
                self.box_primitive_if_needed(op.clone(), op_type, mir_func)
            } else {
                op.clone() // ELEM_RAW_INT, already i64
            };

            let dummy_local =
                self.alloc_and_add_local(Type::Tuple(vec![elem_type.clone()]), mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::TupleSet,
                args: vec![
                    mir::Operand::Local(tuple_local),
                    mir::Operand::Constant(mir::Constant::Int(i as i64)),
                    final_operand,
                ],
            });
        }

        tuple_local
    }

    /// Emit a TupleSetHeapMask instruction for a tuple with mixed types.
    /// Computes a bitmask from operand types and emits the runtime call.
    fn emit_heap_field_mask(
        &mut self,
        tuple_local: pyaot_utils::LocalId,
        operand_types: &[Type],
        mir_func: &mut mir::Function,
    ) {
        let mut mask: u64 = 0;
        for (i, ty) in operand_types.iter().enumerate() {
            if i < 64 && ty.is_heap() {
                mask |= 1u64 << i;
            }
        }
        let dummy = self.alloc_and_add_local(Type::None, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: dummy,
            func: mir::RuntimeFunc::TupleSetHeapMask,
            args: vec![
                mir::Operand::Local(tuple_local),
                mir::Operand::Constant(mir::Constant::Int(mask as i64)),
            ],
        });
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
        let result_local = self.alloc_gc_local(Type::Tuple(vec![Type::Any]), mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::TupleConcat,
            args: vec![
                mir::Operand::Local(prefix_tuple),
                mir::Operand::Local(list_tail_tuple),
            ],
        });

        result_local
    }

    /// Create a dict from keyword arguments
    fn create_dict_from_keywords(
        &mut self,
        keywords: &indexmap::IndexMap<pyaot_utils::InternedString, mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        let dict_local = self.alloc_gc_local(
            Type::Dict(Box::new(Type::Str), Box::new(Type::Any)),
            mir_func,
        );

        // Emit: MakeDict(capacity)
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: dict_local,
            func: mir::RuntimeFunc::MakeDict,
            args: vec![mir::Operand::Constant(mir::Constant::Int(0))],
        });

        // Emit: DictSet for each key-value pair
        for (key_name, value_op) in keywords {
            // key_name is already an InternedString, so we can use it directly
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
                    mir::Operand::Local(dict_local),
                    mir::Operand::Local(key_local),
                    value_op.clone(),
                ],
            });
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
        match param_type {
            Type::Int => {
                let unboxed_local = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: unboxed_local,
                    func: mir::RuntimeFunc::UnboxInt,
                    args: vec![dict_value_operand],
                });
                mir::Operand::Local(unboxed_local)
            }
            Type::Float => {
                let unboxed_local = self.alloc_and_add_local(Type::Float, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: unboxed_local,
                    func: mir::RuntimeFunc::UnboxFloat,
                    args: vec![dict_value_operand],
                });
                mir::Operand::Local(unboxed_local)
            }
            Type::Bool => {
                let unboxed_local = self.alloc_and_add_local(Type::Bool, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: unboxed_local,
                    func: mir::RuntimeFunc::UnboxBool,
                    args: vec![dict_value_operand],
                });
                mir::Operand::Local(unboxed_local)
            }
            _ => {
                // Heap types (str, list, etc.) are stored as pointers and can be used directly
                dict_value_operand
            }
        }
    }
}
