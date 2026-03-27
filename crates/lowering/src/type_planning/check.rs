//! Check mode: top-down type validation + error reporting
//!
//! Validates expression types against expected types (from annotations, parameters,
//! return types) and reports mismatches as CompilerWarning::TypeError.
//!
//! Check mode is called from:
//! - `assign.rs`: validate RHS against type hint
//! - `control_flow.rs`: validate return value against return type
//! - `calls.rs`: validate arguments against parameter types
//!
//! Eventually, type_inference.rs and lambda_inference.rs will be merged here.

use pyaot_hir as hir;
use pyaot_types::Type;
use smallvec::SmallVec;

use crate::context::Lowering;

// =============================================================================
// Check mode: top-down type propagation + error reporting
// =============================================================================

impl<'a> Lowering<'a> {
    /// Bidirectional check: validate expression against expected type.
    /// Reports a type warning if types are incompatible.
    /// Called from assignment (type hint), return (return type), call args (param types).
    pub(crate) fn check_expr_type(
        &mut self,
        expr_id: hir::ExprId,
        expected: &Type,
        hir_module: &hir::Module,
    ) {
        // Skip check for Any (gradual typing: Any is compatible with everything)
        if *expected == Type::Any {
            return;
        }

        let expr = &hir_module.exprs[expr_id];

        // Empty containers always accept expected type — handled by lower_list/dict/set
        match &expr.kind {
            hir::ExprKind::List(elems) if elems.is_empty() => return,
            hir::ExprKind::Dict(pairs) if pairs.is_empty() => return,
            hir::ExprKind::Set(elems) if elems.is_empty() => return,
            // Builtin constructors with no args (list(), dict(), set())
            hir::ExprKind::BuiltinCall { args, .. } if args.is_empty() => return,
            _ => {}
        }

        // Infer the actual type
        let inferred = self.get_type_of_expr_id(expr_id, hir_module);

        // Skip if inferred is Any (insufficient type info — not an error)
        if inferred == Type::Any {
            return;
        }

        // Python special cases: int is compatible with float (implicit promotion)
        let is_python_compatible = matches!(
            (&inferred, expected),
            (Type::Int, Type::Float) | (Type::Bool, Type::Int) | (Type::Bool, Type::Float)
        );

        // Protocol classes accept any type (structural subtyping)
        if let Type::Class { class_id, .. } = expected {
            if let Some(class_def) = hir_module.class_defs.get(class_id) {
                if class_def.is_protocol {
                    return;
                }
            }
        }

        // Check compatibility: inferred must be a subtype of expected
        if !is_python_compatible && !inferred.is_subtype_of(expected) {
            self.warnings
                .add(pyaot_diagnostics::CompilerWarning::TypeError {
                    span: expr.span,
                    message: format!(
                        "type '{}' is not compatible with expected type '{}'",
                        inferred, expected
                    ),
                });
        }
    }

    /// Check function call: validate arg count and arg types against parameters.
    /// `call_span` is the source location of the call expression (e.g. `f(1)`).
    pub(crate) fn check_call_args(
        &mut self,
        func_id: &pyaot_utils::FuncId,
        arg_expr_ids: &[hir::ExprId],
        kwargs: &[hir::KeywordArg],
        call_span: pyaot_utils::Span,
        hir_module: &hir::Module,
    ) {
        let Some(func_def) = hir_module.func_defs.get(func_id) else {
            return;
        };

        // Collect regular params for matching
        let regular_params: Vec<&hir::Param> = func_def
            .params
            .iter()
            .filter(|p| matches!(p.kind, hir::ParamKind::Regular))
            .collect();

        // Count required params (no default, regular kind)
        let required_count = regular_params
            .iter()
            .filter(|p| p.default.is_none())
            .count();

        // Count how many required params are satisfied by keyword arguments
        let kwargs_filling_required = kwargs
            .iter()
            .filter(|kw| {
                let kw_name = self.resolve(kw.name);
                regular_params
                    .iter()
                    .any(|p| p.default.is_none() && self.resolve(p.name) == kw_name)
            })
            .count();

        let effective_count = arg_expr_ids.len() + kwargs_filling_required;
        if effective_count < required_count {
            // Find the first missing parameter (SmallVec avoids heap for typical ≤8 params)
            let positional_names: SmallVec<[String; 8]> = (0..arg_expr_ids.len())
                .filter_map(|i| {
                    regular_params
                        .get(i)
                        .map(|p| self.resolve(p.name).to_string())
                })
                .collect();
            let kwarg_names: SmallVec<[String; 8]> = kwargs
                .iter()
                .map(|kw| self.resolve(kw.name).to_string())
                .collect();
            for param in &regular_params {
                if param.default.is_none() {
                    let name = self.resolve(param.name).to_string();
                    if !positional_names.contains(&name) && !kwarg_names.contains(&name) {
                        self.warnings
                            .add(pyaot_diagnostics::CompilerWarning::TypeError {
                                span: call_span,
                                message: format!("missing required argument: '{}'", name),
                            });
                        break;
                    }
                }
            }
        }

        // Check for excess positional arguments
        let has_var_positional = func_def
            .params
            .iter()
            .any(|p| matches!(p.kind, hir::ParamKind::VarPositional));
        if !has_var_positional && arg_expr_ids.len() > regular_params.len() {
            self.warnings
                .add(pyaot_diagnostics::CompilerWarning::TypeError {
                    span: call_span,
                    message: format!(
                        "too many positional arguments: expected at most {}, got {}",
                        regular_params.len(),
                        arg_expr_ids.len()
                    ),
                });
        }

        // Check each positional arg type against param type.
        // Use regular_params (filtered to Regular kind) to avoid indexing into
        // *args/**kwargs entries in the raw param list.
        for (i, arg_id) in arg_expr_ids.iter().enumerate() {
            if let Some(param) = regular_params.get(i) {
                if let Some(ref param_ty) = param.ty {
                    self.check_expr_type(*arg_id, param_ty, hir_module);
                }
            }
        }

        // Check each kwarg type against its matching param type
        for kw in kwargs {
            let kw_name = self.resolve(kw.name);
            if let Some(param) = regular_params
                .iter()
                .find(|p| self.resolve(p.name) == kw_name)
            {
                if let Some(ref param_ty) = param.ty {
                    self.check_expr_type(kw.value, param_ty, hir_module);
                }
            }
        }
    }
}
