//! *args/**kwargs building and runtime dict extraction.

use crate::context::Lowering;
use indexmap::IndexMap;
use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{InternedString, LocalId};

impl<'a> Lowering<'a> {
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

                    self.emit_runtime_call(
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_SET),
                        vec![
                            mir::Operand::Local(remaining_dict),
                            mir::Operand::Local(key_local),
                            value_op.clone(),
                        ],
                        Type::Dict(Box::new(Type::Str), Box::new(Type::Any)),
                        mir_func,
                    );
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
                func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_COPY),
                args: vec![mir::Operand::Local(dict_local)],
            });

            // Remove consumed keys
            for key_local in &consumed_keys {
                self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_POP),
                    vec![
                        mir::Operand::Local(remaining_dict),
                        mir::Operand::Local(*key_local),
                    ],
                    Type::HeapAny,
                    mir_func,
                );
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
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_CONTAINS),
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
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_GET),
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
