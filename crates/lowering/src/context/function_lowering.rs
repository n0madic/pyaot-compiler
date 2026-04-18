//! Module and function lowering entry points

use pyaot_diagnostics::{CompilerWarnings, Result};
use pyaot_hir as hir;
use pyaot_mir::{self as mir, ValueKind};
use pyaot_types::Type;

use super::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a complete HIR module to MIR.
    /// Returns the MIR module and any warnings collected during lowering.
    pub fn lower_module(
        mut self,
        mut hir_module: hir::Module,
    ) -> Result<(mir::Module, CompilerWarnings)> {
        // Copy global variables set from HIR module
        self.symbols.globals = hir_module.globals.clone();

        // Pre-populate global variable types from module init function
        // This must happen before lowering any functions since they may reference globals
        self.scan_global_var_types(&hir_module);

        // First pass: build class info
        self.build_class_info(&hir_module);

        // Desugar generator functions into regular functions at HIR level.
        // Must run after build_class_info (needs class field info for yield type
        // inference) and before function name map / type planning (so the desugared
        // functions are visible to both).
        self.desugar_generators(&mut hir_module)?;

        // Second pass: build function name map
        for func_id in &hir_module.functions {
            if let Some(func) = hir_module.func_defs.get(func_id) {
                let func_name = self.interner.resolve(func.name).to_string();
                self.symbols.func_name_map.insert(func_name, *func_id);
            }
        }

        // Pass 2.5: scan for mutable default parameters and allocate global slots
        // In Python, mutable defaults (list, dict, set, class instances) are evaluated
        // once at function definition time and shared across all calls.
        self.scan_mutable_defaults(&hir_module);

        // Phase 1: Type Planning — pre-scan + compute types for all expressions
        // Fills type_map, closure_capture_types, lambda_param_type_hints, func_return_types
        self.run_type_planning(&hir_module);

        // Phase 2: Code Generation — lower functions using type_map
        // After desugaring, no functions should have is_generator=true
        for func_id in &hir_module.functions {
            if let Some(func) = hir_module.func_defs.get(func_id) {
                debug_assert!(
                    !func.is_generator,
                    "generator function not desugared: {:?}",
                    func.name
                );
                let mir_func = self.lower_function(func, &hir_module)?;
                self.mir_module.add_function(mir_func);
            }
        }

        // Fifth pass: export vtable information
        self.build_vtables();

        // Sixth pass: project per-class metadata into `mir::Module.class_info`
        // so optimizer passes (WPA field inference) can consume it.
        for (class_id, info) in self.classes.class_info.iter() {
            self.mir_module.class_info.insert(
                *class_id,
                mir::ClassMetadata {
                    class_id: *class_id,
                    init_func_id: info.init_func,
                    field_offsets: info.field_offsets.clone(),
                    field_types: info.field_types.clone(),
                    base_class: info.base_class,
                },
            );
        }

        // Return both MIR module and collected warnings
        Ok((self.mir_module, self.warnings))
    }

    /// Lower a single HIR function to MIR
    pub(crate) fn lower_function(
        &mut self,
        func: &hir::Function,
        hir_module: &hir::Module,
    ) -> Result<mir::Function> {
        // Reset per-function state
        self.symbols.var_to_local.clear();
        self.symbols.var_types.clear();
        self.hir_types.prescan_var_types.clear();
        self.symbols.var_to_func.clear();
        self.closures.var_to_closure.clear();
        self.closures.var_to_wrapper.clear();
        self.closures.dynamic_closure_vars.clear();
        self.closures.func_ptr_params.clear();
        self.closures.varargs_params.clear();
        self.codegen.current_blocks.clear();
        self.codegen.current_block_idx = 0;
        self.codegen.next_block_id = 0;
        self.codegen.next_local_id = 0;
        self.symbols.cell_vars.clear();
        self.symbols.nonlocal_cells.clear();
        self.hir_types.narrowed_union_vars.clear();
        self.codegen.loop_stack.clear();
        self.codegen.expected_type = None;
        self.codegen.pending_varargs_from_unpack = None;
        self.codegen.pending_kwargs_from_unpack = None;
        // expr_types NOT cleared — ExprIds are unique per-module, so
        // memoized types from other functions remain valid.

        // Copy cell_vars and nonlocal_vars from HIR function
        // (nonlocal_vars will be mapped to cell locals during parameter processing)
        for var_id in func.cell_vars.iter().chain(func.nonlocal_vars.iter()) {
            self.symbols.cell_vars.insert(*var_id);
        }

        // Check if this function is a wrapper (closure returned by a decorator).
        // If so, mark the function-pointer parameter for indirect calls.
        let is_wrapper_func = self.is_wrapper_func(&func.id);
        if is_wrapper_func && !func.params.is_empty() {
            // The pre-scan recorded the decorator's function-parameter name (e.g. "f", "fn",
            // "decorated") alongside the wrapper FuncId. Use it to find the right parameter
            // regardless of what name the user chose. Fall back to the hardcoded names
            // "func" / "__capture_func" for wrappers not covered by the pre-scan.
            let known_param_name = self.closures.wrapper_func_param_name.get(&func.id).copied();
            for param in &func.params {
                let param_name = self.interner.resolve(param.name);
                let is_func_ptr = if let Some(known) = known_param_name {
                    let known_str = self.interner.resolve(known);
                    let capture_variant = format!("__capture_{}", known_str);
                    param_name == known_str || param_name == capture_variant.as_str()
                } else {
                    // Fallback: pre-scan didn't record a name (e.g. complex decorator factory)
                    param_name == "func" || param_name == "__capture_func"
                };
                if is_func_ptr {
                    self.insert_func_ptr_param(param.var);
                    break;
                }
            }
        }

        let func_name = self.interner.resolve(func.name).to_string();
        let is_lambda = func_name.starts_with("__lambda_") || func_name.starts_with("__nested_");
        // Gen-expr functions receive their captures as implicit leading params
        // (see `desugar_generator_expression` in frontend-python/comprehensions.rs).
        // Reuse the lambda capture-type inference path so those params get the
        // concrete outer-var types instead of defaulting to `Any`, which would
        // mis-tag raw-int lists as ELEM_HEAP_OBJ in iterator setup.
        let is_genexp_creator = func_name.starts_with("__genexp_");
        let is_module_init = func_name == "__pyaot_module_init__";

        // For lambdas and gen-expr creators, infer parameter types from captures.
        let inferred_param_types = if (is_lambda || is_genexp_creator) && !func.body.is_empty() {
            self.infer_lambda_param_types(func, hir_module)
        } else {
            Vec::new()
        };

        // Convert parameters from HIR to MIR
        let mut params = Vec::new();
        for (i, hir_param) in func.params.iter().enumerate() {
            let local_id = self.alloc_local_id();

            // Check if this parameter is a cell pointer (nonlocal variable)
            let is_cell_param = func.nonlocal_vars.contains(&hir_param.var);

            // Use declared type if available, otherwise inferred type, otherwise Any
            // Declared types take precedence over inferred types
            let base_ty = hir_param.ty.clone().unwrap_or_else(|| {
                if i < inferred_param_types.len() {
                    inferred_param_types[i].clone()
                } else {
                    Type::Any
                }
            });

            // Track VarPositional params for runtime *args forwarding
            if hir_param.kind == hir::ParamKind::VarPositional {
                self.closures.varargs_params.insert(hir_param.var);
            }

            // For nonlocal parameters, the type is a cell pointer (heap object pointer)
            let param_ty = if is_cell_param {
                Type::HeapAny
            } else {
                base_ty
            };

            // Register parameter variable
            self.insert_var_local(hir_param.var, local_id);
            // Track parameter type for type inference (use the underlying value type, not cell type)
            // This is needed so that reading from the cell returns the correct type
            if is_cell_param {
                // Get the underlying value type from inferred types or default to Any
                let value_ty = if i < inferred_param_types.len() {
                    inferred_param_types[i].clone()
                } else {
                    Type::Any
                };
                self.insert_var_type(hir_param.var, value_ty);
            } else {
                self.insert_var_type(hir_param.var, param_ty.clone());
            }

            let mir_param = mir::Local {
                id: local_id,
                name: Some(hir_param.name),
                ty: param_ty.clone(),
                is_gc_root: is_cell_param || param_ty.is_heap(), // Cells are heap objects
            };
            params.push(mir_param);
        }

        // Area E §E.6 — copy this function's pre-scan results (computed
        // during `run_type_planning::precompute_all_local_var_types`) into
        // the active `prescan_var_types` map. `get_or_create_local` and
        // `lower_assign` consult it to size MIR locals and coerce RHS
        // values through the numeric tower.
        if let Some(prescanned) = self
            .hir_types
            .per_function_prescan_var_types
            .get(&func.id)
            .cloned()
        {
            for (var_id, ty) in prescanned {
                // Params whose signature carries a concrete type keep
                // the signature type — the MIR local is allocated once
                // at that type, and overriding it here would break the
                // caller ABI (boxing, arg coercion). For unannotated
                // params (signature `None` → `Any`) the prescan
                // override is safe because the param is otherwise
                // `Any`-typed and won't receive coerced values at the
                // call site (§G.13: `other = other if isinstance(other,
                // Value) else Value(other)` in a plain function
                // narrows `other` to `Value`).
                let is_annotated_param = func
                    .params
                    .iter()
                    .any(|p| p.var == var_id && p.ty.is_some());
                if !is_annotated_param {
                    self.hir_types.prescan_var_types.insert(var_id, ty);
                }
            }
        }

        // Infer return type for functions without explicit return type annotation
        // The frontend sets return_type to Some(Type::None) as the default when no annotation
        // is provided, so we need to infer the actual type from the function body.
        // Only infer when there's no explicit annotation (None or Some(Type::None))
        let has_explicit_return_type =
            func.return_type.is_some() && func.return_type.as_ref() != Some(&Type::None);
        let needs_return_type_inference = !func.body.is_empty() && !has_explicit_return_type;
        let return_type = if needs_return_type_inference {
            // Prefer the return type already inferred by the type-planning
            // pass (`infer_all_return_types`), which walks the full body
            // including nested if/for/try and uses declared param types.
            // Fall back to the lambda-style single-top-level-return inference
            // only if that pass produced nothing (empty body, unreachable,
            // or the pass couldn't see this function).
            let from_planning = self.get_func_return_type(&func.id).cloned();
            from_planning.unwrap_or_else(|| {
                let lambda_inferred = self.infer_lambda_return_type(func, hir_module);
                if lambda_inferred == Type::None
                    && self.find_returned_closure(func, hir_module).is_some()
                {
                    Type::Any
                } else {
                    lambda_inferred
                }
            })
        } else {
            // Use the declared return type
            func.return_type.clone().unwrap_or(Type::None)
        };

        // Store the inferred return type for later lookup
        self.insert_func_return_type(func.id, return_type.clone());

        // Store the current function's return type for type inference during lowering
        self.symbols.current_func_return_type = Some(return_type.clone());

        let mut mir_func = mir::Function::new(
            func.id,
            func_name,
            params.clone(),
            return_type,
            Some(func.span),
        );

        // Add parameters to locals
        for param in params {
            mir_func.add_local(param);
        }

        let entry_block = self.new_block();
        self.push_block(entry_block);

        // Initialize cells for cell_vars (variables used by inner functions via nonlocal)
        // These need to be wrapped in cells from the start
        for &var_id in &func.cell_vars {
            // Skip if this is a nonlocal var (it's already a cell passed as parameter)
            if func.nonlocal_vars.contains(&var_id) {
                continue;
            }

            let var_type = self.get_var_type(&var_id).cloned().unwrap_or(Type::Int);

            // Get the current local for this variable (might be a parameter or declared later)
            let initial_local = self.get_var_local(&var_id);

            // Create the cell with the initial value (or default if not yet initialized)
            let make_cell_func = self.get_make_cell_func(&var_type);
            let initial_value = if let Some(local_id) = initial_local {
                mir::Operand::Local(local_id)
            } else {
                // Default initial value based on type
                match &var_type {
                    Type::Int => mir::Operand::Constant(mir::Constant::Int(0)),
                    Type::Float => mir::Operand::Constant(mir::Constant::Float(0.0)),
                    Type::Bool => mir::Operand::Constant(mir::Constant::Bool(false)),
                    _ => mir::Operand::Constant(mir::Constant::None),
                }
            };

            // Create a local to hold the cell pointer (heap object, always GC root)
            let cell_local = self.emit_runtime_call_gc(
                make_cell_func,
                vec![initial_value],
                Type::HeapAny,
                &mut mir_func,
            );

            // Map this variable to its cell for later reads/writes
            self.symbols.nonlocal_cells.insert(var_id, cell_local);
        }

        // For nonlocal_vars, the parameter is already a cell pointer
        // Map them to nonlocal_cells for later reads/writes
        for &var_id in &func.nonlocal_vars {
            if let Some(local_id) = self.get_var_local(&var_id) {
                self.symbols.nonlocal_cells.insert(var_id, local_id);
            }
        }

        // For module init function, emit initialization code that must run first
        // This must happen before any class instantiation or function calls
        if is_module_init {
            // Initialize mutable default parameters (evaluated once, shared across calls)
            self.emit_mutable_default_initializations(hir_module, &mut mir_func)?;
            // Register classes for inheritance
            self.emit_class_registrations(hir_module, &mut mir_func);
            self.emit_class_attr_initializations(hir_module, &mut mir_func)?;
        }

        for stmt_id in &func.body {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, &mut mir_func)?;
        }

        if !self.current_block_has_terminator() {
            // Create a default return value that matches the function's return type
            // For abstract methods (which have pass bodies), this provides a dummy return value.
            // Since abstract classes can't be instantiated, these methods won't actually be called.
            let default_return = match &mir_func.return_type {
                Type::Int => mir::Operand::Constant(mir::Constant::Int(0)),
                Type::Float => mir::Operand::Constant(mir::Constant::Float(0.0)),
                Type::Bool => mir::Operand::Constant(mir::Constant::Bool(false)),
                Type::Str => {
                    // For string return types, we need to return a valid string pointer.
                    // Create an empty string via runtime call.
                    let empty_str = self.interner.intern("");
                    let str_local = self.emit_runtime_call(
                        mir::RuntimeFunc::MakeStr,
                        vec![mir::Operand::Constant(mir::Constant::Str(empty_str))],
                        Type::Str,
                        &mut mir_func,
                    );
                    mir::Operand::Local(str_local)
                }
                _ => mir::Operand::Constant(mir::Constant::None),
            };
            self.current_block_mut().terminator = mir::Terminator::Return(Some(default_return));
        }

        for block in self.codegen.current_blocks.drain(..) {
            mir_func.blocks.insert(block.id, block);
        }

        Ok(mir_func)
    }

    /// Scan all functions for mutable default parameters and allocate global slots.
    /// In Python, mutable defaults (list, dict, set, class instances) are evaluated
    /// once at function definition time and shared across all calls.
    pub(crate) fn scan_mutable_defaults(&mut self, hir_module: &hir::Module) {
        use crate::utils::is_mutable_default_expr;

        for func_id in &hir_module.functions {
            if let Some(func) = hir_module.func_defs.get(func_id) {
                for (param_idx, param) in func.params.iter().enumerate() {
                    if let Some(default_id) = param.default {
                        let default_expr = &hir_module.exprs[default_id];
                        if is_mutable_default_expr(default_expr) {
                            // Allocate a global slot for this mutable default
                            let slot = self.symbols.next_default_slot;
                            self.symbols.next_default_slot += 1;
                            assert!(
                                self.symbols.next_default_slot > slot,
                                "next_default_slot overflow: mutable default slot counter wrapped"
                            );
                            self.symbols
                                .default_value_slots
                                .insert((*func_id, param_idx), slot);
                        }
                    }
                }
            }
        }
    }

    /// Emit initialization code for mutable default parameters.
    /// This is called at the start of __pyaot_module_init__ to evaluate all
    /// mutable defaults once and store them in global slots.
    pub(crate) fn emit_mutable_default_initializations(
        &mut self,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        // Collect all defaults that need initialization
        // We need to clone the keys to avoid borrow issues
        let slots_to_init: Vec<((pyaot_utils::FuncId, usize), u32)> = self
            .symbols
            .default_value_slots
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();

        for ((func_id, param_idx), slot) in slots_to_init {
            // Get the function and parameter
            let Some(func) = hir_module.func_defs.get(&func_id) else {
                eprintln!(
                    "warning: function {:?} not found for mutable default initialization",
                    func_id
                );
                continue;
            };
            let param = &func.params[param_idx];

            if let Some(default_id) = param.default {
                let default_expr = &hir_module.exprs[default_id];

                // Lower the default expression with expected type for correct elem_tag
                let default_operand = self.lower_expr_expecting(
                    default_expr,
                    param.ty.clone(),
                    hir_module,
                    mir_func,
                )?;

                // Store in global slot - mutable defaults are always heap types (ptr)
                self.emit_runtime_call(
                    mir::RuntimeFunc::Call(ValueKind::Ptr.global_set_def()),
                    vec![
                        mir::Operand::Constant(mir::Constant::Int(slot as i64)),
                        default_operand,
                    ],
                    Type::None,
                    mir_func,
                );
            }
        }

        Ok(())
    }

    /// Pre-populate global variable types from module init function statements.
    /// This must run before lowering any functions since they may reference globals.
    /// Only stores types for variables with explicit type hints to avoid incorrect inference.
    fn scan_global_var_types(&mut self, hir_module: &hir::Module) {
        // Find the module init function (name is __pyaot_module_init__)
        let init_func = hir_module
            .func_defs
            .values()
            .find(|f| self.interner.resolve(f.name) == "__pyaot_module_init__");

        let Some(init_func) = init_func else {
            return;
        };

        // Scan statements in module init for assignments to global variables.
        // Records the global's type for persistence across function boundaries:
        // explicit annotations take precedence; when absent, best-effort
        // inference from the RHS expression covers literal containers
        // (list, dict, set, tuple) and simple constants so that gen-exprs
        // referencing the global (e.g. `d.items()` inside `sum(...)`) can
        // dispatch the right method — otherwise the global defaults to `Int`
        // and `MethodCall` on dict lowers to `None` (§G.11).
        for stmt_id in &init_func.body {
            let stmt = &hir_module.stmts[*stmt_id];
            let (target, type_hint, value) = match &stmt.kind {
                hir::StmtKind::Bind {
                    target: hir::BindingTarget::Var(target_var),
                    type_hint,
                    value,
                } => (*target_var, type_hint.as_ref(), *value),
                _ => continue,
            };

            if !self.symbols.globals.contains(&target) {
                continue;
            }

            // Explicit annotation wins.
            if let Some(var_type) = type_hint {
                self.symbols
                    .global_var_types
                    .insert(target, var_type.clone());
                continue;
            }

            // Fall back to RHS inference for **literal-shaped** globals only
            // (container literals and primitive constants). Skipping
            // `Call` / `Var` / `MethodCall` etc. is intentional — those
            // lowering paths have their own typing machinery (module-var
            // wrappers, closure tracking, nonlocal cells) and pre-recording
            // a potentially wrong type here has caused regressions in
            // escaping-closure globals. Container literals are safe because
            // their type is fully determined by the literal shape.
            let value_expr = &hir_module.exprs[value];
            let is_literal_shape = matches!(
                value_expr.kind,
                hir::ExprKind::Dict(_)
                    | hir::ExprKind::List(_)
                    | hir::ExprKind::Set(_)
                    | hir::ExprKind::Tuple(_)
                    | hir::ExprKind::Int(_)
                    | hir::ExprKind::Float(_)
                    | hir::ExprKind::Bool(_)
                    | hir::ExprKind::Str(_)
                    | hir::ExprKind::Bytes(_)
                    | hir::ExprKind::None
            );
            if is_literal_shape {
                // §1.4u step 1 (2026-04-18): the empty-overlay
                // wrapper `infer_expr_type` was deleted; callers with
                // no param overlay pass an empty map directly.
                let inferred =
                    self.infer_deep_expr_type(value_expr, hir_module, &indexmap::IndexMap::new());
                if !matches!(inferred, Type::Any) {
                    self.symbols.global_var_types.insert(target, inferred);
                }
            }
        }
    }

    /// Process module init for decorated functions (module-level wrappers).
    /// This must run before lowering any functions since other functions may call decorated functions.
    /// The decorator pattern produces: var = decorator(FuncRef(func))
    pub(crate) fn process_module_decorated_functions(&mut self, hir_module: &hir::Module) {
        // Find the module init function (name is __pyaot_module_init__)
        let init_func = hir_module
            .func_defs
            .values()
            .find(|f| self.interner.resolve(f.name) == "__pyaot_module_init__");

        let Some(init_func) = init_func else {
            return;
        };

        // Scan statements in module init for decorated function assignments
        for stmt_id in &init_func.body {
            let stmt = &hir_module.stmts[*stmt_id];
            let var_assign = match &stmt.kind {
                hir::StmtKind::Bind {
                    target: hir::BindingTarget::Var(target_var),
                    value,
                    ..
                } => Some((*target_var, *value)),
                _ => None,
            };
            if let Some((target, value)) = var_assign {
                let expr = &hir_module.exprs[value];

                // Check for decorated function pattern: Call { func: FuncRef(decorator), args: [FuncRef(original)] }
                match &expr.kind {
                    hir::ExprKind::FuncRef(func_id) => {
                        // Simple function reference: var = func
                        self.insert_module_var_func(target, *func_id);
                    }
                    hir::ExprKind::Closure { func, .. } => {
                        // Direct closure assignment: var = lambda
                        self.insert_module_var_func(target, *func);
                    }
                    hir::ExprKind::Call { func, args, .. } => {
                        // Check for decorator factory pattern first: Call { func: Call(...), args: [FuncRef] }
                        // This is @factory(arg) def f - the func is itself a call expression
                        let func_expr = &hir_module.exprs[*func];
                        if matches!(&func_expr.kind, hir::ExprKind::Call { .. }) {
                            // Decorator factory pattern - needs runtime evaluation because:
                            // 1. The factory must be called first with its arguments
                            // 2. The result (a closure/decorator) must then be applied to the function
                            // Mark as global for runtime evaluation
                            self.symbols.globals.insert(target);
                            // Register any wrapper functions that might be involved
                            self.register_all_wrappers_in_chain(expr, hir_module);
                            continue;
                        }

                        // Check for decorator pattern
                        if let Some(innermost_func_id) =
                            self.find_innermost_func_ref_static(expr, hir_module)
                        {
                            // Register ALL wrapper functions in the decorator chain
                            // This is needed for chained decorators like @triple @add_one
                            self.register_all_wrappers_in_chain(expr, hir_module);

                            // Check if this is a chained decorator (nested calls).
                            // For chained wrapper decorators (like @triple @add_one), we need to
                            // evaluate the full call chain at runtime because each wrapper captures
                            // the result of the previous decorator.
                            //
                            // For single wrapper decorators (like @decorator def f), we can use
                            // the wrapper shortcut which is more efficient.
                            //
                            // For chained identity decorators (like @identity1 @identity2), we can
                            // use the identity shortcut because they just return the original function.
                            let is_chained = args.iter().any(|arg| {
                                if let hir::CallArg::Regular(arg_id) = arg {
                                    matches!(
                                        hir_module.exprs[*arg_id].kind,
                                        hir::ExprKind::Call { .. }
                                    )
                                } else {
                                    false
                                }
                            });

                            // Only skip the shortcut if we have BOTH:
                            // 1. Nested calls (chained decorators)
                            // 2. At least one wrapper decorator in the chain
                            // Single wrapper decorators should use the wrapper shortcut.
                            if is_chained && self.chain_contains_wrapper_decorator(expr, hir_module)
                            {
                                // Mark the target as a global so that when it's called,
                                // we load from global storage and do an indirect call
                                self.symbols.globals.insert(target);
                                continue;
                            }

                            if let hir::ExprKind::FuncRef(decorator_func_id) = &func_expr.kind {
                                // Check if the decorator returns a closure (wrapper pattern)
                                if let Some(decorator_def) =
                                    hir_module.func_defs.get(decorator_func_id)
                                {
                                    if let Some(wrapper_func_id) =
                                        self.find_returned_closure(decorator_def, hir_module)
                                    {
                                        // Wrapper decorator: track wrapper with original function
                                        self.insert_module_var_wrapper(
                                            target,
                                            wrapper_func_id,
                                            innermost_func_id,
                                        );
                                        // Mark this function as a wrapper
                                        self.insert_wrapper_func_id(wrapper_func_id);
                                        continue;
                                    }
                                }
                            }
                            // Identity-like decorator: track the original function directly
                            self.insert_module_var_func(target, innermost_func_id);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Find the innermost FuncRef in a decorator call chain (static version for pre-scan).
    /// This follows the pattern: decorator(decorator2(... FuncRef(original) ...))
    fn find_innermost_func_ref_static(
        &self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
    ) -> Option<pyaot_utils::FuncId> {
        match &expr.kind {
            hir::ExprKind::FuncRef(func_id) => Some(*func_id),
            hir::ExprKind::Call { args, .. } => {
                // Look through all call arguments for FuncRef or nested calls
                for arg in args {
                    if let hir::CallArg::Regular(arg_expr_id) = arg {
                        let arg_expr = &hir_module.exprs[*arg_expr_id];
                        if let Some(func_id) =
                            self.find_innermost_func_ref_static(arg_expr, hir_module)
                        {
                            return Some(func_id);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Register all wrapper functions in a decorator call chain.
    /// For chained decorators like @triple @add_one, this walks through:
    /// triple(add_one(base)) and registers both triple's wrapper AND add_one's wrapper.
    /// This ensures that when wrapper functions call their captured func parameter,
    /// we recognize it as an indirect call.
    fn register_all_wrappers_in_chain(&mut self, expr: &hir::Expr, hir_module: &hir::Module) {
        self.register_all_wrappers_in_chain_inner(expr, hir_module, 0);
    }

    fn register_all_wrappers_in_chain_inner(
        &mut self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        depth: usize,
    ) {
        if depth > 64 {
            return;
        }

        if let hir::ExprKind::Call { func, args, .. } = &expr.kind {
            let func_expr = &hir_module.exprs[*func];

            // Handle decorator factory pattern: func is itself a Call
            // e.g., @multiply(3) def f -> multiply(3) returns a decorator
            if let hir::ExprKind::Call {
                func: factory_func, ..
            } = &func_expr.kind
            {
                // For decorator factory, find the actual wrapper which is nested two levels deep:
                // factory(args) returns decorator, decorator(func) returns wrapper
                let factory_func_expr = &hir_module.exprs[*factory_func];
                if let hir::ExprKind::FuncRef(factory_func_id) = &factory_func_expr.kind {
                    if let Some(factory_def) = hir_module.func_defs.get(factory_func_id) {
                        // Factory returns decorator closure
                        if let Some(decorator_func_id) =
                            self.find_returned_closure(factory_def, hir_module)
                        {
                            // Decorator returns wrapper closure
                            if let Some(decorator_def) =
                                hir_module.func_defs.get(&decorator_func_id)
                            {
                                if let Some(wrapper_func_id) =
                                    self.find_returned_closure(decorator_def, hir_module)
                                {
                                    // Register the actual wrapper function
                                    self.insert_wrapper_func_id(wrapper_func_id);
                                }
                            }
                        }
                    }
                }
                // Also recursively process in case of deeper nesting
                self.register_all_wrappers_in_chain_inner(func_expr, hir_module, depth + 1);
            }

            // Check if the function being called is a decorator (direct decorator, not factory)
            if let hir::ExprKind::FuncRef(decorator_func_id) = &func_expr.kind {
                // Check if this decorator returns a closure (wrapper pattern)
                if let Some(decorator_def) = hir_module.func_defs.get(decorator_func_id) {
                    if let Some(wrapper_func_id) =
                        self.find_returned_closure(decorator_def, hir_module)
                    {
                        // Register this wrapper function
                        self.insert_wrapper_func_id(wrapper_func_id);
                    }
                }
            }

            // Recursively process arguments to handle chained decorators
            for arg in args {
                if let hir::CallArg::Regular(arg_expr_id) = arg {
                    let arg_expr = &hir_module.exprs[*arg_expr_id];
                    self.register_all_wrappers_in_chain_inner(arg_expr, hir_module, depth + 1);
                }
            }
        }
    }

    /// Check if a decorator chain contains at least one wrapper decorator.
    /// Wrapper decorators return closures that capture the original function.
    /// Identity decorators just return the original function unchanged.
    pub(crate) fn chain_contains_wrapper_decorator(
        &self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
    ) -> bool {
        match &expr.kind {
            hir::ExprKind::Call { func, args, .. } => {
                let func_expr = &hir_module.exprs[*func];

                // Check if func is itself a Call (decorator factory pattern)
                // Decorator factories typically return wrapper closures
                if matches!(&func_expr.kind, hir::ExprKind::Call { .. }) {
                    // Recursively check the inner call
                    if self.chain_contains_wrapper_decorator(func_expr, hir_module) {
                        return true;
                    }
                    // For factory pattern, we conservatively assume it returns a wrapper
                    // since most decorator factories do (they capture the factory args)
                    return true;
                }

                // Check if the function being called is a wrapper decorator
                if let hir::ExprKind::FuncRef(decorator_func_id) = &func_expr.kind {
                    if let Some(decorator_def) = hir_module.func_defs.get(decorator_func_id) {
                        // Check if this decorator returns a closure (wrapper pattern)
                        if self
                            .find_returned_closure(decorator_def, hir_module)
                            .is_some()
                        {
                            return true; // Found a wrapper decorator
                        }
                    }
                }

                // Recursively check arguments for nested decorator calls
                for arg in args {
                    if let hir::CallArg::Regular(arg_expr_id) = arg {
                        let arg_expr = &hir_module.exprs[*arg_expr_id];
                        if self.chain_contains_wrapper_decorator(arg_expr, hir_module) {
                            return true;
                        }
                    }
                }
                false
            }
            _ => false,
        }
    }
}
