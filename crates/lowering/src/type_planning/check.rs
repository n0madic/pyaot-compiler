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

        // Check compatibility (bidirectional: either direction is fine)
        if !is_python_compatible
            && !inferred.is_subtype_of(expected)
            && !expected.is_subtype_of(&inferred)
        {
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
    pub(crate) fn check_call_args(
        &mut self,
        func_id: &pyaot_utils::FuncId,
        arg_expr_ids: &[hir::ExprId],
        hir_module: &hir::Module,
    ) {
        let Some(func_def) = hir_module.func_defs.get(func_id) else {
            return;
        };

        // Count required params (no default, regular kind)
        let required_count = func_def
            .params
            .iter()
            .filter(|p| {
                p.default.is_none() && matches!(p.kind, hir::ParamKind::Regular)
            })
            .count();

        if arg_expr_ids.len() < required_count {
            if let Some(missing_param) = func_def.params.get(arg_expr_ids.len()) {
                let name = self.resolve(missing_param.name).to_string();
                self.warnings
                    .add(pyaot_diagnostics::CompilerWarning::TypeError {
                        span: pyaot_utils::Span::dummy(),
                        message: format!("missing required argument: '{}'", name),
                    });
            }
        }

        // Check each arg type against param type (skip *args/**kwargs params)
        for (i, arg_id) in arg_expr_ids.iter().enumerate() {
            if let Some(param) = func_def.params.get(i) {
                if matches!(
                    param.kind,
                    hir::ParamKind::VarPositional | hir::ParamKind::VarKeyword
                ) {
                    continue; // *args and **kwargs accept any types
                }
                if let Some(ref param_ty) = param.ty {
                    self.check_expr_type(*arg_id, param_ty, hir_module);
                }
            }
        }
    }
}
