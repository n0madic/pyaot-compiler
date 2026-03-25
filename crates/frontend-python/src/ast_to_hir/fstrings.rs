use super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::*;
use pyaot_types::Type;
use pyaot_utils::{InternedString, Span};
use rustpython_parser::ast as py;
use rustpython_parser::ast::ConversionFlag;

/// Parsed format specification (e.g., ">10.2f")
#[derive(Debug, Default)]
struct FormatSpec {
    fill: Option<char>,      // Fill character (default: space)
    align: Option<char>,     // '<' (left), '>' (right), '^' (center), '=' (pad after sign)
    sign: Option<char>,      // '+', '-', ' '
    zero_pad: bool,          // '0' flag (zero-pad, implies fill='0' and align='=' as defaults)
    width: Option<u32>,      // Minimum field width
    grouping: Option<char>,  // Grouping option: ',' or '_'
    precision: Option<u32>,  // For floats: decimal places
    type_char: Option<char>, // 'f', 's', 'd', 'x', 'X', 'o', 'b', etc.
}

/// Kind of placeholder in format string
#[derive(Debug)]
enum PlaceholderKind {
    Auto,          // {}
    Index(usize),  // {0}, {1}
    Named(String), // {name}
}

/// Parsed placeholder from format string
#[derive(Debug)]
struct ParsedPlaceholder {
    kind: PlaceholderKind,
    format_spec: FormatSpec,
}

impl FormatSpec {
    /// Parse a format specification string (everything after ':' in a placeholder)
    /// Format: [[fill]align][sign][0][width][.precision][type]
    /// Python's full spec: [[fill]align][sign][z][#][0][width][grouping_option][.precision][type]
    fn parse(spec: &str) -> Self {
        let mut result = FormatSpec::default();
        if spec.is_empty() {
            return result;
        }

        let chars: Vec<char> = spec.chars().collect();
        let mut i = 0;

        // Check for fill+align (fill character followed by align)
        // Or just align character
        if chars.len() >= 2 && matches!(chars[1], '<' | '>' | '^' | '=') {
            result.fill = Some(chars[0]);
            result.align = Some(chars[1]);
            i = 2;
        } else if !chars.is_empty() && matches!(chars[0], '<' | '>' | '^' | '=') {
            result.align = Some(chars[0]);
            i = 1;
        }

        // Parse sign ('+', '-', ' ')
        if i < chars.len() && matches!(chars[i], '+' | '-' | ' ') {
            result.sign = Some(chars[i]);
            i += 1;
        }

        // Parse zero-pad flag '0' (before width)
        if i < chars.len() && chars[i] == '0' {
            result.zero_pad = true;
            i += 1;
            // Set default fill/align for zero-pad if not explicitly set
            if result.fill.is_none() {
                result.fill = Some('0');
            }
            if result.align.is_none() {
                result.align = Some('=');
            }
        }

        // Parse width (sequence of digits)
        let mut width_str = String::new();
        while i < chars.len() && chars[i].is_ascii_digit() {
            width_str.push(chars[i]);
            i += 1;
        }
        if !width_str.is_empty() {
            result.width = width_str.parse().ok();
        }

        // Parse grouping option (',' or '_')
        if i < chars.len() && matches!(chars[i], ',' | '_') {
            result.grouping = Some(chars[i]);
            i += 1;
        }

        // Parse precision (.N)
        if i < chars.len() && chars[i] == '.' {
            i += 1;
            let mut prec_str = String::new();
            while i < chars.len() && chars[i].is_ascii_digit() {
                prec_str.push(chars[i]);
                i += 1;
            }
            if !prec_str.is_empty() {
                result.precision = prec_str.parse().ok();
            }
        }

        // Parse type character (f, s, d, x, X, o, b, etc.)
        if i < chars.len() && chars[i].is_alphabetic() {
            result.type_char = Some(chars[i]);
        }

        result
    }
}

impl ParsedPlaceholder {
    /// Parse the content inside {} in a format string
    /// Examples: "", "0", "name", ":>10", "0:>10", "name:<20.2f"
    fn parse(content: &str) -> Self {
        // Split on ':' to separate field name from format spec
        let (field_part, spec_part) = match content.find(':') {
            Some(pos) => (&content[..pos], &content[pos + 1..]),
            None => (content, ""),
        };

        // Parse the field part
        let kind = if field_part.is_empty() {
            PlaceholderKind::Auto
        } else if field_part.chars().all(|c| c.is_ascii_digit()) {
            // Index placeholder
            match field_part.parse::<usize>() {
                Ok(idx) => PlaceholderKind::Index(idx),
                Err(_) => PlaceholderKind::Auto, // Fallback for very large numbers
            }
        } else {
            // Named placeholder
            PlaceholderKind::Named(field_part.to_string())
        };

        let format_spec = FormatSpec::parse(spec_part);

        ParsedPlaceholder { kind, format_spec }
    }
}

impl AstToHir {
    /// Desugar an f-string into string concatenations.
    /// f"Hello {name}!" becomes "Hello " + str(name) + "!"
    pub(crate) fn desugar_fstring(
        &mut self,
        values: &[py::Expr],
        fstring_span: Span,
    ) -> Result<ExprId> {
        if values.is_empty() {
            // Empty f-string: f""
            let interned = self.interner.intern("");
            return Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::Str(interned),
                ty: Some(Type::Str),
                span: fstring_span,
            }));
        }

        // Convert each part of the f-string
        let mut parts: Vec<ExprId> = Vec::new();
        for value in values {
            let part_id = self.convert_fstring_part(value, fstring_span)?;
            parts.push(part_id);
        }

        // If only one part, return it directly
        if parts.len() == 1 {
            return Ok(parts[0]);
        }

        // Chain the parts with string concatenation (BinOp::Add)
        // ("a" + "b") + "c" etc.
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

    /// Convert a single part of an f-string (either a literal string or a formatted value)
    fn convert_fstring_part(&mut self, value: &py::Expr, fstring_span: Span) -> Result<ExprId> {
        match value {
            py::Expr::Constant(c) => {
                // String literal part
                if let py::Constant::Str(s) = &c.value {
                    let interned = self.interner.intern(s);
                    Ok(self.module.exprs.alloc(Expr {
                        kind: ExprKind::Str(interned),
                        ty: Some(Type::Str),
                        span: fstring_span,
                    }))
                } else {
                    Err(CompilerError::parse_error(
                        "Non-string constant in f-string",
                        fstring_span,
                    ))
                }
            }
            py::Expr::FormattedValue(fv) => {
                // Interpolated value: {expr} or {expr!s} or {expr!r}
                self.convert_formatted_value(fv, fstring_span)
            }
            _ => {
                // Shouldn't happen in a well-formed f-string
                Err(CompilerError::parse_error(
                    format!("Unexpected expression in f-string: {:?}", value),
                    fstring_span,
                ))
            }
        }
    }

    /// Convert a FormattedValue (the {expr} part of an f-string)
    ///
    /// Python semantics (PEP 498): conversion flag is applied BEFORE format spec.
    /// - `f"{x!r:>20}"` → repr(x), then right-align to width 20
    /// - `f"{x:.2f}"` → format x as float with 2 decimal places, then str()
    fn convert_formatted_value(
        &mut self,
        fv: &py::ExprFormattedValue,
        fstring_span: Span,
    ) -> Result<ExprId> {
        // Convert the expression inside the braces
        let expr_id = self.convert_expr(*fv.value.clone())?;

        match fv.conversion {
            ConversionFlag::Repr => {
                // !r: apply repr() first (conversion before formatting per PEP 498)
                let repr_expr = self.module.exprs.alloc(Expr {
                    kind: ExprKind::BuiltinCall {
                        builtin: Builtin::Repr,
                        args: vec![expr_id],
                        kwargs: vec![],
                    },
                    ty: Some(Type::Str),
                    span: fstring_span,
                });
                // Apply format spec to the repr string (string-level formatting only)
                if let Some(ref format_spec) = fv.format_spec {
                    self.apply_format_spec(repr_expr, format_spec, fstring_span)
                } else {
                    Ok(repr_expr)
                }
            }
            ConversionFlag::Ascii => {
                // !a: apply ascii() first (conversion before formatting per PEP 498)
                let ascii_expr = self.module.exprs.alloc(Expr {
                    kind: ExprKind::BuiltinCall {
                        builtin: Builtin::Ascii,
                        args: vec![expr_id],
                        kwargs: vec![],
                    },
                    ty: Some(Type::Str),
                    span: fstring_span,
                });
                if let Some(ref format_spec) = fv.format_spec {
                    self.apply_format_spec(ascii_expr, format_spec, fstring_span)
                } else {
                    Ok(ascii_expr)
                }
            }
            ConversionFlag::Str => {
                // !s: apply str() first, then format spec (string-level formatting)
                let str_expr = self.module.exprs.alloc(Expr {
                    kind: ExprKind::BuiltinCall {
                        builtin: Builtin::Str,
                        args: vec![expr_id],
                        kwargs: vec![],
                    },
                    ty: Some(Type::Str),
                    span: fstring_span,
                });
                if let Some(ref format_spec) = fv.format_spec {
                    self.apply_format_spec(str_expr, format_spec, fstring_span)
                } else {
                    Ok(str_expr)
                }
            }
            ConversionFlag::None => {
                // No conversion: apply format spec to original type (numeric formatting),
                // then ensure string result
                let formatted_expr = if let Some(ref format_spec) = fv.format_spec {
                    self.apply_format_spec(expr_id, format_spec, fstring_span)?
                } else {
                    expr_id
                };
                let expr = &self.module.exprs[formatted_expr];
                if matches!(expr.kind, ExprKind::Str(_)) {
                    Ok(formatted_expr)
                } else {
                    Ok(self.module.exprs.alloc(Expr {
                        kind: ExprKind::BuiltinCall {
                            builtin: Builtin::Str,
                            args: vec![formatted_expr],
                            kwargs: vec![],
                        },
                        ty: Some(Type::Str),
                        span: fstring_span,
                    }))
                }
            }
        }
    }

    /// Apply format spec to an expression (e.g., :.2f for float formatting)
    fn apply_format_spec(
        &mut self,
        expr_id: ExprId,
        format_spec: &py::Expr,
        fstring_span: Span,
    ) -> Result<ExprId> {
        // The format_spec is a JoinedStr containing the format string parts
        // For simple cases like ".2f", it will be a single Constant(Str(".2f"))
        let spec_str = self.extract_format_spec_string(format_spec)?;

        // Parse the format spec using the full parser
        let spec = FormatSpec::parse(&spec_str);
        let mut result = expr_id;

        // Check if the original expression is numeric (for default alignment)
        let is_numeric = matches!(
            self.module.exprs[expr_id].ty,
            Some(Type::Int) | Some(Type::Float)
        );

        // Apply precision for floats
        if let Some(precision) = spec.precision {
            if spec.type_char == Some('f') || spec.type_char == Some('F') {
                let precision_expr = self.module.exprs.alloc(Expr {
                    kind: ExprKind::Int(precision as i64),
                    ty: Some(Type::Int),
                    span: fstring_span,
                });

                result = self.module.exprs.alloc(Expr {
                    kind: ExprKind::BuiltinCall {
                        builtin: Builtin::Round,
                        args: vec![result, precision_expr],
                        kwargs: vec![],
                    },
                    ty: Some(Type::Float),
                    span: fstring_span,
                });
            }
        }

        // Apply grouping OR type_char conversions (mutually exclusive)
        if let Some(sep) = spec.grouping {
            let sep_expr = self.module.exprs.alloc(Expr {
                kind: ExprKind::Int(sep as i64),
                ty: Some(Type::Int),
                span: fstring_span,
            });

            // Check if value is float (has precision with f/F type)
            let is_float_fmt =
                spec.precision.is_some() && matches!(spec.type_char, Some('f') | Some('F'));

            if is_float_fmt {
                let precision_expr = self.module.exprs.alloc(Expr {
                    kind: ExprKind::Int(spec.precision.unwrap() as i64),
                    ty: Some(Type::Int),
                    span: fstring_span,
                });
                result = self.module.exprs.alloc(Expr {
                    kind: ExprKind::BuiltinCall {
                        builtin: Builtin::FmtFloatGrouped,
                        args: vec![result, precision_expr, sep_expr],
                        kwargs: vec![],
                    },
                    ty: Some(Type::Str),
                    span: fstring_span,
                });
            } else {
                result = self.module.exprs.alloc(Expr {
                    kind: ExprKind::BuiltinCall {
                        builtin: Builtin::FmtIntGrouped,
                        args: vec![result, sep_expr],
                        kwargs: vec![],
                    },
                    ty: Some(Type::Str),
                    span: fstring_span,
                });
            }
        } else {
            // Apply integer format type conversions
            match spec.type_char {
                Some('x') => {
                    result = self.module.exprs.alloc(Expr {
                        kind: ExprKind::BuiltinCall {
                            builtin: Builtin::FmtHex,
                            args: vec![result],
                            kwargs: vec![],
                        },
                        ty: Some(Type::Str),
                        span: fstring_span,
                    });
                }
                Some('X') => {
                    result = self.module.exprs.alloc(Expr {
                        kind: ExprKind::BuiltinCall {
                            builtin: Builtin::FmtHexUpper,
                            args: vec![result],
                            kwargs: vec![],
                        },
                        ty: Some(Type::Str),
                        span: fstring_span,
                    });
                }
                Some('o') => {
                    result = self.module.exprs.alloc(Expr {
                        kind: ExprKind::BuiltinCall {
                            builtin: Builtin::FmtOct,
                            args: vec![result],
                            kwargs: vec![],
                        },
                        ty: Some(Type::Str),
                        span: fstring_span,
                    });
                }
                Some('b') => {
                    result = self.module.exprs.alloc(Expr {
                        kind: ExprKind::BuiltinCall {
                            builtin: Builtin::FmtBin,
                            args: vec![result],
                            kwargs: vec![],
                        },
                        ty: Some(Type::Str),
                        span: fstring_span,
                    });
                }
                _ => {}
            }
        }

        // Apply width/alignment if specified
        if let Some(width) = spec.width {
            // Ensure the value is a string before applying alignment
            if !matches!(self.module.exprs[result].ty, Some(Type::Str)) {
                result = self.module.exprs.alloc(Expr {
                    kind: ExprKind::BuiltinCall {
                        builtin: Builtin::Str,
                        args: vec![result],
                        kwargs: vec![],
                    },
                    ty: Some(Type::Str),
                    span: fstring_span,
                });
            }

            let width_expr = self.module.exprs.alloc(Expr {
                kind: ExprKind::Int(width as i64),
                ty: Some(Type::Int),
                span: fstring_span,
            });

            let fill_char = spec.fill.unwrap_or(' ');
            let fill_str = fill_char.to_string();
            let fill_interned = self.interner.intern(&fill_str);
            let fill_expr = self.module.exprs.alloc(Expr {
                kind: ExprKind::Str(fill_interned),
                ty: Some(Type::Str),
                span: fstring_span,
            });

            // Default alignment: '>' (right) for numbers, '<' (left) for strings
            let default_align = if is_numeric { '>' } else { '<' };
            let align = spec.align.unwrap_or(default_align);
            let method_name = match align {
                '<' => "ljust",
                '>' | '=' => "rjust",
                '^' => "center",
                _ => "ljust",
            };

            let method = self.interner.intern(method_name);
            result = self.module.exprs.alloc(Expr {
                kind: ExprKind::MethodCall {
                    obj: result,
                    method,
                    args: vec![width_expr, fill_expr],
                    kwargs: vec![],
                },
                ty: Some(Type::Str),
                span: fstring_span,
            });
        }

        Ok(result)
    }

    /// Extract the format spec as a string
    fn extract_format_spec_string(&self, format_spec: &py::Expr) -> Result<String> {
        match format_spec {
            py::Expr::JoinedStr(js) => {
                // JoinedStr contains a list of values
                let mut result = String::new();
                for val in &js.values {
                    if let py::Expr::Constant(c) = val {
                        if let py::Constant::Str(s) = &c.value {
                            result.push_str(s);
                        }
                    }
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

    /// Desugar a .format() call into string concatenations.
    /// "Hello {}!".format(name) becomes "Hello " + str(name) + "!"
    /// Supports:
    /// - {} - auto-numbered positional
    /// - {0}, {1} - indexed positional
    /// - {name} - keyword arguments
    /// - {:>10}, {:<5}, {:^20} - width and alignment
    /// - {0:>10}, {name:<20} - combined
    pub(crate) fn desugar_format_string(
        &mut self,
        format_str: &str,
        args: &[ExprId],
        kwargs: &[(InternedString, ExprId)],
        format_span: Span,
    ) -> Result<ExprId> {
        // Parse the format string into parts (literals and placeholders)
        let mut parts: Vec<ExprId> = Vec::new();
        let mut current_literal = String::new();
        let mut auto_arg_index = 0;
        let mut chars = format_str.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '{' {
                if chars.peek() == Some(&'{') {
                    // Escaped {{ -> literal {
                    chars.next();
                    current_literal.push('{');
                } else {
                    // Collect everything until matching '}'
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

                    // Add current literal if not empty
                    if !current_literal.is_empty() {
                        let interned = self.interner.intern(&current_literal);
                        parts.push(self.module.exprs.alloc(Expr {
                            kind: ExprKind::Str(interned),
                            ty: Some(Type::Str),
                            span: format_span,
                        }));
                        current_literal.clear();
                    }

                    // Parse the placeholder
                    let placeholder = ParsedPlaceholder::parse(&placeholder_content);

                    // Resolve the argument based on placeholder kind
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
                            // Look up in kwargs
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

                    // Apply formatting and add to parts
                    let formatted = self.apply_format_placeholder(
                        arg_expr,
                        &placeholder.format_spec,
                        format_span,
                    )?;
                    parts.push(formatted);
                }
            } else if c == '}' {
                if chars.peek() == Some(&'}') {
                    // Escaped }} -> literal }
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

        // Add remaining literal
        if !current_literal.is_empty() {
            let interned = self.interner.intern(&current_literal);
            parts.push(self.module.exprs.alloc(Expr {
                kind: ExprKind::Str(interned),
                ty: Some(Type::Str),
                span: format_span,
            }));
        }

        // Handle empty format string
        if parts.is_empty() {
            let interned = self.interner.intern("");
            return Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::Str(interned),
                ty: Some(Type::Str),
                span: format_span,
            }));
        }

        // Chain parts with concatenation
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

    /// Apply format specification to an expression for .format() placeholders
    /// Handles precision, string conversion, and alignment
    fn apply_format_placeholder(
        &mut self,
        expr_id: ExprId,
        spec: &FormatSpec,
        span: Span,
    ) -> Result<ExprId> {
        let mut result = expr_id;

        // Check if the original expression is numeric (for default alignment)
        let is_numeric = matches!(
            self.module.exprs[expr_id].ty,
            Some(Type::Int) | Some(Type::Float)
        );

        // Apply precision for floats (e.g., .2f)
        if let Some(precision) = spec.precision {
            if spec.type_char == Some('f') || spec.type_char == Some('F') {
                let precision_expr = self.module.exprs.alloc(Expr {
                    kind: ExprKind::Int(precision as i64),
                    ty: Some(Type::Int),
                    span,
                });
                result = self.module.exprs.alloc(Expr {
                    kind: ExprKind::BuiltinCall {
                        builtin: Builtin::Round,
                        args: vec![result, precision_expr],
                        kwargs: vec![],
                    },
                    ty: Some(Type::Float),
                    span,
                });
            }
        }

        // Apply grouping if specified (e.g., :, or :_)
        if let Some(sep) = spec.grouping {
            let sep_expr = self.module.exprs.alloc(Expr {
                kind: ExprKind::Int(sep as i64),
                ty: Some(Type::Int),
                span,
            });

            let is_float_fmt =
                spec.precision.is_some() && matches!(spec.type_char, Some('f') | Some('F'));

            if is_float_fmt {
                let precision_expr = self.module.exprs.alloc(Expr {
                    kind: ExprKind::Int(spec.precision.unwrap() as i64),
                    ty: Some(Type::Int),
                    span,
                });
                result = self.module.exprs.alloc(Expr {
                    kind: ExprKind::BuiltinCall {
                        builtin: Builtin::FmtFloatGrouped,
                        args: vec![result, precision_expr, sep_expr],
                        kwargs: vec![],
                    },
                    ty: Some(Type::Str),
                    span,
                });
            } else {
                result = self.module.exprs.alloc(Expr {
                    kind: ExprKind::BuiltinCall {
                        builtin: Builtin::FmtIntGrouped,
                        args: vec![result, sep_expr],
                        kwargs: vec![],
                    },
                    ty: Some(Type::Str),
                    span,
                });
            }
        } else {
            // Convert to string based on type_char
            let fmt_builtin = match spec.type_char {
                Some('x') => Some(Builtin::FmtHex),
                Some('X') => Some(Builtin::FmtHexUpper),
                Some('o') => Some(Builtin::FmtOct),
                Some('b') => Some(Builtin::FmtBin),
                _ => None,
            };

            if let Some(builtin) = fmt_builtin {
                // Integer format type: emit format-specific conversion
                result = self.module.exprs.alloc(Expr {
                    kind: ExprKind::BuiltinCall {
                        builtin,
                        args: vec![result],
                        kwargs: vec![],
                    },
                    ty: Some(Type::Str),
                    span,
                });
            } else {
                // Default: convert to string using str()
                let expr = &self.module.exprs[result];
                let is_str = matches!(expr.kind, ExprKind::Str(_));
                if !is_str {
                    result = self.module.exprs.alloc(Expr {
                        kind: ExprKind::BuiltinCall {
                            builtin: Builtin::Str,
                            args: vec![result],
                            kwargs: vec![],
                        },
                        ty: Some(Type::Str),
                        span,
                    });
                }
            }
        }

        // Apply alignment if width is specified
        if let Some(width) = spec.width {
            let width_expr = self.module.exprs.alloc(Expr {
                kind: ExprKind::Int(width as i64),
                ty: Some(Type::Int),
                span,
            });

            // Determine fill character (default is space)
            let fill_char = spec.fill.unwrap_or(' ');
            let fill_str = fill_char.to_string();
            let fill_interned = self.interner.intern(&fill_str);
            let fill_expr = self.module.exprs.alloc(Expr {
                kind: ExprKind::Str(fill_interned),
                ty: Some(Type::Str),
                span,
            });

            // Determine alignment method
            // Default alignment is '>' (right) for numbers, '<' (left) for strings
            let default_align = if is_numeric { '>' } else { '<' };
            let align = spec.align.unwrap_or(default_align);
            let method_name = match align {
                '<' => "ljust",
                '>' => "rjust",
                '^' => "center",
                _ => "ljust", // Fallback
            };

            let method = self.interner.intern(method_name);
            result = self.module.exprs.alloc(Expr {
                kind: ExprKind::MethodCall {
                    obj: result,
                    method,
                    args: vec![width_expr, fill_expr],
                    kwargs: vec![],
                },
                ty: Some(Type::Str),
                span,
            });
        }

        Ok(result)
    }
}
