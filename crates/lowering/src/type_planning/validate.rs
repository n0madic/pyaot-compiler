//! Type annotation validation pass
//!
//! Validates type annotations against inferred types at declaration sites:
//! - Default parameter values vs parameter type annotations
//! - Class attribute initializers vs declared types
//!
//! This pass runs after return type inference (so func_return_types is populated)
//! but before codegen. It catches mismatches that inline validation misses
//! because those sites are only checked when the code is actually lowered.

use pyaot_hir as hir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Validate type annotations across the entire module.
    /// Called from `run_type_planning()` after return type inference.
    pub(crate) fn validate_type_annotations(&mut self, hir_module: &hir::Module) {
        self.validate_default_param_types(hir_module);
        self.validate_class_attr_types(hir_module);
    }

    /// Validate that default parameter values match their declared parameter types.
    /// E.g., `def f(x: int = "hello")` → type warning.
    fn validate_default_param_types(&mut self, hir_module: &hir::Module) {
        for func_id in &hir_module.functions {
            let Some(func) = hir_module.func_defs.get(func_id) else {
                continue;
            };

            for param in &func.params {
                // Only check params that have BOTH a type annotation AND a default value
                let (Some(ref param_ty), Some(default_id)) = (&param.ty, param.default) else {
                    continue;
                };

                // Skip Any — gradual typing
                if *param_ty == Type::Any {
                    continue;
                }

                let inferred = self.get_type_of_expr_id(default_id, hir_module);

                // Skip if inferred is Any (insufficient info)
                if inferred == Type::Any {
                    continue;
                }

                // Python special cases: int/float promotion, bool/int promotion
                let is_python_compatible = matches!(
                    (&inferred, param_ty),
                    (Type::Int, Type::Float) | (Type::Bool, Type::Int) | (Type::Bool, Type::Float)
                );

                if !is_python_compatible && !inferred.is_subtype_of(param_ty) {
                    let expr = &hir_module.exprs[default_id];
                    self.warnings
                        .add(pyaot_diagnostics::CompilerWarning::TypeError {
                            span: expr.span,
                            message: format!(
                            "default value type '{}' is not compatible with parameter type '{}'",
                            inferred, param_ty
                        ),
                        });
                }
            }
        }
    }

    /// Validate that class attribute initializers match their declared types.
    /// E.g., `x: int = "hello"` in a class body → type warning.
    fn validate_class_attr_types(&mut self, hir_module: &hir::Module) {
        for class_def in hir_module.class_defs.values() {
            for attr in &class_def.class_attrs {
                // Skip Any — gradual typing
                if attr.ty == Type::Any {
                    continue;
                }

                let inferred = self.get_type_of_expr_id(attr.initializer, hir_module);

                // Skip if inferred is Any (insufficient info)
                if inferred == Type::Any {
                    continue;
                }

                // Python special cases: int/float promotion, bool/int promotion
                let is_python_compatible = matches!(
                    (&inferred, &attr.ty),
                    (Type::Int, Type::Float) | (Type::Bool, Type::Int) | (Type::Bool, Type::Float)
                );

                if !is_python_compatible && !inferred.is_subtype_of(&attr.ty) {
                    let expr = &hir_module.exprs[attr.initializer];
                    self.warnings
                        .add(pyaot_diagnostics::CompilerWarning::TypeError {
                            span: expr.span,
                            message: format!(
                                "class attribute '{}' initializer type '{}' is not compatible with declared type '{}'",
                                self.resolve(attr.name),
                                inferred,
                                attr.ty
                            ),
                        });
                }
            }
        }
    }
}
