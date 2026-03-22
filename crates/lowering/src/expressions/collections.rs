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
        let mut list_type = self.get_expr_type(expr, hir_module);

        // For empty lists with unknown element type, use the expected type from
        // the assignment context (e.g., `x: list[int] = []` or `x = []` where x
        // was previously declared as list[int]).
        if elements.is_empty() {
            if let Type::List(ref elem_ty) = list_type {
                if **elem_ty == Type::Any {
                    if let Some(Type::List(ref expected_elem)) = self.expected_type {
                        list_type = Type::List(expected_elem.clone());
                    }
                }
            }
        }

        let result_local = self.alloc_and_add_local(list_type.clone(), mir_func);

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
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::MakeList,
            args: vec![
                mir::Operand::Constant(mir::Constant::Int(capacity)),
                mir::Operand::Constant(mir::Constant::Int(elem_tag)),
            ],
        });

        // Create a dummy local for void returns
        let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

        // Push each element
        for elem_id in elements {
            let elem_expr = &hir_module.exprs[*elem_id];
            let elem_operand = self.lower_expr(elem_expr, hir_module, mir_func)?;

            // Box elements before pushing to list:
            // - Float elements always need boxing
            // - Union element types need boxing for primitive values
            // - Bool elements: box for heap lists, extend to i64 for raw bool lists
            let push_operand = if elem_type == Type::Float {
                let boxed_local = self.alloc_and_add_local(Type::Str, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: boxed_local,
                    func: mir::RuntimeFunc::BoxFloat,
                    args: vec![elem_operand],
                });
                mir::Operand::Local(boxed_local)
            } else if elem_type == Type::Bool {
                // ELEM_HEAP_OBJ: box bools
                let boxed_local = self.alloc_and_add_local(Type::Str, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: boxed_local,
                    func: mir::RuntimeFunc::BoxBool,
                    args: vec![elem_operand],
                });
                mir::Operand::Local(boxed_local)
            } else if matches!(elem_type, Type::Union(_)) {
                // Box primitives for Union element types
                let actual_elem_type = self.get_expr_type(elem_expr, hir_module);
                self.box_primitive_if_needed(elem_operand, &actual_elem_type, mir_func)
            } else {
                elem_operand
            };

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::ListPush,
                args: vec![mir::Operand::Local(result_local), push_operand],
            });
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
        let tuple_type = self.get_expr_type(expr, hir_module);
        let result_local = self.alloc_and_add_local(tuple_type.clone(), mir_func);

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
        } else {
            0
        };

        // Create tuple with size and elem_tag
        let size = elements.len() as i64;
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::MakeTuple,
            args: vec![
                mir::Operand::Constant(mir::Constant::Int(size)),
                mir::Operand::Constant(mir::Constant::Int(elem_tag)),
            ],
        });

        // Create a dummy local for void returns
        let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

        // Set each element
        for (i, elem_id) in elements.iter().enumerate() {
            let elem_expr = &hir_module.exprs[*elem_id];
            let elem_operand = self.lower_expr(elem_expr, hir_module, mir_func)?;

            // Box primitive values when elem_tag is ELEM_HEAP_OBJ
            let final_operand = if elem_tag == 0 {
                // ELEM_HEAP_OBJ - need to box primitives
                let elem_type = self.get_expr_type(elem_expr, hir_module);
                match elem_type {
                    Type::Int => {
                        let boxed_local = self.alloc_and_add_local(Type::Str, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::BoxInt,
                            args: vec![elem_operand],
                        });
                        mir::Operand::Local(boxed_local)
                    }
                    Type::Bool => {
                        let boxed_local = self.alloc_and_add_local(Type::Str, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::BoxBool,
                            args: vec![elem_operand],
                        });
                        mir::Operand::Local(boxed_local)
                    }
                    Type::Float => {
                        let boxed_local = self.alloc_and_add_local(Type::Str, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::BoxFloat,
                            args: vec![elem_operand],
                        });
                        mir::Operand::Local(boxed_local)
                    }
                    _ => elem_operand, // Already a heap object
                }
            } else {
                elem_operand // ELEM_RAW_INT, already i64
            };

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::TupleSet,
                args: vec![
                    mir::Operand::Local(result_local),
                    mir::Operand::Constant(mir::Constant::Int(i as i64)),
                    final_operand,
                ],
            });
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
        let dict_type = self.get_expr_type(expr, hir_module);
        let result_local = self.alloc_and_add_local(dict_type.clone(), mir_func);

        // Create dict with capacity
        let capacity = pairs.len().max(8) as i64;
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::MakeDict,
            args: vec![mir::Operand::Constant(mir::Constant::Int(capacity))],
        });

        // Create a dummy local for void returns
        let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

        // Insert each key-value pair
        for (key_id, value_id) in pairs {
            let key_expr = &hir_module.exprs[*key_id];
            let key_type = self.get_expr_type(key_expr, hir_module);
            let key_operand = self.lower_expr(key_expr, hir_module, mir_func)?;

            // Box non-heap keys (int, bool) so dict can use them as object pointers
            let boxed_key = self.box_primitive_if_needed(key_operand, &key_type, mir_func);

            let value_expr = &hir_module.exprs[*value_id];
            let value_operand = self.lower_expr(value_expr, hir_module, mir_func)?;
            let actual_value_type = self.get_expr_type(value_expr, hir_module);

            // Box primitive values (all dict values must be heap pointers for GC)
            let boxed_value =
                self.box_primitive_if_needed(value_operand, &actual_value_type, mir_func);

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::DictSet,
                args: vec![mir::Operand::Local(result_local), boxed_key, boxed_value],
            });
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
        let set_type = self.get_expr_type(expr, hir_module);
        let result_local = self.alloc_and_add_local(set_type.clone(), mir_func);

        // Create set with capacity
        let capacity = elements.len().max(8) as i64;
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::MakeSet,
            args: vec![mir::Operand::Constant(mir::Constant::Int(capacity))],
        });

        // Create a dummy local for void returns
        let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

        // Add each element
        for elem_id in elements {
            let elem_expr = &hir_module.exprs[*elem_id];
            let elem_type = self.get_expr_type(elem_expr, hir_module);
            let elem_operand = self.lower_expr(elem_expr, hir_module, mir_func)?;

            // Box non-heap elements (int, bool) so set can use them as object pointers
            let boxed_elem = self.box_primitive_if_needed(elem_operand, &elem_type, mir_func);

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::SetAdd,
                args: vec![mir::Operand::Local(result_local), boxed_elem],
            });
        }

        Ok(mir::Operand::Local(result_local))
    }
}
