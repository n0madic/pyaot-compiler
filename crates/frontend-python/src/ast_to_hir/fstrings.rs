use super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::*;
use pyaot_types::Type;
use pyaot_utils::{InternedString, Span};
use rustpython_parser::ast as py;
use rustpython_parser::ast::ConversionFlag;

/// Kind of placeholder in a `.format()` string.
#[derive(Debug)]
enum PlaceholderKind {
    Auto,          // {}
    Index(usize),  // {0}, {1}
    Named(String), // {name}
}

/// Parsed placeholder: argument selector + raw format spec string.
#[derive(Debug)]
struct ParsedPlaceholder {
    kind: PlaceholderKind,
    /// Raw format spec string (everything after ':' in the placeholder), e.g. ">10.2f".
    spec: String,
}

impl ParsedPlaceholder {
    fn parse(content: &str) -> Self {
        let (field_part, spec_part) = match content.find(':') {
            Some(pos) => (&content[..pos], &content[pos + 1..]),
            None => (content, ""),
        };

        let kind = if field_part.is_empty() {
            PlaceholderKind::Auto
        } else if field_part.chars().all(|c| c.is_ascii_digit()) {
            match field_part.parse::<usize>() {
                Ok(idx) => PlaceholderKind::Index(idx),
                Err(_) => PlaceholderKind::Auto,
            }
        } else {
            PlaceholderKind::Named(field_part.to_string())
        };

        ParsedPlaceholder {
            kind,
            spec: spec_part.to_string(),
        }
    }
}

impl AstToHir {
    /// Desugar an f-string into string concatenations.
    /// `f"Hello {name!r:>10}!"` desugars into individual `FormatSpec` / `BuiltinCall::Str`
    /// nodes joined with `BinOp::Add`.
    pub(crate) fn desugar_fstring(
        &mut self,
        values: &[py::Expr],
        fstring_span: Span,
    ) -> Result<ExprId> {
        if values.is_empty() {
            let interned = self.interner.intern("");
            return Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::Str(interned),
                ty: Some(Type::Str),
                span: fstring_span,
            }));
        }

        let mut parts: Vec<ExprId> = Vec::new();
        for value in values {
            let part_id = self.convert_fstring_part(value, fstring_span)?;
            parts.push(part_id);
        }

        if parts.len() == 1 {
            return Ok(parts[0]);
        }

        // Chain with BinOp::Add
        let mut result = parts[0];
        for &part in &parts[1..] {
            result = self.module.exprs.alloc(Expr {
                kind: ExprKind::BinOp {
                    op: BinOp::Add,
                    left: result,
                    right: part,
                },
                ty: Some(Type::Str),
                span: fstring_span,
            });
        }
        Ok(result)
    }

    fn convert_fstring_part(&mut self, value: &py::Expr, span: Span) -> Result<ExprId> {
        match value {
            py::Expr::Constant(c) => {
                if let py::Constant::Str(s) = &c.value {
                    let interned = self.interner.intern(s);
                    Ok(self.module.exprs.alloc(Expr {
                        kind: ExprKind::Str(interned),
                        ty: Some(Type::Str),
                        span,
                    }))
                } else {
                    Err(CompilerError::parse_error(
                        "Non-string constant in f-string",
                        span,
                    ))
                }
            }
            py::Expr::FormattedValue(fv) => self.convert_formatted_value(fv, span),
            _ => Err(CompilerError::parse_error(
                format!("Unexpected expression in f-string: {:?}", value),
                span,
            )),
        }
    }

    /// Convert a `FormattedValue` (`{expr}`, `{expr!r}`, `{expr:spec}`).
    ///
    /// PEP 498: conversion flag is applied BEFORE the format spec.
    fn convert_formatted_value(
        &mut self,
        fv: &py::ExprFormattedValue,
        span: Span,
    ) -> Result<ExprId> {
        let expr_id = self.convert_expr(*fv.value.clone())?;

        // Apply conversion flag (always produces Type::Str)
        let converted = match fv.conversion {
            ConversionFlag::Repr => self.module.exprs.alloc(Expr {
                kind: ExprKind::BuiltinCall {
                    builtin: Builtin::Repr,
                    args: vec![expr_id],
                    kwargs: vec![],
                },
                ty: Some(Type::Str),
                span,
            }),
            ConversionFlag::Ascii => self.module.exprs.alloc(Expr {
                kind: ExprKind::BuiltinCall {
                    builtin: Builtin::Ascii,
                    args: vec![expr_id],
                    kwargs: vec![],
                },
                ty: Some(Type::Str),
                span,
            }),
            ConversionFlag::Str => self.module.exprs.alloc(Expr {
                kind: ExprKind::BuiltinCall {
                    builtin: Builtin::Str,
                    args: vec![expr_id],
                    kwargs: vec![],
                },
                ty: Some(Type::Str),
                span,
            }),
            ConversionFlag::None => expr_id,
        };

        // Apply format spec if present
        if let Some(ref spec_ast) = fv.format_spec {
            self.apply_format_spec_node(converted, spec_ast, span)
        } else if fv.conversion == ConversionFlag::None {
            // No conversion, no spec: PEP 498 says call format(value, "").
            // Route through FormatSpec so user-class __format__ is honoured.
            let empty_spec = self.interner.intern("");
            Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::FormatSpec {
                    value: converted,
                    spec: empty_spec,
                    span,
                },
                ty: Some(Type::Str),
                span,
            }))
        } else {
            // After conversion (!r/!s/!a) with no spec: already a str
            Ok(converted)
        }
    }

    /// Emit `ExprKind::FormatSpec` for static specs, or `Builtin::Format` for dynamic
    /// (nested f-string) specs like `f"{x:.{n}f}"`.
    fn apply_format_spec_node(
        &mut self,
        value: ExprId,
        spec_ast: &py::Expr,
        span: Span,
    ) -> Result<ExprId> {
        // Try to extract a static spec string first
        let spec_str = self.extract_format_spec_string(spec_ast)?;

        if !self.spec_is_dynamic(spec_ast) {
            // Static spec: emit FormatSpec HIR node (lowered to rt_format at MIR level).
            // Empty spec is allowed — lower_format_spec handles it correctly (calls
            // __format__("") for user classes, rt_obj_to_str for primitives).
            let spec_interned = self.interner.intern(&spec_str);
            Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::FormatSpec {
                    value,
                    spec: spec_interned,
                    span,
                },
                ty: Some(Type::Str),
                span,
            }))
        } else {
            // Dynamic spec (nested f-string expressions): desugar spec to a str expression,
            // then call format(value, spec_str_expr) — uses the same RT_FORMAT path.
            let spec_expr = self.desugar_fstring_spec(spec_ast, span)?;
            Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::BuiltinCall {
                    builtin: Builtin::Format,
                    args: vec![value, spec_expr],
                    kwargs: vec![],
                },
                ty: Some(Type::Str),
                span,
            }))
        }
    }

    /// Check whether a format_spec AST node contains nested dynamic expressions.
    fn spec_is_dynamic(&self, spec_ast: &py::Expr) -> bool {
        if let py::Expr::JoinedStr(js) = spec_ast {
            js.values
                .iter()
                .any(|v| matches!(v, py::Expr::FormattedValue(_)))
        } else {
            false
        }
    }

    /// Desugar a format_spec JoinedStr to a str expression (for dynamic specs).
    fn desugar_fstring_spec(&mut self, spec_ast: &py::Expr, span: Span) -> Result<ExprId> {
        match spec_ast {
            py::Expr::JoinedStr(js) => self.desugar_fstring(&js.values, span),
            py::Expr::Constant(c) => {
                let s = if let py::Constant::Str(s) = &c.value {
                    s.as_str()
                } else {
                    ""
                };
                let interned = self.interner.intern(s);
                Ok(self.module.exprs.alloc(Expr {
                    kind: ExprKind::Str(interned),
                    ty: Some(Type::Str),
                    span,
                }))
            }
            _ => {
                let interned = self.interner.intern("");
                Ok(self.module.exprs.alloc(Expr {
                    kind: ExprKind::Str(interned),
                    ty: Some(Type::Str),
                    span,
                }))
            }
        }
    }

    /// Extract the static format spec as a raw string (e.g., `">10.2f"`, `","`).
    /// Returns `""` for empty or non-static specs.
    fn extract_format_spec_string(&self, format_spec: &py::Expr) -> Result<String> {
        match format_spec {
            py::Expr::JoinedStr(js) => {
                let mut result = String::new();
                for val in &js.values {
                    if let py::Expr::Constant(c) = val {
                        if let py::Constant::Str(s) = &c.value {
                            result.push_str(s);
                        }
                    }
                    // Dynamic parts (FormattedValue) are skipped; caller uses spec_is_dynamic
                }
                Ok(result)
            }
            py::Expr::Constant(c) => {
                if let py::Constant::Str(s) = &c.value {
                    Ok(s.clone())
                } else {
                    Ok(String::new())
                }
            }
            _ => Ok(String::new()),
        }
    }

    /// Desugar a `.format()` call: `"Hello {:>10}!".format(name)`.
    ///
    /// Supports `{}`, `{0}`, `{name}`, `{:spec}` placeholders.
    pub(crate) fn desugar_format_string(
        &mut self,
        format_str: &str,
        args: &[ExprId],
        kwargs: &[(InternedString, ExprId)],
        format_span: Span,
    ) -> Result<ExprId> {
        let mut parts: Vec<ExprId> = Vec::new();
        let mut current_literal = String::new();
        let mut auto_arg_index = 0;
        let mut chars = format_str.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '{' {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    current_literal.push('{');
                } else {
                    let mut placeholder_content = String::new();
                    let mut found_close = false;
                    for ch in chars.by_ref() {
                        if ch == '}' {
                            found_close = true;
                            break;
                        }
                        placeholder_content.push(ch);
                    }
                    if !found_close {
                        return Err(CompilerError::parse_error(
                            "Unclosed { in format string",
                            format_span,
                        ));
                    }

                    if !current_literal.is_empty() {
                        let interned = self.interner.intern(&current_literal);
                        parts.push(self.module.exprs.alloc(Expr {
                            kind: ExprKind::Str(interned),
                            ty: Some(Type::Str),
                            span: format_span,
                        }));
                        current_literal.clear();
                    }

                    let placeholder = ParsedPlaceholder::parse(&placeholder_content);

                    let arg_expr = match &placeholder.kind {
                        PlaceholderKind::Auto => {
                            if auto_arg_index < args.len() {
                                let expr = args[auto_arg_index];
                                auto_arg_index += 1;
                                expr
                            } else {
                                return Err(CompilerError::parse_error(
                                    "Not enough arguments for format string",
                                    format_span,
                                ));
                            }
                        }
                        PlaceholderKind::Index(idx) => {
                            if *idx < args.len() {
                                args[*idx]
                            } else {
                                return Err(CompilerError::parse_error(
                                    format!(
                                        "Replacement index {} out of range for positional args tuple",
                                        idx
                                    ),
                                    format_span,
                                ));
                            }
                        }
                        PlaceholderKind::Named(name) => {
                            let interned_name = self.interner.intern(name);
                            kwargs
                                .iter()
                                .find(|(k, _)| *k == interned_name)
                                .map(|(_, v)| *v)
                                .ok_or_else(|| {
                                    CompilerError::parse_error(
                                        format!("KeyError: '{}'", name),
                                        format_span,
                                    )
                                })?
                        }
                    };

                    // Emit FormatSpec node or Str fallback
                    let formatted = if placeholder.spec.is_empty() {
                        self.module.exprs.alloc(Expr {
                            kind: ExprKind::BuiltinCall {
                                builtin: Builtin::Str,
                                args: vec![arg_expr],
                                kwargs: vec![],
                            },
                            ty: Some(Type::Str),
                            span: format_span,
                        })
                    } else {
                        let spec_interned = self.interner.intern(&placeholder.spec);
                        self.module.exprs.alloc(Expr {
                            kind: ExprKind::FormatSpec {
                                value: arg_expr,
                                spec: spec_interned,
                                span: format_span,
                            },
                            ty: Some(Type::Str),
                            span: format_span,
                        })
                    };
                    parts.push(formatted);
                }
            } else if c == '}' {
                if chars.peek() == Some(&'}') {
                    chars.next();
                    current_literal.push('}');
                } else {
                    return Err(CompilerError::parse_error(
                        "Single } in format string",
                        format_span,
                    ));
                }
            } else {
                current_literal.push(c);
            }
        }

        if !current_literal.is_empty() {
            let interned = self.interner.intern(&current_literal);
            parts.push(self.module.exprs.alloc(Expr {
                kind: ExprKind::Str(interned),
                ty: Some(Type::Str),
                span: format_span,
            }));
        }

        if parts.is_empty() {
            let interned = self.interner.intern("");
            return Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::Str(interned),
                ty: Some(Type::Str),
                span: format_span,
            }));
        }

        if parts.len() == 1 {
            return Ok(parts[0]);
        }

        let mut result = parts[0];
        for &part in &parts[1..] {
            result = self.module.exprs.alloc(Expr {
                kind: ExprKind::BinOp {
                    op: BinOp::Add,
                    left: result,
                    right: part,
                },
                ty: Some(Type::Str),
                span: format_span,
            });
        }
        Ok(result)
    }
}
