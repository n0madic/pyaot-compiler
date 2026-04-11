//! Tuple/list unpacking assignment lowering
//!
//! Handles: UnpackAssign (a, *rest, b = value), NestedUnpackAssign ((a, (b, c)) = value)

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower an unpacking assignment: a, b = value or a, *rest, b = value
    ///
    /// Supports extended unpacking with starred expression:
    /// - `a, *rest = [1, 2, 3, 4]` → a=1, rest=[2, 3, 4]
    /// - `*rest, b = [1, 2, 3, 4]` → rest=[1, 2, 3], b=4
    /// - `a, *mid, b = [1, 2, 3, 4]` → a=1, mid=[2, 3], b=4
    pub(crate) fn lower_unpack_assign(
        &mut self,
        before_star: &[VarId],
        starred: Option<&VarId>,
        after_star: &[VarId],
        value: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let expr = &hir_module.exprs[value];
        let value_type = self.get_type_of_expr_id(value, hir_module);

        // Lower the RHS expression once
        let value_operand = self.lower_expr(expr, hir_module, mir_func)?;

        // Determine the slice function based on source type
        // For starred unpacking, we always produce a list, so tuples use TupleSliceToList
        let (is_tuple, slice_func) = match &value_type {
            Type::List(_) => (
                false,
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_SLICE),
            ),
            _ => (
                true,
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_SLICE_TO_LIST),
            ),
        };

        // Helper to determine element type
        let get_elem_type = |index: usize| -> Type {
            match &value_type {
                Type::Tuple(elem_types) => {
                    if index < elem_types.len() {
                        elem_types[index].clone()
                    } else {
                        Type::Any
                    }
                }
                Type::List(elem_ty) => (**elem_ty).clone(),
                _ => Type::Any,
            }
        };

        // Helper to determine element type for negative index
        let get_elem_type_neg = |neg_index: i64| -> Type {
            match &value_type {
                Type::Tuple(elem_types) => {
                    let len = elem_types.len() as i64;
                    let actual_idx = (len + neg_index) as usize;
                    if actual_idx < elem_types.len() {
                        elem_types[actual_idx].clone()
                    } else {
                        Type::Any
                    }
                }
                Type::List(elem_ty) => (**elem_ty).clone(),
                _ => Type::Any,
            }
        };

        // Store all extracted values in temps first (for parallel assignment safety)
        let mut temp_locals = Vec::new();

        // 1. Extract before_star elements with positive indices
        for (i, _target) in before_star.iter().enumerate() {
            let index_operand = mir::Operand::Constant(mir::Constant::Int(i as i64));
            let elem_type = get_elem_type(i);

            let temp_local = self.alloc_and_add_local(elem_type.clone(), mir_func);

            let get_func = if is_tuple {
                crate::type_dispatch::tuple_get_func(&elem_type)
            } else {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET)
            };

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: temp_local,
                func: get_func,
                args: vec![value_operand.clone(), index_operand],
            });

            temp_locals.push((temp_local, elem_type));
        }

        // 2. Extract starred portion (if any) using slice
        if let Some(_starred_var) = starred {
            // Slice indices: start = before_star.len(), end = -after_star.len() (or None if 0)
            let start_idx = before_star.len() as i64;
            let end_idx = if after_star.is_empty() {
                // No after_star: slice to the end (use a large number, runtime will clamp)
                i64::MAX
            } else {
                -(after_star.len() as i64)
            };

            // Starred always produces a list
            let starred_elem_type = match &value_type {
                Type::List(elem_ty) => (**elem_ty).clone(),
                Type::Tuple(elem_types) => {
                    // Union of all types in the starred portion
                    let middle_start = before_star.len();
                    let middle_end = elem_types.len().saturating_sub(after_star.len());
                    if middle_start < middle_end {
                        let middle_types: Vec<_> = elem_types[middle_start..middle_end].to_vec();
                        Type::normalize_union(middle_types)
                    } else {
                        Type::Any
                    }
                }
                _ => Type::Any,
            };
            let starred_type = Type::List(Box::new(starred_elem_type));

            let temp_local = self.emit_runtime_call(
                slice_func,
                vec![
                    value_operand.clone(),
                    mir::Operand::Constant(mir::Constant::Int(start_idx)),
                    mir::Operand::Constant(mir::Constant::Int(end_idx)),
                ],
                starred_type.clone(),
                mir_func,
            );

            temp_locals.push((temp_local, starred_type));
        }

        // 3. Extract after_star elements with negative indices
        for (i, _target) in after_star.iter().enumerate() {
            // Negative index: -(after_star.len() - i)
            let neg_index = -((after_star.len() - i) as i64);
            let index_operand = mir::Operand::Constant(mir::Constant::Int(neg_index));
            let elem_type = get_elem_type_neg(neg_index);

            let temp_local = self.alloc_and_add_local(elem_type.clone(), mir_func);

            let get_func = if is_tuple {
                crate::type_dispatch::tuple_get_func(&elem_type)
            } else {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET)
            };

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: temp_local,
                func: get_func,
                args: vec![value_operand.clone(), index_operand],
            });

            temp_locals.push((temp_local, elem_type));
        }

        // 4. Copy from temps to actual target variables
        // Verify that we have the expected number of temp locals before iterating
        assert_eq!(
            temp_locals.len(),
            before_star.len() + starred.is_some() as usize + after_star.len(),
            "temp_locals count mismatch in unpacking assignment"
        );

        let mut temp_iter = temp_locals.into_iter();

        // Copy to before_star targets
        for target in before_star {
            let (temp_local, elem_type) = temp_iter
                .next()
                .expect("unpacking requires at least one temporary for before_star targets");
            self.insert_var_type(*target, elem_type.clone());
            let dest_local = self.get_or_create_local(*target, elem_type, mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: dest_local,
                src: mir::Operand::Local(temp_local),
            });
        }

        // Copy to starred target (if any)
        if let Some(starred_var) = starred {
            let (temp_local, starred_type) = temp_iter
                .next()
                .expect("unpacking requires at least one temporary for starred target");
            self.insert_var_type(*starred_var, starred_type.clone());
            let dest_local = self.get_or_create_local(*starred_var, starred_type, mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: dest_local,
                src: mir::Operand::Local(temp_local),
            });
        }

        // Copy to after_star targets
        for target in after_star {
            let (temp_local, elem_type) = temp_iter
                .next()
                .expect("unpacking requires at least one temporary for after_star targets");
            self.insert_var_type(*target, elem_type.clone());
            let dest_local = self.get_or_create_local(*target, elem_type, mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: dest_local,
                src: mir::Operand::Local(temp_local),
            });
        }

        Ok(())
    }

    /// Lower nested unpacking assignment: (a, (b, c)) = value
    /// Supports arbitrary depth nesting through recursive extraction
    pub(crate) fn lower_nested_unpack_assign(
        &mut self,
        targets: &[hir::UnpackTarget],
        value: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let expr = &hir_module.exprs[value];
        let value_type = self.get_type_of_expr_id(value, hir_module);

        // Lower the RHS expression once
        let value_operand = self.lower_expr(expr, hir_module, mir_func)?;

        // Recursively extract nested targets
        self.lower_nested_recursive(targets, value_operand, &value_type, mir_func)?;

        Ok(())
    }

    /// Recursively lower nested unpacking pattern - handles arbitrary nesting depth
    pub(super) fn lower_nested_recursive(
        &mut self,
        targets: &[hir::UnpackTarget],
        source_operand: mir::Operand,
        source_type: &Type,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let is_tuple = matches!(source_type, Type::Tuple(_));
        let elem_types: Vec<Type> = match source_type {
            Type::Tuple(types) => types.clone(),
            Type::List(inner) => vec![(**inner).clone(); targets.len()],
            _ => vec![Type::Any; targets.len()],
        };

        for (i, target) in targets.iter().enumerate() {
            let elem_type = elem_types.get(i).cloned().unwrap_or(Type::Any);

            match target {
                hir::UnpackTarget::Var(var_id) => {
                    // Simple variable - extract and assign directly
                    let get_func = if is_tuple {
                        crate::type_dispatch::tuple_get_func(&elem_type)
                    } else {
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET)
                    };

                    self.insert_var_type(*var_id, elem_type.clone());
                    let dest_local = if let Some(existing) = self.get_var_local(var_id) {
                        existing
                    } else {
                        let new_local = self.alloc_and_add_local(elem_type.clone(), mir_func);
                        self.insert_var_local(*var_id, new_local);
                        new_local
                    };

                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: dest_local,
                        func: get_func,
                        args: vec![
                            source_operand.clone(),
                            mir::Operand::Constant(mir::Constant::Int(i as i64)),
                        ],
                    });
                }
                hir::UnpackTarget::Nested(nested_targets) => {
                    // Extract nested tuple first into a temp
                    let nested_temp = self.alloc_and_add_local(elem_type.clone(), mir_func);
                    let get_func = if is_tuple {
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_GET)
                    } else {
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET)
                    };

                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: nested_temp,
                        func: get_func,
                        args: vec![
                            source_operand.clone(),
                            mir::Operand::Constant(mir::Constant::Int(i as i64)),
                        ],
                    });

                    // Recursively unpack nested targets
                    self.lower_nested_recursive(
                        nested_targets,
                        mir::Operand::Local(nested_temp),
                        &elem_type,
                        mir_func,
                    )?;
                }
            }
        }

        Ok(())
    }
}
