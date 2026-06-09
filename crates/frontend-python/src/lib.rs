//! # frontend-python — parse + desugar → HIR
//!
//! Parses with `rustpython-parser` (`Mode::Module`) and lowers top-level
//! statements into the synthetic `__main__` [`HirFunction`]. The literal's type
//! is assigned here (a `str` literal is `SemTy::Str`); every other node is left
//! `SemTy::Dyn` for `pyaot-typeck` to refine.
//!
//! ## Phase 1 scope
//!
//! Only what `print("hello")` needs: expression statements, calls, name
//! references, and `str` literals. Any other AST node kind returns a
//! [`CompilerError::parse_error`] — that error *is* the phase allowlist (a
//! program that uses an unsupported construct simply fails to compile). No
//! desugaring (generators, comprehensions, `with`, `match`, decorators, walrus,
//! PEP 563) yet — all reserved for later phases.

#![forbid(unsafe_code)]

use la_arena::{Arena, Idx};
use rustpython_parser::ast::{Constant, Expr, Mod, Ranged, Stmt};
use rustpython_parser::text_size::TextRange;
use rustpython_parser::{parse as rustpython_parse, Mode};

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{
    HirBlock, HirExpr, HirExprKind, HirFunction, HirModule, HirStmt, HirTerminator, SymbolRef,
};
use pyaot_types::SemTy;
use pyaot_utils::{FuncId, Span, StringInterner};

/// Parse Python source into an [`HirModule`] whose `__main__` holds the
/// top-level statements. Interns every literal/identifier through `interner`.
pub fn parse(src: &str, interner: &mut StringInterner) -> Result<HirModule> {
    let parsed = rustpython_parse(src, Mode::Module, "<input>").map_err(|e| {
        let off = e.offset.to_u32();
        CompilerError::parse_error(e.to_string(), Span::new(off, off))
    })?;

    let body = match parsed {
        Mod::Module(m) => m.body,
        _ => return Err(CompilerError::parse_error("expected a module", Span::dummy())),
    };

    let mut exprs: Arena<HirExpr> = Arena::new();
    let mut stmts: Vec<HirStmt> = Vec::new();
    for stmt in &body {
        lower_stmt(stmt, &mut exprs, &mut stmts, interner)?;
    }

    let mut blocks: Arena<HirBlock> = Arena::new();
    // The module body falls off the end with an implicit `return None`.
    let entry = blocks.alloc(HirBlock {
        stmts,
        term: HirTerminator::Return(None),
    });

    let main_fn = HirFunction {
        name: interner.intern("__main__"),
        params: Vec::new(),
        ret_ty: SemTy::NoneTy,
        blocks,
        entry,
        exprs,
    };

    Ok(HirModule {
        functions: vec![main_fn],
        main: FuncId::new(0),
    })
}

fn lower_stmt(
    stmt: &Stmt,
    exprs: &mut Arena<HirExpr>,
    out: &mut Vec<HirStmt>,
    interner: &mut StringInterner,
) -> Result<()> {
    match stmt {
        Stmt::Expr(stmt_expr) => {
            let idx = lower_expr(stmt_expr.value.as_ref(), exprs, interner)?;
            out.push(HirStmt::Expr(idx));
            Ok(())
        }
        other => Err(CompilerError::parse_error(
            "unsupported statement for Phase 1 (only bare expressions are supported)",
            to_span(other.range()),
        )),
    }
}

fn lower_expr(
    expr: &Expr,
    exprs: &mut Arena<HirExpr>,
    interner: &mut StringInterner,
) -> Result<Idx<HirExpr>> {
    let span = to_span(expr.range());
    let (kind, ty) = match expr {
        Expr::Constant(c) => match &c.value {
            Constant::Str(s) => (HirExprKind::StrLit(interner.intern(s)), SemTy::Str),
            _ => {
                return Err(CompilerError::parse_error(
                    "unsupported literal for Phase 1 (only str literals are supported)",
                    span,
                ))
            }
        },
        Expr::Name(n) => (
            HirExprKind::Name(SymbolRef::Unresolved(interner.intern(n.id.as_str()))),
            SemTy::Dyn,
        ),
        Expr::Call(call) => {
            if !call.keywords.is_empty() {
                return Err(CompilerError::parse_error(
                    "keyword arguments are not supported in Phase 1 (sep=/end= are Phase 2)",
                    span,
                ));
            }
            let callee = lower_expr(call.func.as_ref(), exprs, interner)?;
            let mut args = Vec::with_capacity(call.args.len());
            for arg in &call.args {
                args.push(lower_expr(arg, exprs, interner)?);
            }
            (HirExprKind::Call { callee, args }, SemTy::Dyn)
        }
        other => {
            return Err(CompilerError::parse_error(
                "unsupported expression for Phase 1",
                to_span(other.range()),
            ))
        }
    };
    Ok(exprs.alloc(HirExpr { kind, ty, span }))
}

fn to_span(range: TextRange) -> Span {
    Span::new(range.start().to_u32(), range.end().to_u32())
}
