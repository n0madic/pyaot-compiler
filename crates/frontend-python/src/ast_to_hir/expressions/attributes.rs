//! Attribute access: obj.attr, module.attr, class.attr, chained pkg.sub.attr.

use super::super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::*;
use pyaot_stdlib_defs::{self as stdlib, StdlibItem as RegistryItem};
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    /// Convert an Attribute expression (obj.attr).
    /// Handles stdlib module.attr, class.attr, user module.attr, chained access,
    /// and general field/attribute access.
    pub(crate) fn convert_attribute_expr(
        &mut self,
        attr: py::ExprAttribute,
        expr_span: Span,
    ) -> Result<ExprId> {
        // Check if this is stdlib module.attr, class.attr, or module.attr access
        if let py::Expr::Name(name) = &*attr.value {
            let name_str = self.interner.intern(&name.id);

            // Check if this is a class attribute access: ClassName.attr
            if let Some(&class_id) = self.symbols.class_map.get(&name_str) {
                let attr_name = self.interner.intern(&attr.attr);
                return Ok(self.module.exprs.alloc(Expr {
                    kind: ExprKind::ClassAttrRef {
                        class_id,
                        attr: attr_name,
                    },
                    ty: None,
                    span: expr_span,
                }));
            }

            // Handle stdlib module attribute access
            if self.imports.stdlib_imports.contains(&name_str) {
                let module_name = self.interner.resolve(name_str);
                let attr_name = attr.attr.as_str();

                // Handle os.path as a submodule
                if module_name == "os" && attr_name == "path" {
                    return Err(CompilerError::parse_error(
                        "os.path cannot be used as a value; use 'os.path.join()' etc.",
                        expr_span,
                    ));
                }

                // Use registry to determine what kind of item this is
                match stdlib::get_item(module_name, attr_name) {
                    Some(RegistryItem::Attr(attr_def)) => {
                        return Ok(self.module.exprs.alloc(Expr {
                            kind: ExprKind::StdlibAttr(attr_def),
                            ty: None,
                            span: expr_span,
                        }));
                    }
                    Some(RegistryItem::Function(_)) => {
                        return Err(CompilerError::parse_error(
                            format!(
                                "{}.{} must be called, cannot be used as value",
                                module_name, attr_name
                            ),
                            expr_span,
                        ));
                    }
                    Some(RegistryItem::Constant(const_def)) => {
                        return Ok(self.module.exprs.alloc(Expr {
                            kind: ExprKind::StdlibConst(const_def),
                            ty: None,
                            span: expr_span,
                        }));
                    }
                    Some(RegistryItem::Class(_)) => {
                        return Err(CompilerError::parse_error(
                            format!(
                                "Stdlib class '{}.{}' cannot be used as value",
                                module_name, attr_name
                            ),
                            expr_span,
                        ));
                    }
                    None => {
                        let available = stdlib::list_all_names(module_name);
                        return Err(CompilerError::parse_error(
                            format!(
                                "Unknown attribute '{}.{}'. Available: {}",
                                module_name,
                                attr_name,
                                available.join(", ")
                            ),
                            expr_span,
                        ));
                    }
                }
            }

            // Check if this is user module.attr access
            if let Some(module_path) = self.imports.imported_modules.get(&name_str).cloned() {
                let attr_name = self.interner.intern(&attr.attr);
                return Ok(self.module.exprs.alloc(Expr {
                    kind: ExprKind::ModuleAttr {
                        module: module_path,
                        attr: attr_name,
                    },
                    ty: None,
                    span: expr_span,
                }));
            }
        }

        // Check for chained module access: pkg.sub.VAR
        if let Some(module_path) = self.try_resolve_chained_module_attr(&attr.value, &attr.attr) {
            let attr_name = self.interner.intern(&attr.attr);
            return Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::ModuleAttr {
                    module: module_path,
                    attr: attr_name,
                },
                ty: None,
                span: expr_span,
            }));
        }

        // Field/attribute access: obj.field
        let obj = self.convert_expr(*attr.value)?;
        let attr_name = self.interner.intern(&attr.attr);
        Ok(self.module.exprs.alloc(Expr {
            kind: ExprKind::Attribute {
                obj,
                attr: attr_name,
            },
            ty: None,
            span: expr_span,
        }))
    }

    /// Try to resolve a chained attribute access like `pkg.sub` to a module path.
    /// This handles `import pkg.sub` then accessing `pkg.sub.func`.
    pub(crate) fn try_resolve_chained_module_path(
        &self,
        expr: &py::Expr,
        _final_attr: &str,
    ) -> Option<String> {
        self.build_module_path_from_expr(expr)
    }

    /// Try to resolve a chained attribute access for variable access.
    /// This handles `import pkg.sub` then accessing `pkg.sub.VAR`.
    fn try_resolve_chained_module_attr(
        &self,
        expr: &py::Expr,
        _final_attr: &str,
    ) -> Option<String> {
        self.build_module_path_from_expr(expr)
    }

    /// Build a module path from a chained attribute expression.
    /// For `pkg.sub`, returns Some("pkg.sub") if it matches a dotted import.
    fn build_module_path_from_expr(&self, expr: &py::Expr) -> Option<String> {
        let mut parts = Vec::new();
        let mut current = expr;

        loop {
            match current {
                py::Expr::Attribute(attr) => {
                    parts.push(attr.attr.as_str());
                    current = &attr.value;
                }
                py::Expr::Name(name) => {
                    parts.push(&name.id);
                    break;
                }
                _ => return None,
            }
        }

        // Reverse to get the path in order (root to leaf)
        parts.reverse();
        let full_path = parts.join(".");

        // Check if this full path matches a dotted import
        if self.imports.dotted_imports.contains_key(&full_path) {
            return Some(full_path);
        }

        None
    }
}
