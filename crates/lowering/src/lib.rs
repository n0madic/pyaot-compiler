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
    BinOp as HBinOp, ClassTable, CmpOp as HCmpOp, ContainerArg, ContainerMethod, ContainerOp,
    ContainerResult, HirBlock, HirExpr, HirExprKind, HirFunction, HirLocal, HirModule, HirStmt,
    HirTerminator, ResolveResult, Symbol, SymbolRef, UnaryOp as HUnaryOp,
};
use pyaot_mir::{
    BinOp as MBinOp, CmpOp as MCmpOp, CoerceInst, Const, ExcQuery, GenOp, LocalDecl, MirBlock,
    MirClass, MirFunction, MirInst, MirProgram, MirRaise, MirTerminator, Operand, PrintKind,
    StrPool, UnaryOp as MUnaryOp,
};
use pyaot_types::{
    generic_sig, repr_of, HeapShape, RawKind, Repr, SemTy, SigRepr, RAW_I64_NARROW_BOUND,
};
use pyaot_utils::{BlockId, ClassId, FuncId, InternedString, LocalId, StringInterner};

/// Lower a resolved, inferred [`HirModule`] into a [`MirProgram`].
pub fn lower(
    module: &HirModule,
    resolve: &ResolveResult,
    interner: &StringInterner,
    classes: &ClassTable,
) -> Result<MirProgram> {
    let mut str_pool = StrPool::new();
    // Dunder method FuncIds: their MIR return is forced to `Tagged` so the
    // runtime's registry dispatch sees a uniform `fn(Value…) -> Value` ABI even
    // for `__eq__`/`__lt__`/`__truediv__` (whose declared returns are
    // `bool`/`float`, repr `Raw`, not i64) — PITFALLS B11.
    let mut dunder_funcs: std::collections::HashSet<FuncId> = std::collections::HashSet::new();
    for info in classes.iter() {
        for m in &info.methods {
            if is_dunder(interner.resolve(m.name)) {
                dunder_funcs.insert(m.func_id);
            }
        }
    }
    // Signature table (ABI = f(param Repr)), needed before lowering bodies so
    // calls — including forward / recursive ones — coerce args correctly.
    let sigs: Vec<FnSig> = module
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| FnSig {
            // Param repr derives from the param-LOCAL's `local_repr`, not
            // `repr_of(p.ty)`: a param proven raw-int by typeck's interprocedural
            // interval pass (`HirLocal::raw_int_ok`) takes `Raw(I64)`. This MUST
            // match the param-local table (`FnLower::new`) and the `MirFunction
            // .params` the callee declares — deriving all three from the same
            // `local_repr` keeps them in lockstep so the verifier never sees a
            // `Call.arg` ↔ `callee.params` mismatch (the ABI = f(Repr) seam).
            params: (0..f.params.len())
                .map(|p| local_repr(&f.locals[p]))
                .collect(),
            param_names: f.params.iter().map(|p| p.name).collect(),
            defaults: f.params.iter().map(|p| p.default.clone()).collect(),
            ret: if dunder_funcs.contains(&FuncId::new(i as u32)) {
                Repr::Tagged
            } else if f.ret_raw_int && f.ret_ty == SemTy::Int {
                // Proven raw-int return (interprocedural interval pass): the
                // signature returns an unboxed i64 and `Call.dst` follows suit.
                Repr::Raw(RawKind::I64)
            } else {
                repr_of(&f.ret_ty)
            },
            varargs: f.varargs,
            kwargs: f.kwargs,
        })
        .collect();
    let mut funcs = Vec::with_capacity(module.functions.len());
    for (i, func) in module.functions.iter().enumerate() {
        let ret_repr = sigs[i].ret.clone();
        let mut fl = FnLower::new(
            func,
            resolve,
            interner,
            &mut str_pool,
            &sigs,
            classes,
            &module.deletable_globals,
            &module.deletable_fields,
            &module.global_annotations,
            ret_repr,
        );
        funcs.push(fl.lower()?);
    }
    // The codegen-facing class registration data (`__pyaot_classinit`). Qualname
    // bytes go into the string pool so codegen can build the `StrObj`.
    let mir_classes = build_mir_classes(
        classes,
        interner,
        &mut str_pool,
        &module.method_uniform_thunks,
        &module.iternext_thunks,
    );
    Ok(MirProgram {
        funcs,
        entry: module.main,
        str_pool,
        classes: mir_classes,
        generators: module.generators.clone(),
    })
}

/// Assemble the [`MirClass`] registration records from the resolved [`ClassTable`],
/// interning each qualname's bytes into `str_pool`. Phase 5A populates identity +
/// parent + field_count; the vtable / method-name / dunder tables are filled by
/// 5B/5C (left empty here).
fn build_mir_classes(
    classes: &ClassTable,
    interner: &StringInterner,
    str_pool: &mut StrPool,
    method_uniform_thunks: &std::collections::HashMap<FuncId, FuncId>,
    iternext_thunks: &std::collections::HashMap<FuncId, FuncId>,
) -> Vec<MirClass> {
    let mut out = Vec::new();
    for info in classes.iter() {
        str_pool.insert(
            info.qualname,
            interner.resolve(info.qualname).as_bytes().to_vec(),
        );
        if info.is_exception_class() {
            str_pool.insert(info.name, interner.resolve(info.name).as_bytes().to_vec());
        }
        // Vtable: slot → resolved FuncId (5B). Each distinct method name occupies
        // a stable slot across the class and its subclasses.
        let mut vtable = vec![None; info.num_vtable_slots];
        let mut method_names = Vec::with_capacity(info.methods.len());
        let mut method_uniforms = Vec::new();
        let mut dunders = Vec::new();
        for m in &info.methods {
            vtable[m.slot] = Some(m.func_id);
            let name = interner.resolve(m.name);
            method_names.push((pyaot_utils::fnv1a_hash(name), m.slot));
            // Gradual-completeness method dispatch (Phase B): if this method (own
            // or inherited — keyed by its resolved FuncId) has a uniform thunk,
            // register it under THIS class id so `rt_obj_method` can invoke it on
            // a `Dyn` receiver. An inherited method's `func_id` is the base's, so
            // the base's thunk resolves; an override has its own thunk.
            if let Some(&thunk) = method_uniform_thunks.get(&m.func_id) {
                method_uniforms.push((pyaot_utils::fnv1a_hash(name), thunk));
            }
            // Register every dunder (own or inherited) under THIS class id so the
            // runtime's registry-dispatched ops (`rt_obj_add`/`rt_obj_neg`/the
            // default-repr path/…) resolve for instances of this exact class (5C).
            if is_dunder(name) {
                dunders.push((pyaot_utils::fnv1a_hash(name), m.func_id));
            }
        }
        let vtable: Vec<_> = vtable
            .into_iter()
            .map(|f| f.expect("every vtable slot is filled by a resolved method"))
            .collect();
        // Field-name registry (Phase 8H, D4): `(fnv1a(name), slot)` in the
        // SAME slot order as the static `GetField` path (`field_slot` =
        // position in `info.fields`). A 64-bit FNV-1a collision between two
        // field names of one class would silently alias them — check here
        // (cheap; classes have few fields).
        let mut field_names = Vec::with_capacity(info.fields.len());
        for (slot, fld) in info.fields.iter().enumerate() {
            let name = interner.resolve(fld.name);
            let hash = pyaot_utils::fnv1a_hash(name);
            if field_names.iter().any(|(h, _)| *h == hash) {
                panic!(
                    "FNV-1a-64 hash collision between fields of class `{}` (field `{}`)",
                    interner.resolve(info.name),
                    name
                );
            }
            field_names.push((hash, slot));
        }
        // Class-attribute initializers → MIR `Const`s (literal bytes interned).
        let class_attr_inits = info
            .class_attrs
            .iter()
            .map(|a| (a.attr_idx, class_attr_const(&a.init, interner, str_pool)))
            .collect();
        // Lazy user-class iterator protocol: if this class defines (own or
        // inherited) `__next__`, resolve its `<iternext>` thunk by the method's
        // FuncId. An inherited `__next__` reuses the base's FuncId, so it
        // resolves the base's thunk and registers it under this subclass id.
        let iternext_thunk = info
            .methods
            .iter()
            .find(|m| interner.resolve(m.name) == "__next__")
            .and_then(|m| iternext_thunks.get(&m.func_id).copied());
        out.push(MirClass {
            class_id: info.class_id,
            name: info.name,
            qualname: info.qualname,
            parent: info.parent,
            exception_base: info.exception_base.map(|k| k.tag()),
            field_count: info.field_count(),
            vtable,
            method_names,
            field_names,
            dunders,
            method_uniforms,
            class_attr_inits,
            iternext_thunk,
        });
    }
    // Deterministic order (the table is a HashMap) so codegen output is stable.
    out.sort_by_key(|c| c.class_id.0);
    out
}

/// A function's representation-level signature (ABI), plus the Phase-6C
/// variadic flags (the trailing `*args` tuple / `**kwargs` dict params) so
/// method-call sites can pack excess args (Phase 7D: `__exit__(self, *a)`).
struct FnSig {
    params: Vec<Repr>,
    /// Source parameter names (parallel to `params`, including `self` and the
    /// `*args`/`**kwargs` slots) — keyword → slot matching (Phase 10).
    param_names: Vec<InternedString>,
    /// Per-param default (parallel to `params`; `None` for `self` and for params
    /// without a default). Lets constructor / method calls fill missing trailing
    /// args (Phase 8E) the way direct function calls already do in the frontend.
    /// A `Slot` default reads a once-evaluated GC-rooted global (mutable/computed
    /// top-level defaults).
    defaults: Vec<Option<pyaot_hir::ParamDefault>>,
    ret: Repr,
    varargs: bool,
    kwargs: bool,
}

/// Per-function lowering state with a small MIR block builder.
struct FnLower<'a> {
    func: &'a HirFunction,
    resolve: &'a ResolveResult,
    interner: &'a StringInterner,
    str_pool: &'a mut StrPool,
    sigs: &'a [FnSig],
    classes: &'a ClassTable,
    /// Module globals a `del` unbinds (`var_id → name`). A `GlobalGet` of one of
    /// these is wrapped in `rt_check_bound` (kind=Global → NameError).
    deletable_globals: &'a HashMap<u32, InternedString>,
    /// Instance-field names a `del obj.attr` unbinds (by name). A field read of
    /// one of these is wrapped in `rt_check_bound` (kind=Attr → AttributeError).
    deletable_fields: &'a std::collections::HashSet<InternedString>,
    /// Annotated module-global slot types (`var_id → SemTy`). A `GlobalSet` into a
    /// `float`-annotated slot routes through `box_float_for_slot` so an int/bool
    /// value lands as a genuine `FloatObj` (the numeric tower, PLAN §8).
    global_annotations: &'a HashMap<u32, SemTy>,
    /// This function's actual MIR return repr (`Tagged` for dunder methods; B11).
    ret_repr: Repr,
    locals: Vec<LocalDecl>,
    /// Finalized + reserved MIR blocks (placeholders until sealed).
    blocks: Vec<MirBlock>,
    /// HIR block → its *first* MIR block id.
    block_map: HashMap<Idx<HirBlock>, BlockId>,
    /// Instructions accumulating for the current MIR block.
    cur_insts: Vec<MirInst>,
    cur_id: BlockId,
    /// Handler annotation (already block-mapped) of the HIR block being
    /// lowered — stamped onto every MIR block sealed while it is active.
    cur_handler: Option<BlockId>,
}

/// Wanted repr for a sequence-method argument (`str`/`bytes`): a tagged heap
/// value (sep / sub / prefix / chars), or a raw i64 (`maxsplit` / `tabsize` /
/// `count` / `start`/`end` — a count or bound that rides `Raw(I64)`, never a
/// tagged int misread as a width, B16). [`ArgWant::RawI64`] carries the value
/// substituted when the optional argument is ABSENT (its Python default in the
/// raw register class): `-1` (unlimited) for `maxsplit`/`count`, `8` for
/// `tabsize`, `0` for a `start` bound, `i64::MAX` for an `end` bound (the
/// runtime clamps it to the length). Required (non-optional) raw args never read
/// the default — `0` is conventional there.
#[derive(Clone, Copy)]
enum ArgWant {
    Tagged,
    RawI64(i64),
}

impl<'a> FnLower<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        func: &'a HirFunction,
        resolve: &'a ResolveResult,
        interner: &'a StringInterner,
        str_pool: &'a mut StrPool,
        sigs: &'a [FnSig],
        classes: &'a ClassTable,
        deletable_globals: &'a HashMap<u32, InternedString>,
        deletable_fields: &'a std::collections::HashSet<InternedString>,
        global_annotations: &'a HashMap<u32, SemTy>,
        ret_repr: Repr,
    ) -> Self {
        // MIR locals 0..nhir mirror the HIR locals (LocalId is preserved);
        // temporaries are appended after.
        let locals: Vec<LocalDecl> = func
            .locals
            .iter()
            .map(|l| LocalDecl {
                repr: local_repr(l),
            })
            .collect();
        FnLower {
            func,
            resolve,
            interner,
            str_pool,
            sigs,
            classes,
            deletable_globals,
            deletable_fields,
            global_annotations,
            ret_repr,
            locals,
            blocks: Vec::new(),
            block_map: HashMap::new(),
            cur_insts: Vec::new(),
            cur_id: BlockId::new(0),
            cur_handler: None,
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
            // Every MIR block materialized from this HIR block — including
            // the extra ones synthesized mid-stream (short-circuit, staged
            // loops) — carries the HIR block's handler annotation: a
            // synthesized call inside a protected block is just as protected.
            self.cur_handler = block.handler.map(|h| self.block_map[&h]);
            for stmt in &block.stmts {
                self.lower_stmt(stmt)?;
            }
            let term = self.lower_terminator(&block.term)?;
            self.seal(term);
        }

        // The declared param reprs are exactly the param-local reprs (MIR locals
        // 0..n_params mirror the params, computed once in `FnLower::new` via
        // `local_repr`). Reading them back here — instead of recomputing from
        // `repr_of(p.ty)` — guarantees the function signature, the entry-param
        // binding in codegen, and the caller-side `sigs` table all agree on a
        // raw-int param's `Raw(I64)` repr.
        let params = (0..self.func.params.len())
            .map(|i| self.locals[i].repr.clone())
            .collect();
        Ok(MirFunction {
            name: self.func.name,
            file: self.func.file,
            params,
            ret: self.ret_repr.clone(),
            locals: std::mem::take(&mut self.locals),
            blocks: std::mem::take(&mut self.blocks),
            entry,
        })
    }

    // ── block builder ──────────────────────────────────────────────────────

    /// Reserve a fresh MIR block slot (placeholder), returning its id.
    fn reserve_block(&mut self) -> BlockId {
        let id = BlockId::new(self.blocks.len() as u32);
        self.blocks.push(MirBlock {
            insts: Vec::new(),
            term: MirTerminator::Unreachable,
            handler: None,
        });
        id
    }

    /// Finalize the current block with `term`, stamping the active handler.
    fn seal(&mut self, term: MirTerminator) {
        let insts = std::mem::take(&mut self.cur_insts);
        self.blocks[self.cur_id.index()] = MirBlock {
            insts,
            term,
            handler: self.cur_handler,
        };
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
        // A class instance → a *different* class repr is a sub/superclass cast
        // (`Dog` into an `Animal` slot, `self` into a parent's `self`): route
        // through the tagged baseline (`Heap→Tagged` Noop, `Tagged→Heap`
        // reinterpret). Both legs are in the table; the nominal subtype relation
        // was validated at the typeck boundary (`check_reinterpret`).
        if is_class_repr(&from) && is_class_repr(&to) {
            let tagged = self.coerce(src, from, Repr::Tagged)?;
            return self.coerce(tagged, Repr::Tagged, to);
        }
        // `CoerceInst::new` IS the legality check (the same table behind
        // `legalize::coerce`): an illegal pair is unconstructible.
        let dst = self.alloc_temp(to.clone());
        let inst = CoerceInst::new(dst, Operand::Local(src), from.clone(), to.clone())
            .ok_or_else(|| cg_illegal(&from, &to))?;
        self.emit(MirInst::Coerce(inst));
        Ok(dst)
    }

    /// Coerce `src` (`from`) into the *existing* local `dst` (whose declared repr
    /// is `to`) — used for assignment and result-local stores.
    fn coerce_into(&mut self, dst: LocalId, src: LocalId, from: Repr, to: Repr) -> Result<()> {
        // A sub/superclass cast routes through the tagged baseline (see `coerce`).
        if from != to && is_class_repr(&from) && is_class_repr(&to) {
            let tagged = self.coerce(src, from, Repr::Tagged)?;
            return self.coerce_into(dst, tagged, Repr::Tagged, to);
        }
        let inst = CoerceInst::new(dst, Operand::Local(src), from.clone(), to.clone())
            .ok_or_else(|| cg_illegal(&from, &to))?;
        self.emit(MirInst::Coerce(inst));
        Ok(())
    }

    /// Coerce `src` (`from`, static type `ty`) into a fresh local of `want`,
    /// returning it.
    ///
    /// When `want` is a `Raw(F64)`/`Raw(I64)`/`Raw(I8)` register slot and the
    /// value is not statically that type, the conversion is a *real* one (an
    /// int/bool/gradual value into a `float` slot — the numeric tower, PLAN §8;
    /// or a gradual value into a stdlib raw-ABI param — Phase 8H, D3), so it
    /// takes the CHECKED unbox: box to `Tagged` then `CoerceInst::new_checked`,
    /// whose codegen calls `rt_unbox_float` / `rt_unbox_int` / `rt_unbox_bool`
    /// (TypeError on a wrong tag, bignum→f64 round-to-nearest) instead of
    /// reinterpreting bits.
    ///
    /// Likewise (PLAN §1), when `want` is a guard-backed `Heap(shape)` and the
    /// value is genuinely `Dyn`/`Union` (a gradual heap seam), it takes a CHECKED
    /// `Tagged → Heap(shape)` coercion whose codegen calls `rt_check_heap_kind` /
    /// `rt_check_instance` (TypeError at the boundary on a wrong shape) instead
    /// of the unchecked bit-identical `TaggedToHeap` reinterpret that would crash
    /// later at a container op.
    ///
    /// A statically proven value keeps the plain unchecked `coerce`. This is the
    /// single seam for checked coercion (used by runtime-arg passing, the return
    /// terminator, and `float`-local assignment).
    fn coerce_value(
        &mut self,
        src: LocalId,
        from: Repr,
        ty: &SemTy,
        want: Repr,
    ) -> Result<LocalId> {
        let needs_check = match &want {
            Repr::Raw(RawKind::F64) => *ty != SemTy::Float,
            // A `Tagged` int into a raw-i64 slot takes the CHECKED unbox
            // (`rt_unbox_int`): the value may dynamically be a heap `BigInt`, and an
            // unchecked `UntagInt` (`sshr 3`) on a bignum pointer is silently
            // garbage (PITFALLS B16/A6). A range-proven int is already `Raw(I64)`
            // (`from != Tagged`, a Noop here), so this fires only for an unproven
            // tagged int or a gradual `Dyn`/`Union` value reaching a raw-i64 param.
            Repr::Raw(RawKind::I64) => from == Repr::Tagged,
            // A bool slot (`Raw(I8)`) fed a gradual value takes the CHECKED unbox
            // (`rt_unbox_bool`, the third member of the checked family) — a
            // statically-proven `bool` keeps the plain `UntagBool`.
            Repr::Raw(RawKind::I8) => matches!(ty, SemTy::Dyn | SemTy::Union(_)),
            // PLAN §1: a genuinely-`Dyn` value flowing into a typed `Heap` slot
            // (builtin container / class instance) takes a CHECKED `Tagged →
            // Heap(shape)` coercion — `rt_check_heap_kind` / `rt_check_instance`
            // raise `TypeError` at the boundary instead of crashing later at the
            // first container op. Gated on `dyn_check().is_some()` (the
            // guard-less `BigInt`/`RuntimeObj`/`Iterator` shapes keep the
            // unchecked `TaggedToHeap` reinterpret). A statically-proven Heap
            // value (`from` already `Heap`) keeps the plain unchecked path.
            Repr::Heap(shape) => {
                matches!(ty, SemTy::Dyn | SemTy::Union(_))
                    && from == Repr::Tagged
                    && shape.dyn_check().is_some()
            }
            _ => false,
        };
        if !needs_check {
            return self.coerce(src, from, want);
        }
        let tagged = self.coerce(src, from, Repr::Tagged)?;
        let dst = self.alloc_temp(want.clone());
        // `needs_check` fires only for the guard-backed checked shapes — the Raw
        // unbox targets (F64/I64/I8) and the guarded `Heap` shapes — exactly
        // `new_checked`'s domain, so the `None` arm is unreachable by
        // construction, but a loud internal error beats an `unwrap`.
        let inst = CoerceInst::new_checked(dst, Operand::Local(tagged), Repr::Tagged, want.clone())
            .ok_or_else(|| {
                CompilerError::codegen_error(
                    format!("internal: checked coerce to non-unbox repr {want:?}"),
                    None,
                )
            })?;
        self.emit(MirInst::Coerce(inst));
        Ok(dst)
    }

    /// Coerce `src` (`from`, static type `val_ty`) into a *tagged* slot — a module
    /// global or an instance field — whose reader unboxes by the slot's annotated
    /// type `slot_ty`. The result is always `Tagged`.
    ///
    /// When the slot is `float` and the value is an `int`/`bool`/gradual (the
    /// numeric tower, PLAN §8), a bare `coerce(src, Tagged)` would store a tagged
    /// fixnum that the slot's unchecked `UnboxFloat` read later misreads as an f64
    /// (PITFALLS A2). So first coerce to a genuine f64 (the CHECKED `Tagged →
    /// Raw(F64)` unbox via `coerce_value`/`rt_unbox_float`, with a bignum arm),
    /// then re-box it (`Raw(F64) → Tagged` = `BoxFloat` → a real `FloatObj`),
    /// keeping the slot's read sound (A2). A statically-`float` value (or any
    /// non-float slot) takes the plain tag. Both the checked unbox and `BoxFloat`
    /// are `may_allocate`, and the boxed `FloatObj` is consumed by the immediately
    /// following `GlobalSet`/`SetField` store with no intervening allocation (B5).
    fn box_float_for_slot(
        &mut self,
        src: LocalId,
        from: Repr,
        val_ty: &SemTy,
        slot_ty: &SemTy,
    ) -> Result<LocalId> {
        if *slot_ty == SemTy::Float
            && matches!(
                val_ty,
                SemTy::Int | SemTy::Bool | SemTy::Dyn | SemTy::Union(_)
            )
        {
            let f64r = Repr::Raw(RawKind::F64);
            let f = self.coerce_value(src, from, val_ty, f64r.clone())?;
            return self.coerce(f, f64r, Repr::Tagged);
        }
        self.coerce(src, from, Repr::Tagged)
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
            HirStmt::Line(line) => {
                self.emit(MirInst::LineMarker(*line));
                Ok(())
            }
            HirStmt::Print { args, sep, end } => self.lower_print(args, *sep, *end),
            HirStmt::Expr(idx) => {
                // Evaluate for side effects; discard the result.
                let _ = self.lower_expr(*idx)?;
                Ok(())
            }
            HirStmt::Assign { target, value } => {
                let (vloc, vrepr) = self.lower_expr(*value)?;
                let target_repr = self.local_repr(*target);
                let ty = self.func.exprs[*value].ty.clone();
                // A gradual value into an annotated `: float`/`: bool` local is a
                // real (checked) coercion, not a bit reinterpret. Float is the
                // numeric tower (PLAN §8: int/bool/gradual → f64); bool is the
                // Dyn→`Raw(I8)` checked unbox (`rt_unbox_bool`).
                let raw_checked = matches!(
                    target_repr,
                    Repr::Raw(RawKind::F64) | Repr::Raw(RawKind::I8)
                );
                // PLAN §1 read-back seam: a genuinely-`Dyn` value (a `Dyn` global
                // / field / element read as `Tagged`) assigned into an annotated
                // guard-backed `Heap` local (`list`/`str`/`dict`/…/class instance)
                // takes the CHECKED `Tagged → Heap(shape)` coercion
                // (`rt_check_heap_kind`/`rt_check_instance`) instead of the
                // unchecked `TaggedToHeap` trust — the store analogue of the call/
                // return seams. Gated on a genuinely gradual source (`vrepr ==
                // Tagged`, `ty` gradual) so statically-typed `Heap` assignments —
                // including subclass class→class casts that need `coerce_into`'s
                // tagged-baseline reroute — keep the unchecked path untouched.
                let heap_checked = vrepr == Repr::Tagged
                    && matches!(ty, SemTy::Dyn | SemTy::Union(_))
                    && matches!(&target_repr, Repr::Heap(s) if s.dyn_check().is_some());
                if raw_checked || heap_checked {
                    // The shared helper only emits the runtime guard for a
                    // genuinely gradual source — a statically-proven value stays a
                    // Noop — then we store into the existing slot (a same-repr copy).
                    let v = self.coerce_value(vloc, vrepr, &ty, target_repr.clone())?;
                    self.coerce_into(*target, v, target_repr.clone(), target_repr)?;
                } else {
                    self.coerce_into(*target, vloc, vrepr, target_repr)?;
                }
                Ok(())
            }
            HirStmt::Assert { cond } => {
                // Truthiness branch: true → continue; false → fail block that
                // raises AssertionError (no message in Phase 2) and is unreachable.
                let cond_op = self.lower_cond(*cond)?;
                let ok = self.reserve_block();
                let fail = self.reserve_block();
                self.seal(MirTerminator::Branch {
                    cond: cond_op,
                    then: ok,
                    else_: fail,
                });
                self.switch(fail);
                self.emit(MirInst::AssertFail);
                self.seal(MirTerminator::Unreachable);
                self.switch(ok);
                Ok(())
            }
            HirStmt::SetItem { base, index, value } => self.lower_setitem(*base, *index, *value),
            HirStmt::DelItem { base, index } => self.lower_delitem(*base, *index),
            HirStmt::SetAttr { base, name, value } => self.lower_setattr(*base, *name, *value),
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
            HirStmt::ContainerInsert {
                container,
                key,
                value,
            } => {
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
            HirStmt::CellSet { cell, value } => {
                let (vl, vr) = self.lower_expr(*value)?;
                let vt = self.coerce(vl, vr, Repr::Tagged)?;
                let cr = self.local_repr(*cell);
                let ct = self.coerce(*cell, cr, Repr::Tagged)?;
                self.emit(MirInst::CellSet {
                    cell: Operand::Local(ct),
                    value: Operand::Local(vt),
                });
                Ok(())
            }
            HirStmt::GlobalSet { var_id, value } => {
                // A `float`-annotated global is a tagged slot read back via an
                // unchecked `UnboxFloat`, so an int/bool/gradual value must be
                // coerced to a real `FloatObj` at the store (numeric tower, §8).
                let vty = self.func.exprs[*value].ty.clone();
                let slot_ty = self.global_annotations.get(var_id).cloned();
                let (vl, vr) = self.lower_expr(*value)?;
                let vt = match &slot_ty {
                    Some(st) => self.box_float_for_slot(vl, vr, &vty, st)?,
                    None => self.coerce(vl, vr, Repr::Tagged)?,
                };
                self.emit(MirInst::GlobalSet {
                    var_id: *var_id,
                    value: Operand::Local(vt),
                });
                Ok(())
            }
            // ── generators (Phase 6E) ──
            HirStmt::GenSetLocal { gen, slot, value } => {
                let g = self.lower_gen_operand(*gen)?;
                let (vl, vr) = self.lower_expr(*value)?;
                let vt = self.coerce(vl, vr, Repr::Tagged)?;
                self.emit(MirInst::GenOpInst {
                    dst: None,
                    op: GenOp::SetLocal,
                    gen: Operand::Local(g),
                    imm: *slot,
                    value: Some(Operand::Local(vt)),
                });
                Ok(())
            }
            HirStmt::GenSetState { gen, state } => {
                let g = self.lower_gen_operand(*gen)?;
                self.emit(MirInst::GenOpInst {
                    dst: None,
                    op: GenOp::SetState,
                    gen: Operand::Local(g),
                    imm: *state,
                    value: None,
                });
                Ok(())
            }
            HirStmt::GenSetExhausted { gen } => {
                let g = self.lower_gen_operand(*gen)?;
                self.emit(MirInst::GenOpInst {
                    dst: None,
                    op: GenOp::SetExhausted,
                    gen: Operand::Local(g),
                    imm: 0,
                    value: None,
                });
                Ok(())
            }
            // ── exceptions (Phase 7) ──
            HirStmt::ExcOp(op) => {
                self.emit(MirInst::ExcOp(*op));
                Ok(())
            }
            HirStmt::Raise(r) => self.lower_raise(r),
        }
    }

    /// Lower a `raise` (Phase 7A/7C). The frontend guarantees the `Raise` is
    /// the last statement of its block with an `Unreachable` terminator (the
    /// verifier re-checks).
    fn lower_raise(&mut self, r: &pyaot_hir::HirRaise) -> Result<()> {
        use pyaot_hir::HirRaise as H;
        match r {
            H::Builtin { tag, msg } => {
                let msg = self.lower_exc_msg(*msg)?;
                self.emit(MirInst::Raise(MirRaise::Builtin { tag: *tag, msg }));
            }
            H::BuiltinFromNone { tag, msg } => {
                let msg = self.lower_exc_msg(*msg)?;
                self.emit(MirInst::Raise(MirRaise::BuiltinFromNone { tag: *tag, msg }));
            }
            H::BuiltinFrom {
                tag,
                msg,
                cause_tag,
                cause_msg,
            } => {
                let msg = self.lower_exc_msg(*msg)?;
                let cause_msg = self.lower_exc_msg(*cause_msg)?;
                self.emit(MirInst::Raise(MirRaise::BuiltinFrom {
                    tag: *tag,
                    msg,
                    cause_tag: *cause_tag,
                    cause_msg,
                }));
            }
            H::Custom { class_id, args } => {
                let span = args
                    .first()
                    .map(|a| self.func.exprs[*a].span)
                    .unwrap_or_else(pyaot_utils::Span::dummy);
                let has_init = self.classes.get(*class_id).is_some_and(|info| {
                    info.methods
                        .iter()
                        .any(|m| self.interner.resolve(m.name) == "__init__")
                });
                if has_init {
                    // Construct + run __init__ at the raise site; the instance
                    // carries the user fields.
                    let (inst, irep) = self.lower_construct(*class_id, args, span)?;
                    let it = self.coerce(inst, irep, Repr::Tagged)?;
                    self.emit(MirInst::Raise(MirRaise::CustomWithInstance {
                        class_id: *class_id,
                        msg: None,
                        instance: Operand::Local(it),
                    }));
                } else {
                    // No __init__: a single argument becomes the message (so
                    // `str(e)` works); the instance is bare.
                    if args.len() > 1 {
                        return Err(CompilerError::semantic_error(
                            "multi-argument exceptions without __init__ are out of scope",
                            span,
                        ));
                    }
                    let msg = self.lower_exc_msg(args.first().copied())?;
                    let (inst, irep) = self.lower_construct(*class_id, &[], span)?;
                    let it = self.coerce(inst, irep, Repr::Tagged)?;
                    self.emit(MirInst::Raise(MirRaise::CustomWithInstance {
                        class_id: *class_id,
                        msg,
                        instance: Operand::Local(it),
                    }));
                }
            }
            H::Stdlib {
                class_id,
                exc_type_tag,
                msg,
            } => {
                let msg = self.lower_exc_msg(*msg)?;
                self.emit(MirInst::Raise(MirRaise::Stdlib {
                    class_id: *class_id,
                    exc_type_tag: *exc_type_tag,
                    msg,
                }));
            }
            H::Instance { value } => {
                let (vl, vr) = self.lower_expr(*value)?;
                let vt = self.coerce(vl, vr, Repr::Tagged)?;
                self.emit(MirInst::Raise(MirRaise::Instance {
                    value: Operand::Local(vt),
                }));
            }
            H::Reraise => self.emit(MirInst::Raise(MirRaise::Reraise)),
        }
        Ok(())
    }

    /// Lower a raise message operand to a Tagged `StrObj` (codegen reads its
    /// bytes via `rt_str_data`/`rt_str_len`; the runtime copies them — B2). A
    /// non-`str` message converts through the builtin `str`.
    fn lower_exc_msg(&mut self, msg: Option<Idx<HirExpr>>) -> Result<Option<Operand>> {
        let Some(e) = msg else { return Ok(None) };
        let is_str = self.func.exprs[e].ty == SemTy::Str;
        let (l, r) = self.lower_expr(e)?;
        let t = self.coerce(l, r, Repr::Tagged)?;
        if is_str {
            return Ok(Some(Operand::Local(t)));
        }
        let dst = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::CallBuiltin {
            dst: Some(dst),
            kind: pyaot_mir::BuiltinFunctionKind::Str,
            args: vec![Operand::Local(t)],
        });
        Ok(Some(Operand::Local(dst)))
    }

    /// Lower a generator operand expr to a `Tagged` local (the generator value).
    fn lower_gen_operand(&mut self, gen: Idx<HirExpr>) -> Result<LocalId> {
        let (gl, gr) = self.lower_expr(gen)?;
        self.coerce(gl, gr, Repr::Tagged)
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
        // Class `__setitem__` (Phase 5C) — a direct devirtualized call.
        let bt = self.func.exprs[base].ty.clone();
        if let Some(fid) = self.concrete_dunder(&bt, "__setitem__") {
            let (bl, br) = self.lower_expr(base)?;
            let (il, ir) = self.lower_expr(index)?;
            let (vl, vr) = self.lower_expr(value)?;
            self.emit_dunder_call(fid, vec![(bl, br), (il, ir), (vl, vr)])?;
            return Ok(());
        }
        // Counter item assignment (§10): `counter[key] = v` writes directly into
        // the shared `DictObj` via the dict setter (the dict family shares layout).
        // This also backs `counter[key] += n` (read via `rt_counter_get`, write here).
        if matches!(&bt, SemTy::RuntimeObject(t) if *t == pyaot_core_defs::TypeTagKind::Counter) {
            let (bl, br) = self.lower_expr(base)?;
            let (il, ir) = self.lower_expr(index)?;
            let (vl, vr) = self.lower_expr(value)?;
            self.emit_container(
                ContainerOp::DictSet,
                vec![(bl, br), (il, ir), (vl, vr)],
                None,
            )?;
            return Ok(());
        }
        // deque item assignment (§10): `dq[i] = v` — O(1) ring-buffer write
        // (negative indices + bounds checks inside `rt_deque_set`). The index is a
        // RAW i64, the value Tagged. `sub_kind` would classify a deque as `Generic`
        // and reject the assignment, so handle it before that dispatch.
        if matches!(&bt, SemTy::RuntimeObject(t) if *t == pyaot_core_defs::TypeTagKind::Deque) {
            use pyaot_core_defs::runtime_func_def as rf;
            let (bl, br) = self.lower_expr(base)?;
            let recv = self.coerce(bl, br, Repr::Tagged)?;
            let (il, ir) = self.lower_expr(index)?;
            let idx = self.coerce_to_i64(il, ir)?;
            let (vl, vr) = self.lower_expr(value)?;
            let val = self.coerce(vl, vr, Repr::Tagged)?;
            self.emit(MirInst::CallRuntime {
                dst: None,
                def: &rf::RT_DEQUE_SET,
                args: vec![
                    Operand::Local(recv),
                    Operand::Local(idx),
                    Operand::Local(val),
                ],
            });
            return Ok(());
        }
        let kind = sub_kind(
            &self.func.exprs[base].ty,
            &repr_of(&self.func.exprs[base].ty),
        );
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

    /// Lower `del base[index]`, dispatching the runtime deleter from the static
    /// container type (mirrors [`Self::lower_setitem`]). A class `__delitem__`
    /// takes a direct devirtualized call; tuple/str/bytes are a compile error.
    /// Emits `MirInst::CallRuntime` directly (the [Tagged, Raw] index ABI of the
    /// list/any deleters matches `RT_FILE_READ_N`, so no `ContainerOp` is
    /// needed).
    fn lower_delitem(&mut self, base: Idx<HirExpr>, index: Idx<HirExpr>) -> Result<()> {
        use pyaot_core_defs::runtime_func_def as rf;
        let span = self.func.exprs[base].span;
        // Class `__delitem__` (a user container) — a direct devirtualized call.
        let bt = self.func.exprs[base].ty.clone();
        if let Some(fid) = self.concrete_dunder(&bt, "__delitem__") {
            let (bl, br) = self.lower_expr(base)?;
            let (il, ir) = self.lower_expr(index)?;
            self.emit_dunder_call(fid, vec![(bl, br), (il, ir)])?;
            return Ok(());
        }
        let kind = sub_kind(
            &self.func.exprs[base].ty,
            &repr_of(&self.func.exprs[base].ty),
        );
        // Lower both operands left-to-right, then coerce per the dispatched ABI.
        let (bl, br) = self.lower_expr(base)?;
        let (il, ir) = self.lower_expr(index)?;
        let base_op = self.coerce(bl, br, Repr::Tagged)?;
        match kind {
            SubKind::List => {
                let idx = self.coerce_to_i64(il, ir)?;
                self.emit(MirInst::CallRuntime {
                    dst: None,
                    def: &rf::RT_LIST_DELETE,
                    args: vec![Operand::Local(base_op), Operand::Local(idx)],
                });
            }
            SubKind::Dict => {
                let key = self.coerce(il, ir, Repr::Tagged)?;
                self.emit(MirInst::CallRuntime {
                    dst: None,
                    def: &rf::RT_DICT_DELETE,
                    args: vec![Operand::Local(base_op), Operand::Local(key)],
                });
            }
            // Unknown base (deque / gradual `Dyn`). A statically-`str` key is a
            // MAPPING delete → the Tagged-key dict deleter; otherwise the
            // tag-dispatched sequence deleter takes a RAW i64 index (the same
            // split `lower_subscript` makes for `rt_any_getitem`).
            SubKind::Generic if matches!(self.func.exprs[index].ty, SemTy::Str) => {
                let key = self.coerce(il, ir, Repr::Tagged)?;
                self.emit(MirInst::CallRuntime {
                    dst: None,
                    def: &rf::RT_DICT_DELETE,
                    args: vec![Operand::Local(base_op), Operand::Local(key)],
                });
            }
            SubKind::Generic => {
                let idx = self.coerce_to_i64(il, ir)?;
                self.emit(MirInst::CallRuntime {
                    dst: None,
                    def: &rf::RT_ANY_DELITEM,
                    args: vec![Operand::Local(base_op), Operand::Local(idx)],
                });
            }
            SubKind::Tuple | SubKind::Str | SubKind::Bytes => {
                return Err(CompilerError::semantic_error(
                    "object doesn't support item deletion",
                    span,
                ));
            }
        }
        Ok(())
    }

    /// Wrap a tagged `value` read out of a `del`-able slot in the
    /// `rt_check_bound` guard: it returns the value unchanged unless the value
    /// is `Value::UNBOUND`, in which case it raises by `kind` (0 → local /
    /// UnboundLocalError, 1 → global / NameError, 2 → attr / AttributeError).
    /// `name` names the slot/attribute (for the message). Returns the guarded
    /// (still Tagged) value local.
    fn emit_check_bound(
        &mut self,
        value: LocalId,
        kind: i64,
        name: InternedString,
    ) -> Result<LocalId> {
        let kind_const = self.raw_i64_const(kind);
        self.str_pool
            .insert(name, self.interner.resolve(name).as_bytes().to_vec());
        let name_str = self.alloc_temp(Repr::Heap(HeapShape::Str));
        self.emit(MirInst::Const {
            dst: name_str,
            val: Const::Str(name),
        });
        let name_op = self.coerce(name_str, Repr::Heap(HeapShape::Str), Repr::Tagged)?;
        let dst = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::CallRuntime {
            dst: Some(dst),
            def: &pyaot_core_defs::runtime_func_def::RT_CHECK_BOUND,
            args: vec![
                Operand::Local(value),
                Operand::Local(kind_const),
                Operand::Local(name_op),
            ],
        });
        Ok(dst)
    }

    fn lower_print(
        &mut self,
        args: &[Idx<HirExpr>],
        sep: Option<pyaot_utils::InternedString>,
        end: Option<pyaot_utils::InternedString>,
    ) -> Result<()> {
        // Evaluate ALL argument expressions before writing anything (CPython
        // order: a side-effecting argument's output precedes the whole line).
        // String conversion still happens per-argument during the write, which
        // matches CPython's `print` converting as it writes.
        let mut vals = Vec::with_capacity(args.len());
        for arg_idx in args {
            let (loc, repr) = self.lower_expr(*arg_idx)?;
            vals.push((*arg_idx, loc, repr));
        }
        for (i, (arg_idx, loc, repr)) in vals.into_iter().enumerate() {
            if i > 0 {
                match sep {
                    None => self.emit(MirInst::Print {
                        kind: PrintKind::Sep,
                        arg: None,
                    }),
                    Some(id) => self.emit_print_str(id),
                }
            }
            self.lower_print_arg(arg_idx, loc, repr)?;
        }
        match end {
            None => self.emit(MirInst::Print {
                kind: PrintKind::Newline,
                arg: None,
            }),
            Some(id) => self.emit_print_str(id),
        }
        Ok(())
    }

    /// Print one already-evaluated argument with the `PrintKind` selected from
    /// its `SemTy`.
    fn lower_print_arg(&mut self, arg_idx: Idx<HirExpr>, loc: LocalId, repr: Repr) -> Result<()> {
        let ty = self.func.exprs[arg_idx].ty.clone();
        // A concrete class with `__str__`/`__repr__` (CPython print precedence):
        // the runtime's top-level print path renders the *default* repr for
        // instances, so route to the user dunder directly (5C).
        if let Some(fid) = self
            .concrete_dunder(&ty, "__str__")
            .or_else(|| self.concrete_dunder(&ty, "__repr__"))
        {
            let (res, rrep) = self.emit_dunder_call(fid, vec![(loc, repr)])?;
            let tagged = self.coerce(res, rrep, Repr::Tagged)?;
            self.emit(MirInst::Print {
                kind: PrintKind::StrObj,
                arg: Some(Operand::Local(tagged)),
            });
            return Ok(());
        }
        // `print(e)` of a caught exception prints its message (Phase 7B) —
        // the generic instance print would render the default object repr.
        if self.is_exception_value(&ty) {
            let vt = self.coerce(loc, repr, Repr::Tagged)?;
            let s = self.alloc_temp(Repr::Heap(HeapShape::Str));
            self.emit(MirInst::ExcInstanceStr {
                dst: s,
                value: Operand::Local(vt),
            });
            let tagged = self.coerce(s, Repr::Heap(HeapShape::Str), Repr::Tagged)?;
            self.emit(MirInst::Print {
                kind: PrintKind::StrObj,
                arg: Some(Operand::Local(tagged)),
            });
            return Ok(());
        }
        let (kind, want) = print_dispatch(&ty);
        match want {
            // No-operand kinds (None_): value already evaluated for side effects.
            None => self.emit(MirInst::Print { kind, arg: None }),
            Some(want_repr) => {
                let coerced = self.coerce(loc, repr, want_repr)?;
                self.emit(MirInst::Print {
                    kind,
                    arg: Some(Operand::Local(coerced)),
                });
            }
        }
        Ok(())
    }

    /// Emit `print(<str literal>)` with no separator/newline (used for custom
    /// `sep=`/`end=` strings).
    fn emit_print_str(&mut self, id: pyaot_utils::InternedString) {
        self.str_pool
            .insert(id, self.interner.resolve(id).as_bytes().to_vec());
        let s = self.alloc_temp(Repr::Heap(HeapShape::Str));
        self.emit(MirInst::Const {
            dst: s,
            val: Const::Str(id),
        });
        // Heap(Str) → Tagged is a free no-op coercion via legalize.
        let tagged = self
            .coerce(s, Repr::Heap(HeapShape::Str), Repr::Tagged)
            .expect("Heap(Str)->Tagged is always legal");
        self.emit(MirInst::Print {
            kind: PrintKind::StrObj,
            arg: Some(Operand::Local(tagged)),
        });
    }

    // ── terminators ──────────────────────────────────────────────────────────

    fn lower_terminator(&mut self, term: &HirTerminator) -> Result<MirTerminator> {
        match term {
            HirTerminator::Return(None) => Ok(MirTerminator::Return(None)),
            HirTerminator::Return(Some(idx)) => {
                let (loc, repr) = self.lower_expr(*idx)?;
                let want = self.ret_repr.clone();
                // Numeric tower (PLAN §8): an int/bool/gradual value through a
                // `-> float` slot is a real (checked) coercion, not a noop.
                let ty = self.func.exprs[*idx].ty.clone();
                let coerced = self.coerce_value(loc, repr, &ty, want)?;
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
        self.emit(MirInst::Truthy {
            dst,
            operand: Operand::Local(tagged),
        });
        Ok(Operand::Local(dst))
    }

    // ── expressions ──────────────────────────────────────────────────────────

    /// Lower an expression, returning its result local and that local's `Repr`.
    fn lower_expr(&mut self, idx: Idx<HirExpr>) -> Result<(LocalId, Repr)> {
        let expr = &self.func.exprs[idx];
        match &expr.kind {
            HirExprKind::StrLit(id) => {
                let id = *id;
                self.str_pool
                    .insert(id, self.interner.resolve(id).as_bytes().to_vec());
                let dst = self.alloc_temp(Repr::Heap(HeapShape::Str));
                self.emit(MirInst::Const {
                    dst,
                    val: Const::Str(id),
                });
                Ok((dst, Repr::Heap(HeapShape::Str)))
            }
            HirExprKind::IntLit(v) => {
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::Const {
                    dst,
                    val: Const::Int(*v),
                });
                Ok((dst, Repr::Tagged))
            }
            HirExprKind::BigIntLit(id) => {
                let id = *id;
                self.str_pool
                    .insert(id, self.interner.resolve(id).as_bytes().to_vec());
                let dst = self.alloc_temp(Repr::Heap(HeapShape::BigInt));
                self.emit(MirInst::Const {
                    dst,
                    val: Const::BigIntStr(id),
                });
                Ok((dst, Repr::Heap(HeapShape::BigInt)))
            }
            HirExprKind::FloatLit(f) => {
                let dst = self.alloc_temp(Repr::Raw(RawKind::F64));
                self.emit(MirInst::Const {
                    dst,
                    val: Const::Float(*f),
                });
                Ok((dst, Repr::Raw(RawKind::F64)))
            }
            HirExprKind::BoolLit(b) => {
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::Const {
                    dst,
                    val: Const::Bool(*b),
                });
                Ok((dst, Repr::Tagged))
            }
            HirExprKind::NoneLit => {
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::Const {
                    dst,
                    val: Const::None,
                });
                Ok((dst, Repr::Tagged))
            }
            // `NotImplemented` (§4a) → the runtime singleton. Always Tagged (the
            // dunder-fallback protocol consumes it as a `Value`); GC-rooting is
            // derived from the Tagged repr like any heap object.
            HirExprKind::NotImplementedLit => {
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::CallRuntime {
                    dst: Some(dst),
                    def: &pyaot_core_defs::runtime_func_def::RT_NOT_IMPLEMENTED_SINGLETON,
                    args: vec![],
                });
                Ok((dst, Repr::Tagged))
            }
            // `object.__new__(cls)` (§3) → `rt_object_new(cls)`. The `cls`
            // operand (a `cls`-as-int value) is untagged to a raw i64 class id
            // (`Tagged → Raw(I64)` is `UntagInt`; `Tagged → Raw(I8)` would wrongly
            // bit-mask it as a bool). The result is a fresh heap instance ptr
            // (Tagged, GC-rooted via the dst local).
            HirExprKind::ObjectNew { cls } => {
                let (cl, cr) = self.lower_expr(*cls)?;
                let cls_raw = self.coerce(cl, cr, Repr::Raw(RawKind::I64))?;
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::CallRuntime {
                    dst: Some(dst),
                    def: &pyaot_core_defs::runtime_func_def::RT_OBJECT_NEW,
                    args: vec![Operand::Local(cls_raw)],
                });
                Ok((dst, Repr::Tagged))
            }
            HirExprKind::Unbound => {
                // The `Value::UNBOUND` sentinel a `del` stores into the slot.
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::Const {
                    dst,
                    val: Const::Unbound,
                });
                Ok((dst, Repr::Tagged))
            }
            HirExprKind::Name(symref) => self.lower_name(*symref, expr.span),
            HirExprKind::Local(lid) => {
                // A `del`'d local stays bound but may hold the UNBOUND sentinel —
                // guard the read (the slot is pinned Tagged, so the value is a
                // tagged `Value`). Kind 0 → UnboundLocalError.
                if self.func.locals[lid.index()].deletable {
                    let name = self.func.locals[lid.index()].name;
                    let guarded = self.emit_check_bound(*lid, 0, name)?;
                    Ok((guarded, Repr::Tagged))
                } else {
                    Ok((*lid, self.local_repr(*lid)))
                }
            }
            HirExprKind::BinOp { op, l, r } => self.lower_binop(idx, *op, *l, *r),
            HirExprKind::Unary { op, operand } => self.lower_unary(*op, *operand),
            HirExprKind::Compare { op, l, r } => self.lower_compare(*op, *l, *r),
            HirExprKind::Call { callee, args } => self.lower_call(idx, *callee, args.clone()),
            HirExprKind::CallValue {
                callee,
                args,
                kwargs,
            } => self.lower_call_value(*callee, *args, *kwargs),
            // ── containers (Phase 4) ──
            HirExprKind::ListLit { elems } => self.lower_list_lit(idx, &elems.clone()),
            HirExprKind::SetLit { elems } => self.lower_set_lit(idx, &elems.clone()),
            HirExprKind::TupleLit { elems } => self.lower_tuple_lit(idx, &elems.clone()),
            HirExprKind::DictLit { pairs } => self.lower_dict_lit(idx, &pairs.clone()),
            HirExprKind::BytesLit(id) => {
                let id = *id;
                // Read back as RAW bytes — a `bytes` literal may not be valid UTF-8
                // (`b"\xff"`), so `resolve(id).as_bytes()` would panic in the interner.
                self.str_pool
                    .insert(id, self.interner.resolve_bytes(id).to_vec());
                let dst = self.alloc_temp(Repr::Heap(HeapShape::Bytes));
                self.emit(MirInst::Const {
                    dst,
                    val: Const::Bytes(id),
                });
                Ok((dst, Repr::Heap(HeapShape::Bytes)))
            }
            HirExprKind::Subscript { base, index } => self.lower_subscript(*base, *index),
            HirExprKind::Slice {
                base,
                start,
                end,
                step,
            } => self.lower_slice(idx, *base, *start, *end, *step),
            HirExprKind::FormatValue { value, spec } => self.lower_format_value(idx, *value, *spec),
            HirExprKind::ContainerExpr { op, args } => {
                self.lower_container_expr(idx, *op, &args.clone())
            }
            HirExprKind::Sum { iterable, start } => self.lower_sum_expr(idx, *iterable, *start),
            HirExprKind::MethodCall {
                recv,
                method_name,
                args,
                kwargs,
            } => self.lower_method_call(idx, *recv, *method_name, &args.clone(), &kwargs.clone()),
            HirExprKind::Attribute { value, name } => self.lower_attribute(idx, *value, *name),
            HirExprKind::IsInstance { value, class_id } => self.lower_isinstance(*value, *class_id),
            HirExprKind::IsInstanceBuiltin { value, target } => {
                self.lower_isinstance_builtin(*value, target)
            }
            HirExprKind::HasAttr { value, name } => self.lower_hasattr(*value, *name),
            HirExprKind::GetAttrByName {
                value,
                name,
                default,
            } => self.lower_get_attr_by_name(idx, *value, *name, *default),
            HirExprKind::IsSubclass { sub, sup } => self.lower_issubclass(*sub, *sup),
            HirExprKind::IsNone { value } => {
                // `value is None` → `rt_is_none(value)` (recognizes both the
                // immediate None tag and a heap None object). Result is Raw(I8).
                let (vl, vr) = self.lower_expr(*value)?;
                let vt = self.coerce(vl, vr, Repr::Tagged)?;
                let dst = self.alloc_temp(Repr::Raw(RawKind::I8));
                self.emit(MirInst::CallRuntime {
                    dst: Some(dst),
                    def: &pyaot_core_defs::runtime_func_def::RT_IS_NONE,
                    args: vec![Operand::Local(vt)],
                });
                Ok((dst, Repr::Raw(RawKind::I8)))
            }
            HirExprKind::Is { l, r } => {
                // `l is r` → `rt_is(l, r)` (bit-identity; None's ABI encodings
                // are normalized). Both operands ride the Tagged baseline, like
                // `rt_is_none`. Result is Raw(I8).
                let (ll, lr) = self.lower_expr(*l)?;
                let lt = self.coerce(ll, lr, Repr::Tagged)?;
                let (rl, rr) = self.lower_expr(*r)?;
                let rt = self.coerce(rl, rr, Repr::Tagged)?;
                let dst = self.alloc_temp(Repr::Raw(RawKind::I8));
                self.emit(MirInst::CallRuntime {
                    dst: Some(dst),
                    def: &pyaot_core_defs::runtime_func_def::RT_IS,
                    args: vec![Operand::Local(lt), Operand::Local(rt)],
                });
                Ok((dst, Repr::Raw(RawKind::I8)))
            }
            HirExprKind::CallRuntime {
                target,
                args,
                provided,
            } => self.lower_call_runtime(idx, target, args, *provided),
            // `Stack[int](...)` lowers identically to `Stack(...)` — type args are
            // erased at repr (one shared physical layout for every instantiation).
            HirExprKind::GenericConstruct { class_id, args, .. } => {
                self.lower_construct(*class_id, &args.clone(), expr.span)
            }
            // `super()` is only valid as a MethodCall receiver (handled before the
            // receiver is lowered); standalone it is a usage error.
            HirExprKind::Super(_) => Err(CompilerError::semantic_error(
                "super() is only supported as `super().method(...)`".to_string(),
                expr.span,
            )),
            // ── closures / cells / globals (Phase 6) ──
            HirExprKind::MakeClosure { func, captures } => {
                self.lower_make_closure(*func, &captures.clone())
            }
            HirExprKind::MakeCell { init } => {
                let iv = match init {
                    Some(e) => {
                        let e = *e;
                        let (il, ir) = self.lower_expr(e)?;
                        self.coerce(il, ir, Repr::Tagged)?
                    }
                    None => self.none_temp(),
                };
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::MakeCell {
                    dst,
                    init: Operand::Local(iv),
                });
                Ok((dst, Repr::Tagged))
            }
            HirExprKind::CellGet { cell } => {
                let cell = *cell;
                let cr = self.local_repr(cell);
                let ct = self.coerce(cell, cr, Repr::Tagged)?;
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::CellGet {
                    dst,
                    cell: Operand::Local(ct),
                });
                // Uniform tagged cell storage: consumers legalize per their own
                // typed context (the same seam as container reads).
                Ok((dst, Repr::Tagged))
            }
            HirExprKind::GlobalGet { var_id } => {
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::GlobalGet {
                    dst,
                    var_id: *var_id,
                });
                // A `del`'d global may hold the UNBOUND sentinel — guard the read
                // (kind 1 → NameError). Globals are physically tagged.
                if let Some(name) = self.deletable_globals.get(var_id).copied() {
                    let guarded = self.emit_check_bound(dst, 1, name)?;
                    return Ok((guarded, Repr::Tagged));
                }
                Ok((dst, Repr::Tagged))
            }
            // ── generators (Phase 6E) ──
            HirExprKind::MakeGenerator { gen_id, num_locals } => {
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::MakeGenerator {
                    dst,
                    gen_id: *gen_id,
                    num_locals: *num_locals,
                });
                Ok((dst, Repr::Tagged))
            }
            HirExprKind::GenQuery {
                op,
                gen,
                imm,
                value,
            } => self.lower_gen_query(*op, *gen, *imm, *value),
            // ── exceptions (Phase 7) ──
            HirExprKind::ExcQuery(q) => match q {
                ExcQuery::Current => {
                    let dst = self.alloc_temp(Repr::Tagged);
                    self.emit(MirInst::ExcQuery { dst, query: *q });
                    Ok((dst, Repr::Tagged))
                }
                ExcQuery::MatchesBuiltin(_) | ExcQuery::MatchesClass(_) => {
                    let dst = self.alloc_temp(Repr::Raw(RawKind::I8));
                    self.emit(MirInst::ExcQuery { dst, query: *q });
                    Ok((dst, Repr::Raw(RawKind::I8)))
                }
            },
            HirExprKind::ExcInstanceStr { value } => {
                let (vl, vr) = self.lower_expr(*value)?;
                let vt = self.coerce(vl, vr, Repr::Tagged)?;
                let dst = self.alloc_temp(Repr::Heap(HeapShape::Str));
                self.emit(MirInst::ExcInstanceStr {
                    dst,
                    value: Operand::Local(vt),
                });
                Ok((dst, Repr::Heap(HeapShape::Str)))
            }
        }
    }

    /// Lower a value-producing generator query (Phase 6E). The `GenState` result
    /// is normalized from `Raw(I64)` to the tagged int baseline so it flows as
    /// an ordinary `int` (the state-dispatch compares it against fixnums). A
    /// `Close` op (result `None`) yields the tagged `None` singleton.
    fn lower_gen_query(
        &mut self,
        op: GenOp,
        gen: Idx<HirExpr>,
        imm: u32,
        value: Option<Idx<HirExpr>>,
    ) -> Result<(LocalId, Repr)> {
        let g = self.lower_gen_operand(gen)?;
        let val_op = match value {
            Some(v) => {
                let (vl, vr) = self.lower_expr(v)?;
                Some(Operand::Local(self.coerce(vl, vr, Repr::Tagged)?))
            }
            None => None,
        };
        if op.result() == pyaot_mir::GenResult::None {
            // `Close` — a mutating op used as a statement; yield `None`.
            self.emit(MirInst::GenOpInst {
                dst: None,
                op,
                gen: Operand::Local(g),
                imm,
                value: val_op,
            });
            return self.none_value();
        }
        let (dst, ret) = match op.result() {
            pyaot_mir::GenResult::Value => (self.alloc_temp(Repr::Tagged), Repr::Tagged),
            pyaot_mir::GenResult::Int => (
                self.alloc_temp(Repr::Raw(RawKind::I64)),
                Repr::Raw(RawKind::I64),
            ),
            pyaot_mir::GenResult::Bool => (
                self.alloc_temp(Repr::Raw(RawKind::I8)),
                Repr::Raw(RawKind::I8),
            ),
            pyaot_mir::GenResult::None => {
                return Err(CompilerError::semantic_error(
                    "internal: a mutating generator op cannot be a value".to_string(),
                    self.func.exprs[gen].span,
                ))
            }
        };
        self.emit(MirInst::GenOpInst {
            dst: Some(dst),
            op,
            gen: Operand::Local(g),
            imm,
            value: val_op,
        });
        if ret == Repr::Raw(RawKind::I64) {
            // Normalize the state to a tagged int.
            let t = self.coerce(dst, ret, Repr::Tagged)?;
            Ok((t, Repr::Tagged))
        } else {
            Ok((dst, ret))
        }
    }

    /// Lower `MakeClosure` (Phase 6A): the dst signature comes from the target
    /// function's MIR signature minus its env param 0 (the same `repr_of`-derived
    /// source the verifier checks against).
    fn lower_make_closure(
        &mut self,
        func: FuncId,
        captures: &[Idx<HirExpr>],
    ) -> Result<(LocalId, Repr)> {
        let fsig = &self.sigs[func.index()];
        let sig = SigRepr {
            params: fsig.params[1..].to_vec(),
            ret: Box::new(fsig.ret.clone()),
        };
        let dst_repr = Repr::Closure(Box::new(sig));
        let mut caps = Vec::with_capacity(captures.len());
        for c in captures {
            let (cl, cr) = self.lower_expr(*c)?;
            caps.push(Operand::Local(self.coerce(cl, cr, Repr::Tagged)?));
        }
        let dst = self.alloc_temp(dst_repr.clone());
        self.emit(MirInst::MakeClosure {
            dst,
            func,
            captures: caps,
        });
        Ok((dst, dst_repr))
    }

    /// Lower an indirect call through a callable value — the **single uniform
    /// value-call path** (the prior precise-`Sig` route is gone). The callee may
    /// be a `Callable` *or* a genuinely-`Dyn` value: every closure shares the one
    /// `Closure(GENERIC_SIG)` repr (slot 0 is the arity-generic uniform thunk), so
    /// this packs the positional args into a `tuple[Dyn, ...]`, passes the null
    /// kwargs sentinel (call-site keywords are out of scope here), coerces the
    /// callee to `Closure(GENERIC_SIG)`, and emits a `CallIndirect` carrying
    /// `GENERIC_SIG`. The result is the tagged baseline (`Dyn`); the consuming
    /// seam (`: bool` / `: float` / return) recovers precision via the Phase-1
    /// checked unbox. Runtime arg→param binding (defaults, `*args`, the checked
    /// float/bool unbox) happens inside the thunk, so a fixed-arity native closure
    /// is bound correctly here without any static arity/repr coercion.
    fn lower_indirect_call(
        &mut self,
        callee: Idx<HirExpr>,
        args: &[Idx<HirExpr>],
    ) -> Result<(LocalId, Repr)> {
        // §6: a concrete class instance with a `__call__` dunder → `obj(args)` ≡
        // `obj.__call__(args)`, a direct devirtualized method call (the runtime
        // closure ABI does not apply — the instance is not a closure value).
        let callee_ty = self.func.exprs[callee].ty.clone();
        if let Some(fid) = self.concrete_dunder(&callee_ty, "__call__") {
            let span = self.func.exprs[callee].span;
            let (cl, cr) = self.lower_expr(callee)?;
            let params = self.sigs[fid.index()].params.clone();
            let ret = self.sigs[fid.index()].ret.clone();
            let self_arg = self.coerce(cl, cr, params[0].clone())?;
            let mut argvals = vec![Operand::Local(self_arg)];
            argvals.extend(self.build_call_operands(fid, true, args, &[], span)?);
            let dst = self.alloc_temp(ret.clone());
            self.emit(MirInst::Call {
                dst: Some(dst),
                func: fid,
                args: argvals,
            });
            return Ok((dst, ret));
        }
        let sig = generic_sig();
        let closure_repr = Repr::Closure(Box::new(sig.clone()));
        let (cl, cr) = self.lower_expr(callee)?;
        let ccl = self.coerce(cl, cr, closure_repr)?;

        // Pack the positional args into a `tuple[Dyn, ...]` (Tagged elements).
        let tup_repr = Repr::Heap(HeapShape::TupleVar(Box::new(Repr::Tagged)));
        let size = self.raw_i64_const(args.len() as i64);
        let (tup, _) = self.emit_container(
            ContainerOp::TupleNew,
            vec![(size, Repr::Raw(RawKind::I64))],
            Some(tup_repr.clone()),
        )?;
        let tup = tup.expect("TupleNew produces a tuple");
        for (i, a) in args.iter().enumerate() {
            let (al, ar) = self.lower_expr(*a)?;
            let pos = self.raw_i64_const(i as i64);
            self.emit_container(
                ContainerOp::TupleSet,
                vec![
                    (tup, tup_repr.clone()),
                    (pos, Repr::Raw(RawKind::I64)),
                    (al, ar),
                ],
                None,
            )?;
        }
        let args_tuple = self.coerce(tup, tup_repr, Repr::Tagged)?;

        // No call-site keywords on the uniform path → the null `__kwargs__`
        // sentinel (no allocation; the thunk reads it only when `F` has
        // keyword-only / `**kwargs` params, which a value call never supplies).
        let kwargs = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::Const {
            dst: kwargs,
            val: Const::NullPtr,
        });

        let dst = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::CallIndirect {
            dst: Some(dst),
            callee: Operand::Local(ccl),
            args: vec![Operand::Local(args_tuple), Operand::Local(kwargs)],
            sig,
        });
        Ok((dst, Repr::Tagged))
    }

    /// Lower a **pre-packed** indirect call ([`HirExprKind::CallValue`]): the
    /// frontend already built the positional `args` tuple and optional `kwargs`
    /// dict (a `*seq` / `**dict` forward into a value callee), so this just coerces
    /// the callee to `Closure(GENERIC_SIG)` and the two operands to `Tagged`, then
    /// emits the uniform `CallIndirect`. Result is the tagged baseline (`Dyn`).
    fn lower_call_value(
        &mut self,
        callee: Idx<HirExpr>,
        args: Idx<HirExpr>,
        kwargs: Option<Idx<HirExpr>>,
    ) -> Result<(LocalId, Repr)> {
        let sig = generic_sig();
        let (cl, cr) = self.lower_expr(callee)?;
        let ccl = self.coerce(cl, cr, Repr::Closure(Box::new(sig.clone())))?;
        let (al, ar) = self.lower_expr(args)?;
        let args_tagged = self.coerce(al, ar, Repr::Tagged)?;
        let kw_tagged = match kwargs {
            Some(k) => {
                let (kl, kr) = self.lower_expr(k)?;
                self.coerce(kl, kr, Repr::Tagged)?
            }
            None => {
                let n = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::Const {
                    dst: n,
                    val: Const::NullPtr,
                });
                n
            }
        };
        let dst = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::CallIndirect {
            dst: Some(dst),
            callee: Operand::Local(ccl),
            args: vec![Operand::Local(args_tagged), Operand::Local(kw_tagged)],
            sig,
        });
        Ok((dst, Repr::Tagged))
    }

    // ── classes (Phase 5) ────────────────────────────────────────────────────

    /// The class id a `Name` expr resolves to (`Symbol::Class`), for
    /// `ClassName.attr` / `ClassName.method()` class-level access (Phase 5D).
    fn class_name_ref(&self, idx: Idx<HirExpr>) -> Option<ClassId> {
        if let HirExprKind::Name(SymbolRef::Resolved(id)) = self.func.exprs[idx].kind {
            if let Symbol::Class(cid) = self.resolve.symbol(id) {
                return Some(cid);
            }
        }
        None
    }

    /// Resolve the field `name` to its slot on `base`'s static class type.
    fn field_slot(&self, base: Idx<HirExpr>, name: InternedString) -> Result<usize> {
        let span = self.func.exprs[base].span;
        let cid = class_of(&self.func.exprs[base].ty, self.classes).ok_or_else(|| {
            CompilerError::semantic_error(
                "attribute access requires a statically-known class instance".to_string(),
                span,
            )
        })?;
        let info = self.classes.get(cid).ok_or_else(|| {
            CompilerError::semantic_error("internal: unknown class id".to_string(), span)
        })?;
        info.field_slot(name)
            .ok_or_else(|| CompilerError::semantic_error(self.missing_attr_msg(cid, name), span))
    }

    /// Diagnostic for an unresolved attribute on class `cid`. Names the Phase-5D
    /// limitation when the attribute is actually an **inherited** decorated member
    /// or class attribute (5D resolves those own-only), rather than the generic
    /// "no field" message.
    fn missing_attr_msg(&self, cid: ClassId, name: InternedString) -> String {
        let nm = self.interner.resolve(name);
        if let Some(info) = self.classes.get(cid) {
            // Walk the strict ancestors; an own property / class attr / static or
            // class method there is an inheritance gap, not a typo.
            for ancestor in info.mro.iter().skip(1) {
                if let Some(ac) = self.classes.get(*ancestor) {
                    if ac.property(name).is_some()
                        || ac.class_attr(name).is_some()
                        || ac.static_method(name).is_some()
                        || ac.class_method(name).is_some()
                    {
                        return format!(
                            "`.{nm}` is an inherited decorated member / class attribute — \
                             inheritance of @property/@staticmethod/@classmethod/class \
                             attributes is a Phase 5D limitation (define it on this class)"
                        );
                    }
                }
            }
        }
        format!("class has no field `.{nm}`")
    }

    /// Lower an attribute read `value.name` → `GetField` + legalize the uniform
    /// tagged field value to the field's representation (the A5 read seam).
    /// `isinstance(value, str|int|float|bool)` (Phase 8B): folded statically
    /// from `value`'s inferred type. The corpus only asks about values whose
    /// type inference already proves; a gradual `Dyn` receiver would need a
    /// runtime tag query the frozen runtime does not expose — loud error.
    fn lower_isinstance_builtin(
        &mut self,
        value: Idx<HirExpr>,
        target: &SemTy,
    ) -> Result<(LocalId, Repr)> {
        let got = &self.func.exprs[value].ty;
        let span = self.func.exprs[value].span;
        let verdict = match got {
            // A gradual receiver cannot fold the verdict statically — inspect the
            // runtime tag via `rt_isinstance_builtin` (needed for the NI dunders,
            // whose `other` param is `Dyn`, e.g. `isinstance(other, (int, float))`).
            SemTy::Dyn | SemTy::Union(_) => {
                return self.lower_isinstance_builtin_runtime(value, target, span);
            }
            // Container builtins (`list`/`dict`/`set`/`tuple`) match by KIND —
            // isinstance ignores element types, so a `list[int]` value satisfies
            // `isinstance(x, list)` regardless of the canonical Dyn-element target.
            // (A fixed `tuple[A, B]` and a variable `tuple[T, ...]` are both `tuple`.)
            _ if target.list_elem().is_some() => got.list_elem().is_some(),
            _ if target.dict_kv().is_some() => got.dict_kv().is_some(),
            _ if target.set_elem().is_some() => got.set_elem().is_some(),
            _ if target.tuple_elems().is_some() || target.tuple_var_elem().is_some() => {
                got.tuple_elems().is_some() || got.tuple_var_elem().is_some()
            }
            // bool ⊂ int in Python: `isinstance(True, int)` is True.
            SemTy::Bool if *target == SemTy::Int => true,
            t => t == target,
        };
        // Evaluate the receiver for side effects, then materialize the verdict.
        let _ = self.lower_expr(value)?;
        let tagged = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::Const {
            dst: tagged,
            val: Const::Bool(verdict),
        });
        let dst = self.coerce(tagged, Repr::Tagged, Repr::Raw(RawKind::I8))?;
        Ok((dst, Repr::Raw(RawKind::I8)))
    }

    /// Runtime `isinstance(value, T)` against a builtin `T` for a gradual
    /// (`Dyn`/`Union`) receiver — `rt_isinstance_builtin(value, kind)`. The
    /// `kind` maps the target SemTy to a [`pyaot_core_defs::isinstance_kind`]
    /// code; an unmapped target (none of the canonical builtins) is a loud
    /// error. Needed for the NI dunders, whose `other` param is `Dyn` (e.g.
    /// `isinstance(other, (int, float))`).
    fn lower_isinstance_builtin_runtime(
        &mut self,
        value: Idx<HirExpr>,
        target: &SemTy,
        span: pyaot_utils::Span,
    ) -> Result<(LocalId, Repr)> {
        let kind = builtin_isinstance_kind(target).ok_or_else(|| {
            CompilerError::type_error(
                "isinstance() against this builtin type is out of scope for a gradual value",
                span,
            )
        })?;
        let (vl, vr) = self.lower_expr(value)?;
        let base = self.coerce(vl, vr, Repr::Tagged)?;
        let kind_local = self.alloc_temp(Repr::Raw(RawKind::I64));
        self.emit(MirInst::Const {
            dst: kind_local,
            val: Const::Int(kind),
        });
        let dst = self.alloc_temp(Repr::Raw(RawKind::I8));
        self.emit(MirInst::CallRuntime {
            dst: Some(dst),
            def: &pyaot_core_defs::runtime_func_def::RT_ISINSTANCE_BUILTIN,
            args: vec![Operand::Local(base), Operand::Local(kind_local)],
        });
        Ok((dst, Repr::Raw(RawKind::I8)))
    }

    /// `hasattr(value, "name")` (§5): folded statically from `value`'s
    /// `ClassInfo`. The verdict is true iff the name resolves to any member —
    /// field, method, `@property`, `@staticmethod`, or `@classmethod`. A
    /// `Dyn` / non-class receiver is a loud compile error (the same posture as
    /// [`Self::lower_isinstance_builtin`]: a runtime name-hash probe on a gradual
    /// value is out of scope). The receiver is still evaluated for side effects.
    fn lower_hasattr(
        &mut self,
        value: Idx<HirExpr>,
        name: InternedString,
    ) -> Result<(LocalId, Repr)> {
        let got = &self.func.exprs[value].ty;
        let span = self.func.exprs[value].span;
        let verdict = match class_of(got, self.classes) {
            Some(cid) => {
                let info = self.classes.get(cid).ok_or_else(|| {
                    CompilerError::semantic_error("hasattr() on unknown class", span)
                })?;
                info.field_slot(name).is_some()
                    || info.method(name).is_some()
                    || info.property(name).is_some()
                    || info.static_method(name).is_some()
                    || info.class_method(name).is_some()
                    || info.class_attr(name).is_some()
            }
            None => {
                return Err(CompilerError::type_error(
                    "hasattr() requires a statically-typed class instance \
                     (a runtime attribute probe on a gradual value is out of scope)",
                    span,
                ));
            }
        };
        // Evaluate the receiver for side effects, then materialize the verdict.
        let _ = self.lower_expr(value)?;
        let tagged = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::Const {
            dst: tagged,
            val: Const::Bool(verdict),
        });
        let dst = self.coerce(tagged, Repr::Tagged, Repr::Raw(RawKind::I8))?;
        Ok((dst, Repr::Raw(RawKind::I8)))
    }

    /// `issubclass(sub, sup)` (§5): folded statically via
    /// [`ClassTable::is_subclass`] (the C3-MRO check). Both classes are user
    /// classes resolved by the frontend; there is no receiver to evaluate.
    fn lower_issubclass(&mut self, sub: ClassId, sup: ClassId) -> Result<(LocalId, Repr)> {
        let verdict = self.classes.is_subclass(sub, sup);
        let tagged = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::Const {
            dst: tagged,
            val: Const::Bool(verdict),
        });
        let dst = self.coerce(tagged, Repr::Tagged, Repr::Raw(RawKind::I8))?;
        Ok((dst, Repr::Raw(RawKind::I8)))
    }

    /// Lower a stdlib runtime call through its declarative descriptor (Phase
    /// 8B) — the ONE generic seam. Each provided arg is legalized to the repr
    /// its `(TypeSpec, ParamType)` pair demands via the standard `coerce` path;
    /// an absent optional slot becomes the null-pointer `Value` sentinel; the
    /// user-written arg count rides as a trailing raw immediate when the hints
    /// say `pass_arg_count`. NOTE on raw `Int` params (`rt_math_gcd(i64, i64)`):
    /// the `UntagInt` coercion truncates a heap `BigInt` to its low word —
    /// accepted divergence beyond ±2^62 (plan risk #4), as CPython-exact bignum
    /// handling would need per-function runtime entry points.
    fn lower_call_runtime(
        &mut self,
        expr_idx: Idx<HirExpr>,
        target: &pyaot_hir::RuntimeCallTarget,
        args: &[Option<Idx<HirExpr>>],
        provided: u32,
    ) -> Result<(LocalId, Repr)> {
        use pyaot_hir::RuntimeCallTarget;
        let def = target.codegen();
        // `itertools.islice` needs the iterable `iter()`-wrapped (the runtime
        // advances it with the iterator protocol) and start/stop/step resolved
        // from the argument count — the one-numeric form is the STOP, not the
        // start. The generic positional path can express neither; the descriptor
        // declares this via the `slice_iterator` hint (not a symbol-name match).
        if matches!(target, RuntimeCallTarget::Func(f) if f.hints.slice_iterator) {
            return self.lower_islice(expr_idx, def, args, provided);
        }
        let (params, ret_spec, pass_arg_count): (
            &[pyaot_stdlib_defs::ParamDef],
            &pyaot_stdlib_defs::TypeSpec,
            bool,
        ) = match target {
            RuntimeCallTarget::Func(f) => (f.params, &f.return_type, f.hints.pass_arg_count),
            RuntimeCallTarget::Attr(a) => (&[], &a.ty, false),
            // Field reads are built directly in `lower_attribute`.
            RuntimeCallTarget::Field(f) => (&[], &f.field_type, false),
        };

        let mut ops: Vec<Operand> = Vec::with_capacity(def.params.len());
        for (i, slot) in args.iter().enumerate() {
            let want = runtime_param_repr(def.params[i], params.get(i).map(|p| &p.ty));
            match slot {
                Some(arg) => {
                    // An integer literal headed for a raw i64 register slot
                    // materializes directly — no tagged round-trip. Required
                    // for sentinel defaults (`i64::MIN`) outside the 61-bit
                    // tagged range, where tagging would wrap.
                    if want == Repr::Raw(RawKind::I64) {
                        if let HirExprKind::IntLit(v) = self.func.exprs[*arg].kind {
                            let dst = self.alloc_temp(Repr::Raw(RawKind::I64));
                            self.emit(MirInst::Const {
                                dst,
                                val: Const::Int(v),
                            });
                            ops.push(Operand::Local(dst));
                            continue;
                        }
                    }
                    let (al, ar) = self.lower_expr(*arg)?;
                    // A gradual argument headed for a raw register slot takes
                    // the CHECKED unbox (Phase 8H, D3): `rt_unbox_float` /
                    // `rt_unbox_int` validate the tag at runtime (TypeError on
                    // mismatch) instead of reinterpreting bits. Statically
                    // proven types keep the unchecked fast path. Shared with the
                    // return / `float`-local seams via `coerce_value`.
                    let arg_ty = self.func.exprs[*arg].ty.clone();
                    let coerced = self.coerce_value(al, ar, &arg_ty, want.clone())?;
                    ops.push(Operand::Local(coerced));
                }
                None => {
                    // Absent optional object param → the null-pointer sentinel.
                    let null = self.alloc_temp(Repr::Tagged);
                    self.emit(MirInst::Const {
                        dst: null,
                        val: Const::NullPtr,
                    });
                    ops.push(Operand::Local(null));
                }
            }
        }
        if pass_arg_count {
            let count_repr = match def.params.last() {
                Some(pt) => runtime_param_repr(*pt, None),
                None => Repr::Raw(RawKind::I64),
            };
            let c = self.alloc_temp(count_repr);
            self.emit(MirInst::Const {
                dst: c,
                val: Const::Int(provided as i64),
            });
            ops.push(Operand::Local(c));
        }

        self.emit_runtime_call(expr_idx, def, ops, ret_spec)
    }

    /// Lower `itertools.islice(iterable, stop)` /
    /// `islice(iterable, start, stop[, step])`. The iterable is `iter()`-wrapped
    /// (the runtime walks it with the iterator protocol) and start/stop/step are
    /// resolved from the provided argument count, matching CPython: a lone
    /// numeric arg is the STOP (start defaults to 0); step defaults to 1.
    fn lower_islice(
        &mut self,
        call_idx: Idx<HirExpr>,
        def: &'static pyaot_core_defs::RuntimeFuncDef,
        args: &[Option<Idx<HirExpr>>],
        provided: u32,
    ) -> Result<(LocalId, Repr)> {
        let span = self.func.exprs[call_idx].span;
        let iterable = args.first().copied().flatten().ok_or_else(|| {
            CompilerError::codegen_error("internal: islice without an iterable", Some(span))
        })?;
        let (it, _) = self.lower_iter_arg(iterable)?;

        // islice(it, stop)               → start=0, stop,      step=1
        // islice(it, start, stop[, step])→ start,   stop,      step (default 1)
        let (start, stop, step) = if provided <= 2 {
            let stop = self.lower_raw_index(args.get(1).copied().flatten(), span)?;
            (self.raw_i64_const(0), stop, self.raw_i64_const(1))
        } else {
            let start = self.lower_raw_index(args.get(1).copied().flatten(), span)?;
            let stop = self.lower_raw_index(args.get(2).copied().flatten(), span)?;
            let step = if provided >= 4 {
                self.lower_raw_index(args.get(3).copied().flatten(), span)?
            } else {
                self.raw_i64_const(1)
            };
            (start, stop, step)
        };

        let ops = vec![
            Operand::Local(it),
            Operand::Local(start),
            Operand::Local(stop),
            Operand::Local(step),
        ];
        self.emit_runtime_call(call_idx, def, ops, &pyaot_stdlib_defs::TypeSpec::Any)
    }

    /// Lower an islice index/step argument into a `Raw(I64)` register. An integer
    /// literal materializes directly (no tagged round-trip); anything else is
    /// lowered and coerced.
    fn lower_raw_index(
        &mut self,
        slot: Option<Idx<HirExpr>>,
        span: pyaot_utils::Span,
    ) -> Result<LocalId> {
        let arg = slot.ok_or_else(|| {
            CompilerError::codegen_error("internal: islice missing a numeric argument", Some(span))
        })?;
        if let HirExprKind::IntLit(v) = self.func.exprs[arg].kind {
            let dst = self.alloc_temp(Repr::Raw(RawKind::I64));
            self.emit(MirInst::Const {
                dst,
                val: Const::Int(v),
            });
            return Ok(dst);
        }
        let (l, r) = self.lower_expr(arg)?;
        self.coerce(l, r, Repr::Raw(RawKind::I64))
    }

    /// Emit a `CallRuntime` for descriptor `def` with already-legalized arg
    /// operands, then coerce its result to `call_idx`'s static repr (Phase 8B/C
    /// shared tail). A void descriptor yields a `None` value so the call can
    /// stand in expression position.
    fn emit_runtime_call(
        &mut self,
        call_idx: Idx<HirExpr>,
        def: &'static pyaot_core_defs::RuntimeFuncDef,
        ops: Vec<Operand>,
        ret_spec: &pyaot_stdlib_defs::TypeSpec,
    ) -> Result<(LocalId, Repr)> {
        match def.returns {
            Some(_) => {
                let ret_repr = runtime_return_repr(def, ret_spec);
                let dst = self.alloc_temp(ret_repr.clone());
                self.emit(MirInst::CallRuntime {
                    dst: Some(dst),
                    def,
                    args: ops,
                });
                let result_repr = repr_of(&self.func.exprs[call_idx].ty);
                let coerced = self.coerce(dst, ret_repr, result_repr.clone())?;
                Ok((coerced, result_repr))
            }
            None => {
                self.emit(MirInst::CallRuntime {
                    dst: None,
                    def,
                    args: ops,
                });
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::Const {
                    dst,
                    val: Const::None,
                });
                Ok((dst, Repr::Tagged))
            }
        }
    }

    /// Lower a stdlib runtime-object method (`m.group(0)`, Phase 8C). The
    /// receiver is descriptor arg 0 (always a Tagged pointer); the written args
    /// fill the method's declared params (`codegen.params[1..]`), legalized to
    /// each param's repr. An absent optional param defaults to a zero / null
    /// sentinel matching its register class (e.g. `m.group()` ≡ `m.group(0)`).
    fn lower_runtime_object_method(
        &mut self,
        call_idx: Idx<HirExpr>,
        recv: Idx<HirExpr>,
        method: &'static pyaot_stdlib_defs::StdlibMethodDef,
        args: &[Idx<HirExpr>],
        span: pyaot_utils::Span,
    ) -> Result<(LocalId, Repr)> {
        let def = &method.codegen;
        if args.len() > method.params.len() {
            return Err(CompilerError::semantic_error(
                format!(
                    "`{}()` takes at most {} argument(s)",
                    method.name,
                    method.params.len()
                ),
                span,
            ));
        }
        let (rl, rr) = self.lower_expr(recv)?;
        let recv_op = self.coerce(rl, rr, Repr::Tagged)?;
        let mut ops = vec![Operand::Local(recv_op)];
        for (i, p) in method.params.iter().enumerate() {
            let pt = def
                .params
                .get(i + 1)
                .copied()
                .unwrap_or(pyaot_core_defs::runtime_func_def::P_I64);
            let want = runtime_param_repr(pt, Some(&p.ty));
            if i < args.len() {
                if want == Repr::Raw(RawKind::I64) {
                    if let HirExprKind::IntLit(v) = self.func.exprs[args[i]].kind {
                        let d = self.alloc_temp(Repr::Raw(RawKind::I64));
                        self.emit(MirInst::Const {
                            dst: d,
                            val: Const::Int(v),
                        });
                        ops.push(Operand::Local(d));
                        continue;
                    }
                }
                let (al, ar) = self.lower_expr(args[i])?;
                ops.push(Operand::Local(self.coerce(al, ar, want)?));
            } else {
                // Absent optional param: emit its DECLARED default in the param's
                // register class (e.g. `Counter.most_common()` → `i64::MIN`
                // sentinel = "all"; `deque.rotate()` → `1`; `OrderedDict.popitem()`
                // → `last=true`). Falls back to a zero / null-pointer sentinel when
                // the descriptor declares no default.
                use pyaot_stdlib_defs::ConstValue;
                let d = self.alloc_temp(want.clone());
                let val = match (&p.default, &want) {
                    (Some(ConstValue::Int(v)), Repr::Raw(_)) => Const::Int(*v),
                    (Some(ConstValue::Bool(b)), Repr::Raw(_)) => Const::Int(*b as i64),
                    _ if matches!(want, Repr::Raw(_)) => Const::Int(0),
                    _ => Const::NullPtr,
                };
                self.emit(MirInst::Const { dst: d, val });
                ops.push(Operand::Local(d));
            }
        }
        self.emit_runtime_call(call_idx, def, ops, &method.return_type)
    }

    /// Lower a str-receiver method to its `rt_str_*` descriptor (Phase 8B/8C).
    /// Returns `None` for an unrecognized name so the caller falls through to
    /// the container path. `find` rides the generic `rt_str_search` with op_tag
    /// 0; the result reprs follow `method_call_ty` (str/bool/int).
    fn lower_str_method(
        &mut self,
        call_idx: Idx<HirExpr>,
        recv: Idx<HirExpr>,
        method_name: InternedString,
        args: &[Idx<HirExpr>],
        span: pyaot_utils::Span,
    ) -> Result<Option<(LocalId, Repr)>> {
        use pyaot_core_defs::runtime_func_def as rf;
        use pyaot_stdlib_defs::TypeSpec;
        use ArgWant::{RawI64, Tagged as TaggedArg};
        let name = self.interner.resolve(method_name);
        // (descriptor, arg reprs, min args, trailing op_tag if any, return
        // spec). Args past `min` are optional — an absent one lowers to the
        // null-pointer object sentinel. The return spec disambiguates the I64
        // ABI: `find` returns a RAW index (Int → Raw(I64), then tagged), the
        // case ops a heap str (Tagged).
        type StrPlan = (
            &'static pyaot_core_defs::RuntimeFuncDef,
            &'static [ArgWant],
            usize,
            Option<i64>,
            TypeSpec,
        );
        let plan: Option<StrPlan> = match name {
            "upper" => Some((&rf::RT_STR_UPPER, &[], 0, None, TypeSpec::Str)),
            "lower" => Some((&rf::RT_STR_LOWER, &[], 0, None, TypeSpec::Str)),
            "title" => Some((&rf::RT_STR_TITLE, &[], 0, None, TypeSpec::Str)),
            "capitalize" => Some((&rf::RT_STR_CAPITALIZE, &[], 0, None, TypeSpec::Str)),
            "swapcase" => Some((&rf::RT_STR_SWAPCASE, &[], 0, None, TypeSpec::Str)),
            "strip" => Some((&rf::RT_STR_STRIP, &[], 0, None, TypeSpec::Str)),
            "startswith" => Some((
                &rf::RT_STR_STARTSWITH,
                &[TaggedArg],
                1,
                None,
                TypeSpec::Bool,
            )),
            "endswith" => Some((&rf::RT_STR_ENDSWITH, &[TaggedArg], 1, None, TypeSpec::Bool)),
            // §9 — find/index family. `start`/`end` ride RAW i64 slots (codepoint
            // bounds; absent → 0 / i64::MAX which the runtime clamps to the
            // length). The trailing `op_tag` (`Some(..)`) is appended as Raw(I8).
            "find" => Some((
                &rf::RT_STR_FIND,
                &[TaggedArg, RawI64(0), RawI64(i64::MAX)],
                1,
                Some(0),
                TypeSpec::Int,
            )),
            "rfind" => Some((
                &rf::RT_STR_RFIND,
                &[TaggedArg, RawI64(0), RawI64(i64::MAX)],
                1,
                Some(1),
                TypeSpec::Int,
            )),
            "index" => Some((
                &rf::RT_STR_INDEX,
                &[TaggedArg, RawI64(0), RawI64(i64::MAX)],
                1,
                Some(2),
                TypeSpec::Int,
            )),
            "count" => Some((&rf::RT_STR_COUNT, &[TaggedArg], 1, None, TypeSpec::Int)),
            "zfill" => Some((&rf::RT_STR_ZFILL, &[RawI64(0)], 1, None, TypeSpec::Str)),
            "center" => Some((
                &rf::RT_STR_CENTER,
                &[RawI64(0), TaggedArg],
                1,
                None,
                TypeSpec::Str,
            )),
            "ljust" => Some((
                &rf::RT_STR_LJUST,
                &[RawI64(0), TaggedArg],
                1,
                None,
                TypeSpec::Str,
            )),
            "rjust" => Some((
                &rf::RT_STR_RJUST,
                &[RawI64(0), TaggedArg],
                1,
                None,
                TypeSpec::Str,
            )),
            // §9 — split family. `maxsplit` rides a RAW i64 slot (descriptor
            // retyped to `STR_SPLIT_TERNARY`); an absent sep is whitespace
            // split, an absent maxsplit defaults to `-1` (unlimited, A2).
            "split" => Some((
                &rf::RT_STR_SPLIT,
                &[TaggedArg, RawI64(-1)],
                0,
                None,
                TypeSpec::List(&pyaot_stdlib_defs::types::TYPE_STR),
            )),
            "rsplit" => Some((
                &rf::RT_STR_RSPLIT,
                &[TaggedArg, RawI64(-1)],
                0,
                None,
                TypeSpec::List(&pyaot_stdlib_defs::types::TYPE_STR),
            )),
            "splitlines" => Some((
                &rf::RT_STR_SPLITLINES,
                &[],
                0,
                None,
                TypeSpec::List(&pyaot_stdlib_defs::types::TYPE_STR),
            )),
            // `replace(old, new[, count])` — `count` rides a RAW i64 slot
            // (absent → -1 = replace all, §9). min_args stays 2.
            "replace" => Some((
                &rf::RT_STR_REPLACE,
                &[TaggedArg, TaggedArg, RawI64(-1)],
                2,
                None,
                TypeSpec::Str,
            )),
            // `lstrip`/`rstrip([chars])` — chars optional (null = whitespace).
            "lstrip" => Some((&rf::RT_STR_LSTRIP, &[TaggedArg], 0, None, TypeSpec::Str)),
            "rstrip" => Some((&rf::RT_STR_RSTRIP, &[TaggedArg], 0, None, TypeSpec::Str)),
            "removeprefix" => Some((
                &rf::RT_STR_REMOVEPREFIX,
                &[TaggedArg],
                1,
                None,
                TypeSpec::Str,
            )),
            "removesuffix" => Some((
                &rf::RT_STR_REMOVESUFFIX,
                &[TaggedArg],
                1,
                None,
                TypeSpec::Str,
            )),
            // `expandtabs([tabsize])` — tabsize a RAW i64 (default 8, A2).
            "expandtabs" => Some((&rf::RT_STR_EXPANDTABS, &[RawI64(8)], 0, None, TypeSpec::Str)),
            // `partition`/`rpartition(sep)` → a 3-tuple. Typed `Dyn` (a stdlib
            // `Tuple` spec is gradual), so `a, sep, b = …` unpacks through the
            // gradual seam.
            "partition" => Some((
                &rf::RT_STR_PARTITION,
                &[TaggedArg],
                1,
                None,
                TypeSpec::Tuple(&pyaot_stdlib_defs::types::TYPE_STR),
            )),
            "rpartition" => Some((
                &rf::RT_STR_RPARTITION,
                &[TaggedArg],
                1,
                None,
                TypeSpec::Tuple(&pyaot_stdlib_defs::types::TYPE_STR),
            )),
            // `encode(encoding=, errors=)` is NOT here: it takes keyword args and
            // a 3-arg ABI, handled by the dedicated codec path in
            // `lower_method_call` (before the no-keyword gate).
            // `rindex(sub[, start[, end]])` via the shared `rt_str_search` with
            // op_tag 3 (raises ValueError on a miss, like `index`); `start`/`end`
            // ride RAW i64 slots like `find`.
            "rindex" => Some((
                &rf::RT_STR_RINDEX,
                &[TaggedArg, RawI64(0), RawI64(i64::MAX)],
                1,
                Some(3),
                TypeSpec::Int,
            )),
            // Codepoint predicates (Unicode-aware in the runtime, §9). The
            // numeric trio (`isdecimal`/`isdigit`/`isnumeric`) rides the
            // generated Numeric_Type table; the rest use `char::is_*`.
            "isdecimal" => Some((&rf::RT_STR_ISDECIMAL, &[], 0, None, TypeSpec::Bool)),
            "isdigit" => Some((&rf::RT_STR_ISDIGIT, &[], 0, None, TypeSpec::Bool)),
            "isnumeric" => Some((&rf::RT_STR_ISNUMERIC, &[], 0, None, TypeSpec::Bool)),
            "isalpha" => Some((&rf::RT_STR_ISALPHA, &[], 0, None, TypeSpec::Bool)),
            "isalnum" => Some((&rf::RT_STR_ISALNUM, &[], 0, None, TypeSpec::Bool)),
            "isspace" => Some((&rf::RT_STR_ISSPACE, &[], 0, None, TypeSpec::Bool)),
            "isupper" => Some((&rf::RT_STR_ISUPPER, &[], 0, None, TypeSpec::Bool)),
            "islower" => Some((&rf::RT_STR_ISLOWER, &[], 0, None, TypeSpec::Bool)),
            "isascii" => Some((&rf::RT_STR_ISASCII, &[], 0, None, TypeSpec::Bool)),
            // `sep.join(iterable)` → `rt_str_join(sep, list)` (Phase 8E). The
            // argument is coerced to Tagged (a list/tuple of strings).
            "join" => Some((&rf::RT_STR_JOIN, &[TaggedArg], 1, None, TypeSpec::Str)),
            _ => None,
        };
        let Some((def, wants, min_args, op_tag, ret_spec)) = plan else {
            return Ok(None);
        };
        Ok(Some(self.emit_seq_method(
            call_idx, recv, "str", name, def, wants, min_args, op_tag, &ret_spec, args, span,
        )?))
    }

    /// Lower a bytes-receiver method to its `rt_bytes_*` descriptor (§9), the
    /// exact sibling of [`Self::lower_str_method`]: a declarative `BytesPlan`
    /// table → the shared [`Self::emit_seq_method`]. Returns `None` for an
    /// unrecognized name so the caller falls through. All methods are
    /// positional-only. Notable shape differences from str: `find`/`rfind` use
    /// dedicated 2-arg runtime fns (NO op_tag, unlike str's `rt_str_search`);
    /// `split`/`rsplit` ride a RAW i64 `maxsplit` (B16) with an absent/`None`
    /// sep meaning whitespace split; the strip family is 1-arg (no `chars`); and
    /// `join` materializes its iterable into a list, like `str.join`. The split
    /// descriptors return `list[bytes]`.
    fn lower_bytes_method(
        &mut self,
        call_idx: Idx<HirExpr>,
        recv: Idx<HirExpr>,
        method_name: InternedString,
        args: &[Idx<HirExpr>],
        span: pyaot_utils::Span,
    ) -> Result<Option<(LocalId, Repr)>> {
        use pyaot_core_defs::runtime_func_def as rf;
        use pyaot_stdlib_defs::TypeSpec;
        use ArgWant::{RawI64, Tagged as TaggedArg};
        let name = self.interner.resolve(method_name);
        // Same shape as `StrPlan`: (descriptor, arg reprs, min args, op_tag,
        // return spec). No bytes method needs an op_tag (find/rfind are dedicated
        // 2-arg fns), so the slot is always `None` here.
        type BytesPlan = (
            &'static pyaot_core_defs::RuntimeFuncDef,
            &'static [ArgWant],
            usize,
            Option<i64>,
            TypeSpec,
        );
        let plan: Option<BytesPlan> = match name {
            "startswith" => Some((
                &rf::RT_BYTES_STARTS_WITH,
                &[TaggedArg],
                1,
                None,
                TypeSpec::Bool,
            )),
            "endswith" => Some((
                &rf::RT_BYTES_ENDS_WITH,
                &[TaggedArg],
                1,
                None,
                TypeSpec::Bool,
            )),
            // `find`/`rfind(sub[, start[, end]])` — dedicated 2-arg fns (no
            // op_tag); `start`/`end` ride RAW i64 slots (absent → 0 / i64::MAX,
            // clamped to len by the runtime).
            "find" => Some((
                &rf::RT_BYTES_FIND,
                &[TaggedArg, RawI64(0), RawI64(i64::MAX)],
                1,
                None,
                TypeSpec::Int,
            )),
            "rfind" => Some((
                &rf::RT_BYTES_RFIND,
                &[TaggedArg, RawI64(0), RawI64(i64::MAX)],
                1,
                None,
                TypeSpec::Int,
            )),
            "count" => Some((&rf::RT_BYTES_COUNT, &[TaggedArg], 1, None, TypeSpec::Int)),
            // `replace(old, new[, count])` — `count` rides a RAW i64 slot (absent
            // → -1 = replace all). min_args stays 2.
            "replace" => Some((
                &rf::RT_BYTES_REPLACE,
                &[TaggedArg, TaggedArg, RawI64(-1)],
                2,
                None,
                TypeSpec::Bytes,
            )),
            // split family → `list[bytes]`; an absent/`None` sep is whitespace
            // split, an absent `maxsplit` defaults to -1 (unlimited).
            "split" => Some((
                &rf::RT_BYTES_SPLIT,
                &[TaggedArg, RawI64(-1)],
                0,
                None,
                TypeSpec::List(&pyaot_stdlib_defs::types::TYPE_BYTES),
            )),
            "rsplit" => Some((
                &rf::RT_BYTES_RSPLIT,
                &[TaggedArg, RawI64(-1)],
                0,
                None,
                TypeSpec::List(&pyaot_stdlib_defs::types::TYPE_BYTES),
            )),
            // strip family: 1-arg (`bytes` only — no `chars`, a documented limit).
            "strip" => Some((&rf::RT_BYTES_STRIP, &[], 0, None, TypeSpec::Bytes)),
            "lstrip" => Some((&rf::RT_BYTES_LSTRIP, &[], 0, None, TypeSpec::Bytes)),
            "rstrip" => Some((&rf::RT_BYTES_RSTRIP, &[], 0, None, TypeSpec::Bytes)),
            "upper" => Some((&rf::RT_BYTES_UPPER, &[], 0, None, TypeSpec::Bytes)),
            "lower" => Some((&rf::RT_BYTES_LOWER, &[], 0, None, TypeSpec::Bytes)),
            // `sep.join(iterable)` → `rt_bytes_join(sep, list)`; materialize the
            // iterable into a list first (like `str.join`).
            "join" => Some((&rf::RT_BYTES_JOIN, &[TaggedArg], 1, None, TypeSpec::Bytes)),
            // `decode(encoding=, errors=)` is NOT here: keyword args + 3-arg ABI,
            // handled by the dedicated codec path in `lower_method_call`.
            _ => None,
        };
        let Some((def, wants, min_args, op_tag, ret_spec)) = plan else {
            return Ok(None);
        };
        Ok(Some(self.emit_seq_method(
            call_idx, recv, "bytes", name, def, wants, min_args, op_tag, &ret_spec, args, span,
        )?))
    }

    /// Lower an `int` / `bool` receiver method (§9). All four are zero-arg and
    /// yield `int`. `bit_length`/`bit_count` route to bignum-aware runtime counts
    /// (`rt_int_bit_*`, Tagged receiver → Raw(I64) → tagged int). `conjugate` /
    /// `__index__` return the receiver's int value via `rt_int_index` (which
    /// widens a `bool` to its int 0/1 and preserves a bignum), so a `bool`
    /// receiver produces an Int-typed result, never a tagged bool. Returns `None`
    /// for an unrecognized name so the caller falls through.
    fn lower_int_method(
        &mut self,
        recv: Idx<HirExpr>,
        method_name: InternedString,
        args: &[Idx<HirExpr>],
        span: pyaot_utils::Span,
    ) -> Result<Option<(LocalId, Repr)>> {
        use pyaot_core_defs::runtime_func_def as rf;
        let name = self.interner.resolve(method_name);
        let def: &'static pyaot_core_defs::RuntimeFuncDef = match name {
            "bit_length" => &rf::RT_INT_BIT_LENGTH,
            "bit_count" => &rf::RT_INT_BIT_COUNT,
            "conjugate" | "__index__" => &rf::RT_INT_INDEX,
            _ => return Ok(None),
        };
        if !args.is_empty() {
            return Err(CompilerError::semantic_error(
                format!("`int.{name}()` takes no arguments"),
                span,
            ));
        }
        let (rl, rr) = self.lower_expr(recv)?;
        let recv_tagged = self.coerce(rl, rr, Repr::Tagged)?;
        // `bit_*` return a Raw(I64) count (normalized to a tagged int); `index`
        // returns the tagged int value directly.
        let returns_count = matches!(name, "bit_length" | "bit_count");
        let dst = self.alloc_temp(if returns_count {
            Repr::Raw(RawKind::I64)
        } else {
            Repr::Tagged
        });
        self.emit(MirInst::CallRuntime {
            dst: Some(dst),
            def,
            args: vec![Operand::Local(recv_tagged)],
        });
        if returns_count {
            Ok(Some(self.normalize_container_result(
                dst,
                Repr::Raw(RawKind::I64),
            )?))
        } else {
            Ok(Some((dst, Repr::Tagged)))
        }
    }

    /// Shared post-plan emitter for a sequence-method runtime call (`str`/
    /// `bytes`). Given a descriptor already resolved from a `StrPlan`/`BytesPlan`
    /// row, it runs the common arg loop and emits the `CallRuntime`:
    /// - an explicit `None` for an optional Tagged arg (`split(None)`,
    ///   `rstrip(None)`) lowers to the NullPtr "use the default" sentinel — NOT
    ///   the `None` value (`NONE_TAG`), which the runtime would mis-deref;
    /// - `join` materializes its iterable into a list (the runtime reads a
    ///   `ListObj`), like CPython accepting any iterable;
    /// - an absent optional Tagged arg → NullPtr (whitespace sep / strip / UTF-8
    ///   encode); an absent `RawI64` arg → its Python default in the raw class
    ///   (`tabsize = 8` for `expandtabs`, else `maxsplit = -1`);
    /// - a trailing `op_tag` (str `find`/`index` family) is appended as `Raw(I8)`.
    ///
    /// `recv_label` is the receiver-type name for the arity diagnostic
    /// (`str` / `bytes`); `name` is the already-resolved method name.
    #[allow(clippy::too_many_arguments)]
    fn emit_seq_method(
        &mut self,
        call_idx: Idx<HirExpr>,
        recv: Idx<HirExpr>,
        recv_label: &str,
        name: &str,
        def: &'static pyaot_core_defs::RuntimeFuncDef,
        wants: &[ArgWant],
        min_args: usize,
        op_tag: Option<i64>,
        ret_spec: &pyaot_stdlib_defs::TypeSpec,
        args: &[Idx<HirExpr>],
        span: pyaot_utils::Span,
    ) -> Result<(LocalId, Repr)> {
        if args.len() < min_args || args.len() > wants.len() {
            return Err(CompilerError::semantic_error(
                format!(
                    "`{recv_label}.{name}()` takes {min_args}..={} argument(s), got {}",
                    wants.len(),
                    args.len()
                ),
                span,
            ));
        }
        let (rl, rr) = self.lower_expr(recv)?;
        let mut ops = vec![Operand::Local(self.coerce(rl, rr, Repr::Tagged)?)];
        for (i, want) in wants.iter().enumerate() {
            let op = if let Some(a) = args.get(i) {
                // An explicit `None` for an optional Tagged arg means "use the
                // default" — the SAME null sentinel the runtime tests via
                // `sep.is_null()`. Lower it to NullPtr, NOT the None value
                // (`NONE_TAG`), which the runtime would mis-deref: `unwrap_ptr`
                // debug-asserts the ptr tag, and `0b101` is non-null garbage in
                // release (SEGV).
                if matches!(want, ArgWant::Tagged)
                    && matches!(self.func.exprs[*a].kind, HirExprKind::NoneLit)
                {
                    let d = self.alloc_temp(Repr::Tagged);
                    self.emit(MirInst::Const {
                        dst: d,
                        val: Const::NullPtr,
                    });
                    d
                } else {
                    // `sep.join(iterable)` accepts ANY iterable in CPython (str,
                    // tuple, generator, …), but `rt_str_join`/`rt_bytes_join` read
                    // a `ListObj`. Materialize the argument into a list first —
                    // passing a non-list straight through has the runtime cast a
                    // mismatched heap object and SEGV (the gradual heap-param
                    // exemption does not guard the shape at the seam).
                    let (al, ar) = if name == "join" {
                        self.materialize_list(*a)?
                    } else {
                        self.lower_expr(*a)?
                    };
                    let want_repr = match want {
                        ArgWant::Tagged => Repr::Tagged,
                        ArgWant::RawI64(_) => Repr::Raw(RawKind::I64),
                    };
                    self.coerce(al, ar, want_repr)?
                }
            } else {
                // Absent optional arg. A Tagged slot gets the null-pointer object
                // sentinel (the runtime reads it as "default": whitespace sep,
                // whitespace strip, UTF-8 encode). A RawI64 slot must NOT receive
                // a Tagged null — that would fail the verifier — so it gets the
                // Python default carried by the `ArgWant` in its raw register
                // class (`maxsplit`/`count = -1`, `tabsize = 8`, search `start = 0`
                // / `end = i64::MAX`).
                match want {
                    ArgWant::Tagged => {
                        let d = self.alloc_temp(Repr::Tagged);
                        self.emit(MirInst::Const {
                            dst: d,
                            val: Const::NullPtr,
                        });
                        d
                    }
                    ArgWant::RawI64(default) => {
                        let d = self.alloc_temp(Repr::Raw(RawKind::I64));
                        self.emit(MirInst::Const {
                            dst: d,
                            val: Const::Int(*default),
                        });
                        d
                    }
                }
            };
            ops.push(Operand::Local(op));
        }
        if let Some(tag) = op_tag {
            let t = self.alloc_temp(Repr::Raw(RawKind::I8));
            self.emit(MirInst::Const {
                dst: t,
                val: Const::Int(tag),
            });
            ops.push(Operand::Local(t));
        }
        self.emit_runtime_call(call_idx, def, ops, ret_spec)
    }

    /// Lower `str.encode` / `bytes.decode` (§9) through the 3-arg codec ABI
    /// `rt_str_encode(s, encoding, errors)` / `rt_bytes_decode(b, encoding,
    /// errors)`. Binds the positional-or-keyword `encoding`/`errors` params into
    /// their fixed slots; an absent OR explicit `None` arg lowers to the NullPtr
    /// "use default" sentinel (utf-8 / strict — the runtime tests `is_null()`).
    /// Unknown keywords, a duplicate positional+keyword binding, or more than two
    /// positionals are precise diagnostics.
    #[allow(clippy::too_many_arguments)]
    fn lower_codec_method(
        &mut self,
        call_idx: Idx<HirExpr>,
        recv: Idx<HirExpr>,
        method_name: InternedString,
        def: &'static pyaot_core_defs::RuntimeFuncDef,
        ret_spec: &pyaot_stdlib_defs::TypeSpec,
        args: &[Idx<HirExpr>],
        kwargs: &[(InternedString, Idx<HirExpr>)],
        span: pyaot_utils::Span,
    ) -> Result<(LocalId, Repr)> {
        const PARAMS: [&str; 2] = ["encoding", "errors"];
        let mname = self.interner.resolve(method_name).to_string();
        let mut slots: [Option<Idx<HirExpr>>; 2] = [None, None];
        if args.len() > PARAMS.len() {
            return Err(CompilerError::semantic_error(
                format!(
                    "`{mname}()` takes at most {} arguments, got {}",
                    PARAMS.len(),
                    args.len()
                ),
                span,
            ));
        }
        for (i, a) in args.iter().enumerate() {
            slots[i] = Some(*a);
        }
        for (kname, kexpr) in kwargs {
            let kn = self.interner.resolve(*kname);
            let Some(idx) = PARAMS.iter().position(|&p| p == kn) else {
                return Err(CompilerError::semantic_error(
                    format!("`{mname}()` got an unexpected keyword argument '{kn}'"),
                    span,
                ));
            };
            if slots[idx].is_some() {
                return Err(CompilerError::semantic_error(
                    format!("`{mname}()` got multiple values for argument '{}'", PARAMS[idx]),
                    span,
                ));
            }
            slots[idx] = Some(*kexpr);
        }
        let (rl, rr) = self.lower_expr(recv)?;
        let mut ops = vec![Operand::Local(self.coerce(rl, rr, Repr::Tagged)?)];
        for slot in slots {
            let op = match slot {
                Some(a) if !matches!(self.func.exprs[a].kind, HirExprKind::NoneLit) => {
                    let (al, ar) = self.lower_expr(a)?;
                    self.coerce(al, ar, Repr::Tagged)?
                }
                _ => {
                    let d = self.alloc_temp(Repr::Tagged);
                    self.emit(MirInst::Const {
                        dst: d,
                        val: Const::NullPtr,
                    });
                    d
                }
            };
            ops.push(Operand::Local(op));
        }
        self.emit_runtime_call(call_idx, def, ops, ret_spec)
    }

    /// Lower a File method (`f.read()`, `f.write(s)`, Phase 8C) to its
    /// `rt_file_*` descriptor. Context-manager dunders route here too: the
    /// `with`-desugar's `__exit__(e, e, None)` drops its three exception args
    /// (`rt_file_exit` takes only the file and always returns "don't swallow").
    fn lower_file_method(
        &mut self,
        call_idx: Idx<HirExpr>,
        recv: Idx<HirExpr>,
        method_name: InternedString,
        args: &[Idx<HirExpr>],
        binary: bool,
        span: pyaot_utils::Span,
    ) -> Result<(LocalId, Repr)> {
        use pyaot_core_defs::runtime_func_def as rf;
        let name = self.interner.resolve(method_name);
        // `read(n)` selects the n-arg descriptor; bare `read()` the whole-file one.
        let def: &'static pyaot_core_defs::RuntimeFuncDef = match (name, args.len()) {
            ("read", 0) => &rf::RT_FILE_READ,
            ("read", _) => &rf::RT_FILE_READ_N,
            ("readline", 0) => &rf::RT_FILE_READLINE,
            ("readlines", 0) => &rf::RT_FILE_READLINES,
            ("write", 1) => &rf::RT_FILE_WRITE,
            ("close", 0) => &rf::RT_FILE_CLOSE,
            ("flush", 0) => &rf::RT_FILE_FLUSH,
            ("__enter__", 0) => &rf::RT_FILE_ENTER,
            // `__exit__` arrives with 3 exception args; drop them.
            ("__exit__", _) => &rf::RT_FILE_EXIT,
            _ => {
                return Err(CompilerError::semantic_error(
                    format!("File has no method `.{name}()` (or wrong arity)"),
                    span,
                ));
            }
        };
        let (rl, rr) = self.lower_expr(recv)?;
        let recv_op = self.coerce(rl, rr, Repr::Tagged)?;
        let mut ops = vec![Operand::Local(recv_op)];
        // `read(n)`'s count is a raw i64; `write(data)`'s buffer is a Tagged
        // pointer. Other methods take no trailing arg.
        match name {
            "read" if !args.is_empty() => {
                let (al, ar) = self.lower_expr(args[0])?;
                ops.push(Operand::Local(self.coerce(
                    al,
                    ar,
                    Repr::Raw(RawKind::I64),
                )?));
            }
            "write" => {
                let (al, ar) = self.lower_expr(args[0])?;
                ops.push(Operand::Local(self.coerce(al, ar, Repr::Tagged)?));
            }
            _ => {}
        }
        // Return spec drives the result repr: `read`/`readline` are bytes/str by
        // mode; `write` returns a raw byte count (Int); the rest ride Tagged.
        let ret_spec = match name {
            "read" | "readline" if binary => pyaot_stdlib_defs::TypeSpec::Bytes,
            "read" | "readline" => pyaot_stdlib_defs::TypeSpec::Str,
            "write" => pyaot_stdlib_defs::TypeSpec::Int,
            _ => pyaot_stdlib_defs::TypeSpec::Any,
        };
        self.emit_runtime_call(call_idx, def, ops, &ret_spec)
    }

    /// Is `value` a `type(<exactly one arg>)` builtin call? Mirrors the callee
    /// dispatch `lower_call` uses (a `Name` resolving to `Symbol::Builtin(Type)`),
    /// gating the `type(x).__name__` peephole.
    fn is_type_builtin_call(&self, value: Idx<HirExpr>) -> bool {
        if let HirExprKind::Call { callee, args } = &self.func.exprs[value].kind {
            if args.len() == 1 {
                if let HirExprKind::Name(SymbolRef::Resolved(id)) = &self.func.exprs[*callee].kind {
                    return matches!(
                        self.resolve.symbol(*id),
                        Symbol::Builtin(pyaot_mir::BuiltinFunctionKind::Type)
                    );
                }
            }
        }
        false
    }

    fn lower_attribute(
        &mut self,
        attr_idx: Idx<HirExpr>,
        value: Idx<HirExpr>,
        name: InternedString,
    ) -> Result<(LocalId, Repr)> {
        let span = self.func.exprs[value].span;
        let result_repr = repr_of(&self.func.exprs[attr_idx].ty);

        // `type(x).__name__` → the bare class name. `type(x)` already produces
        // the `<class 'mod.Name'>` repr string at runtime (builtins via the type
        // tag, user instances via the registered qualname); `rt_type_name_extract`
        // takes THAT one string and returns its last dotted segment. Routing
        // `.__name__` through the same runtime source — never a compile-time
        // class-name table — keeps every `type()` form (`str(type(x))`,
        // `print(type(x))`, `type(x).__name__`) on one formatting path (PLAN §6).
        if self.interner.resolve(name) == "__name__" && self.is_type_builtin_call(value) {
            let (vl, vr) = self.lower_expr(value)?;
            let str_local = self.coerce(vl, vr, Repr::Tagged)?;
            let dst = self.alloc_temp(Repr::Tagged);
            self.emit(MirInst::CallRuntime {
                dst: Some(dst),
                def: &pyaot_core_defs::runtime_func_def::RT_TYPE_NAME_EXTRACT,
                args: vec![Operand::Local(str_local)],
            });
            let coerced = self.coerce(dst, Repr::Tagged, result_repr.clone())?;
            return Ok((coerced, result_repr));
        }

        // `e.args` on a caught builtin exception — or a tuple clause of only
        // builtins — (Phase 7B): the args tuple at instance field slot 0 (the
        // layout `create_builtin_exception_instance` produces; user exception
        // classes keep their own field layout and are NOT routed here). Other
        // attributes on builtin exceptions are out of scope.
        if is_builtin_exception_ty(&self.func.exprs[value].ty) {
            if self.interner.resolve(name) != "args" {
                return Err(CompilerError::semantic_error(
                    format!(
                        "`.{}` on a builtin exception is out of scope (only `.args`, \
                         `str(e)`, and `e.__class__.__name__` are supported)",
                        self.interner.resolve(name)
                    ),
                    span,
                ));
            }
            let (bl, br) = self.lower_expr(value)?;
            let bt = self.coerce(bl, br, Repr::Tagged)?;
            let dst = self.alloc_temp(Repr::Tagged);
            self.emit(MirInst::GetField {
                dst,
                base: Operand::Local(bt),
                slot: 0,
            });
            let coerced = self.coerce(dst, Repr::Tagged, result_repr.clone())?;
            return Ok((coerced, result_repr));
        }

        // A stdlib runtime object's field (`t.tm_year`, Phase 8B): the
        // `ObjectFieldDef` descriptor selects the getter; its constant
        // `field_index` rides as a trailing raw i64 when present.
        if let SemTy::RuntimeObject(tag) = &self.func.exprs[value].ty {
            let field = pyaot_stdlib_defs::object_types::lookup_object_type(*tag)
                .and_then(|obj| obj.get_field(self.interner.resolve(name)))
                .ok_or_else(|| {
                    CompilerError::semantic_error(
                        format!(
                            "runtime object has no attribute `.{}`",
                            self.interner.resolve(name)
                        ),
                        span,
                    )
                })?;
            let (bl, br) = self.lower_expr(value)?;
            let recv = self.coerce(bl, br, Repr::Tagged)?;
            let mut args = vec![Operand::Local(recv)];
            if let Some(fi) = field.field_index {
                // The constant index in the register class the descriptor's
                // trailing param demands (e.g. `rt_struct_time_get_field`'s u8).
                let idx_repr = match field.codegen.params.get(args.len()) {
                    Some(pt) => runtime_param_repr(*pt, None),
                    None => Repr::Raw(RawKind::I64),
                };
                let idx_const = self.alloc_temp(idx_repr);
                self.emit(MirInst::Const {
                    dst: idx_const,
                    val: Const::Int(fi),
                });
                args.push(Operand::Local(idx_const));
            }
            let ret_repr = runtime_return_repr(&field.codegen, &field.field_type);
            let dst = self.alloc_temp(ret_repr.clone());
            self.emit(MirInst::CallRuntime {
                dst: Some(dst),
                def: &field.codegen,
                args,
            });
            let coerced = self.coerce(dst, ret_repr, result_repr.clone())?;
            return Ok((coerced, result_repr));
        }

        // (a) `ClassName.attr` → a class-level attribute (Phase 5D).
        if let Some(cid) = self.class_name_ref(value) {
            let attr = self
                .classes
                .get(cid)
                .and_then(|i| i.class_attr(name))
                .ok_or_else(|| {
                    CompilerError::semantic_error(
                        format!("class has no attribute `.{}`", self.interner.resolve(name)),
                        span,
                    )
                })?;
            return self.read_class_attr(cid, attr.attr_idx, result_repr);
        }

        let bt = self.func.exprs[value].ty.clone();
        if let Some(cid) = class_of(&bt, self.classes) {
            // (b) `instance.prop` → the `@property` getter call (Phase 5D).
            if let Some(getter) = self
                .classes
                .get(cid)
                .and_then(|i| i.property(name))
                .map(|p| p.getter)
            {
                let (bl, br) = self.lower_expr(value)?;
                return self.emit_dunder_call(getter, vec![(bl, br)]);
            }
            // (c) `instance.classattr` (not an instance field) → class attribute.
            let is_field = self
                .classes
                .get(cid)
                .and_then(|i| i.field_slot(name))
                .is_some();
            if !is_field {
                if let Some(idx) = self
                    .classes
                    .get(cid)
                    .and_then(|i| i.class_attr(name))
                    .map(|a| a.attr_idx)
                {
                    // Evaluate the receiver for side effects, then read the class slot.
                    let _ = self.lower_expr(value)?;
                    return self.read_class_attr(cid, idx, result_repr);
                }
            }
        }

        // (d) Instance field read. A `Dyn` receiver resolves the slot at
        // RUNTIME by name hash (Phase 8H, D4) — `rt_getattr_name` raises
        // AttributeError on a miss/non-instance. Method-calls on `Dyn` stay a
        // loud compile error (no generic thunk for an unknown signature).
        // Other unknown receivers keep the loud `field_slot` error.
        if matches!(bt, SemTy::Dyn | SemTy::Union(_)) {
            let (bl, br) = self.lower_expr(value)?;
            let base = self.coerce(bl, br, Repr::Tagged)?;
            let dst = self.alloc_temp(Repr::Tagged);
            let name_hash = pyaot_utils::fnv1a_hash(self.interner.resolve(name));
            self.emit(MirInst::GetFieldNamed {
                dst,
                base: Operand::Local(base),
                name_hash,
            });
            // A `del obj.attr` stores UNBOUND into the named slot — guard the
            // tagged read before any unbox (kind 2 → AttributeError).
            let guarded = if self.deletable_fields.contains(&name) {
                self.emit_check_bound(dst, 2, name)?
            } else {
                dst
            };
            let coerced = self.coerce(guarded, Repr::Tagged, result_repr.clone())?;
            return Ok((coerced, result_repr));
        }
        let slot = self.field_slot(value, name)?;
        let (bl, _br) = self.lower_expr(value)?;
        let dst = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::GetField {
            dst,
            base: Operand::Local(bl),
            slot,
        });
        // A `del obj.attr` stores UNBOUND into this field's (tagged) slot —
        // guard the tagged read BEFORE the Tagged→repr unbox below, so a
        // `del`'d float/bool field raises rather than unboxing the sentinel
        // (kind 2 → AttributeError).
        let dst = if self.deletable_fields.contains(&name) {
            self.emit_check_bound(dst, 2, name)?
        } else {
            dst
        };
        // The field's static type drives the read's representation; the
        // Tagged→repr coercion is guarded by `typeck::check_repr_boundaries`.
        let coerced = self.coerce(dst, Repr::Tagged, result_repr.clone())?;
        Ok((coerced, result_repr))
    }

    /// `getattr(value, "name"[, default])` (§5). Keeps the static `Attribute`
    /// read when the attr is provably present on a concrete receiver (so
    /// methods/properties/class-attrs and the fast slot read still work);
    /// otherwise routes to the runtime by-name probe — `rt_getattr_name` (2-arg,
    /// raises on a miss) / `rt_getattr_name_or_default` (3-arg, returns
    /// `default`). A provably-absent attr on a concrete receiver with a default
    /// folds straight to that default.
    fn lower_get_attr_by_name(
        &mut self,
        idx: Idx<HirExpr>,
        value: Idx<HirExpr>,
        name: InternedString,
        default: Option<Idx<HirExpr>>,
    ) -> Result<(LocalId, Repr)> {
        let bt = self.func.exprs[value].ty.clone();
        let concrete_cid = class_of(&bt, self.classes);
        let present = concrete_cid.is_some_and(|cid| {
            self.classes.get(cid).is_some_and(|i| {
                i.field_slot(name).is_some()
                    || i.property(name).is_some()
                    || i.class_attr(name).is_some()
            })
        });
        if present {
            // Provably present → the ordinary static attribute read.
            return self.lower_attribute(idx, value, name);
        }
        if concrete_cid.is_some() {
            // Concrete receiver, attr provably absent. 3-arg → constant-fold to
            // the default (the probe could only ever miss); 2-arg → fall through
            // to the raising probe (the concrete instance ptr is a valid object).
            if let Some(d) = default {
                let _ = self.lower_expr(value)?; // receiver side effects
                let (dl, dr) = self.lower_expr(d)?;
                let coerced = self.coerce(dl, dr, Repr::Tagged)?;
                return Ok((coerced, Repr::Tagged));
            }
        }
        // Runtime by-name probe (Dyn/Union receiver, or a concrete-absent
        // 2-arg). `obj`/`default`/result ride the Tagged baseline; the FNV hash
        // is a RAW i64 immediate.
        let (vl, vr) = self.lower_expr(value)?;
        let base = self.coerce(vl, vr, Repr::Tagged)?;
        let name_hash = pyaot_utils::fnv1a_hash(self.interner.resolve(name));
        let dst = self.alloc_temp(Repr::Tagged);
        match default {
            Some(d) => {
                let (dl, dr) = self.lower_expr(d)?;
                let deflt = self.coerce(dl, dr, Repr::Tagged)?;
                let hash_local = self.alloc_temp(Repr::Raw(RawKind::I64));
                self.emit(MirInst::Const {
                    dst: hash_local,
                    val: Const::Int(name_hash as i64),
                });
                self.emit(MirInst::CallRuntime {
                    dst: Some(dst),
                    def: &pyaot_core_defs::runtime_func_def::RT_GETATTR_NAME_OR_DEFAULT,
                    args: vec![
                        Operand::Local(base),
                        Operand::Local(hash_local),
                        Operand::Local(deflt),
                    ],
                });
            }
            None => {
                self.emit(MirInst::GetFieldNamed {
                    dst,
                    base: Operand::Local(base),
                    name_hash,
                });
            }
        }
        Ok((dst, Repr::Tagged))
    }

    /// Read class attribute `attr_idx` of `cid` and legalize to `want` repr.
    fn read_class_attr(
        &mut self,
        cid: ClassId,
        attr_idx: u32,
        want: Repr,
    ) -> Result<(LocalId, Repr)> {
        let dst = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::GetClassAttr {
            dst,
            class_id: cid,
            attr_idx,
        });
        let coerced = self.coerce(dst, Repr::Tagged, want.clone())?;
        Ok((coerced, want))
    }

    /// Lower an attribute write `base.name = value` → coerce the value to the
    /// uniform tagged field slot (A5) + `SetField`.
    fn lower_setattr(
        &mut self,
        base: Idx<HirExpr>,
        name: InternedString,
        value: Idx<HirExpr>,
    ) -> Result<()> {
        let span = self.func.exprs[base].span;

        // (a) `ClassName.attr = v` → class-level attribute write (Phase 5D).
        if let Some(cid) = self.class_name_ref(base) {
            let idx = self
                .classes
                .get(cid)
                .and_then(|i| i.class_attr(name))
                .map(|a| a.attr_idx)
                .ok_or_else(|| {
                    CompilerError::semantic_error(
                        format!("class has no attribute `.{}`", self.interner.resolve(name)),
                        span,
                    )
                })?;
            let (vl, vr) = self.lower_expr(value)?;
            let vt = self.coerce(vl, vr, Repr::Tagged)?;
            self.emit(MirInst::SetClassAttr {
                class_id: cid,
                attr_idx: idx,
                value: Operand::Local(vt),
            });
            return Ok(());
        }

        let bt = self.func.exprs[base].ty.clone();
        if let Some(cid) = class_of(&bt, self.classes) {
            // (b) `instance.prop = v` → the `@x.setter` call (Phase 5D).
            if let Some(setter) = self
                .classes
                .get(cid)
                .and_then(|i| i.property(name))
                .and_then(|p| p.setter)
            {
                let (bl, br) = self.lower_expr(base)?;
                let (vl, vr) = self.lower_expr(value)?;
                self.emit_dunder_call(setter, vec![(bl, br), (vl, vr)])?;
                return Ok(());
            }
            // (c) `instance.classattr = v` (not an instance field) → class attribute.
            let is_field = self
                .classes
                .get(cid)
                .and_then(|i| i.field_slot(name))
                .is_some();
            if !is_field {
                if let Some(idx) = self
                    .classes
                    .get(cid)
                    .and_then(|i| i.class_attr(name))
                    .map(|a| a.attr_idx)
                {
                    let _ = self.lower_expr(base)?;
                    let (vl, vr) = self.lower_expr(value)?;
                    let vt = self.coerce(vl, vr, Repr::Tagged)?;
                    self.emit(MirInst::SetClassAttr {
                        class_id: cid,
                        attr_idx: idx,
                        value: Operand::Local(vt),
                    });
                    return Ok(());
                }
            }
        }

        // (d) Instance field write. A `Dyn` receiver writes by name hash at
        // runtime (Phase 8H, D4) — AttributeError on a miss/non-instance.
        if matches!(bt, SemTy::Dyn | SemTy::Union(_)) {
            let (bl, br) = self.lower_expr(base)?;
            let bt = self.coerce(bl, br, Repr::Tagged)?;
            let (vl, vr) = self.lower_expr(value)?;
            let vt = self.coerce(vl, vr, Repr::Tagged)?;
            let name_hash = pyaot_utils::fnv1a_hash(self.interner.resolve(name));
            self.emit(MirInst::SetFieldNamed {
                base: Operand::Local(bt),
                name_hash,
                value: Operand::Local(vt),
            });
            return Ok(());
        }
        // (e) Static instance-field write. A `float`-typed field is a tagged slot
        // read back via an unchecked `UnboxFloat`, so an int/bool/gradual value
        // coerces to a real `FloatObj` at the store (numeric tower, §8). The slot
        // type comes from the receiver's class (`bt` is a concrete class here —
        // the `Dyn`/`Union` receivers were handled by the (d) arm above).
        let slot = self.field_slot(base, name)?;
        let field_ty = class_of(&bt, self.classes)
            .and_then(|cid| self.classes.get(cid))
            .and_then(|info| info.field_ty(name).cloned());
        let vty = self.func.exprs[value].ty.clone();
        let (bl, _br) = self.lower_expr(base)?;
        let (vl, vr) = self.lower_expr(value)?;
        let vt = match &field_ty {
            Some(fty) => self.box_float_for_slot(vl, vr, &vty, fty)?,
            None => self.coerce(vl, vr, Repr::Tagged)?,
        };
        self.emit(MirInst::SetField {
            base: Operand::Local(bl),
            slot,
            value: Operand::Local(vt),
        });
        Ok(())
    }

    /// Construct `Cls(args)` (D3): `MakeInstance` → (if `__init__`) a direct
    /// `Call(__init__, [inst, args…])` → yield the instance.
    fn lower_construct(
        &mut self,
        cid: ClassId,
        args: &[Idx<HirExpr>],
        span: pyaot_utils::Span,
    ) -> Result<(LocalId, Repr)> {
        let info = self.classes.get(cid).ok_or_else(|| {
            CompilerError::semantic_error("internal: unknown class id".to_string(), span)
        })?;
        let field_count = info.field_count() as i64;
        let inst_repr = Repr::Heap(HeapShape::Class(cid));
        // §3: a user `__new__` (stored as a static method, `cls`-as-int) is the
        // allocator — it calls `object.__new__(cls)` itself. When present, call
        // it to obtain the instance and SKIP `MakeInstance` (emitting both would
        // double-allocate); otherwise allocate directly.
        let new_fid = info
            .static_methods
            .iter()
            .find(|m| self.interner.resolve(m.name) == "__new__")
            .map(|m| m.func_id);
        let inst = if let Some(nfid) = new_fid {
            self.lower_construct_via_new(cid, nfid, args, &inst_repr, span)?
        } else {
            let inst = self.alloc_temp(inst_repr.clone());
            self.emit(MirInst::MakeInstance {
                dst: inst,
                class_id: cid,
                field_count,
            });
            inst
        };

        let info = self.classes.get(cid).ok_or_else(|| {
            CompilerError::semantic_error("internal: unknown class id".to_string(), span)
        })?;
        let init = info
            .methods
            .iter()
            .find(|m| self.interner.resolve(m.name) == "__init__")
            .map(|m| m.func_id);
        if let Some(fid) = init {
            let params = self.sigs[fid.index()].params.clone();
            let defaults = self.sigs[fid.index()].defaults.clone();
            // Provided args fill the leading params; any remaining trailing params
            // must carry a constant default (Phase 8E — e.g. `Value(x)` with
            // `children=()`/`local_grads=()`). `self` occupies params[0].
            if args.len() + 1 > params.len() {
                return Err(CompilerError::semantic_error(
                    "too many arguments to constructor".to_string(),
                    span,
                ));
            }
            // `self` may target an *inherited* `__init__` whose self repr is a base
            // class — cast the fresh instance to the parent's self repr.
            let self_arg = self.coerce(inst, inst_repr.clone(), params[0].clone())?;
            let mut argvals = vec![Operand::Local(self_arg)];
            for (i, prepr) in params.iter().enumerate().skip(1) {
                let arg_idx = i - 1;
                let at = if arg_idx < args.len() {
                    // A constructor arg into a `float`/`bool` param takes the
                    // CHECKED unbox (`rt_unbox_float`/`rt_unbox_bool`) for a
                    // gradual value — typeck admits `Dyn → float` here (§6), so a
                    // numeric-tower class fed a `Dyn`-demoted field (`NumTower(
                    // self.x + other.x)`) is unboxed soundly rather than misread.
                    let aty = self.func.exprs[args[arg_idx]].ty.clone();
                    let (al, ar) = self.lower_expr(args[arg_idx])?;
                    self.coerce_value(al, ar, &aty, prepr.clone())?
                } else if let Some(def) = &defaults[i] {
                    let (dl, dr) = self.materialize_default(def)?;
                    self.coerce(dl, dr, prepr.clone())?
                } else {
                    return Err(CompilerError::semantic_error(
                        "missing required argument to constructor".to_string(),
                        span,
                    ));
                };
                argvals.push(Operand::Local(at));
            }
            self.emit(MirInst::Call {
                dst: None,
                func: fid,
                args: argvals,
            });
        } else if !args.is_empty() {
            return Err(CompilerError::semantic_error(
                "class has no __init__ to accept constructor arguments".to_string(),
                span,
            ));
        }
        Ok((inst, inst_repr))
    }

    /// §3: allocate `Cls(args)` through a user `__new__` (a static method whose
    /// first param `cls` is the class-id int). Emits `Call(__new__, [cid,
    /// ...args])` and coerces the result to the instance repr; `MakeInstance` is
    /// skipped (the allocation happens inside `__new__` via `object.__new__`).
    /// CPython forwards the constructor args to `__new__` too, so they fill the
    /// trailing params (with constant-default fallback).
    fn lower_construct_via_new(
        &mut self,
        cid: ClassId,
        nfid: FuncId,
        args: &[Idx<HirExpr>],
        inst_repr: &Repr,
        span: pyaot_utils::Span,
    ) -> Result<LocalId> {
        let params = self.sigs[nfid.index()].params.clone();
        let defaults = self.sigs[nfid.index()].defaults.clone();
        let ret = self.sigs[nfid.index()].ret.clone();
        if params.is_empty() {
            return Err(CompilerError::semantic_error(
                "__new__ must take a `cls` parameter".to_string(),
                span,
            ));
        }
        if args.len() + 1 > params.len() {
            return Err(CompilerError::semantic_error(
                "too many arguments to __new__".to_string(),
                span,
            ));
        }
        // params[0] = `cls`, the class-id int.
        let cls_local = self.alloc_temp(params[0].clone());
        self.emit(MirInst::Const {
            dst: cls_local,
            val: Const::Int(cid.0 as i64),
        });
        let mut argvals = vec![Operand::Local(cls_local)];
        for (i, prepr) in params.iter().enumerate().skip(1) {
            let arg_idx = i - 1;
            let at = if arg_idx < args.len() {
                // Forward args through the CHECKED coerce so a `float`/`bool`
                // `__new__` param fed an int/bool/gradual value is unboxed
                // soundly (numeric tower, §8), matching `lower_construct`.
                let aty = self.func.exprs[args[arg_idx]].ty.clone();
                let (al, ar) = self.lower_expr(args[arg_idx])?;
                self.coerce_value(al, ar, &aty, prepr.clone())?
            } else if let Some(def) = &defaults[i] {
                let (dl, dr) = self.materialize_default(def)?;
                self.coerce(dl, dr, prepr.clone())?
            } else {
                return Err(CompilerError::semantic_error(
                    "missing required argument to __new__".to_string(),
                    span,
                ));
            };
            argvals.push(Operand::Local(at));
        }
        let new_dst = self.alloc_temp(ret.clone());
        self.emit(MirInst::Call {
            dst: Some(new_dst),
            func: nfid,
            args: argvals,
        });
        // `__new__` returns the fresh instance; legalize to the instance repr.
        self.coerce(new_dst, ret, inst_repr.clone())
    }

    /// Materialize a parameter default (Phase 8E) as a fresh MIR value. A `Const`
    /// literal mirrors how `lower_expr` lowers the equivalent literal; a `Slot`
    /// reads the once-evaluated GC-rooted global (the shared mutable/computed
    /// top-level default), leaving the caller's existing `coerce(.., prepr)` to
    /// reinterpret the tagged slot value into the param repr.
    fn materialize_default(&mut self, init: &pyaot_hir::ParamDefault) -> Result<(LocalId, Repr)> {
        use pyaot_hir::ParamDefault as PD;
        match init {
            PD::Const(c) => self.materialize_const_default(c),
            PD::Slot(var_id) => {
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::GlobalGet {
                    dst,
                    var_id: *var_id,
                });
                Ok((dst, Repr::Tagged))
            }
        }
    }

    /// Materialize a constant (literal) parameter default. The empty-tuple
    /// default builds a fresh zero-length tuple (immutable, so per-call freshness
    /// is unobservable).
    fn materialize_const_default(
        &mut self,
        init: &pyaot_hir::ClassAttrInit,
    ) -> Result<(LocalId, Repr)> {
        use pyaot_hir::ClassAttrInit as A;
        Ok(match init {
            A::Int(v) => {
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::Const {
                    dst,
                    val: Const::Int(*v),
                });
                (dst, Repr::Tagged)
            }
            A::Bool(b) => {
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::Const {
                    dst,
                    val: Const::Bool(*b),
                });
                (dst, Repr::Tagged)
            }
            A::None => {
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::Const {
                    dst,
                    val: Const::None,
                });
                (dst, Repr::Tagged)
            }
            A::Float(f) => {
                let dst = self.alloc_temp(Repr::Raw(RawKind::F64));
                self.emit(MirInst::Const {
                    dst,
                    val: Const::Float(*f),
                });
                (dst, Repr::Raw(RawKind::F64))
            }
            A::Str(s) => {
                self.str_pool
                    .insert(*s, self.interner.resolve(*s).as_bytes().to_vec());
                let dst = self.alloc_temp(Repr::Heap(HeapShape::Str));
                self.emit(MirInst::Const {
                    dst,
                    val: Const::Str(*s),
                });
                (dst, Repr::Heap(HeapShape::Str))
            }
            A::Bytes(s) => {
                // RAW bytes (a non-UTF-8 `b"\xff"` class-attr default would panic
                // through `resolve(...).as_bytes()`).
                self.str_pool
                    .insert(*s, self.interner.resolve_bytes(*s).to_vec());
                let dst = self.alloc_temp(Repr::Heap(HeapShape::Bytes));
                self.emit(MirInst::Const {
                    dst,
                    val: Const::Bytes(*s),
                });
                (dst, Repr::Heap(HeapShape::Bytes))
            }
            A::BigInt(s) => {
                self.str_pool
                    .insert(*s, self.interner.resolve(*s).as_bytes().to_vec());
                let dst = self.alloc_temp(Repr::Heap(HeapShape::BigInt));
                self.emit(MirInst::Const {
                    dst,
                    val: Const::BigIntStr(*s),
                });
                (dst, Repr::Heap(HeapShape::BigInt))
            }
            A::EmptyTuple => {
                let tup_repr = Repr::Heap(HeapShape::Tuple(vec![]));
                let size = self.raw_i64_const(0);
                let (tup, ret) = self.emit_container(
                    ContainerOp::TupleNew,
                    vec![(size, Repr::Raw(RawKind::I64))],
                    Some(tup_repr),
                )?;
                (tup.expect("TupleNew produces a tuple"), ret)
            }
        })
    }

    /// Dispatch a `recv.method(args)` call by the receiver's static type.
    fn lower_method_call(
        &mut self,
        call_idx: Idx<HirExpr>,
        recv: Idx<HirExpr>,
        method_name: InternedString,
        args: &[Idx<HirExpr>],
        kwargs: &[(InternedString, Idx<HirExpr>)],
    ) -> Result<(LocalId, Repr)> {
        let span = self.func.exprs[recv].span;
        // `super().m()` → resolve the parent method from the enclosing MRO and
        // direct-call with the current `self` (super is statically resolvable).
        if let HirExprKind::Super(cid) = self.func.exprs[recv].kind {
            return self.lower_super_call(cid, method_name, args, kwargs, span);
        }
        // `ClassName.m(args)` → a `@staticmethod`/`@classmethod` on the class (5D).
        if let Some(cid) = self.class_name_ref(recv) {
            if let Some(res) =
                self.try_static_or_class_call(cid, None, method_name, args, kwargs, span)?
            {
                return Ok(res);
            }
            return Err(CompilerError::semantic_error(
                format!(
                    "class has no static/class method `.{}()`",
                    self.interner.resolve(method_name)
                ),
                span,
            ));
        }
        let recv_ty = self.func.exprs[recv].ty.clone();
        // Gradual completeness: a `Dyn`/`Union` receiver has no statically-known
        // shape, so dispatch the method at run time by the receiver's tag via
        // the unified `rt_obj_method` (the CPython `type(obj).method` model —
        // container methods AND user methods alike). Placed BEFORE the
        // `class_of` check (which is `None` for `Dyn`/`Union`) and the container
        // path (which rejects a tagless receiver, `lib.rs` "method calls require
        // a statically-known …"). The result rides the tagged baseline; the
        // consuming seam recovers precision via the Phase-1 checked unbox.
        if matches!(recv_ty, SemTy::Dyn | SemTy::Union(_)) {
            return self.lower_dyn_method_call(recv, method_name, args, kwargs, span);
        }
        // Class receiver: a `@staticmethod`/`@classmethod` called on an instance,
        // else an instance method (devirtualized unless overridden below — D7).
        if let Some(cid) = class_of(&recv_ty, self.classes) {
            if let Some(res) =
                self.try_static_or_class_call(cid, Some(recv), method_name, args, kwargs, span)?
            {
                return Ok(res);
            }
            if self.classes.method_overridden_below(cid, method_name) {
                return self.lower_virtual_call(cid, recv, method_name, args, kwargs, span);
            }
            return self.lower_class_method_call(cid, recv, method_name, args, kwargs, span);
        }
        // `str.encode` / `bytes.decode` (§9) accept positional-OR-keyword
        // `encoding`/`errors` and a 3-arg codec ABI — handle them before the
        // generic no-keyword gate below (which would otherwise reject `errors=`).
        let codec = match &recv_ty {
            SemTy::Str if self.interner.resolve(method_name) == "encode" => Some((
                &pyaot_core_defs::runtime_func_def::RT_STR_ENCODE,
                pyaot_stdlib_defs::TypeSpec::Bytes,
            )),
            SemTy::Bytes if self.interner.resolve(method_name) == "decode" => Some((
                &pyaot_core_defs::runtime_func_def::RT_BYTES_DECODE,
                pyaot_stdlib_defs::TypeSpec::Str,
            )),
            _ => None,
        };
        if let Some((def, ret_spec)) = codec {
            return self.lower_codec_method(
                call_idx,
                recv,
                method_name,
                def,
                &ret_spec,
                args,
                kwargs,
                span,
            );
        }
        // Keywords on non-class receivers (Phase 10): only `list.sort` consumes
        // them (`reverse=` / a literal `key=None`; a real `key=` was desugared
        // by the frontend). Every other container / str / file / stdlib-object
        // method is a precise diagnostic — the mechanism is ready when those
        // surfaces grow keyword parameters.
        if !kwargs.is_empty()
            && ContainerMethod::from_name(self.interner.resolve(method_name))
                != Some(ContainerMethod::Sort)
        {
            return Err(CompilerError::semantic_error(
                format!(
                    "`.{}()` takes no keyword arguments",
                    self.interner.resolve(method_name)
                ),
                span,
            ));
        }
        // A str-receiver method routed through its runtime descriptor (Phase
        // 8B/8C; the full str-method surface lands with 8E). Covers the
        // zero-arg `str → str` trio and the one-arg `startswith`/`endswith`
        // (`→ bool`) / `find` (`→ int`, op_tag 0).
        if matches!(recv_ty, SemTy::Str) {
            if let Some(res) = self.lower_str_method(call_idx, recv, method_name, args, span)? {
                return Ok(res);
            }
        }
        // A stdlib runtime object method (`m.group(0)`, Phase 8C): resolved via
        // the object-type registry, lowered to its `CallRuntime` descriptor.
        if let SemTy::RuntimeObject(tag) = &recv_ty {
            let name = self.interner.resolve(method_name);
            let method = pyaot_stdlib_defs::object_types::lookup_object_method(*tag, name)
                .ok_or_else(|| {
                    CompilerError::semantic_error(
                        format!("runtime object has no method `.{name}()`"),
                        span,
                    )
                })?;
            return self.lower_runtime_object_method(call_idx, recv, method, args, span);
        }
        // File methods (Phase 8C): a fixed surface mapped to `rt_file_*`.
        if let SemTy::File { binary } = &recv_ty {
            return self.lower_file_method(call_idx, recv, method_name, args, *binary, span);
        }
        // A bytes-receiver method routed through its `rt_bytes_*` descriptor
        // (§9; the exact sibling of `lower_str_method`). Covers `decode` (Phase
        // 8D) plus the §9 batch (startswith/endswith/find/rfind/count/replace/
        // split/rsplit/strip family/upper/lower/join).
        if matches!(recv_ty, SemTy::Bytes) {
            if let Some(res) = self.lower_bytes_method(call_idx, recv, method_name, args, span)? {
                return Ok(res);
            }
        }
        // int / bool receiver methods (§9): `bit_length`/`bit_count` (bignum-aware
        // runtime counts) and `conjugate`/`__index__` (the receiver's int value).
        if matches!(recv_ty, SemTy::Int | SemTy::Bool) {
            if let Some(res) = self.lower_int_method(recv, method_name, args, span)? {
                return Ok(res);
            }
        }
        // Container receiver → the Phase-4D ContainerMethod path.
        let cm =
            ContainerMethod::from_name(self.interner.resolve(method_name)).ok_or_else(|| {
                CompilerError::semantic_error(
                    format!(
                        "unsupported method `.{}()` (receiver type {:?})",
                        self.interner.resolve(method_name),
                        recv_ty
                    ),
                    span,
                )
            })?;
        self.lower_container_method_call(call_idx, recv, cm, args, kwargs)
    }

    /// Lower `recv.method(args, kwargs)` for a `Dyn`/`Union` receiver to the
    /// unified runtime dispatcher [`RT_OBJ_METHOD`]: the receiver coerced to
    /// `Tagged`, the FNV-1a method-name hash as a RAW `i64` immediate, the
    /// positional args packed into a `tuple[Tagged]` (the exact tuple-build of
    /// [`Self::lower_indirect_call`]), and the keywords as a `dict[str, Tagged]`
    /// or the null sentinel. The runtime decides by the receiver's tag —
    /// container methods route to the typed `rt_*` family, an instance to its
    /// uniform thunk. Result is the tagged baseline (`Dyn`, GC-rooted).
    fn lower_dyn_method_call(
        &mut self,
        recv: Idx<HirExpr>,
        method_name: InternedString,
        args: &[Idx<HirExpr>],
        kwargs: &[(InternedString, Idx<HirExpr>)],
        _span: pyaot_utils::Span,
    ) -> Result<(LocalId, Repr)> {
        // Receiver → Tagged.
        let (rl, rr) = self.lower_expr(recv)?;
        let recv_t = self.coerce(rl, rr, Repr::Tagged)?;

        // Method-name hash: a RAW `i64` immediate (like `GetFieldNamed`).
        let name_hash = pyaot_utils::fnv1a_hash(self.interner.resolve(method_name));
        let hash_local = self.alloc_temp(Repr::Raw(RawKind::I64));
        self.emit(MirInst::Const {
            dst: hash_local,
            val: Const::Int(name_hash as i64),
        });

        // Pack the positional args into a `tuple[Tagged]` (the uniform-call shape).
        let tup_repr = Repr::Heap(HeapShape::TupleVar(Box::new(Repr::Tagged)));
        let size = self.raw_i64_const(args.len() as i64);
        let (tup, _) = self.emit_container(
            ContainerOp::TupleNew,
            vec![(size, Repr::Raw(RawKind::I64))],
            Some(tup_repr.clone()),
        )?;
        let tup = tup.expect("TupleNew produces a tuple");
        for (i, a) in args.iter().enumerate() {
            let (al, ar) = self.lower_expr(*a)?;
            let pos = self.raw_i64_const(i as i64);
            self.emit_container(
                ContainerOp::TupleSet,
                vec![
                    (tup, tup_repr.clone()),
                    (pos, Repr::Raw(RawKind::I64)),
                    (al, ar),
                ],
                None,
            )?;
        }
        let args_tuple = self.coerce(tup, tup_repr, Repr::Tagged)?;

        // Keywords → a `dict[str, Tagged]`, or the null `__kwargs__` sentinel on
        // the common (no-keyword) path (no allocation; the dispatcher reads it
        // only for a user method with keyword params).
        let kwargs_op = if kwargs.is_empty() {
            let k = self.alloc_temp(Repr::Tagged);
            self.emit(MirInst::Const {
                dst: k,
                val: Const::NullPtr,
            });
            k
        } else {
            let dict_repr = Repr::Heap(HeapShape::Dict(
                Box::new(Repr::Tagged),
                Box::new(Repr::Tagged),
            ));
            let (d, _) = self.empty_container(ContainerOp::DictNew, dict_repr.clone())?;
            for (kname, kexpr) in kwargs {
                self.str_pool
                    .insert(*kname, self.interner.resolve(*kname).as_bytes().to_vec());
                let key = self.alloc_temp(Repr::Heap(HeapShape::Str));
                self.emit(MirInst::Const {
                    dst: key,
                    val: Const::Str(*kname),
                });
                let key_t = self.coerce(key, Repr::Heap(HeapShape::Str), Repr::Tagged)?;
                let (vl, vr) = self.lower_expr(*kexpr)?;
                let val_t = self.coerce(vl, vr, Repr::Tagged)?;
                self.emit_container(
                    ContainerOp::DictSet,
                    vec![
                        (d, dict_repr.clone()),
                        (key_t, Repr::Tagged),
                        (val_t, Repr::Tagged),
                    ],
                    None,
                )?;
            }
            self.coerce(d, dict_repr, Repr::Tagged)?
        };

        let dst = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::CallRuntime {
            dst: Some(dst),
            def: &pyaot_core_defs::runtime_func_def::RT_OBJ_METHOD,
            args: vec![
                Operand::Local(recv_t),
                Operand::Local(hash_local),
                Operand::Local(args_tuple),
                Operand::Local(kwargs_op),
            ],
        });
        Ok((dst, Repr::Tagged))
    }

    /// Devirtualized class-method call: `Call(method_FuncId, [recv, args…])`.
    fn lower_class_method_call(
        &mut self,
        cid: ClassId,
        recv: Idx<HirExpr>,
        method_name: InternedString,
        args: &[Idx<HirExpr>],
        kwargs: &[(InternedString, Idx<HirExpr>)],
        span: pyaot_utils::Span,
    ) -> Result<(LocalId, Repr)> {
        let fid = self
            .classes
            .get(cid)
            .and_then(|info| info.method(method_name))
            .map(|m| m.func_id)
            .ok_or_else(|| {
                CompilerError::semantic_error(
                    format!(
                        "class has no method `.{}()`",
                        self.interner.resolve(method_name)
                    ),
                    span,
                )
            })?;
        let ret = self.sigs[fid.index()].ret.clone();
        let self_param = self.sigs[fid.index()].params[0].clone();
        let (rl, rr) = self.lower_expr(recv)?;
        let self_arg = self.coerce(rl, rr, self_param)?;
        let mut argvals = vec![Operand::Local(self_arg)];
        argvals.extend(self.build_call_operands(fid, true, args, kwargs, span)?);
        let dst = self.alloc_temp(ret.clone());
        self.emit(MirInst::Call {
            dst: Some(dst),
            func: fid,
            args: argvals,
        });
        Ok((dst, ret))
    }

    /// Build a direct call's non-`self` operand vector, adapting the call-site
    /// positional + keyword args to the callee's MIR signature: each fixed
    /// param slot is filled from a positional, a keyword, or its constant
    /// default ([`pyaot_hir::match_keywords`]) and coerced to the param's
    /// `Repr`; a `*args` callee packs excess positionals into a fresh tuple;
    /// a `**kwargs` callee collects leftover keywords into a dict (Phase 7D /
    /// Phase 10). When keywords are present the frontend has already staged
    /// every value, so per-slot evaluation order here cannot reorder effects.
    fn build_call_operands(
        &mut self,
        fid: FuncId,
        skip_self: bool,
        args: &[Idx<HirExpr>],
        kwargs: &[(InternedString, Idx<HirExpr>)],
        span: pyaot_utils::Span,
    ) -> Result<Vec<Operand>> {
        let params = self.sigs[fid.index()].params.clone();
        let param_names = self.sigs[fid.index()].param_names.clone();
        let defaults = self.sigs[fid.index()].defaults.clone();
        let varargs = self.sigs[fid.index()].varargs;
        let has_kwargs = self.sigs[fid.index()].kwargs;
        let first = usize::from(skip_self);
        let fixed = params.len() - first - usize::from(varargs) - usize::from(has_kwargs);
        if args.len() > fixed && !varargs {
            return Err(CompilerError::semantic_error(
                "wrong number of arguments in method call".to_string(),
                span,
            ));
        }
        let n_pos = args.len().min(fixed);
        let has_default: Vec<bool> = defaults[first..first + fixed]
            .iter()
            .map(|d| d.is_some())
            .collect();
        let kw_names: Vec<InternedString> = kwargs.iter().map(|(n, _)| *n).collect();
        let m = pyaot_hir::match_keywords(
            &param_names[first..first + fixed],
            &has_default,
            n_pos,
            &kw_names,
            has_kwargs,
        )
        .map_err(|e| self.kw_match_error(e, span))?;
        // Evaluate positionals then keyword values in CALL-SITE order before
        // assembling slots (kwarg values are frontend-staged local refs, but
        // keeping the lowering order written-shaped costs nothing).
        // Carry each call-site value's static `SemTy` alongside its lowered
        // `(loc, repr)` so the slot fill can pick the CHECKED numeric coercion
        // (`coerce_value`) when an int/bool/gradual arg lands in a `float`/`bool`
        // param (the numeric tower, PLAN §8) — covering instance / static /
        // classmethod / `super()` / virtual / `__call__` dispatch (all funnel
        // here). Default / `*args` / `**kwargs` slots carry no call-site expr; a
        // `Dyn` placeholder is correct either way (`coerce_value` round-trips it).
        let mut pos_vals = Vec::with_capacity(args.len());
        let mut pos_tys = Vec::with_capacity(args.len());
        for a in args {
            pos_tys.push(self.func.exprs[*a].ty.clone());
            pos_vals.push(self.lower_expr(*a)?);
        }
        let mut kw_vals = Vec::with_capacity(kwargs.len());
        let mut kw_tys = Vec::with_capacity(kwargs.len());
        for (_, e) in kwargs {
            kw_tys.push(self.func.exprs[*e].ty.clone());
            kw_vals.push(self.lower_expr(*e)?);
        }
        let mut argvals = Vec::with_capacity(params.len() - first);
        for (i, src) in m.slots.iter().enumerate() {
            let prepr = params[first + i].clone();
            let (vl, vr, ty) = match src {
                pyaot_hir::SlotSource::Pos(p) => {
                    let (l, r) = pos_vals[*p].clone();
                    (l, r, pos_tys[*p].clone())
                }
                pyaot_hir::SlotSource::Kw(k) => {
                    let (l, r) = kw_vals[*k].clone();
                    (l, r, kw_tys[*k].clone())
                }
                pyaot_hir::SlotSource::Default => {
                    let def = defaults[first + i]
                        .as_ref()
                        .expect("match_keywords picked Default only for defaulted params")
                        .clone();
                    let (l, r) = self.materialize_default(&def)?;
                    (l, r, SemTy::Dyn)
                }
            };
            argvals.push(Operand::Local(self.coerce_value(vl, vr, &ty, prepr)?));
        }
        if varargs {
            let tup_repr = params[first + fixed].clone();
            let excess = &pos_vals[n_pos..];
            let size = self.raw_i64_const(excess.len() as i64);
            let (tup, _) = self.emit_container(
                ContainerOp::TupleNew,
                vec![(size, Repr::Raw(RawKind::I64))],
                Some(tup_repr.clone()),
            )?;
            let tup = tup.expect("TupleNew produces a tuple");
            for (i, (el, er)) in excess.iter().cloned().enumerate() {
                let pos = self.raw_i64_const(i as i64);
                self.emit_container(
                    ContainerOp::TupleSet,
                    vec![
                        (tup, tup_repr.clone()),
                        (pos, Repr::Raw(RawKind::I64)),
                        (el, er),
                    ],
                    None,
                )?;
            }
            argvals.push(Operand::Local(tup));
        }
        if has_kwargs {
            let dict_repr = params.last().expect("kwargs param exists").clone();
            let (d, _) = self.empty_container(ContainerOp::DictNew, dict_repr.clone())?;
            // Leftover keywords (written order) land in the `**kwargs` dict.
            for &k in &m.leftover {
                let name = kw_names[k];
                self.str_pool
                    .insert(name, self.interner.resolve(name).as_bytes().to_vec());
                let key = self.alloc_temp(Repr::Heap(HeapShape::Str));
                self.emit(MirInst::Const {
                    dst: key,
                    val: Const::Str(name),
                });
                let key_t = self.coerce(key, Repr::Heap(HeapShape::Str), Repr::Tagged)?;
                let (vl, vr) = kw_vals[k].clone();
                let val_t = self.coerce(vl, vr, Repr::Tagged)?;
                self.emit_container(
                    ContainerOp::DictSet,
                    vec![
                        (d, dict_repr.clone()),
                        (key_t, Repr::Tagged),
                        (val_t, Repr::Tagged),
                    ],
                    None,
                )?;
            }
            argvals.push(Operand::Local(d));
        }
        Ok(argvals)
    }

    /// Verify every override of `method_name` below `cid` declares the same
    /// parameter names and constant defaults as the statically resolved method
    /// — the precondition for call-site keyword/default adaptation on a
    /// virtual call (the actual callee is chosen at runtime).
    fn check_override_kw_compat(
        &self,
        cid: ClassId,
        method_name: InternedString,
        fid: FuncId,
        span: pyaot_utils::Span,
    ) -> Result<()> {
        let base = &self.sigs[fid.index()];
        for info in self.classes.iter() {
            if info.class_id == cid || !info.mro.contains(&cid) {
                continue;
            }
            if let Some((_, ofid)) = info.own_methods.iter().find(|(n, _)| *n == method_name) {
                let over = &self.sigs[ofid.index()];
                if over.param_names[1..] != base.param_names[1..]
                    || over.defaults[1..] != base.defaults[1..]
                {
                    return Err(CompilerError::semantic_error(
                        format!(
                            "keyword/default adaptation of virtual `.{}()` requires \
                             identical parameter names and defaults across overrides \
                             (class `{}` differs) — pass the arguments positionally",
                            self.interner.resolve(method_name),
                            self.interner.resolve(info.name),
                        ),
                        span,
                    ));
                }
            }
        }
        Ok(())
    }

    /// Render a [`pyaot_hir::KwMatchError`] as a CPython-flavored diagnostic.
    fn kw_match_error(&self, e: pyaot_hir::KwMatchError, span: pyaot_utils::Span) -> CompilerError {
        use pyaot_hir::KwMatchError as E;
        let msg = match e {
            E::Unexpected(n) => format!(
                "got an unexpected keyword argument `{}`",
                self.interner.resolve(n)
            ),
            E::Duplicate(n) => format!(
                "got multiple values for argument `{}`",
                self.interner.resolve(n)
            ),
            E::Missing(n) => format!("missing required argument `{}`", self.interner.resolve(n)),
            E::TooManyPositional { expected, got } => {
                format!("takes {expected} positional argument(s) but {got} were given")
            }
        };
        CompilerError::semantic_error(msg, span)
    }

    /// If `name` is a `@staticmethod` / `@classmethod` on `cid`, lower it as a
    /// direct call (the `cls` of a classmethod was dropped at the frontend; both
    /// take just the positional args). `recv` (an instance) is evaluated for side
    /// effects when present. Returns `None` if `name` is not static/class.
    fn try_static_or_class_call(
        &mut self,
        cid: ClassId,
        recv: Option<Idx<HirExpr>>,
        name: InternedString,
        args: &[Idx<HirExpr>],
        kwargs: &[(InternedString, Idx<HirExpr>)],
        span: pyaot_utils::Span,
    ) -> Result<Option<(LocalId, Repr)>> {
        let fid = match self.classes.get(cid).and_then(|i| {
            i.static_method(name)
                .or_else(|| i.class_method(name))
                .map(|m| m.func_id)
        }) {
            Some(f) => f,
            None => return Ok(None),
        };
        // Evaluate the receiver for side effects (`instance.staticmethod()`).
        if let Some(r) = recv {
            let _ = self.lower_expr(r)?;
        }
        let ret = self.sigs[fid.index()].ret.clone();
        // No `self` param (the classmethod `cls` was dropped at the frontend).
        let argvals = self.build_call_operands(fid, false, args, kwargs, span)?;
        let dst = self.alloc_temp(ret.clone());
        self.emit(MirInst::Call {
            dst: Some(dst),
            func: fid,
            args: argvals,
        });
        Ok(Some((dst, ret)))
    }

    /// Lower `super().m(args)`: resolve the parent method via the enclosing class's
    /// MRO and direct-`Call` it with the *current* `self` (param 0), cast to the
    /// parent's `self` repr.
    fn lower_super_call(
        &mut self,
        cid: ClassId,
        method_name: InternedString,
        args: &[Idx<HirExpr>],
        kwargs: &[(InternedString, Idx<HirExpr>)],
        span: pyaot_utils::Span,
    ) -> Result<(LocalId, Repr)> {
        let fid = self
            .classes
            .resolve_super_method(cid, method_name)
            .ok_or_else(|| {
                CompilerError::semantic_error(
                    format!(
                        "super() has no method `.{}()`",
                        self.interner.resolve(method_name)
                    ),
                    span,
                )
            })?;
        let params = self.sigs[fid.index()].params.clone();
        let ret = self.sigs[fid.index()].ret.clone();
        // `self` is parameter 0 of the current method.
        let self_local = LocalId::new(0);
        let self_from = self.local_repr(self_local);
        let self_arg = self.coerce(self_local, self_from, params[0].clone())?;
        let mut argvals = vec![Operand::Local(self_arg)];
        argvals.extend(self.build_call_operands(fid, true, args, kwargs, span)?);
        let dst = self.alloc_temp(ret.clone());
        self.emit(MirInst::Call {
            dst: Some(dst),
            func: fid,
            args: argvals,
        });
        Ok((dst, ret))
    }

    /// Lower a polymorphic method call → `CallVirtual` (D7). The statically
    /// resolved method on `cid` provides the indirect-call signature; the runtime
    /// resolves the actual override from the receiver's class id.
    fn lower_virtual_call(
        &mut self,
        cid: ClassId,
        recv: Idx<HirExpr>,
        method_name: InternedString,
        args: &[Idx<HirExpr>],
        kwargs: &[(InternedString, Idx<HirExpr>)],
        span: pyaot_utils::Span,
    ) -> Result<(LocalId, Repr)> {
        let fid = self
            .classes
            .get(cid)
            .and_then(|info| info.method(method_name))
            .map(|m| m.func_id)
            .ok_or_else(|| {
                CompilerError::semantic_error(
                    format!(
                        "class has no method `.{}()`",
                        self.interner.resolve(method_name)
                    ),
                    span,
                )
            })?;
        let params = self.sigs[fid.index()].params.clone();
        let ret = self.sigs[fid.index()].ret.clone();
        // Keyword/default adaptation happens at the CALL SITE against the
        // statically resolved method — CPython resolves both in the ACTUAL
        // callee's frame. Sound only when every override agrees on parameter
        // names and defaults; otherwise a Derived receiver would observe
        // Base's defaults. Reject loudly when the call relies on either.
        let fixed = params.len()
            - 1
            - usize::from(self.sigs[fid.index()].varargs)
            - usize::from(self.sigs[fid.index()].kwargs);
        if !kwargs.is_empty() || args.len() < fixed {
            self.check_override_kw_compat(cid, method_name, fid, span)?;
        }
        let (rl, rr) = self.lower_expr(recv)?;
        let self_arg = self.coerce(rl, rr, params[0].clone())?;
        let argvals = self.build_call_operands(fid, true, args, kwargs, span)?;
        let name_hash = pyaot_utils::fnv1a_hash(self.interner.resolve(method_name));
        let dst = self.alloc_temp(ret.clone());
        self.emit(MirInst::CallVirtual {
            dst: Some(dst),
            recv: Operand::Local(self_arg),
            name_hash,
            args: argvals,
            ret: ret.clone(),
        });
        Ok((dst, ret))
    }

    /// Lower `isinstance(value, Cls)` → the inheritance-aware runtime check, or —
    /// for a `Protocol` class (PLAN §3 H) — a structural method-presence check.
    fn lower_isinstance(
        &mut self,
        value: Idx<HirExpr>,
        class_id: ClassId,
    ) -> Result<(LocalId, Repr)> {
        let (vl, vr) = self.lower_expr(value)?;
        let vt = self.coerce(vl, vr, Repr::Tagged)?;
        // A `Protocol` class is checked STRUCTURALLY: probe the receiver for each
        // method the protocol declares (existence only, dunders included) via the
        // existing `rt_obj_has_method` primitive — no new IR, no nominal MRO walk.
        if self.classes.get(class_id).is_some_and(|c| c.is_protocol) {
            return self.lower_protocol_isinstance(vt, class_id);
        }
        let dst = self.alloc_temp(Repr::Raw(RawKind::I8));
        self.emit(MirInst::IsInstance {
            dst,
            value: Operand::Local(vt),
            class_id,
        });
        Ok((dst, Repr::Raw(RawKind::I8)))
    }

    /// Structural `isinstance(obj, P)` for a protocol `P` (PLAN §3 H): True iff the
    /// receiver has EVERY method `P` declares. Each method is probed with
    /// `rt_obj_has_method` (returns 0 for a non-instance receiver, so
    /// `isinstance(42, P)` is correctly False); the per-method `Raw(I8)` flags are
    /// AND-combined as Tagged booleans — a raw-`I8` `BitAnd` is not a legal MIR
    /// fast-path (the verifier admits raw arithmetic only on `Raw(F64)`/`Raw(I64)`)
    /// — then read back to `Raw(I8)` so the result is bit-for-bit the nominal
    /// path's shape (tuple-of-types `or_combine` stays uniform). An empty protocol
    /// → `True` for any receiver.
    fn lower_protocol_isinstance(
        &mut self,
        recv_t: LocalId,
        class_id: ClassId,
    ) -> Result<(LocalId, Repr)> {
        let method_names: Vec<InternedString> = self
            .classes
            .get(class_id)
            .expect("protocol class resolved")
            .methods
            .iter()
            .map(|m| m.name)
            .collect();
        // Empty protocol: every object satisfies it.
        if method_names.is_empty() {
            let t = self.alloc_temp(Repr::Tagged);
            self.emit(MirInst::Const {
                dst: t,
                val: Const::Bool(true),
            });
            let dst = self.coerce(t, Repr::Tagged, Repr::Raw(RawKind::I8))?;
            return Ok((dst, Repr::Raw(RawKind::I8)));
        }
        let mut acc: Option<LocalId> = None; // Tagged-boolean accumulator.
        for name in method_names {
            // The method-name hash is a RAW `i64` immediate emitted DIRECTLY into a
            // `Raw(I64)` local (as `lower_dyn_method_call` does) — NOT via
            // `raw_i64_const`, whose `Const::Int → Tagged → untag` round-trip would
            // drop the top 3 bits of a 64-bit FNV hash (no fixnum range here).
            let hash = pyaot_utils::fnv1a_hash(self.interner.resolve(name)) as i64;
            let hash_local = self.alloc_temp(Repr::Raw(RawKind::I64));
            self.emit(MirInst::Const {
                dst: hash_local,
                val: Const::Int(hash),
            });
            let flag = self.alloc_temp(Repr::Raw(RawKind::I8));
            self.emit(MirInst::CallRuntime {
                dst: Some(flag),
                def: &pyaot_core_defs::runtime_func_def::RT_OBJ_HAS_METHOD,
                args: vec![Operand::Local(recv_t), Operand::Local(hash_local)],
            });
            let flag_t = self.coerce(flag, Repr::Raw(RawKind::I8), Repr::Tagged)?;
            acc = Some(match acc {
                None => flag_t,
                Some(prev) => {
                    let dst = self.alloc_temp(Repr::Tagged);
                    self.emit(MirInst::BinOp {
                        dst,
                        op: MBinOp::BitAnd,
                        l: Operand::Local(prev),
                        r: Operand::Local(flag_t),
                    });
                    dst
                }
            });
        }
        let acc = acc.expect("method set is non-empty here");
        let dst = self.coerce(acc, Repr::Tagged, Repr::Raw(RawKind::I8))?;
        Ok((dst, Repr::Raw(RawKind::I8)))
    }

    /// True iff `ty` statically denotes a caught exception value (a builtin
    /// exception, a user exception class, or a tuple-clause `Union` of those)
    /// — drives the `str(e)`/`print(e)` message routing (Phase 7B/7C).
    fn is_exception_value(&self, ty: &SemTy) -> bool {
        match ty {
            SemTy::BuiltinException(_) => true,
            SemTy::Union(members) => {
                !members.is_empty() && members.iter().all(|m| self.is_exception_value(m))
            }
            _ => class_of(ty, self.classes).is_some_and(|cid| self.classes.is_exception_class(cid)),
        }
    }

    /// If `ty` is a concrete user class defining the dunder `name` (and not
    /// overridden in a subclass — else the dynamic path is needed), return its
    /// resolved `FuncId`. Used to compiler-route the dunders the runtime does *not*
    /// dispatch (`__eq__`/`__len__`/`__getitem__`/…) to a direct devirtualized call.
    fn concrete_dunder(&self, ty: &SemTy, name: &str) -> Option<FuncId> {
        let cid = class_of(ty, self.classes)?;
        let info = self.classes.get(cid)?;
        let m = info
            .methods
            .iter()
            .find(|m| self.interner.resolve(m.name) == name)?;
        if self.classes.method_overridden_below(cid, m.name) {
            return None;
        }
        Some(m.func_id)
    }

    /// Emit a direct call to a dunder `FuncId` with already-lowered `(loc, repr)`
    /// args (coerced to the method's param reprs), returning its result.
    fn emit_dunder_call(
        &mut self,
        fid: FuncId,
        args: Vec<(LocalId, Repr)>,
    ) -> Result<(LocalId, Repr)> {
        let params = self.sigs[fid.index()].params.clone();
        let ret = self.sigs[fid.index()].ret.clone();
        let mut argvals = Vec::with_capacity(args.len());
        for ((loc, repr), prepr) in args.into_iter().zip(&params) {
            // Defensive numeric-tower hardening (§8): operator/property-setter
            // dunders reach here with `(loc, repr)` but no per-arg `SemTy`, so a
            // `float`/`bool`-typed dunder param fed a tagged int would do a wild
            // unchecked `UnboxFloat` (latent SEGV — no typeck guards this path).
            // Routing through `coerce_value` with `Dyn` makes a `Raw(F64)`/
            // `Raw(I8)`/`Raw(I64)` param take the CHECKED unbox (defined
            // TypeError / correct int→f64); a non-raw param stays a plain coerce.
            argvals.push(Operand::Local(self.coerce_value(
                loc,
                repr,
                &SemTy::Dyn,
                prepr.clone(),
            )?));
        }
        let dst = self.alloc_temp(ret.clone());
        self.emit(MirInst::Call {
            dst: Some(dst),
            func: fid,
            args: argvals,
        });
        Ok((dst, ret))
    }

    /// Truthy-test a tagged value into a fresh `Raw(I8)` (for `__eq__`/`__contains__`
    /// / ordering dunder results, which return a Python bool object).
    fn truthy_i8(&mut self, val: LocalId, repr: Repr) -> Result<LocalId> {
        let tagged = self.coerce(val, repr, Repr::Tagged)?;
        let dst = self.alloc_temp(Repr::Raw(RawKind::I8));
        self.emit(MirInst::Truthy {
            dst,
            operand: Operand::Local(tagged),
        });
        Ok(dst)
    }

    /// The `__eq__`/`__ne__` dunder to route `==`/`!=` through, plus whether to
    /// logically negate the result (the `!=`-from-`__eq__` derivation).
    fn eq_dunder(&self, ty: &SemTy, op: HCmpOp) -> Option<(FuncId, bool)> {
        match op {
            HCmpOp::Eq => self.concrete_dunder(ty, "__eq__").map(|f| (f, false)),
            HCmpOp::NotEq => match self.concrete_dunder(ty, "__ne__") {
                Some(f) => Some((f, false)),
                None => self.concrete_dunder(ty, "__eq__").map(|f| (f, true)),
            },
            _ => None,
        }
    }

    // ── container expressions (Phase 4) ──────────────────────────────────────

    /// Materialize a small `Raw(I64)` constant (a capacity / size / count). The
    /// value is a compile-time element count well within the fixnum range, so the
    /// `Tagged → Raw(I64)` untag round-trips soundly.
    fn raw_i64_const(&mut self, n: i64) -> LocalId {
        let t = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::Const {
            dst: t,
            val: Const::Int(n),
        });
        self.coerce(t, Repr::Tagged, Repr::Raw(RawKind::I64))
            .expect("Tagged -> Raw(I64) is always legal")
    }

    /// Materialize a `Raw(I8)` boolean constant (a default `reverse=False`).
    fn raw_i8_const(&mut self, b: bool) -> LocalId {
        let t = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::Const {
            dst: t,
            val: Const::Bool(b),
        });
        self.coerce(t, Repr::Tagged, Repr::Raw(RawKind::I8))
            .expect("Tagged -> Raw(I8) is always legal")
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
                // CPython truthiness — `reverse=1` / `reverse=[]` work for free.
                ContainerArg::Bool if repr == Repr::Raw(RawKind::I8) => loc,
                ContainerArg::Bool => self.truthy_i8(loc, repr)?,
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

    fn lower_list_lit(
        &mut self,
        idx: Idx<HirExpr>,
        elems: &[Idx<HirExpr>],
    ) -> Result<(LocalId, Repr)> {
        let list_repr = repr_of(&self.func.exprs[idx].ty);
        let cap = self.raw_i64_const(elems.len() as i64);
        let (list, _) = self.emit_container(
            ContainerOp::ListNew,
            vec![(cap, Repr::Raw(RawKind::I64))],
            Some(list_repr.clone()),
        )?;
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

    fn lower_set_lit(
        &mut self,
        idx: Idx<HirExpr>,
        elems: &[Idx<HirExpr>],
    ) -> Result<(LocalId, Repr)> {
        let set_repr = repr_of(&self.func.exprs[idx].ty);
        let cap = self.raw_i64_const(elems.len() as i64);
        let (set, _) = self.emit_container(
            ContainerOp::SetNew,
            vec![(cap, Repr::Raw(RawKind::I64))],
            Some(set_repr.clone()),
        )?;
        let set = set.expect("SetNew produces a set");
        for e in elems {
            let (el, er) = self.lower_expr(*e)?;
            self.emit_container(
                ContainerOp::SetAdd,
                vec![(set, set_repr.clone()), (el, er)],
                None,
            )?;
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
        let (dict, _) = self.emit_container(
            ContainerOp::DictNew,
            vec![(cap, Repr::Raw(RawKind::I64))],
            Some(dict_repr.clone()),
        )?;
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

    fn lower_tuple_lit(
        &mut self,
        idx: Idx<HirExpr>,
        elems: &[Idx<HirExpr>],
    ) -> Result<(LocalId, Repr)> {
        let tup_repr = repr_of(&self.func.exprs[idx].ty);
        let size = self.raw_i64_const(elems.len() as i64);
        let (tup, _) = self.emit_container(
            ContainerOp::TupleNew,
            vec![(size, Repr::Raw(RawKind::I64))],
            Some(tup_repr.clone()),
        )?;
        let tup = tup.expect("TupleNew produces a tuple");
        for (i, e) in elems.iter().enumerate() {
            let (el, er) = self.lower_expr(*e)?;
            let pos = self.raw_i64_const(i as i64);
            self.emit_container(
                ContainerOp::TupleSet,
                vec![
                    (tup, tup_repr.clone()),
                    (pos, Repr::Raw(RawKind::I64)),
                    (el, er),
                ],
                None,
            )?;
        }
        Ok((tup, tup_repr))
    }

    /// Lower a subscript read `base[index]`, dispatching the runtime getter from
    /// the base's *static type* (which survives even when a nested get lowered the
    /// base into a uniform-tagged slot) and falling back to its representation. The
    /// result is normalized to the tagged baseline.
    fn lower_subscript(
        &mut self,
        base: Idx<HirExpr>,
        index: Idx<HirExpr>,
    ) -> Result<(LocalId, Repr)> {
        // Class `__getitem__` (Phase 5C) — a direct devirtualized call.
        let bt = self.func.exprs[base].ty.clone();
        if let Some(fid) = self.concrete_dunder(&bt, "__getitem__") {
            let (bl, br) = self.lower_expr(base)?;
            let (il, ir) = self.lower_expr(index)?;
            return self.emit_dunder_call(fid, vec![(bl, br), (il, ir)]);
        }
        // Counter subscript (§10): `counter[key]` returns the count, or a boxed
        // `0` for a MISSING key — Counter's defining semantic (no KeyError). The
        // key rides Tagged (`rt_counter_get` reads its raw bits, like dict get).
        if matches!(&bt, SemTy::RuntimeObject(t) if *t == pyaot_core_defs::TypeTagKind::Counter) {
            use pyaot_core_defs::runtime_func_def as rf;
            let (bl, br) = self.lower_expr(base)?;
            let recv = self.coerce(bl, br, Repr::Tagged)?;
            let (il, ir) = self.lower_expr(index)?;
            let key = self.coerce(il, ir, Repr::Tagged)?;
            let dst = self.alloc_temp(Repr::Tagged);
            self.emit(MirInst::CallRuntime {
                dst: Some(dst),
                def: &rf::RT_COUNTER_GET,
                args: vec![Operand::Local(recv), Operand::Local(key)],
            });
            return self.normalize_container_result(dst, Repr::Tagged);
        }
        // defaultdict subscript (§10): `dd[key]` AUTO-CREATES the factory default
        // on a MISSING key (defaultdict's defining divergence — no KeyError), then
        // inserts and returns it. Keyed on the defaultdict base BEFORE the generic
        // dict-read path below (a `defaultdict_of` matches `dict_kv()`, so `sub_kind`
        // would otherwise route it to `rt_dict_get` = KeyError). The key rides
        // Tagged (like dict/Counter get).
        if bt.is_defaultdict() {
            use pyaot_core_defs::runtime_func_def as rf;
            let (bl, br) = self.lower_expr(base)?;
            let recv = self.coerce(bl, br, Repr::Tagged)?;
            let (il, ir) = self.lower_expr(index)?;
            let key = self.coerce(il, ir, Repr::Tagged)?;
            let dst = self.alloc_temp(Repr::Tagged);
            self.emit(MirInst::CallRuntime {
                dst: Some(dst),
                def: &rf::RT_DEFAULT_DICT_GET,
                args: vec![Operand::Local(recv), Operand::Local(key)],
            });
            return self.normalize_container_result(dst, Repr::Tagged);
        }
        // deque subscript (§10): `dq[i]` — O(1) ring-buffer access (negative
        // indices and bounds checks handled inside `rt_deque_get`). The index is a
        // RAW i64 (like list get/set); the result is a tagged element.
        if matches!(&bt, SemTy::RuntimeObject(t) if *t == pyaot_core_defs::TypeTagKind::Deque) {
            use pyaot_core_defs::runtime_func_def as rf;
            let (bl, br) = self.lower_expr(base)?;
            let recv = self.coerce(bl, br, Repr::Tagged)?;
            let (il, ir) = self.lower_expr(index)?;
            let idx = self.coerce_to_i64(il, ir)?;
            let dst = self.alloc_temp(Repr::Tagged);
            self.emit(MirInst::CallRuntime {
                dst: Some(dst),
                def: &rf::RT_DEQUE_GET,
                args: vec![Operand::Local(recv), Operand::Local(idx)],
            });
            return self.normalize_container_result(dst, Repr::Tagged);
        }
        let kind = sub_kind(
            &self.func.exprs[base].ty,
            &repr_of(&self.func.exprs[base].ty),
        );
        let (bl, br) = self.lower_expr(base)?;
        let (il, ir) = self.lower_expr(index)?;
        // A class index with `__index__` (`lst[IndexObj(2)]`): dispatch it to the
        // integer index BEFORE the sequence getter coerces to `Raw(I64)` — an
        // unbox of the instance pointer would be garbage (SEGV). CPython calls
        // `__index__` for SEQUENCE subscripts only; a mapping key is used as-is
        // (rides Tagged), so gate on the sequence kinds.
        let (il, ir) = if matches!(
            kind,
            SubKind::List | SubKind::Tuple | SubKind::Bytes | SubKind::Str
        ) {
            let index_ty = self.func.exprs[index].ty.clone();
            match self.concrete_dunder(&index_ty, "__index__") {
                Some(fid) => self.emit_dunder_call(fid, vec![(il, ir)])?,
                None => (il, ir),
            }
        } else {
            (il, ir)
        };
        let op = match kind {
            SubKind::List => ContainerOp::ListGet,
            SubKind::Dict => ContainerOp::DictGet,
            SubKind::Tuple => ContainerOp::TupleGet,
            SubKind::Bytes => ContainerOp::BytesGet,
            // Str needs the codepoint-aware getter (handles negative indices); the
            // generic `rt_any_getitem` only does byte indexing.
            SubKind::Str => ContainerOp::StrGet,
            // Unknown base: a statically-`str` key means a MAPPING subscript
            // (`json.loads(...)["k"]`) — route to the Tagged-key dict getter, since
            // `rt_any_getitem` takes an i64 INDEX and would coerce the string key to
            // garbage (the json-subscript-returns-`None` bug). An int / unknown key
            // stays on the tag-dispatched sequence getter.
            SubKind::Generic if matches!(self.func.exprs[index].ty, SemTy::Str) => {
                ContainerOp::DictGet
            }
            SubKind::Generic => ContainerOp::AnyGetItem,
        };
        let (dst, ret) = self.emit_container(op, vec![(bl, br), (il, ir)], None)?;
        self.normalize_container_result(dst.expect("subscript produces a value"), ret)
    }

    /// Lower a slice `base[start:end:step]` (Phase 8E) to the runtime slicer
    /// selected by the base's static type, with `i64::MIN`/`i64::MAX`/`1`
    /// defaults for absent bounds (the sentinels the runtime reads as "from the
    /// start / to the end / step 1"). A statically-unknown base routes to the
    /// tag-dispatched `rt_obj_slice`.
    fn lower_slice(
        &mut self,
        slice_idx: Idx<HirExpr>,
        base: Idx<HirExpr>,
        start: Option<Idx<HirExpr>>,
        end: Option<Idx<HirExpr>>,
        step: Option<Idx<HirExpr>>,
    ) -> Result<(LocalId, Repr)> {
        use pyaot_core_defs::runtime_func_def as rf;
        let bt = self.func.exprs[base].ty.clone();
        let stepped = step.is_some();
        let def: &'static pyaot_core_defs::RuntimeFuncDef = if matches!(bt, SemTy::Str) {
            if stepped {
                &rf::RT_STR_SLICE_STEP
            } else {
                &rf::RT_STR_SLICE
            }
        } else if matches!(bt, SemTy::Bytes) {
            if stepped {
                &rf::RT_BYTES_SLICE_STEP
            } else {
                &rf::RT_BYTES_SLICE
            }
        } else if bt.list_elem().is_some() {
            if stepped {
                &rf::RT_LIST_SLICE_STEP
            } else {
                &rf::RT_LIST_SLICE
            }
        } else if bt.tuple_elems().is_some() || bt.tuple_var_elem().is_some() {
            if stepped {
                &rf::RT_TUPLE_SLICE_STEP
            } else {
                &rf::RT_TUPLE_SLICE
            }
        } else if stepped {
            &rf::RT_OBJ_SLICE_STEP
        } else {
            &rf::RT_OBJ_SLICE
        };
        let (bl, br) = self.lower_expr(base)?;
        let base_op = self.coerce(bl, br, Repr::Tagged)?;
        let mut ops = vec![
            Operand::Local(base_op),
            Operand::Local(self.slice_bound(start, i64::MIN)?),
            Operand::Local(self.slice_bound(end, i64::MAX)?),
        ];
        if stepped {
            ops.push(Operand::Local(self.slice_bound(step, 1)?));
        }
        // Any non-`Int` ret spec yields a `Tagged` descriptor result, which
        // `emit_runtime_call` then coerces to the slice expr's own repr (the
        // base kind preserved by `slice_ty`).
        self.emit_runtime_call(slice_idx, def, ops, &pyaot_stdlib_defs::TypeSpec::Any)
    }

    /// Lower a format field — `rt_format(value, spec)` → `str` (§5/§9/§13). Both
    /// the value and the spec are ordinary string-valued exprs passed `Tagged`:
    /// the spec is a `StrLit` for a static spec (`f"{x:.4f}"`) or an f-string
    /// concat for a dynamic one (`f"{x:.{n}f}"`). Any `!s`/`!r`/`!a` conversion
    /// is already baked into `value` by the frontend; an empty spec routes a
    /// class instance to its `__format__` inside `rt_format`.
    fn lower_format_value(
        &mut self,
        fmt_idx: Idx<HirExpr>,
        value: Idx<HirExpr>,
        spec: Idx<HirExpr>,
    ) -> Result<(LocalId, Repr)> {
        let (vl, vr) = self.lower_expr(value)?;
        let value_op = self.coerce(vl, vr, Repr::Tagged)?;
        let (sl, sr) = self.lower_expr(spec)?;
        let spec_op = self.coerce(sl, sr, Repr::Tagged)?;
        self.emit_runtime_call(
            fmt_idx,
            &pyaot_core_defs::runtime_func_def::RT_FORMAT,
            vec![Operand::Local(value_op), Operand::Local(spec_op)],
            &pyaot_stdlib_defs::TypeSpec::Str,
        )
    }

    /// A slice bound → `Raw(I64)`: the provided expr untagged to a machine int,
    /// or the sentinel `default` for an absent bound. The sentinel is emitted
    /// directly into a `Raw(I64)` slot — `i64::MIN`/`i64::MAX` are outside the
    /// fixnum range, so the `raw_i64_const` tag round-trip would corrupt them.
    fn slice_bound(&mut self, bound: Option<Idx<HirExpr>>, default: i64) -> Result<LocalId> {
        match bound {
            Some(e) => {
                let (el, er) = self.lower_expr(e)?;
                self.coerce_to_i64(el, er)
            }
            None => {
                let t = self.alloc_temp(Repr::Raw(RawKind::I64));
                self.emit(MirInst::Const {
                    dst: t,
                    val: Const::Int(default),
                });
                Ok(t)
            }
        }
    }

    /// Lower a frontend-synthesized container op (`in`, the iterator protocol).
    fn lower_container_expr(
        &mut self,
        idx: Idx<HirExpr>,
        op: ContainerOp,
        args: &[Idx<HirExpr>],
    ) -> Result<(LocalId, Repr)> {
        // `x in obj` on a class with `__contains__` (Phase 5C). `Contains` args are
        // `[container, elem]`; route a concrete-class container to a direct call.
        if op == ContainerOp::Contains {
            let ct = self.func.exprs[args[0]].ty.clone();
            if let Some(fid) = self.concrete_dunder(&ct, "__contains__") {
                let (cl, cr) = self.lower_expr(args[0])?;
                let (el, er) = self.lower_expr(args[1])?;
                let (res, rrep) = self.emit_dunder_call(fid, vec![(cl, cr), (el, er)])?;
                let dst = self.truthy_i8(res, rrep)?;
                return Ok((dst, Repr::Raw(RawKind::I8)));
            }
        }
        // `for line in f:` where `f` is a File VARIABLE (Phase 8H): there is no
        // runtime File iterator kind, so materialize the lines list via
        // `rt_file_readlines` first, then iterate that list. (The syntactic
        // `for line in open(...)` form takes this same path now.)
        if op == ContainerOp::Iter
            && args.len() == 1
            && matches!(self.func.exprs[args[0]].ty, SemTy::File { .. })
        {
            let (fl, fr) = self.lower_expr(args[0])?;
            let ft = self.coerce(fl, fr, Repr::Tagged)?;
            use pyaot_core_defs::runtime_func_def as rf;
            let lines = self.alloc_temp(Repr::Heap(HeapShape::List(Box::new(Repr::Tagged))));
            self.emit(MirInst::CallRuntime {
                dst: Some(lines),
                def: &rf::RT_FILE_READLINES,
                args: vec![Operand::Local(ft)],
            });
            let heap =
                (op.result() == ContainerResult::Heap).then(|| repr_of(&self.func.exprs[idx].ty));
            let (dst, ret) = self.emit_container(
                op,
                vec![(lines, Repr::Heap(HeapShape::List(Box::new(Repr::Tagged))))],
                heap,
            )?;
            return self
                .normalize_container_result(dst.expect("container expr produces a value"), ret);
        }
        let mut lowered = Vec::with_capacity(args.len());
        for a in args {
            lowered.push(self.lower_expr(*a)?);
        }
        let heap =
            (op.result() == ContainerResult::Heap).then(|| repr_of(&self.func.exprs[idx].ty));
        let (dst, ret) = self.emit_container(op, lowered, heap)?;
        // A mutating op in expression position (the frontend `sort(key=)`
        // desugar emits `ListSortByKeys` as a `ContainerExpr`) yields `None`.
        let Some(dst) = dst else {
            return self.none_value();
        };
        self.normalize_container_result(dst, ret)
    }

    /// Lower a container method call `recv.method(args)` (Phase 4D), dispatching
    /// the concrete runtime op from the receiver's static type. Args/values are
    /// coerced to `Tagged`; results are normalized from `Tagged`.
    fn lower_container_method_call(
        &mut self,
        call_idx: Idx<HirExpr>,
        recv: Idx<HirExpr>,
        method: ContainerMethod,
        args: &[Idx<HirExpr>],
        kwargs: &[(InternedString, Idx<HirExpr>)],
    ) -> Result<(LocalId, Repr)> {
        use ContainerMethod as M;
        let span = self.func.exprs[recv].span;
        // Keywords reach the container path only for `list.sort` (`reverse=`,
        // a literal `key=None`; a real key was frontend-desugared) — checked
        // in `lower_method_call`. Extract the reverse expression here.
        let mut sort_reverse: Option<Idx<HirExpr>> = None;
        for (kname, kexpr) in kwargs {
            match self.interner.resolve(*kname) {
                "reverse" => sort_reverse = Some(*kexpr),
                "key" if matches!(self.func.exprs[*kexpr].kind, HirExprKind::NoneLit) => {}
                other => {
                    return Err(CompilerError::semantic_error(
                        format!("sort() got an unexpected keyword argument `{other}`"),
                        span,
                    ))
                }
            }
        }
        let recv_ty = self.func.exprs[recv].ty.clone();
        let kind = if recv_ty.list_elem().is_some() {
            MethodRecv::List
        } else if recv_ty.dict_kv().is_some() {
            MethodRecv::Dict
        } else if recv_ty.set_elem().is_some() {
            MethodRecv::Set
        } else if recv_ty.tuple_elems().is_some() || recv_ty.tuple_var_elem().is_some() {
            MethodRecv::Tuple
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
                    let (d, r) =
                        self.emit_container(ContainerOp::ListPop, vec![recv_arg, idx], None)?;
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
                    self.emit_container(
                        ContainerOp::ListExtend,
                        vec![recv_arg, a[0].clone()],
                        None,
                    )?;
                    self.none_value()
                }
                // `list.remove(x)` — mutate in place (ValueError on miss, raised
                // by the runtime), returns None. The op's i8 result is discarded.
                M::Remove if argn == 1 => {
                    self.emit_container(
                        ContainerOp::ListRemove,
                        vec![recv_arg, a[0].clone()],
                        None,
                    )?;
                    self.none_value()
                }
                M::Index if argn == 1 => {
                    self.method_scalar(ContainerOp::ListIndexOf, recv_arg, vec![a[0].clone()])
                }
                M::Count if argn == 1 => {
                    self.method_scalar(ContainerOp::ListCount, recv_arg, vec![a[0].clone()])
                }
                M::Clear if argn == 0 => {
                    self.emit_container(ContainerOp::ListClear, vec![recv_arg], None)?;
                    self.none_value()
                }
                M::Copy if argn == 0 => {
                    self.method_heap(ContainerOp::ListCopy, recv_arg, vec![], heap())
                }
                M::Reverse if argn == 0 => {
                    self.emit_container(ContainerOp::ListReverse, vec![recv_arg], None)?;
                    self.none_value()
                }
                M::Sort if argn == 0 => {
                    let rev = match sort_reverse {
                        Some(e) => self.lower_expr(e)?,
                        None => (self.raw_i8_const(false), Repr::Raw(RawKind::I8)),
                    };
                    self.emit_container(ContainerOp::ListSortMut, vec![recv_arg, rev], None)?;
                    self.none_value()
                }
                _ => Err(bad("unsupported list method / arity")),
            },
            MethodRecv::Dict => match method {
                M::Get if argn == 1 || argn == 2 => {
                    let default = if argn == 2 {
                        a[1].clone()
                    } else {
                        (self.none_temp(), Repr::Tagged)
                    };
                    let (d, r) = self.emit_container(
                        ContainerOp::DictGetDefault,
                        vec![recv_arg, a[0].clone(), default],
                        None,
                    )?;
                    self.normalize_container_result(d.unwrap(), r)
                }
                M::Setdefault if argn == 1 || argn == 2 => {
                    let default = if argn == 2 {
                        a[1].clone()
                    } else {
                        (self.none_temp(), Repr::Tagged)
                    };
                    let (d, r) = self.emit_container(
                        ContainerOp::DictSetdefault,
                        vec![recv_arg, a[0].clone(), default],
                        None,
                    )?;
                    self.normalize_container_result(d.unwrap(), r)
                }
                M::Pop if argn == 1 => {
                    let (d, r) = self.emit_container(
                        ContainerOp::DictPopM,
                        vec![recv_arg, a[0].clone()],
                        None,
                    )?;
                    self.normalize_container_result(d.unwrap(), r)
                }
                M::Keys if argn == 0 => {
                    self.method_heap(ContainerOp::DictKeys, recv_arg, vec![], heap())
                }
                M::Values if argn == 0 => {
                    self.method_heap(ContainerOp::DictValues, recv_arg, vec![], heap())
                }
                M::Items if argn == 0 => {
                    self.method_heap(ContainerOp::DictItems, recv_arg, vec![], heap())
                }
                M::Update if argn == 1 => {
                    self.emit_container(
                        ContainerOp::DictUpdate,
                        vec![recv_arg, a[0].clone()],
                        None,
                    )?;
                    self.none_value()
                }
                M::Clear if argn == 0 => {
                    self.emit_container(ContainerOp::DictClear, vec![recv_arg], None)?;
                    self.none_value()
                }
                M::Copy if argn == 0 => {
                    self.method_heap(ContainerOp::DictCopy, recv_arg, vec![], heap())
                }
                // `popitem()` → a fresh `(key, value)` 2-tuple. The `Value`-
                // category result is `Tagged` (GC-rootable, B5); `normalize`
                // passes it through → `Tagged` = `repr_of(Dyn)`, so `k, v =
                // d.popitem()` unpacks through the gradual seam.
                M::Popitem if argn == 0 => {
                    self.method_scalar(ContainerOp::DictPopitem, recv_arg, vec![])
                }
                // `popitem(last)` (OrderedDict, §10) — `last` truthy → LIFO (end),
                // falsy → FIFO (front). The flag rides a RAW i64 (UntagInt on a
                // tagged bool yields 0/1 since INT_SHIFT == BOOL_SHIFT). Tagged
                // result so `k, v = od.popitem(last)` unpacks through the gradual
                // seam, exactly like the 0-arg form above.
                M::Popitem if argn == 1 => {
                    let recv = self.coerce(recv_arg.0, recv_arg.1.clone(), Repr::Tagged)?;
                    let last = self.coerce_to_i64(a[0].0, a[0].1.clone())?;
                    let dst = self.alloc_temp(Repr::Tagged);
                    self.emit(MirInst::CallRuntime {
                        dst: Some(dst),
                        def: &pyaot_core_defs::runtime_func_def::RT_DICT_POPITEM_ORDERED,
                        args: vec![Operand::Local(recv), Operand::Local(last)],
                    });
                    self.normalize_container_result(dst, Repr::Tagged)
                }
                // `move_to_end(key, last=True)` (OrderedDict, §10) — move an
                // existing key to either end; mutates in place, returns None.
                M::MoveToEnd if argn == 1 || argn == 2 => {
                    let recv = self.coerce(recv_arg.0, recv_arg.1.clone(), Repr::Tagged)?;
                    let key = self.coerce(a[0].0, a[0].1.clone(), Repr::Tagged)?;
                    let last = if argn == 2 {
                        self.coerce_to_i64(a[1].0, a[1].1.clone())?
                    } else {
                        self.raw_i64_const(1)
                    };
                    self.emit(MirInst::CallRuntime {
                        dst: None,
                        def: &pyaot_core_defs::runtime_func_def::RT_DICT_MOVE_TO_END,
                        args: vec![
                            Operand::Local(recv),
                            Operand::Local(key),
                            Operand::Local(last),
                        ],
                    });
                    self.none_value()
                }
                // `d.fromkeys(keys[, value])` — the receiver (already lowered
                // above for its side effects) is discarded; `rt_dict_fromkeys`
                // takes only the keys list and the per-key value. The keys arg
                // is snapshotted to a list (accepts any iterable via the
                // iterator protocol); an absent value defaults to a null pointer
                // the runtime reads as `None`. Emitted as a `CallRuntime`
                // directly (no receiver slot, so not `emit_container`).
                M::Fromkeys if argn == 1 || argn == 2 => {
                    let (keys_list, _) = self.materialize_list_from(a[0].0, a[0].1.clone())?;
                    let value = if argn == 2 {
                        self.coerce(a[1].0, a[1].1.clone(), Repr::Tagged)?
                    } else {
                        let n = self.alloc_temp(Repr::Tagged);
                        self.emit(MirInst::Const {
                            dst: n,
                            val: Const::NullPtr,
                        });
                        n
                    };
                    let dst_repr = heap();
                    let dst = self.alloc_temp(dst_repr.clone());
                    self.emit(MirInst::CallRuntime {
                        dst: Some(dst),
                        def: &pyaot_core_defs::runtime_func_def::RT_DICT_FROM_KEYS,
                        args: vec![Operand::Local(keys_list), Operand::Local(value)],
                    });
                    Ok((dst, dst_repr))
                }
                _ => Err(bad("unsupported dict method / arity")),
            },
            MethodRecv::Set => match method {
                M::Add if argn == 1 => {
                    self.emit_container(ContainerOp::SetAdd, vec![recv_arg, a[0].clone()], None)?;
                    self.none_value()
                }
                M::Remove if argn == 1 => {
                    self.emit_container(
                        ContainerOp::SetRemove,
                        vec![recv_arg, a[0].clone()],
                        None,
                    )?;
                    self.none_value()
                }
                M::Discard if argn == 1 => {
                    self.emit_container(
                        ContainerOp::SetDiscard,
                        vec![recv_arg, a[0].clone()],
                        None,
                    )?;
                    self.none_value()
                }
                M::Update if argn == 1 => {
                    self.emit_container(
                        ContainerOp::SetUpdate,
                        vec![recv_arg, a[0].clone()],
                        None,
                    )?;
                    self.none_value()
                }
                M::Union if argn == 1 => {
                    self.method_heap(ContainerOp::SetUnion, recv_arg, vec![a[0].clone()], heap())
                }
                M::Intersection if argn == 1 => self.method_heap(
                    ContainerOp::SetIntersection,
                    recv_arg,
                    vec![a[0].clone()],
                    heap(),
                ),
                M::Difference if argn == 1 => self.method_heap(
                    ContainerOp::SetDifference,
                    recv_arg,
                    vec![a[0].clone()],
                    heap(),
                ),
                // `set.symmetric_difference(other)` → a fresh set (new-set
                // algebra; the `*_update` sibling mutates in place instead).
                M::SymmetricDifference if argn == 1 => self.method_heap(
                    ContainerOp::SetSymmetricDifference,
                    recv_arg,
                    vec![a[0].clone()],
                    heap(),
                ),
                M::Copy if argn == 0 => {
                    self.method_heap(ContainerOp::SetCopy, recv_arg, vec![], heap())
                }
                M::Clear if argn == 0 => {
                    self.emit_container(ContainerOp::SetClear, vec![recv_arg], None)?;
                    self.none_value()
                }
                // Comparisons (§9). A `Bool`-category op: `method_scalar`'s
                // `emit_container` allocates a `Raw(I8)` dst and `normalize`
                // passes it through → `Raw(I8)` = `repr_of(Bool)` (B13:
                // value-comparing `rt_set_*`, not pointer ordering).
                M::IsSubset if argn == 1 => {
                    self.method_scalar(ContainerOp::SetIsSubset, recv_arg, vec![a[0].clone()])
                }
                M::IsSuperset if argn == 1 => {
                    self.method_scalar(ContainerOp::SetIsSuperset, recv_arg, vec![a[0].clone()])
                }
                M::IsDisjoint if argn == 1 => {
                    self.method_scalar(ContainerOp::SetIsDisjoint, recv_arg, vec![a[0].clone()])
                }
                // In-place updates (§9): mutate the receiver, return `None`.
                M::IntersectionUpdate if argn == 1 => {
                    self.emit_container(
                        ContainerOp::SetIntersectionUpdate,
                        vec![recv_arg, a[0].clone()],
                        None,
                    )?;
                    self.none_value()
                }
                M::DifferenceUpdate if argn == 1 => {
                    self.emit_container(
                        ContainerOp::SetDifferenceUpdate,
                        vec![recv_arg, a[0].clone()],
                        None,
                    )?;
                    self.none_value()
                }
                M::SymmetricDifferenceUpdate if argn == 1 => {
                    self.emit_container(
                        ContainerOp::SetSymmetricDifferenceUpdate,
                        vec![recv_arg, a[0].clone()],
                        None,
                    )?;
                    self.none_value()
                }
                _ => Err(bad("unsupported set method / arity")),
            },
            // Tuple receiver (§9): `index`/`count` — value-comparing queries
            // (B13). `method_scalar` normalizes the `Raw(I64)` result to the
            // tagged int baseline.
            MethodRecv::Tuple => match method {
                M::Index if argn == 1 => {
                    self.method_scalar(ContainerOp::TupleIndexOf, recv_arg, vec![a[0].clone()])
                }
                M::Count if argn == 1 => {
                    self.method_scalar(ContainerOp::TupleCount, recv_arg, vec![a[0].clone()])
                }
                _ => Err(bad("unsupported tuple method / arity")),
            },
            _ => Err(bad(
                "method calls require a statically-known list, dict, or set receiver",
            )),
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
        self.emit(MirInst::Const {
            dst: t,
            val: Const::None,
        });
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
    ///
    /// `idx` is this `BinOp` node, carrying typeck's per-expr `raw_int_ok`
    /// certificate (the interval proof). It gates the raw `Mul`/`Mod`/`FloorDiv`
    /// path: those ops can leave the proven fixnum range (`*`) or carry
    /// floor/sign semantics (`% //`), so unlike raw `Add`/`Sub` they fire only
    /// when the result is provably in-bound *and* the operand-closure invariant
    /// holds (each operand lowers to `Raw(I64)` or is a small fixnum literal).
    fn lower_binop(
        &mut self,
        idx: Idx<HirExpr>,
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

        // Raw int fast path (Phase 3c). `Add`/`Sub` fire whenever an operand is a
        // range-proven `Raw(I64)` (a bounded cursor / flagged sub-expr) — the
        // result of `a±b` with `|a|,|b| ≤ 2^48` cannot overflow i64. `Mul`/`Mod`/
        // `FloorDiv` additionally require this node's `raw_int_ok` certificate
        // (typeck proved the result stays in `±2^48` and, for `% //`, the divisor
        // is statically positive), so a possibly-overflowing or possibly-zero
        // divisor case stays tagged and the runtime handles it correctly. In
        // every case the other operand is supplied as `Raw(I64)` too (a flagged
        // sub-expr already lowered to `Raw(I64)`, or a fixnum literal small enough
        // to untag soundly).
        let i64r = Repr::Raw(RawKind::I64);
        let raw_addsub = matches!(mop, MBinOp::Add | MBinOp::Sub) && (lr == i64r || rr == i64r);
        let proven = self.func.exprs[idx].raw_int_ok && self.func.exprs[idx].ty == SemTy::Int;
        let raw_muldivmod = proven && matches!(mop, MBinOp::Mul | MBinOp::Mod | MBinOp::FloorDiv);
        if raw_addsub || raw_muldivmod {
            if let (Some(la), Some(ra)) = (
                self.raw_i64_operand(l, ll, &lr)?,
                self.raw_i64_operand(r, rl, &rr)?,
            ) {
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
        use HeapShape::{Bytes, Dict, List, Set, Tuple, TupleVar};
        match op {
            MBinOp::Add => {
                let cop = match (lr, rr) {
                    (Repr::Heap(List(_)), Repr::Heap(List(_))) => ContainerOp::ListConcat,
                    (Repr::Heap(Tuple(_) | TupleVar(_)), Repr::Heap(Tuple(_) | TupleVar(_))) => {
                        ContainerOp::TupleConcat
                    }
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
            // Set algebra operators (`|` `&` `-` `^`) and dict merge (`|`, PEP
            // 584). Fire ONLY when both operands are the same statically-known
            // container; any other repr combo (a numeric pair, a gradual `Dyn`
            // operand, …) returns `Ok(None)` so the numeric / tagged baseline is
            // unchanged — numeric `|`/`&`/`-`/`^` and bignum paths are untouched.
            MBinOp::BitOr => match (lr, rr) {
                (Repr::Heap(Set(_)), Repr::Heap(Set(_))) => {
                    self.emit_container_binop(ContainerOp::SetUnion, ll, lr, rl, rr)
                }
                (Repr::Heap(Dict(..)), Repr::Heap(Dict(..))) => {
                    self.emit_container_binop(ContainerOp::DictMerge, ll, lr, rl, rr)
                }
                _ => Ok(None),
            },
            MBinOp::BitAnd => match (lr, rr) {
                (Repr::Heap(Set(_)), Repr::Heap(Set(_))) => {
                    self.emit_container_binop(ContainerOp::SetIntersection, ll, lr, rl, rr)
                }
                _ => Ok(None),
            },
            MBinOp::Sub => match (lr, rr) {
                (Repr::Heap(Set(_)), Repr::Heap(Set(_))) => {
                    self.emit_container_binop(ContainerOp::SetDifference, ll, lr, rl, rr)
                }
                _ => Ok(None),
            },
            MBinOp::BitXor => match (lr, rr) {
                (Repr::Heap(Set(_)), Repr::Heap(Set(_))) => {
                    self.emit_container_binop(ContainerOp::SetSymmetricDifference, ll, lr, rl, rr)
                }
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }

    /// Emit a binary container op (set algebra / dict merge) whose result is a
    /// fresh container of the left operand's representation. Both operands are
    /// tagged (`Val`); the new container rides `lr` (left = right family here).
    fn emit_container_binop(
        &mut self,
        cop: ContainerOp,
        ll: LocalId,
        lr: &Repr,
        rl: LocalId,
        rr: &Repr,
    ) -> Result<Option<(LocalId, Repr)>> {
        let (dst, ret) = self.emit_container(
            cop,
            vec![(ll, lr.clone()), (rl, rr.clone())],
            Some(lr.clone()),
        )?;
        Ok(Some((
            dst.expect("container binop produces a container"),
            ret,
        )))
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
        self.emit(MirInst::Unary {
            dst,
            op: mop,
            operand: Operand::Local(ot),
        });
        Ok((dst, dst_repr))
    }

    /// Materialize the `NotImplemented` singleton (Tagged) for a pointer-identity
    /// NI check (§4b).
    fn ni_singleton(&mut self) -> LocalId {
        let dst = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::CallRuntime {
            dst: Some(dst),
            def: &pyaot_core_defs::runtime_func_def::RT_NOT_IMPLEMENTED_SINGLETON,
            args: vec![],
        });
        dst
    }

    /// `a is b` (bit-identity) → `Raw(I8)` via `rt_is`. Both operands Tagged.
    fn rt_is_i8(&mut self, a: LocalId, b: LocalId) -> LocalId {
        let dst = self.alloc_temp(Repr::Raw(RawKind::I8));
        self.emit(MirInst::CallRuntime {
            dst: Some(dst),
            def: &pyaot_core_defs::runtime_func_def::RT_IS,
            args: vec![Operand::Local(a), Operand::Local(b)],
        });
        dst
    }

    /// §4b: the rich-comparison `NotImplemented` protocol for user-class
    /// operands, lowered INLINE (each dunder is called with its native compiled
    /// ABI — the runtime `DunderFn (ptr,Value)->Value` seam cannot carry a
    /// pure-`bool` raw-i8 dunder). Returns `Some(result)` when it emits the
    /// forward→NI→reflected→NI→default diamond; `None` to defer to the existing
    /// fast path / tagged baseline. The protocol fires only when the forward
    /// dunder returns `Tagged` (a `Union[Bool, NotImplementedT]` — a possible NI),
    /// or is absent while the reflected dunder on the right operand is present
    /// (e.g. `a > b` with only `__lt__` defined). A pure-`Bool` (`Raw(I8)`)
    /// forward dunder keeps the existing devirtualized fast path.
    fn try_compare_protocol(
        &mut self,
        op: HCmpOp,
        l: Idx<HirExpr>,
        r: Idx<HirExpr>,
    ) -> Result<Option<(LocalId, Repr)>> {
        let lt = self.func.exprs[l].ty.clone();
        let rt = self.func.exprs[r].ty.clone();
        // The op's forward dunder + whether the final bool is negated (`!=`
        // derived from `__eq__`). A class that defines `__ne__` directly uses it.
        let (fwd_name, negate): (&str, bool) = match op {
            HCmpOp::Eq => ("__eq__", false),
            HCmpOp::NotEq => {
                if self.concrete_dunder(&lt, "__ne__").is_some() {
                    ("__ne__", false)
                } else {
                    ("__eq__", true)
                }
            }
            HCmpOp::Lt => ("__lt__", false),
            HCmpOp::LtE => ("__le__", false),
            HCmpOp::Gt => ("__gt__", false),
            HCmpOp::GtE => ("__ge__", false),
        };
        let rev_name = pyaot_types::dunders::reflected_name(fwd_name);
        let fwd_fid = self.concrete_dunder(&lt, fwd_name);
        let rev_fid = rev_name.and_then(|n| self.concrete_dunder(&rt, n));

        // Gate: a forward dunder that returns a pure `Bool` (`Raw(I8)`) never
        // yields NI → keep the existing devirtualized fast path. Enter the
        // protocol only for a Tagged-returning forward, or an absent forward
        // with a present reflected dunder.
        let fwd_is_tagged = fwd_fid.is_some_and(|f| self.sigs[f.index()].ret == Repr::Tagged);
        let needs_protocol = fwd_is_tagged || (fwd_fid.is_none() && rev_fid.is_some());
        if !needs_protocol {
            return Ok(None);
        }

        // `==`/`!=` fall back to identity; ordering raises `TypeError`.
        let is_eq = matches!(op, HCmpOp::Eq | HCmpOp::NotEq);

        // Evaluate both operands once, on the Tagged baseline.
        let (ll, lr) = self.lower_expr(l)?;
        let left = self.coerce(ll, lr, Repr::Tagged)?;
        let (rl, rr) = self.lower_expr(r)?;
        let right = self.coerce(rl, rr, Repr::Tagged)?;

        // The merged bool result, written on each non-diverging arm.
        let result = self.alloc_temp(Repr::Raw(RawKind::I8));
        let reflected_bb = self.reserve_block();
        let default_bb = self.reserve_block();
        let merge_bb = self.reserve_block();

        // ── forward: call `left.fwd(right)`; on NI fall to the reflected arm ──
        if let Some(ffid) = fwd_fid {
            self.emit_compare_arm(ffid, left, right, result, reflected_bb, merge_bb)?;
        } else {
            self.seal(MirTerminator::Jump(reflected_bb));
        }

        // ── reflected: call `right.rev(left)` (operands SWAPPED); NI → default ──
        self.switch(reflected_bb);
        if let Some(rfid) = rev_fid {
            self.emit_compare_arm(rfid, right, left, result, default_bb, merge_bb)?;
        } else {
            self.seal(MirTerminator::Jump(default_bb));
        }

        // ── default: `==`/`!=` → identity; ordering → TypeError ──
        self.switch(default_bb);
        if is_eq {
            let id = self.rt_is_i8(left, right);
            self.coerce_into(result, id, Repr::Raw(RawKind::I8), Repr::Raw(RawKind::I8))?;
            self.seal(MirTerminator::Jump(merge_bb));
        } else {
            self.emit(MirInst::Raise(MirRaise::Builtin {
                tag: pyaot_core_defs::BuiltinExceptionKind::TypeError.tag(),
                msg: None,
            }));
            self.seal(MirTerminator::Unreachable);
        }

        // ── merge: the result bool, negated for a derived `!=` ──
        self.switch(merge_bb);
        if negate {
            let tagged = self.coerce(result, Repr::Raw(RawKind::I8), Repr::Tagged)?;
            let dst = self.alloc_temp(Repr::Raw(RawKind::I8));
            self.emit(MirInst::Unary {
                dst,
                op: MUnaryOp::Not,
                operand: Operand::Local(tagged),
            });
            return Ok(Some((dst, Repr::Raw(RawKind::I8))));
        }
        Ok(Some((result, Repr::Raw(RawKind::I8))))
    }

    /// One arm of the §4b comparison diamond: call dunder `fid(recv, arg)`; if it
    /// returns the `NotImplemented` singleton branch to `ni_bb`, else write the
    /// truthy result into `result` and jump to `merge_bb`. A `Raw(I8)` (pure
    /// `Bool`) dunder never yields NI, so it writes directly. Seals the current
    /// block; leaves the builder on a fresh block (the caller `switch`es next).
    fn emit_compare_arm(
        &mut self,
        fid: FuncId,
        recv: LocalId,
        arg: LocalId,
        result: LocalId,
        ni_bb: BlockId,
        merge_bb: BlockId,
    ) -> Result<()> {
        let (res, res_repr) =
            self.emit_dunder_call(fid, vec![(recv, Repr::Tagged), (arg, Repr::Tagged)])?;
        if res_repr == Repr::Tagged {
            // The dunder may return NI — pointer-identity check against the
            // singleton; on NI fall to the next arm.
            let res_t = self.coerce(res, res_repr, Repr::Tagged)?;
            let ni = self.ni_singleton();
            let is_ni = self.rt_is_i8(res_t, ni);
            let not_ni_bb = self.reserve_block();
            self.seal(MirTerminator::Branch {
                cond: Operand::Local(is_ni),
                then: ni_bb,
                else_: not_ni_bb,
            });
            self.switch(not_ni_bb);
            let b = self.truthy_i8(res_t, Repr::Tagged)?;
            self.coerce_into(result, b, Repr::Raw(RawKind::I8), Repr::Raw(RawKind::I8))?;
            self.seal(MirTerminator::Jump(merge_bb));
        } else {
            // A pure-`Bool` (`Raw(I8)`) dunder: no NI possible, use directly.
            let b = self.truthy_i8(res, res_repr)?;
            self.coerce_into(result, b, Repr::Raw(RawKind::I8), Repr::Raw(RawKind::I8))?;
            self.seal(MirTerminator::Jump(merge_bb));
        }
        Ok(())
    }

    fn lower_compare(
        &mut self,
        op: HCmpOp,
        l: Idx<HirExpr>,
        r: Idx<HirExpr>,
    ) -> Result<(LocalId, Repr)> {
        // §4b: the rich-comparison `NotImplemented` protocol (forward→reflected→
        // identity/TypeError) for user-class operands whose dunder may return NI.
        if let Some(res) = self.try_compare_protocol(op, l, r)? {
            return Ok(res);
        }
        // Class comparison dunders the runtime does NOT dispatch: route a
        // concrete-class left operand to a direct call (5C). `rt_obj_eq` falls to
        // identity, and `rt_obj_cmp` raises on instances, so this is mandatory.
        let lt = self.func.exprs[l].ty.clone();
        if let Some((fid, negate)) = self.eq_dunder(&lt, op) {
            let (ll, lr) = self.lower_expr(l)?;
            let (rl, rr) = self.lower_expr(r)?;
            let (res, rrep) = self.emit_dunder_call(fid, vec![(ll, lr), (rl, rr)])?;
            if negate {
                // `!=` derived from `__eq__`: logical-negate the (tagged) result.
                let tagged = self.coerce(res, rrep, Repr::Tagged)?;
                let dst = self.alloc_temp(Repr::Raw(RawKind::I8));
                self.emit(MirInst::Unary {
                    dst,
                    op: MUnaryOp::Not,
                    operand: Operand::Local(tagged),
                });
                return Ok((dst, Repr::Raw(RawKind::I8)));
            }
            let dst = self.truthy_i8(res, rrep)?;
            return Ok((dst, Repr::Raw(RawKind::I8)));
        }
        let ord_name = match op {
            HCmpOp::Lt => Some("__lt__"),
            HCmpOp::LtE => Some("__le__"),
            HCmpOp::Gt => Some("__gt__"),
            HCmpOp::GtE => Some("__ge__"),
            _ => None,
        };
        if let Some(name) = ord_name {
            if let Some(fid) = self.concrete_dunder(&lt, name) {
                let (ll, lr) = self.lower_expr(l)?;
                let (rl, rr) = self.lower_expr(r)?;
                let (res, rrep) = self.emit_dunder_call(fid, vec![(ll, lr), (rl, rr)])?;
                let dst = self.truthy_i8(res, rrep)?;
                return Ok((dst, Repr::Raw(RawKind::I8)));
            }
        }

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
            (
                self.coerce(ll, lr, Repr::Tagged)?,
                self.coerce(rl, rr, Repr::Tagged)?,
            )
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
            HirExprKind::Name(SymbolRef::Resolved(id)) => match self.resolve.symbol(*id) {
                // A local holding a callable VALUE → indirect call (Phase 6A).
                Symbol::Local(_) => return self.lower_indirect_call(callee, &args),
                sym => sym,
            },
            HirExprKind::Name(SymbolRef::Unresolved(_)) => {
                return Err(CompilerError::semantic_error(
                    "internal: callee name reached lowering unresolved",
                    span,
                ))
            }
            // Any other callee expression (a closure read, a call result, …) is
            // an indirect call through its Callable type (Phase 6A).
            _ => return self.lower_indirect_call(callee, &args),
        };
        match sym {
            Symbol::Builtin(kind) => {
                use pyaot_mir::BuiltinFunctionKind as BK;
                // Zero-arg type-conversion builtins yield their default constant
                // (CPython: `int() == 0`, `float() == 0.0`, `bool() == False`).
                // The unary `rt_builtin_*` take one argument, so calling them with
                // none would build an arity-mismatched (invalid) Cranelift call —
                // fold the literal here instead. (`list()`/`dict()`/… empty forms
                // ride the separate `Symbol::Container` path.)
                if args.is_empty() {
                    match kind {
                        BK::Int => {
                            let dst = self.alloc_temp(Repr::Tagged);
                            self.emit(MirInst::Const {
                                dst,
                                val: Const::Int(0),
                            });
                            return Ok((dst, Repr::Tagged));
                        }
                        BK::Bool => {
                            let dst = self.alloc_temp(Repr::Tagged);
                            self.emit(MirInst::Const {
                                dst,
                                val: Const::Bool(false),
                            });
                            return Ok((dst, Repr::Tagged));
                        }
                        BK::Float => {
                            let dst = self.alloc_temp(Repr::Raw(RawKind::F64));
                            self.emit(MirInst::Const {
                                dst,
                                val: Const::Float(0.0),
                            });
                            return Ok((dst, Repr::Raw(RawKind::F64)));
                        }
                        // `str()` with no args is desugared to a `""` literal in
                        // the frontend (interning lives there). Everything else
                        // here (`abs()`/`ord()`/… — a `TypeError` with no args)
                        // gets a clean error, never an arity-mismatched call.
                        _ => {
                            return Err(CompilerError::semantic_error(
                                format!("`{kind:?}()` requires at least one argument"),
                                span,
                            ))
                        }
                    }
                }
                // `int(s, base)` — the two-arg form parses string `s` in the
                // given radix via `rt_str_to_int_with_base` (the base is a RAW
                // i64). The generic unary `rt_builtin_int` path ignores a second
                // argument (parsing in base 10), so intercept here.
                if kind == BK::Int && args.len() == 2 {
                    let (sl, sr) = self.lower_expr(args[0])?;
                    let s_tagged = self.coerce(sl, sr, Repr::Tagged)?;
                    let (bl, br) = self.lower_expr(args[1])?;
                    let base_raw = self.coerce_to_i64(bl, br)?;
                    let dst = self.alloc_temp(Repr::Raw(RawKind::I64));
                    self.emit(MirInst::CallRuntime {
                        dst: Some(dst),
                        def: &pyaot_core_defs::runtime_func_def::RT_STR_TO_INT_WITH_BASE,
                        args: vec![Operand::Local(s_tagged), Operand::Local(base_raw)],
                    });
                    return self.normalize_container_result(dst, Repr::Raw(RawKind::I64));
                }
                // `str(x)` / `repr(x)` of a concrete class instance route to the
                // user dunder (CPython precedence: `str` → `__str__` then
                // `__repr__`; `repr` → `__repr__`), mirroring the print path (5C).
                // The generic `rt_builtin_str` / `rt_builtin_repr` render the
                // *default* object repr for instances, so this is the only path
                // that honours a user `__str__`/`__repr__` (Phase 8E).
                if args.len() == 1 && matches!(kind, BK::Str | BK::Repr) {
                    let arg_ty = self.func.exprs[args[0]].ty.clone();
                    let dunder = if kind == BK::Str {
                        self.concrete_dunder(&arg_ty, "__str__")
                            .or_else(|| self.concrete_dunder(&arg_ty, "__repr__"))
                    } else {
                        self.concrete_dunder(&arg_ty, "__repr__")
                    };
                    if let Some(fid) = dunder {
                        let (al, ar) = self.lower_expr(args[0])?;
                        let (res, rrep) = self.emit_dunder_call(fid, vec![(al, ar)])?;
                        let tagged = self.coerce(res, rrep, Repr::Tagged)?;
                        return Ok((tagged, Repr::Tagged));
                    }
                }
                // `str(e)` of a caught exception returns its message (Phase 7B);
                // the generic `rt_builtin_str` would render the object repr.
                if kind == pyaot_mir::BuiltinFunctionKind::Str && args.len() == 1 {
                    let arg_ty = self.func.exprs[args[0]].ty.clone();
                    if self.is_exception_value(&arg_ty) {
                        let (vl, vr) = self.lower_expr(args[0])?;
                        let vt = self.coerce(vl, vr, Repr::Tagged)?;
                        let dst = self.alloc_temp(Repr::Heap(HeapShape::Str));
                        self.emit(MirInst::ExcInstanceStr {
                            dst,
                            value: Operand::Local(vt),
                        });
                        return Ok((dst, Repr::Heap(HeapShape::Str)));
                    }
                }
                let mut argvals = Vec::with_capacity(args.len());
                for a in &args {
                    let (al, ar) = self.lower_expr(*a)?;
                    let at = self.coerce(al, ar, Repr::Tagged)?;
                    argvals.push(Operand::Local(at));
                }
                let dst = self.alloc_temp(Repr::Tagged);
                self.emit(MirInst::CallBuiltin {
                    dst: Some(dst),
                    kind,
                    args: argvals,
                });
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
                    // A free-fn arg into a `float`/`bool` param takes the CHECKED
                    // coercion for an int/bool/gradual value (numeric tower, §8 —
                    // `coerce_value` → `rt_unbox_float`/`rt_unbox_bool`); a
                    // statically-matching value stays a plain unchecked coerce.
                    let aty = self.func.exprs[*a].ty.clone();
                    let (al, ar) = self.lower_expr(*a)?;
                    let at = self.coerce_value(al, ar, &aty, prepr)?;
                    argvals.push(Operand::Local(at));
                }
                let dst = self.alloc_temp(ret.clone());
                self.emit(MirInst::Call {
                    dst: Some(dst),
                    func: fid,
                    args: argvals,
                });
                Ok((dst, ret))
            }
            Symbol::Container(op) => self.lower_container_builtin(call_idx, op, &args),
            Symbol::Class(cid) => self.lower_construct(cid, &args, span),
            Symbol::BuiltinRange => self.lower_range_value(call_idx, &args, span),
            // `Symbol::Local` was intercepted above (indirect call).
            Symbol::BuiltinPrint | Symbol::Local(_) => Err(CompilerError::semantic_error(
                "this callee is not usable as a value-returning call here",
                span,
            )),
        }
    }

    /// Expand `sum(iterable[, start])` (Phase 8H, D2) into a Tagged-accumulator
    /// iterator loop:
    /// ```text
    ///   acc = start (Tagged; default tagged 0, or boxed 0.0 when typeck
    ///                solved the node Float — keeps the final unbox legal)
    ///   it  = Iter(iterable)
    /// header:
    ///   elem = IterNext(it); done = IterExhausted(it)
    ///   if done -> exit else -> body
    /// body:
    ///   acc = acc + elem    (Tagged BinOp — runtime dunder dispatch)
    ///   -> header
    /// exit:
    ///   result = coerce(acc, Tagged -> repr_of(node.ty))
    /// ```
    /// Documented divergences: (1) `sum([])` over Float-typed elements yields
    /// `0.0` where CPython yields `0` (the boxed-Float seed); (2) float sums
    /// are a naive left fold, while CPython >= 3.12 uses Neumaier compensated
    /// summation — results can differ in the last ULP for non-binary-exact
    /// fractions.
    fn lower_sum_expr(
        &mut self,
        idx: Idx<HirExpr>,
        iterable: Idx<HirExpr>,
        start: Option<Idx<HirExpr>>,
    ) -> Result<(LocalId, Repr)> {
        let node_ty = self.func.exprs[idx].ty.clone();
        let result_repr = repr_of(&node_ty);

        // Accumulator seed.
        let acc = self.alloc_temp(Repr::Tagged);
        match start {
            Some(s) => {
                let (sl, sr) = self.lower_expr(s)?;
                self.coerce_into(acc, sl, sr, Repr::Tagged)?;
            }
            None if node_ty == SemTy::Float => {
                let raw = self.alloc_temp(Repr::Raw(RawKind::F64));
                self.emit(MirInst::Const {
                    dst: raw,
                    val: Const::Float(0.0),
                });
                self.coerce_into(acc, raw, Repr::Raw(RawKind::F64), Repr::Tagged)?;
            }
            None => {
                let raw = self.alloc_temp(Repr::Raw(RawKind::I64));
                self.emit(MirInst::Const {
                    dst: raw,
                    val: Const::Int(0),
                });
                self.coerce_into(acc, raw, Repr::Raw(RawKind::I64), Repr::Tagged)?;
            }
        }

        // it = Iter(iterable)
        let iter_repr = Repr::Heap(HeapShape::Iterator(Box::new(Repr::Tagged)));
        let (il, ir) = self.lower_expr(iterable)?;
        let (it, _) =
            self.emit_container(ContainerOp::Iter, vec![(il, ir)], Some(iter_repr.clone()))?;
        let it = it.expect("Iter produces a value");

        let header = self.reserve_block();
        let body = self.reserve_block();
        let exit = self.reserve_block();
        self.seal(MirTerminator::Jump(header));

        // header: elem = next(it); done = is_exhausted(it) (the runtime call
        // order contract: next advances and sets the exhausted flag).
        self.switch(header);
        let (elem, _) =
            self.emit_container(ContainerOp::IterNext, vec![(it, iter_repr.clone())], None)?;
        let elem = elem.expect("IterNext produces a value");
        let (done, _) =
            self.emit_container(ContainerOp::IterExhausted, vec![(it, iter_repr)], None)?;
        let done = done.expect("IterExhausted produces a value");
        self.seal(MirTerminator::Branch {
            cond: Operand::Local(done),
            then: exit,
            else_: body,
        });

        // body: acc = acc + elem (both Tagged — runtime dispatch covers
        // int/float/bignum and user dunders alike).
        self.switch(body);
        let tmp = self.alloc_temp(Repr::Tagged);
        self.emit(MirInst::BinOp {
            dst: tmp,
            op: MBinOp::Add,
            l: Operand::Local(acc),
            r: Operand::Local(elem),
        });
        self.coerce_into(acc, tmp, Repr::Tagged, Repr::Tagged)?;
        self.seal(MirTerminator::Jump(header));

        // exit: one reinterpreting coercion to the solved repr.
        self.switch(exit);
        let res = self.coerce(acc, Repr::Tagged, result_repr.clone())?;
        Ok((res, result_repr))
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
                // `len(obj)` on a class with `__len__` (Phase 5C) — `rt_obj_len`
                // does not dispatch instances, so route to a direct call.
                let at = self.func.exprs[args[0]].ty.clone();
                if let Some(fid) = self.concrete_dunder(&at, "__len__") {
                    let (l, r) = self.lower_expr(args[0])?;
                    return self.emit_dunder_call(fid, vec![(l, r)]);
                }
                let (l, r) = self.lower_expr(args[0])?;
                let (dst, ret) = self.emit_container(C::Len, vec![(l, r)], None)?;
                self.normalize_container_result(dst.unwrap(), ret)
            }
            C::Enumerate => {
                let it = self.lower_iter_arg(args[0])?;
                let start = match args.get(1) {
                    Some(a) => {
                        // `start` (positional or `start=`) must be int-like —
                        // it seeds a `Raw(I64)` counter; a float/str would untag
                        // into garbage. CPython raises TypeError; reject at
                        // compile time (gradual `Dyn` / in-progress `Never` pass).
                        let st = &self.func.exprs[*a].ty;
                        if !matches!(st, SemTy::Int | SemTy::Bool | SemTy::Dyn | SemTy::Never) {
                            return Err(CompilerError::semantic_error(
                                format!("enumerate() start must be an integer, not {st:?}"),
                                span,
                            ));
                        }
                        self.lower_expr(*a)?
                    }
                    None => (self.raw_i64_const(0), Repr::Raw(RawKind::I64)),
                };
                let (dst, ret) =
                    self.emit_container(C::Enumerate, vec![it, start], Some(result_heap))?;
                Ok((dst.unwrap(), ret))
            }
            C::Zip => match args.len() {
                // The dedicated 2-iterable path: `rt_zip_new(iter1, iter2)`.
                2 => {
                    let a = self.lower_iter_arg(args[0])?;
                    let b = self.lower_iter_arg(args[1])?;
                    let (dst, ret) = self.emit_container(C::Zip, vec![a, b], Some(result_heap))?;
                    Ok((dst.unwrap(), ret))
                }
                // N≥3 iterables: collect each `iter()`-wrapped source into a fresh
                // runtime list (GC-rooted as a local), then `rt_zipn_new(list,
                // count)`. The result is an iterator of N-tuples consumed through
                // the normal iterator protocol (the object's kind dispatches
                // `rt_iter_next` to `iter_next_zipn`).
                n if n >= 3 => {
                    let list_repr = Repr::Heap(HeapShape::List(Box::new(Repr::Tagged)));
                    let cap = self.raw_i64_const(n as i64);
                    let (list, _) = self.emit_container(
                        ContainerOp::ListNew,
                        vec![(cap, Repr::Raw(RawKind::I64))],
                        Some(list_repr.clone()),
                    )?;
                    let list = list.expect("ListNew produces a list");
                    for &arg in args {
                        let it = self.lower_iter_arg(arg)?;
                        self.emit_container(
                            ContainerOp::ListPush,
                            vec![(list, list_repr.clone()), it],
                            None,
                        )?;
                    }
                    let count = self.raw_i64_const(n as i64);
                    let (dst, ret) = self.emit_container(
                        ContainerOp::ZipN,
                        vec![(list, list_repr), (count, Repr::Raw(RawKind::I64))],
                        Some(result_heap),
                    )?;
                    Ok((dst.unwrap(), ret))
                }
                _ => Err(CompilerError::semantic_error(
                    "zip() requires at least two iterables",
                    span,
                )),
            },
            C::Sorted => {
                // Evaluate the iterable, then the (optional) reverse flag —
                // written order — BEFORE materializing the copy.
                let (il, ir) = self.lower_expr(args[0])?;
                let rev = match args.get(1) {
                    Some(a) => self.lower_expr(*a)?,
                    None => (self.raw_i8_const(false), Repr::Raw(RawKind::I8)),
                };
                let list = self.materialize_list_from(il, ir)?;
                let (dst, ret) =
                    self.emit_container(C::Sorted, vec![list, rev], Some(result_heap))?;
                Ok((dst.unwrap(), ret))
            }
            C::Reversed => {
                let list = self.materialize_list(args[0])?;
                let (dst, ret) = self.emit_container(C::Reversed, vec![list], Some(result_heap))?;
                Ok((dst.unwrap(), ret))
            }
            C::ListFromIter => self.lower_constructor(call_idx, args, C::ListNew, C::ListFromIter),
            C::TupleFromIter => {
                self.lower_constructor(call_idx, args, C::TupleNew, C::TupleFromIter)
            }
            C::DictFromPairs => {
                if args.is_empty() {
                    return self.empty_container(C::DictNew, result_heap);
                }
                let (pl, pr) = self.lower_expr(args[0])?;
                // `dict(d)` on a known dict is a copy, not an iteration of
                // key/value PAIRS (`rt_dict_from_pairs` expects a pair list).
                let op = if self.func.exprs[args[0]].ty.dict_kv().is_some() {
                    C::DictCopy
                } else {
                    C::DictFromPairs
                };
                let (dst, ret) = self.emit_container(op, vec![(pl, pr)], Some(result_heap))?;
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
                // Dispatch the one-or-two-arg `bytes(...)` by the first arg's
                // static type (CPython's overloaded constructor): an int/bool is
                // a zero-fill count, a str is UTF-8 encoded, anything else is an
                // iterable of ints. Each routes to its own runtime maker.
                let arg0_ty = self.func.exprs[args[0]].ty.clone();
                if matches!(arg0_ty, SemTy::Int | SemTy::Bool) {
                    // `bytes(n)` → `n` zero bytes (emit_container unboxes the
                    // count to `Raw(I64)`).
                    let (nl, nr) = self.lower_expr(args[0])?;
                    let (dst, ret) =
                        self.emit_container(C::BytesZero, vec![(nl, nr)], Some(result_heap))?;
                    return Ok((dst.unwrap(), ret));
                }
                if matches!(arg0_ty, SemTy::Str) {
                    // `bytes(s[, encoding])` → encode `s` (UTF-8 only). The
                    // optional encoding arg is evaluated for side effects but
                    // otherwise ignored (only UTF-8 is supported).
                    let (sl, sr) = self.lower_expr(args[0])?;
                    if let Some(enc) = args.get(1) {
                        self.lower_expr(*enc)?;
                    }
                    let (dst, ret) =
                        self.emit_container(C::BytesFromStr, vec![(sl, sr)], Some(result_heap))?;
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
        let (dst, _) =
            self.emit_container(ContainerOp::Iter, vec![(l, r)], Some(iter_repr.clone()))?;
        Ok((dst.unwrap(), iter_repr))
    }

    /// Materialize an argument into a fresh list: an existing list is used as-is
    /// (`sorted`/`reversed` do not mutate their input); anything else is built from
    /// its iterator via `rt_list_from_iter`.
    fn materialize_list(&mut self, arg: Idx<HirExpr>) -> Result<(LocalId, Repr)> {
        let (l, r) = self.lower_expr(arg)?;
        self.materialize_list_from(l, r)
    }

    /// The materialize half of [`Self::materialize_list`], for callers that must
    /// evaluate other arguments between lowering the iterable and copying it.
    fn materialize_list_from(&mut self, l: LocalId, r: Repr) -> Result<(LocalId, Repr)> {
        if matches!(r, Repr::Heap(HeapShape::List(_))) {
            return Ok((l, r));
        }
        let list_repr = Repr::Heap(HeapShape::List(Box::new(Repr::Tagged)));
        let iter_repr = Repr::Heap(HeapShape::Iterator(Box::new(Repr::Tagged)));
        let (it, _) = self.emit_container(ContainerOp::Iter, vec![(l, r)], Some(iter_repr))?;
        let (dst, _) = self.emit_container(
            ContainerOp::ListFromIter,
            vec![(
                it.unwrap(),
                Repr::Heap(HeapShape::Iterator(Box::new(Repr::Tagged))),
            )],
            Some(list_repr.clone()),
        )?;
        Ok((dst.unwrap(), list_repr))
    }

    fn lower_name(
        &mut self,
        symref: SymbolRef,
        span: pyaot_utils::Span,
    ) -> Result<(LocalId, Repr)> {
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
            | Symbol::Container(_)
            | Symbol::Class(_) => Err(CompilerError::semantic_error(
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
/// Map a builtin-type `isinstance` target SemTy to its runtime kind code
/// ([`pyaot_core_defs::isinstance_kind`]) for the gradual-receiver path.
/// Mirrors `isinstance_builtin_target` in the frontend; matches by KIND
/// (container element types ignored). `None` for a non-canonical target.
fn builtin_isinstance_kind(target: &SemTy) -> Option<i64> {
    use pyaot_core_defs::isinstance_kind as k;
    if target.list_elem().is_some() {
        return Some(k::LIST);
    }
    if target.dict_kv().is_some() {
        return Some(k::DICT);
    }
    if target.set_elem().is_some() {
        return Some(k::SET);
    }
    if target.tuple_elems().is_some() || target.tuple_var_elem().is_some() {
        return Some(k::TUPLE);
    }
    match target {
        SemTy::Str => Some(k::STR),
        SemTy::Int => Some(k::INT),
        SemTy::Float => Some(k::FLOAT),
        SemTy::Bool => Some(k::BOOL),
        SemTy::Bytes => Some(k::BYTES),
        _ => None,
    }
}

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
        HBinOp::MatMul => MBinOp::MatMul,
        HBinOp::Div => MBinOp::Div,
        HBinOp::FloorDiv => MBinOp::FloorDiv,
        HBinOp::Mod => MBinOp::Mod,
        HBinOp::Pow => MBinOp::Pow,
        HBinOp::BitAnd => MBinOp::BitAnd,
        HBinOp::BitOr => MBinOp::BitOr,
        HBinOp::IOr => MBinOp::IOr,
        HBinOp::BitXor => MBinOp::BitXor,
        HBinOp::IAnd => MBinOp::IAnd,
        HBinOp::ISub => MBinOp::ISub,
        HBinOp::IXor => MBinOp::IXor,
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
    Tuple,
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
        || matches!(
            repr,
            Repr::Heap(HeapShape::Tuple(_) | HeapShape::TupleVar(_))
        )
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

/// True iff `r` is a user class-instance pointer representation.
fn is_class_repr(r: &Repr) -> bool {
    matches!(r, Repr::Heap(HeapShape::Class(_)))
}

/// True iff `name` is a dunder (`__name__`) — a registry-dispatched special method.
fn is_dunder(name: &str) -> bool {
    name.len() > 4 && name.starts_with("__") && name.ends_with("__")
}

/// True iff `ty` is a builtin exception or a tuple-clause `Union` of only
/// builtin exceptions — the receivers whose instances carry `.args` at field
/// slot 0 (Phase 7B).
fn is_builtin_exception_ty(ty: &SemTy) -> bool {
    match ty {
        SemTy::BuiltinException(_) => true,
        SemTy::Union(members) => {
            !members.is_empty()
                && members
                    .iter()
                    .all(|m| matches!(m, SemTy::BuiltinException(_)))
        }
        _ => false,
    }
}

/// Convert a class-attribute initializer to a MIR [`Const`], interning literal
/// bytes (str / bytes / bignum decimal) into the string pool (Phase 5D).
fn class_attr_const(
    init: &pyaot_hir::ClassAttrInit,
    interner: &StringInterner,
    str_pool: &mut StrPool,
) -> Const {
    use pyaot_hir::ClassAttrInit as A;
    match init {
        A::Int(v) => Const::Int(*v),
        A::Float(v) => Const::Float(*v),
        A::Bool(b) => Const::Bool(*b),
        A::None => Const::None,
        A::Str(s) => {
            str_pool.insert(*s, interner.resolve(*s).as_bytes().to_vec());
            Const::Str(*s)
        }
        A::Bytes(s) => {
            str_pool.insert(*s, interner.resolve(*s).as_bytes().to_vec());
            Const::Bytes(*s)
        }
        A::BigInt(s) => {
            str_pool.insert(*s, interner.resolve(*s).as_bytes().to_vec());
            Const::BigIntStr(*s)
        }
        // `EmptyTuple` is only ever produced as a parameter default (materialized
        // as an HIR `TupleLit` at the call site, never reaching here). A tuple as
        // a class-level attribute is out of scope.
        A::EmptyTuple => unreachable!("empty-tuple class attribute should be rejected in frontend"),
    }
}

/// The class id a receiver type denotes (a nominal `Class`, or a user generic
/// instance whose base is a user class), else `None`.
fn class_of(ty: &SemTy, classes: &ClassTable) -> Option<ClassId> {
    match ty {
        SemTy::Class { class_id, .. } => Some(*class_id),
        SemTy::Generic { base, .. } if classes.get(*base).is_some() => Some(*base),
        _ => None,
    }
}

/// True for the repeatable sequence representations (`list` / `tuple` / `bytes`)
/// — the `*`-repeat operands.
/// The repr a `CallRuntime` arg slot must carry (Phase 8B), from the
/// descriptor's Cranelift register class disambiguated by the declarative
/// `TypeSpec`: an `I64` register holds a raw integer for `Int` params and a
/// tagged `Value` for everything else (strings, containers, objects, `Any`).
/// `F64`/`I8`/`I32` registers are always raw.
fn runtime_param_repr(
    pt: pyaot_core_defs::runtime_func_def::ParamType,
    spec: Option<&pyaot_stdlib_defs::TypeSpec>,
) -> Repr {
    use pyaot_core_defs::runtime_func_def::ParamType;
    use pyaot_stdlib_defs::TypeSpec;
    match pt {
        ParamType::F64 => Repr::Raw(RawKind::F64),
        ParamType::I8 => Repr::Raw(RawKind::I8),
        ParamType::I32 => Repr::Raw(RawKind::I32),
        // `None` spec = a descriptor-internal immediate (field index, arg
        // count) — always raw.
        ParamType::I64 => match spec {
            Some(TypeSpec::Int) | None => Repr::Raw(RawKind::I64),
            Some(_) => Repr::Tagged,
        },
    }
}

/// The repr a `CallRuntime` result lands in (Phase 8B), mirroring
/// [`runtime_param_repr`]: an `I64` return is a raw integer for `Int` specs and
/// a tagged `Value` otherwise.
fn runtime_return_repr(
    def: &pyaot_core_defs::RuntimeFuncDef,
    spec: &pyaot_stdlib_defs::TypeSpec,
) -> Repr {
    use pyaot_core_defs::runtime_func_def::ReturnType;
    use pyaot_stdlib_defs::TypeSpec;
    match def.returns {
        Some(ReturnType::F64) => Repr::Raw(RawKind::F64),
        Some(ReturnType::I8) => Repr::Raw(RawKind::I8),
        Some(ReturnType::I32) => Repr::Raw(RawKind::I32),
        Some(ReturnType::I64) => match spec {
            TypeSpec::Int => Repr::Raw(RawKind::I64),
            _ => Repr::Tagged,
        },
        None => Repr::Tagged,
    }
}

fn is_sequence_repr(r: &Repr) -> bool {
    matches!(
        r,
        Repr::Heap(
            HeapShape::List(_) | HeapShape::Tuple(_) | HeapShape::TupleVar(_) | HeapShape::Bytes
        )
    )
}

#[cfg(test)]
mod tests;
