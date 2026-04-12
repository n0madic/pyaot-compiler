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

            // Start with inherited fields/methods/vtable from parent, or a fresh blank info.
            let (mut info, own_field_offset) = if let Some(base_id) = class_def.base_class {
                let parent_info = self
                    .get_class_info(&base_id)
                    .expect("Parent class must be processed first");
                let offset = parent_info.total_field_count;
                (parent_info.clone(), offset)
            } else {
                (
                    LoweredClassInfo {
                        field_offsets: IndexMap::new(),
                        field_types: IndexMap::new(),
                        method_funcs: IndexMap::new(),
                        init_func: None,
                        dunder_methods: IndexMap::new(),
                        base_class: None,
                        total_field_count: 0,
                        own_field_offset: 0,
                        vtable_slots: IndexMap::new(),
                        class_attr_offsets: IndexMap::new(),
                        class_attr_types: IndexMap::new(),
                        static_methods: IndexMap::new(),
                        class_methods: IndexMap::new(),
                        properties: IndexMap::new(),
                        property_types: IndexMap::new(),
                        is_exception_class: false,
                    },
                    0,
                )
            };

            // Add this class's own fields (starting after inherited fields).
            // Skip fields already inherited from parent to maintain consistent offsets
            // across the inheritance hierarchy (required for class pattern matching).
            let mut own_field_idx = 0;
            for field in class_def.fields.iter() {
                if info.field_offsets.contains_key(&field.name) {
                    // Inherited field — keep parent's offset, update type if refined.
                    // Warn when the child declares the field with a different type than
                    // the parent, because this can cause silent mismatches at runtime.
                    if let Some(parent_ty) = info.field_types.get(&field.name) {
                        if *parent_ty != field.ty {
                            let class_name = self.resolve(class_def.name);
                            let field_name = self.resolve(field.name);
                            eprintln!(
                                "warning: class '{}' overrides inherited field '{}' \
                                 with a different type (parent: {:?}, child: {:?})",
                                class_name, field_name, parent_ty, field.ty
                            );
                        }
                    }
                    info.field_types.insert(field.name, field.ty.clone());
                } else {
                    let offset = own_field_offset + own_field_idx;
                    info.field_offsets.insert(field.name, offset);
                    info.field_types.insert(field.name, field.ty.clone());
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

                    // Detect dunder methods and track them via set_dunder_func.
                    // Non-dunder names (and __init__, which is handled separately) fall through.
                    if !info.set_dunder_func(method_name_str, *method_id) {
                        // For non-dunder methods, we need to intern the name and add to maps.
                        // Look up without mutation - if not found, method was never called
                        // as a method call (e.g., __init__ called via instantiation).
                        if let Some(method_name) = self.lookup_interned(method_name_str) {
                            // Route method to appropriate map based on method_kind
                            match func.method_kind {
                                hir::MethodKind::Static => {
                                    // Static methods: no self/cls, skip vtable
                                    info.static_methods.insert(method_name, *method_id);
                                }
                                hir::MethodKind::ClassMethod => {
                                    // Class methods: receives cls, skip vtable
                                    info.class_methods.insert(method_name, *method_id);
                                }
                                hir::MethodKind::Instance => {
                                    // Instance methods: regular virtual dispatch
                                    info.method_funcs.insert(method_name, *method_id);

                                    // Update vtable: reuse existing slot if overriding, else allocate new slot
                                    if !info.vtable_slots.contains_key(&method_name) {
                                        let slot = info.vtable_slots.len();
                                        info.vtable_slots.insert(method_name, slot);
                                    }
                                    // If method already in vtable (inherited), we reuse the same slot
                                    // but the method_funcs map now points to the overriding method
                                }
                            }
                        }
                    }
                }
            }

            // Build property info from HIR PropertyDef
            for prop in &class_def.properties {
                info.properties
                    .insert(prop.name, (prop.getter, prop.setter));
                info.property_types.insert(prop.name, prop.ty.clone());
            }

            info.init_func = class_def.init_method;
            info.base_class = class_def.base_class;
            info.own_field_offset = own_field_offset;
            info.total_field_count = own_field_offset + own_field_idx;
            info.is_exception_class = class_def.is_exception_class;

            // Build class attribute info (inherited from parent + own).
            // For inherited attributes, we keep the parent's (class_id, offset) to ensure
            // that accessing an inherited attribute uses the parent's storage.
            // The parent clone already populated class_attr_offsets/class_attr_types for
            // the inherited case; for a root class they start empty — both paths are correct.
            let own_attr_offset = info.class_attr_offsets.len();
            for (i, class_attr) in class_def.class_attrs.iter().enumerate() {
                let offset = own_attr_offset + i;
                // Store (owning_class_id, offset) so we know where the attribute is defined
                info.class_attr_offsets
                    .insert(class_attr.name, (class_id, offset));
                info.class_attr_types
                    .insert(class_attr.name, class_attr.ty.clone());
            }

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

            self.emit_runtime_call_void(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_REGISTER_CLASS),
                vec![
                    mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                    mir::Operand::Constant(mir::Constant::Int(parent_class_id)),
                ],
                mir_func,
            );

            // Compute heap field mask: bit i is set if field i is a heap type (pointer)
            // that needs GC tracing. Raw values (Int, Float, Bool, None) are NOT heap.
            // Use class_info.field_types (which includes inherited + own fields in correct
            // absolute order) to ensure inherited heap fields are also tracked by the GC.
            let mut heap_field_mask: i64 = 0;
            if let Some(class_info) = self.get_class_info(class_id) {
                if class_info.field_types.len() > 64 {
                    let class_name = self.resolve(class_def.name);
                    eprintln!(
                        "warning: class '{}' has {} fields (max 64 for GC heap field tracking); \
                         fields beyond index 63 will not have precise GC tracing",
                        class_name,
                        class_info.field_types.len()
                    );
                }
                for (i, (_name, ty)) in class_info.field_types.iter().enumerate() {
                    if i >= 64 {
                        break;
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
            self.emit_runtime_call_void(
                mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_REGISTER_CLASS_FIELDS,
                ),
                vec![
                    mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                    mir::Operand::Constant(mir::Constant::Int(heap_field_mask)),
                ],
                mir_func,
            );

            // Register field count for object.__new__ support
            let total_field_count = self
                .get_class_info(class_id)
                .map(|ci| ci.total_field_count as i64)
                .unwrap_or(0);
            self.emit_runtime_call_void(
                mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_REGISTER_CLASS_FIELD_COUNT,
                ),
                vec![
                    mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                    mir::Operand::Constant(mir::Constant::Int(total_field_count)),
                ],
                mir_func,
            );

            // Register dunder function pointers (__del__, __copy__, __deepcopy__)
            // These are called from the runtime via function pointer registries.
            let dunder_registrations: Vec<(FuncId, mir::RuntimeFunc)> = self
                .get_class_info(class_id)
                .map(|ci| {
                    let mut regs = Vec::new();
                    if let Some(f) = ci.get_dunder_func("__del__") {
                        regs.push((
                            f,
                            mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_REGISTER_DEL_FUNC,
                            ),
                        ));
                    }
                    if let Some(f) = ci.get_dunder_func("__copy__") {
                        regs.push((
                            f,
                            mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_REGISTER_COPY_FUNC,
                            ),
                        ));
                    }
                    if let Some(f) = ci.get_dunder_func("__deepcopy__") {
                        regs.push((
                            f,
                            mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_REGISTER_DEEPCOPY_FUNC,
                            ),
                        ));
                    }
                    regs
                })
                .unwrap_or_default();
            for (func_id, reg_func) in dunder_registrations {
                // Get compiled function address
                let func_addr_local = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::FuncAddr {
                    dest: func_addr_local,
                    func: func_id,
                });
                self.emit_runtime_call_void(
                    reg_func,
                    vec![
                        mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                        mir::Operand::Local(func_addr_local),
                    ],
                    mir_func,
                );
            }

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
                self.emit_runtime_call_void(
                    mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_REGISTER_METHOD_NAME,
                    ),
                    vec![
                        mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                        mir::Operand::Constant(mir::Constant::Int(name_hash)),
                        mir::Operand::Constant(mir::Constant::Int(slot)),
                    ],
                    mir_func,
                );
            }

            // For exception classes, also register the class name for error messages
            if class_def.is_exception_class {
                // class_def.name is already an InternedString, use it directly
                self.emit_runtime_call_void(
                    mir::RuntimeFunc::ExcRegisterClassName,
                    vec![
                        mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                        mir::Operand::Constant(mir::Constant::Str(class_def.name)),
                    ],
                    mir_func,
                );
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
                self.emit_runtime_call_void(
                    set_func,
                    vec![
                        mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                        mir::Operand::Constant(mir::Constant::Int(attr_offset as i64)),
                        init_operand,
                    ],
                    mir_func,
                );
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
