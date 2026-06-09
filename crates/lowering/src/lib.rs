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
//! ## Block model
//!
//! Each HIR block maps to a MIR `BlockId` up front. Most statements are
//! straight-line, but a few (`assert`) need a conditional branch and therefore
//! *split* their HIR block into several MIR blocks; a small block builder tracks
//! the "current" MIR block so the split is local. The HIR terminator attaches to
//! whatever MIR block is current after the statements.

#![forbid(unsafe_code)]

pub mod legalize;

use std::collections::HashMap;

use la_arena::Idx;

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{
    BinOp as HBinOp, CmpOp as HCmpOp, HirBlock, HirExpr, HirExprKind, HirFunction, HirModule,
    HirStmt, HirTerminator, ResolveResult, Symbol, SymbolRef, UnaryOp as HUnaryOp,
};
use pyaot_mir::{
    BinOp as MBinOp, CmpOp as MCmpOp, Const, LocalDecl, MirBlock, MirFunction, MirInst, MirProgram,
    MirTerminator, Operand, PrintKind, StrPool, UnaryOp as MUnaryOp,
};
use pyaot_types::{repr_of, HeapShape, RawKind, Repr, SemTy};
use pyaot_utils::{BlockId, LocalId, StringInterner};

/// Lower a resolved, inferred [`HirModule`] into a [`MirProgram`].
pub fn lower(
    module: &HirModule,
    resolve: &ResolveResult,
    interner: &StringInterner,
) -> Result<MirProgram> {
    let mut str_pool = StrPool::new();
    // Signature table (ABI = f(param Repr)), needed before lowering bodies so
    // calls — including forward / recursive ones — coerce args correctly.
    let sigs: Vec<FnSig> = module
        .functions
        .iter()
        .map(|f| FnSig {
            params: f.params.iter().map(|p| repr_of(&p.ty)).collect(),
            ret: repr_of(&f.ret_ty),
        })
        .collect();
    let mut funcs = Vec::with_capacity(module.functions.len());
    for func in &module.functions {
        let mut fl = FnLower::new(func, resolve, interner, &mut str_pool, &sigs);
        funcs.push(fl.lower()?);
    }
    Ok(MirProgram { funcs, entry: module.main, str_pool })
}

/// A function's representation-level signature (ABI).
struct FnSig {
    params: Vec<Repr>,
    ret: Repr,
}

/// Per-function lowering state with a small MIR block builder.
struct FnLower<'a> {
    func: &'a HirFunction,
    resolve: &'a ResolveResult,
    interner: &'a StringInterner,
    str_pool: &'a mut StrPool,
    sigs: &'a [FnSig],
    locals: Vec<LocalDecl>,
    /// Finalized + reserved MIR blocks (placeholders until sealed).
    blocks: Vec<MirBlock>,
    /// HIR block → its *first* MIR block id.
    block_map: HashMap<Idx<HirBlock>, BlockId>,
    /// Instructions accumulating for the current MIR block.
    cur_insts: Vec<MirInst>,
    cur_id: BlockId,
}

impl<'a> FnLower<'a> {
    fn new(
        func: &'a HirFunction,
        resolve: &'a ResolveResult,
        interner: &'a StringInterner,
        str_pool: &'a mut StrPool,
        sigs: &'a [FnSig],
    ) -> Self {
        // MIR locals 0..nhir mirror the HIR locals (LocalId is preserved);
        // temporaries are appended after.
        let locals: Vec<LocalDecl> =
            func.locals.iter().map(|l| LocalDecl { repr: repr_of(&l.ty) }).collect();
        FnLower {
            func,
            resolve,
            interner,
            str_pool,
            sigs,
            locals,
            blocks: Vec::new(),
            block_map: HashMap::new(),
            cur_insts: Vec::new(),
            cur_id: BlockId::new(0),
        }
    }

    fn lower(&mut self) -> Result<MirFunction> {
        // Reserve one MIR block per HIR block, in arena order.
        let hir_blocks: Vec<Idx<HirBlock>> = self.func.blocks.iter().map(|(idx, _)| idx).collect();
        for hidx in &hir_blocks {
            let id = self.reserve_block();
            self.block_map.insert(*hidx, id);
        }
        let entry = self.block_map[&self.func.entry];

        for hidx in &hir_blocks {
            let first = self.block_map[hidx];
            self.cur_id = first;
            self.cur_insts = Vec::new();
            let block = &self.func.blocks[*hidx];
            for stmt in &block.stmts {
                self.lower_stmt(stmt)?;
            }
            let term = self.lower_terminator(&block.term)?;
            self.seal(term);
        }

        let params = self.func.params.iter().map(|p| repr_of(&p.ty)).collect();
        Ok(MirFunction {
            name: self.func.name,
            params,
            ret: repr_of(&self.func.ret_ty),
            locals: std::mem::take(&mut self.locals),
            blocks: std::mem::take(&mut self.blocks),
            entry,
        })
    }

    // ── block builder ──────────────────────────────────────────────────────

    /// Reserve a fresh MIR block slot (placeholder), returning its id.
    fn reserve_block(&mut self) -> BlockId {
        let id = BlockId::new(self.blocks.len() as u32);
        self.blocks.push(MirBlock { insts: Vec::new(), term: MirTerminator::Unreachable });
        id
    }

    /// Finalize the current block with `term`.
    fn seal(&mut self, term: MirTerminator) {
        let insts = std::mem::take(&mut self.cur_insts);
        self.blocks[self.cur_id.index()] = MirBlock { insts, term };
    }

    fn switch(&mut self, id: BlockId) {
        self.cur_id = id;
        self.cur_insts = Vec::new();
    }

    fn emit(&mut self, inst: MirInst) {
        self.cur_insts.push(inst);
    }

    fn alloc_temp(&mut self, repr: Repr) -> LocalId {
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(LocalDecl { repr });
        id
    }

    // ── coercion (the single legalize seam) ──────────────────────────────────

    /// Coerce `src` (`from`) into a fresh local of `to`, returning it. No-op
    /// coercions still alias through a fresh local for a uniform interface.
    fn coerce(&mut self, src: LocalId, from: Repr, to: Repr) -> Result<LocalId> {
        if from == to {
            return Ok(src);
        }
        if legalize::coerce(from.clone(), to.clone()).is_none() {
            return Err(cg_illegal(&from, &to));
        }
        let dst = self.alloc_temp(to.clone());
        self.emit(MirInst::Coerce { dst, src: Operand::Local(src), from, to });
        Ok(dst)
    }

    /// Coerce `src` (`from`) into the *existing* local `dst` (whose declared repr
    /// is `to`) — used for assignment and result-local stores.
    fn coerce_into(&mut self, dst: LocalId, src: LocalId, from: Repr, to: Repr) -> Result<()> {
        if legalize::coerce(from.clone(), to.clone()).is_none() {
            return Err(cg_illegal(&from, &to));
        }
        self.emit(MirInst::Coerce { dst, src: Operand::Local(src), from, to });
        Ok(())
    }

    // ── statements ────────────────────────────────────────────────────────────

    fn lower_stmt(&mut self, stmt: &HirStmt) -> Result<()> {
        match stmt {
            HirStmt::Print { args, sep, end } => self.lower_print(args, *sep, *end),
            HirStmt::Expr(idx) => {
                // Evaluate for side effects; discard the result.
                let _ = self.lower_expr(*idx)?;
                Ok(())
            }
            HirStmt::Assign { target, value } => {
                let (vloc, vrepr) = self.lower_expr(*value)?;
                let target_repr = self.local_repr(*target);
                self.coerce_into(*target, vloc, vrepr, target_repr)?;
                Ok(())
            }
            HirStmt::Assert { cond } => {
                // Truthiness branch: true → continue; false → fail block that
                // raises AssertionError (no message in Phase 2) and is unreachable.
                let cond_op = self.lower_cond(*cond)?;
                let ok = self.reserve_block();
                let fail = self.reserve_block();
                self.seal(MirTerminator::Branch { cond: cond_op, then: ok, else_: fail });
                self.switch(fail);
                self.emit(MirInst::AssertFail);
                self.seal(MirTerminator::Unreachable);
                self.switch(ok);
                Ok(())
            }
        }
    }

    fn lower_print(
        &mut self,
        args: &[Idx<HirExpr>],
        sep: Option<pyaot_utils::InternedString>,
        end: Option<pyaot_utils::InternedString>,
    ) -> Result<()> {
        for (i, arg_idx) in args.iter().enumerate() {
            if i > 0 {
                match sep {
                    None => self.emit(MirInst::Print { kind: PrintKind::Sep, arg: None }),
                    Some(id) => self.emit_print_str(id),
                }
            }
            self.lower_print_arg(*arg_idx)?;
        }
        match end {
            None => self.emit(MirInst::Print { kind: PrintKind::Newline, arg: None }),
            Some(id) => self.emit_print_str(id),
        }
        Ok(())
    }

    /// Print one argument with the `PrintKind` selected from its `SemTy`.
    fn lower_print_arg(&mut self, arg_idx: Idx<HirExpr>) -> Result<()> {
        let ty = self.func.exprs[arg_idx].ty.clone();
        let (loc, repr) = self.lower_expr(arg_idx)?;
        let (kind, want) = print_dispatch(&ty);
        match want {
            // No-operand kinds (None_): value already evaluated for side effects.
            None => self.emit(MirInst::Print { kind, arg: None }),
            Some(want_repr) => {
                let coerced = self.coerce(loc, repr, want_repr)?;
                self.emit(MirInst::Print { kind, arg: Some(Operand::Local(coerced)) });
            }
        }
        Ok(())
    }

    /// Emit `print(<str literal>)` with no separator/newline (used for custom
    /// `sep=`/`end=` strings).
    fn emit_print_str(&mut self, id: pyaot_utils::InternedString) {
        self.str_pool.insert(id, self.interner.resolve(id).as_bytes().to_vec());
        let s = self.alloc_temp(Repr::Heap(HeapShape::Str));
        self.emit(MirInst::Const { dst: s, val: Const::Str(id) });
        // Heap(Str) → Tagged is a free no-op coercion via legalize.
        let tagged = self
            .coerce(s, Repr::Heap(HeapShape::Str), Repr::Tagged)
            .expect("Heap(Str)->Tagged is always legal");
        self.emit(MirInst::Print { kind: PrintKind::StrObj, arg: Some(Operand::Local(tagged)) });
    }

    // ── terminators ──────────────────────────────────────────────────────────

    fn lower_terminator(&mut self, term: &HirTerminator) -> Result<MirTerminator> {
        match term {
            HirTerminator::Return(None) => Ok(MirTerminator::Return(None)),
            HirTerminator::Return(Some(idx)) => {
                let (loc, repr) = self.lower_expr(*idx)?;
                let want = repr_of(&self.func.ret_ty);
                let coerced = self.coerce(loc, repr, want)?;
                Ok(MirTerminator::Return(Some(Operand::Local(coerced))))
            }
            HirTerminator::Jump(target) => Ok(MirTerminator::Jump(self.block_map[target])),
            HirTerminator::Branch { cond, then, else_ } => {
                let cond_op = self.lower_cond(*cond)?;
                Ok(MirTerminator::Branch {
                    cond: cond_op,
                    then: self.block_map[then],
                    else_: self.block_map[else_],
                })
            }
            HirTerminator::Unreachable => Ok(MirTerminator::Unreachable),
        }
    }

    /// Lower a condition expression to a `Raw(I8)` operand suitable for `Branch`.
    fn lower_cond(&mut self, idx: Idx<HirExpr>) -> Result<Operand> {
        let (loc, repr) = self.lower_expr(idx)?;
        if repr == Repr::Raw(RawKind::I8) {
            // Already a bool / comparison result.
            return Ok(Operand::Local(loc));
        }
        // Truthiness on the tagged baseline.
        let tagged = self.coerce(loc, repr, Repr::Tagged)?;
        let dst = self.alloc_temp(Repr::Raw(RawKind::I8));
        self.emit(MirInst::Truthy { dst, operand: Operand::Local(tagged) });
        Ok(Operand::Local(dst))
    }

    // ── expressions ──────────────────────────────────────────────────────────

    /// Lower an expression, returning its result local and that local's `Repr`.
    fn lower_expr(&mut self, idx: Idx<HirExpr>) -> Result<(LocalId, Repr)> {
        let expr = &self.func.exprs[idx];
        match &expr.kind {
            HirExprKind::StrLit(id) => {
                let id = *id;
                self.str_pool.insert(id, self.interner.resolve(id).as_bytes().to_vec());
                let dst = self.alloc_temp(Repr::Heap(HeapShape::Str));
                self.emit(MirInst::Const { dst, val: Const::Str(id) });
                Ok((dst, Repr::Heap(HeapShape::Str)))
            }
            HirExprKind::IntLit(v) => {
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::Const { dst, val: Const::Int(*v) });
                Ok((dst, Repr::Tagged))
            }
            HirExprKind::BigIntLit(id) => {
                let id = *id;
                self.str_pool.insert(id, self.interner.resolve(id).as_bytes().to_vec());
                let dst = self.alloc_temp(Repr::Heap(HeapShape::BigInt));
                self.emit(MirInst::Const { dst, val: Const::BigIntStr(id) });
                Ok((dst, Repr::Heap(HeapShape::BigInt)))
            }
            HirExprKind::FloatLit(f) => {
                let dst = self.alloc_temp(Repr::Raw(RawKind::F64));
                self.emit(MirInst::Const { dst, val: Const::Float(*f) });
                Ok((dst, Repr::Raw(RawKind::F64)))
            }
            HirExprKind::BoolLit(b) => {
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::Const { dst, val: Const::Bool(*b) });
                Ok((dst, Repr::Tagged))
            }
            HirExprKind::NoneLit => {
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::Const { dst, val: Const::None });
                Ok((dst, Repr::Tagged))
            }
            HirExprKind::Name(symref) => self.lower_name(*symref, expr.span),
            HirExprKind::Local(lid) => Ok((*lid, self.local_repr(*lid))),
            HirExprKind::BinOp { op, l, r } => self.lower_binop(*op, *l, *r),
            HirExprKind::Unary { op, operand } => self.lower_unary(*op, *operand),
            HirExprKind::Compare { op, l, r } => self.lower_compare(*op, *l, *r),
            HirExprKind::Call { callee, args } => self.lower_call(*callee, args.clone()),
        }
    }

    /// Lower `l <op> r`. Every binary op (arithmetic *and* bitwise/shift) runs on
    /// the tagged baseline so it stays bignum-safe — an `int` operand may be a
    /// heap `BigInt`, and unboxing to raw `i64` would silently miscompile. A
    /// range-proven raw fast path for bitwise/shift is a Phase-3 optimization.
    fn lower_binop(
        &mut self,
        op: HBinOp,
        l: Idx<HirExpr>,
        r: Idx<HirExpr>,
    ) -> Result<(LocalId, Repr)> {
        let mop = map_binop(op);
        let (ll, lr) = self.lower_expr(l)?;
        let la = self.coerce(ll, lr, Repr::Tagged)?;
        let (rl, rr) = self.lower_expr(r)?;
        let ra = self.coerce(rl, rr, Repr::Tagged)?;
        let dst = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::BinOp {
            dst,
            op: mop,
            l: Operand::Local(la),
            r: Operand::Local(ra),
        });
        Ok((dst, Repr::Tagged))
    }

    fn lower_unary(&mut self, op: HUnaryOp, operand: Idx<HirExpr>) -> Result<(LocalId, Repr)> {
        let mop = map_unaryop(op);
        let (ol, orr) = self.lower_expr(operand)?;
        let ot = self.coerce(ol, orr, Repr::Tagged)?;
        let dst_repr = if mop == MUnaryOp::Not {
            Repr::Raw(RawKind::I8)
        } else {
            Repr::Tagged
        };
        let dst = self.alloc_temp(dst_repr.clone());
        self.emit(MirInst::Unary { dst, op: mop, operand: Operand::Local(ot) });
        Ok((dst, dst_repr))
    }

    fn lower_compare(
        &mut self,
        op: HCmpOp,
        l: Idx<HirExpr>,
        r: Idx<HirExpr>,
    ) -> Result<(LocalId, Repr)> {
        let (ll, lr) = self.lower_expr(l)?;
        let la = self.coerce(ll, lr, Repr::Tagged)?;
        let (rl, rr) = self.lower_expr(r)?;
        let ra = self.coerce(rl, rr, Repr::Tagged)?;
        let dst = self.alloc_temp(Repr::Raw(RawKind::I8));
        self.emit(MirInst::Compare {
            dst,
            op: map_cmpop(op),
            l: Operand::Local(la),
            r: Operand::Local(ra),
        });
        Ok((dst, Repr::Raw(RawKind::I8)))
    }

    fn lower_call(&mut self, callee: Idx<HirExpr>, args: Vec<Idx<HirExpr>>) -> Result<(LocalId, Repr)> {
        let span = self.func.exprs[callee].span;
        let sym = match &self.func.exprs[callee].kind {
            HirExprKind::Name(SymbolRef::Resolved(id)) => self.resolve.symbol(*id),
            _ => {
                return Err(CompilerError::semantic_error(
                    "only direct calls to a named function or builtin are supported",
                    span,
                ))
            }
        };
        match sym {
            Symbol::Builtin(kind) => {
                let mut argvals = Vec::with_capacity(args.len());
                for a in &args {
                    let (al, ar) = self.lower_expr(*a)?;
                    let at = self.coerce(al, ar, Repr::Tagged)?;
                    argvals.push(Operand::Local(at));
                }
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::CallBuiltin { dst: Some(dst), kind, args: argvals });
                Ok((dst, Repr::Tagged))
            }
            Symbol::Function(fid) => {
                let params = self.sigs[fid.index()].params.clone();
                let ret = self.sigs[fid.index()].ret.clone();
                if args.len() != params.len() {
                    return Err(CompilerError::semantic_error(
                        "wrong number of arguments in call",
                        span,
                    ));
                }
                let mut argvals = Vec::with_capacity(args.len());
                for (a, prepr) in args.iter().zip(params) {
                    let (al, ar) = self.lower_expr(*a)?;
                    let at = self.coerce(al, ar, prepr)?;
                    argvals.push(Operand::Local(at));
                }
                let dst = self.alloc_temp(ret.clone());
                self.emit(MirInst::Call { dst: Some(dst), func: fid, args: argvals });
                Ok((dst, ret))
            }
            Symbol::BuiltinPrint | Symbol::BuiltinRange | Symbol::Local(_) => {
                Err(CompilerError::semantic_error(
                    "this callee is not usable as a value-returning call here",
                    span,
                ))
            }
        }
    }

    fn lower_name(&mut self, symref: SymbolRef, span: pyaot_utils::Span) -> Result<(LocalId, Repr)> {
        let id = match symref {
            SymbolRef::Resolved(id) => id,
            SymbolRef::Unresolved(_) => {
                return Err(CompilerError::semantic_error(
                    "internal: name reached lowering unresolved",
                    span,
                ))
            }
        };
        match self.resolve.symbol(id) {
            Symbol::Local(lid) => Ok((lid, self.local_repr(lid))),
            Symbol::BuiltinPrint | Symbol::BuiltinRange | Symbol::Builtin(_) | Symbol::Function(_) => {
                Err(CompilerError::semantic_error(
                    "this name cannot be used as a value here (only call targets are supported)",
                    span,
                ))
            }
        }
    }

    fn local_repr(&self, id: LocalId) -> Repr {
        self.locals[id.index()].repr.clone()
    }
}

/// Pick the `PrintKind` and required operand `Repr` from an argument's `SemTy`.
/// `None` means the kind takes no operand.
fn print_dispatch(ty: &SemTy) -> (PrintKind, Option<Repr>) {
    match ty {
        SemTy::Str => (PrintKind::StrObj, Some(Repr::Tagged)),
        SemTy::Float => (PrintKind::Float, Some(Repr::Raw(RawKind::F64))),
        SemTy::Bool => (PrintKind::Bool, Some(Repr::Raw(RawKind::I8))),
        SemTy::NoneTy => (PrintKind::None_, None),
        // Ints route through the tag-dispatched catch-all so bignum prints
        // correctly without untagging; Dyn/everything-else likewise.
        _ => (PrintKind::Obj, Some(Repr::Tagged)),
    }
}

fn map_binop(op: HBinOp) -> MBinOp {
    match op {
        HBinOp::Add => MBinOp::Add,
        HBinOp::Sub => MBinOp::Sub,
        HBinOp::Mul => MBinOp::Mul,
        HBinOp::Div => MBinOp::Div,
        HBinOp::FloorDiv => MBinOp::FloorDiv,
        HBinOp::Mod => MBinOp::Mod,
        HBinOp::Pow => MBinOp::Pow,
        HBinOp::BitAnd => MBinOp::BitAnd,
        HBinOp::BitOr => MBinOp::BitOr,
        HBinOp::BitXor => MBinOp::BitXor,
        HBinOp::Shl => MBinOp::Shl,
        HBinOp::Shr => MBinOp::Shr,
    }
}

fn map_unaryop(op: HUnaryOp) -> MUnaryOp {
    match op {
        HUnaryOp::Neg => MUnaryOp::Neg,
        HUnaryOp::Pos => MUnaryOp::Pos,
        HUnaryOp::Invert => MUnaryOp::Invert,
        HUnaryOp::Not => MUnaryOp::Not,
    }
}

fn map_cmpop(op: HCmpOp) -> MCmpOp {
    match op {
        HCmpOp::Eq => MCmpOp::Eq,
        HCmpOp::NotEq => MCmpOp::NotEq,
        HCmpOp::Lt => MCmpOp::Lt,
        HCmpOp::LtE => MCmpOp::LtE,
        HCmpOp::Gt => MCmpOp::Gt,
        HCmpOp::GtE => MCmpOp::GtE,
    }
}

fn cg_illegal(from: &Repr, to: &Repr) -> CompilerError {
    CompilerError::codegen_error(format!("illegal coercion {from:?} -> {to:?}"), None)
}
