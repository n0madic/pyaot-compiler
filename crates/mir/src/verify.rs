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
    classify_coercion, is_heap_str, Const, MirFunction, MirInst, MirTerminator, Operand, PrintKind,
    UnaryOp,
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
        }
    }
}

impl std::error::Error for VerifyError {}

const TAGGED: Repr = Repr::Tagged;
const RAW_I8: Repr = Repr::Raw(RawKind::I8);
const RAW_I64: Repr = Repr::Raw(RawKind::I64);
const RAW_F64: Repr = Repr::Raw(RawKind::F64);

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
        MirInst::BinOp { dst, l, r, .. } => {
            // Every binary op runs on the tagged baseline (bignum-safe).
            want(f, l, &TAGGED, "BinOp.l")?;
            want(f, r, &TAGGED, "BinOp.r")?;
            want_local(f, *dst, &TAGGED, "BinOp.dst")?;
        }
        MirInst::Unary { dst, op, operand } => {
            want(f, operand, &TAGGED, "Unary.operand")?;
            let dst_want = if *op == UnaryOp::Not { &RAW_I8 } else { &TAGGED };
            want_local(f, *dst, dst_want, "Unary.dst")?;
        }
        MirInst::Compare { dst, l, r, .. } => {
            want(f, l, &TAGGED, "Compare.l")?;
            want(f, r, &TAGGED, "Compare.r")?;
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
    fn rejects_arithmetic_on_raw() {
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

    #[test]
    fn rejects_illegal_coercion() {
        let f = single_block(
            vec![Repr::Tagged, Repr::Heap(HeapShape::Str)],
            vec![MirInst::Coerce {
                dst: LocalId::new(1),
                src: Operand::Local(LocalId::new(0)),
                from: Repr::Tagged,
                to: Repr::Heap(HeapShape::Str),
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(verify(&f, &[]), Err(VerifyError::IllegalCoercion { .. })));
    }
}
