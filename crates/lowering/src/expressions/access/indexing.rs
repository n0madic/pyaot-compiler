//! Index expression lowering: obj[index]

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::{Type, TypeLattice};

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower an index expression: obj[index]
    pub(in crate::expressions) fn lower_index(
        &mut self,
        obj: hir::ExprId,
        index: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let obj_expr = &hir_module.exprs[obj];
        let obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
        // Use seed_expr_type for proper type inference
        let obj_type = self.seed_expr_type(obj, hir_module);

        let index_expr = &hir_module.exprs[index];
        let mut index_operand = self.lower_expr(index_expr, hir_module, mir_func)?;

        // If index is a class with __index__, call it to convert to int
        let index_type = self.seed_expr_type(index, hir_module);
        if let Type::Class { class_id, .. } = &index_type {
            if let Some(func_id) = self
                .get_class_info(class_id)
                .and_then(|info| info.get_dunder_func("__index__"))
            {
                let int_local = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest: int_local,
                    func: func_id,
                    args: vec![index_operand],
                });
                index_operand = mir::Operand::Local(int_local);
            }
        }

        let result_local = self.alloc_local_id();

        match &obj_type {
            Type::Str => {
                // String indexing: index is a Python codepoint index (may be negative).
                // Use StrSubscript which converts codepoint index → byte offset.
                mir_func.add_local(mir::Local {
                    id: result_local,
                    name: None,
                    ty: Type::Str,
                    is_gc_root: true,
                });
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_STR_SUBSCRIPT,
                    ),
                    args: vec![obj_operand, index_operand],
                });
            }
            _ if obj_type.is_list_like() => {
                let elem_ty = obj_type.list_elem().expect("list_like but no list_elem");
                // List indexing returns element type
                // Use ListGet which returns tagged Value bits (unwrapped to raw scalar by caller)
                // For Bool/Float elements, add unboxing step since they're stored as boxed objects
                match elem_ty {
                    Type::Bool => {
                        // rt_list_get_typed(list, idx, KIND_BOOL=2) returns raw 0/1 as i64.
                        // Codegen truncates i64→i8 automatically via ireduce.
                        mir_func.add_local(mir::Local {
                            id: result_local,
                            name: None,
                            ty: Type::Bool,
                            is_gc_root: false,
                        });
                        let kind_tag = mir::Operand::Constant(mir::Constant::Int(
                            mir::GetElementKind::Bool.to_tag() as i64,
                        ));
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: result_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_LIST_GET_TYPED,
                            ),
                            args: vec![obj_operand, index_operand, kind_tag],
                        });
                    }
                    Type::Float => {
                        // ListGet returns *mut Obj (boxed float), need to unbox to f64
                        let boxed_local = self.alloc_local_id();
                        mir_func.add_local(mir::Local {
                            id: boxed_local,
                            name: None,
                            ty: Type::Str, // Placeholder for *mut Obj
                            is_gc_root: false,
                        });
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_LIST_GET,
                            ),
                            args: vec![obj_operand, index_operand],
                        });
                        // Unbox to float
                        mir_func.add_local(mir::Local {
                            id: result_local,
                            name: None,
                            ty: Type::Float,
                            is_gc_root: false,
                        });
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: result_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_UNBOX_FLOAT,
                            ),
                            args: vec![mir::Operand::Local(boxed_local)],
                        });
                    }
                    Type::Int => {
                        // rt_list_get_typed(Int) decodes the tagged Value int.
                        mir_func.add_local(mir::Local {
                            id: result_local,
                            name: None,
                            ty: Type::Int,
                            is_gc_root: false,
                        });
                        let kind_tag = mir::Operand::Constant(mir::Constant::Int(
                            mir::GetElementKind::Int.to_tag() as i64,
                        ));
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: result_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_LIST_GET_TYPED,
                            ),
                            args: vec![obj_operand, index_operand, kind_tag],
                        });
                    }
                    _ => {
                        // For heap types (Str, List, etc.), ListGet returns *mut Obj.
                        // Any element type → HeapAny (always a valid pointer from ListGet).
                        let result_ty = if matches!(elem_ty, Type::Any) {
                            Type::HeapAny
                        } else {
                            elem_ty.clone()
                        };
                        mir_func.add_local(mir::Local {
                            id: result_local,
                            name: None,
                            ty: result_ty.clone(),
                            is_gc_root: result_ty.is_heap(),
                        });
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: result_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_LIST_GET,
                            ),
                            args: vec![obj_operand, index_operand],
                        });
                    }
                }
            }
            _ if obj_type.tuple_elems().is_some() => {
                let elem_types = obj_type.tuple_elems().expect("checked above");
                // Tuple indexing - try to extract precise element type from constant index
                let elem_ty = if elem_types.is_empty() {
                    Type::Any
                } else if let hir::ExprKind::Int(idx) = &index_expr.kind {
                    // Handle constant integer index - extract the precise element type
                    let len = elem_types.len() as i64;
                    let actual_idx = if *idx < 0 { len + idx } else { *idx };
                    if actual_idx >= 0 && (actual_idx as usize) < elem_types.len() {
                        elem_types[actual_idx as usize].clone()
                    } else {
                        // Out of bounds - will error at runtime, use first type as fallback
                        elem_types[0].clone()
                    }
                } else if elem_types.iter().all(|t| t == &elem_types[0]) {
                    // Homogeneous tuple - all elements have the same type
                    elem_types[0].clone()
                } else {
                    // Heterogeneous tuple with dynamic index - return union of all types
                    elem_types
                        .iter()
                        .cloned()
                        .reduce(|a, b| a.join(&b))
                        .unwrap_or(Type::Never)
                };

                // After §F.4, rt_tuple_get always returns raw tagged Value bits;
                // emit_tuple_get applies the appropriate unbox for primitive elem types.
                // When element type is Any, result is a heap pointer → HeapAny.
                let eff_elem_ty = if matches!(elem_ty, Type::Any) {
                    Type::HeapAny
                } else {
                    elem_ty.clone()
                };
                let value_local =
                    self.emit_tuple_get(obj_operand, index_operand, eff_elem_ty, mir_func);
                return Ok(mir::Operand::Local(value_local));
            }
            _ if obj_type.tuple_var_elem().is_some() => {
                let elem_ty = obj_type.tuple_var_elem().expect("checked above").clone();
                // Variable-length tuple — every index returns the element type.
                // Runtime bounds-check is done by rt_tuple_get.
                let eff_elem_ty = if matches!(elem_ty, Type::Any) {
                    Type::HeapAny
                } else {
                    elem_ty.clone()
                };
                let value_local =
                    self.emit_tuple_get(obj_operand, index_operand, eff_elem_ty, mir_func);
                return Ok(mir::Operand::Local(value_local));
            }
            _ if obj_type.dict_kv().is_some() && !matches!(obj_type, Type::DefaultDict(..)) => {
                let (_key_ty, value_ty) = obj_type.dict_kv().expect("checked above");
                // Dict indexing: dict[key] returns value type
                // Dict values are always stored as boxed pointers for GC, so we need to unbox primitives
                mir_func.add_local(mir::Local {
                    id: result_local,
                    name: None,
                    ty: value_ty.clone(),
                    is_gc_root: value_ty.is_heap(),
                });
                // Box key if needed (int/bool keys need boxing)
                let index_type = self.seed_expr_type(index, hir_module);
                let boxed_key = self.emit_value_slot(index_operand, &index_type, mir_func);

                // Check if value type needs unboxing — `unbox_if_needed`
                // emits ValueFromInt/Bool MIR or rt_unbox_float.
                let needs_unbox = matches!(value_ty, Type::Int | Type::Float | Type::Bool);

                if needs_unbox {
                    // Get returns a boxed pointer, need to unbox
                    let boxed_local = self.emit_runtime_call(
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_GET),
                        vec![obj_operand, boxed_key],
                        Type::HeapAny,
                        mir_func,
                    );
                    let unboxed =
                        self.unbox_if_needed(mir::Operand::Local(boxed_local), value_ty, mir_func);
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: result_local,
                        src: unboxed,
                    });
                } else {
                    // Heap types can be returned directly
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_DICT_GET,
                        ),
                        args: vec![obj_operand, boxed_key],
                    });
                }
            }
            Type::DefaultDict(_key_ty, value_ty) => {
                // DefaultDict indexing: dd[key] creates default on miss
                // Uses DefaultDictGet instead of DictGet
                mir_func.add_local(mir::Local {
                    id: result_local,
                    name: None,
                    ty: (**value_ty).clone(),
                    is_gc_root: value_ty.is_heap(),
                });
                let index_type = self.seed_expr_type(index, hir_module);
                let boxed_key = self.emit_value_slot(index_operand, &index_type, mir_func);

                let needs_unbox = matches!(**value_ty, Type::Int | Type::Float | Type::Bool);

                if needs_unbox {
                    let boxed_local = self.emit_runtime_call(
                        mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_DEFAULT_DICT_GET,
                        ),
                        vec![obj_operand, boxed_key],
                        Type::HeapAny,
                        mir_func,
                    );
                    let unboxed = self.unbox_if_needed(
                        mir::Operand::Local(boxed_local),
                        value_ty.as_ref(),
                        mir_func,
                    );
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: result_local,
                        src: unboxed,
                    });
                } else {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_DEFAULT_DICT_GET,
                        ),
                        args: vec![obj_operand, boxed_key],
                    });
                }
            }
            Type::Bytes => {
                // Bytes indexing returns an integer (0-255)
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_BYTES_GET),
                    vec![obj_operand, index_operand],
                    Type::Int,
                    mir_func,
                );
                return Ok(mir::Operand::Local(result_local));
            }
            Type::Class { class_id, .. } => {
                // Class with __getitem__ dunder
                let getitem_func = self
                    .get_class_info(class_id)
                    .and_then(|info| info.get_dunder_func("__getitem__"));

                if let Some(func_id) = getitem_func {
                    let return_ty = self
                        .get_func_return_type(&func_id)
                        .cloned()
                        .unwrap_or(Type::Any);
                    mir_func.add_local(mir::Local {
                        id: result_local,
                        name: None,
                        ty: return_ty.clone(),
                        is_gc_root: return_ty.is_heap(),
                    });
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: result_local,
                        func: func_id,
                        args: vec![obj_operand, index_operand],
                    });
                } else {
                    mir_func.add_local(mir::Local {
                        id: result_local,
                        name: None,
                        ty: Type::Any,
                        is_gc_root: false,
                    });
                    return Ok(mir::Operand::Constant(mir::Constant::None));
                }
            }
            _ => {
                // Runtime-dispatched subscript for Any/unknown types.
                // Calls rt_any_getitem which dispatches on the object's type tag.
                // Result is always a *mut Obj (HeapAny), never a raw primitive.
                mir_func.add_local(mir::Local {
                    id: result_local,
                    name: None,
                    ty: Type::HeapAny,
                    is_gc_root: true,
                });
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_ANY_GETITEM,
                    ),
                    args: vec![obj_operand, index_operand],
                });
            }
        }

        Ok(mir::Operand::Local(result_local))
    }
}
