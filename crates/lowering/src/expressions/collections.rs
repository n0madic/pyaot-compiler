//! Collection expression lowering: List, Tuple, Dict, Set

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a list literal expression: [e1, e2, ...]
    pub(super) fn lower_list(
        &mut self,
        elements: &[hir::ExprId],
        expr: &hir::Expr,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let mut list_type = expr.ty.clone().unwrap_or(Type::Any);

        if matches!(list_type, Type::Any) {
            if let Some(Type::List(ref expected_elem)) = self.codegen.expected_type {
                list_type = Type::List(expected_elem.clone());
            }
        }

        // For empty lists with unknown element type, use the expected type from
        // the assignment context (e.g., `x: list[int] = []` or `x = []` where x
        // was previously declared as list[int]).
        if elements.is_empty() {
            if let Some(Type::List(ref expected_elem)) = self.codegen.expected_type {
                match &list_type {
                    Type::Any => {
                        list_type = Type::List(expected_elem.clone());
                    }
                    Type::List(elem_ty) if **elem_ty == Type::Any => {
                        list_type = Type::List(expected_elem.clone());
                    }
                    _ => {}
                }
            }
        } else if matches!(list_type, Type::Any) {
            let inferred_elem = elements.iter().fold(None, |acc: Option<Type>, elem_id| {
                let next = self.seed_expr_type(*elem_id, hir_module);
                Some(match acc {
                    Some(prev) => Type::unify_field_type(&prev, &next),
                    None => next,
                })
            });
            list_type = Type::List(Box::new(inferred_elem.unwrap_or(Type::Any)));
        }

        // Determine elem_tag based on element type
        let elem_type = match &list_type {
            Type::List(elem_ty) => (**elem_ty).clone(),
            _ => Type::Any,
        };
        // NOTE: ELEM_RAW_BOOL (2) is not used because ListPush requires pointer parameter,
        // and converting i8 -> i64 in lowering is complex. Bool lists use ELEM_HEAP_OBJ.
        let elem_tag: i64 = match &elem_type {
            Type::Int => 1, // ELEM_RAW_INT
            _ => 0,         // ELEM_HEAP_OBJ (Bool, Float, Str, Union, List, etc.)
        };

        // Create list with capacity and elem_tag
        let capacity = elements.len() as i64;
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_LIST),
            vec![
                mir::Operand::Constant(mir::Constant::Int(capacity)),
                mir::Operand::Constant(mir::Constant::Int(elem_tag)),
            ],
            list_type.clone(),
            mir_func,
        );

        // Push each element
        for elem_id in elements {
            let elem_expr = &hir_module.exprs[*elem_id];
            let elem_operand = self.lower_expr_expecting(
                elem_expr,
                Some(elem_type.clone()),
                hir_module,
                mir_func,
            )?;
            let actual_elem_type = self.seed_expr_type(*elem_id, hir_module);
            let elem_operand = if elem_type == Type::Float {
                self.coerce_to_field_type(elem_operand, &actual_elem_type, &elem_type, mir_func)
            } else {
                elem_operand
            };

            // Box elements before pushing to list:
            // - Float elements always need boxing
            // - Union element types need boxing for primitive values
            // - Bool elements: box for heap lists, extend to i64 for raw bool lists
            let push_operand = if elem_type == Type::Float {
                self.box_primitive_if_needed(elem_operand, &Type::Float, mir_func)
            } else if elem_type == Type::Bool {
                // ELEM_HEAP_OBJ: box bools
                let boxed_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_BOX_BOOL),
                    vec![elem_operand],
                    Type::HeapAny,
                    mir_func,
                );
                mir::Operand::Local(boxed_local)
            } else if matches!(elem_type, Type::Union(_)) {
                // Box primitives for Union element types
                self.box_primitive_if_needed(elem_operand, &actual_elem_type, mir_func)
            } else {
                elem_operand
            };

            self.emit_runtime_call_void(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_PUSH),
                vec![mir::Operand::Local(result_local), push_operand],
                mir_func,
            );
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower a tuple literal expression: (e1, e2, ...)
    pub(super) fn lower_tuple(
        &mut self,
        elements: &[hir::ExprId],
        expr: &hir::Expr,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let mut tuple_type = expr.ty.clone().unwrap_or(Type::Any);

        if matches!(tuple_type, Type::Any) {
            match self.codegen.expected_type.clone() {
                Some(Type::Tuple(expected)) => {
                    tuple_type = Type::Tuple(expected);
                }
                Some(Type::TupleVar(expected)) => {
                    tuple_type = Type::TupleVar(expected);
                }
                _ => {}
            }
        }
        if matches!(tuple_type, Type::Any) {
            tuple_type = Type::Tuple(
                elements
                    .iter()
                    .map(|elem_id| self.seed_expr_type(*elem_id, hir_module))
                    .collect(),
            );
        }

        // Determine elem_tag for tuple
        // If all elements have the same primitive type, use that tag
        // Otherwise use ELEM_HEAP_OBJ (0)
        // NOTE: ELEM_RAW_BOOL (2) is not used because TupleSet requires pointer parameter,
        // and converting i8 -> i64 in lowering is complex. Bool tuples use ELEM_HEAP_OBJ.
        let elem_tag: i64 = if let Type::Tuple(ref elem_types) = tuple_type {
            if !elem_types.is_empty() && elem_types.iter().all(|t| *t == Type::Int) {
                1 // ELEM_RAW_INT
            } else {
                0 // ELEM_HEAP_OBJ (including bool tuples)
            }
        } else if matches!(tuple_type, Type::TupleVar(ref elem_type) if **elem_type == Type::Int) {
            1 // ELEM_RAW_INT for homogeneous tuple[int, ...]
        } else {
            0
        };

        // Create tuple with size and elem_tag
        let size = elements.len() as i64;
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_TUPLE),
            vec![
                mir::Operand::Constant(mir::Constant::Int(size)),
                mir::Operand::Constant(mir::Constant::Int(elem_tag)),
            ],
            tuple_type.clone(),
            mir_func,
        );

        // Set each element
        for (i, elem_id) in elements.iter().enumerate() {
            let elem_expr = &hir_module.exprs[*elem_id];
            let expected_elem_type = match &tuple_type {
                Type::Tuple(elem_types) => elem_types.get(i).cloned(),
                Type::TupleVar(elem_type) => Some((**elem_type).clone()),
                _ => None,
            };
            let elem_operand =
                self.lower_expr_expecting(elem_expr, expected_elem_type, hir_module, mir_func)?;

            // Box primitive values when elem_tag is ELEM_HEAP_OBJ
            let final_operand = if elem_tag == 0 {
                // ELEM_HEAP_OBJ - need to box primitives
                let elem_type = self.seed_expr_type(*elem_id, hir_module);
                match elem_type {
                    Type::Int => {
                        let boxed_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_BOX_INT),
                            vec![elem_operand],
                            Type::HeapAny,
                            mir_func,
                        );
                        mir::Operand::Local(boxed_local)
                    }
                    Type::Bool => {
                        let boxed_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_BOX_BOOL),
                            vec![elem_operand],
                            Type::HeapAny,
                            mir_func,
                        );
                        mir::Operand::Local(boxed_local)
                    }
                    Type::Float => {
                        let boxed_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_BOX_FLOAT,
                            ),
                            vec![elem_operand],
                            Type::HeapAny,
                            mir_func,
                        );
                        mir::Operand::Local(boxed_local)
                    }
                    _ => elem_operand, // Already a heap object
                }
            } else {
                elem_operand // ELEM_RAW_INT, already i64
            };

            self.emit_runtime_call_void(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_SET),
                vec![
                    mir::Operand::Local(result_local),
                    mir::Operand::Constant(mir::Constant::Int(i as i64)),
                    final_operand,
                ],
                mir_func,
            );
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower a dict literal expression: {k1: v1, k2: v2, ...}
    pub(super) fn lower_dict(
        &mut self,
        pairs: &[(hir::ExprId, hir::ExprId)],
        expr: &hir::Expr,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let mut dict_type = expr.ty.clone().unwrap_or(Type::Any);

        if matches!(dict_type, Type::Any) {
            if let Some(Type::Dict(ref expected_key, ref expected_val)) = self.codegen.expected_type
            {
                dict_type = Type::Dict(expected_key.clone(), expected_val.clone());
            }
        }

        // Bidirectional: for empty dicts, use expected_type from context
        if pairs.is_empty() {
            if let Some(Type::Dict(ref expected_key, ref expected_val)) = self.codegen.expected_type
            {
                match &dict_type {
                    Type::Any => {
                        dict_type = Type::Dict(expected_key.clone(), expected_val.clone());
                    }
                    Type::Dict(key_ty, val_ty)
                        if **key_ty == Type::Any && **val_ty == Type::Any =>
                    {
                        dict_type = Type::Dict(expected_key.clone(), expected_val.clone());
                    }
                    _ => {}
                }
            }
        } else if matches!(dict_type, Type::Any) {
            let inferred_key = pairs.iter().fold(None, |acc: Option<Type>, (key_id, _)| {
                let next = self.seed_expr_type(*key_id, hir_module);
                Some(match acc {
                    Some(prev) => Type::unify_field_type(&prev, &next),
                    None => next,
                })
            });
            let inferred_val = pairs.iter().fold(None, |acc: Option<Type>, (_, value_id)| {
                let next = self.seed_expr_type(*value_id, hir_module);
                Some(match acc {
                    Some(prev) => Type::unify_field_type(&prev, &next),
                    None => next,
                })
            });
            dict_type = Type::Dict(
                Box::new(inferred_key.unwrap_or(Type::Any)),
                Box::new(inferred_val.unwrap_or(Type::Any)),
            );
        }

        // Create dict with capacity
        let capacity = pairs.len().max(8) as i64;
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_DICT),
            vec![mir::Operand::Constant(mir::Constant::Int(capacity))],
            dict_type.clone(),
            mir_func,
        );

        // Insert each key-value pair
        let (dict_key_type, dict_value_type) = match &dict_type {
            Type::Dict(key_ty, value_ty) => ((**key_ty).clone(), (**value_ty).clone()),
            _ => (Type::Any, Type::Any),
        };
        for (key_id, value_id) in pairs {
            let key_type = self.seed_expr_type(*key_id, hir_module);
            let key_expr = &hir_module.exprs[*key_id];
            let key_operand = self.lower_expr_expecting(
                key_expr,
                Some(dict_key_type.clone()),
                hir_module,
                mir_func,
            )?;
            let key_operand = if dict_key_type == Type::Float {
                self.coerce_to_field_type(key_operand, &key_type, &dict_key_type, mir_func)
            } else {
                key_operand
            };

            // Box non-heap keys (int, bool) so dict can use them as object pointers
            let boxed_key = if dict_key_type == Type::Float {
                self.box_primitive_if_needed(key_operand, &dict_key_type, mir_func)
            } else {
                self.box_primitive_if_needed(key_operand, &key_type, mir_func)
            };

            let value_expr = &hir_module.exprs[*value_id];
            let value_operand = self.lower_expr_expecting(
                value_expr,
                Some(dict_value_type.clone()),
                hir_module,
                mir_func,
            )?;
            let actual_value_type = self.seed_expr_type(*value_id, hir_module);
            let value_operand = if dict_value_type == Type::Float {
                self.coerce_to_field_type(
                    value_operand,
                    &actual_value_type,
                    &dict_value_type,
                    mir_func,
                )
            } else {
                value_operand
            };

            // Box primitive values (all dict values must be heap pointers for GC)
            let boxed_value = if dict_value_type == Type::Float {
                self.box_primitive_if_needed(value_operand, &dict_value_type, mir_func)
            } else {
                self.box_primitive_if_needed(value_operand, &actual_value_type, mir_func)
            };

            self.emit_runtime_call_void(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_SET),
                vec![mir::Operand::Local(result_local), boxed_key, boxed_value],
                mir_func,
            );
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower a set literal expression: {e1, e2, ...}
    pub(super) fn lower_set(
        &mut self,
        elements: &[hir::ExprId],
        expr: &hir::Expr,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let mut set_type = expr.ty.clone().unwrap_or(Type::Any);

        if matches!(set_type, Type::Any) {
            if let Some(Type::Set(ref expected_elem)) = self.codegen.expected_type {
                set_type = Type::Set(expected_elem.clone());
            }
        }

        // Bidirectional: for empty sets, use expected_type from context
        if elements.is_empty() {
            if let Some(Type::Set(ref expected_elem)) = self.codegen.expected_type {
                match &set_type {
                    Type::Any => {
                        set_type = Type::Set(expected_elem.clone());
                    }
                    Type::Set(elem_ty) if **elem_ty == Type::Any => {
                        set_type = Type::Set(expected_elem.clone());
                    }
                    _ => {}
                }
            }
        } else if matches!(set_type, Type::Any) {
            let inferred_elem = elements.iter().fold(None, |acc: Option<Type>, elem_id| {
                let next = self.seed_expr_type(*elem_id, hir_module);
                Some(match acc {
                    Some(prev) => Type::unify_field_type(&prev, &next),
                    None => next,
                })
            });
            set_type = Type::Set(Box::new(inferred_elem.unwrap_or(Type::Any)));
        }

        // Create set with capacity
        let capacity = elements.len().max(8) as i64;
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_SET),
            vec![mir::Operand::Constant(mir::Constant::Int(capacity))],
            set_type.clone(),
            mir_func,
        );

        // Add each element
        for elem_id in elements {
            let elem_type = self.seed_expr_type(*elem_id, hir_module);
            let elem_expr = &hir_module.exprs[*elem_id];
            let expected_elem_type = match &set_type {
                Type::Set(inner) => Some((**inner).clone()),
                _ => None,
            };
            let elem_operand =
                self.lower_expr_expecting(elem_expr, expected_elem_type, hir_module, mir_func)?;
            let elem_operand = if matches!(set_type, Type::Set(ref inner) if **inner == Type::Float)
            {
                self.coerce_to_field_type(elem_operand, &elem_type, &Type::Float, mir_func)
            } else {
                elem_operand
            };

            // Box non-heap elements (int, bool) so set can use them as object pointers
            let boxed_elem = if matches!(set_type, Type::Set(ref inner) if **inner == Type::Float) {
                self.box_primitive_if_needed(elem_operand, &Type::Float, mir_func)
            } else {
                self.box_primitive_if_needed(elem_operand, &elem_type, mir_func)
            };

            self.emit_runtime_call_void(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_ADD),
                vec![mir::Operand::Local(result_local), boxed_elem],
                mir_func,
            );
        }

        Ok(mir::Operand::Local(result_local))
    }
}
