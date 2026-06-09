//! # lowering — HIR → MIR (mechanical)
//!
//! Types are already solved by `pyaot_typeck`, so lowering is purely mechanical:
//! translate HIR nodes to MIR, asking [`pyaot_types::repr_of`] for each slot's
//! representation. Two things deliberately do NOT exist here:
//!
//! * **No type inference** — it finished in `typeck`.
//! * **No ABI-repair stage** — a function's ABI is a deterministic function of
//!   its parameters' `Repr`, so call sites are correct by construction.
//!
//! [`legalize`] is the SINGLE place coercions are inserted. One rule —
//! `coerce(have, need)` — subsumes every per-case boxing decision (PITFALLS A5).
//!
//! ## Phase 1 scope
//!
//! Lowers `print(<str literal>, …)`. For each argument: a `Const{Str}` into a
//! `Heap(Str)` local, a `Coerce(Heap(Str) → Tagged)` through [`legalize::coerce`],
//! then a `Print{StrObj}`; a trailing `Print{Newline}` ends the call; `__main__`
//! terminates with `Return(None)`.
//!
//! The `Coerce` is the single-coercion seam exercised for real: `Heap(Str)` *is*
//! a tagged `Value` at the bit level, so the coercion is a runtime no-op the
//! verifier still tracks — and we deliberately route through `legalize` rather
//! than faking it. We then call `rt_print_str_obj` (correct `str()` output:
//! `hello`, no quotes), never `rt_print_obj` (which would print `'hello'`).

#![forbid(unsafe_code)]

pub mod legalize;

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{
    HirExpr, HirExprKind, HirFunction, HirModule, HirStmt, HirTerminator, ResolveResult, Symbol,
    SymbolRef,
};
use pyaot_mir::{
    Const, LocalDecl, MirBlock, MirFunction, MirInst, MirProgram, MirTerminator, Operand, PrintKind,
    StrPool,
};
use pyaot_types::{repr_of, Repr};
use pyaot_utils::{BlockId, LocalId, Span, StringInterner};

/// Lower a resolved, inferred [`HirModule`] into a [`MirProgram`].
///
/// Takes `interner` (beyond the illustrative `lower(&HirModule, &ResolveResult)`)
/// to materialize string-literal bytes into the program's [`StrPool`].
pub fn lower(
    module: &HirModule,
    resolve: &ResolveResult,
    interner: &StringInterner,
) -> Result<MirProgram> {
    let mut str_pool = StrPool::new();
    let mut funcs = Vec::with_capacity(module.functions.len());
    for func in &module.functions {
        funcs.push(lower_function(func, resolve, interner, &mut str_pool)?);
    }
    Ok(MirProgram {
        funcs,
        entry: module.main,
        str_pool,
    })
}

fn lower_function(
    func: &HirFunction,
    resolve: &ResolveResult,
    interner: &StringInterner,
    str_pool: &mut StrPool,
) -> Result<MirFunction> {
    let mut locals: Vec<LocalDecl> = Vec::new();
    let mut insts: Vec<MirInst> = Vec::new();

    let block = &func.blocks[func.entry];
    for stmt in &block.stmts {
        match stmt {
            HirStmt::Expr(expr_idx) => {
                lower_expr_stmt(&func.exprs[*expr_idx], func, resolve, interner, &mut locals, &mut insts, str_pool)?;
            }
        }
    }

    let term = match &block.term {
        HirTerminator::Return(None) => MirTerminator::Return(None),
        HirTerminator::Return(Some(_)) => {
            return Err(CompilerError::semantic_error(
                "non-None return is not supported in Phase 1",
                Span::dummy(),
            ))
        }
    };

    Ok(MirFunction {
        name: func.name,
        params: Vec::new(),
        ret: repr_of(&func.ret_ty),
        locals,
        blocks: vec![MirBlock { insts, term }],
        entry: BlockId::new(0),
    })
}

fn lower_expr_stmt(
    expr: &HirExpr,
    func: &HirFunction,
    resolve: &ResolveResult,
    interner: &StringInterner,
    locals: &mut Vec<LocalDecl>,
    insts: &mut Vec<MirInst>,
    str_pool: &mut StrPool,
) -> Result<()> {
    let HirExprKind::Call { callee, args } = &expr.kind else {
        return Err(CompilerError::semantic_error(
            "Phase 1 supports only `print(...)` statements",
            expr.span,
        ));
    };

    let callee_expr = &func.exprs[*callee];
    let symbol = match &callee_expr.kind {
        HirExprKind::Name(SymbolRef::Resolved(id)) => resolve.symbol(*id),
        HirExprKind::Name(SymbolRef::Unresolved(_)) => {
            return Err(CompilerError::semantic_error(
                "internal: name reached lowering unresolved",
                callee_expr.span,
            ))
        }
        _ => {
            return Err(CompilerError::semantic_error(
                "Phase 1 supports only direct `print(...)` calls",
                callee_expr.span,
            ))
        }
    };

    match symbol {
        Symbol::BuiltinPrint => lower_print(args, func, interner, locals, insts, str_pool),
        _ => Err(CompilerError::semantic_error(
            "Phase 1 supports only `print(...)`",
            callee_expr.span,
        )),
    }
}

fn lower_print(
    args: &[la_arena::Idx<HirExpr>],
    func: &HirFunction,
    interner: &StringInterner,
    locals: &mut Vec<LocalDecl>,
    insts: &mut Vec<MirInst>,
    str_pool: &mut StrPool,
) -> Result<()> {
    for arg_idx in args {
        let arg = &func.exprs[*arg_idx];
        let HirExprKind::StrLit(interned) = &arg.kind else {
            return Err(CompilerError::semantic_error(
                "Phase 1 `print` supports only str-literal arguments",
                arg.span,
            ));
        };
        let interned = *interned;

        // Record the literal's bytes for codegen's data object.
        str_pool.insert(interned, interner.resolve(interned).as_bytes().to_vec());

        // `Const{Str}` lands in a typed `Heap(Str)` local (from `repr_of(Str)`).
        let str_repr = repr_of(&arg.ty);
        let str_local = alloc_local(locals, str_repr.clone());
        insts.push(MirInst::Const {
            dst: str_local,
            val: Const::Str(interned),
        });

        // Route Heap(Str) → Tagged through the single legalize seam.
        let tagged_local = legalize_coerce(str_local, str_repr, Repr::Tagged, locals, insts)?;

        insts.push(MirInst::Print {
            kind: PrintKind::StrObj,
            arg: Some(Operand::Local(tagged_local)),
        });
        // PHASE-2-TODO: emit `Print{Sep}` between successive arguments.
    }

    insts.push(MirInst::Print {
        kind: PrintKind::Newline,
        arg: None,
    });
    Ok(())
}

/// Allocate a fresh local with the given representation, returning its id.
fn alloc_local(locals: &mut Vec<LocalDecl>, repr: Repr) -> LocalId {
    let id = LocalId::new(locals.len() as u32);
    locals.push(LocalDecl { repr });
    id
}

/// Emit a `Coerce` from `src` (`from`) into a fresh local (`to`) — the SINGLE
/// place a coercion is inserted. Errors if `(from, to)` is not a legal coercion.
fn legalize_coerce(
    src: LocalId,
    from: Repr,
    to: Repr,
    locals: &mut Vec<LocalDecl>,
    insts: &mut Vec<MirInst>,
) -> Result<LocalId> {
    if legalize::coerce(from.clone(), to.clone()).is_none() {
        return Err(CompilerError::codegen_error(
            format!("illegal coercion {from:?} -> {to:?}"),
            None,
        ));
    }
    let dst = alloc_local(locals, to.clone());
    insts.push(MirInst::Coerce {
        dst,
        src: Operand::Local(src),
        from,
        to,
    });
    Ok(dst)
}
