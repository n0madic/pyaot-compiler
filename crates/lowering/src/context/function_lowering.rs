//! Module and function lowering entry points

use pyaot_diagnostics::{CompilerWarnings, Result};
use pyaot_hir as hir;
use pyaot_mir::{self as mir, ValueKind};
use pyaot_types::Type;

use crate::utils::is_heap_type;

use super::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a complete HIR module to MIR.
    /// Returns the MIR module and any warnings collected during lowering.
    pub fn lower_module(
        mut self,
        hir_module: &hir::Module,
    ) -> Result<(mir::Module, CompilerWarnings)> {
        // Copy global variables set from HIR module
        self.globals = hir_module.globals.clone();

        // Pre-populate global variable types from module init function
        // This must happen before lowering any functions since they may reference globals
        self.scan_global_var_types(hir_module);

        // First pass: build class info
        self.build_class_info(hir_module);

        // Second pass: build function name map
        for func_id in &hir_module.functions {
            if let Some(func) = hir_module.func_defs.get(func_id) {
                let func_name = self.interner.resolve(func.name).to_string();
                self.func_name_map.insert(func_name, *func_id);
            }
        }

        // Pass 2.5: scan for mutable default parameters and allocate global slots
        // In Python, mutable defaults (list, dict, set, class instances) are evaluated
        // once at function definition time and shared across all calls.
        self.scan_mutable_defaults(hir_module);

        // Third pass: pre-compute closure capture types
        self.precompute_closure_capture_types(hir_module);

        // Pass 3.5: scan for decorated functions in module init
        // This must happen before lowering any functions since other functions may call decorated functions
        self.scan_module_decorated_functions(hir_module);

        // Fourth pass: lower functions
        for func_id in &hir_module.functions {
            if let Some(func) = hir_module.func_defs.get(func_id) {
                if func.is_generator {
                    // Generator functions create two functions: creator and resume
                    let (creator_func, resume_func) =
                        self.lower_generator_function(func, hir_module)?;
                    self.mir_module.add_function(creator_func);
                    self.mir_module.add_function(resume_func);
                } else {
                    let mir_func = self.lower_function(func, hir_module)?;
                    self.mir_module.add_function(mir_func);
                }
            }
        }

        // Fifth pass: export vtable information
        self.build_vtables();

        // Return both MIR module and collected warnings
        Ok((self.mir_module, self.warnings))
    }

    /// Lower a single HIR function to MIR
    pub(crate) fn lower_function(
        &mut self,
        func: &hir::Function,
        hir_module: &hir::Module,
    ) -> Result<mir::Function> {
        self.var_to_local.clear();
        self.var_types.clear();
        self.var_to_func.clear();
        self.var_to_closure.clear();
        self.var_to_wrapper.clear();
        self.func_ptr_params.clear();
        self.current_blocks.clear();
        self.current_block_idx = 0;
        self.next_block_id = 0;
        self.cell_vars.clear();
        self.nonlocal_cells.clear();
        self.narrowed_union_vars.clear();
        self.loop_stack.clear();
        self.expr_type_cache.borrow_mut().clear();

        // Copy cell_vars and nonlocal_vars from HIR function
        for var_id in &func.cell_vars {
            self.cell_vars.insert(*var_id);
        }
        for var_id in &func.nonlocal_vars {
            // nonlocal_vars will be mapped to cell locals during parameter processing
            self.cell_vars.insert(*var_id);
        }

        // Check if this function is a wrapper (closure returned by a decorator)
        // If so, mark the `func` parameter as a function pointer for indirect calls
        let is_wrapper_func = self.is_wrapper_func(&func.id);
        if is_wrapper_func && !func.params.is_empty() {
            // Find the parameter that holds the decorated function.
            // This is typically named "func" or "__capture_func" (when captured from outer scope).
            // In simple decorators, it's the first parameter.
            // In decorator factories with multiple captures, it may be at a different position.
            for param in &func.params {
                let param_name = self.interner.resolve(param.name);
                // Check for both direct parameter "func" and captured parameter "__capture_func"
                if param_name == "func" || param_name == "__capture_func" {
                    self.insert_func_ptr_param(param.var);
                    break;
                }
            }
        }

        let func_name = self.interner.resolve(func.name).to_string();
        let is_lambda = func_name.starts_with("__lambda_") || func_name.starts_with("__nested_");
        let is_module_init = func_name == "__pyaot_module_init__";

        // For lambdas, try to infer parameter types from the body
        let inferred_param_types = if is_lambda && !func.body.is_empty() {
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

            // For nonlocal parameters, the type is a cell pointer (use Str as placeholder)
            let param_ty = if is_cell_param { Type::Str } else { base_ty };

            // Register parameter variable
            self.insert_var_local(hir_param.var, local_id);
            // Track parameter type for type inference (use the underlying value type, not cell type)
            // This is needed so that reading from the cell returns the correct type
            if is_cell_param {
                // Get the underlying value type from inferred types or default to Int
                let value_ty = if i < inferred_param_types.len() {
                    inferred_param_types[i].clone()
                } else {
                    Type::Int
                };
                self.insert_var_type(hir_param.var, value_ty);
            } else {
                self.insert_var_type(hir_param.var, param_ty.clone());
            }

            let mir_param = mir::Local {
                id: local_id,
                name: None, // Name is in var_to_local mapping
                ty: param_ty.clone(),
                is_gc_root: is_cell_param || is_heap_type(&param_ty), // Cells are heap objects
            };
            params.push(mir_param);
        }

        // Infer return type for functions without explicit return type annotation
        // The frontend sets return_type to Some(Type::None) as the default when no annotation
        // is provided, so we need to infer the actual type from the function body.
        // Only infer when there's no explicit annotation (None or Some(Type::None))
        let has_explicit_return_type =
            func.return_type.is_some() && func.return_type.as_ref() != Some(&Type::None);
        let needs_return_type_inference = !func.body.is_empty() && !has_explicit_return_type;
        let return_type = if needs_return_type_inference {
            let inferred = self.infer_lambda_return_type(func, hir_module);
            // If inference returns None (no return statement found), check for closure return
            if inferred == Type::None {
                if self.find_returned_closure(func, hir_module).is_some() {
                    Type::Any
                } else {
                    Type::None
                }
            } else {
                inferred
            }
        } else {
            // Use the declared return type
            func.return_type.clone().unwrap_or(Type::None)
        };

        // Store the inferred return type for later lookup
        self.insert_func_return_type(func.id, return_type.clone());

        // Store the current function's return type for type inference during lowering
        self.current_func_return_type = Some(return_type.clone());

        let mut mir_func = mir::Function::new(func.id, func_name, params.clone(), return_type);

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

            // Create a local to hold the cell pointer
            let cell_local = self.alloc_local_id();
            mir_func.add_local(mir::Local {
                id: cell_local,
                name: None,
                ty: Type::Str,    // Placeholder type for cell pointer
                is_gc_root: true, // Cells are heap objects
            });

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

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: cell_local,
                func: make_cell_func,
                args: vec![initial_value],
            });

            // Map this variable to its cell for later reads/writes
            self.nonlocal_cells.insert(var_id, cell_local);
        }

        // For nonlocal_vars, the parameter is already a cell pointer
        // Map them to nonlocal_cells for later reads/writes
        for &var_id in &func.nonlocal_vars {
            if let Some(local_id) = self.get_var_local(&var_id) {
                self.nonlocal_cells.insert(var_id, local_id);
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
                    let str_local = self.alloc_and_add_local(Type::Str, &mut mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: str_local,
                        func: mir::RuntimeFunc::MakeStr,
                        args: vec![mir::Operand::Constant(mir::Constant::Str(empty_str))],
                    });
                    mir::Operand::Local(str_local)
                }
                _ => mir::Operand::Constant(mir::Constant::None),
            };
            self.current_block_mut().terminator = mir::Terminator::Return(Some(default_return));
        }

        for block in self.current_blocks.drain(..) {
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
                            let slot = self.next_default_slot;
                            self.next_default_slot += 1;
                            self.default_value_slots.insert((*func_id, param_idx), slot);
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
            .default_value_slots
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();

        for ((func_id, param_idx), slot) in slots_to_init {
            // Get the function and parameter
            let Some(func) = hir_module.func_defs.get(&func_id) else {
                continue;
            };
            let param = &func.params[param_idx];

            if let Some(default_id) = param.default {
                let default_expr = &hir_module.exprs[default_id];

                // Set expected type from parameter type for correct elem_tag on empty lists
                let prev_expected = self.expected_type.take();
                self.expected_type = param.ty.clone();

                // Lower the default expression to get its value
                let default_operand = self.lower_expr(default_expr, hir_module, mir_func)?;

                self.expected_type = prev_expected;

                // Store in global slot - mutable defaults are always heap types (ptr)
                let dummy_local = self.alloc_and_add_local(Type::None, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::GlobalSet(ValueKind::Ptr),
                    args: vec![
                        mir::Operand::Constant(mir::Constant::Int(slot as i64)),
                        default_operand,
                    ],
                });
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

        // Scan statements in module init for assignments to global variables
        for stmt_id in &init_func.body {
            let stmt = &hir_module.stmts[*stmt_id];
            if let hir::StmtKind::Assign {
                target, type_hint, ..
            } = &stmt.kind
            {
                // Only process if target is a global variable with an explicit type hint.
                // Variables without type hints will get their types from lowering.
                if self.globals.contains(target) {
                    if let Some(var_type) = type_hint {
                        // Store in global_var_types for persistence across function boundaries
                        self.global_var_types.insert(*target, var_type.clone());
                    }
                }
            }
        }
    }

    /// Pre-scan module init for decorated functions (module-level wrappers).
    /// This must run before lowering any functions since other functions may call decorated functions.
    /// The decorator pattern produces: var = decorator(FuncRef(func))
    fn scan_module_decorated_functions(&mut self, hir_module: &hir::Module) {
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
            if let hir::StmtKind::Assign { target, value, .. } = &stmt.kind {
                let expr = &hir_module.exprs[*value];

                // Check for decorated function pattern: Call { func: FuncRef(decorator), args: [FuncRef(original)] }
                match &expr.kind {
                    hir::ExprKind::FuncRef(func_id) => {
                        // Simple function reference: var = func
                        self.insert_module_var_func(*target, *func_id);
                    }
                    hir::ExprKind::Closure { func, .. } => {
                        // Direct closure assignment: var = lambda
                        self.insert_module_var_func(*target, *func);
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
                            self.globals.insert(*target);
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
                                self.globals.insert(*target);
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
                                            *target,
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
                            self.insert_module_var_func(*target, innermost_func_id);
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
                self.register_all_wrappers_in_chain(func_expr, hir_module);
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
                    self.register_all_wrappers_in_chain(arg_expr, hir_module);
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
