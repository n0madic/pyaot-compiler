//! Unified binding-target lowering (HIR `BindingTarget` → MIR).
//!
//! Single recursive entry point [`Lowering::lower_binding_target`] handles
//! every shape that the new [`hir::BindingTarget`] enum can express:
//! plain variable, attribute, subscript, class attribute, arbitrarily nested
//! tuple/list patterns, and (one) starred slot per tuple level. Nested+starred
//! combinations work for free via mutual recursion between
//! [`Lowering::lower_binding_target`] and [`Lowering::lower_tuple_pattern`].
//!
//! This module also exposes operand-taking *leaf* helpers
//! (`bind_var_op`, `bind_attr_op`, `bind_index_op`, `bind_class_attr_op`)
//! that emit MIR for one binding given an already-lowered RHS operand.

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{ClassId, InternedString, LocalId, VarId};

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Recursive entry point: bind `value_operand` (of `value_type`) into
    /// `target`. Emits MIR into `mir_func` as a side effect.
    pub(crate) fn lower_binding_target(
        &mut self,
        target: &hir::BindingTarget,
        value_operand: mir::Operand,
        value_type: &Type,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        match target {
            hir::BindingTarget::Var(var_id) => {
                self.bind_var_op(*var_id, value_operand, value_type, mir_func)
            }
            hir::BindingTarget::Attr { obj, field, .. } => {
                let obj_expr = &hir_module.exprs[*obj];
                let obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
                let obj_type = self.get_type_of_expr_id(*obj, hir_module);
                self.bind_attr_op(
                    obj_operand,
                    &obj_type,
                    *field,
                    value_operand,
                    value_type,
                    mir_func,
                )
            }
            hir::BindingTarget::Index { obj, index, .. } => {
                let obj_expr = &hir_module.exprs[*obj];
                let obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
                let obj_type = self.get_type_of_expr_id(*obj, hir_module);
                let index_expr = &hir_module.exprs[*index];
                let index_operand = self.lower_expr(index_expr, hir_module, mir_func)?;
                let index_type = self.get_type_of_expr_id(*index, hir_module);

                // Refine Dict(Any, Any) type based on actual key/value types.
                // For patterns like `d = defaultdict(); d["k"] = 42`, infer
                // the dict element types from the first index assignment.
                let refined_obj_type = if let Type::Dict(ref key_ty, ref val_ty) = obj_type {
                    if **key_ty == Type::Any || **val_ty == Type::Any {
                        if let hir::ExprKind::Var(var_id) = &obj_expr.kind {
                            let refined_key = if **key_ty == Type::Any && index_type != Type::Any {
                                Box::new(index_type.clone())
                            } else {
                                key_ty.clone()
                            };
                            let refined_val = if **val_ty == Type::Any && *value_type != Type::Any {
                                Box::new(value_type.clone())
                            } else {
                                val_ty.clone()
                            };
                            let refined = Type::Dict(refined_key, refined_val);
                            self.insert_var_type(*var_id, refined.clone());
                            if let Some(local_id) = self.get_var_local(var_id) {
                                if let Some(local) = mir_func.locals.get_mut(&local_id) {
                                    local.ty = refined.clone();
                                    local.is_gc_root = local.ty.is_heap();
                                }
                            }
                            refined
                        } else {
                            obj_type.clone()
                        }
                    } else {
                        obj_type.clone()
                    }
                } else {
                    obj_type.clone()
                };

                self.bind_index_op(
                    obj_operand,
                    &refined_obj_type,
                    index_operand,
                    &index_type,
                    value_operand,
                    value_type,
                    mir_func,
                )
            }
            hir::BindingTarget::ClassAttr { class_id, attr, .. } => {
                self.bind_class_attr_op(*class_id, *attr, value_operand, mir_func)
            }
            hir::BindingTarget::Tuple { elts, .. } => {
                self.lower_tuple_pattern(elts, value_operand, value_type, hir_module, mir_func)
            }
            hir::BindingTarget::Starred { .. } => {
                // Starred is only meaningful as a child of Tuple; the parent
                // tuple-pattern lowering owns its handling. Reaching this arm
                // means the validator in `bind_target` allowed a `Starred` at
                // top level, which is a frontend bug.
                unreachable!("Starred binding target outside of Tuple — frontend bug");
            }
        }
    }

    /// Lower a `Tuple { elts }` pattern with at most one `Starred` slot.
    /// Mirrors the structure of legacy `lower_unpack_assign` (flat starred)
    /// and `lower_nested_recursive` (recursive non-starred), but recurses
    /// into [`Self::lower_binding_target`] so each leaf can itself be a
    /// `Var`, `Attr`, `Index`, `ClassAttr`, or another nested `Tuple`.
    fn lower_tuple_pattern(
        &mut self,
        elts: &[hir::BindingTarget],
        source: mir::Operand,
        source_type: &Type,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let star_idx = elts
            .iter()
            .position(|e| matches!(e, hir::BindingTarget::Starred { .. }));
        let is_tuple = matches!(source_type, Type::Tuple(_));

        // Mirror the heap-obj promotion from `lower_unpack_assign` so nested
        // `Any` elements route through `HeapAny` when the container backs
        // them with `ELEM_HEAP_OBJ` storage.
        let uses_heap_obj = match source_type {
            Type::Tuple(elem_types) => {
                elem_types.is_empty() || !elem_types.iter().all(|t| *t == Type::Int)
            }
            _ => true,
        };
        let promote_any = |t: Type| -> Type {
            if uses_heap_obj && matches!(t, Type::Any) {
                Type::HeapAny
            } else {
                t
            }
        };

        let elem_type_at = |index: usize, total: usize| -> Type {
            let raw = match source_type {
                Type::Tuple(types) => types.get(index).cloned().unwrap_or(Type::Any),
                Type::List(inner) => (**inner).clone(),
                _ => Type::Any,
            };
            // `total` reserved for future variable-tuple element computation;
            // currently unused but kept in the signature so the closure can
            // be reused across the no-star and starred branches without churn.
            let _ = total;
            promote_any(raw)
        };

        let elem_type_at_neg = |neg_index: i64| -> Type {
            let raw = match source_type {
                Type::Tuple(types) => {
                    let len = types.len() as i64;
                    let actual = (len + neg_index) as usize;
                    types.get(actual).cloned().unwrap_or(Type::Any)
                }
                Type::List(inner) => (**inner).clone(),
                _ => Type::Any,
            };
            promote_any(raw)
        };

        match star_idx {
            None => {
                // Pure positional: extract elt[i] for each leaf, then recurse.
                // Use intermediate temps so any side-effect in the leaf
                // (e.g. attribute set) sees a stable input across iterations.
                let mut temps: Vec<(usize, LocalId, Type)> = Vec::with_capacity(elts.len());
                for (i, _leaf) in elts.iter().enumerate() {
                    let elem_ty = elem_type_at(i, elts.len());
                    let get_func = if is_tuple {
                        crate::type_dispatch::tuple_get_func(&elem_ty)
                    } else {
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET)
                    };
                    let temp_local = self.emit_runtime_call(
                        get_func,
                        vec![
                            source.clone(),
                            mir::Operand::Constant(mir::Constant::Int(i as i64)),
                        ],
                        elem_ty.clone(),
                        mir_func,
                    );
                    temps.push((i, temp_local, elem_ty));
                }
                for (i, leaf) in elts.iter().enumerate() {
                    let (_, temp_local, elem_ty) = &temps[i];
                    self.lower_binding_target(
                        leaf,
                        mir::Operand::Local(*temp_local),
                        elem_ty,
                        hir_module,
                        mir_func,
                    )?;
                }
            }
            Some(k) => {
                let before = &elts[..k];
                let after = &elts[k + 1..];
                let starred_inner = match &elts[k] {
                    hir::BindingTarget::Starred { inner, .. } => inner.as_ref(),
                    _ => unreachable!("position() found a Starred"),
                };

                // Stage every extraction into temps before binding any leaf
                // (parallel-assignment semantics for arbitrary leaf side effects).
                let mut before_temps: Vec<(LocalId, Type)> = Vec::with_capacity(before.len());
                for (i, _) in before.iter().enumerate() {
                    let elem_ty = elem_type_at(i, elts.len());
                    let get_func = if is_tuple {
                        crate::type_dispatch::tuple_get_func(&elem_ty)
                    } else {
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET)
                    };
                    let temp_local = self.emit_runtime_call(
                        get_func,
                        vec![
                            source.clone(),
                            mir::Operand::Constant(mir::Constant::Int(i as i64)),
                        ],
                        elem_ty.clone(),
                        mir_func,
                    );
                    before_temps.push((temp_local, elem_ty));
                }

                // Starred slice: always materialises a list.
                let slice_func = if is_tuple {
                    mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_TUPLE_SLICE_TO_LIST,
                    )
                } else {
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_SLICE)
                };
                let starred_elem_type = match source_type {
                    Type::List(elem_ty) => (**elem_ty).clone(),
                    Type::Tuple(elem_types) => {
                        let middle_start = before.len();
                        let middle_end = elem_types.len().saturating_sub(after.len());
                        if middle_start < middle_end {
                            let middle_types: Vec<_> =
                                elem_types[middle_start..middle_end].to_vec();
                            Type::normalize_union(middle_types)
                        } else {
                            Type::Any
                        }
                    }
                    _ => Type::Any,
                };
                let starred_type = Type::List(Box::new(starred_elem_type));
                let start_idx = before.len() as i64;
                let end_idx = if after.is_empty() {
                    i64::MAX
                } else {
                    -(after.len() as i64)
                };
                let starred_temp = self.emit_runtime_call(
                    slice_func,
                    vec![
                        source.clone(),
                        mir::Operand::Constant(mir::Constant::Int(start_idx)),
                        mir::Operand::Constant(mir::Constant::Int(end_idx)),
                    ],
                    starred_type.clone(),
                    mir_func,
                );

                // Trailing leaves use negative indices (counting from the end).
                let mut after_temps: Vec<(LocalId, Type)> = Vec::with_capacity(after.len());
                for (i, _) in after.iter().enumerate() {
                    let neg_index = -((after.len() - i) as i64);
                    let elem_ty = elem_type_at_neg(neg_index);
                    let get_func = if is_tuple {
                        crate::type_dispatch::tuple_get_func(&elem_ty)
                    } else {
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET)
                    };
                    let temp_local = self.emit_runtime_call(
                        get_func,
                        vec![
                            source.clone(),
                            mir::Operand::Constant(mir::Constant::Int(neg_index)),
                        ],
                        elem_ty.clone(),
                        mir_func,
                    );
                    after_temps.push((temp_local, elem_ty));
                }

                // Now bind in pattern order.
                for (leaf, (temp_local, elem_ty)) in before.iter().zip(before_temps.into_iter()) {
                    self.lower_binding_target(
                        leaf,
                        mir::Operand::Local(temp_local),
                        &elem_ty,
                        hir_module,
                        mir_func,
                    )?;
                }
                self.lower_binding_target(
                    starred_inner,
                    mir::Operand::Local(starred_temp),
                    &starred_type,
                    hir_module,
                    mir_func,
                )?;
                for (leaf, (temp_local, elem_ty)) in after.iter().zip(after_temps.into_iter()) {
                    self.lower_binding_target(
                        leaf,
                        mir::Operand::Local(temp_local),
                        &elem_ty,
                        hir_module,
                        mir_func,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Bind an already-lowered RHS operand to a simple variable. Used both
    /// by [`Self::lower_binding_target`] and (eventually) by the legacy
    /// `lower_assign` after it has handled FuncRef/Closure/dict-update
    /// special cases that don't apply inside an unpack pattern.
    pub(crate) fn bind_var_op(
        &mut self,
        var_id: VarId,
        value_operand: mir::Operand,
        value_type: &Type,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        // Prefer the pre-scanned unified type (Area E §E.6) so the MIR
        // local is typed consistently across all writes. Locals widened
        // through the numeric tower store via the same coercion path
        // used for class fields (§E.3 Part B).
        let local_ty = self
            .hir_types
            .prescan_var_types
            .get(&var_id)
            .cloned()
            .unwrap_or_else(|| value_type.clone());
        self.insert_var_type(var_id, local_ty.clone());
        let dest_local = self.get_or_create_local(var_id, local_ty.clone(), mir_func);
        let coerced = self.coerce_to_field_type(value_operand, value_type, &local_ty, mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: dest_local,
            src: coerced,
        });
        Ok(())
    }

    /// Bind an already-lowered RHS operand into `obj.field` (instance field
    /// or `@property` setter).
    ///
    /// When the declared field type is wider than the value type through
    /// the numeric tower (Area E §E.3), emits an `IntToFloat` conversion
    /// before the runtime call — `self.total = 0` into a `float`-widened
    /// field stores `0.0`, not the bit-pattern of `0_i64`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn bind_attr_op(
        &mut self,
        obj_operand: mir::Operand,
        obj_type: &Type,
        field: InternedString,
        value_operand: mir::Operand,
        value_type: &Type,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        if let Type::Class { class_id, .. } = obj_type {
            if let Some(class_info) = self.get_class_info(class_id).cloned() {
                // 1. @property setter
                if let Some((_getter, Some(setter_id))) = class_info.properties.get(&field) {
                    let setter_id = *setter_id;
                    let dummy_local = self.alloc_and_add_local(Type::None, mir_func);
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: dummy_local,
                        func: setter_id,
                        args: vec![obj_operand, value_operand],
                    });
                    return Ok(());
                }
                // 2. Regular instance field
                if let Some(&offset) = class_info.field_offsets.get(&field) {
                    let field_ty = class_info
                        .field_types
                        .get(&field)
                        .cloned()
                        .unwrap_or(Type::Any);
                    let coerced =
                        self.coerce_to_field_type(value_operand, value_type, &field_ty, mir_func);
                    self.emit_runtime_call(
                        mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_INSTANCE_SET_FIELD,
                        ),
                        vec![
                            obj_operand,
                            mir::Operand::Constant(mir::Constant::Int(offset as i64)),
                            coerced,
                        ],
                        Type::None,
                        mir_func,
                    );
                    return Ok(());
                }
                // 3. Fallback to class attribute (instance.class_attr = ...)
                if let (Some(&(owning_class_id, attr_offset)), Some(attr_type)) = (
                    class_info.class_attr_offsets.get(&field),
                    class_info.class_attr_types.get(&field).cloned(),
                ) {
                    let set_func = self.get_class_attr_set_func(&attr_type);
                    let effective_class_id = self.get_effective_class_id(owning_class_id);
                    self.emit_runtime_call(
                        set_func,
                        vec![
                            mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                            mir::Operand::Constant(mir::Constant::Int(attr_offset as i64)),
                            value_operand,
                        ],
                        Type::None,
                        mir_func,
                    );
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Coerce `value_operand` to match the declared `field_ty` before it is
    /// stored via `rt_instance_set_field`. The runtime stores every field
    /// as a raw `i64`; read-side codegen then interprets those bits
    /// according to the field's declared type. Without coercion, writing
    /// an `Int` literal into a field that was widened to `Float` (Area E
    /// §E.3) stores the `i64` bit-pattern, which the read would later
    /// bitcast into a subnormal `f64`.
    ///
    /// Rules:
    /// - `(Int | Bool, Float)` → emit `IntToFloat` (reuses
    ///   `promote_to_float_if_needed`).
    /// - `(primitive, Union | Any | HeapAny)` → box via
    ///   `box_primitive_if_needed` so the field can hold a heap pointer.
    /// - Everything else — identity. `Bool → Int` is a safe bit-extension
    ///   handled by the existing `GlobalSet(Ptr)` pattern (Cranelift zero-
    ///   extends `i8` to `i64`).
    pub(crate) fn coerce_to_field_type(
        &mut self,
        value_operand: mir::Operand,
        value_ty: &Type,
        field_ty: &Type,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        // Identity — no coercion required.
        if value_ty == field_ty {
            return value_operand;
        }
        match (value_ty, field_ty) {
            // Numeric tower widening.
            (Type::Int | Type::Bool, Type::Float) => {
                self.promote_to_float_if_needed(mir_func, value_operand, value_ty)
            }
            // Primitive into union / dynamic field — box so the raw bits
            // become a heap pointer.
            (Type::Int | Type::Bool | Type::Float | Type::None, Type::Union(_))
            | (Type::Int | Type::Bool | Type::Float | Type::None, Type::Any)
            | (Type::Int | Type::Bool | Type::Float | Type::None, Type::HeapAny) => {
                self.box_primitive_if_needed(value_operand, value_ty, mir_func)
            }
            // Everything else — identity.
            _ => value_operand,
        }
    }

    /// Bind an already-lowered RHS operand into `obj[index]`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn bind_index_op(
        &mut self,
        obj_operand: mir::Operand,
        obj_type: &Type,
        index_operand: mir::Operand,
        index_type: &Type,
        value_operand: mir::Operand,
        value_type: &Type,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        match obj_type {
            Type::Dict(_, _) => {
                let boxed_key = self.box_primitive_if_needed(index_operand, index_type, mir_func);
                let boxed_value = self.box_primitive_if_needed(value_operand, value_type, mir_func);
                self.emit_runtime_call_void(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_SET),
                    vec![obj_operand, boxed_key, boxed_value],
                    mir_func,
                );
            }
            Type::DefaultDict(_, val_ty) => {
                let boxed_key = self.box_primitive_if_needed(index_operand, index_type, mir_func);
                let box_type = if *value_type == Type::Any {
                    val_ty.as_ref()
                } else {
                    value_type
                };
                let boxed_value = self.box_primitive_if_needed(value_operand, box_type, mir_func);
                self.emit_runtime_call_void(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_SET),
                    vec![obj_operand, boxed_key, boxed_value],
                    mir_func,
                );
            }
            Type::List(elem_ty) => {
                let store_operand = if **elem_ty == Type::Float {
                    let boxed_local = self.emit_runtime_call(
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_BOX_FLOAT),
                        vec![value_operand],
                        Type::HeapAny,
                        mir_func,
                    );
                    mir::Operand::Local(boxed_local)
                } else if **elem_ty == Type::Bool {
                    let boxed_local = self.emit_runtime_call(
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_BOX_BOOL),
                        vec![value_operand],
                        Type::HeapAny,
                        mir_func,
                    );
                    mir::Operand::Local(boxed_local)
                } else {
                    value_operand
                };
                self.emit_runtime_call_void(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_SET),
                    vec![obj_operand, index_operand, store_operand],
                    mir_func,
                );
            }
            Type::Class { class_id, .. } => {
                let setitem_func = self
                    .get_class_info(class_id)
                    .and_then(|info| info.get_dunder_func("__setitem__"));
                if let Some(func_id) = setitem_func {
                    let dummy_local = self.alloc_and_add_local(Type::None, mir_func);
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: dummy_local,
                        func: func_id,
                        args: vec![obj_operand, index_operand, value_operand],
                    });
                }
            }
            _ => {
                // Unsupported container: silently no-op, matching legacy behaviour.
            }
        }
        Ok(())
    }

    /// Bind an already-lowered RHS operand into `ClassName.attr`.
    pub(crate) fn bind_class_attr_op(
        &mut self,
        class_id: ClassId,
        attr: InternedString,
        value_operand: mir::Operand,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        if let Some(class_info) = self.get_class_info(&class_id) {
            if let (Some(&(owning_class_id, attr_offset)), Some(attr_type)) = (
                class_info.class_attr_offsets.get(&attr),
                class_info.class_attr_types.get(&attr).cloned(),
            ) {
                let set_func = self.get_class_attr_set_func(&attr_type);
                let effective_class_id = self.get_effective_class_id(owning_class_id);
                self.emit_runtime_call(
                    set_func,
                    vec![
                        mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                        mir::Operand::Constant(mir::Constant::Int(attr_offset as i64)),
                        value_operand,
                    ],
                    Type::None,
                    mir_func,
                );
            }
        }
        Ok(())
    }
}
