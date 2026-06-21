//! Synthesized-class desugaring (frontend-only).
//!
//! Two source forms are rewritten into ordinary class definitions whose dunders
//! are synthesized from field names, BEFORE any downstream pass runs. There is no
//! runtime magic: the synthetic methods are indistinguishable from hand-written
//! ones, so `lower_class`, field discovery, `concrete_dunder` dispatch and the
//! differential corpus gate all work unchanged. The pattern mirrors
//! [`super::desugar_module_lambda_defs`], which likewise builds synthetic AST
//! nodes from scratch and injects them.
//!
//! 1. **`@dataclass`** (`@dataclass`/`@dataclass()`/`@dataclasses.dataclass`[`()`]
//!    — recognized syntactically by name, like `@runtime_checkable`): synthesizes
//!    the missing `__init__`/`__repr__`/`__eq__` from the field annotations
//!    (`x: T` and literal defaults `x: T = <lit>`); `ClassVar[...]` is excluded.
//!
//! 2. **`collections.namedtuple`** (`Name = namedtuple('Name', [...])`, qualified
//!    or bare — also recognized by name): rewrites the assignment into a class
//!    with synthesized `__init__`/`__repr__`/`__eq__`/`__len__`/`__getitem__`.
//!    `__len__` + `__getitem__` give positional indexing (`p[0]`) and
//!    tuple-unpacking (`a, b = p`, which the unpack lowering drives via
//!    `len`+subscript, not the iterator protocol).
//!
//! Out of scope (clean compile error where applicable):
//!   * dataclass: `@dataclass(...)` args/kwargs, `field()`/`default_factory`,
//!     mutable/non-literal defaults, inheritance, `InitVar`, `__hash__`;
//!   * namedtuple: `rename=`/`defaults=`/`module=` kwargs, the `_make`/`_asdict`/
//!     `_replace`/`_fields` API, equality against a real `tuple`, negative
//!     indices / slices, iteration (`for`/`list`/`*`) over the instance.

use super::*;

use rustpython_parser::ast::bigint::BigInt;
use rustpython_parser::ast::{
    Arg, ArgWithDefault, Arguments, ConversionFlag, ExceptHandler, ExprConstant, ExprContext,
    ExprFormattedValue, ExprJoinedStr, ExprName, ExprYield, Identifier, OptionalRange, StmtAssign,
    StmtExpr, StmtIf, StmtPass, StmtRaise, StmtReturn,
};
use rustpython_parser::text_size::TextRange;

/// One synthesized field. `annotation`/`default` are `Some` only for dataclass
/// fields; namedtuple fields carry neither (untyped, positional).
struct SynthField {
    name: Identifier,
    annotation: Option<Expr>,
    default: Option<Expr>,
}

/// Rewrite every `@dataclass` class and `namedtuple` assignment in `body`
/// (recursively, so nested forms inside functions / classes / control flow are
/// covered). Called once at the top of `lower_module_into`, BEFORE the body is
/// partitioned and scanned.
pub(crate) fn desugar_synthesized_classes(body: &mut [Stmt]) -> Result<()> {
    for stmt in body.iter_mut() {
        desugar_stmt(stmt)?;
    }
    Ok(())
}

fn desugar_stmt(stmt: &mut Stmt) -> Result<()> {
    // `Name = namedtuple(...)` → a synthesized class, replacing the assignment in
    // place. Inspected through a shared borrow so `*stmt` can be reassigned.
    let nt: Option<Result<StmtClassDef>> = match &*stmt {
        Stmt::Assign(a) => namedtuple_class_from_assign(a),
        _ => None,
    };
    if let Some(res) = nt {
        *stmt = Stmt::ClassDef(res?);
        // The synthesized class contains no further dataclasses/namedtuples.
        return Ok(());
    }

    match stmt {
        Stmt::ClassDef(cdef) => {
            // Desugar nested forms first; our synthetic methods never contain
            // further dataclasses/namedtuples, so one descent suffices.
            desugar_synthesized_classes(&mut cdef.body)?;
            if let Some(idx) = find_dataclass_deco(cdef)? {
                apply_dataclass(cdef)?;
                cdef.decorator_list.remove(idx);
            }
        }
        Stmt::FunctionDef(f) => desugar_synthesized_classes(&mut f.body)?,
        Stmt::If(s) => {
            desugar_synthesized_classes(&mut s.body)?;
            desugar_synthesized_classes(&mut s.orelse)?;
        }
        Stmt::For(s) => {
            desugar_synthesized_classes(&mut s.body)?;
            desugar_synthesized_classes(&mut s.orelse)?;
        }
        Stmt::While(s) => {
            desugar_synthesized_classes(&mut s.body)?;
            desugar_synthesized_classes(&mut s.orelse)?;
        }
        Stmt::With(s) => desugar_synthesized_classes(&mut s.body)?,
        Stmt::Try(s) => {
            desugar_synthesized_classes(&mut s.body)?;
            for h in &mut s.handlers {
                let ExceptHandler::ExceptHandler(eh) = h;
                desugar_synthesized_classes(&mut eh.body)?;
            }
            desugar_synthesized_classes(&mut s.orelse)?;
            desugar_synthesized_classes(&mut s.finalbody)?;
        }
        _ => {}
    }
    Ok(())
}

// ── @dataclass ──────────────────────────────────────────────────────────────

/// Returns `true` if `e` names the `dataclass` decorator: bare `dataclass` or
/// qualified `dataclasses.dataclass` (recognized syntactically, no import).
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

/// Classify a decorator: `None` (not dataclass), `Some(Ok)` (supported bare
/// form), `Some(Err)` (an out-of-scope argument-carrying `@dataclass(...)`).
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

/// Find the `@dataclass` decorator's index, propagating the out-of-scope error.
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

/// `ClassVar[...]` / bare / qualified `typing.ClassVar` — excluded from fields.
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

/// Validate that a dataclass field default is an accepted literal (int/float/str/
/// bool/None, incl. a unary-minus numeric literal) and return a clone.
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

/// `true` if the class body already defines a method named `name`.
fn body_defines_method(body: &[Stmt], name: &str) -> bool {
    body.iter()
        .any(|s| matches!(s, Stmt::FunctionDef(f) if f.name.as_str() == name))
}

/// Transform one `@dataclass` class: collect fields, strip literal defaults off
/// the field annotations, and append the missing dunders.
fn apply_dataclass(cdef: &mut StmtClassDef) -> Result<()> {
    let class_name = cdef.name.as_str().to_string();
    let range = cdef.range();

    let mut fields: Vec<SynthField> = Vec::new();
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
        fields.push(SynthField {
            name: target.id.clone(),
            annotation: Some(a.annotation.as_ref().clone()),
            default,
        });
    }

    // Strip defaults so a defaulted field (`x: int = 0`) becomes a plain
    // instance-field hint (`x: int`); the default now lives only on `__init__`.
    for &i in &strip_indices {
        if let Stmt::AnnAssign(a) = &mut cdef.body[i] {
            a.value = None;
        }
    }

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

// ── collections.namedtuple ──────────────────────────────────────────────────

/// If `value` is a `namedtuple(...)` / `collections.namedtuple(...)` call, return
/// it (recognized syntactically by name, like `@dataclass`).
fn namedtuple_call(value: &Expr) -> Option<&ExprCall> {
    let Expr::Call(c) = value else { return None };
    let is_nt = match c.func.as_ref() {
        Expr::Name(n) => n.id.as_str() == "namedtuple",
        Expr::Attribute(a) => {
            a.attr.as_str() == "namedtuple"
                && matches!(a.value.as_ref(), Expr::Name(n) if n.id.as_str() == "collections")
        }
        _ => false,
    };
    is_nt.then_some(c)
}

/// `Name = namedtuple('Type', [...])` → a synthesized class. `None` when the
/// assignment is not a namedtuple factory; `Some(Err)` for a malformed one.
fn namedtuple_class_from_assign(a: &StmtAssign) -> Option<Result<StmtClassDef>> {
    let call = namedtuple_call(a.value.as_ref())?;
    // It IS a namedtuple call — any shape problem is now an error, not a skip.
    if a.targets.len() != 1 {
        return Some(Err(parse_error(
            "a namedtuple must be assigned to a single name",
            to_span(a.range()),
        )));
    }
    let Expr::Name(target) = &a.targets[0] else {
        return Some(Err(parse_error(
            "a namedtuple must be assigned to a single name",
            to_span(a.range()),
        )));
    };
    Some(build_namedtuple_class(target.id.as_str(), call, a.range()))
}

/// Build the synthesized class. The class is named after the assigned variable
/// (so the binding and `isinstance`/construction resolve), while the typename
/// literal drives `__repr__` — CPython's namedtuple repr always uses the
/// typename, never `__qualname__`, so the hard-coded literal is byte-exact.
fn build_namedtuple_class(
    var_name: &str,
    call: &ExprCall,
    range: TextRange,
) -> Result<StmtClassDef> {
    let span = to_span(range);
    if !call.keywords.is_empty() {
        return Err(parse_error(
            "namedtuple keyword arguments (rename=/defaults=/module=) are out of scope",
            span,
        ));
    }
    if call.args.len() != 2 {
        return Err(parse_error(
            "namedtuple(typename, field_names) requires exactly two positional arguments",
            span,
        ));
    }
    let Expr::Constant(tc) = &call.args[0] else {
        return Err(parse_error("namedtuple typename must be a string literal", span));
    };
    let Constant::Str(typename) = &tc.value else {
        return Err(parse_error("namedtuple typename must be a string literal", span));
    };
    let field_names = parse_field_names(&call.args[1], span)?;
    let fields: Vec<SynthField> = field_names
        .iter()
        .map(|n| SynthField {
            name: Identifier::new(n.as_str()),
            annotation: None,
            default: None,
        })
        .collect();
    let body = vec![
        synth_init(&fields, range),
        synth_repr(typename, &fields, range),
        synth_eq(var_name, &fields, range),
        synth_len(&fields, range),
        synth_getitem(&fields, range),
        synth_iter(&fields, range),
        synth_contains(&fields, range),
    ];
    Ok(StmtClassDef {
        range,
        name: Identifier::new(var_name),
        bases: vec![],
        keywords: vec![],
        body,
        decorator_list: vec![],
        type_params: vec![],
    })
}

/// Parse the `field_names` argument: a list/tuple of string literals, or a single
/// string of space/comma-separated names (CPython's `str.replace(',', ' ').split()`).
fn parse_field_names(spec: &Expr, span: Span) -> Result<Vec<String>> {
    let names: Vec<String> = match spec {
        Expr::List(l) => collect_str_names(&l.elts, span)?,
        Expr::Tuple(t) => collect_str_names(&t.elts, span)?,
        Expr::Constant(c) => {
            let Constant::Str(s) = &c.value else {
                return Err(field_names_shape_error(span));
            };
            s.replace(',', " ")
                .split_whitespace()
                .map(|x| x.to_string())
                .collect()
        }
        _ => return Err(field_names_shape_error(span)),
    };
    let mut seen: HashSet<&str> = HashSet::new();
    for n in &names {
        if !is_field_identifier(n) {
            return Err(parse_error(
                format!("invalid namedtuple field name `{n}`"),
                span,
            ));
        }
        if n.starts_with('_') {
            return Err(parse_error(
                format!(
                    "namedtuple field name `{n}` must not start with an underscore \
                     (rename= is out of scope)"
                ),
                span,
            ));
        }
        if !seen.insert(n.as_str()) {
            return Err(parse_error(
                format!("duplicate namedtuple field name `{n}`"),
                span,
            ));
        }
    }
    if names.is_empty() {
        // A zero-field namedtuple is degenerate (no positional fields); reject so
        // the synthesized `__getitem__`/unpack paths stay well-defined.
        return Err(parse_error(
            "a namedtuple needs at least one field name",
            span,
        ));
    }
    Ok(names)
}

fn collect_str_names(elts: &[Expr], span: Span) -> Result<Vec<String>> {
    elts.iter()
        .map(|e| {
            let Expr::Constant(c) = e else {
                return Err(field_names_shape_error(span));
            };
            let Constant::Str(s) = &c.value else {
                return Err(field_names_shape_error(span));
            };
            Ok(s.clone())
        })
        .collect()
}

fn field_names_shape_error(span: Span) -> CompilerError {
    parse_error(
        "namedtuple field_names must be a list/tuple of string literals or a single \
         space/comma-separated string literal",
        span,
    )
}

/// A valid Python identifier (first char alpha/`_`, rest alphanumeric/`_`).
fn is_field_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

// ── AST building blocks ─────────────────────────────────────────────────────
// All synthetic nodes reuse the source range, so diagnostics point at the form.

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

fn mk_int(n: i64, range: TextRange) -> Expr {
    mk_const(Constant::Int(BigInt::from(n)), range)
}

fn mk_self_attr(field: &str, range: TextRange) -> Expr {
    mk_attr(
        mk_name("self", ExprContext::Load, range),
        field,
        ExprContext::Load,
        range,
    )
}

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

fn mk_return(value: Expr, range: TextRange) -> Stmt {
    Stmt::Return(StmtReturn {
        range,
        value: Some(Box::new(value)),
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

/// `def __init__(self, f1[: T1][= d1], …): self.f1 = f1; …`. Field annotations /
/// defaults move onto the parameters (dataclass); namedtuple params are untyped.
fn synth_init(fields: &[SynthField], range: TextRange) -> Stmt {
    let mut args = vec![mk_arg("self", None, None, range)];
    for f in fields {
        args.push(mk_arg(
            f.name.as_str(),
            f.annotation.clone(),
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

/// `def __repr__(self) -> str: return f"Name(f1={self.f1!r}, …)"`. `repr_name` is
/// the dataclass class name or the namedtuple typename; `!r` gives byte-exact
/// CPython reprs. A zero-field class yields `f"Name()"`.
fn synth_repr(repr_name: &str, fields: &[SynthField], range: TextRange) -> Stmt {
    let mut values: Vec<Expr> = vec![mk_const(Constant::Str(format!("{repr_name}(")), range)];
    for (i, f) in fields.iter().enumerate() {
        let sep = if i == 0 { "" } else { ", " };
        values.push(mk_const(
            Constant::Str(format!("{sep}{}=", f.name.as_str())),
            range,
        ));
        values.push(Expr::FormattedValue(ExprFormattedValue {
            range,
            value: Box::new(mk_self_attr(f.name.as_str(), range)),
            conversion: ConversionFlag::Repr,
            format_spec: None,
        }));
    }
    values.push(mk_const(Constant::Str(")".to_string()), range));
    let joined = Expr::JoinedStr(ExprJoinedStr { range, values });
    let args = mk_arguments(vec![mk_arg("self", None, None, range)], range);
    mk_funcdef(
        "__repr__",
        args,
        vec![mk_return(joined, range)],
        Some(mk_name("str", ExprContext::Load, range)),
        range,
    )
}

/// `def __eq__(self, other) -> bool:` with the CPython field-compare idiom:
/// `if isinstance(other, Cls): return self.f1 == other.f1 and …; return False`.
/// A zero-field class returns `True` inside the guard.
fn synth_eq(class_name: &str, fields: &[SynthField], range: TextRange) -> Stmt {
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
                    left: Box::new(mk_self_attr(f.name.as_str(), range)),
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
        body: vec![mk_return(guarded_value, range)],
        orelse: vec![],
    });
    let fallback = mk_return(mk_const(Constant::Bool(false), range), range);
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

/// `def __len__(self) -> int: return N` (namedtuple).
fn synth_len(fields: &[SynthField], range: TextRange) -> Stmt {
    let args = mk_arguments(vec![mk_arg("self", None, None, range)], range);
    mk_funcdef(
        "__len__",
        args,
        vec![mk_return(mk_int(fields.len() as i64, range), range)],
        Some(mk_name("int", ExprContext::Load, range)),
        range,
    )
}

/// `def __getitem__(self, index): if index == 0: return self.f0; …; raise
/// IndexError("tuple index out of range")` (namedtuple). Drives both positional
/// indexing and tuple-unpacking. Non-negative indices only (negatives/slices
/// fall through to `IndexError`).
fn synth_getitem(fields: &[SynthField], range: TextRange) -> Stmt {
    let mut body: Vec<Stmt> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let test = Expr::Compare(ExprCompare {
                range,
                left: Box::new(mk_name("index", ExprContext::Load, range)),
                ops: vec![PyCmpOp::Eq],
                comparators: vec![mk_int(i as i64, range)],
            });
            Stmt::If(StmtIf {
                range,
                test: Box::new(test),
                body: vec![mk_return(mk_self_attr(f.name.as_str(), range), range)],
                orelse: vec![],
            })
        })
        .collect();
    let raise = Stmt::Raise(StmtRaise {
        range,
        exc: Some(Box::new(Expr::Call(ExprCall {
            range,
            func: Box::new(mk_name("IndexError", ExprContext::Load, range)),
            args: vec![mk_const(
                Constant::Str("tuple index out of range".to_string()),
                range,
            )],
            keywords: vec![],
        }))),
        cause: None,
    });
    body.push(raise);
    let args = mk_arguments(
        vec![
            mk_arg("self", None, None, range),
            mk_arg("index", None, None, range),
        ],
        range,
    );
    mk_funcdef("__getitem__", args, body, None, range)
}

/// `def __iter__(self): yield self.f0; yield self.f1; …` (namedtuple). A
/// generator method, so iteration (`for v in p` / `list(p)` / `*p`) yields the
/// fields in order. Always at least one field (a zero-field namedtuple is
/// rejected), so the body is a real generator.
fn synth_iter(fields: &[SynthField], range: TextRange) -> Stmt {
    let body: Vec<Stmt> = fields
        .iter()
        .map(|f| {
            Stmt::Expr(StmtExpr {
                range,
                value: Box::new(Expr::Yield(ExprYield {
                    range,
                    value: Some(Box::new(mk_self_attr(f.name.as_str(), range))),
                })),
            })
        })
        .collect();
    let args = mk_arguments(vec![mk_arg("self", None, None, range)], range);
    mk_funcdef("__iter__", args, body, None, range)
}

/// `def __contains__(self, value) -> bool: return value == self.f0 or …`
/// (namedtuple). CPython's `in` on a tuple is value-equality over the elements;
/// synthesizing `__contains__` gives `x in p` directly (the compiler's `in` does
/// not fall back to `__iter__`).
fn synth_contains(fields: &[SynthField], range: TextRange) -> Stmt {
    let mut comparisons: Vec<Expr> = fields
        .iter()
        .map(|f| {
            Expr::Compare(ExprCompare {
                range,
                left: Box::new(mk_name("value", ExprContext::Load, range)),
                ops: vec![PyCmpOp::Eq],
                comparators: vec![mk_self_attr(f.name.as_str(), range)],
            })
        })
        .collect();
    let value = if comparisons.len() == 1 {
        comparisons.pop().expect("one comparison")
    } else {
        Expr::BoolOp(ExprBoolOp {
            range,
            op: PyBoolOp::Or,
            values: comparisons,
        })
    };
    let args = mk_arguments(
        vec![
            mk_arg("self", None, None, range),
            mk_arg("value", None, None, range),
        ],
        range,
    );
    mk_funcdef(
        "__contains__",
        args,
        vec![mk_return(value, range)],
        Some(mk_name("bool", ExprContext::Load, range)),
        range,
    )
}
