//! Function call type inference and validation

use indexmap::IndexMap;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{CallArg, ExprId, ExprKind, Module, Param};
use pyaot_types::Type;
use pyaot_utils::Span;

use crate::context::TypeChecker;

impl<'a> TypeChecker<'a> {
    /// Infer the return type of a function call
    pub(crate) fn infer_call_type_with_args(
        &mut self,
        func_expr: ExprId,
        args: &[CallArg],
        kwargs: &[pyaot_hir::KeywordArg],
        has_runtime_kwargs: bool,
        module: &Module,
        call_span: Span,
    ) -> Type {
        let func_kind = &module.exprs[func_expr].kind;

        match func_kind {
            ExprKind::FuncRef(func_id) => {
                if let Some(func) = module.func_defs.get(func_id) {
                    // Check argument count and types
                    if let Err(e) = self.check_call_args_with_starred(
                        &func.params,
                        args,
                        kwargs,
                        has_runtime_kwargs,
                        module,
                        call_span,
                    ) {
                        self.add_error(e);
                    }
                    // Functions without explicit return type annotation are untyped (Any),
                    // not implicitly returning None. This matches Python semantics where
                    // unannotated functions can return any type.
                    func.return_type.clone().unwrap_or(Type::Any)
                } else {
                    Type::Any
                }
            }
            ExprKind::ClassRef(class_id) => {
                // Class instantiation - check __init__ args
                if let Some(class_def) = module.class_defs.get(class_id) {
                    if let Some(init_id) = class_def.init_method {
                        if let Some(init_func) = module.func_defs.get(&init_id) {
                            // Skip 'self' parameter
                            let params: Vec<_> = init_func.params.iter().skip(1).cloned().collect();
                            if let Err(e) = self.check_call_args_with_starred(
                                &params,
                                args,
                                kwargs,
                                has_runtime_kwargs,
                                module,
                                call_span,
                            ) {
                                self.add_error(e);
                            }
                        }
                    }
                    if let Some(class_info) = self.class_info.get(class_id) {
                        Type::Class {
                            class_id: *class_id,
                            name: class_info.name,
                        }
                    } else {
                        Type::Any
                    }
                } else {
                    Type::Any
                }
            }
            _ => Type::Any,
        }
    }

    /// Check function call arguments with starred argument support
    pub(crate) fn check_call_args_with_starred(
        &mut self,
        params: &[Param],
        args: &[CallArg],
        kwargs: &[pyaot_hir::KeywordArg],
        has_runtime_kwargs: bool,
        module: &Module,
        call_span: Span,
    ) -> Result<()> {
        use pyaot_hir::ParamKind;

        // Split params by kind
        let regular_params: Vec<_> = params
            .iter()
            .filter(|p| matches!(p.kind, ParamKind::Regular))
            .collect();
        let has_varargs = params
            .iter()
            .any(|p| matches!(p.kind, ParamKind::VarPositional));
        let kwonly_params: Vec<_> = params
            .iter()
            .filter(|p| matches!(p.kind, ParamKind::KeywordOnly))
            .collect();
        let has_kwargs = params
            .iter()
            .any(|p| matches!(p.kind, ParamKind::VarKeyword));

        // Expand arguments and track starred positions
        let mut effective_arg_count = 0;
        let mut regular_args = Vec::new(); // (expr_id, is_from_unpack)
        let mut starred_unpack_info = Vec::new(); // (position_in_regular_args, expr_id, elem_types)

        for arg in args {
            match arg {
                CallArg::Regular(expr_id) => {
                    effective_arg_count += 1;
                    regular_args.push((*expr_id, false));
                }
                CallArg::Starred(expr_id) => {
                    let arg_expr = &module.exprs[*expr_id];
                    match &arg_expr.kind {
                        // Compile-time expansion (literals)
                        ExprKind::Tuple(elems) | ExprKind::List(elems) => {
                            effective_arg_count += elems.len();
                            for elem in elems {
                                regular_args.push((*elem, false));
                            }
                        }
                        // Runtime unpacking (variables)
                        _ => {
                            let arg_type = self.infer_expr_type(*expr_id, module);
                            match &arg_type {
                                Type::Tuple(elem_types) => {
                                    // ALWAYS unpack tuples - matches lowering behavior
                                    effective_arg_count += elem_types.len();
                                    starred_unpack_info.push((
                                        regular_args.len(),
                                        *expr_id,
                                        elem_types.clone(),
                                    ));
                                    // Add placeholder entries for each unpacked element
                                    for _ in elem_types {
                                        regular_args.push((*expr_id, true));
                                    }
                                }
                                Type::List(elem_type) => {
                                    // For list unpacking, we don't know the element count at compile time.
                                    // We'll assume it provides the correct number of elements for the
                                    // remaining parameters. The actual count will be validated at runtime.
                                    // Calculate how many parameters are left to fill
                                    let remaining_params =
                                        regular_params.len().saturating_sub(effective_arg_count);
                                    effective_arg_count += remaining_params;

                                    // Add placeholder entries for type checking
                                    // We create a virtual tuple type for the unpacked elements
                                    let elem_types =
                                        vec![elem_type.as_ref().clone(); remaining_params];
                                    starred_unpack_info.push((
                                        regular_args.len(),
                                        *expr_id,
                                        elem_types.clone(),
                                    ));
                                    for _ in 0..remaining_params {
                                        regular_args.push((*expr_id, true));
                                    }
                                }
                                _ => {
                                    return Err(CompilerError::type_error(
                                        format!(
                                            "cannot unpack type '{}' in function call",
                                            arg_type
                                        ),
                                        arg_expr.span,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Check that all required regular params are satisfied
        // Skip this check when runtime **kwargs is present since it might provide the values
        if !has_runtime_kwargs {
            for (i, param) in regular_params.iter().enumerate() {
                if param.default.is_none() {
                    let provided_positionally = i < effective_arg_count;
                    let param_name = self.interner.resolve(param.name);
                    let provided_by_keyword = kwargs.iter().any(|kw| {
                        let kw_name = self.interner.resolve(kw.name);
                        kw_name == param_name
                    });

                    if !provided_positionally && !provided_by_keyword {
                        return Err(CompilerError::type_error(
                            format!("missing required argument '{}'", param_name),
                            call_span,
                        ));
                    }
                }
            }
        }

        // Check for too many positional arguments (only if no *args)
        if !has_varargs && effective_arg_count > regular_params.len() {
            return Err(CompilerError::type_error(
                format!(
                    "takes {} positional argument(s) but {} were given",
                    regular_params.len(),
                    effective_arg_count
                ),
                call_span,
            ));
        }

        // Check that required keyword-only arguments are provided
        // Skip this check when runtime **kwargs is present
        if !has_runtime_kwargs {
            for kwonly in &kwonly_params {
                if kwonly.default.is_none() {
                    let kwonly_name = self.interner.resolve(kwonly.name);
                    if !kwargs.iter().any(|kw| {
                        let kw_name = self.interner.resolve(kw.name);
                        kw_name == kwonly_name
                    }) {
                        return Err(CompilerError::type_error(
                            format!("missing required keyword-only argument '{}'", kwonly_name),
                            call_span,
                        ));
                    }
                }
            }
        }

        // Check positional argument types
        for (arg_idx, (arg_expr_id, is_from_unpack)) in regular_args.iter().enumerate() {
            if arg_idx >= regular_params.len() {
                // Extra positional args go to *args if present
                break;
            }

            let param = regular_params[arg_idx];
            if let Some(param_type) = &param.ty {
                // Find the type to check
                let arg_type = if *is_from_unpack {
                    // This is from a starred unpack - find which one and get the element type
                    let mut elem_type = Type::Any;
                    for (unpack_start, _unpack_expr_id, elem_types) in &starred_unpack_info {
                        if arg_idx >= *unpack_start && arg_idx < unpack_start + elem_types.len() {
                            let elem_idx = arg_idx - unpack_start;
                            elem_type = elem_types[elem_idx].clone();
                            break;
                        }
                    }
                    elem_type
                } else {
                    self.infer_expr_type(*arg_expr_id, module)
                };

                if !arg_type.is_subtype_of(param_type) && arg_type != Type::Any {
                    let arg_span = module.exprs[*arg_expr_id].span;
                    let param_name = self.interner.resolve(param.name);

                    let error_msg = if *is_from_unpack {
                        format!(
                            "unpacked argument '{}' has type '{}', expected '{}'",
                            param_name, arg_type, param_type
                        )
                    } else {
                        format!(
                            "argument '{}' has type '{}', expected '{}'",
                            param_name, arg_type, param_type
                        )
                    };

                    return Err(CompilerError::type_error(error_msg, arg_span));
                }
            }
        }

        // Check keyword argument types (can match regular or keyword-only params)
        // Build param lookup map once for O(1) access instead of O(params) per kwarg
        let param_map: IndexMap<pyaot_utils::InternedString, &Param> =
            params.iter().map(|p| (p.name, p)).collect();

        for kwarg in kwargs {
            if let Some(param) = param_map.get(&kwarg.name) {
                let arg_type = self.infer_expr_type(kwarg.value, module);
                if let Some(param_type) = &param.ty {
                    if !arg_type.is_subtype_of(param_type) && arg_type != Type::Any {
                        let kwarg_name = self.interner.resolve(kwarg.name);
                        return Err(CompilerError::type_error(
                            format!(
                                "argument '{}' has type '{}', expected '{}'",
                                kwarg_name, arg_type, param_type
                            ),
                            kwarg.span,
                        ));
                    }
                }
            } else if !has_kwargs {
                // Only error if there's no **kwargs to collect it
                let kwarg_name = self.interner.resolve(kwarg.name);
                return Err(CompilerError::type_error(
                    format!("unexpected keyword argument '{}'", kwarg_name),
                    kwarg.span,
                ));
            }
        }

        Ok(())
    }
}
