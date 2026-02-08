//! Lowering from HIR to MIR

#![forbid(unsafe_code)]

mod call_resolution;
mod class_metadata;
mod context;
mod exceptions;
mod expressions;
mod generators;
mod lambda_inference;
mod narrowing;
mod runtime_selector;
mod statements;
mod type_inference;
mod utils;

pub use context::{CrossModuleClassInfo, FuncOrBuiltin, LoweredClassInfo, Lowering};

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{LocalId, VarId};

use utils::is_heap_type;

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

    /// Dict keys need to be object pointers (primitives need to be boxed).
    /// Use Type::Str as proxy for pointer type since it maps to I64 in Cranelift.
    fn box_dict_key_if_needed(
        &mut self,
        key_operand: mir::Operand,
        key_type: &Type,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        match key_type {
            Type::Int => {
                self.emit_box_primitive(key_operand, Type::Str, mir::RuntimeFunc::BoxInt, mir_func)
            }
            Type::Bool => {
                self.emit_box_primitive(key_operand, Type::Str, mir::RuntimeFunc::BoxBool, mir_func)
            }
            Type::Float => self.emit_box_primitive(
                key_operand,
                Type::Str,
                mir::RuntimeFunc::BoxFloat,
                mir_func,
            ),
            Type::None => {
                self.emit_box_primitive(key_operand, Type::Str, mir::RuntimeFunc::BoxNone, mir_func)
            }
            // Str, Tuple, and other heap types are already object pointers
            _ => key_operand,
        }
    }

    /// Dict values need to be object pointers for GC to track them correctly.
    /// Primitives (Int, Bool, Float, None) must be boxed.
    fn box_dict_value_if_needed(
        &mut self,
        value_operand: mir::Operand,
        value_type: &Type,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        // Use Type::Str as proxy for pointer type (maps to I64 in Cranelift)
        match value_type {
            Type::Int => self.emit_box_primitive(
                value_operand,
                Type::Str,
                mir::RuntimeFunc::BoxInt,
                mir_func,
            ),
            Type::Bool => self.emit_box_primitive(
                value_operand,
                Type::Str,
                mir::RuntimeFunc::BoxBool,
                mir_func,
            ),
            Type::Float => self.emit_box_primitive(
                value_operand,
                Type::Str,
                mir::RuntimeFunc::BoxFloat,
                mir_func,
            ),
            Type::None => self.emit_box_primitive(
                value_operand,
                Type::Str,
                mir::RuntimeFunc::BoxNone,
                mir_func,
            ),
            // Str, List, Dict, Tuple, Set, class instances, etc. are already object pointers
            _ => value_operand,
        }
    }

    /// Box a primitive value when assigned to a Union-typed variable.
    /// Union values are stored as boxed pointers (*mut Obj).
    /// Note: We use Type::Str as a proxy for pointer types since it maps to I64 in Cranelift.
    fn box_value_for_union(
        &mut self,
        value_operand: mir::Operand,
        value_type: &Type,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        // Use Str as proxy for pointer type (maps to I64 in Cranelift)
        match value_type {
            Type::Int => self.emit_box_primitive(
                value_operand,
                Type::Str,
                mir::RuntimeFunc::BoxInt,
                mir_func,
            ),
            Type::Bool => self.emit_box_primitive(
                value_operand,
                Type::Str,
                mir::RuntimeFunc::BoxBool,
                mir_func,
            ),
            Type::Float => self.emit_box_primitive(
                value_operand,
                Type::Str,
                mir::RuntimeFunc::BoxFloat,
                mir_func,
            ),
            Type::None => self.emit_box_primitive(
                value_operand,
                Type::Str,
                mir::RuntimeFunc::BoxNone,
                mir_func,
            ),
            // Heap types are already pointers - no boxing needed
            _ => value_operand,
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
                is_gc_root: is_heap_type(&var_type),
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
            hir_module,
            mir_func,
        )?;

        // Step 7.5: Box primitive values passed to Any-typed parameters
        for (i, operand_opt) in resolved.iter_mut().enumerate() {
            if let Some(operand) = operand_opt {
                if i < param_class.regular.len() {
                    let param = &param_class.regular[i];
                    if let Some(Type::Any) = &param.ty {
                        let arg_type = self.operand_type(operand, mir_func);
                        *operand = self.box_value_for_union(operand.clone(), &arg_type, mir_func);
                    }
                }
            }
        }
        for (i, operand_opt) in kwonly_resolved.iter_mut().enumerate() {
            if let Some(operand) = operand_opt {
                if i < param_class.kwonly.len() {
                    let param = &param_class.kwonly[i];
                    if let Some(Type::Any) = &param.ty {
                        let arg_type = self.operand_type(operand, mir_func);
                        *operand = self.box_value_for_union(operand.clone(), &arg_type, mir_func);
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
                pyaot_utils::Span::dummy(),
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

    /// Create a tuple from a vector of operands with proper element tag handling
    fn create_tuple_from_operands(
        &mut self,
        operands: &[mir::Operand],
        elem_type: &Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        let tuple_local = self.alloc_gc_local(Type::Tuple(vec![elem_type.clone()]), mir_func);

        // Determine elem_tag based on element type
        // Use ELEM_RAW_INT (1) for int tuples, ELEM_HEAP_OBJ (0) for others
        let elem_tag: i64 = if *elem_type == Type::Int {
            1 // ELEM_RAW_INT
        } else {
            0 // ELEM_HEAP_OBJ
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
                match elem_type {
                    Type::Bool => {
                        let boxed_local = self.alloc_and_add_local(Type::Str, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::BoxBool,
                            args: vec![op.clone()],
                        });
                        mir::Operand::Local(boxed_local)
                    }
                    Type::Float => {
                        let boxed_local = self.alloc_and_add_local(Type::Str, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::BoxFloat,
                            args: vec![op.clone()],
                        });
                        mir::Operand::Local(boxed_local)
                    }
                    _ => op.clone(), // Already heap objects or int (which shouldn't happen with elem_tag 0)
                }
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
