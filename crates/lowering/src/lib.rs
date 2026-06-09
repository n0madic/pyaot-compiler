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
    BinOp as HBinOp, CmpOp as HCmpOp, ContainerArg, ContainerMethod, ContainerOp, ContainerResult,
    HirBlock, HirExpr, HirExprKind, HirFunction, HirLocal, HirModule, HirStmt, HirTerminator,
    ResolveResult, Symbol, SymbolRef, UnaryOp as HUnaryOp,
};
use pyaot_mir::{
    BinOp as MBinOp, CmpOp as MCmpOp, Const, LocalDecl, MirBlock, MirFunction, MirInst, MirProgram,
    MirTerminator, Operand, PrintKind, StrPool, UnaryOp as MUnaryOp,
};
use pyaot_types::{repr_of, HeapShape, RawKind, Repr, SemTy, RAW_I64_NARROW_BOUND};
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
            func.locals.iter().map(|l| LocalDecl { repr: local_repr(l) }).collect();
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

    /// Supply an already-lowered operand as a sound `Raw(I64)`, or `None`.
    ///
    /// It qualifies if it already lowered to `Raw(I64)` (a range-proven cursor),
    /// or it is a fixnum integer literal within [`RAW_I64_NARROW_BOUND`] — such a
    /// literal provably is not a heap `BigInt`, so untagging it to a machine i64
    /// is sound (PITFALLS B16) and the bound keeps the arithmetic result in range.
    fn raw_i64_operand(
        &mut self,
        expr: Idx<HirExpr>,
        loc: LocalId,
        repr: &Repr,
    ) -> Result<Option<LocalId>> {
        let i64r = Repr::Raw(RawKind::I64);
        if *repr == i64r {
            return Ok(Some(loc));
        }
        if *repr == Repr::Tagged {
            if let HirExprKind::IntLit(v) = self.func.exprs[expr].kind {
                if (-RAW_I64_NARROW_BOUND..=RAW_I64_NARROW_BOUND).contains(&v) {
                    let raw = self.coerce(loc, Repr::Tagged, i64r)?;
                    return Ok(Some(raw));
                }
            }
        }
        Ok(None)
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
            HirStmt::SetItem { base, index, value } => self.lower_setitem(*base, *index, *value),
            HirStmt::ContainerPush { container, value } => {
                let cont_repr = self.local_repr(*container);
                let (vl, vr) = self.lower_expr(*value)?;
                let op = match &cont_repr {
                    Repr::Heap(HeapShape::Set(_)) => ContainerOp::SetAdd,
                    _ => ContainerOp::ListPush,
                };
                self.emit_container(op, vec![(*container, cont_repr), (vl, vr)], None)?;
                Ok(())
            }
            HirStmt::ContainerInsert { container, key, value } => {
                let cont_repr = self.local_repr(*container);
                let (kl, kr) = self.lower_expr(*key)?;
                let (vl, vr) = self.lower_expr(*value)?;
                self.emit_container(
                    ContainerOp::DictSet,
                    vec![(*container, cont_repr), (kl, kr), (vl, vr)],
                    None,
                )?;
                Ok(())
            }
        }
    }

    /// Lower `base[index] = value`, dispatching the runtime setter from the static
    /// container type. Assigning to a tuple element is a compile error.
    fn lower_setitem(
        &mut self,
        base: Idx<HirExpr>,
        index: Idx<HirExpr>,
        value: Idx<HirExpr>,
    ) -> Result<()> {
        let span = self.func.exprs[base].span;
        let kind = sub_kind(&self.func.exprs[base].ty, &repr_of(&self.func.exprs[base].ty));
        let (bl, br) = self.lower_expr(base)?;
        let (il, ir) = self.lower_expr(index)?;
        let (vl, vr) = self.lower_expr(value)?;
        match kind {
            SubKind::List => {
                self.emit_container(
                    ContainerOp::ListSet,
                    vec![(bl, br), (il, ir), (vl, vr)],
                    None,
                )?;
            }
            SubKind::Dict => {
                self.emit_container(
                    ContainerOp::DictSet,
                    vec![(bl, br), (il, ir), (vl, vr)],
                    None,
                )?;
            }
            SubKind::Tuple => {
                return Err(CompilerError::semantic_error(
                    "'tuple' object does not support item assignment",
                    span,
                ));
            }
            SubKind::Bytes | SubKind::Str | SubKind::Generic => {
                return Err(CompilerError::semantic_error(
                    "subscript assignment requires a statically-known list or dict target",
                    span,
                ));
            }
        }
        Ok(())
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
            HirExprKind::Call { callee, args } => self.lower_call(idx, *callee, args.clone()),
            // ── containers (Phase 4) ──
            HirExprKind::ListLit { elems } => self.lower_list_lit(idx, &elems.clone()),
            HirExprKind::SetLit { elems } => self.lower_set_lit(idx, &elems.clone()),
            HirExprKind::TupleLit { elems } => self.lower_tuple_lit(idx, &elems.clone()),
            HirExprKind::DictLit { pairs } => self.lower_dict_lit(idx, &pairs.clone()),
            HirExprKind::BytesLit(id) => {
                let id = *id;
                self.str_pool.insert(id, self.interner.resolve(id).as_bytes().to_vec());
                let dst = self.alloc_temp(Repr::Heap(HeapShape::Bytes));
                self.emit(MirInst::Const { dst, val: Const::Bytes(id) });
                Ok((dst, Repr::Heap(HeapShape::Bytes)))
            }
            HirExprKind::Subscript { base, index } => self.lower_subscript(*base, *index),
            HirExprKind::ContainerExpr { op, args } => {
                self.lower_container_expr(idx, *op, &args.clone())
            }
            HirExprKind::MethodCall { recv, method, args } => {
                self.lower_method_call(idx, *recv, *method, &args.clone())
            }
        }
    }

    // ── container expressions (Phase 4) ──────────────────────────────────────

    /// Materialize a small `Raw(I64)` constant (a capacity / size / count). The
    /// value is a compile-time element count well within the fixnum range, so the
    /// `Tagged → Raw(I64)` untag round-trips soundly.
    fn raw_i64_const(&mut self, n: i64) -> LocalId {
        let t = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::Const { dst: t, val: Const::Int(n) });
        self.coerce(t, Repr::Tagged, Repr::Raw(RawKind::I64))
            .expect("Tagged -> Raw(I64) is always legal")
    }

    /// Bring an already-lowered operand to an unboxed `Raw(I64)` index/count. A
    /// `Raw(I64)` (range cursor) is used directly; anything else is routed through
    /// `Tagged` then untagged. Sound for the in-range fixnum indices this phase
    /// gates (a pathological bignum index is out of scope — it would be an
    /// `IndexError` in CPython either way).
    fn coerce_to_i64(&mut self, loc: LocalId, repr: Repr) -> Result<LocalId> {
        let i64r = Repr::Raw(RawKind::I64);
        if repr == i64r {
            return Ok(loc);
        }
        let tagged = self.coerce(loc, repr, Repr::Tagged)?;
        self.coerce(tagged, Repr::Tagged, i64r)
    }

    /// Emit a `CallContainer`, coercing each argument to the representation its
    /// position requires (`Val` → `Tagged`, the A5 element-coercion seam; `Idx` →
    /// `Raw(I64)`) and allocating a `dst` per the op's result category. For a
    /// `Heap` result the caller supplies the exact heap `dst` representation.
    fn emit_container(
        &mut self,
        op: ContainerOp,
        args: Vec<(LocalId, Repr)>,
        heap_repr: Option<Repr>,
    ) -> Result<(Option<LocalId>, Repr)> {
        let kinds = op.arg_kinds();
        debug_assert_eq!(kinds.len(), args.len(), "arity mismatch for {op:?}");
        let mut ops = Vec::with_capacity(args.len());
        for ((loc, repr), kind) in args.into_iter().zip(kinds) {
            let coerced = match kind {
                ContainerArg::Val => self.coerce(loc, repr, Repr::Tagged)?,
                ContainerArg::Idx => self.coerce_to_i64(loc, repr)?,
            };
            ops.push(Operand::Local(coerced));
        }
        let (dst, ret) = match op.result() {
            ContainerResult::None => (None, Repr::Tagged),
            ContainerResult::Value => {
                let d = self.alloc_temp(Repr::Tagged);
                (Some(d), Repr::Tagged)
            }
            ContainerResult::Int => {
                let d = self.alloc_temp(Repr::Raw(RawKind::I64));
                (Some(d), Repr::Raw(RawKind::I64))
            }
            ContainerResult::Bool => {
                let d = self.alloc_temp(Repr::Raw(RawKind::I8));
                (Some(d), Repr::Raw(RawKind::I8))
            }
            ContainerResult::Heap => {
                let r = heap_repr.expect("heap-producing container op needs a dst repr");
                let d = self.alloc_temp(r.clone());
                (Some(d), r)
            }
        };
        self.emit(MirInst::CallContainer { dst, op, args: ops });
        Ok((dst, ret))
    }

    /// Normalize a `Raw(I64)` container result (`len`, a byte value) to the tagged
    /// int baseline so it flows as an ordinary `int` value; other results pass
    /// through (`Bool` stays `Raw(I8)`, container results stay `Heap`).
    fn normalize_container_result(&mut self, dst: LocalId, ret: Repr) -> Result<(LocalId, Repr)> {
        if ret == Repr::Raw(RawKind::I64) {
            let t = self.coerce(dst, ret, Repr::Tagged)?;
            Ok((t, Repr::Tagged))
        } else {
            Ok((dst, ret))
        }
    }

    fn lower_list_lit(&mut self, idx: Idx<HirExpr>, elems: &[Idx<HirExpr>]) -> Result<(LocalId, Repr)> {
        let list_repr = repr_of(&self.func.exprs[idx].ty);
        let cap = self.raw_i64_const(elems.len() as i64);
        let (list, _) =
            self.emit_container(ContainerOp::ListNew, vec![(cap, Repr::Raw(RawKind::I64))], Some(list_repr.clone()))?;
        let list = list.expect("ListNew produces a list");
        for e in elems {
            let (el, er) = self.lower_expr(*e)?;
            self.emit_container(
                ContainerOp::ListPush,
                vec![(list, list_repr.clone()), (el, er)],
                None,
            )?;
        }
        Ok((list, list_repr))
    }

    fn lower_set_lit(&mut self, idx: Idx<HirExpr>, elems: &[Idx<HirExpr>]) -> Result<(LocalId, Repr)> {
        let set_repr = repr_of(&self.func.exprs[idx].ty);
        let cap = self.raw_i64_const(elems.len() as i64);
        let (set, _) =
            self.emit_container(ContainerOp::SetNew, vec![(cap, Repr::Raw(RawKind::I64))], Some(set_repr.clone()))?;
        let set = set.expect("SetNew produces a set");
        for e in elems {
            let (el, er) = self.lower_expr(*e)?;
            self.emit_container(ContainerOp::SetAdd, vec![(set, set_repr.clone()), (el, er)], None)?;
        }
        Ok((set, set_repr))
    }

    fn lower_dict_lit(
        &mut self,
        idx: Idx<HirExpr>,
        pairs: &[(Idx<HirExpr>, Idx<HirExpr>)],
    ) -> Result<(LocalId, Repr)> {
        let dict_repr = repr_of(&self.func.exprs[idx].ty);
        let cap = self.raw_i64_const(pairs.len() as i64);
        let (dict, _) =
            self.emit_container(ContainerOp::DictNew, vec![(cap, Repr::Raw(RawKind::I64))], Some(dict_repr.clone()))?;
        let dict = dict.expect("DictNew produces a dict");
        for (k, v) in pairs {
            let (kl, kr) = self.lower_expr(*k)?;
            let (vl, vr) = self.lower_expr(*v)?;
            self.emit_container(
                ContainerOp::DictSet,
                vec![(dict, dict_repr.clone()), (kl, kr), (vl, vr)],
                None,
            )?;
        }
        Ok((dict, dict_repr))
    }

    fn lower_tuple_lit(&mut self, idx: Idx<HirExpr>, elems: &[Idx<HirExpr>]) -> Result<(LocalId, Repr)> {
        let tup_repr = repr_of(&self.func.exprs[idx].ty);
        let size = self.raw_i64_const(elems.len() as i64);
        let (tup, _) =
            self.emit_container(ContainerOp::TupleNew, vec![(size, Repr::Raw(RawKind::I64))], Some(tup_repr.clone()))?;
        let tup = tup.expect("TupleNew produces a tuple");
        for (i, e) in elems.iter().enumerate() {
            let (el, er) = self.lower_expr(*e)?;
            let pos = self.raw_i64_const(i as i64);
            self.emit_container(
                ContainerOp::TupleSet,
                vec![(tup, tup_repr.clone()), (pos, Repr::Raw(RawKind::I64)), (el, er)],
                None,
            )?;
        }
        Ok((tup, tup_repr))
    }

    /// Lower a subscript read `base[index]`, dispatching the runtime getter from
    /// the base's *static type* (which survives even when a nested get lowered the
    /// base into a uniform-tagged slot) and falling back to its representation. The
    /// result is normalized to the tagged baseline.
    fn lower_subscript(&mut self, base: Idx<HirExpr>, index: Idx<HirExpr>) -> Result<(LocalId, Repr)> {
        let kind = sub_kind(&self.func.exprs[base].ty, &repr_of(&self.func.exprs[base].ty));
        let (bl, br) = self.lower_expr(base)?;
        let (il, ir) = self.lower_expr(index)?;
        let op = match kind {
            SubKind::List => ContainerOp::ListGet,
            SubKind::Dict => ContainerOp::DictGet,
            SubKind::Tuple => ContainerOp::TupleGet,
            SubKind::Bytes => ContainerOp::BytesGet,
            // Str needs the codepoint-aware getter (handles negative indices); the
            // generic `rt_any_getitem` only does byte indexing.
            SubKind::Str => ContainerOp::StrGet,
            // Unknown base → the tag-dispatched generic getter.
            SubKind::Generic => ContainerOp::AnyGetItem,
        };
        let (dst, ret) = self.emit_container(op, vec![(bl, br), (il, ir)], None)?;
        self.normalize_container_result(dst.expect("subscript produces a value"), ret)
    }

    /// Lower a frontend-synthesized container op (`in`, the iterator protocol).
    fn lower_container_expr(
        &mut self,
        idx: Idx<HirExpr>,
        op: ContainerOp,
        args: &[Idx<HirExpr>],
    ) -> Result<(LocalId, Repr)> {
        let mut lowered = Vec::with_capacity(args.len());
        for a in args {
            lowered.push(self.lower_expr(*a)?);
        }
        let heap = (op.result() == ContainerResult::Heap)
            .then(|| repr_of(&self.func.exprs[idx].ty));
        let (dst, ret) = self.emit_container(op, lowered, heap)?;
        self.normalize_container_result(dst.expect("container expr produces a value"), ret)
    }

    /// Lower a container method call `recv.method(args)` (Phase 4D), dispatching
    /// the concrete runtime op from the receiver's static type. Args/values are
    /// coerced to `Tagged`; results are normalized from `Tagged`.
    fn lower_method_call(
        &mut self,
        call_idx: Idx<HirExpr>,
        recv: Idx<HirExpr>,
        method: ContainerMethod,
        args: &[Idx<HirExpr>],
    ) -> Result<(LocalId, Repr)> {
        use ContainerMethod as M;
        let span = self.func.exprs[recv].span;
        let recv_ty = self.func.exprs[recv].ty.clone();
        let kind = if recv_ty.list_elem().is_some() {
            MethodRecv::List
        } else if recv_ty.dict_kv().is_some() {
            MethodRecv::Dict
        } else if recv_ty.set_elem().is_some() {
            MethodRecv::Set
        } else {
            MethodRecv::Other
        };
        let (rl, rr) = self.lower_expr(recv)?;
        let recv_arg = (rl, rr);
        // Result heap shape for container-producing methods (`copy`, set algebra,
        // dict views) comes from the method's own inferred result type.
        let heap = || repr_of(&self.func.exprs[call_idx].ty);

        // Lower the call's positional args once (most methods take 0-2).
        let mut a = Vec::with_capacity(args.len());
        for arg in args {
            a.push(self.lower_expr(*arg)?);
        }
        let argn = a.len();

        let bad = |m: &str| CompilerError::semantic_error(m.to_string(), span);

        match kind {
            MethodRecv::List => match method {
                M::Append if argn == 1 => {
                    self.emit_container(ContainerOp::ListPush, vec![recv_arg, a[0].clone()], None)?;
                    self.none_value()
                }
                M::Pop => {
                    let idx = if argn == 1 {
                        a[0].clone()
                    } else {
                        (self.raw_i64_const(-1), Repr::Raw(RawKind::I64))
                    };
                    let (d, r) = self.emit_container(ContainerOp::ListPop, vec![recv_arg, idx], None)?;
                    self.normalize_container_result(d.unwrap(), r)
                }
                M::Insert if argn == 2 => {
                    self.emit_container(
                        ContainerOp::ListInsert,
                        vec![recv_arg, a[0].clone(), a[1].clone()],
                        None,
                    )?;
                    self.none_value()
                }
                M::Extend if argn == 1 => {
                    self.emit_container(ContainerOp::ListExtend, vec![recv_arg, a[0].clone()], None)?;
                    self.none_value()
                }
                M::Index if argn == 1 => self.method_scalar(ContainerOp::ListIndexOf, recv_arg, vec![a[0].clone()]),
                M::Count if argn == 1 => self.method_scalar(ContainerOp::ListCount, recv_arg, vec![a[0].clone()]),
                M::Clear if argn == 0 => {
                    self.emit_container(ContainerOp::ListClear, vec![recv_arg], None)?;
                    self.none_value()
                }
                M::Copy if argn == 0 => self.method_heap(ContainerOp::ListCopy, recv_arg, vec![], heap()),
                M::Reverse if argn == 0 => {
                    self.emit_container(ContainerOp::ListReverse, vec![recv_arg], None)?;
                    self.none_value()
                }
                M::Sort if argn == 0 => {
                    self.emit_container(ContainerOp::ListSortMut, vec![recv_arg], None)?;
                    self.none_value()
                }
                _ => Err(bad("unsupported list method / arity")),
            },
            MethodRecv::Dict => match method {
                M::Get if argn == 1 || argn == 2 => {
                    let default = if argn == 2 { a[1].clone() } else { (self.none_temp(), Repr::Tagged) };
                    let (d, r) = self.emit_container(
                        ContainerOp::DictGetDefault,
                        vec![recv_arg, a[0].clone(), default],
                        None,
                    )?;
                    self.normalize_container_result(d.unwrap(), r)
                }
                M::Setdefault if argn == 1 || argn == 2 => {
                    let default = if argn == 2 { a[1].clone() } else { (self.none_temp(), Repr::Tagged) };
                    let (d, r) = self.emit_container(
                        ContainerOp::DictSetdefault,
                        vec![recv_arg, a[0].clone(), default],
                        None,
                    )?;
                    self.normalize_container_result(d.unwrap(), r)
                }
                M::Pop if argn == 1 => {
                    let (d, r) = self.emit_container(ContainerOp::DictPopM, vec![recv_arg, a[0].clone()], None)?;
                    self.normalize_container_result(d.unwrap(), r)
                }
                M::Keys if argn == 0 => self.method_heap(ContainerOp::DictKeys, recv_arg, vec![], heap()),
                M::Values if argn == 0 => self.method_heap(ContainerOp::DictValues, recv_arg, vec![], heap()),
                M::Items if argn == 0 => self.method_heap(ContainerOp::DictItems, recv_arg, vec![], heap()),
                M::Update if argn == 1 => {
                    self.emit_container(ContainerOp::DictUpdate, vec![recv_arg, a[0].clone()], None)?;
                    self.none_value()
                }
                M::Clear if argn == 0 => {
                    self.emit_container(ContainerOp::DictClear, vec![recv_arg], None)?;
                    self.none_value()
                }
                M::Copy if argn == 0 => self.method_heap(ContainerOp::DictCopy, recv_arg, vec![], heap()),
                _ => Err(bad("unsupported dict method / arity")),
            },
            MethodRecv::Set => match method {
                M::Add if argn == 1 => {
                    self.emit_container(ContainerOp::SetAdd, vec![recv_arg, a[0].clone()], None)?;
                    self.none_value()
                }
                M::Remove if argn == 1 => {
                    self.emit_container(ContainerOp::SetRemove, vec![recv_arg, a[0].clone()], None)?;
                    self.none_value()
                }
                M::Discard if argn == 1 => {
                    self.emit_container(ContainerOp::SetDiscard, vec![recv_arg, a[0].clone()], None)?;
                    self.none_value()
                }
                M::Update if argn == 1 => {
                    self.emit_container(ContainerOp::SetUpdate, vec![recv_arg, a[0].clone()], None)?;
                    self.none_value()
                }
                M::Union if argn == 1 => self.method_heap(ContainerOp::SetUnion, recv_arg, vec![a[0].clone()], heap()),
                M::Intersection if argn == 1 => {
                    self.method_heap(ContainerOp::SetIntersection, recv_arg, vec![a[0].clone()], heap())
                }
                M::Difference if argn == 1 => {
                    self.method_heap(ContainerOp::SetDifference, recv_arg, vec![a[0].clone()], heap())
                }
                M::Copy if argn == 0 => self.method_heap(ContainerOp::SetCopy, recv_arg, vec![], heap()),
                M::Clear if argn == 0 => {
                    self.emit_container(ContainerOp::SetClear, vec![recv_arg], None)?;
                    self.none_value()
                }
                _ => Err(bad("unsupported set method / arity")),
            },
            _ => Err(bad("method calls require a statically-known list, dict, or set receiver")),
        }
    }

    /// Emit a method op with an `Int` result, normalized to the tagged baseline.
    fn method_scalar(
        &mut self,
        op: ContainerOp,
        recv: (LocalId, Repr),
        rest: Vec<(LocalId, Repr)>,
    ) -> Result<(LocalId, Repr)> {
        let mut args = vec![recv];
        args.extend(rest);
        let (d, r) = self.emit_container(op, args, None)?;
        self.normalize_container_result(d.unwrap(), r)
    }

    /// Emit a method op producing a fresh container of representation `heap`.
    fn method_heap(
        &mut self,
        op: ContainerOp,
        recv: (LocalId, Repr),
        rest: Vec<(LocalId, Repr)>,
        heap: Repr,
    ) -> Result<(LocalId, Repr)> {
        let mut args = vec![recv];
        args.extend(rest);
        let (d, r) = self.emit_container(op, args, Some(heap))?;
        Ok((d.unwrap(), r))
    }

    /// Materialize the tagged `None` singleton into a fresh local.
    fn none_temp(&mut self) -> LocalId {
        let t = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::Const { dst: t, val: Const::None });
        t
    }

    /// A `None`-valued result (for mutating methods used as expressions).
    fn none_value(&mut self) -> Result<(LocalId, Repr)> {
        Ok((self.none_temp(), Repr::Tagged))
    }

    /// Lower `l <op> r`.
    ///
    /// **Float fast path (Phase 3b):** when `op ∈ {+, -, *}` and *both* operands
    /// lower to unboxed `Raw(F64)`, emit a `Raw(F64)` `BinOp` — codegen inlines it
    /// as `fadd`/`fsub`/`fmul` with no boxing and no call. These three ops are
    /// exception-free; `/` differs from CPython on `x/0.0` and `// % **` carry
    /// floor/sign/promotion semantics, so they stay on the tagged baseline
    /// (PITFALLS B1). Mixed int/float also stays tagged (the runtime promotes).
    ///
    /// **Tagged baseline (everything else):** every operand is coerced to a
    /// `Tagged` `Value` and the op dispatches on the tag in the runtime
    /// (`rt_obj_*`), so it is bignum-safe — an `int` operand may dynamically be a
    /// heap `BigInt`, and unboxing it to raw `i64` would silently miscompile
    /// (Invariant 2). The proof-gated raw `int` fast path is Phase 3c.
    fn lower_binop(
        &mut self,
        op: HBinOp,
        l: Idx<HirExpr>,
        r: Idx<HirExpr>,
    ) -> Result<(LocalId, Repr)> {
        let mop = map_binop(op);
        let (ll, lr) = self.lower_expr(l)?;
        let (rl, rr) = self.lower_expr(r)?;

        // Container `+` / `*`: the tagged `rt_obj_add`/`rt_obj_mul` handle only
        // str + numeric, so list/tuple/bytes concatenation and repetition dispatch
        // by static type to the typed runtime ops.
        if let Some(res) = self.try_container_binop(mop, ll, &lr, rl, &rr)? {
            return Ok(res);
        }

        let raw_addsubmul = matches!(mop, MBinOp::Add | MBinOp::Sub | MBinOp::Mul);
        let f64 = Repr::Raw(RawKind::F64);
        if raw_addsubmul && lr == f64 && rr == f64 {
            let dst = self.alloc_temp(f64.clone());
            self.emit(MirInst::BinOp {
                dst,
                op: mop,
                l: Operand::Local(ll),
                r: Operand::Local(rl),
            });
            return Ok((dst, f64));
        }

        // Raw int fast path (Phase 3c): when one operand is a range-proven
        // `Raw(I64)` cursor, emit a raw machine `Add`/`Sub`. The other operand is
        // supplied as `Raw(I64)` too (another cursor, or a fixnum literal small
        // enough to untag soundly). `Mul` is deliberately excluded — a raw product
        // of two bounded values could leave the proven fixnum range and overflow.
        let i64r = Repr::Raw(RawKind::I64);
        if matches!(mop, MBinOp::Add | MBinOp::Sub) && (lr == i64r || rr == i64r) {
            if let (Some(la), Some(ra)) =
                (self.raw_i64_operand(l, ll, &lr)?, self.raw_i64_operand(r, rl, &rr)?)
            {
                let dst = self.alloc_temp(i64r.clone());
                self.emit(MirInst::BinOp {
                    dst,
                    op: mop,
                    l: Operand::Local(la),
                    r: Operand::Local(ra),
                });
                return Ok((dst, i64r));
            }
        }

        let la = self.coerce(ll, lr, Repr::Tagged)?;
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

    /// Dispatch a container `+` (concat) or `*` (repeat) by static type, or return
    /// `None` to fall through to the numeric / tagged baseline.
    fn try_container_binop(
        &mut self,
        op: MBinOp,
        ll: LocalId,
        lr: &Repr,
        rl: LocalId,
        rr: &Repr,
    ) -> Result<Option<(LocalId, Repr)>> {
        use HeapShape::{Bytes, List, Tuple, TupleVar};
        match op {
            MBinOp::Add => {
                let cop = match (lr, rr) {
                    (Repr::Heap(List(_)), Repr::Heap(List(_))) => ContainerOp::ListConcat,
                    (
                        Repr::Heap(Tuple(_) | TupleVar(_)),
                        Repr::Heap(Tuple(_) | TupleVar(_)),
                    ) => ContainerOp::TupleConcat,
                    (Repr::Heap(Bytes), Repr::Heap(Bytes)) => ContainerOp::BytesConcat,
                    _ => return Ok(None),
                };
                let (dst, ret) = self.emit_container(
                    cop,
                    vec![(ll, lr.clone()), (rl, rr.clone())],
                    Some(lr.clone()),
                )?;
                Ok(Some((dst.expect("concat produces a container"), ret)))
            }
            MBinOp::Mul => {
                // `seq * int` or `int * seq` — identify the sequence operand.
                let (seq, seq_repr, count, count_repr) = if is_sequence_repr(lr) {
                    (ll, lr.clone(), rl, rr.clone())
                } else if is_sequence_repr(rr) {
                    (rl, rr.clone(), ll, lr.clone())
                } else {
                    return Ok(None);
                };
                let cop = match &seq_repr {
                    Repr::Heap(List(_)) => ContainerOp::ListRepeat,
                    Repr::Heap(Bytes) => ContainerOp::BytesRepeat,
                    // No `rt_tuple_repeat` in the runtime → leave `tuple * int` to
                    // the tagged baseline.
                    _ => return Ok(None),
                };
                let (dst, ret) = self.emit_container(
                    cop,
                    vec![(seq, seq_repr.clone()), (count, count_repr)],
                    Some(seq_repr),
                )?;
                Ok(Some((dst.expect("repeat produces a container"), ret)))
            }
            _ => Ok(None),
        }
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
        let (rl, rr) = self.lower_expr(r)?;

        // Container *ordering* (`<` `<=` `>` `>=`) on list / tuple: the tagged
        // `rt_obj_cmp` raises `TypeError` on them, so dispatch to the typed runtime
        // comparator. `==` / `!=` (and bytes / str ordering) ride the tagged
        // baseline below — `rt_obj_eq` already compares containers structurally.
        if matches!(op, HCmpOp::Lt | HCmpOp::LtE | HCmpOp::Gt | HCmpOp::GtE) {
            let cop = match (&lr, &rr) {
                (Repr::Heap(HeapShape::List(_)), Repr::Heap(HeapShape::List(_))) => {
                    Some(ContainerOp::ListCmp(op))
                }
                (
                    Repr::Heap(HeapShape::Tuple(_) | HeapShape::TupleVar(_)),
                    Repr::Heap(HeapShape::Tuple(_) | HeapShape::TupleVar(_)),
                ) => Some(ContainerOp::TupleCmp(op)),
                _ => None,
            };
            if let Some(cop) = cop {
                let (dst, ret) = self.emit_container(cop, vec![(ll, lr), (rl, rr)], None)?;
                return Ok((dst.expect("container compare produces a bool"), ret));
            }
        }

        let i64r = Repr::Raw(RawKind::I64);
        // Raw int compare (Phase 3c): when both operands are range-proven
        // `Raw(I64)` cursors (the `range()` loop guard `cursor < stop`), compare
        // them with a machine `icmp` — no tagging, no `rt_obj_cmp` call. Anything
        // else runs on the tagged baseline (a lone raw operand re-tags soundly).
        let (la, ra) = if lr == i64r && rr == i64r {
            (ll, rl)
        } else {
            (self.coerce(ll, lr, Repr::Tagged)?, self.coerce(rl, rr, Repr::Tagged)?)
        };
        let dst = self.alloc_temp(Repr::Raw(RawKind::I8));
        self.emit(MirInst::Compare {
            dst,
            op: map_cmpop(op),
            l: Operand::Local(la),
            r: Operand::Local(ra),
        });
        Ok((dst, Repr::Raw(RawKind::I8)))
    }

    fn lower_call(
        &mut self,
        call_idx: Idx<HirExpr>,
        callee: Idx<HirExpr>,
        args: Vec<Idx<HirExpr>>,
    ) -> Result<(LocalId, Repr)> {
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
            Symbol::Container(op) => self.lower_container_builtin(call_idx, op, &args),
            Symbol::BuiltinRange => self.lower_range_value(call_idx, &args, span),
            Symbol::BuiltinPrint | Symbol::Local(_) => {
                Err(CompilerError::semantic_error(
                    "this callee is not usable as a value-returning call here",
                    span,
                ))
            }
        }
    }

    /// Lower a container / iteration builtin resolved to `Symbol::Container`. Each
    /// op composes the runtime one-shot it needs: `iter()`-wrapping its iterable
    /// args, pre-materializing a list for `sorted`/`reversed`, or branching on the
    /// argument count for the constructors (`list()` empty vs `list(it)`).
    fn lower_container_builtin(
        &mut self,
        call_idx: Idx<HirExpr>,
        op: ContainerOp,
        args: &[Idx<HirExpr>],
    ) -> Result<(LocalId, Repr)> {
        use ContainerOp as C;
        let span = self.func.exprs[call_idx].span;
        let result_heap = repr_of(&self.func.exprs[call_idx].ty);
        match op {
            C::Len => {
                let (l, r) = self.lower_expr(args[0])?;
                let (dst, ret) = self.emit_container(C::Len, vec![(l, r)], None)?;
                self.normalize_container_result(dst.unwrap(), ret)
            }
            C::Enumerate => {
                let it = self.lower_iter_arg(args[0])?;
                let start = match args.get(1) {
                    Some(a) => self.lower_expr(*a)?,
                    None => (self.raw_i64_const(0), Repr::Raw(RawKind::I64)),
                };
                let (dst, ret) =
                    self.emit_container(C::Enumerate, vec![it, start], Some(result_heap))?;
                Ok((dst.unwrap(), ret))
            }
            C::Zip => {
                if args.len() != 2 {
                    return Err(CompilerError::semantic_error(
                        "zip() currently supports exactly two iterables",
                        span,
                    ));
                }
                let a = self.lower_iter_arg(args[0])?;
                let b = self.lower_iter_arg(args[1])?;
                let (dst, ret) = self.emit_container(C::Zip, vec![a, b], Some(result_heap))?;
                Ok((dst.unwrap(), ret))
            }
            C::Sorted => {
                let list = self.materialize_list(args[0])?;
                let (dst, ret) = self.emit_container(C::Sorted, vec![list], Some(result_heap))?;
                Ok((dst.unwrap(), ret))
            }
            C::Reversed => {
                let list = self.materialize_list(args[0])?;
                let (dst, ret) = self.emit_container(C::Reversed, vec![list], Some(result_heap))?;
                Ok((dst.unwrap(), ret))
            }
            C::ListFromIter => self.lower_constructor(call_idx, args, C::ListNew, C::ListFromIter),
            C::TupleFromIter => self.lower_constructor(call_idx, args, C::TupleNew, C::TupleFromIter),
            C::DictFromPairs => {
                if args.is_empty() {
                    return self.empty_container(C::DictNew, result_heap);
                }
                let (pl, pr) = self.lower_expr(args[0])?;
                let (dst, ret) =
                    self.emit_container(C::DictFromPairs, vec![(pl, pr)], Some(result_heap))?;
                Ok((dst.unwrap(), ret))
            }
            C::BytesFromList => {
                if args.is_empty() {
                    // `bytes()` → empty bytes via an empty backing list.
                    let (empty, _) = self.empty_container(
                        C::ListNew,
                        Repr::Heap(HeapShape::List(Box::new(Repr::Tagged))),
                    )?;
                    let (dst, ret) = self.emit_container(
                        C::BytesFromList,
                        vec![(empty, Repr::Heap(HeapShape::List(Box::new(Repr::Tagged))))],
                        Some(result_heap),
                    )?;
                    return Ok((dst.unwrap(), ret));
                }
                let (ll, lr) = self.lower_expr(args[0])?;
                let (dst, ret) =
                    self.emit_container(C::BytesFromList, vec![(ll, lr)], Some(result_heap))?;
                Ok((dst.unwrap(), ret))
            }
            other => Err(CompilerError::semantic_error(
                format!("internal: {other:?} is not a name-resolved container builtin"),
                span,
            )),
        }
    }

    /// Lower `range(...)` used as a value into a range iterator
    /// (`rt_iter_range(start, stop, step)`), defaulting start to 0 and step to 1.
    fn lower_range_value(
        &mut self,
        call_idx: Idx<HirExpr>,
        args: &[Idx<HirExpr>],
        span: pyaot_utils::Span,
    ) -> Result<(LocalId, Repr)> {
        let i64r = Repr::Raw(RawKind::I64);
        let (start, stop, step) = match args.len() {
            1 => {
                let stop = self.lower_index_arg(args[0])?;
                let start = self.raw_i64_const(0);
                let step = self.raw_i64_const(1);
                (start, stop, step)
            }
            2 => {
                let start = self.lower_index_arg(args[0])?;
                let stop = self.lower_index_arg(args[1])?;
                let step = self.raw_i64_const(1);
                (start, stop, step)
            }
            3 => {
                let start = self.lower_index_arg(args[0])?;
                let stop = self.lower_index_arg(args[1])?;
                let step = self.lower_index_arg(args[2])?;
                (start, stop, step)
            }
            _ => {
                return Err(CompilerError::semantic_error(
                    "range() takes 1 to 3 arguments",
                    span,
                ))
            }
        };
        let result_heap = repr_of(&self.func.exprs[call_idx].ty);
        let (dst, ret) = self.emit_container(
            ContainerOp::RangeIter,
            vec![(start, i64r.clone()), (stop, i64r.clone()), (step, i64r)],
            Some(result_heap),
        )?;
        Ok((dst.unwrap(), ret))
    }

    /// Lower an integer argument and untag it to `Raw(I64)` (a range bound/index).
    fn lower_index_arg(&mut self, arg: Idx<HirExpr>) -> Result<LocalId> {
        let (l, r) = self.lower_expr(arg)?;
        self.coerce_to_i64(l, r)
    }

    /// `list(it)` / `tuple(it)` constructor: 0 args → an empty container, 1 arg →
    /// materialize from the `iter()`-wrapped argument.
    fn lower_constructor(
        &mut self,
        call_idx: Idx<HirExpr>,
        args: &[Idx<HirExpr>],
        empty_op: ContainerOp,
        from_iter_op: ContainerOp,
    ) -> Result<(LocalId, Repr)> {
        let result_heap = repr_of(&self.func.exprs[call_idx].ty);
        if args.is_empty() {
            return self.empty_container(empty_op, result_heap);
        }
        let it = self.lower_iter_arg(args[0])?;
        let (dst, ret) = self.emit_container(from_iter_op, vec![it], Some(result_heap))?;
        Ok((dst.unwrap(), ret))
    }

    /// Build an empty container (`ListNew`/`DictNew`/`SetNew`/`TupleNew`) of the
    /// given heap representation.
    fn empty_container(&mut self, op: ContainerOp, heap: Repr) -> Result<(LocalId, Repr)> {
        let cap = self.raw_i64_const(0);
        let (dst, ret) =
            self.emit_container(op, vec![(cap, Repr::Raw(RawKind::I64))], Some(heap))?;
        Ok((dst.unwrap(), ret))
    }

    /// Lower an argument and wrap it in a runtime iterator (`iter(arg)`), returning
    /// the iterator local + its `Heap(Iterator)` representation.
    fn lower_iter_arg(&mut self, arg: Idx<HirExpr>) -> Result<(LocalId, Repr)> {
        let (l, r) = self.lower_expr(arg)?;
        let iter_repr = Repr::Heap(HeapShape::Iterator(Box::new(Repr::Tagged)));
        let (dst, _) = self.emit_container(ContainerOp::Iter, vec![(l, r)], Some(iter_repr.clone()))?;
        Ok((dst.unwrap(), iter_repr))
    }

    /// Materialize an argument into a fresh list: an existing list is used as-is
    /// (`sorted`/`reversed` do not mutate their input); anything else is built from
    /// its iterator via `rt_list_from_iter`.
    fn materialize_list(&mut self, arg: Idx<HirExpr>) -> Result<(LocalId, Repr)> {
        let (l, r) = self.lower_expr(arg)?;
        if matches!(r, Repr::Heap(HeapShape::List(_))) {
            return Ok((l, r));
        }
        let list_repr = Repr::Heap(HeapShape::List(Box::new(Repr::Tagged)));
        let iter_repr = Repr::Heap(HeapShape::Iterator(Box::new(Repr::Tagged)));
        let (it, _) = self.emit_container(ContainerOp::Iter, vec![(l, r)], Some(iter_repr))?;
        let (dst, _) = self.emit_container(
            ContainerOp::ListFromIter,
            vec![(it.unwrap(), Repr::Heap(HeapShape::Iterator(Box::new(Repr::Tagged))))],
            Some(list_repr.clone()),
        )?;
        Ok((dst.unwrap(), list_repr))
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
            Symbol::BuiltinPrint
            | Symbol::BuiltinRange
            | Symbol::Builtin(_)
            | Symbol::Function(_)
            | Symbol::Container(_) => Err(CompilerError::semantic_error(
                "this name cannot be used as a value here (only call targets are supported)",
                span,
            )),
        }
    }

    fn local_repr(&self, id: LocalId) -> Repr {
        self.locals[id.index()].repr.clone()
    }
}

/// The representation of a HIR local. Almost always `repr_of(ty)`, with two
/// overrides: the proof-gated Phase-3c `Raw(I64)` range cursor, and the
/// `pin_tagged` slot (an `iter_next` result, null on exhaustion) forced to
/// `Tagged` so the on-exhaustion store never unboxes null — the slot keeps its
/// inferred *type* (so the typed loop variable bound from it stays precise), only
/// its *representation* is pinned.
fn local_repr(l: &HirLocal) -> Repr {
    if l.pin_tagged {
        Repr::Tagged
    } else if l.raw_int_ok && l.ty == SemTy::Int {
        Repr::Raw(RawKind::I64)
    } else {
        repr_of(&l.ty)
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

/// The receiver family a container method dispatches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MethodRecv {
    List,
    Dict,
    Set,
    Other,
}

/// The container family a subscript dispatches to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubKind {
    List,
    Dict,
    Tuple,
    Bytes,
    Str,
    Generic,
}

/// Classify a subscript base from its static type first (which survives a nested
/// get that lowered the base into a uniform-tagged slot — `grid[0][1]`) and its
/// representation second.
fn sub_kind(ty: &SemTy, repr: &Repr) -> SubKind {
    if ty.list_elem().is_some() || matches!(repr, Repr::Heap(HeapShape::List(_))) {
        SubKind::List
    } else if ty.dict_kv().is_some() || matches!(repr, Repr::Heap(HeapShape::Dict(..))) {
        SubKind::Dict
    } else if ty.tuple_elems().is_some()
        || ty.tuple_var_elem().is_some()
        || matches!(repr, Repr::Heap(HeapShape::Tuple(_) | HeapShape::TupleVar(_)))
    {
        SubKind::Tuple
    } else if matches!(ty, SemTy::Bytes) || matches!(repr, Repr::Heap(HeapShape::Bytes)) {
        SubKind::Bytes
    } else if matches!(ty, SemTy::Str) || matches!(repr, Repr::Heap(HeapShape::Str)) {
        SubKind::Str
    } else {
        SubKind::Generic
    }
}

/// True for the repeatable sequence representations (`list` / `tuple` / `bytes`)
/// — the `*`-repeat operands.
fn is_sequence_repr(r: &Repr) -> bool {
    matches!(
        r,
        Repr::Heap(HeapShape::List(_) | HeapShape::Tuple(_) | HeapShape::TupleVar(_) | HeapShape::Bytes)
    )
}

#[cfg(test)]
mod tests;
