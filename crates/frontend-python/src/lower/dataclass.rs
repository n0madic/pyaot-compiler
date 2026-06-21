//! `@dataclass` desugaring (frontend-only).
//!
//! Synthesizes the `__init__`/`__repr__`/`__eq__` dunders of a
//! `@dataclasses.dataclass`-decorated class from its field annotations, as plain
//! AST methods injected into the class body. There is no runtime magic: once the
//! synthetic methods are in place they are indistinguishable from hand-written
//! ones, so the whole downstream pipeline (`lower_class`, field-annotation
//! handling, `concrete_dunder` dispatch, the differential corpus gate) works
//! unchanged. The pattern mirrors [`super::desugar_module_lambda_defs`], which
//! likewise builds synthetic `StmtFunctionDef`s from scratch and injects them.
//!
//! Scope (everything else is a clean compile error or documented divergence):
//!   * decorator forms `@dataclass`, `@dataclass()`, `@dataclasses.dataclass`,
//!     `@dataclasses.dataclass()` — recognized syntactically by name, like
//!     `@runtime_checkable`;
//!   * annotated fields `x: T` and literal defaults `x: T = <lit>` (int/float/
//!     str/bool/None, incl. a unary-minus numeric literal);
//!   * `ClassVar[...]` is excluded from the fields (stays a class attribute);
//!   * only *missing* dunders are synthesized (a user `__init__`/`__repr__`/
//!     `__eq__` is preserved, matching CPython).
//!
//! Out of scope (rejected with a clear error where applicable): `@dataclass(...)`
//! with any args/kwargs (`frozen=`/`order=`/…), `field()`/`default_factory`,
//! non-literal/mutable defaults, dataclass inheritance, `InitVar`, `__hash__`.

use super::*;

use rustpython_parser::ast::{
    Arg, ArgWithDefault, Arguments, ConversionFlag, ExceptHandler, ExprConstant, ExprContext,
    ExprFormattedValue, ExprJoinedStr, ExprName, Identifier, OptionalRange, StmtAssign, StmtIf,
    StmtPass, StmtReturn,
};
use rustpython_parser::text_size::TextRange;

/// One dataclass field collected from the class body, in source order.
struct DcField {
    /// Field name (the `AnnAssign` target).
    name: Identifier,
    /// The field's type annotation (moved onto the synthesized `__init__` param).
    annotation: Expr,
    /// A validated literal default, moved onto the `__init__` param.
    default: Option<Expr>,
}

/// Rewrite every `@dataclass`-decorated class in `body` (recursively, so nested
/// dataclasses inside functions / classes / control flow are covered) by
/// synthesizing its missing `__init__`/`__repr__`/`__eq__` dunders from the field
/// annotations. Called once at the top of `lower_module_into`, BEFORE the body is
/// partitioned and scanned, so the synthetic methods are present everywhere the
/// class is later inspected.
pub(crate) fn desugar_dataclasses(body: &mut [Stmt]) -> Result<()> {
    for stmt in body.iter_mut() {
        desugar_stmt(stmt)?;
    }
    Ok(())
}

/// Recurse into one statement's nested bodies, then (for a class) apply the
/// dataclass transform if it carries a `@dataclass` decorator.
fn desugar_stmt(stmt: &mut Stmt) -> Result<()> {
    match stmt {
        Stmt::ClassDef(cdef) => {
            // Desugar nested dataclasses first; the methods we synthesize below
            // never contain further dataclasses, so one descent suffices.
            desugar_dataclasses(&mut cdef.body)?;
            if let Some(idx) = find_dataclass_deco(cdef)? {
                apply_dataclass(cdef)?;
                cdef.decorator_list.remove(idx);
            }
        }
        Stmt::FunctionDef(f) => desugar_dataclasses(&mut f.body)?,
        Stmt::If(s) => {
            desugar_dataclasses(&mut s.body)?;
            desugar_dataclasses(&mut s.orelse)?;
        }
        Stmt::For(s) => {
            desugar_dataclasses(&mut s.body)?;
            desugar_dataclasses(&mut s.orelse)?;
        }
        Stmt::While(s) => {
            desugar_dataclasses(&mut s.body)?;
            desugar_dataclasses(&mut s.orelse)?;
        }
        Stmt::With(s) => desugar_dataclasses(&mut s.body)?,
        Stmt::Try(s) => {
            desugar_dataclasses(&mut s.body)?;
            for h in &mut s.handlers {
                let ExceptHandler::ExceptHandler(eh) = h;
                desugar_dataclasses(&mut eh.body)?;
            }
            desugar_dataclasses(&mut s.orelse)?;
            desugar_dataclasses(&mut s.finalbody)?;
        }
        _ => {}
    }
    Ok(())
}

/// Returns `true` if `e` names the `dataclass` decorator: a bare `dataclass` or a
/// qualified `dataclasses.dataclass`. Recognized purely syntactically (no import
/// binding), like `@runtime_checkable`.
fn expr_is_dataclass_name(e: &Expr) -> bool {
    match e {
        Expr::Name(n) => n.id.as_str() == "dataclass",
        Expr::Attribute(a) => {
            a.attr.as_str() == "dataclass"
                && matches!(a.value.as_ref(), Expr::Name(n) if n.id.as_str() == "dataclasses")
        }
        _ => false,
    }
}

/// Classify a decorator expression:
///   * `None` — not a dataclass decorator;
///   * `Some(Ok(()))` — a supported bare `@dataclass` / `@dataclass()` form;
///   * `Some(Err(_))` — a dataclass decorator carrying arguments/keywords
///     (`frozen=`, `order=`, …), which is out of scope.
fn classify_dataclass_deco(deco: &Expr) -> Option<Result<()>> {
    if expr_is_dataclass_name(deco) {
        return Some(Ok(()));
    }
    if let Expr::Call(c) = deco {
        if expr_is_dataclass_name(c.func.as_ref()) {
            if c.args.is_empty() && c.keywords.is_empty() {
                return Some(Ok(()));
            }
            return Some(Err(parse_error(
                "@dataclass(...) with arguments is out of scope (frozen=/order=/eq= and \
                 other keywords, field() options, etc.); only bare @dataclass / \
                 @dataclass() is supported",
                to_span(deco.range()),
            )));
        }
    }
    None
}

/// Find the `@dataclass` decorator on a class, returning its index in
/// `decorator_list`. Propagates the out-of-scope error for an argument-carrying
/// form so the class is not silently mis-synthesized.
fn find_dataclass_deco(cdef: &StmtClassDef) -> Result<Option<usize>> {
    let mut found = None;
    for (i, deco) in cdef.decorator_list.iter().enumerate() {
        if let Some(res) = classify_dataclass_deco(deco) {
            res?;
            found = Some(i);
        }
    }
    Ok(found)
}

/// `ClassVar[...]` / bare `ClassVar` (possibly qualified `typing.ClassVar`) — a
/// class variable, excluded from the dataclass fields (CPython semantics).
fn is_classvar(ann: &Expr) -> bool {
    let head = match ann {
        Expr::Subscript(s) => s.value.as_ref(),
        other => other,
    };
    match head {
        Expr::Name(n) => n.id.as_str() == "ClassVar",
        Expr::Attribute(a) => a.attr.as_str() == "ClassVar",
        _ => false,
    }
}

/// Validate that a field default is a literal we accept (int/float/str/bool/None,
/// including a unary-minus numeric literal) and return a clone of it. Anything
/// else — `field()`, `default_factory`, a mutable `[]`/`{}`, a name, a call — is
/// out of scope.
fn validate_literal_default(v: &Expr) -> Result<Expr> {
    let is_scalar = |c: &ExprConstant| {
        matches!(
            c.value,
            Constant::Int(_)
                | Constant::Float(_)
                | Constant::Str(_)
                | Constant::Bool(_)
                | Constant::None
        )
    };
    match v {
        Expr::Constant(c) if is_scalar(c) => Ok(v.clone()),
        Expr::UnaryOp(u) if matches!(u.op, PyUnaryOp::USub) => {
            if matches!(u.operand.as_ref(), Expr::Constant(c)
                if matches!(c.value, Constant::Int(_) | Constant::Float(_)))
            {
                Ok(v.clone())
            } else {
                Err(literal_default_error(v))
            }
        }
        _ => Err(literal_default_error(v)),
    }
}

fn literal_default_error(v: &Expr) -> CompilerError {
    parse_error(
        "dataclass field default must be a literal (int/float/str/bool/None); \
         field()/default_factory and mutable defaults ([]/{}) are out of scope",
        to_span(v.range()),
    )
}

/// `true` if the class body already defines a method named `name` (so the
/// dataclass transform leaves the user's version intact).
fn body_defines_method(body: &[Stmt], name: &str) -> bool {
    body.iter()
        .any(|s| matches!(s, Stmt::FunctionDef(f) if f.name.as_str() == name))
}

/// Transform one `@dataclass` class: collect its fields, strip literal defaults
/// off the field annotations (so `lower_class` treats them as instance fields,
/// not class attributes), and append the missing dunders.
fn apply_dataclass(cdef: &mut StmtClassDef) -> Result<()> {
    let class_name = cdef.name.as_str().to_string();
    let range = cdef.range();

    // Pass 1: collect fields in source order; remember which `AnnAssign`s carry a
    // default that must be stripped.
    let mut fields: Vec<DcField> = Vec::new();
    let mut strip_indices: Vec<usize> = Vec::new();
    let mut seen_default = false;
    for (i, stmt) in cdef.body.iter().enumerate() {
        let Stmt::AnnAssign(a) = stmt else { continue };
        let Expr::Name(target) = a.target.as_ref() else {
            continue;
        };
        if is_classvar(a.annotation.as_ref()) {
            continue;
        }
        let default = match &a.value {
            Some(v) => {
                let lit = validate_literal_default(v.as_ref())?;
                seen_default = true;
                strip_indices.push(i);
                Some(lit)
            }
            None => {
                if seen_default {
                    return Err(parse_error(
                        format!(
                            "non-default dataclass field `{}` follows a field with a \
                             default in `{class_name}`",
                            target.id.as_str()
                        ),
                        to_span(a.range()),
                    ));
                }
                None
            }
        };
        fields.push(DcField {
            name: target.id.clone(),
            annotation: a.annotation.as_ref().clone(),
            default,
        });
    }

    // Pass 2: strip the defaults off the field annotations so a defaulted field
    // (`x: int = 0`) becomes a plain instance-field hint (`x: int`); the default
    // now lives only on the synthesized `__init__` parameter.
    for &i in &strip_indices {
        if let Stmt::AnnAssign(a) = &mut cdef.body[i] {
            a.value = None;
        }
    }

    // Synthesize only the dunders the user did not write (CPython semantics).
    if !body_defines_method(&cdef.body, "__init__") {
        cdef.body.push(synth_init(&fields, range));
    }
    if !body_defines_method(&cdef.body, "__repr__") {
        cdef.body.push(synth_repr(&class_name, &fields, range));
    }
    if !body_defines_method(&cdef.body, "__eq__") {
        cdef.body.push(synth_eq(&class_name, &fields, range));
    }
    Ok(())
}

// ── AST building blocks ────────────────────────────────────────────────────
// All synthetic nodes reuse the class's source range (like the lambda desugar
// reuses the assignment's range), so diagnostics point at the class.

fn mk_name(id: &str, ctx: ExprContext, range: TextRange) -> Expr {
    Expr::Name(ExprName {
        range,
        id: Identifier::new(id),
        ctx,
    })
}

fn mk_attr(value: Expr, attr: &str, ctx: ExprContext, range: TextRange) -> Expr {
    Expr::Attribute(ExprAttribute {
        range,
        value: Box::new(value),
        attr: Identifier::new(attr),
        ctx,
    })
}

fn mk_const(value: Constant, range: TextRange) -> Expr {
    Expr::Constant(ExprConstant {
        range,
        value,
        kind: None,
    })
}

/// One `def` parameter (`name[: ann][= default]`) for a synthesized method.
fn mk_arg(
    name: &str,
    annotation: Option<Expr>,
    default: Option<Expr>,
    range: TextRange,
) -> ArgWithDefault {
    ArgWithDefault {
        range: OptionalRange::from(range),
        def: Arg {
            range,
            arg: Identifier::new(name),
            annotation: annotation.map(Box::new),
            type_comment: None,
        },
        default: default.map(Box::new),
    }
}

fn mk_arguments(args: Vec<ArgWithDefault>, range: TextRange) -> Box<Arguments> {
    Box::new(Arguments {
        range: OptionalRange::from(range),
        posonlyargs: vec![],
        args,
        vararg: None,
        kwonlyargs: vec![],
        kwarg: None,
    })
}

fn mk_funcdef(
    name: &str,
    args: Box<Arguments>,
    body: Vec<Stmt>,
    returns: Option<Expr>,
    range: TextRange,
) -> Stmt {
    Stmt::FunctionDef(StmtFunctionDef {
        range,
        name: Identifier::new(name),
        args,
        body,
        decorator_list: vec![],
        returns: returns.map(Box::new),
        type_comment: None,
        type_params: vec![],
    })
}

/// `def __init__(self, f1: T1, f2: T2 = d2, …): self.f1 = f1; …`. The field
/// annotations move onto the parameters (the existing typed-param path), and the
/// body just stores each parameter into its field.
fn synth_init(fields: &[DcField], range: TextRange) -> Stmt {
    let mut args = vec![mk_arg("self", None, None, range)];
    for f in fields {
        args.push(mk_arg(
            f.name.as_str(),
            Some(f.annotation.clone()),
            f.default.clone(),
            range,
        ));
    }
    let mut body: Vec<Stmt> = fields
        .iter()
        .map(|f| {
            let target = mk_attr(
                mk_name("self", ExprContext::Load, range),
                f.name.as_str(),
                ExprContext::Store,
                range,
            );
            let value = mk_name(f.name.as_str(), ExprContext::Load, range);
            Stmt::Assign(StmtAssign {
                range,
                targets: vec![target],
                value: Box::new(value),
                type_comment: None,
            })
        })
        .collect();
    if body.is_empty() {
        body.push(Stmt::Pass(StmtPass { range }));
    }
    mk_funcdef("__init__", mk_arguments(args, range), body, None, range)
}

/// `def __repr__(self) -> str: return f"Cls(f1={self.f1!r}, …)"`. The `!r`
/// conversion gives byte-exact CPython reprs (`Cls(x=1, name='hi')`). A zero-field
/// class yields `f"Cls()"`.
fn synth_repr(class_name: &str, fields: &[DcField], range: TextRange) -> Stmt {
    let mut values: Vec<Expr> = vec![mk_const(Constant::Str(format!("{class_name}(")), range)];
    for (i, f) in fields.iter().enumerate() {
        let sep = if i == 0 { "" } else { ", " };
        values.push(mk_const(
            Constant::Str(format!("{sep}{}=", f.name.as_str())),
            range,
        ));
        let attr = mk_attr(
            mk_name("self", ExprContext::Load, range),
            f.name.as_str(),
            ExprContext::Load,
            range,
        );
        values.push(Expr::FormattedValue(ExprFormattedValue {
            range,
            value: Box::new(attr),
            conversion: ConversionFlag::Repr,
            format_spec: None,
        }));
    }
    values.push(mk_const(Constant::Str(")".to_string()), range));
    let joined = Expr::JoinedStr(ExprJoinedStr { range, values });
    let body = vec![Stmt::Return(StmtReturn {
        range,
        value: Some(Box::new(joined)),
    })];
    let args = mk_arguments(vec![mk_arg("self", None, None, range)], range);
    mk_funcdef(
        "__repr__",
        args,
        body,
        Some(mk_name("str", ExprContext::Load, range)),
        range,
    )
}

/// `def __eq__(self, other) -> bool:` with the CPython field-compare idiom:
/// `if isinstance(other, Cls): return self.f1 == other.f1 and …; return False`.
/// `other` is unannotated (Dyn); the `isinstance` guard makes the by-name
/// `other.fi` access (FIELD_NAME_REGISTRY) always hit a `Cls` at runtime. A
/// zero-field class returns `True` inside the guard.
fn synth_eq(class_name: &str, fields: &[DcField], range: TextRange) -> Stmt {
    let test = Expr::Call(ExprCall {
        range,
        func: Box::new(mk_name("isinstance", ExprContext::Load, range)),
        args: vec![
            mk_name("other", ExprContext::Load, range),
            mk_name(class_name, ExprContext::Load, range),
        ],
        keywords: vec![],
    });
    let guarded_value = if fields.is_empty() {
        mk_const(Constant::Bool(true), range)
    } else {
        let mut comparisons: Vec<Expr> = fields
            .iter()
            .map(|f| {
                Expr::Compare(ExprCompare {
                    range,
                    left: Box::new(mk_attr(
                        mk_name("self", ExprContext::Load, range),
                        f.name.as_str(),
                        ExprContext::Load,
                        range,
                    )),
                    ops: vec![PyCmpOp::Eq],
                    comparators: vec![mk_attr(
                        mk_name("other", ExprContext::Load, range),
                        f.name.as_str(),
                        ExprContext::Load,
                        range,
                    )],
                })
            })
            .collect();
        if comparisons.len() == 1 {
            comparisons.pop().expect("one comparison")
        } else {
            Expr::BoolOp(ExprBoolOp {
                range,
                op: PyBoolOp::And,
                values: comparisons,
            })
        }
    };
    let guard = Stmt::If(StmtIf {
        range,
        test: Box::new(test),
        body: vec![Stmt::Return(StmtReturn {
            range,
            value: Some(Box::new(guarded_value)),
        })],
        orelse: vec![],
    });
    let fallback = Stmt::Return(StmtReturn {
        range,
        value: Some(Box::new(mk_const(Constant::Bool(false), range))),
    });
    let args = mk_arguments(
        vec![
            mk_arg("self", None, None, range),
            mk_arg("other", None, None, range),
        ],
        range,
    );
    mk_funcdef(
        "__eq__",
        args,
        vec![guard, fallback],
        Some(mk_name("bool", ExprContext::Load, range)),
        range,
    )
}
