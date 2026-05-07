//! Class metadata management and vtable construction
//!
//! This module handles class hierarchy processing, building class information
//! (fields, methods, vtables), and emitting class initialization code.

use indexmap::IndexMap;
use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::{Type, TypeLattice};
use pyaot_utils::{ClassId, FuncId, InternedString, VarId};

use crate::{LoweredClassInfo, Lowering};

impl<'a> Lowering<'a> {
    fn should_refine_field_seed_type(storage_ty: &Type) -> bool {
        matches!(storage_ty, Type::Any | Type::HeapAny)
            || storage_ty
                .tuple_elems()
                .is_some_and(|elems| elems.is_empty())
            || storage_ty.tuple_var_elem().is_some_and(|e| *e == Type::Any)
    }

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

    /// Refine class field seed types from constructor call sites after
    /// `build_lowering_seed_info()` has populated expression/var seed metadata.
    ///
    /// This complements the frontend's `self.field = param` scan with
    /// actual argument types observed at `ClassName(...)` call sites. The
    /// pass is intentionally lightweight and flow-sensitive only within a
    /// single straight-line CFG walk: it tracks the current semantic type of
    /// locals as statements execute, then uses that overlay to infer
    /// constructor argument types.
    ///
    /// Why this exists:
    /// - `__init__(self, children=())` seeds `_children` as `Tuple([])`,
    ///   which is too weak for later `for child in node._children: child.grad`.
    /// - numeric/operator dunders often normalize `other` before calling the
    ///   constructor again; the constructor call should see the post-rebind
    ///   semantic type, not only the wide ABI seed of the original param.
    pub(crate) fn refine_class_fields_from_constructor_calls(&mut self, hir_module: &hir::Module) {
        let init_bindings = self.collect_init_field_bindings(hir_module);
        if init_bindings.is_empty() {
            return;
        }

        let mut observed_arg_types: IndexMap<(ClassId, usize), Type> = IndexMap::new();
        for func in hir_module.func_defs.values() {
            if func.has_no_blocks() {
                continue;
            }
            let mut current_types = self.constructor_scan_seed_types(func);
            for block in func.blocks.values() {
                for &stmt_id in &block.stmts {
                    let stmt = &hir_module.stmts[stmt_id];
                    self.scan_constructor_calls_in_stmt(
                        stmt,
                        hir_module,
                        &current_types,
                        &init_bindings,
                        &mut observed_arg_types,
                    );
                    self.update_constructor_scan_types_from_stmt(
                        stmt,
                        hir_module,
                        &mut current_types,
                    );
                }
                self.scan_constructor_calls_in_terminator(
                    &block.terminator,
                    hir_module,
                    &current_types,
                    &init_bindings,
                    &mut observed_arg_types,
                );
            }
        }

        for ((class_id, param_idx), observed_ty) in observed_arg_types {
            if matches!(observed_ty, Type::Any | Type::HeapAny) {
                continue;
            }
            let Some(field_names) = init_bindings
                .get(&class_id)
                .and_then(|bindings| bindings.param_fields.get(&param_idx))
                .cloned()
            else {
                continue;
            };
            let storage_types: Vec<(InternedString, Type)> = field_names
                .iter()
                .map(|field_name| {
                    let storage_ty = self
                        .get_class_info(&class_id)
                        .and_then(|info| info.field_types.get(field_name))
                        .cloned()
                        .unwrap_or(Type::Any);
                    (*field_name, storage_ty)
                })
                .collect();
            let class_fields = self
                .lowering_seed_info
                .refined_class_field_types
                .entry(class_id)
                .or_default();
            for (field_name, storage_ty) in storage_types {
                if !Self::should_refine_field_seed_type(&storage_ty) {
                    continue;
                }
                // An empty-tuple seed (`()` from `__init__`'s default arg)
                // is a placeholder shape, not a constraint — joining it
                // with the first observed concrete tuple promotes the
                // result to `TupleVar[T]` (per the empty-tuple ⊔ tuple
                // rule), which then refuses to merge element-wise with
                // later same-arity fixed tuples and degenerates to a
                // `Union[TupleVar[Int], tuple[Float, Float]]` shape.
                // Iterating that Union yields raw bits whose runtime tag
                // doesn't match the consumer's expected element type
                // (the autograd `_local_grads` field SEGV in
                // `Value.backward()` was traced to this — `local_grad`
                // unboxed Int-tagged Value bits as a Float pointer at
                // 0x18, dereferenced, crashed). Treat empty-tuple storage
                // identically to `Any`: use the observed type directly so
                // subsequent observations element-wise join cleanly.
                let refined_raw = class_fields
                    .get(&field_name)
                    .map(|prev| prev.join(&observed_ty))
                    .unwrap_or_else(|| {
                        if matches!(storage_ty, Type::Any | Type::HeapAny) {
                            observed_ty.clone()
                        } else {
                            storage_ty.join(&observed_ty)
                        }
                    });
                // Collapse `Union[tuple[A, B], tuple[A], …]` (heterogeneous
                // fixed-arity tuples of the same element kind) into
                // `TupleVar[join_of_elements]`. The default `join` falls
                // back to a Union for different non-empty arities to
                // preserve shape distinctions for pattern matching, but
                // for an *iterable* field type that's a footgun: lowering
                // can't pick a precise `IterSourceKind` for a Union of
                // tuples (see `type_to_iter_source`), so it falls through
                // to `IterSourceKind::List` and the runtime then tries to
                // iterate the tuple as a list — reading past the inline
                // data layout and corrupting subsequent iterator state.
                // The autograd `Value._local_grads` field is the
                // motivating case: `__add__` writes a 2-tuple `(1, 1)`,
                // `__pow__` writes a 1-tuple `(d,)`, and the resulting
                // `Union[tuple[Float, Float], tuple[Float]]` produced an
                // iterator that mis-dispatched into `iter_next_dict` via
                // a corrupted `kind` field at runtime.
                let collapsed = Self::collapse_tuple_union_to_var(refined_raw);
                // Boundary coercion: `Never` (top-level or in container
                // parameters) becomes `Any` before the field type is
                // consumed by codegen / vtable layout.
                let refined = match collapsed {
                    Type::Never => Type::Any,
                    other => other.demote_never_params_to_any(),
                };
                class_fields.insert(field_name, refined);
            }
        }
    }

    /// If `ty` is `Union[tuple[…], …]` consisting entirely of same-base
    /// fixed-arity tuples (`Generic{TUPLE_ID, args}`) or already-`TupleVar`
    /// members, collapse it to a single `TupleVar[join_of_all_elements]`
    /// so iteration can pick a single deterministic source kind. Returns
    /// the type unchanged when the pattern doesn't match.
    fn collapse_tuple_union_to_var(ty: Type) -> Type {
        let Type::Union(members) = &ty else {
            return ty;
        };
        let mut element_join = Type::Never;
        for m in members {
            if let Some(elems) = m.tuple_elems() {
                for e in elems {
                    element_join = element_join.join(e);
                }
            } else if let Some(e) = m.tuple_var_elem() {
                element_join = element_join.join(e);
            } else {
                // A non-tuple member — give up and keep the Union.
                return ty;
            }
        }
        if matches!(element_join, Type::Never) {
            // All-empty tuples — meaningless; preserve the original.
            return ty;
        }
        Type::tuple_var_of(element_join)
    }

    /// Refine class field seed types from **cross-instance** attribute
    /// writes. Complements `refine_class_fields_from_constructor_calls`,
    /// which only sees `__init__` arg flow. This pass walks every function
    /// in the module looking for `Bind { target: Attr { obj, field, .. },
    /// value }` where `obj` is a class instance and `field` is one of that
    /// class's known fields. The RHS expression type (computed via the
    /// converged seed pipeline) is joined into `refined_class_field_types`
    /// so the field's effective type reflects writes through any reference,
    /// not just `self.X = ...`.
    ///
    /// Why this exists:
    ///   Autograd-style code mutates a sibling instance's field —
    ///   `child.grad += local_grad * v.grad` accumulates a Float result into
    ///   `Value.grad`, but the frontend's pre-WPA scan can't infer
    ///   `local_grad * v.grad` precisely (operands are reads from a
    ///   `tuple`/`Attribute` that need the full prescan to type). The field
    ///   was harvested as `Int` from the constructor literal `self.grad = 0`
    ///   alone, so the Float write later slammed boxed-FloatObj pointer bits
    ///   into the slot — read sites then unbox them as raw integers (or
    ///   worse, as object pointers in the `Value.backward()` SEGV).
    ///
    /// Conservative join semantics:
    ///   - `Any` / `HeapAny` RHS contributes nothing (would dilute precision).
    ///   - `Never` RHS skipped (uninitialised slot).
    ///   - Writes are joined via the lattice; reads converge through the
    ///     normal fixpoint loop in `build_lowering_seed_info`.
    pub(crate) fn refine_class_fields_from_cross_instance_writes(
        &mut self,
        hir_module: &hir::Module,
    ) {
        // Collect (func_id, class_id, field, value_expr_id) tuples first so
        // each write is paired with the function whose prescan overlay
        // should drive its `seed_infer_expr_type` query. Per-function
        // overlays are essential because zip-destructured iter targets
        // (`for child, local_grad in zip(v._children, v._local_grads)`)
        // get their narrowed types only via the per-function local seed
        // map; without that, a `BinOp` involving `child.grad` collapses
        // to `Any` in the global infer.
        let mut writes: Vec<(pyaot_utils::FuncId, ClassId, InternedString, hir::ExprId)> =
            Vec::new();

        for (fid, func) in hir_module.func_defs.iter() {
            if func.has_no_blocks() {
                continue;
            }
            for block in func.blocks.values() {
                for &stmt_id in &block.stmts {
                    let stmt = &hir_module.stmts[stmt_id];
                    if let hir::StmtKind::Bind { target, value, .. } = &stmt.kind {
                        let mut local_writes = Vec::new();
                        Self::collect_attr_writes_in_target(
                            target,
                            *value,
                            hir_module,
                            self,
                            &mut local_writes,
                        );
                        for (cid, fname, val_id) in local_writes {
                            writes.push((*fid, cid, fname, val_id));
                        }
                    }
                }
            }
        }

        let empty_overlay: IndexMap<VarId, Type> = IndexMap::new();
        for (func_id, class_id, field, value_expr_id) in writes {
            // Use the full pre-scan inference (`seed_infer_expr_type`)
            // rather than the cached read API (`seed_expr_type`): the
            // cached form has no BinOp/Compare/MethodCall/etc. arms and
            // falls back to a `lowering_seed_info.expr_types` cache
            // lookup, but the cache is only populated by
            // `eagerly_populate_expr_types` AFTER the fixpoint loop
            // converges. Our pass runs INSIDE the loop, so without the
            // full inference path every BinOp value collapses to `Any`
            // — exactly the autograd `child.grad += local_grad * v.grad`
            // case where the harvest must observe the BinOp's `Float`
            // result, not a stale `Any`.
            //
            // The per-function prescan overlay carries types for vars
            // bound by `IterAdvance` and other CFG-sensitive sites that
            // global var_types doesn't see — without the overlay,
            // `child` and `local_grad` resolve to `Any` and the whole
            // BinOp collapses again.
            let overlay = self
                .lowering_seed_info
                .per_function_local_seed_types
                .get(&func_id)
                .cloned()
                .unwrap_or_else(IndexMap::new);
            let overlay = if overlay.is_empty() {
                &empty_overlay
            } else {
                &overlay
            };
            let value_expr = &hir_module.exprs[value_expr_id];
            let value_ty = self.seed_infer_expr_type(value_expr, hir_module, overlay);
            // Skip writes whose RHS we can't type precisely — joining `Any`
            // would only erase information, and aggressive widening would
            // break code that legitimately stays inside a narrow precise
            // type (e.g. `count: int = 0; count = count + 1` — the BinOp
            // seed may transiently see `Any` while the prescan converges
            // but the steady-state remains `Int`).
            if matches!(value_ty, Type::Any | Type::HeapAny | Type::Never) {
                continue;
            }
            let storage_ty = self
                .get_class_info(&class_id)
                .and_then(|info| info.field_types.get(&field))
                .cloned()
                .unwrap_or(Type::Any);
            let class_fields = self
                .lowering_seed_info
                .refined_class_field_types
                .entry(class_id)
                .or_default();
            let refined_raw = class_fields
                .get(&field)
                .map(|prev| prev.join(&value_ty))
                .unwrap_or_else(|| {
                    if matches!(storage_ty, Type::Any | Type::HeapAny) {
                        value_ty.clone()
                    } else {
                        storage_ty.join(&value_ty)
                    }
                });
            let refined = match refined_raw {
                Type::Never => Type::Any,
                other => other.demote_never_params_to_any(),
            };
            class_fields.insert(field, refined);
        }
    }

    /// Walk a `BindingTarget` recursively, recording every leaf attribute
    /// write `obj.field = …` whose `obj` resolves to a class instance with
    /// a registered field of that name. The recursion handles tuple-unpack
    /// targets (`(self.a, self.b) = …`) which are valid in Python.
    ///
    /// When `obj_ty` cannot be narrowed (e.g. iterator-element vars whose
    /// type wasn't propagated through a tuple destructuring of a `zip` of
    /// containers with mixed-shape elements), the `field` name is matched
    /// against every known class that has a field of that name. The harvest
    /// is conservative: any matching class's field gets widened. False
    /// positives are bounded by the field-name match — a class without that
    /// field is never touched.
    fn collect_attr_writes_in_target(
        target: &hir::BindingTarget,
        value: hir::ExprId,
        hir_module: &hir::Module,
        lowering: &Lowering<'_>,
        out: &mut Vec<(ClassId, InternedString, hir::ExprId)>,
    ) {
        match target {
            hir::BindingTarget::Attr { obj, field, .. } => {
                let obj_ty = lowering.seed_expr_type(*obj, hir_module);
                let class_id = match obj_ty {
                    Type::Class { class_id, .. } => Some(class_id),
                    Type::Generic { ref base, .. } => {
                        // Generic class types (e.g. parametric instances) — the
                        // base class id still owns the field offset table.
                        if hir_module.class_defs.contains_key(base) {
                            Some(*base)
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(class_id) = class_id {
                    if let Some(info) = lowering.get_class_info(&class_id) {
                        if info.field_offsets.contains_key(field) {
                            out.push((class_id, *field, value));
                        }
                    }
                } else {
                    // obj_ty could not be narrowed — fall back to a
                    // field-name lookup across every class. Autograd-style
                    // tuple-destructured for-targets (`for child, grad in
                    // zip(v._children, v._local_grads)`) typically lose
                    // their concrete element type when one zip arg has a
                    // wider element type than the other. The write is
                    // still legitimate; harvesting it across every class
                    // that has the field is monotonic — read sites only
                    // converge faster.
                    for (cid, info) in lowering.class_info_iter() {
                        if info.field_offsets.contains_key(field) {
                            out.push((*cid, *field, value));
                        }
                    }
                }
            }
            hir::BindingTarget::Tuple { elts, .. } => {
                for elt in elts {
                    Self::collect_attr_writes_in_target(elt, value, hir_module, lowering, out);
                }
            }
            hir::BindingTarget::Starred { inner, .. } => {
                Self::collect_attr_writes_in_target(inner, value, hir_module, lowering, out);
            }
            hir::BindingTarget::Var(_)
            | hir::BindingTarget::Index { .. }
            | hir::BindingTarget::ClassAttr { .. } => {}
        }
    }

    fn collect_init_field_bindings(
        &self,
        hir_module: &hir::Module,
    ) -> IndexMap<ClassId, ConstructorFieldBindings> {
        let mut out = IndexMap::new();
        for (class_id, class_def) in &hir_module.class_defs {
            let Some(init_func_id) = class_def.init_method else {
                continue;
            };
            let Some(init_func) = hir_module.func_defs.get(&init_func_id) else {
                continue;
            };
            let Some(self_param) = init_func.params.first() else {
                continue;
            };

            let mut param_name_to_index = IndexMap::new();
            let mut param_var_to_index = IndexMap::new();
            for (idx, param) in init_func.params.iter().skip(1).enumerate() {
                param_name_to_index.insert(param.name, idx);
                param_var_to_index.insert(param.var, idx);
            }

            let mut bindings = ConstructorFieldBindings {
                param_fields: IndexMap::new(),
                param_name_to_index,
            };

            for block in init_func.blocks.values() {
                for &stmt_id in &block.stmts {
                    let stmt = &hir_module.stmts[stmt_id];
                    let hir::StmtKind::Bind { target, value, .. } = &stmt.kind else {
                        continue;
                    };
                    let hir::BindingTarget::Attr { obj, field, .. } = target else {
                        continue;
                    };
                    let hir::ExprKind::Var(obj_var) = hir_module.exprs[*obj].kind else {
                        continue;
                    };
                    if obj_var != self_param.var {
                        continue;
                    }
                    let hir::ExprKind::Var(value_var) = hir_module.exprs[*value].kind else {
                        continue;
                    };
                    let Some(param_idx) = param_var_to_index.get(&value_var).copied() else {
                        continue;
                    };
                    bindings
                        .param_fields
                        .entry(param_idx)
                        .or_default()
                        .push(*field);
                }
            }

            if !bindings.param_fields.is_empty() {
                out.insert(*class_id, bindings);
            }
        }
        out
    }

    fn constructor_scan_seed_types(&self, func: &hir::Function) -> IndexMap<VarId, Type> {
        let inferred_hints = self.get_lambda_param_type_hints(&func.id).cloned();
        let mut current_types = IndexMap::new();
        for (idx, param) in func.params.iter().enumerate() {
            if let Some(ty) = param.ty.clone().or_else(|| {
                inferred_hints
                    .as_ref()
                    .and_then(|hints| hints.get(idx).cloned())
            }) {
                current_types.insert(param.var, ty);
            }
        }
        current_types
    }

    fn scan_constructor_calls_in_stmt(
        &self,
        stmt: &hir::Stmt,
        hir_module: &hir::Module,
        current_types: &IndexMap<VarId, Type>,
        init_bindings: &IndexMap<ClassId, ConstructorFieldBindings>,
        observed_arg_types: &mut IndexMap<(ClassId, usize), Type>,
    ) {
        match &stmt.kind {
            hir::StmtKind::Bind { target, value, .. } => {
                self.scan_constructor_calls_in_binding_target(
                    target,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
                self.scan_constructor_calls_in_expr(
                    *value,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
            }
            hir::StmtKind::IterAdvance { iter, target } => {
                self.scan_constructor_calls_in_expr(
                    *iter,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
                self.scan_constructor_calls_in_binding_target(
                    target,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
            }
            hir::StmtKind::Expr(expr_id) => self.scan_constructor_calls_in_expr(
                *expr_id,
                hir_module,
                current_types,
                init_bindings,
                observed_arg_types,
            ),
            hir::StmtKind::Return(Some(expr_id)) => self.scan_constructor_calls_in_expr(
                *expr_id,
                hir_module,
                current_types,
                init_bindings,
                observed_arg_types,
            ),
            hir::StmtKind::Raise { exc, cause } => {
                if let Some(expr_id) = exc {
                    self.scan_constructor_calls_in_expr(
                        *expr_id,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
                if let Some(expr_id) = cause {
                    self.scan_constructor_calls_in_expr(
                        *expr_id,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
            }
            hir::StmtKind::Assert { cond, msg } => {
                self.scan_constructor_calls_in_expr(
                    *cond,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
                if let Some(expr_id) = msg {
                    self.scan_constructor_calls_in_expr(
                        *expr_id,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
            }
            hir::StmtKind::IndexDelete { obj, index } => {
                self.scan_constructor_calls_in_expr(
                    *obj,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
                self.scan_constructor_calls_in_expr(
                    *index,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
            }
            hir::StmtKind::Break
            | hir::StmtKind::Continue
            | hir::StmtKind::Pass
            | hir::StmtKind::Return(None)
            | hir::StmtKind::IterSetup { .. } => {}
        }
    }

    fn scan_constructor_calls_in_terminator(
        &self,
        term: &hir::HirTerminator,
        hir_module: &hir::Module,
        current_types: &IndexMap<VarId, Type>,
        init_bindings: &IndexMap<ClassId, ConstructorFieldBindings>,
        observed_arg_types: &mut IndexMap<(ClassId, usize), Type>,
    ) {
        match term {
            hir::HirTerminator::Branch { cond, .. } => self.scan_constructor_calls_in_expr(
                *cond,
                hir_module,
                current_types,
                init_bindings,
                observed_arg_types,
            ),
            hir::HirTerminator::Return(Some(expr_id))
            | hir::HirTerminator::Yield { value: expr_id, .. } => self
                .scan_constructor_calls_in_expr(
                    *expr_id,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                ),
            hir::HirTerminator::Raise { exc, cause } => {
                self.scan_constructor_calls_in_expr(
                    *exc,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
                if let Some(expr_id) = cause {
                    self.scan_constructor_calls_in_expr(
                        *expr_id,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
            }
            hir::HirTerminator::Jump(_)
            | hir::HirTerminator::Return(None)
            | hir::HirTerminator::Reraise
            | hir::HirTerminator::Unreachable => {}
        }
    }

    fn scan_constructor_calls_in_binding_target(
        &self,
        target: &hir::BindingTarget,
        hir_module: &hir::Module,
        current_types: &IndexMap<VarId, Type>,
        init_bindings: &IndexMap<ClassId, ConstructorFieldBindings>,
        observed_arg_types: &mut IndexMap<(ClassId, usize), Type>,
    ) {
        match target {
            hir::BindingTarget::Attr { obj, .. } => self.scan_constructor_calls_in_expr(
                *obj,
                hir_module,
                current_types,
                init_bindings,
                observed_arg_types,
            ),
            hir::BindingTarget::Index { obj, index, .. } => {
                self.scan_constructor_calls_in_expr(
                    *obj,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
                self.scan_constructor_calls_in_expr(
                    *index,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
            }
            hir::BindingTarget::Tuple { elts, .. } => {
                for elt in elts {
                    self.scan_constructor_calls_in_binding_target(
                        elt,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
            }
            hir::BindingTarget::Starred { inner, .. } => self
                .scan_constructor_calls_in_binding_target(
                    inner,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                ),
            hir::BindingTarget::Var(_) | hir::BindingTarget::ClassAttr { .. } => {}
        }
    }

    fn scan_constructor_calls_in_expr(
        &self,
        expr_id: hir::ExprId,
        hir_module: &hir::Module,
        current_types: &IndexMap<VarId, Type>,
        init_bindings: &IndexMap<ClassId, ConstructorFieldBindings>,
        observed_arg_types: &mut IndexMap<(ClassId, usize), Type>,
    ) {
        let expr = &hir_module.exprs[expr_id];
        match &expr.kind {
            hir::ExprKind::Call {
                func,
                args,
                kwargs,
                kwargs_unpack,
            } => {
                let func_expr = &hir_module.exprs[*func];
                if let hir::ExprKind::ClassRef(class_id) = func_expr.kind {
                    if let Some(bindings) = init_bindings.get(&class_id) {
                        for (arg_idx, arg) in args.iter().enumerate() {
                            let hir::CallArg::Regular(arg_expr_id) = arg else {
                                continue;
                            };
                            if !bindings.param_fields.contains_key(&arg_idx) {
                                continue;
                            }
                            let arg_ty = self.seed_infer_expr_type(
                                &hir_module.exprs[*arg_expr_id],
                                hir_module,
                                current_types,
                            );
                            Self::record_constructor_arg_type(
                                observed_arg_types,
                                class_id,
                                arg_idx,
                                arg_ty,
                            );
                        }
                        for kwarg in kwargs {
                            let Some(param_idx) =
                                bindings.param_name_to_index.get(&kwarg.name).copied()
                            else {
                                continue;
                            };
                            if !bindings.param_fields.contains_key(&param_idx) {
                                continue;
                            }
                            let arg_ty = self.seed_infer_expr_type(
                                &hir_module.exprs[kwarg.value],
                                hir_module,
                                current_types,
                            );
                            Self::record_constructor_arg_type(
                                observed_arg_types,
                                class_id,
                                param_idx,
                                arg_ty,
                            );
                        }
                    }
                }

                self.scan_constructor_calls_in_expr(
                    *func,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
                for arg in args {
                    let expr_id = match arg {
                        hir::CallArg::Regular(expr_id) | hir::CallArg::Starred(expr_id) => expr_id,
                    };
                    self.scan_constructor_calls_in_expr(
                        *expr_id,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
                for kwarg in kwargs {
                    self.scan_constructor_calls_in_expr(
                        kwarg.value,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
                if let Some(expr_id) = kwargs_unpack {
                    self.scan_constructor_calls_in_expr(
                        *expr_id,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
            }
            hir::ExprKind::BuiltinCall { args, kwargs, .. } => {
                for expr_id in args {
                    self.scan_constructor_calls_in_expr(
                        *expr_id,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
                for kwarg in kwargs {
                    self.scan_constructor_calls_in_expr(
                        kwarg.value,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
            }
            hir::ExprKind::IfExpr {
                cond,
                then_val,
                else_val,
            } => {
                // Apply `isinstance(var, T)` narrowing to the recursion
                // overlays so any `ClassName(...)` call inside a branch
                // sees the narrowed (or excluded) type for `var`. Without
                // this, the very common idiom
                //
                //     other = other if isinstance(other, Value) else Value(other)
                //
                // pollutes the harvested `data` field type for `Value`:
                // the `Value(other)` call in the else branch is recorded
                // with the *un-narrowed* `other` type (the WPA-harvested
                // param type, e.g. `Union[Value, int, float, bool]`),
                // which then `join`s into `data: Union[Float, Class[Value]]`
                // and propagates downstream — `_local_grads` element type
                // collapses to `Any`, the autograd `child.grad += local_grad
                // * v.grad` aug-assign loses precision, and the field write
                // corrupts tagged-Value bits at runtime (the
                // `Value.backward()` SEGV in microgpt-style code).
                //
                // The else branch must see `other.minus(Value)` —
                // `Union[int, float, bool]` collapses through the numeric
                // tower to `Float`, which is the correct `data` type for
                // numeric arithmetic.
                let cond_expr = &hir_module.exprs[*cond];
                let narrow = self.extract_simple_isinstance_narrowing(
                    cond_expr,
                    hir_module,
                    Some(current_types),
                );
                self.scan_constructor_calls_in_expr(
                    *cond,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
                if let Some((var_id, then_narrow, else_narrow)) = narrow {
                    let mut then_overlay = current_types.clone();
                    then_overlay.insert(var_id, then_narrow);
                    let mut else_overlay = current_types.clone();
                    else_overlay.insert(var_id, else_narrow);
                    self.scan_constructor_calls_in_expr(
                        *then_val,
                        hir_module,
                        &then_overlay,
                        init_bindings,
                        observed_arg_types,
                    );
                    self.scan_constructor_calls_in_expr(
                        *else_val,
                        hir_module,
                        &else_overlay,
                        init_bindings,
                        observed_arg_types,
                    );
                } else {
                    self.scan_constructor_calls_in_expr(
                        *then_val,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                    self.scan_constructor_calls_in_expr(
                        *else_val,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
            }
            hir::ExprKind::BinOp { left, right, .. }
            | hir::ExprKind::Compare { left, right, .. }
            | hir::ExprKind::LogicalOp { left, right, .. } => {
                self.scan_constructor_calls_in_expr(
                    *left,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
                self.scan_constructor_calls_in_expr(
                    *right,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
            }
            hir::ExprKind::UnOp { operand, .. }
            | hir::ExprKind::Attribute { obj: operand, .. }
            | hir::ExprKind::Yield(Some(operand))
            | hir::ExprKind::IterHasNext(operand) => self.scan_constructor_calls_in_expr(
                *operand,
                hir_module,
                current_types,
                init_bindings,
                observed_arg_types,
            ),
            hir::ExprKind::Yield(None)
            | hir::ExprKind::Int(_)
            | hir::ExprKind::Float(_)
            | hir::ExprKind::Bool(_)
            | hir::ExprKind::Str(_)
            | hir::ExprKind::Bytes(_)
            | hir::ExprKind::None
            | hir::ExprKind::NotImplemented
            | hir::ExprKind::Var(_)
            | hir::ExprKind::FuncRef(_)
            | hir::ExprKind::ClassRef(_)
            | hir::ExprKind::ClassAttrRef { .. }
            | hir::ExprKind::TypeRef(_)
            | hir::ExprKind::ImportedRef { .. }
            | hir::ExprKind::ModuleAttr { .. }
            | hir::ExprKind::BuiltinRef(_)
            | hir::ExprKind::StdlibAttr(_)
            | hir::ExprKind::StdlibConst(_)
            | hir::ExprKind::ExcCurrentValue => {}
            hir::ExprKind::FormatSpec { value, .. } => {
                self.scan_constructor_calls_in_expr(
                    *value,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
            }
            hir::ExprKind::List(items)
            | hir::ExprKind::Tuple(items)
            | hir::ExprKind::Set(items)
            | hir::ExprKind::Closure {
                captures: items, ..
            } => {
                for item in items {
                    self.scan_constructor_calls_in_expr(
                        *item,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
            }
            hir::ExprKind::Dict(entries) => {
                for (key, value) in entries {
                    self.scan_constructor_calls_in_expr(
                        *key,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                    self.scan_constructor_calls_in_expr(
                        *value,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
            }
            hir::ExprKind::Index { obj, index } => {
                self.scan_constructor_calls_in_expr(
                    *obj,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
                self.scan_constructor_calls_in_expr(
                    *index,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
            }
            hir::ExprKind::Slice {
                obj,
                start,
                end,
                step,
            } => {
                self.scan_constructor_calls_in_expr(
                    *obj,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
                if let Some(expr_id) = start {
                    self.scan_constructor_calls_in_expr(
                        *expr_id,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
                if let Some(expr_id) = end {
                    self.scan_constructor_calls_in_expr(
                        *expr_id,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
                if let Some(expr_id) = step {
                    self.scan_constructor_calls_in_expr(
                        *expr_id,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
            }
            hir::ExprKind::MethodCall {
                obj, args, kwargs, ..
            } => {
                self.scan_constructor_calls_in_expr(
                    *obj,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
                for expr_id in args {
                    self.scan_constructor_calls_in_expr(
                        *expr_id,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
                for kwarg in kwargs {
                    self.scan_constructor_calls_in_expr(
                        kwarg.value,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
            }
            hir::ExprKind::SuperCall { args, .. } | hir::ExprKind::StdlibCall { args, .. } => {
                for expr_id in args {
                    self.scan_constructor_calls_in_expr(
                        *expr_id,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
            }
            hir::ExprKind::GeneratorIntrinsic(intrinsic) => match intrinsic {
                hir::GeneratorIntrinsic::GetState(expr_id)
                | hir::GeneratorIntrinsic::SetExhausted(expr_id)
                | hir::GeneratorIntrinsic::IsExhausted(expr_id)
                | hir::GeneratorIntrinsic::GetSentValue(expr_id)
                | hir::GeneratorIntrinsic::IterNextNoExc(expr_id)
                | hir::GeneratorIntrinsic::IterIsExhausted(expr_id) => self
                    .scan_constructor_calls_in_expr(
                        *expr_id,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    ),
                hir::GeneratorIntrinsic::SetState { gen, .. }
                | hir::GeneratorIntrinsic::GetLocal { gen, .. } => self
                    .scan_constructor_calls_in_expr(
                        *gen,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    ),
                hir::GeneratorIntrinsic::SetLocal { gen, value, .. } => {
                    self.scan_constructor_calls_in_expr(
                        *gen,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                    self.scan_constructor_calls_in_expr(
                        *value,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
                hir::GeneratorIntrinsic::Create { .. } => {}
            },
            hir::ExprKind::MatchPattern { subject, pattern } => {
                self.scan_constructor_calls_in_expr(
                    *subject,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
                self.scan_constructor_calls_in_pattern(
                    pattern,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
            }
        }
    }

    fn scan_constructor_calls_in_pattern(
        &self,
        pattern: &hir::Pattern,
        hir_module: &hir::Module,
        current_types: &IndexMap<VarId, Type>,
        init_bindings: &IndexMap<ClassId, ConstructorFieldBindings>,
        observed_arg_types: &mut IndexMap<(ClassId, usize), Type>,
    ) {
        match pattern {
            hir::Pattern::MatchValue(expr_id) => self.scan_constructor_calls_in_expr(
                *expr_id,
                hir_module,
                current_types,
                init_bindings,
                observed_arg_types,
            ),
            hir::Pattern::MatchAs { pattern, .. } => {
                if let Some(inner) = pattern.as_ref() {
                    self.scan_constructor_calls_in_pattern(
                        inner,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
            }
            hir::Pattern::MatchSequence { patterns } | hir::Pattern::MatchOr(patterns) => {
                for inner in patterns {
                    self.scan_constructor_calls_in_pattern(
                        inner,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
            }
            hir::Pattern::MatchMapping { keys, patterns, .. } => {
                for expr_id in keys {
                    self.scan_constructor_calls_in_expr(
                        *expr_id,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
                for inner in patterns {
                    self.scan_constructor_calls_in_pattern(
                        inner,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
            }
            hir::Pattern::MatchClass {
                cls,
                patterns,
                kwd_patterns,
                ..
            } => {
                self.scan_constructor_calls_in_expr(
                    *cls,
                    hir_module,
                    current_types,
                    init_bindings,
                    observed_arg_types,
                );
                for inner in patterns {
                    self.scan_constructor_calls_in_pattern(
                        inner,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
                for inner in kwd_patterns {
                    self.scan_constructor_calls_in_pattern(
                        inner,
                        hir_module,
                        current_types,
                        init_bindings,
                        observed_arg_types,
                    );
                }
            }
            hir::Pattern::MatchSingleton(_) | hir::Pattern::MatchStar(_) => {}
        }
    }

    fn update_constructor_scan_types_from_stmt(
        &self,
        stmt: &hir::Stmt,
        hir_module: &hir::Module,
        current_types: &mut IndexMap<VarId, Type>,
    ) {
        match &stmt.kind {
            hir::StmtKind::Bind {
                target,
                value,
                type_hint,
            } => {
                let rhs_ty = type_hint.clone().unwrap_or_else(|| {
                    self.seed_infer_expr_type(&hir_module.exprs[*value], hir_module, current_types)
                });
                Self::assign_constructor_scan_target_types(target, &rhs_ty, current_types);
            }
            hir::StmtKind::IterAdvance { iter, target } => {
                let iter_ty =
                    self.seed_infer_expr_type(&hir_module.exprs[*iter], hir_module, current_types);
                let elem_ty = Self::constructor_scan_iter_elem_type(&iter_ty);
                Self::assign_constructor_scan_target_types(target, &elem_ty, current_types);
            }
            hir::StmtKind::Expr(_)
            | hir::StmtKind::Return(_)
            | hir::StmtKind::Raise { .. }
            | hir::StmtKind::Assert { .. }
            | hir::StmtKind::IndexDelete { .. }
            | hir::StmtKind::Break
            | hir::StmtKind::Continue
            | hir::StmtKind::Pass
            | hir::StmtKind::IterSetup { .. } => {}
        }
    }

    fn assign_constructor_scan_target_types(
        target: &hir::BindingTarget,
        value_ty: &Type,
        current_types: &mut IndexMap<VarId, Type>,
    ) {
        match target {
            hir::BindingTarget::Var(var_id) => {
                current_types.insert(*var_id, value_ty.clone());
            }
            hir::BindingTarget::Tuple { elts, .. } => {
                if let Some(types) = value_ty.tuple_elems() {
                    for (elt, ty) in elts.iter().zip(types.iter()) {
                        Self::assign_constructor_scan_target_types(elt, ty, current_types);
                    }
                    if types.len() < elts.len() {
                        for elt in &elts[types.len()..] {
                            Self::assign_constructor_scan_target_types(
                                elt,
                                &Type::Any,
                                current_types,
                            );
                        }
                    }
                } else if let Some(elem_ty) = value_ty.tuple_var_elem() {
                    for elt in elts {
                        Self::assign_constructor_scan_target_types(elt, elem_ty, current_types);
                    }
                } else {
                    for elt in elts {
                        Self::assign_constructor_scan_target_types(elt, &Type::Any, current_types);
                    }
                }
            }
            hir::BindingTarget::Starred { inner, .. } => {
                let starred_ty = Type::list_of(value_ty.clone());
                Self::assign_constructor_scan_target_types(inner, &starred_ty, current_types);
            }
            hir::BindingTarget::Attr { .. }
            | hir::BindingTarget::Index { .. }
            | hir::BindingTarget::ClassAttr { .. } => {}
        }
    }

    fn constructor_scan_iter_elem_type(ty: &Type) -> Type {
        if let Some(e) = ty.list_elem() {
            return e.clone();
        }
        if let Some(e) = ty.set_elem() {
            return e.clone();
        }
        if let Some(e) = ty.tuple_var_elem() {
            return e.clone();
        }
        if let Some(elems) = ty.tuple_elems() {
            return if !elems.is_empty() {
                elems
                    .iter()
                    .cloned()
                    .reduce(|a, b| a.join(&b))
                    .unwrap_or(Type::Never)
            } else {
                Type::Any
            };
        }
        if let Some((k, _)) = ty.dict_kv() {
            return k.clone();
        }
        match ty {
            Type::Iterator(e) => (**e).clone(),
            Type::Str => Type::Str,
            Type::Bytes => Type::Int,
            _ => Type::Any,
        }
    }

    fn record_constructor_arg_type(
        observed_arg_types: &mut IndexMap<(ClassId, usize), Type>,
        class_id: ClassId,
        param_idx: usize,
        arg_ty: Type,
    ) {
        if matches!(arg_ty, Type::Any | Type::HeapAny) {
            return;
        }
        observed_arg_types
            .entry((class_id, param_idx))
            .and_modify(|prev| *prev = prev.join(&arg_ty))
            .or_insert(arg_ty);
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
                                    name_hash: pyaot_utils::fnv1a_hash(self.resolve(*name)),
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

            // §F.7: per-class field-heap mask eliminated. Every instance
            // field slot is a properly-tagged `Value` (Int/Bool via
            // ValueFromInt/Bool, Float as boxed FloatObj pointer per
            // §F.1, heap shapes as pointers). The GC's `mark_object`
            // walk uses `Value::is_ptr()` per slot — no per-class mask
            // is needed.

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

            // Register raw f64 field mask: bit k = 1 means field k holds a
            // raw f64 bit pattern rather than a tagged Value pointer. The GC
            // must skip those slots to avoid dereferencing float bits as ptrs.
            let raw_field_mask: u64 = self
                .get_class_info(class_id)
                .map(|ci| {
                    ci.field_offsets
                        .iter()
                        .filter(|(name, _)| {
                            matches!(ci.field_types.get(*name), Some(pyaot_types::Type::Float))
                        })
                        .fold(
                            0u64,
                            |m, (_, &off)| {
                                if off < 64 {
                                    m | (1u64 << off)
                                } else {
                                    m
                                }
                            },
                        )
                })
                .unwrap_or(0);
            if raw_field_mask != 0 {
                self.emit_runtime_call_void(
                    mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_REGISTER_CLASS_RAW_FIELD_MASK,
                    ),
                    vec![
                        mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                        mir::Operand::Constant(mir::Constant::Int(raw_field_mask as i64)),
                    ],
                    mir_func,
                );
            }

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

            // Also register dunder methods for Protocol isinstance structural checks.
            // Dunders have no vtable slot (use i64::MAX sentinel); rt_obj_has_method
            // checks existence only and does not use the slot for vtable dispatch.
            let dunder_hashes: Vec<i64> = self
                .get_class_info(class_id)
                .map(|ci| {
                    ci.dunder_methods
                        .keys()
                        .map(|&name| pyaot_utils::fnv1a_hash(name) as i64)
                        .collect()
                })
                .unwrap_or_default();
            for name_hash in dunder_hashes {
                self.emit_runtime_call_void(
                    mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_REGISTER_METHOD_NAME,
                    ),
                    vec![
                        mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                        mir::Operand::Constant(mir::Constant::Int(name_hash)),
                        mir::Operand::Constant(mir::Constant::Int(i64::MAX)),
                    ],
                    mir_func,
                );
            }

            // Register dunder function pointers in DUNDER_FUNC_REGISTRY so
            // runtime arithmetic ops (`rt_obj_add`, `rt_obj_mul`, etc.) can
            // dispatch through user-defined dunders when an operand turns
            // out to be a class instance at runtime (Union[Class, ...] +
            // Class case). Distinct from METHOD_NAME_REGISTRY (Protocol
            // dispatch via vtable slot) — dunders have no vtable slot.
            let dunder_funcs: Vec<(i64, pyaot_utils::FuncId)> = self
                .get_class_info(class_id)
                .map(|ci| {
                    ci.dunder_methods
                        .iter()
                        .map(|(&name, &func_id)| (pyaot_utils::fnv1a_hash(name) as i64, func_id))
                        .collect()
                })
                .unwrap_or_default();
            for (name_hash, func_id) in dunder_funcs {
                let func_addr_local = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::FuncAddr {
                    dest: func_addr_local,
                    func: func_id,
                });
                self.emit_runtime_call_void(
                    mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_REGISTER_DUNDER_FUNC,
                    ),
                    vec![
                        mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                        mir::Operand::Constant(mir::Constant::Int(name_hash)),
                        mir::Operand::Local(func_addr_local),
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

    /// Find if a function returns a closure (for decorator pattern analysis).
    /// Returns the FuncId of the returned closure if found.
    ///
    /// §1.17b-d — reads `Return(Some(expr))` from CFG block terminators.
    /// The first matching block wins (same semantics as the former tree
    /// walk's first-Return-stmt). Variable-rebinding search now walks all
    /// block stmts across the function.
    pub(crate) fn find_returned_closure(
        &self,
        func: &hir::Function,
        hir_module: &hir::Module,
    ) -> Option<FuncId> {
        for block in func.blocks.values() {
            let return_expr_id = match &block.terminator {
                hir::HirTerminator::Return(Some(e)) => Some(*e),
                _ => None,
            };
            let Some(expr_id) = return_expr_id else {
                continue;
            };
            let expr = &hir_module.exprs[expr_id];
            if let hir::ExprKind::Closure {
                func: closure_fn, ..
            } = &expr.kind
            {
                return Some(*closure_fn);
            }
            // Check if returning a variable that holds a closure (common pattern)
            if let hir::ExprKind::Var(var_id) = &expr.kind {
                for search_block in func.blocks.values() {
                    for &other_stmt_id in &search_block.stmts {
                        let other_stmt = &hir_module.stmts[other_stmt_id];
                        if let hir::StmtKind::Bind {
                            target: hir::BindingTarget::Var(target_var),
                            value,
                            ..
                        } = &other_stmt.kind
                        {
                            if target_var == var_id {
                                let value_expr = &hir_module.exprs[*value];
                                if let hir::ExprKind::Closure {
                                    func: closure_fn, ..
                                } = &value_expr.kind
                                {
                                    return Some(*closure_fn);
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

#[derive(Debug, Default, Clone)]
struct ConstructorFieldBindings {
    param_fields: IndexMap<usize, Vec<InternedString>>,
    param_name_to_index: IndexMap<InternedString, usize>,
}
