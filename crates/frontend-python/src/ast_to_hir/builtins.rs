use super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::*;
use pyaot_types::{BuiltinExceptionKind, Type};
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    /// Helper to create a simple builtin call expression.
    /// Most builtins follow this pattern: convert args, create BuiltinCall, set type.
    fn create_simple_builtin(
        &mut self,
        call: py::ExprCall,
        builtin: Builtin,
        ty: Option<Type>,
        kwargs: Vec<KeywordArg>,
        call_span: Span,
    ) -> Result<ExprId> {
        let mut args = Vec::new();
        for arg in call.args {
            args.push(self.convert_expr(arg)?);
        }
        Ok(self.module.exprs.alloc(Expr {
            kind: ExprKind::BuiltinCall {
                builtin,
                args,
                kwargs,
            },
            ty,
            span: call_span,
        }))
    }

    /// Handle builtin function calls (print, range, len, str, int, float, bool, abs, pow, min, max, round, sum, all, any, chr, ord, isinstance, hash, id, iter, next, reversed, sorted, set, Exception)
    pub(crate) fn handle_builtin_call(
        &mut self,
        name: &str,
        call: py::ExprCall,
        kwargs: Vec<KeywordArg>,
        kwargs_unpack: Option<ExprId>,
        call_span: Span,
    ) -> Result<Option<ExprId>> {
        // Check kwargs_unpack early — builtin functions don't support **kwargs unpacking
        // This must be checked before matching to avoid rejecting non-builtins
        if kwargs_unpack.is_some() {
            // Only reject if this is actually a builtin name
            let is_builtin = matches!(
                name,
                "print"
                    | "setattr"
                    | "len"
                    | "ord"
                    | "hash"
                    | "id"
                    | "int"
                    | "float"
                    | "pow"
                    | "bool"
                    | "all"
                    | "any"
                    | "callable"
                    | "hasattr"
                    | "str"
                    | "chr"
                    | "bin"
                    | "hex"
                    | "oct"
                    | "repr"
                    | "ascii"
                    | "type"
                    | "input"
                    | "bytes"
                    | "open"
                    | "set"
                    | "divmod"
                    | "getattr"
                    | "range"
                    | "abs"
                    | "min"
                    | "max"
                    | "round"
                    | "sum"
                    | "iter"
                    | "next"
                    | "reversed"
                    | "sorted"
                    | "enumerate"
                    | "zip"
                    | "map"
                    | "filter"
                    | "list"
                    | "tuple"
                    | "dict"
                    | "format"
                    | "reduce"
                    | "isinstance"
                    | "issubclass"
            ) || BuiltinExceptionKind::from_name(name).is_some();

            if is_builtin {
                return Err(CompilerError::parse_error(
                    "**kwargs unpacking not supported for builtin functions",
                    call_span,
                ));
            }
        }

        // Group builtins by return type for cleaner handling
        let result = match name {
            // Builtins returning None
            "print" => self.create_simple_builtin(
                call,
                Builtin::Print,
                Some(Type::None),
                kwargs,
                call_span,
            ),
            "setattr" => self.create_simple_builtin(
                call,
                Builtin::Setattr,
                Some(Type::None),
                kwargs,
                call_span,
            ),

            // Builtins returning Int
            "len" => {
                self.create_simple_builtin(call, Builtin::Len, Some(Type::Int), kwargs, call_span)
            }
            "ord" => {
                self.create_simple_builtin(call, Builtin::Ord, Some(Type::Int), kwargs, call_span)
            }
            "hash" => {
                self.create_simple_builtin(call, Builtin::Hash, Some(Type::Int), kwargs, call_span)
            }
            "id" => {
                self.create_simple_builtin(call, Builtin::Id, Some(Type::Int), kwargs, call_span)
            }
            "int" => {
                self.create_simple_builtin(call, Builtin::Int, Some(Type::Int), kwargs, call_span)
            }

            // Builtins returning Float
            "float" => self.create_simple_builtin(
                call,
                Builtin::Float,
                Some(Type::Float),
                kwargs,
                call_span,
            ),
            "pow" => {
                self.create_simple_builtin(call, Builtin::Pow, Some(Type::Float), kwargs, call_span)
            }

            // Builtins returning Bool
            "bool" => {
                self.create_simple_builtin(call, Builtin::Bool, Some(Type::Bool), kwargs, call_span)
            }
            "all" => {
                self.create_simple_builtin(call, Builtin::All, Some(Type::Bool), kwargs, call_span)
            }
            "any" => {
                self.create_simple_builtin(call, Builtin::Any, Some(Type::Bool), kwargs, call_span)
            }
            "callable" => self.create_simple_builtin(
                call,
                Builtin::Callable,
                Some(Type::Bool),
                kwargs,
                call_span,
            ),
            "hasattr" => self.create_simple_builtin(
                call,
                Builtin::Hasattr,
                Some(Type::Bool),
                kwargs,
                call_span,
            ),

            // Builtins returning Str
            "str" => {
                self.create_simple_builtin(call, Builtin::Str, Some(Type::Str), kwargs, call_span)
            }
            "chr" => {
                self.create_simple_builtin(call, Builtin::Chr, Some(Type::Str), kwargs, call_span)
            }
            "bin" => {
                self.create_simple_builtin(call, Builtin::Bin, Some(Type::Str), kwargs, call_span)
            }
            "hex" => {
                self.create_simple_builtin(call, Builtin::Hex, Some(Type::Str), kwargs, call_span)
            }
            "oct" => {
                self.create_simple_builtin(call, Builtin::Oct, Some(Type::Str), kwargs, call_span)
            }
            "repr" => {
                self.create_simple_builtin(call, Builtin::Repr, Some(Type::Str), kwargs, call_span)
            }
            "ascii" => {
                self.create_simple_builtin(call, Builtin::Ascii, Some(Type::Str), kwargs, call_span)
            }
            "type" => {
                self.create_simple_builtin(call, Builtin::Type, Some(Type::Str), kwargs, call_span)
            }
            "input" => {
                self.create_simple_builtin(call, Builtin::Input, Some(Type::Str), kwargs, call_span)
            }

            // Builtins returning Bytes
            "bytes" => self.create_simple_builtin(
                call,
                Builtin::Bytes,
                Some(Type::Bytes),
                kwargs,
                call_span,
            ),

            // Builtins returning File — inspect the `mode` literal (positional
            // arg index 1 or `mode=` kwarg) so `open(p, "rb").read()` statically
            // types as bytes while `open(p, "r").read()` stays as str. Mode
            // strings we can't constant-fold default to text.
            "open" => {
                let is_binary_mode = {
                    fn mode_from_ast(e: &py::Expr) -> Option<bool> {
                        if let py::Expr::Constant(c) = e {
                            if let rustpython_parser::ast::Constant::Str(s) = &c.value {
                                return Some(s.contains('b'));
                            }
                        }
                        None
                    }
                    let mode_interned = self.interner.lookup("mode");
                    let pos_mode = call.args.get(1).and_then(mode_from_ast);
                    let kw_mode = mode_interned.and_then(|m| {
                        kwargs.iter().find(|kw| kw.name == m).and_then(|kw| {
                            match &self.module.exprs[kw.value].kind {
                                ExprKind::Str(s) => self.interner.get(*s).map(|s| s.contains('b')),
                                _ => None,
                            }
                        })
                    });
                    pos_mode.or(kw_mode).unwrap_or(false)
                };
                self.create_simple_builtin(
                    call,
                    Builtin::Open,
                    Some(Type::File(is_binary_mode)),
                    kwargs,
                    call_span,
                )
            }

            // Builtins returning Set
            "set" => self.create_simple_builtin(
                call,
                Builtin::Set,
                Some(Type::set_of(Type::Any)),
                kwargs,
                call_span,
            ),

            // Builtins returning Tuple
            "divmod" => self.create_simple_builtin(
                call,
                Builtin::Divmod,
                Some(Type::tuple_of(vec![Type::Int, Type::Int])),
                kwargs,
                call_span,
            ),

            // Builtins returning Any
            "getattr" => self.create_simple_builtin(
                call,
                Builtin::Getattr,
                Some(Type::Any),
                kwargs,
                call_span,
            ),

            // Check if it's a built-in exception type. Stdlib-submodule
            // exceptions (e.g. HTTPError from urllib.error) are synthetic
            // classes handled through the regular class-instantiation path,
            // not here — so this only matches true Python builtins.
            _ if BuiltinExceptionKind::from_name(name).is_some() => {
                let kind = BuiltinExceptionKind::from_name(name)
                    .expect("internal error: from_name guaranteed by is_some() match guard");
                self.create_simple_builtin(
                    call,
                    Builtin::BuiltinException(kind),
                    Some(Type::Any),
                    kwargs,
                    call_span,
                )
            }

            // Builtins with type inferred from arguments (ty = None)
            "range" => self.create_simple_builtin(call, Builtin::Range, None, kwargs, call_span),
            "abs" => self.create_simple_builtin(call, Builtin::Abs, None, kwargs, call_span),
            "min" => self.create_simple_builtin(call, Builtin::Min, None, kwargs, call_span),
            "max" => self.create_simple_builtin(call, Builtin::Max, None, kwargs, call_span),
            "round" => self.create_simple_builtin(call, Builtin::Round, None, kwargs, call_span),
            "sum" => self.create_simple_builtin(call, Builtin::Sum, None, kwargs, call_span),
            "iter" => self.create_simple_builtin(call, Builtin::Iter, None, kwargs, call_span),
            "next" => self.create_simple_builtin(call, Builtin::Next, None, kwargs, call_span),
            "reversed" => {
                self.create_simple_builtin(call, Builtin::Reversed, None, kwargs, call_span)
            }
            "sorted" => self.create_simple_builtin(call, Builtin::Sorted, None, kwargs, call_span),
            "enumerate" => {
                self.create_simple_builtin(call, Builtin::Enumerate, None, kwargs, call_span)
            }
            "zip" => self.create_simple_builtin(call, Builtin::Zip, None, kwargs, call_span),
            "map" => self.create_simple_builtin(call, Builtin::Map, None, kwargs, call_span),
            "filter" => self.create_simple_builtin(call, Builtin::Filter, None, kwargs, call_span),
            "list" => self.create_simple_builtin(call, Builtin::List, None, kwargs, call_span),
            "tuple" => self.create_simple_builtin(call, Builtin::Tuple, None, kwargs, call_span),
            "dict" => self.create_simple_builtin(call, Builtin::Dict, None, kwargs, call_span),
            "format" => self.create_simple_builtin(call, Builtin::Format, None, kwargs, call_span),
            "reduce" => self.create_simple_builtin(call, Builtin::Reduce, None, kwargs, call_span),

            // Special case: isinstance requires exactly 2 args with special type handling
            "isinstance" => {
                if call.args.len() != 2 {
                    return Err(CompilerError::parse_error(
                        "isinstance requires exactly 2 arguments",
                        call_span,
                    ));
                }
                let obj_expr = self.convert_expr(call.args[0].clone())?;
                let type_expr = self.convert_type_expr(&call.args[1])?;
                Ok(self.module.exprs.alloc(Expr {
                    kind: ExprKind::BuiltinCall {
                        builtin: Builtin::Isinstance,
                        args: vec![obj_expr, type_expr],
                        kwargs,
                    },
                    ty: Some(Type::Bool),
                    span: call_span,
                }))
            }

            // Special case: issubclass requires exactly 2 args with special type handling
            "issubclass" => {
                if call.args.len() != 2 {
                    return Err(CompilerError::parse_error(
                        "issubclass requires exactly 2 arguments",
                        call_span,
                    ));
                }
                let class_expr = self.convert_type_expr(&call.args[0])?;
                let parent_expr = self.convert_type_expr(&call.args[1])?;
                Ok(self.module.exprs.alloc(Expr {
                    kind: ExprKind::BuiltinCall {
                        builtin: Builtin::Issubclass,
                        args: vec![class_expr, parent_expr],
                        kwargs,
                    },
                    ty: Some(Type::Bool),
                    span: call_span,
                }))
            }

            // Not a builtin
            _ => return Ok(None),
        };

        result.map(Some)
    }
}
