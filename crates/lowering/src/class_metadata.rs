//! Class metadata management and vtable construction
//!
//! This module handles class hierarchy processing, building class information
//! (fields, methods, vtables), and emitting class initialization code.

use indexmap::IndexMap;
use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{ClassId, FuncId};

use crate::{LoweredClassInfo, Lowering};

impl<'a> Lowering<'a> {
    // ==================== Class Hierarchy Processing ====================

    /// Topological sort of classes to ensure parents are processed before children
    pub(crate) fn topological_sort_classes(&self, hir_module: &hir::Module) -> Vec<ClassId> {
        let mut sorted = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut visiting = std::collections::HashSet::new();

        fn visit(
            class_id: ClassId,
            hir_module: &hir::Module,
            visited: &mut std::collections::HashSet<ClassId>,
            visiting: &mut std::collections::HashSet<ClassId>,
            sorted: &mut Vec<ClassId>,
        ) {
            if visited.contains(&class_id) {
                return;
            }
            if visiting.contains(&class_id) {
                // Circular inheritance detected (should be caught earlier, but handle gracefully)
                return;
            }

            visiting.insert(class_id);

            // Process parent first
            if let Some(class_def) = hir_module.class_defs.get(&class_id) {
                if let Some(base_id) = class_def.base_class {
                    visit(base_id, hir_module, visited, visiting, sorted);
                }
            }

            visiting.remove(&class_id);
            visited.insert(class_id);
            sorted.push(class_id);
        }

        for class_id in hir_module.class_defs.keys() {
            visit(
                *class_id,
                hir_module,
                &mut visited,
                &mut visiting,
                &mut sorted,
            );
        }

        sorted
    }

    /// Build class information from HIR class definitions.
    /// This processes all classes in topological order to ensure parents are processed first.
    pub(crate) fn build_class_info(&mut self, hir_module: &hir::Module) {
        // Topological sort ensures parents are processed before children
        let sorted_classes = self.topological_sort_classes(hir_module);

        for class_id in sorted_classes {
            let class_def = &hir_module.class_defs[&class_id];

            let class_name = self.resolve(class_def.name).to_string();
            self.register_class_name(class_name, class_id);

            // Start with inherited fields/methods/vtable from parent
            let (
                mut field_offsets,
                mut field_types,
                mut method_funcs,
                mut vtable_slots,
                mut static_methods,
                mut class_methods,
                mut properties,
                mut property_types,
                mut str_func,
                mut repr_func,
                mut eq_func,
                mut ne_func,
                mut lt_func,
                mut le_func,
                mut gt_func,
                mut ge_func,
                mut hash_func,
                mut len_func,
                mut add_func,
                mut sub_func,
                mut mul_func,
                mut truediv_func,
                mut floordiv_func,
                mut mod_func,
                mut pow_func,
                mut radd_func,
                mut rsub_func,
                mut rmul_func,
                mut rtruediv_func,
                mut rfloordiv_func,
                mut rmod_func,
                mut rpow_func,
                mut neg_func,
                mut pos_func,
                mut abs_func,
                mut invert_func,
                mut bool_func,
                mut int_func,
                mut float_func,
                mut getitem_func,
                mut setitem_func,
                mut delitem_func,
                mut contains_func,
                mut iter_func,
                mut next_func,
                mut call_func,
                own_field_offset,
            ) = if let Some(base_id) = class_def.base_class {
                let parent_info = self
                    .get_class_info(&base_id)
                    .expect("Parent class must be processed first");
                (
                    parent_info.field_offsets.clone(),
                    parent_info.field_types.clone(),
                    parent_info.method_funcs.clone(),
                    parent_info.vtable_slots.clone(),
                    parent_info.static_methods.clone(),
                    parent_info.class_methods.clone(),
                    parent_info.properties.clone(),
                    parent_info.property_types.clone(),
                    parent_info.str_func,
                    parent_info.repr_func,
                    parent_info.eq_func,
                    parent_info.ne_func,
                    parent_info.lt_func,
                    parent_info.le_func,
                    parent_info.gt_func,
                    parent_info.ge_func,
                    parent_info.hash_func,
                    parent_info.len_func,
                    parent_info.add_func,
                    parent_info.sub_func,
                    parent_info.mul_func,
                    parent_info.truediv_func,
                    parent_info.floordiv_func,
                    parent_info.mod_func,
                    parent_info.pow_func,
                    parent_info.radd_func,
                    parent_info.rsub_func,
                    parent_info.rmul_func,
                    parent_info.rtruediv_func,
                    parent_info.rfloordiv_func,
                    parent_info.rmod_func,
                    parent_info.rpow_func,
                    parent_info.neg_func,
                    parent_info.pos_func,
                    parent_info.abs_func,
                    parent_info.invert_func,
                    parent_info.bool_func,
                    parent_info.int_func,
                    parent_info.float_func,
                    parent_info.getitem_func,
                    parent_info.setitem_func,
                    parent_info.delitem_func,
                    parent_info.contains_func,
                    parent_info.iter_func,
                    parent_info.next_func,
                    parent_info.call_func,
                    parent_info.total_field_count,
                )
            } else {
                (
                    IndexMap::new(),
                    IndexMap::new(),
                    IndexMap::new(),
                    IndexMap::new(),
                    IndexMap::new(),
                    IndexMap::new(),
                    IndexMap::new(),
                    IndexMap::new(),
                    None, // str_func
                    None, // repr_func
                    None, // eq_func
                    None, // ne_func
                    None, // lt_func
                    None, // le_func
                    None, // gt_func
                    None, // ge_func
                    None, // hash_func
                    None, // len_func
                    None, // add_func
                    None, // sub_func
                    None, // mul_func
                    None, // truediv_func
                    None, // floordiv_func
                    None, // mod_func
                    None, // pow_func
                    None, // radd_func
                    None, // rsub_func
                    None, // rmul_func
                    None, // rtruediv_func
                    None, // rfloordiv_func
                    None, // rmod_func
                    None, // rpow_func
                    None, // neg_func
                    None, // pos_func
                    None, // abs_func
                    None, // invert_func
                    None, // bool_func
                    None, // int_func
                    None, // float_func
                    None, // getitem_func
                    None, // setitem_func
                    None, // delitem_func
                    None, // contains_func
                    None, // iter_func
                    None, // next_func
                    None, // call_func
                    0,
                )
            };

            // Add this class's own fields (starting after inherited fields).
            // Skip fields already inherited from parent to maintain consistent offsets
            // across the inheritance hierarchy (required for class pattern matching).
            let mut own_field_idx = 0;
            for field in class_def.fields.iter() {
                if field_offsets.contains_key(&field.name) {
                    // Inherited field — keep parent's offset, update type if refined
                    field_types.insert(field.name, field.ty.clone());
                } else {
                    let offset = own_field_offset + own_field_idx;
                    field_offsets.insert(field.name, offset);
                    field_types.insert(field.name, field.ty.clone());
                    own_field_idx += 1;
                }
            }

            // Add/override methods and update vtable slots based on method_kind
            for method_id in &class_def.methods {
                if let Some(func) = hir_module.func_defs.get(method_id) {
                    // Method name mangling convention: ClassName$method_name.
                    // The `$` separator is guaranteed unique as Python identifiers cannot contain `$`.
                    let func_name_str = self.resolve(func.name);
                    let method_name_str = if let Some(idx) = func_name_str.find('$') {
                        // Extract method name after the '$'
                        &func_name_str[idx + 1..]
                    } else {
                        // Not mangled, use as-is (shouldn't happen for methods)
                        func_name_str
                    };

                    // Detect dunder methods and track them separately
                    match method_name_str {
                        "__str__" => {
                            str_func = Some(*method_id);
                        }
                        "__repr__" => {
                            repr_func = Some(*method_id);
                        }
                        "__eq__" => {
                            eq_func = Some(*method_id);
                        }
                        "__ne__" => {
                            ne_func = Some(*method_id);
                        }
                        "__lt__" => {
                            lt_func = Some(*method_id);
                        }
                        "__le__" => {
                            le_func = Some(*method_id);
                        }
                        "__gt__" => {
                            gt_func = Some(*method_id);
                        }
                        "__ge__" => {
                            ge_func = Some(*method_id);
                        }
                        "__hash__" => {
                            hash_func = Some(*method_id);
                        }
                        "__len__" => {
                            len_func = Some(*method_id);
                        }
                        "__add__" => {
                            add_func = Some(*method_id);
                        }
                        "__sub__" => {
                            sub_func = Some(*method_id);
                        }
                        "__mul__" => {
                            mul_func = Some(*method_id);
                        }
                        "__truediv__" => {
                            truediv_func = Some(*method_id);
                        }
                        "__floordiv__" => {
                            floordiv_func = Some(*method_id);
                        }
                        "__mod__" => {
                            mod_func = Some(*method_id);
                        }
                        "__pow__" => {
                            pow_func = Some(*method_id);
                        }
                        "__radd__" => {
                            radd_func = Some(*method_id);
                        }
                        "__rsub__" => {
                            rsub_func = Some(*method_id);
                        }
                        "__rmul__" => {
                            rmul_func = Some(*method_id);
                        }
                        "__rtruediv__" => {
                            rtruediv_func = Some(*method_id);
                        }
                        "__rfloordiv__" => {
                            rfloordiv_func = Some(*method_id);
                        }
                        "__rmod__" => {
                            rmod_func = Some(*method_id);
                        }
                        "__rpow__" => {
                            rpow_func = Some(*method_id);
                        }
                        "__neg__" => {
                            neg_func = Some(*method_id);
                        }
                        "__pos__" => {
                            pos_func = Some(*method_id);
                        }
                        "__abs__" => {
                            abs_func = Some(*method_id);
                        }
                        "__invert__" => {
                            invert_func = Some(*method_id);
                        }
                        "__bool__" => {
                            bool_func = Some(*method_id);
                        }
                        "__int__" => {
                            int_func = Some(*method_id);
                        }
                        "__float__" => {
                            float_func = Some(*method_id);
                        }
                        "__getitem__" => {
                            getitem_func = Some(*method_id);
                        }
                        "__setitem__" => {
                            setitem_func = Some(*method_id);
                        }
                        "__delitem__" => {
                            delitem_func = Some(*method_id);
                        }
                        "__contains__" => {
                            contains_func = Some(*method_id);
                        }
                        "__iter__" => {
                            iter_func = Some(*method_id);
                        }
                        "__next__" => {
                            next_func = Some(*method_id);
                        }
                        "__call__" => {
                            call_func = Some(*method_id);
                        }
                        _ => {
                            // For non-dunder methods, we need to intern the name and add to maps
                            // Look up without mutation - if not found, method was never called
                            // as a method call (e.g., __init__ called via instantiation)
                            if let Some(method_name) = self.lookup_interned(method_name_str) {
                                // Route method to appropriate map based on method_kind
                                match func.method_kind {
                                    hir::MethodKind::Static => {
                                        // Static methods: no self/cls, skip vtable
                                        static_methods.insert(method_name, *method_id);
                                    }
                                    hir::MethodKind::ClassMethod => {
                                        // Class methods: receives cls, skip vtable
                                        class_methods.insert(method_name, *method_id);
                                    }
                                    hir::MethodKind::Instance => {
                                        // Instance methods: regular virtual dispatch
                                        method_funcs.insert(method_name, *method_id);

                                        // Update vtable: reuse existing slot if overriding, else allocate new slot
                                        if !vtable_slots.contains_key(&method_name) {
                                            let slot = vtable_slots.len();
                                            vtable_slots.insert(method_name, slot);
                                        }
                                        // If method already in vtable (inherited), we reuse the same slot
                                        // but the method_funcs map now points to the overriding method
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Build property info from HIR PropertyDef
            for prop in &class_def.properties {
                properties.insert(prop.name, (prop.getter, prop.setter));
                property_types.insert(prop.name, prop.ty.clone());
            }

            let total_field_count = own_field_offset + own_field_idx;

            // Build class attribute info (inherited from parent + own)
            // For inherited attributes, we keep the parent's (class_id, offset) to ensure
            // that accessing an inherited attribute uses the parent's storage
            let (mut class_attr_offsets, mut class_attr_types, own_attr_offset) =
                if let Some(base_id) = class_def.base_class {
                    let parent_info = self
                        .get_class_info(&base_id)
                        .expect("Parent class must be processed first");
                    // Clone inherited attributes - they keep their original (class_id, offset)
                    (
                        parent_info.class_attr_offsets.clone(),
                        parent_info.class_attr_types.clone(),
                        parent_info.class_attr_offsets.len(),
                    )
                } else {
                    (IndexMap::new(), IndexMap::new(), 0)
                };

            // Add this class's own class attributes with the current class_id as owner
            for (i, class_attr) in class_def.class_attrs.iter().enumerate() {
                let offset = own_attr_offset + i;
                // Store (owning_class_id, offset) so we know where the attribute is defined
                class_attr_offsets.insert(class_attr.name, (class_id, offset));
                class_attr_types.insert(class_attr.name, class_attr.ty.clone());
            }

            let info = LoweredClassInfo {
                field_offsets,
                field_types,
                method_funcs,
                init_func: class_def.init_method,
                str_func,
                repr_func,
                eq_func,
                ne_func,
                lt_func,
                le_func,
                gt_func,
                ge_func,
                hash_func,
                len_func,
                add_func,
                sub_func,
                mul_func,
                truediv_func,
                floordiv_func,
                mod_func,
                pow_func,
                radd_func,
                rsub_func,
                rmul_func,
                rtruediv_func,
                rfloordiv_func,
                rmod_func,
                rpow_func,
                neg_func,
                pos_func,
                abs_func,
                invert_func,
                bool_func,
                int_func,
                float_func,
                getitem_func,
                setitem_func,
                delitem_func,
                contains_func,
                iter_func,
                next_func,
                call_func,
                base_class: class_def.base_class,
                total_field_count,
                own_field_offset,
                vtable_slots,
                class_attr_offsets,
                class_attr_types,
                static_methods,
                class_methods,
                properties,
                property_types,
                is_exception_class: class_def.is_exception_class,
            };
            self.insert_class_info(class_id, info);
        }
    }

    /// Build vtables from class information and export to MIR module.
    /// This should be called after all functions have been lowered.
    pub(crate) fn build_vtables(&mut self) {
        // Collect vtable info for all classes
        let vtables: Vec<mir::VtableInfo> =
            self.class_info_iter()
                .map(|(class_id, class_info)| {
                    let mut entries: Vec<mir::VtableEntry> = class_info
                        .vtable_slots
                        .iter()
                        .filter_map(|(name, &slot)| {
                            class_info.method_funcs.get(name).map(|&method_func_id| {
                                mir::VtableEntry {
                                    slot,
                                    method_func_id,
                                }
                            })
                        })
                        .collect();
                    // Sort by slot index to ensure consistent vtable layout
                    entries.sort_by_key(|e| e.slot);
                    mir::VtableInfo {
                        class_id: *class_id,
                        entries,
                    }
                })
                .collect();

        // Add all vtables to the MIR module
        for vtable in vtables {
            self.add_vtable(vtable);
        }
    }

    // ==================== Class Initialization ====================

    /// Emit class registration calls for inheritance support.
    /// This registers each class with its parent in the runtime vtable registry.
    ///
    /// For exception classes:
    /// - If inheriting from a built-in exception (base_exception_type is Some), use that tag as parent
    /// - If inheriting from another user-defined exception class, use the parent's effective class ID
    pub(crate) fn emit_class_registrations(
        &mut self,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) {
        // Allocate a dummy local for the void return of registration calls
        // Use Type::Int since we store a 0 constant (i64)
        let dummy_local = self.alloc_and_add_local(Type::Int, mir_func);

        // Register all classes with their parent (or 255 if no parent - sentinel value)
        // Use offset-adjusted ClassIds to avoid collisions across modules
        for (class_id, class_def) in &hir_module.class_defs {
            if class_def.is_protocol {
                continue;
            }
            let effective_class_id = self.get_effective_class_id(*class_id);

            // Determine parent class ID:
            // 1. If this is an exception class inheriting from a built-in exception,
            //    and there's no HIR base_class (direct inheritance from built-in), use base_exception_type
            // 2. If there's a base_class (user-defined parent), use its effective class ID
            // 3. Otherwise, use 255 (NO_PARENT sentinel)
            let parent_class_id = if let Some(base_id) = class_def.base_class {
                // User-defined base class
                self.get_effective_class_id(base_id)
            } else if class_def.is_exception_class {
                // Exception class inheriting from built-in exception
                // Use the built-in exception type tag (0-12) as parent
                class_def.base_exception_type.unwrap_or(0) as i64
            } else {
                // No parent
                255
            };

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::RegisterClass,
                args: vec![
                    mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                    mir::Operand::Constant(mir::Constant::Int(parent_class_id)),
                ],
            });

            // Compute heap field mask: bit i is set if field i is a heap type (pointer)
            // that needs GC tracing. Raw values (Int, Float, Bool, None) are NOT heap.
            // Use class_info.field_types (which includes inherited + own fields in correct
            // absolute order) to ensure inherited heap fields are also tracked by the GC.
            let mut heap_field_mask: i64 = 0;
            if let Some(class_info) = self.get_class_info(class_id) {
                for (i, (_name, ty)) in class_info.field_types.iter().enumerate() {
                    if i >= 64 {
                        break; // Only support up to 64 fields
                    }
                    let is_heap = !matches!(
                        ty,
                        pyaot_types::Type::Int
                            | pyaot_types::Type::Float
                            | pyaot_types::Type::Bool
                            | pyaot_types::Type::None
                    );
                    if is_heap {
                        heap_field_mask |= 1i64 << i;
                    }
                }
            }
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::RegisterClassFields,
                args: vec![
                    mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                    mir::Operand::Constant(mir::Constant::Int(heap_field_mask)),
                ],
            });

            // Register method name→slot mappings for Protocol dispatch.
            // Collect data first to avoid borrow conflict with emit_instruction.
            let method_slots: Vec<(i64, i64)> = self
                .get_class_info(class_id)
                .map(|ci| {
                    ci.vtable_slots
                        .iter()
                        .map(|(name, &slot)| {
                            let name_str = self.resolve(*name);
                            let hash = pyaot_utils::fnv1a_hash(name_str) as i64;
                            (hash, slot as i64)
                        })
                        .collect()
                })
                .unwrap_or_default();
            for (name_hash, slot) in method_slots {
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::RegisterMethodName,
                    args: vec![
                        mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                        mir::Operand::Constant(mir::Constant::Int(name_hash)),
                        mir::Operand::Constant(mir::Constant::Int(slot)),
                    ],
                });
            }

            // For exception classes, also register the class name for error messages
            if class_def.is_exception_class {
                // class_def.name is already an InternedString, use it directly
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::ExcRegisterClassName,
                    args: vec![
                        mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                        mir::Operand::Constant(mir::Constant::Str(class_def.name)),
                    ],
                });
            }
        }
    }

    /// Emit class attribute initialization calls for all class attributes.
    /// This initializes class attributes with their initial values at module load time.
    pub(crate) fn emit_class_attr_initializations(
        &mut self,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        // Allocate a dummy local for void returns
        let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

        // Initialize class attributes for each class
        for (class_id, class_def) in &hir_module.class_defs {
            if class_def.is_protocol {
                continue;
            }
            let effective_class_id = self.get_effective_class_id(*class_id);

            // Look up class info for attribute offsets
            let class_info = self.get_class_info(class_id).cloned();

            for class_attr in &class_def.class_attrs {
                // Get attribute (owning_class_id, offset) from class info
                // For initialization, owning_class_id should be this class_id
                let attr_offset = class_info
                    .as_ref()
                    .and_then(|info| info.class_attr_offsets.get(&class_attr.name))
                    .map(|(_owning, offset)| *offset)
                    .unwrap_or(0);

                // Lower the initializer expression
                let init_expr = &hir_module.exprs[class_attr.initializer];
                let init_operand = self.lower_expr(init_expr, hir_module, mir_func)?;

                // Get the appropriate runtime function based on type
                let set_func = self.get_class_attr_set_func(&class_attr.ty);

                // Emit runtime call: rt_class_attr_set_*(class_id, attr_idx, value)
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: set_func,
                    args: vec![
                        mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                        mir::Operand::Constant(mir::Constant::Int(attr_offset as i64)),
                        init_operand,
                    ],
                });
            }
        }

        Ok(())
    }

    // ==================== Decorator Analysis Helpers ====================

    /// Find the innermost FuncRef in a chain of decorator calls
    /// e.g., dec1(dec2(FuncRef(f))) -> returns f's FuncId
    pub(crate) fn find_innermost_func_ref(
        &self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
    ) -> Option<FuncId> {
        match &expr.kind {
            hir::ExprKind::FuncRef(func_id) => Some(*func_id),
            hir::ExprKind::Call { args, .. } if args.len() == 1 => {
                // Recursively check the argument
                if let hir::CallArg::Regular(expr_id) = args[0] {
                    let arg_expr = &hir_module.exprs[expr_id];
                    self.find_innermost_func_ref(arg_expr, hir_module)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Find if a function returns a closure (for decorator pattern analysis)
    /// Returns the FuncId of the returned closure if found
    pub(crate) fn find_returned_closure(
        &self,
        func: &hir::Function,
        hir_module: &hir::Module,
    ) -> Option<FuncId> {
        // Look through the function body for return statements that return closures
        for stmt_id in &func.body {
            let stmt = &hir_module.stmts[*stmt_id];
            if let hir::StmtKind::Return(Some(expr_id)) = &stmt.kind {
                let expr = &hir_module.exprs[*expr_id];
                if let hir::ExprKind::Closure { func, .. } = &expr.kind {
                    return Some(*func);
                }
                // Check if returning a variable that holds a closure (common pattern)
                if let hir::ExprKind::Var(var_id) = &expr.kind {
                    // Check if this variable was assigned a closure in this function
                    for other_stmt_id in &func.body {
                        let other_stmt = &hir_module.stmts[*other_stmt_id];
                        if let hir::StmtKind::Assign { target, value, .. } = &other_stmt.kind {
                            if target == var_id {
                                let value_expr = &hir_module.exprs[*value];
                                if let hir::ExprKind::Closure { func, .. } = &value_expr.kind {
                                    return Some(*func);
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }
}
