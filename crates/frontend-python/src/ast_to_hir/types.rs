use super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::*;
use pyaot_stdlib_defs::registry::get_class_type;
use pyaot_types::{typespec_to_type, Type};
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    pub(crate) fn convert_type_annotation(&mut self, ann: &py::Expr) -> Result<Type> {
        let ann_span = Self::span_from(ann);
        match ann {
            py::Expr::Name(name) => {
                // Check for basic types first
                match name.id.as_str() {
                    "int" => return Ok(Type::Int),
                    "float" => return Ok(Type::Float),
                    "bool" => return Ok(Type::Bool),
                    "str" => return Ok(Type::Str),
                    "bytes" => return Ok(Type::Bytes),
                    "None" => return Ok(Type::None),
                    "Any" => return Ok(Type::Any), // typing.Any doesn't need parameters
                    _ => {}
                }

                let interned = self.interner.intern(&name.id);

                // Check for type aliases
                if let Some(aliased_type) = self.types.type_aliases.get(&interned).cloned() {
                    return Ok(aliased_type);
                }

                // Check for TypeVar definitions
                // Constrained/bounded TypeVars resolve to their constraint type.
                // Unconstrained TypeVars resolve to Type::Var(name) — a placeholder
                // that signals "leave untyped for inference" in function parameters.
                if let Some(tv_def) = self.types.typevar_defs.get(&interned).cloned() {
                    if let Some(bound) = &tv_def.bound {
                        return Ok(bound.clone());
                    } else if !tv_def.constraints.is_empty() {
                        return Ok(Type::normalize_union(tv_def.constraints.clone()));
                    } else {
                        return Ok(Type::Var(interned));
                    }
                }

                // Check for user-defined class names
                if let Some(&class_id) = self.symbols.class_map.get(&interned) {
                    return Ok(Type::Class {
                        class_id,
                        name: interned,
                    });
                }

                // Check if it's a typing import that needs to be subscripted
                // (but not TypeAlias/TypeVar/Protocol which are handled differently)
                if self.types.typing_imports.contains(&interned) {
                    let name_str = name.id.as_str();
                    if name_str != "TypeAlias" && name_str != "TypeVar" && name_str != "Protocol" {
                        return Err(CompilerError::parse_error(
                            format!(
                                "Generic type '{}' from typing module requires type parameters, e.g. {}[...]",
                                name.id, name.id
                            ),
                            ann_span,
                        ));
                    }
                }

                Err(CompilerError::parse_error(
                    format!("Unknown type: {}", name.id),
                    ann_span,
                ))
            }
            // Handle None as a constant (Python 3 parses `-> None` as Constant::None)
            py::Expr::Constant(c) => {
                if matches!(c.value, py::Constant::None) {
                    Ok(Type::None)
                } else {
                    Err(CompilerError::parse_error(
                        "Only None constant allowed in type annotations",
                        ann_span,
                    ))
                }
            }
            py::Expr::Subscript(sub) => {
                // Handle generic types like list[int], dict[str, int]
                if let py::Expr::Name(name) = &*sub.value {
                    // Check if this is a typing module import
                    let interned = self.interner.intern(&name.id);
                    let is_typing_import = self.types.typing_imports.contains(&interned);

                    let name_str = name.id.as_str();
                    match name_str {
                        // Handle both PEP 585 (list[int]) and typing module (List[int])
                        "list" | "List" if name_str == "list" || is_typing_import => {
                            let elem_type = self.convert_type_annotation(&sub.slice)?;
                            Ok(Type::List(Box::new(elem_type)))
                        }
                        "dict" | "Dict" if name_str == "dict" || is_typing_import => {
                            // dict[K, V]
                            if let py::Expr::Tuple(tuple) = &*sub.slice {
                                if tuple.elts.len() == 2 {
                                    let key_type = self.convert_type_annotation(&tuple.elts[0])?;
                                    let val_type = self.convert_type_annotation(&tuple.elts[1])?;
                                    return Ok(Type::Dict(Box::new(key_type), Box::new(val_type)));
                                }
                            }
                            Err(CompilerError::parse_error(
                                "dict type must have [K, V]",
                                ann_span,
                            ))
                        }
                        "set" | "Set" if name_str == "set" || is_typing_import => {
                            // set[T]
                            let elem_type = self.convert_type_annotation(&sub.slice)?;
                            Ok(Type::Set(Box::new(elem_type)))
                        }
                        "tuple" | "Tuple" if name_str == "tuple" || is_typing_import => {
                            let mut types = Vec::new();
                            if let py::Expr::Tuple(tuple) = &*sub.slice {
                                for elem in &tuple.elts {
                                    types.push(self.convert_type_annotation(elem)?);
                                }
                            } else {
                                types.push(self.convert_type_annotation(&sub.slice)?);
                            }
                            Ok(Type::Tuple(types))
                        }
                        "Optional" if is_typing_import => {
                            // Optional[T] → Union[T, None]
                            let inner_type = self.convert_type_annotation(&sub.slice)?;
                            Ok(Type::normalize_union(vec![inner_type, Type::None]))
                        }
                        "Union" if is_typing_import => {
                            // Union[A, B, ...] → Union of all types
                            let mut types = Vec::new();
                            if let py::Expr::Tuple(tuple) = &*sub.slice {
                                for elem in &tuple.elts {
                                    types.push(self.convert_type_annotation(elem)?);
                                }
                            } else {
                                // Single type in Union (weird but possible)
                                types.push(self.convert_type_annotation(&sub.slice)?);
                            }
                            Ok(Type::normalize_union(types))
                        }
                        "Literal" if is_typing_import => {
                            // Literal[42] → int, Literal["hello"] → str (type erasure)
                            // Literal[42, "hello"] → Union[int, str]
                            if let py::Expr::Tuple(tuple) = &*sub.slice {
                                let mut types = Vec::new();
                                for elem in &tuple.elts {
                                    types.push(self.literal_value_to_type(elem, ann_span)?);
                                }
                                Ok(Type::normalize_union(types))
                            } else {
                                self.literal_value_to_type(&sub.slice, ann_span)
                            }
                        }
                        _ => Err(CompilerError::parse_error(
                            format!("Unknown generic type: {}", name.id),
                            ann_span,
                        )),
                    }
                } else {
                    Err(CompilerError::parse_error(
                        "Complex type annotations not supported",
                        ann_span,
                    ))
                }
            }
            py::Expr::BinOp(binop) => {
                // Handle Union types: int | str
                if matches!(binop.op, py::Operator::BitOr) {
                    let left = self.convert_type_annotation(&binop.left)?;
                    let right = self.convert_type_annotation(&binop.right)?;
                    Ok(Type::normalize_union(vec![left, right]))
                } else {
                    Err(CompilerError::parse_error(
                        "Only | operator supported for union types",
                        ann_span,
                    ))
                }
            }
            py::Expr::Attribute(attr) => {
                // Handle module-qualified types like time.struct_time
                if let py::Expr::Name(module_name) = &*attr.value {
                    let module = module_name.id.as_str();
                    let type_name = attr.attr.as_str();

                    // Look up in stdlib registry (Single Source of Truth)
                    if let Some(type_spec) = get_class_type(module, type_name) {
                        return Ok(typespec_to_type(&type_spec));
                    }
                }
                Err(CompilerError::parse_error(
                    "Unsupported type annotation",
                    ann_span,
                ))
            }
            _ => Err(CompilerError::parse_error(
                "Unsupported type annotation",
                ann_span,
            )),
        }
    }

    pub(crate) fn convert_keywords(
        &mut self,
        keywords: Vec<py::Keyword>,
    ) -> Result<(Vec<KeywordArg>, Option<ExprId>)> {
        let mut kwargs = Vec::new();
        let mut kwargs_unpack = None;
        for kw in keywords {
            let kw_span = Self::span_from(&kw);
            if let Some(arg_name) = kw.arg {
                let name = self.interner.intern(&arg_name);
                let value = self.convert_expr(kw.value)?;
                kwargs.push(KeywordArg {
                    name,
                    value,
                    span: kw_span,
                });
            } else {
                // **kwargs unpacking
                if kwargs_unpack.is_some() {
                    return Err(CompilerError::parse_error(
                        "multiple **kwargs unpacking not supported",
                        kw_span,
                    ));
                }
                kwargs_unpack = Some(self.convert_expr(kw.value)?);
            }
        }
        Ok((kwargs, kwargs_unpack))
    }

    /// Convert keyword arguments for method calls.
    /// Unlike convert_keywords(), this doesn't support **kwargs unpacking
    /// since method calls typically don't need it.
    pub(crate) fn convert_method_keywords(
        &mut self,
        keywords: Vec<py::Keyword>,
    ) -> Result<Vec<KeywordArg>> {
        let mut kwargs = Vec::new();
        for kw in keywords {
            let kw_span = Self::span_from(&kw);
            if let Some(arg_name) = kw.arg {
                let name = self.interner.intern(&arg_name);
                let value = self.convert_expr(kw.value)?;
                kwargs.push(KeywordArg {
                    name,
                    value,
                    span: kw_span,
                });
            } else {
                return Err(CompilerError::parse_error(
                    "**kwargs unpacking not supported in method calls",
                    kw_span,
                ));
            }
        }
        Ok(kwargs)
    }

    /// Convert a type expression (int, str, MyClass) for isinstance() calls.
    /// Returns TypeRef for built-in types or ClassRef for user-defined classes.
    pub(crate) fn convert_type_expr(&mut self, expr: &py::Expr) -> Result<ExprId> {
        let expr_span = Self::span_from(expr);
        match expr {
            py::Expr::Name(name) => {
                let name_str = self.interner.intern(&name.id);

                // First check if it's a user-defined class
                if let Some(&class_id) = self.symbols.class_map.get(&name_str) {
                    return Ok(self.module.exprs.alloc(Expr {
                        kind: ExprKind::ClassRef(class_id),
                        ty: None,
                        span: expr_span,
                    }));
                }

                // Then check for built-in types
                let ty = match name.id.as_str() {
                    "int" => Type::Int,
                    "float" => Type::Float,
                    "bool" => Type::Bool,
                    "str" => Type::Str,
                    "bytes" => Type::Bytes,
                    "list" => Type::List(Box::new(Type::Any)),
                    "tuple" => Type::Tuple(vec![]),
                    "dict" => Type::Dict(Box::new(Type::Any), Box::new(Type::Any)),
                    "set" => Type::Set(Box::new(Type::Any)),
                    "NoneType" => Type::None,
                    _ => {
                        return Err(CompilerError::parse_error(
                            format!("Unknown type for isinstance: {}", name.id),
                            expr_span,
                        ))
                    }
                };

                Ok(self.module.exprs.alloc(Expr {
                    kind: ExprKind::TypeRef(ty),
                    ty: None,
                    span: expr_span,
                }))
            }
            _ => Err(CompilerError::parse_error(
                "isinstance type argument must be a type name",
                expr_span,
            )),
        }
    }

    /// Convert a literal value to its erased base type for Literal[value] support.
    /// Literal[42] → int, Literal["hello"] → str, Literal[True] → bool
    fn literal_value_to_type(&self, expr: &py::Expr, span: Span) -> Result<Type> {
        match expr {
            py::Expr::Constant(c) => match &c.value {
                py::Constant::Int(_) => Ok(Type::Int),
                py::Constant::Float(_) => Ok(Type::Float),
                py::Constant::Bool(_) => Ok(Type::Bool),
                py::Constant::Str(_) => Ok(Type::Str),
                py::Constant::Bytes(_) => Ok(Type::Bytes),
                py::Constant::None => Ok(Type::None),
                _ => Err(CompilerError::parse_error(
                    "unsupported Literal value",
                    span,
                )),
            },
            // Handle negative numbers: Literal[-1] parses as UnaryOp(USub, 1)
            py::Expr::UnaryOp(unop) if matches!(unop.op, py::UnaryOp::USub) => {
                if let py::Expr::Constant(c) = &*unop.operand {
                    match &c.value {
                        py::Constant::Int(_) => Ok(Type::Int),
                        py::Constant::Float(_) => Ok(Type::Float),
                        _ => Err(CompilerError::parse_error(
                            "unsupported Literal value",
                            span,
                        )),
                    }
                } else {
                    Err(CompilerError::parse_error(
                        "Literal type parameters must be literal values",
                        span,
                    ))
                }
            }
            _ => Err(CompilerError::parse_error(
                "Literal type parameters must be literal values",
                span,
            )),
        }
    }
}
