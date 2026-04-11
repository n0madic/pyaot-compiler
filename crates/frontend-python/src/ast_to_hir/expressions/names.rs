//! Name resolution for variable, function, class, and import references.

use super::super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::*;
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    /// Resolve a Name expression to the appropriate HIR node.
    /// Checks (in order): stdlib names, local vars, functions, classes,
    /// imported names, stdlib/user modules, module vars, __name__, builtins.
    pub(crate) fn convert_name_expr(
        &mut self,
        name: py::ExprName,
        expr_span: Span,
    ) -> Result<ExprKind> {
        let name_str = self.interner.intern(&name.id);

        // First check if it's a stdlib name (from X import Y)
        if let Some(stdlib_item) = self.imports.stdlib_names.get(&name_str).cloned() {
            match stdlib_item {
                super::super::StdlibItem::Attr(attr) => return Ok(ExprKind::StdlibAttr(attr)),
                super::super::StdlibItem::Func(_) => {
                    // Function references need to be handled at call site
                    return Err(CompilerError::parse_error(
                        format!(
                            "Stdlib function '{}' must be called, cannot be used as value",
                            self.interner.resolve(name_str)
                        ),
                        expr_span,
                    ));
                }
                super::super::StdlibItem::Const(const_def) => {
                    return Ok(ExprKind::StdlibConst(const_def));
                }
            }
        }

        // Check if it's a variable
        if let Some(&var_id) = self.symbols.var_map.get(&name_str) {
            return Ok(ExprKind::Var(var_id));
        }

        // Then check if it's a function reference (for passing functions as values)
        if let Some(&func_id) = self.symbols.func_map.get(&name_str) {
            return Ok(ExprKind::FuncRef(func_id));
        }

        // Check if it's a class reference
        if let Some(&class_id) = self.symbols.class_map.get(&name_str) {
            return Ok(ExprKind::ClassRef(class_id));
        }

        // Check if it's an imported name
        if let Some(imported) = self.imports.imported_names.get(&name_str) {
            return Ok(match &imported.kind {
                super::super::ImportedNameKind::Function(func_id) => ExprKind::FuncRef(*func_id),
                super::super::ImportedNameKind::Class(class_id) => ExprKind::ClassRef(*class_id),
                super::super::ImportedNameKind::Variable(var_id) => ExprKind::Var(*var_id),
                super::super::ImportedNameKind::Unresolved => ExprKind::ImportedRef {
                    module: imported.module.clone(),
                    name: imported.original_name.clone(),
                },
            });
        }

        // Check if it's a stdlib module (import sys, etc.)
        if self.imports.stdlib_imports.contains(&name_str) {
            return Err(CompilerError::parse_error(
                format!(
                    "Module '{}' cannot be used as a value; use 'module.name' to access its members",
                    self.interner.resolve(name_str)
                ),
                expr_span,
            ));
        }

        // Check if it's an imported module (for module.attr access)
        if self.imports.imported_modules.contains_key(&name_str) {
            return Err(CompilerError::parse_error(
                format!(
                    "Module '{}' cannot be used as a value; use 'module.name' to access its members",
                    self.interner.resolve(name_str)
                ),
                expr_span,
            ));
        }

        // Check module-level variables (decorated functions, module-level assignments)
        if let Some(&var_id) = self.symbols.module_var_map.get(&name_str) {
            return Ok(ExprKind::Var(var_id));
        }

        // Handle __name__ built-in - always "__main__" for direct script execution
        if name.id.as_str() == "__name__" {
            return Ok(ExprKind::Str(self.interner.intern("__main__")));
        }

        // Check if it's a first-class builtin function (len, str, int, etc.)
        // This must come AFTER checking local variables to allow shadowing
        if let Some(builtin_kind) = BuiltinFunctionKind::from_name(&name.id) {
            return Ok(ExprKind::BuiltinRef(builtin_kind));
        }

        Err(CompilerError::name_error(name.id.clone(), expr_span))
    }
}
