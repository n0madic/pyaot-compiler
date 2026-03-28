//! Index expression lowering: obj[index]

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

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
        // Use get_expr_type for proper type inference
        let obj_type = self.get_expr_type(obj_expr, hir_module);

        let index_expr = &hir_module.exprs[index];
        let index_operand = self.lower_expr(index_expr, hir_module, mir_func)?;

        let result_local = self.alloc_local_id();

        match &obj_type {
            Type::Str => {
                // String indexing returns a single character string
                mir_func.add_local(mir::Local {
                    id: result_local,
                    name: None,
                    ty: Type::Str,
                    is_gc_root: true,
                });
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::StrGetChar,
                    args: vec![obj_operand, index_operand],
                });
            }
            Type::List(elem_ty) => {
                // List indexing returns element type
                // Use ListGet which returns raw value for ELEM_RAW_INT lists
                // or *mut Obj for ELEM_HEAP_OBJ lists
                // For Bool/Float elements, add unboxing step since they're stored as boxed objects
                match **elem_ty {
                    Type::Bool => {
                        // ListGet returns *mut Obj (boxed bool), need to unbox to i8
                        let boxed_local = self.alloc_local_id();
                        mir_func.add_local(mir::Local {
                            id: boxed_local,
                            name: None,
                            ty: Type::Str, // Placeholder for *mut Obj
                            is_gc_root: false,
                        });
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::ListGet,
                            args: vec![obj_operand, index_operand],
                        });
                        // Unbox to bool
                        mir_func.add_local(mir::Local {
                            id: result_local,
                            name: None,
                            ty: Type::Bool,
                            is_gc_root: false,
                        });
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: result_local,
                            func: mir::RuntimeFunc::UnboxBool,
                            args: vec![mir::Operand::Local(boxed_local)],
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
                            func: mir::RuntimeFunc::ListGet,
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
                            func: mir::RuntimeFunc::UnboxFloat,
                            args: vec![mir::Operand::Local(boxed_local)],
                        });
                    }
                    Type::Int => {
                        // ListGetInt transparently handles both ELEM_RAW_INT and
                        // ELEM_HEAP_OBJ storage (unboxes IntObj when needed).
                        mir_func.add_local(mir::Local {
                            id: result_local,
                            name: None,
                            ty: Type::Int,
                            is_gc_root: false,
                        });
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: result_local,
                            func: mir::RuntimeFunc::ListGetInt,
                            args: vec![obj_operand, index_operand],
                        });
                    }
                    _ => {
                        // For heap types (Str, List, etc.), ListGet returns *mut Obj.
                        // Any element type → HeapAny (always a valid pointer from ListGet).
                        let result_ty = if matches!(elem_ty.as_ref(), Type::Any) {
                            Type::HeapAny
                        } else {
                            (**elem_ty).clone()
                        };
                        mir_func.add_local(mir::Local {
                            id: result_local,
                            name: None,
                            ty: result_ty.clone(),
                            is_gc_root: result_ty.is_heap(),
                        });
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: result_local,
                            func: mir::RuntimeFunc::ListGet,
                            args: vec![obj_operand, index_operand],
                        });
                    }
                }
            }
            Type::Tuple(elem_types) => {
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
                    Type::normalize_union(elem_types.clone())
                };

                // Determine if this tuple uses ELEM_HEAP_OBJ storage.
                // Tuples only use ELEM_RAW_INT when ALL elements are Int.
                // Otherwise, primitives are boxed and need typed getters to unbox.
                let uses_heap_obj =
                    elem_types.is_empty() || !elem_types.iter().all(|t| *t == Type::Int);

                // When element type is Any and storage is ELEM_HEAP_OBJ,
                // the result is a heap pointer → use HeapAny for print/compare dispatch.
                let result_ty = if matches!(elem_ty, Type::Any) && uses_heap_obj {
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

                // Choose the appropriate getter based on element type and storage
                let runtime_func = if uses_heap_obj {
                    Self::tuple_get_func(&elem_ty)
                } else {
                    // ELEM_RAW_INT storage - all elements are raw i64
                    mir::RuntimeFunc::TupleGet
                };

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: runtime_func,
                    args: vec![obj_operand, index_operand],
                });
            }
            Type::Dict(_key_ty, value_ty) => {
                // Dict indexing: dict[key] returns value type
                // Dict values are always stored as boxed pointers for GC, so we need to unbox primitives
                mir_func.add_local(mir::Local {
                    id: result_local,
                    name: None,
                    ty: (**value_ty).clone(),
                    is_gc_root: value_ty.is_heap(),
                });
                // Box key if needed (int/bool keys need boxing)
                let index_type = self.get_expr_type(index_expr, hir_module);
                let boxed_key = self.box_primitive_if_needed(index_operand, &index_type, mir_func);

                // Check if value type needs unboxing
                let unbox_func = match value_ty.as_ref() {
                    Type::Int => Some(mir::RuntimeFunc::UnboxInt),
                    Type::Float => Some(mir::RuntimeFunc::UnboxFloat),
                    Type::Bool => Some(mir::RuntimeFunc::UnboxBool),
                    _ => None,
                };

                if let Some(unbox_func) = unbox_func {
                    // Get returns a boxed pointer, need to unbox
                    let boxed_local = self.alloc_and_add_local(Type::Str, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: boxed_local,
                        func: mir::RuntimeFunc::DictGet,
                        args: vec![obj_operand, boxed_key],
                    });
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: unbox_func,
                        args: vec![mir::Operand::Local(boxed_local)],
                    });
                } else {
                    // Heap types can be returned directly
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::DictGet,
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
                let index_type = self.get_expr_type(index_expr, hir_module);
                let boxed_key = self.box_primitive_if_needed(index_operand, &index_type, mir_func);

                let unbox_func = match value_ty.as_ref() {
                    Type::Int => Some(mir::RuntimeFunc::UnboxInt),
                    Type::Float => Some(mir::RuntimeFunc::UnboxFloat),
                    Type::Bool => Some(mir::RuntimeFunc::UnboxBool),
                    _ => None,
                };

                if let Some(unbox_func) = unbox_func {
                    let boxed_local = self.alloc_and_add_local(Type::Str, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: boxed_local,
                        func: mir::RuntimeFunc::DefaultDictGet,
                        args: vec![obj_operand, boxed_key],
                    });
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: unbox_func,
                        args: vec![mir::Operand::Local(boxed_local)],
                    });
                } else {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::DefaultDictGet,
                        args: vec![obj_operand, boxed_key],
                    });
                }
            }
            Type::Bytes => {
                // Bytes indexing returns an integer (0-255)
                mir_func.add_local(mir::Local {
                    id: result_local,
                    name: None,
                    ty: Type::Int,
                    is_gc_root: false,
                });
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::BytesGet,
                    args: vec![obj_operand, index_operand],
                });
            }
            Type::Class { class_id, .. } => {
                // Class with __getitem__ dunder
                let getitem_func = self
                    .get_class_info(class_id)
                    .and_then(|info| info.getitem_func);

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
                    func: mir::RuntimeFunc::AnyGetItem,
                    args: vec![obj_operand, index_operand],
                });
            }
        }

        Ok(mir::Operand::Local(result_local))
    }
}
