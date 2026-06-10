//! MIR verifier — checks `Repr` consistency at every pass boundary.
//!
//! `Repr` is the sole representation type, so there are no widening exceptions to
//! bridge a second (logical) type field. The verifier runs in debug builds at
//! every pass boundary (see `pyaot-optimizer`); it is the executable form of the
//! invariant that representation never silently drifts.
//!
//! Each instruction has a typed `Repr` signature (e.g. arithmetic operands are
//! `Tagged`, `Branch.cond` is `Raw(I8)`); the verifier rejects any violation.
//! `Call` reprs are checked against the callee signature (ABI = f(param Repr)).

use crate::{
    classify_coercion, is_heap_str, BinOp, Const, ContainerArg, ContainerOp, ContainerResult,
    GenOp, GenResult, MirFunction, MirInst, MirTerminator, Operand, PrintKind, UnaryOp,
};
use pyaot_types::{HeapShape, RawKind, Repr};
use pyaot_utils::{BlockId, LocalId};

/// A representation-consistency violation found by [`verify`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    EmptyFunction,
    BadEntry { entry: usize, count: usize },
    LocalOutOfRange { local: usize, count: usize },
    BlockOutOfRange { block: usize, count: usize },
    FuncOutOfRange { func: usize, count: usize },
    /// An instruction operand/result repr disagrees with its typed signature.
    ReprMismatch { ctx: &'static str, expected: Repr, actual: Repr },
    /// `Call` arity disagrees with the callee signature.
    CallArity { func: usize, expected: usize, actual: usize },
    /// `(from, to)` is not an accepted coercion.
    IllegalCoercion { from: Repr, to: Repr },
    PrintUnexpectedArg { kind: PrintKind },
    PrintMissingArg { kind: PrintKind },
    /// `Branch.cond` is not `Raw(I8)`.
    BranchCondNotI8 { got: Repr },
    /// A `BinOp` runs on a representation that does not support it (e.g. a raw
    /// `Add`/`Sub`/`Mul` is fine on `Raw(F64)`/`Raw(I64)`, but `Div`/`//`/`%`/`**`
    /// and bitwise/shift must stay on the tagged baseline).
    BadBinOpRepr { op: BinOp, repr: Repr },
    /// A `CallContainer` arity disagrees with the op's argument signature.
    ContainerArity { op: ContainerOp, expected: usize, actual: usize },
    /// A `CallContainer` carries a `dst` for a mutating op, or omits it for a
    /// value-producing op.
    ContainerDst { op: ContainerOp, want_dst: bool },
    /// A `CallContainer` result local has the wrong representation for the op.
    ContainerResultRepr { op: ContainerOp, actual: Repr },
    /// An instance instruction's `base` is neither `Heap(Class(_))` nor `Tagged`
    /// (PITFALLS B12: only such a value may be handed to `rt_instance_*`).
    InstanceBaseRepr { ctx: &'static str, actual: Repr },
    /// A `MakeInstance` `dst` is not `Heap(Class(class_id))`.
    MakeInstanceDst { class_id: u32, actual: Repr },
    /// A `MakeClosure` whose `dst` signature does not equal the target
    /// function's signature minus its env param (or whose target lacks the
    /// `Tagged` env param 0) — Phase 6A.
    ClosureSigMismatch { func: usize },
    /// A `CallIndirect` whose callee repr is not `Closure(sig)` for the carried
    /// `sig` — Phase 6A.
    IndirectCalleeRepr { actual: Repr },
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::EmptyFunction => write!(f, "MIR function has no blocks"),
            VerifyError::BadEntry { entry, count } => {
                write!(f, "entry block {entry} out of range ({count} blocks)")
            }
            VerifyError::LocalOutOfRange { local, count } => {
                write!(f, "local {local} out of range ({count} locals)")
            }
            VerifyError::BlockOutOfRange { block, count } => {
                write!(f, "block {block} out of range ({count} blocks)")
            }
            VerifyError::FuncOutOfRange { func, count } => {
                write!(f, "func {func} out of range ({count} funcs)")
            }
            VerifyError::ReprMismatch { ctx, expected, actual } => {
                write!(f, "{ctx}: expected {expected:?}, got {actual:?}")
            }
            VerifyError::CallArity { func, expected, actual } => {
                write!(f, "call to func {func}: expected {expected} args, got {actual}")
            }
            VerifyError::IllegalCoercion { from, to } => {
                write!(f, "illegal coercion {from:?} -> {to:?} (not in the legality table)")
            }
            VerifyError::PrintUnexpectedArg { kind } => {
                write!(f, "Print({kind:?}) takes no argument but one was supplied")
            }
            VerifyError::PrintMissingArg { kind } => {
                write!(f, "Print({kind:?}) requires an argument but none was supplied")
            }
            VerifyError::BranchCondNotI8 { got } => {
                write!(f, "Branch.cond must be Raw(I8), got {got:?}")
            }
            VerifyError::BadBinOpRepr { op, repr } => {
                write!(f, "BinOp {op:?} is not legal on representation {repr:?}")
            }
            VerifyError::ContainerArity { op, expected, actual } => {
                write!(f, "CallContainer {op:?}: expected {expected} args, got {actual}")
            }
            VerifyError::ContainerDst { op, want_dst } => {
                if *want_dst {
                    write!(f, "CallContainer {op:?} requires a dst but none was supplied")
                } else {
                    write!(f, "CallContainer {op:?} is a mutating op and must not have a dst")
                }
            }
            VerifyError::ContainerResultRepr { op, actual } => {
                write!(f, "CallContainer {op:?}: dst has the wrong representation {actual:?}")
            }
            VerifyError::InstanceBaseRepr { ctx, actual } => {
                write!(f, "{ctx}: instance base must be Heap(Class) or Tagged, got {actual:?}")
            }
            VerifyError::MakeInstanceDst { class_id, actual } => {
                write!(f, "MakeInstance dst must be Heap(Class({class_id})), got {actual:?}")
            }
            VerifyError::ClosureSigMismatch { func } => {
                write!(
                    f,
                    "MakeClosure over func {func}: dst Closure signature must equal the \
                     target's signature minus its Tagged env param 0"
                )
            }
            VerifyError::IndirectCalleeRepr { actual } => {
                write!(f, "CallIndirect callee must be Closure(sig) matching the carried sig, got {actual:?}")
            }
        }
    }
}

impl std::error::Error for VerifyError {}

const TAGGED: Repr = Repr::Tagged;
const RAW_I8: Repr = Repr::Raw(RawKind::I8);
const RAW_I64: Repr = Repr::Raw(RawKind::I64);
const RAW_F64: Repr = Repr::Raw(RawKind::F64);

/// A short static context string for a generator op's `dst` repr check.
fn gen_op_ctx(op: GenOp) -> &'static str {
    match op {
        GenOp::GetLocal => "GenOp(GetLocal).dst",
        GenOp::GetState => "GenOp(GetState).dst",
        GenOp::GetSentValue => "GenOp(GetSentValue).dst",
        GenOp::IsClosing => "GenOp(IsClosing).dst",
        _ => "GenOp.dst",
    }
}

fn check_local(f: &MirFunction, id: LocalId) -> Result<(), VerifyError> {
    if id.index() >= f.locals.len() {
        Err(VerifyError::LocalOutOfRange { local: id.index(), count: f.locals.len() })
    } else {
        Ok(())
    }
}

fn check_operand(f: &MirFunction, op: &Operand) -> Result<(), VerifyError> {
    match op {
        Operand::Local(id) => check_local(f, *id),
    }
}

fn check_block(f: &MirFunction, id: BlockId) -> Result<(), VerifyError> {
    if id.index() >= f.blocks.len() {
        Err(VerifyError::BlockOutOfRange { block: id.index(), count: f.blocks.len() })
    } else {
        Ok(())
    }
}

/// Require an operand's repr to equal `w`.
fn want(f: &MirFunction, op: &Operand, w: &Repr, ctx: &'static str) -> Result<(), VerifyError> {
    check_operand(f, op)?;
    let got = f.operand_repr(op);
    if got != w {
        return Err(VerifyError::ReprMismatch { ctx, expected: w.clone(), actual: got.clone() });
    }
    Ok(())
}

/// Require an operand to be a valid instance base: `Heap(Class(_))` or `Tagged`
/// (PITFALLS B12 — only such a value may be dereferenced by `rt_instance_*`).
fn want_instance_base(f: &MirFunction, op: &Operand, ctx: &'static str) -> Result<(), VerifyError> {
    let got = f.operand_repr(op);
    match got {
        Repr::Tagged | Repr::Heap(HeapShape::Class(_)) => Ok(()),
        other => Err(VerifyError::InstanceBaseRepr { ctx, actual: other.clone() }),
    }
}

/// Require a destination local's declared repr to equal `w`.
fn want_local(f: &MirFunction, id: LocalId, w: &Repr, ctx: &'static str) -> Result<(), VerifyError> {
    check_local(f, id)?;
    let got = f.local_repr(id);
    if got != w {
        return Err(VerifyError::ReprMismatch { ctx, expected: w.clone(), actual: got.clone() });
    }
    Ok(())
}

/// Verify a single MIR function's representation consistency. `funcs` is the
/// whole program's function table, used to check `Call` signatures.
pub fn verify(f: &MirFunction, funcs: &[MirFunction]) -> Result<(), VerifyError> {
    if f.blocks.is_empty() {
        return Err(VerifyError::EmptyFunction);
    }
    let nblocks = f.blocks.len();
    if f.entry.index() >= nblocks {
        return Err(VerifyError::BadEntry { entry: f.entry.index(), count: nblocks });
    }

    for block in &f.blocks {
        for inst in &block.insts {
            verify_inst(f, funcs, inst)?;
        }
        match &block.term {
            MirTerminator::Return(Some(op)) => check_operand(f, op)?,
            MirTerminator::Return(None) => {}
            MirTerminator::Jump(target) => check_block(f, *target)?,
            MirTerminator::Branch { cond, then, else_ } => {
                check_operand(f, cond)?;
                let got = f.operand_repr(cond);
                if *got != RAW_I8 {
                    return Err(VerifyError::BranchCondNotI8 { got: got.clone() });
                }
                check_block(f, *then)?;
                check_block(f, *else_)?;
            }
            MirTerminator::Unreachable => {}
        }
    }

    Ok(())
}

fn verify_inst(f: &MirFunction, funcs: &[MirFunction], inst: &MirInst) -> Result<(), VerifyError> {
    match inst {
        MirInst::Const { dst, val } => {
            check_local(f, *dst)?;
            let repr = f.local_repr(*dst);
            let (ok, expected) = match val {
                Const::Str(_) => (is_heap_str(repr), Repr::Heap(HeapShape::Str)),
                Const::Bytes(_) => (
                    matches!(repr, Repr::Heap(HeapShape::Bytes)),
                    Repr::Heap(HeapShape::Bytes),
                ),
                Const::BigIntStr(_) => (
                    matches!(repr, Repr::Heap(HeapShape::BigInt)),
                    Repr::Heap(HeapShape::BigInt),
                ),
                Const::Int(_) | Const::Bool(_) | Const::None => (*repr == TAGGED, TAGGED),
                Const::Float(_) => (*repr == RAW_F64, RAW_F64),
            };
            if !ok {
                return Err(VerifyError::ReprMismatch { ctx: "Const dst", expected, actual: repr.clone() });
            }
        }
        MirInst::Coerce { dst, src, from, to } => {
            check_local(f, *dst)?;
            check_operand(f, src)?;
            let src_repr = f.operand_repr(src);
            if src_repr != from {
                return Err(VerifyError::ReprMismatch {
                    ctx: "Coerce.from",
                    expected: from.clone(),
                    actual: src_repr.clone(),
                });
            }
            let dst_repr = f.local_repr(*dst);
            if dst_repr != to {
                return Err(VerifyError::ReprMismatch {
                    ctx: "Coerce.to",
                    expected: to.clone(),
                    actual: dst_repr.clone(),
                });
            }
            if classify_coercion(from, to).is_none() {
                return Err(VerifyError::IllegalCoercion { from: from.clone(), to: to.clone() });
            }
        }
        MirInst::BinOp { dst, op, l, r } => {
            // Repr-consistent: operands and dst share one representation `R`, and
            // `R` must support `op`. `Tagged` handles every op via tag dispatch
            // (`rt_obj_*`, bignum-safe); the unboxed `Raw(F64)` / `Raw(I64)` fast
            // paths carry only `Add`/`Sub`/`Mul`, which lowering proves safe (both
            // operands statically float, or a no-overflow range proof for int).
            check_operand(f, l)?;
            let lhs = f.operand_repr(l).clone();
            want(f, r, &lhs, "BinOp.r")?;
            want_local(f, *dst, &lhs, "BinOp.dst")?;
            let raw_ok = matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul);
            match lhs {
                Repr::Tagged => {}
                Repr::Raw(RawKind::F64) | Repr::Raw(RawKind::I64) if raw_ok => {}
                other => return Err(VerifyError::BadBinOpRepr { op: *op, repr: other }),
            }
        }
        MirInst::Unary { dst, op, operand } => {
            want(f, operand, &TAGGED, "Unary.operand")?;
            let dst_want = if *op == UnaryOp::Not { &RAW_I8 } else { &TAGGED };
            want_local(f, *dst, dst_want, "Unary.dst")?;
        }
        MirInst::Compare { dst, l, r, .. } => {
            // Operands share one repr `R`; the result is the `Raw(I8)` boolean.
            // `R = Tagged` dispatches via `rt_obj_*`; `R = Raw(I64)` is a machine
            // `icmp` on range-proven cursors. Float comparison is not specialized.
            check_operand(f, l)?;
            let lhs = f.operand_repr(l).clone();
            match lhs {
                Repr::Tagged | Repr::Raw(RawKind::I64) => {}
                other => {
                    return Err(VerifyError::ReprMismatch {
                        ctx: "Compare operand repr (Tagged or Raw(I64))",
                        expected: TAGGED,
                        actual: other,
                    })
                }
            }
            want(f, r, &lhs, "Compare.r")?;
            want_local(f, *dst, &RAW_I8, "Compare.dst")?;
        }
        MirInst::Truthy { dst, operand } => {
            want(f, operand, &TAGGED, "Truthy.operand")?;
            want_local(f, *dst, &RAW_I8, "Truthy.dst")?;
        }
        MirInst::Call { dst, func, args } => {
            if func.index() >= funcs.len() {
                return Err(VerifyError::FuncOutOfRange { func: func.index(), count: funcs.len() });
            }
            let callee = &funcs[func.index()];
            if args.len() != callee.params.len() {
                return Err(VerifyError::CallArity {
                    func: func.index(),
                    expected: callee.params.len(),
                    actual: args.len(),
                });
            }
            for (arg, prepr) in args.iter().zip(&callee.params) {
                want(f, arg, prepr, "Call.arg")?;
            }
            if let Some(d) = dst {
                want_local(f, *d, &callee.ret, "Call.dst")?;
            }
        }
        MirInst::CallBuiltin { dst, args, .. } => {
            for arg in args {
                want(f, arg, &TAGGED, "CallBuiltin.arg")?;
            }
            if let Some(d) = dst {
                want_local(f, *d, &TAGGED, "CallBuiltin.dst")?;
            }
        }
        MirInst::CallContainer { dst, op, args } => verify_call_container(f, *op, dst, args)?,
        MirInst::MakeInstance { dst, class_id, .. } => {
            check_local(f, *dst)?;
            let got = f.local_repr(*dst);
            if *got != Repr::Heap(HeapShape::Class(*class_id)) {
                return Err(VerifyError::MakeInstanceDst {
                    class_id: class_id.0,
                    actual: got.clone(),
                });
            }
        }
        MirInst::GetField { dst, base, slot: _ } => {
            check_operand(f, base)?;
            want_instance_base(f, base, "GetField.base")?;
            want_local(f, *dst, &TAGGED, "GetField.dst")?;
        }
        MirInst::SetField { base, slot: _, value } => {
            check_operand(f, base)?;
            want_instance_base(f, base, "SetField.base")?;
            want(f, value, &TAGGED, "SetField.value")?;
        }
        MirInst::CallVirtual { dst, recv, args, ret, .. } => {
            check_operand(f, recv)?;
            want_instance_base(f, recv, "CallVirtual.recv")?;
            for arg in args {
                check_operand(f, arg)?;
            }
            // `dst` (if present) must carry the resolved method's return repr.
            if let Some(d) = dst {
                want_local(f, *d, ret, "CallVirtual.dst")?;
            }
        }
        MirInst::IsInstance { dst, value, .. } => {
            want(f, value, &TAGGED, "IsInstance.value")?;
            want_local(f, *dst, &RAW_I8, "IsInstance.dst")?;
        }
        MirInst::GetClassAttr { dst, .. } => {
            want_local(f, *dst, &TAGGED, "GetClassAttr.dst")?;
        }
        MirInst::SetClassAttr { value, .. } => {
            want(f, value, &TAGGED, "SetClassAttr.value")?;
        }
        // ── closures / cells / globals (Phase 6) ──
        MirInst::MakeClosure { dst, func, captures } => {
            if func.index() >= funcs.len() {
                return Err(VerifyError::FuncOutOfRange { func: func.index(), count: funcs.len() });
            }
            let callee = &funcs[func.index()];
            // The target must carry the Tagged env as explicit param 0, and the
            // dst Closure signature must be exactly the rest of its signature.
            check_local(f, *dst)?;
            let dst_repr = f.local_repr(*dst);
            let Repr::Closure(sig) = dst_repr else {
                return Err(VerifyError::ClosureSigMismatch { func: func.index() });
            };
            let env_ok = callee.params.first() == Some(&TAGGED);
            let sig_ok = sig.params[..] == callee.params[1..] && *sig.ret == callee.ret;
            if !env_ok || !sig_ok {
                return Err(VerifyError::ClosureSigMismatch { func: func.index() });
            }
            // Every capture is a tagged cell pointer (the P6-2 cell rule).
            for c in captures {
                want(f, c, &TAGGED, "MakeClosure.capture")?;
            }
        }
        MirInst::CallIndirect { dst, callee, args, sig } => {
            check_operand(f, callee)?;
            let callee_repr = f.operand_repr(callee);
            match callee_repr {
                Repr::Closure(s) if **s == *sig => {}
                other => return Err(VerifyError::IndirectCalleeRepr { actual: other.clone() }),
            }
            if args.len() != sig.params.len() {
                return Err(VerifyError::CallArity {
                    func: usize::MAX,
                    expected: sig.params.len(),
                    actual: args.len(),
                });
            }
            for (arg, prepr) in args.iter().zip(&sig.params) {
                want(f, arg, prepr, "CallIndirect.arg")?;
            }
            if let Some(d) = dst {
                want_local(f, *d, &sig.ret, "CallIndirect.dst")?;
            }
        }
        MirInst::MakeCell { dst, init } => {
            want(f, init, &TAGGED, "MakeCell.init")?;
            want_local(f, *dst, &TAGGED, "MakeCell.dst")?;
        }
        MirInst::CellGet { dst, cell } => {
            want(f, cell, &TAGGED, "CellGet.cell")?;
            want_local(f, *dst, &TAGGED, "CellGet.dst")?;
        }
        MirInst::CellSet { cell, value } => {
            want(f, cell, &TAGGED, "CellSet.cell")?;
            want(f, value, &TAGGED, "CellSet.value")?;
        }
        MirInst::GlobalGet { dst, .. } => {
            want_local(f, *dst, &TAGGED, "GlobalGet.dst")?;
        }
        MirInst::GlobalSet { value, .. } => {
            want(f, value, &TAGGED, "GlobalSet.value")?;
        }
        // ── generators (Phase 6E) ──
        MirInst::MakeGenerator { dst, .. } => {
            want_local(f, *dst, &TAGGED, "MakeGenerator.dst")?;
        }
        MirInst::GenOpInst { dst, op, gen, value, .. } => {
            // The generator operand and any stored value are Tagged (P6-3).
            want(f, gen, &TAGGED, "GenOp.gen")?;
            match (op.takes_value(), value) {
                (true, Some(v)) => want(f, v, &TAGGED, "GenOp.value")?,
                (true, None) => {
                    return Err(VerifyError::ReprMismatch {
                        ctx: "GenOp(SetLocal) requires a value",
                        expected: TAGGED,
                        actual: Repr::Never,
                    })
                }
                (false, Some(_)) => {
                    return Err(VerifyError::ReprMismatch {
                        ctx: "GenOp without a value got one",
                        expected: Repr::Never,
                        actual: TAGGED,
                    })
                }
                (false, None) => {}
            }
            match (op.result(), dst) {
                (GenResult::None, Some(_)) => {
                    return Err(VerifyError::ReprMismatch {
                        ctx: "GenOp mutating op must have no dst",
                        expected: Repr::Never,
                        actual: TAGGED,
                    })
                }
                (GenResult::None, None) => {}
                (res, Some(d)) => {
                    let want_repr = match res {
                        GenResult::Value => TAGGED,
                        GenResult::Int => RAW_I64,
                        GenResult::Bool => RAW_I8,
                        GenResult::None => unreachable!(),
                    };
                    want_local(f, *d, &want_repr, gen_op_ctx(*op))?;
                }
                (_, None) => {
                    return Err(VerifyError::ReprMismatch {
                        ctx: "GenOp value-producing op requires a dst",
                        expected: TAGGED,
                        actual: Repr::Never,
                    })
                }
            }
        }
        MirInst::AssertFail => {}
        MirInst::Print { kind, arg } => match kind {
            PrintKind::Newline | PrintKind::Sep | PrintKind::None_ => {
                if arg.is_some() {
                    return Err(VerifyError::PrintUnexpectedArg { kind: *kind });
                }
            }
            PrintKind::StrObj | PrintKind::Obj => {
                let op = arg.as_ref().ok_or(VerifyError::PrintMissingArg { kind: *kind })?;
                want(f, op, &TAGGED, "Print(Obj/StrObj).arg")?;
            }
            PrintKind::Float => {
                let op = arg.as_ref().ok_or(VerifyError::PrintMissingArg { kind: *kind })?;
                want(f, op, &RAW_F64, "Print(Float).arg")?;
            }
            PrintKind::Bool => {
                let op = arg.as_ref().ok_or(VerifyError::PrintMissingArg { kind: *kind })?;
                want(f, op, &RAW_I8, "Print(Bool).arg")?;
            }
            PrintKind::Int => {
                let op = arg.as_ref().ok_or(VerifyError::PrintMissingArg { kind: *kind })?;
                want(f, op, &RAW_I64, "Print(Int).arg")?;
            }
        },
    }
    Ok(())
}

/// Verify a `CallContainer` against its op's argument/result signature. This is
/// the structural form of PITFALLS A5: every `Val` argument position is required
/// to be `Tagged`, so a non-tagged element can never reach container storage; only
/// `Idx` positions (indices, counts, sizes) are `Raw(I64)`.
fn verify_call_container(
    f: &MirFunction,
    op: ContainerOp,
    dst: &Option<LocalId>,
    args: &[Operand],
) -> Result<(), VerifyError> {
    let kinds = op.arg_kinds();
    if args.len() != kinds.len() {
        return Err(VerifyError::ContainerArity {
            op,
            expected: kinds.len(),
            actual: args.len(),
        });
    }
    for (arg, kind) in args.iter().zip(kinds) {
        let want_repr = match kind {
            ContainerArg::Val => TAGGED,
            ContainerArg::Idx => RAW_I64,
        };
        want(f, arg, &want_repr, "CallContainer.arg")?;
    }
    match op.result() {
        ContainerResult::None => {
            if dst.is_some() {
                return Err(VerifyError::ContainerDst { op, want_dst: false });
            }
        }
        result => {
            let d = dst.ok_or(VerifyError::ContainerDst { op, want_dst: true })?;
            check_local(f, d)?;
            let got = f.local_repr(d);
            let ok = match result {
                ContainerResult::Value => *got == TAGGED,
                ContainerResult::Int => *got == RAW_I64,
                ContainerResult::Bool => *got == RAW_I8,
                ContainerResult::Heap => matches!(got, Repr::Heap(_)),
                ContainerResult::None => unreachable!(),
            };
            if !ok {
                return Err(VerifyError::ContainerResultRepr { op, actual: got.clone() });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BinOp, CmpOp, Const, LocalDecl, MirBlock, MirFunction, Operand};
    use pyaot_types::HeapShape;
    use pyaot_utils::{BlockId, InternedString, StringInterner};

    fn interned(s: &str) -> InternedString {
        StringInterner::new().intern(s)
    }

    fn single_block(locals: Vec<Repr>, insts: Vec<MirInst>, term: MirTerminator) -> MirFunction {
        MirFunction {
            name: interned("__main__"),
            params: Vec::new(),
            ret: Repr::Tagged,
            locals: locals.into_iter().map(|repr| LocalDecl { repr }).collect(),
            blocks: vec![MirBlock { insts, term }],
            entry: BlockId::new(0),
        }
    }

    fn well_formed_print() -> MirFunction {
        single_block(
            vec![Repr::Heap(HeapShape::Str), Repr::Tagged],
            vec![
                MirInst::Const { dst: LocalId::new(0), val: Const::Str(interned("hello")) },
                MirInst::Coerce {
                    dst: LocalId::new(1),
                    src: Operand::Local(LocalId::new(0)),
                    from: Repr::Heap(HeapShape::Str),
                    to: Repr::Tagged,
                },
                MirInst::Print { kind: PrintKind::StrObj, arg: Some(Operand::Local(LocalId::new(1))) },
                MirInst::Print { kind: PrintKind::Newline, arg: None },
            ],
            MirTerminator::Return(None),
        )
    }

    #[test]
    fn accepts_well_formed_print() {
        let f = well_formed_print();
        assert_eq!(verify(&f, std::slice::from_ref(&f)), Ok(()));
    }

    #[test]
    fn rejects_const_str_into_non_heap_str() {
        let mut f = well_formed_print();
        f.locals[0].repr = Repr::Tagged;
        assert!(matches!(verify(&f, &[]), Err(VerifyError::ReprMismatch { .. })));
    }

    #[test]
    fn rejects_branch_cond_non_i8() {
        let f = single_block(
            vec![Repr::Tagged],
            vec![],
            MirTerminator::Branch {
                cond: Operand::Local(LocalId::new(0)),
                then: BlockId::new(0),
                else_: BlockId::new(0),
            },
        );
        assert!(matches!(verify(&f, &[]), Err(VerifyError::BranchCondNotI8 { .. })));
    }

    #[test]
    fn accepts_compare_into_i8() {
        let f = single_block(
            vec![Repr::Tagged, Repr::Tagged, Repr::Raw(RawKind::I8)],
            vec![MirInst::Compare {
                dst: LocalId::new(2),
                op: CmpOp::Lt,
                l: Operand::Local(LocalId::new(0)),
                r: Operand::Local(LocalId::new(1)),
            }],
            MirTerminator::Return(None),
        );
        assert_eq!(verify(&f, &[]), Ok(()));
    }

    #[test]
    fn rejects_mismatched_binop_operands() {
        // l=Raw(F64), r/dst=Tagged: operands must share one repr.
        let f = single_block(
            vec![Repr::Raw(RawKind::F64), Repr::Tagged, Repr::Tagged],
            vec![MirInst::BinOp {
                dst: LocalId::new(2),
                op: BinOp::Add,
                l: Operand::Local(LocalId::new(0)),
                r: Operand::Local(LocalId::new(1)),
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(verify(&f, &[]), Err(VerifyError::ReprMismatch { .. })));
    }

    /// A single repr-uniform `BinOp` over three locals of `repr`.
    fn binop_block(repr: Repr, op: BinOp) -> MirFunction {
        single_block(
            vec![repr.clone(), repr.clone(), repr],
            vec![MirInst::BinOp {
                dst: LocalId::new(2),
                op,
                l: Operand::Local(LocalId::new(0)),
                r: Operand::Local(LocalId::new(1)),
            }],
            MirTerminator::Return(None),
        )
    }

    #[test]
    fn accepts_repr_consistent_binops() {
        // Tagged supports every op; Raw(F64)/Raw(I64) support Add/Sub/Mul.
        assert_eq!(verify(&binop_block(Repr::Tagged, BinOp::Div), &[]), Ok(()));
        assert_eq!(verify(&binop_block(Repr::Raw(RawKind::F64), BinOp::Add), &[]), Ok(()));
        assert_eq!(verify(&binop_block(Repr::Raw(RawKind::F64), BinOp::Mul), &[]), Ok(()));
        assert_eq!(verify(&binop_block(Repr::Raw(RawKind::I64), BinOp::Sub), &[]), Ok(()));
    }

    #[test]
    fn rejects_unsupported_raw_binops() {
        // Div / Mod / bitwise must stay tagged — never on a Raw fast path.
        assert!(matches!(
            verify(&binop_block(Repr::Raw(RawKind::F64), BinOp::Div), &[]),
            Err(VerifyError::BadBinOpRepr { .. })
        ));
        assert!(matches!(
            verify(&binop_block(Repr::Raw(RawKind::I64), BinOp::Mod), &[]),
            Err(VerifyError::BadBinOpRepr { .. })
        ));
        assert!(matches!(
            verify(&binop_block(Repr::Raw(RawKind::I64), BinOp::BitAnd), &[]),
            Err(VerifyError::BadBinOpRepr { .. })
        ));
    }

    #[test]
    fn accepts_well_formed_call_container() {
        // ListPush(list: Tagged, elem: Tagged) → no dst.
        let f = single_block(
            vec![Repr::Heap(HeapShape::List(Box::new(Repr::Tagged))), Repr::Tagged],
            vec![MirInst::CallContainer {
                dst: None,
                op: ContainerOp::ListPush,
                args: vec![Operand::Local(LocalId::new(0)), Operand::Local(LocalId::new(1))],
            }],
            MirTerminator::Return(None),
        );
        // Note: arg0 is Heap(List) which the verifier requires to be Tagged — so
        // this must actually be coerced first. Use a Tagged list operand instead.
        let f2 = single_block(
            vec![Repr::Tagged, Repr::Tagged],
            vec![MirInst::CallContainer {
                dst: None,
                op: ContainerOp::ListPush,
                args: vec![Operand::Local(LocalId::new(0)), Operand::Local(LocalId::new(1))],
            }],
            MirTerminator::Return(None),
        );
        assert!(verify(&f, &[]).is_err(), "Heap element arg must be rejected");
        assert_eq!(verify(&f2, &[]), Ok(()));
    }

    #[test]
    fn rejects_non_tagged_container_element_arg() {
        // PITFALLS A5: a non-Tagged element argument to ListPush is structurally
        // rejected (the element must be coerced to Tagged before storage).
        let f = single_block(
            vec![Repr::Tagged, Repr::Raw(RawKind::F64)],
            vec![MirInst::CallContainer {
                dst: None,
                op: ContainerOp::ListPush,
                args: vec![Operand::Local(LocalId::new(0)), Operand::Local(LocalId::new(1))],
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(verify(&f, &[]), Err(VerifyError::ReprMismatch { .. })));
    }

    #[test]
    fn container_index_arg_must_be_raw_i64() {
        // ListGet(list: Tagged, index: Raw(I64)) → Tagged. A tagged index is
        // rejected (the index position is `Idx`).
        let bad = single_block(
            vec![Repr::Tagged, Repr::Tagged, Repr::Tagged],
            vec![MirInst::CallContainer {
                dst: Some(LocalId::new(2)),
                op: ContainerOp::ListGet,
                args: vec![Operand::Local(LocalId::new(0)), Operand::Local(LocalId::new(1))],
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(verify(&bad, &[]), Err(VerifyError::ReprMismatch { .. })));
        let good = single_block(
            vec![Repr::Tagged, Repr::Raw(RawKind::I64), Repr::Tagged],
            vec![MirInst::CallContainer {
                dst: Some(LocalId::new(2)),
                op: ContainerOp::ListGet,
                args: vec![Operand::Local(LocalId::new(0)), Operand::Local(LocalId::new(1))],
            }],
            MirTerminator::Return(None),
        );
        assert_eq!(verify(&good, &[]), Ok(()));
    }

    #[test]
    fn rejects_illegal_coercion() {
        // A raw float ↔ raw int reinterpretation is not in the legality table.
        let f = single_block(
            vec![Repr::Raw(RawKind::F64), Repr::Raw(RawKind::I64)],
            vec![MirInst::Coerce {
                dst: LocalId::new(1),
                src: Operand::Local(LocalId::new(0)),
                from: Repr::Raw(RawKind::F64),
                to: Repr::Raw(RawKind::I64),
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(verify(&f, &[]), Err(VerifyError::IllegalCoercion { .. })));
    }

    // ── classes (Phase 5) ──

    #[test]
    fn accepts_well_formed_instance_ops() {
        let cid = pyaot_utils::ClassId::new(67);
        let class_repr = Repr::Heap(HeapShape::Class(cid));
        // local0: instance (Heap(Class)); local1: a Tagged field value/result.
        let f = single_block(
            vec![class_repr.clone(), Repr::Tagged],
            vec![
                MirInst::MakeInstance { dst: LocalId::new(0), class_id: cid, field_count: 2 },
                MirInst::SetField {
                    base: Operand::Local(LocalId::new(0)),
                    slot: 0,
                    value: Operand::Local(LocalId::new(1)),
                },
                MirInst::GetField {
                    dst: LocalId::new(1),
                    base: Operand::Local(LocalId::new(0)),
                    slot: 1,
                },
            ],
            MirTerminator::Return(None),
        );
        assert_eq!(verify(&f, &[]), Ok(()));
    }

    #[test]
    fn rejects_getfield_non_instance_base() {
        // base = Raw(F64) is not a valid instance base (PITFALLS B12).
        let f = single_block(
            vec![Repr::Raw(RawKind::F64), Repr::Tagged],
            vec![MirInst::GetField {
                dst: LocalId::new(1),
                base: Operand::Local(LocalId::new(0)),
                slot: 0,
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(verify(&f, &[]), Err(VerifyError::InstanceBaseRepr { .. })));
    }

    #[test]
    fn rejects_getfield_non_tagged_dst() {
        let cid = pyaot_utils::ClassId::new(67);
        let f = single_block(
            vec![Repr::Heap(HeapShape::Class(cid)), Repr::Raw(RawKind::F64)],
            vec![MirInst::GetField {
                dst: LocalId::new(1),
                base: Operand::Local(LocalId::new(0)),
                slot: 0,
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(verify(&f, &[]), Err(VerifyError::ReprMismatch { .. })));
    }

    #[test]
    fn rejects_make_instance_non_class_dst() {
        let cid = pyaot_utils::ClassId::new(67);
        // dst declared Tagged, not Heap(Class) → rejected.
        let f = single_block(
            vec![Repr::Tagged],
            vec![MirInst::MakeInstance { dst: LocalId::new(0), class_id: cid, field_count: 0 }],
            MirTerminator::Return(None),
        );
        assert!(matches!(verify(&f, &[]), Err(VerifyError::MakeInstanceDst { .. })));
    }

    // ── closures / generators (Phase 6) ──

    #[test]
    fn rejects_non_tagged_closure_capture() {
        use pyaot_types::SigRepr;
        // MakeClosure captures must be Tagged cell pointers (P6-2). A Raw(F64)
        // capture is structurally rejected.
        let sig = SigRepr { params: vec![Repr::Tagged], ret: Box::new(Repr::Tagged) };
        // funcs[0] = the target: (env: Tagged, p0: Tagged) -> Tagged.
        let target = MirFunction {
            name: interned("f"),
            params: vec![Repr::Tagged, Repr::Tagged],
            ret: Repr::Tagged,
            locals: vec![LocalDecl { repr: Repr::Tagged }],
            blocks: vec![MirBlock { insts: vec![], term: MirTerminator::Return(None) }],
            entry: BlockId::new(0),
        };
        let caller = single_block(
            vec![Repr::Closure(Box::new(sig)), Repr::Raw(RawKind::F64)],
            vec![MirInst::MakeClosure {
                dst: LocalId::new(0),
                func: pyaot_utils::FuncId::new(0),
                captures: vec![Operand::Local(LocalId::new(1))],
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(
            verify(&caller, &[target]),
            Err(VerifyError::ReprMismatch { .. })
        ));
    }

    #[test]
    fn rejects_call_indirect_arg_mismatch() {
        use pyaot_types::SigRepr;
        let sig = SigRepr { params: vec![Repr::Tagged], ret: Box::new(Repr::Tagged) };
        // callee is Closure(sig); the single arg is Raw(F64) but sig wants Tagged.
        let f = single_block(
            vec![Repr::Closure(Box::new(sig.clone())), Repr::Raw(RawKind::F64), Repr::Tagged],
            vec![MirInst::CallIndirect {
                dst: Some(LocalId::new(2)),
                callee: Operand::Local(LocalId::new(0)),
                args: vec![Operand::Local(LocalId::new(1))],
                sig,
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(verify(&f, &[]), Err(VerifyError::ReprMismatch { .. })));
    }

    #[test]
    fn rejects_raw_gen_set_local_value() {
        use crate::GenOp;
        // GenOp::SetLocal value must be Tagged (P6-3 tagged slot storage).
        let f = single_block(
            vec![Repr::Tagged, Repr::Raw(RawKind::F64)],
            vec![MirInst::GenOpInst {
                dst: None,
                op: GenOp::SetLocal,
                gen: Operand::Local(LocalId::new(0)),
                imm: 0,
                value: Some(Operand::Local(LocalId::new(1))),
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(verify(&f, &[]), Err(VerifyError::ReprMismatch { .. })));
    }

    #[test]
    fn rejects_make_generator_non_tagged_dst() {
        // MakeGenerator dst must be Tagged.
        let f = single_block(
            vec![Repr::Raw(RawKind::F64)],
            vec![MirInst::MakeGenerator { dst: LocalId::new(0), gen_id: 0, num_locals: 2 }],
            MirTerminator::Return(None),
        );
        assert!(matches!(verify(&f, &[]), Err(VerifyError::ReprMismatch { .. })));
    }

    #[test]
    fn rejects_setfield_non_tagged_value() {
        let cid = pyaot_utils::ClassId::new(67);
        let f = single_block(
            vec![Repr::Heap(HeapShape::Class(cid)), Repr::Raw(RawKind::F64)],
            vec![MirInst::SetField {
                base: Operand::Local(LocalId::new(0)),
                slot: 0,
                value: Operand::Local(LocalId::new(1)),
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(verify(&f, &[]), Err(VerifyError::ReprMismatch { .. })));
    }
}
