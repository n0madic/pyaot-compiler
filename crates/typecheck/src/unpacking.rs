//! Nested unpacking validation

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{ExprId, Module, UnpackTarget};
use pyaot_types::Type;

use crate::context::TypeChecker;

impl<'a> TypeChecker<'a> {
    /// Check nested unpacking types (e.g., `(a, (b, c)) = value`)
    pub(crate) fn check_nested_unpack_types(
        &mut self,
        targets: &[UnpackTarget],
        source_type: &Type,
        value_expr_id: ExprId,
        module: &Module,
    ) -> Result<()> {
        // Get element types from source
        let elem_types: Vec<Type> = match source_type {
            Type::Tuple(types) => types.clone(),
            Type::List(inner) => vec![(**inner).clone(); targets.len()],
            Type::Any => vec![Type::Any; targets.len()],
            _ => {
                let expr_span = module.exprs[value_expr_id].span;
                return Err(CompilerError::type_error(
                    format!("cannot unpack value of type '{}'", source_type),
                    expr_span,
                ));
            }
        };

        // Check that we have enough elements
        if let Type::Tuple(types) = source_type {
            if types.len() != targets.len() {
                let expr_span = module.exprs[value_expr_id].span;
                return Err(CompilerError::type_error(
                    format!(
                        "cannot unpack tuple of {} elements into {} targets",
                        types.len(),
                        targets.len()
                    ),
                    expr_span,
                ));
            }
        }

        // Recursively check each target
        for (i, target) in targets.iter().enumerate() {
            let elem_type = elem_types.get(i).cloned().unwrap_or(Type::Any);
            self.check_nested_unpack_target(target, &elem_type, value_expr_id, module)?;
        }

        Ok(())
    }

    /// Check a single nested unpacking target
    fn check_nested_unpack_target(
        &mut self,
        target: &UnpackTarget,
        target_type: &Type,
        value_expr_id: ExprId,
        module: &Module,
    ) -> Result<()> {
        match target {
            UnpackTarget::Var(var_id) => {
                // Simple variable - assign the type
                self.var_types.insert(*var_id, target_type.clone());
            }
            UnpackTarget::Nested(nested_targets) => {
                // Recursively check nested pattern
                self.check_nested_unpack_types(nested_targets, target_type, value_expr_id, module)?;
            }
        }
        Ok(())
    }
}
