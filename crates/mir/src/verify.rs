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
    ExcQuery, GenOp, GenResult, MirFunction, MirInst, MirRaise, MirTerminator, Operand, PrintKind,
    UnaryOp,
};
use pyaot_types::{HeapShape, RawKind, Repr};
use pyaot_utils::{BlockId, LocalId};

/// A representation-consistency violation found by [`verify`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    EmptyFunction,
    BadEntry {
        entry: usize,
        count: usize,
    },
    LocalOutOfRange {
        local: usize,
        count: usize,
    },
    BlockOutOfRange {
        block: usize,
        count: usize,
    },
    FuncOutOfRange {
        func: usize,
        count: usize,
    },
    /// An instruction operand/result repr disagrees with its typed signature.
    ReprMismatch {
        ctx: &'static str,
        expected: Repr,
        actual: Repr,
    },
    /// `Call` arity disagrees with the callee signature.
    CallArity {
        func: usize,
        expected: usize,
        actual: usize,
    },
    /// `(from, to)` is not an accepted coercion.
    IllegalCoercion {
        from: Repr,
        to: Repr,
    },
    PrintUnexpectedArg {
        kind: PrintKind,
    },
    PrintMissingArg {
        kind: PrintKind,
    },
    /// `Branch.cond` is not `Raw(I8)`.
    BranchCondNotI8 {
        got: Repr,
    },
    /// A `BinOp` runs on a representation that does not support it (e.g. a raw
    /// `Add`/`Sub`/`Mul` is fine on `Raw(F64)`/`Raw(I64)`, but `Div`/`//`/`%`/`**`
    /// and bitwise/shift must stay on the tagged baseline).
    BadBinOpRepr {
        op: BinOp,
        repr: Repr,
    },
    /// A `CallContainer` arity disagrees with the op's argument signature.
    ContainerArity {
        op: ContainerOp,
        expected: usize,
        actual: usize,
    },
    /// `CallRuntime` arg count must equal the descriptor's param count.
    RuntimeArity {
        symbol: &'static str,
        expected: usize,
        actual: usize,
    },
    /// `CallRuntime` dst presence must match the descriptor's `returns`, and
    /// an arg/dst repr must match the descriptor's register class + semantic.
    RuntimeShape {
        symbol: &'static str,
        detail: &'static str,
        actual: Option<Repr>,
    },
    /// A `CallContainer` carries a `dst` for a mutating op, or omits it for a
    /// value-producing op.
    ContainerDst {
        op: ContainerOp,
        want_dst: bool,
    },
    /// A `CallContainer` result local has the wrong representation for the op.
    ContainerResultRepr {
        op: ContainerOp,
        actual: Repr,
    },
    /// An instance instruction's `base` is neither `Heap(Class(_))` nor `Tagged`
    /// (PITFALLS B12: only such a value may be handed to `rt_instance_*`).
    InstanceBaseRepr {
        ctx: &'static str,
        actual: Repr,
    },
    /// A `MakeInstance` `dst` is not `Heap(Class(class_id))`.
    MakeInstanceDst {
        class_id: u32,
        actual: Repr,
    },
    /// A `MakeClosure` whose `dst` signature does not equal the target
    /// function's signature minus its env param (or whose target lacks the
    /// `Tagged` env param 0) — Phase 6A.
    ClosureSigMismatch {
        func: usize,
    },
    /// A `CallIndirect` whose callee repr is not `Closure(sig)` for the carried
    /// `sig` — Phase 6A.
    IndirectCalleeRepr {
        actual: Repr,
    },
    /// A `Raise`/`AssertFail` that is not the last instruction of its block, or
    /// whose block terminator is not `Unreachable` (Phase 7A).
    BadRaiseShape,
    /// A block whose `handler` annotation is the entry block (the entry has
    /// no predecessor frame state to land in) or is out of range.
    BadHandler { block: usize },
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
            VerifyError::ReprMismatch {
                ctx,
                expected,
                actual,
            } => {
                write!(f, "{ctx}: expected {expected:?}, got {actual:?}")
            }
            VerifyError::CallArity {
                func,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "call to func {func}: expected {expected} args, got {actual}"
                )
            }
            VerifyError::IllegalCoercion { from, to } => {
                write!(
                    f,
                    "illegal coercion {from:?} -> {to:?} (not in the legality table)"
                )
            }
            VerifyError::PrintUnexpectedArg { kind } => {
                write!(f, "Print({kind:?}) takes no argument but one was supplied")
            }
            VerifyError::PrintMissingArg { kind } => {
                write!(
                    f,
                    "Print({kind:?}) requires an argument but none was supplied"
                )
            }
            VerifyError::BranchCondNotI8 { got } => {
                write!(f, "Branch.cond must be Raw(I8), got {got:?}")
            }
            VerifyError::BadBinOpRepr { op, repr } => {
                write!(f, "BinOp {op:?} is not legal on representation {repr:?}")
            }
            VerifyError::ContainerArity {
                op,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "CallContainer {op:?}: expected {expected} args, got {actual}"
                )
            }
            VerifyError::RuntimeArity {
                symbol,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "CallRuntime {symbol}: expected {expected} args, got {actual}"
                )
            }
            VerifyError::RuntimeShape {
                symbol,
                detail,
                actual,
            } => match actual {
                Some(repr) => write!(f, "CallRuntime {symbol}: {detail}, got {repr:?}"),
                None => write!(f, "CallRuntime {symbol}: {detail}"),
            },
            VerifyError::ContainerDst { op, want_dst } => {
                if *want_dst {
                    write!(
                        f,
                        "CallContainer {op:?} requires a dst but none was supplied"
                    )
                } else {
                    write!(
                        f,
                        "CallContainer {op:?} is a mutating op and must not have a dst"
                    )
                }
            }
            VerifyError::ContainerResultRepr { op, actual } => {
                write!(
                    f,
                    "CallContainer {op:?}: dst has the wrong representation {actual:?}"
                )
            }
            VerifyError::InstanceBaseRepr { ctx, actual } => {
                write!(
                    f,
                    "{ctx}: instance base must be Heap(Class) or Tagged, got {actual:?}"
                )
            }
            VerifyError::MakeInstanceDst { class_id, actual } => {
                write!(
                    f,
                    "MakeInstance dst must be Heap(Class({class_id})), got {actual:?}"
                )
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
            VerifyError::BadRaiseShape => {
                write!(
                    f,
                    "Raise/AssertFail must be the last instruction of its block, \
                     with an Unreachable terminator"
                )
            }
            VerifyError::BadHandler { block } => {
                write!(
                    f,
                    "block {block}: handler annotation is out of range or the entry block"
                )
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
        Err(VerifyError::LocalOutOfRange {
            local: id.index(),
            count: f.locals.len(),
        })
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
        Err(VerifyError::BlockOutOfRange {
            block: id.index(),
            count: f.blocks.len(),
        })
    } else {
        Ok(())
    }
}

/// Require an operand's repr to equal `w`.
fn want(f: &MirFunction, op: &Operand, w: &Repr, ctx: &'static str) -> Result<(), VerifyError> {
    check_operand(f, op)?;
    let got = f.operand_repr(op);
    if got != w {
        return Err(VerifyError::ReprMismatch {
            ctx,
            expected: w.clone(),
            actual: got.clone(),
        });
    }
    Ok(())
}

/// Require an operand to be a valid instance base: `Heap(Class(_))` or `Tagged`
/// (PITFALLS B12 — only such a value may be dereferenced by `rt_instance_*`).
fn want_instance_base(f: &MirFunction, op: &Operand, ctx: &'static str) -> Result<(), VerifyError> {
    let got = f.operand_repr(op);
    match got {
        Repr::Tagged | Repr::Heap(HeapShape::Class(_)) => Ok(()),
        other => Err(VerifyError::InstanceBaseRepr {
            ctx,
            actual: other.clone(),
        }),
    }
}

/// Require a destination local's declared repr to equal `w`.
fn want_local(
    f: &MirFunction,
    id: LocalId,
    w: &Repr,
    ctx: &'static str,
) -> Result<(), VerifyError> {
    check_local(f, id)?;
    let got = f.local_repr(id);
    if got != w {
        return Err(VerifyError::ReprMismatch {
            ctx,
            expected: w.clone(),
            actual: got.clone(),
        });
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
        return Err(VerifyError::BadEntry {
            entry: f.entry.index(),
            count: nblocks,
        });
    }

    for (bi, block) in f.blocks.iter().enumerate() {
        // A handler annotation must name a real, non-entry block (the entry
        // holds parameter setup and has no frame state to land in).
        if let Some(h) = block.handler {
            if h.index() >= nblocks || h == f.entry {
                return Err(VerifyError::BadHandler { block: bi });
            }
        }
        for (i, inst) in block.insts.iter().enumerate() {
            verify_inst(f, funcs, inst)?;
            // A diverging instruction must be last, with `Unreachable` after it
            // (the AssertFail shape, enforced since Phase 7A).
            if matches!(inst, MirInst::Raise(_) | MirInst::AssertFail)
                && (i + 1 != block.insts.len() || !matches!(block.term, MirTerminator::Unreachable))
            {
                return Err(VerifyError::BadRaiseShape);
            }
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
        // Pure compile-time metadata — nothing to check.
        MirInst::LineMarker(_) => {}
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
                // An integer const materializes tagged by default, but may also
                // target a raw integer register slot directly (Phase 8B:
                // descriptor-ABI immediates — field indexes, arg counts).
                Const::Int(_) => (
                    matches!(
                        repr,
                        Repr::Tagged
                            | Repr::Raw(RawKind::I64)
                            | Repr::Raw(RawKind::I8)
                            | Repr::Raw(RawKind::I32)
                    ),
                    TAGGED,
                ),
                Const::Bool(_) | Const::None | Const::NullPtr => (*repr == TAGGED, TAGGED),
                Const::Float(_) => (*repr == RAW_F64, RAW_F64),
            };
            if !ok {
                return Err(VerifyError::ReprMismatch {
                    ctx: "Const dst",
                    expected,
                    actual: repr.clone(),
                });
            }
        }
        // Direct field access: `verify` is in-crate, and its negative tests
        // build deliberately-illegal payloads the public constructors refuse.
        MirInst::Coerce(c) => {
            check_local(f, c.dst)?;
            check_operand(f, &c.src)?;
            // The repr cross-check against the locals table is NOT reachable
            // through the constructors (they validate the pair, not the
            // tables) — it stays load-bearing.
            let src_repr = f.operand_repr(&c.src);
            if *src_repr != c.from {
                return Err(VerifyError::ReprMismatch {
                    ctx: "Coerce.from",
                    expected: c.from.clone(),
                    actual: src_repr.clone(),
                });
            }
            let dst_repr = f.local_repr(c.dst);
            if *dst_repr != c.to {
                return Err(VerifyError::ReprMismatch {
                    ctx: "Coerce.to",
                    expected: c.to.clone(),
                    actual: dst_repr.clone(),
                });
            }
            // Pair-legality is unreachable via `CoerceInst::new`/`new_checked`
            // — kept as defense-in-depth against in-crate construction.
            if c.checked {
                // A checked (runtime-validated) unbox is legal ONLY for the two
                // stdlib raw-ABI boundary shapes (Phase 8H, D3). These two shapes
                // (`Tagged→Raw(F64)`, `Tagged→Raw(I64)`) are the only checked
                // admissions because each has a matching runtime guard that raises
                // `TypeError` instead of SEGV (`rt_unbox_float` / `rt_unbox_int`,
                // `runtime/src/boxing.rs`). Never widen this set without adding the
                // matching `rt_*` guard first — doing so reopens the Phase 8B–8F
                // gradual-seam SEGV family. See PITFALLS B18.
                let legal = c.from == Repr::Tagged
                    && matches!(c.to, Repr::Raw(RawKind::F64) | Repr::Raw(RawKind::I64));
                if !legal {
                    return Err(VerifyError::IllegalCoercion {
                        from: c.from.clone(),
                        to: c.to.clone(),
                    });
                }
            } else if classify_coercion(&c.from, &c.to).is_none() {
                return Err(VerifyError::IllegalCoercion {
                    from: c.from.clone(),
                    to: c.to.clone(),
                });
            }
        }
        MirInst::BinOp { dst, op, l, r } => {
            // Repr-consistent: operands and dst share one representation `R`, and
            // `R` must support `op`. `Tagged` handles every op via tag dispatch
            // (`rt_obj_*`, bignum-safe). The unboxed fast paths are RawKind-aware:
            // `Raw(F64)` carries `Add`/`Sub`/`Mul` (exception-free IEEE);
            // `Raw(I64)` additionally carries `Mod`/`FloorDiv` (Phase 3c), all
            // proven by typeck's interval pass (no overflow, divisor > 0).
            check_operand(f, l)?;
            let lhs = f.operand_repr(l).clone();
            want(f, r, &lhs, "BinOp.r")?;
            want_local(f, *dst, &lhs, "BinOp.dst")?;
            let raw_ok = match lhs {
                Repr::Raw(RawKind::F64) => matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul),
                Repr::Raw(RawKind::I64) => matches!(
                    op,
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Mod | BinOp::FloorDiv
                ),
                _ => false,
            };
            match lhs {
                Repr::Tagged => {}
                Repr::Raw(RawKind::F64) | Repr::Raw(RawKind::I64) if raw_ok => {}
                other => {
                    return Err(VerifyError::BadBinOpRepr {
                        op: *op,
                        repr: other,
                    })
                }
            }
        }
        MirInst::Unary { dst, op, operand } => {
            want(f, operand, &TAGGED, "Unary.operand")?;
            let dst_want = if *op == UnaryOp::Not {
                &RAW_I8
            } else {
                &TAGGED
            };
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
                return Err(VerifyError::FuncOutOfRange {
                    func: func.index(),
                    count: funcs.len(),
                });
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
        MirInst::CallRuntime { dst, def, args } => verify_call_runtime(f, def, dst, args)?,
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
        MirInst::SetField {
            base,
            slot: _,
            value,
        } => {
            check_operand(f, base)?;
            want_instance_base(f, base, "SetField.base")?;
            want(f, value, &TAGGED, "SetField.value")?;
        }
        // By-name field access on a `Dyn` receiver (Phase 8H, D4): everything
        // rides Tagged — the runtime validates the instance shape itself.
        MirInst::GetFieldNamed {
            dst,
            base,
            name_hash: _,
        } => {
            check_operand(f, base)?;
            want(f, base, &TAGGED, "GetFieldNamed.base")?;
            want_local(f, *dst, &TAGGED, "GetFieldNamed.dst")?;
        }
        MirInst::SetFieldNamed {
            base,
            name_hash: _,
            value,
        } => {
            check_operand(f, base)?;
            want(f, base, &TAGGED, "SetFieldNamed.base")?;
            want(f, value, &TAGGED, "SetFieldNamed.value")?;
        }
        MirInst::CallVirtual {
            dst,
            recv,
            args,
            ret,
            ..
        } => {
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
        MirInst::MakeClosure {
            dst,
            func,
            captures,
        } => {
            if func.index() >= funcs.len() {
                return Err(VerifyError::FuncOutOfRange {
                    func: func.index(),
                    count: funcs.len(),
                });
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
        MirInst::CallIndirect {
            dst,
            callee,
            args,
            sig,
        } => {
            check_operand(f, callee)?;
            let callee_repr = f.operand_repr(callee);
            match callee_repr {
                Repr::Closure(s) if **s == *sig => {}
                other => {
                    return Err(VerifyError::IndirectCalleeRepr {
                        actual: other.clone(),
                    })
                }
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
        MirInst::GenOpInst {
            dst,
            op,
            gen,
            value,
            ..
        } => {
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
        // ── exceptions (Phase 7) ──
        MirInst::ExcOp(_) => {}
        MirInst::ExcQuery { dst, query } => match query {
            ExcQuery::Current => want_local(f, *dst, &TAGGED, "ExcQuery(Current).dst")?,
            ExcQuery::MatchesBuiltin(_) | ExcQuery::MatchesClass(_) => {
                want_local(f, *dst, &RAW_I8, "ExcQuery(Matches*).dst")?
            }
        },
        MirInst::ExcInstanceStr { dst, value } => {
            want(f, value, &TAGGED, "ExcInstanceStr.value")?;
            want_local(f, *dst, &Repr::Heap(HeapShape::Str), "ExcInstanceStr.dst")?;
        }
        MirInst::Raise(r) => {
            let tagged = |op: &Option<Operand>, ctx: &'static str| -> Result<(), VerifyError> {
                if let Some(op) = op {
                    want(f, op, &TAGGED, ctx)?;
                }
                Ok(())
            };
            match r {
                MirRaise::Builtin { msg, .. } | MirRaise::BuiltinFromNone { msg, .. } => {
                    tagged(msg, "Raise.msg")?;
                }
                MirRaise::BuiltinFrom { msg, cause_msg, .. } => {
                    tagged(msg, "Raise.msg")?;
                    tagged(cause_msg, "Raise.cause_msg")?;
                }
                MirRaise::CustomWithInstance { msg, instance, .. } => {
                    tagged(msg, "Raise.msg")?;
                    want(f, instance, &TAGGED, "Raise.instance")?;
                }
                MirRaise::Stdlib { msg, .. } => tagged(msg, "Raise.msg")?,
                MirRaise::Instance { value } => want(f, value, &TAGGED, "Raise.value")?,
                MirRaise::Reraise => {}
            }
        }
        MirInst::Print { kind, arg } => match kind {
            PrintKind::Newline | PrintKind::Sep | PrintKind::None_ => {
                if arg.is_some() {
                    return Err(VerifyError::PrintUnexpectedArg { kind: *kind });
                }
            }
            PrintKind::StrObj | PrintKind::Obj => {
                let op = arg
                    .as_ref()
                    .ok_or(VerifyError::PrintMissingArg { kind: *kind })?;
                want(f, op, &TAGGED, "Print(Obj/StrObj).arg")?;
            }
            PrintKind::Float => {
                let op = arg
                    .as_ref()
                    .ok_or(VerifyError::PrintMissingArg { kind: *kind })?;
                want(f, op, &RAW_F64, "Print(Float).arg")?;
            }
            PrintKind::Bool => {
                let op = arg
                    .as_ref()
                    .ok_or(VerifyError::PrintMissingArg { kind: *kind })?;
                want(f, op, &RAW_I8, "Print(Bool).arg")?;
            }
            PrintKind::Int => {
                let op = arg
                    .as_ref()
                    .ok_or(VerifyError::PrintMissingArg { kind: *kind })?;
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
/// Verify a [`MirInst::CallRuntime`] against its descriptor (Phase 8B).
///
/// Per-slot rule, from the descriptor's Cranelift register class plus its
/// `MirSemantic` annotation when present:
/// * `F64` / `I8` / `I32` → the matching `Raw` exactly.
/// * `I64` + `MirSemantic::Tagged` / `Heap` → a GC-rootable repr
///   (`Tagged`/`Heap`) — never a raw integer misread as a pointer.
/// * `I64` + `MirSemantic::Raw` (incl. the un-annotated default) → `Raw(I64)`
///   or a rootable repr. The class stays wide here because many stdlib
///   descriptors are built with the bare `RuntimeFuncDef::new` constructor
///   (no semantics annotated) yet pass tagged `Value`s — lowering derives the
///   precise repr from the stdlib `TypeSpec`, which this verifier cannot see.
fn verify_call_runtime(
    f: &MirFunction,
    def: &'static pyaot_core_defs::RuntimeFuncDef,
    dst: &Option<LocalId>,
    args: &[Operand],
) -> Result<(), VerifyError> {
    use pyaot_core_defs::runtime_func_def::{MirSemantic, ParamType, ReturnType};
    if args.len() != def.params.len() {
        return Err(VerifyError::RuntimeArity {
            symbol: def.symbol,
            expected: def.params.len(),
            actual: args.len(),
        });
    }
    for (i, (arg, pt)) in args.iter().zip(def.params).enumerate() {
        check_operand(f, arg)?;
        let got = f.operand_repr(arg);
        let ok = match pt {
            ParamType::F64 => *got == RAW_F64,
            ParamType::I8 => *got == RAW_I8,
            ParamType::I32 => *got == Repr::Raw(RawKind::I32),
            ParamType::I64 => match def.param_semantic(i) {
                MirSemantic::Tagged | MirSemantic::Heap(_) => {
                    matches!(got, Repr::Tagged | Repr::Heap(_))
                }
                MirSemantic::Raw => {
                    matches!(got, Repr::Raw(RawKind::I64) | Repr::Tagged | Repr::Heap(_))
                }
            },
        };
        if !ok {
            return Err(VerifyError::RuntimeShape {
                symbol: def.symbol,
                detail: "arg repr does not match the descriptor's register class",
                actual: Some(got.clone()),
            });
        }
    }
    match (def.returns, dst) {
        (None, Some(_)) => Err(VerifyError::RuntimeShape {
            symbol: def.symbol,
            detail: "void descriptor must not have a dst",
            actual: None,
        }),
        (Some(_), None) => Err(VerifyError::RuntimeShape {
            symbol: def.symbol,
            detail: "returning descriptor requires a dst",
            actual: None,
        }),
        (None, None) => Ok(()),
        (Some(rt), Some(d)) => {
            check_local(f, *d)?;
            let got = f.local_repr(*d);
            let ok = match rt {
                ReturnType::F64 => *got == RAW_F64,
                ReturnType::I8 => *got == RAW_I8,
                ReturnType::I32 => *got == Repr::Raw(RawKind::I32),
                ReturnType::I64 => {
                    matches!(got, Repr::Raw(RawKind::I64) | Repr::Tagged | Repr::Heap(_))
                }
            };
            if ok {
                Ok(())
            } else {
                Err(VerifyError::RuntimeShape {
                    symbol: def.symbol,
                    detail: "dst repr does not match the descriptor's return class",
                    actual: Some(got.clone()),
                })
            }
        }
    }
}

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
                return Err(VerifyError::ContainerDst {
                    op,
                    want_dst: false,
                });
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
                return Err(VerifyError::ContainerResultRepr {
                    op,
                    actual: got.clone(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    fn interned_file() -> pyaot_utils::InternedString {
        pyaot_utils::StringInterner::new().intern("test.py")
    }
    use super::*;
    use crate::{BinOp, CmpOp, CoerceInst, Const, LocalDecl, MirBlock, MirFunction, Operand};
    use pyaot_types::HeapShape;
    use pyaot_utils::{BlockId, InternedString, StringInterner};

    fn interned(s: &str) -> InternedString {
        StringInterner::new().intern(s)
    }

    fn single_block(locals: Vec<Repr>, insts: Vec<MirInst>, term: MirTerminator) -> MirFunction {
        MirFunction {
            name: interned("__main__"),
            file: interned_file(),
            params: Vec::new(),
            ret: Repr::Tagged,
            locals: locals.into_iter().map(|repr| LocalDecl { repr }).collect(),
            blocks: vec![MirBlock {
                insts,
                term,
                handler: None,
            }],
            entry: BlockId::new(0),
        }
    }

    fn well_formed_print() -> MirFunction {
        single_block(
            vec![Repr::Heap(HeapShape::Str), Repr::Tagged],
            vec![
                MirInst::Const {
                    dst: LocalId::new(0),
                    val: Const::Str(interned("hello")),
                },
                MirInst::Coerce(CoerceInst {
                    dst: LocalId::new(1),
                    src: Operand::Local(LocalId::new(0)),
                    from: Repr::Heap(HeapShape::Str),
                    to: Repr::Tagged,
                    checked: false,
                }),
                MirInst::Print {
                    kind: PrintKind::StrObj,
                    arg: Some(Operand::Local(LocalId::new(1))),
                },
                MirInst::Print {
                    kind: PrintKind::Newline,
                    arg: None,
                },
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
        assert!(matches!(
            verify(&f, &[]),
            Err(VerifyError::ReprMismatch { .. })
        ));
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
        assert!(matches!(
            verify(&f, &[]),
            Err(VerifyError::BranchCondNotI8 { .. })
        ));
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
        assert!(matches!(
            verify(&f, &[]),
            Err(VerifyError::ReprMismatch { .. })
        ));
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
        // Tagged supports every op; Raw(F64) supports Add/Sub/Mul; Raw(I64) those
        // plus Mod/FloorDiv (the Phase-3c raw division surface).
        assert_eq!(verify(&binop_block(Repr::Tagged, BinOp::Div), &[]), Ok(()));
        assert_eq!(
            verify(&binop_block(Repr::Raw(RawKind::F64), BinOp::Add), &[]),
            Ok(())
        );
        assert_eq!(
            verify(&binop_block(Repr::Raw(RawKind::F64), BinOp::Mul), &[]),
            Ok(())
        );
        assert_eq!(
            verify(&binop_block(Repr::Raw(RawKind::I64), BinOp::Sub), &[]),
            Ok(())
        );
        assert_eq!(
            verify(&binop_block(Repr::Raw(RawKind::I64), BinOp::Mod), &[]),
            Ok(())
        );
        assert_eq!(
            verify(&binop_block(Repr::Raw(RawKind::I64), BinOp::FloorDiv), &[]),
            Ok(())
        );
    }

    #[test]
    fn rejects_unsupported_raw_binops() {
        // Raw(F64) carries only Add/Sub/Mul; Raw division/mod and all bitwise/shift
        // and `Pow` stay tagged — never on a Raw fast path.
        assert!(matches!(
            verify(&binop_block(Repr::Raw(RawKind::F64), BinOp::Div), &[]),
            Err(VerifyError::BadBinOpRepr { .. })
        ));
        assert!(matches!(
            verify(&binop_block(Repr::Raw(RawKind::F64), BinOp::Mod), &[]),
            Err(VerifyError::BadBinOpRepr { .. })
        ));
        assert!(matches!(
            verify(&binop_block(Repr::Raw(RawKind::I64), BinOp::Div), &[]),
            Err(VerifyError::BadBinOpRepr { .. })
        ));
        assert!(matches!(
            verify(&binop_block(Repr::Raw(RawKind::I64), BinOp::Pow), &[]),
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
            vec![
                Repr::Heap(HeapShape::List(Box::new(Repr::Tagged))),
                Repr::Tagged,
            ],
            vec![MirInst::CallContainer {
                dst: None,
                op: ContainerOp::ListPush,
                args: vec![
                    Operand::Local(LocalId::new(0)),
                    Operand::Local(LocalId::new(1)),
                ],
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
                args: vec![
                    Operand::Local(LocalId::new(0)),
                    Operand::Local(LocalId::new(1)),
                ],
            }],
            MirTerminator::Return(None),
        );
        assert!(
            verify(&f, &[]).is_err(),
            "Heap element arg must be rejected"
        );
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
                args: vec![
                    Operand::Local(LocalId::new(0)),
                    Operand::Local(LocalId::new(1)),
                ],
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(
            verify(&f, &[]),
            Err(VerifyError::ReprMismatch { .. })
        ));
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
                args: vec![
                    Operand::Local(LocalId::new(0)),
                    Operand::Local(LocalId::new(1)),
                ],
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(
            verify(&bad, &[]),
            Err(VerifyError::ReprMismatch { .. })
        ));
        let good = single_block(
            vec![Repr::Tagged, Repr::Raw(RawKind::I64), Repr::Tagged],
            vec![MirInst::CallContainer {
                dst: Some(LocalId::new(2)),
                op: ContainerOp::ListGet,
                args: vec![
                    Operand::Local(LocalId::new(0)),
                    Operand::Local(LocalId::new(1)),
                ],
            }],
            MirTerminator::Return(None),
        );
        assert_eq!(verify(&good, &[]), Ok(()));
    }

    #[test]
    fn checked_coerce_legal_only_for_raw_unbox_shapes() {
        // Phase 8H, D3: `checked: true` is legal for (Tagged, Raw(F64)) /
        // (Tagged, Raw(I64)) and nothing else.
        let ok = single_block(
            vec![Repr::Tagged, Repr::Raw(RawKind::F64)],
            vec![MirInst::Coerce(CoerceInst {
                dst: LocalId::new(1),
                src: Operand::Local(LocalId::new(0)),
                from: Repr::Tagged,
                to: Repr::Raw(RawKind::F64),
                checked: true,
            })],
            MirTerminator::Return(None),
        );
        assert_eq!(verify(&ok, &[]), Ok(()));
        // A `checked` pair the public constructor refuses — built directly
        // (in-crate field access) to prove the verifier's own re-check.
        let bad = single_block(
            vec![Repr::Tagged, Repr::Raw(RawKind::I8)],
            vec![MirInst::Coerce(CoerceInst {
                dst: LocalId::new(1),
                src: Operand::Local(LocalId::new(0)),
                from: Repr::Tagged,
                to: Repr::Raw(RawKind::I8),
                checked: true,
            })],
            MirTerminator::Return(None),
        );
        assert!(matches!(
            verify(&bad, &[]),
            Err(VerifyError::IllegalCoercion { .. })
        ));
    }

    #[test]
    fn named_field_insts_ride_tagged() {
        // Phase 8H, D4: GetFieldNamed/SetFieldNamed take Tagged base/dst/value.
        let ok = single_block(
            vec![Repr::Tagged, Repr::Tagged],
            vec![
                MirInst::GetFieldNamed {
                    dst: LocalId::new(1),
                    base: Operand::Local(LocalId::new(0)),
                    name_hash: 42,
                },
                MirInst::SetFieldNamed {
                    base: Operand::Local(LocalId::new(0)),
                    name_hash: 42,
                    value: Operand::Local(LocalId::new(1)),
                },
            ],
            MirTerminator::Return(None),
        );
        assert_eq!(verify(&ok, &[]), Ok(()));
        let bad = single_block(
            vec![Repr::Raw(RawKind::I64), Repr::Tagged],
            vec![MirInst::GetFieldNamed {
                dst: LocalId::new(1),
                base: Operand::Local(LocalId::new(0)),
                name_hash: 42,
            }],
            MirTerminator::Return(None),
        );
        assert!(verify(&bad, &[]).is_err());
    }

    #[test]
    fn rejects_illegal_coercion() {
        // A raw float ↔ raw int reinterpretation is not in the legality table.
        // Built directly (in-crate field access) — `CoerceInst::new` refuses
        // this pair, so the verifier's table re-check needs the back door.
        let f = single_block(
            vec![Repr::Raw(RawKind::F64), Repr::Raw(RawKind::I64)],
            vec![MirInst::Coerce(CoerceInst {
                dst: LocalId::new(1),
                src: Operand::Local(LocalId::new(0)),
                from: Repr::Raw(RawKind::F64),
                to: Repr::Raw(RawKind::I64),
                checked: false,
            })],
            MirTerminator::Return(None),
        );
        assert!(matches!(
            verify(&f, &[]),
            Err(VerifyError::IllegalCoercion { .. })
        ));
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
                MirInst::MakeInstance {
                    dst: LocalId::new(0),
                    class_id: cid,
                    field_count: 2,
                },
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
        assert!(matches!(
            verify(&f, &[]),
            Err(VerifyError::InstanceBaseRepr { .. })
        ));
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
        assert!(matches!(
            verify(&f, &[]),
            Err(VerifyError::ReprMismatch { .. })
        ));
    }

    #[test]
    fn rejects_make_instance_non_class_dst() {
        let cid = pyaot_utils::ClassId::new(67);
        // dst declared Tagged, not Heap(Class) → rejected.
        let f = single_block(
            vec![Repr::Tagged],
            vec![MirInst::MakeInstance {
                dst: LocalId::new(0),
                class_id: cid,
                field_count: 0,
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(
            verify(&f, &[]),
            Err(VerifyError::MakeInstanceDst { .. })
        ));
    }

    // ── closures / generators (Phase 6) ──

    #[test]
    fn rejects_non_tagged_closure_capture() {
        use pyaot_types::SigRepr;
        // MakeClosure captures must be Tagged cell pointers (P6-2). A Raw(F64)
        // capture is structurally rejected.
        let sig = SigRepr {
            params: vec![Repr::Tagged],
            ret: Box::new(Repr::Tagged),
        };
        // funcs[0] = the target: (env: Tagged, p0: Tagged) -> Tagged.
        let target = MirFunction {
            name: interned("f"),
            file: interned_file(),
            params: vec![Repr::Tagged, Repr::Tagged],
            ret: Repr::Tagged,
            locals: vec![LocalDecl { repr: Repr::Tagged }],
            blocks: vec![MirBlock {
                insts: vec![],
                term: MirTerminator::Return(None),
                handler: None,
            }],
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
        let sig = SigRepr {
            params: vec![Repr::Tagged],
            ret: Box::new(Repr::Tagged),
        };
        // callee is Closure(sig); the single arg is Raw(F64) but sig wants Tagged.
        let f = single_block(
            vec![
                Repr::Closure(Box::new(sig.clone())),
                Repr::Raw(RawKind::F64),
                Repr::Tagged,
            ],
            vec![MirInst::CallIndirect {
                dst: Some(LocalId::new(2)),
                callee: Operand::Local(LocalId::new(0)),
                args: vec![Operand::Local(LocalId::new(1))],
                sig,
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(
            verify(&f, &[]),
            Err(VerifyError::ReprMismatch { .. })
        ));
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
        assert!(matches!(
            verify(&f, &[]),
            Err(VerifyError::ReprMismatch { .. })
        ));
    }

    #[test]
    fn rejects_make_generator_non_tagged_dst() {
        // MakeGenerator dst must be Tagged.
        let f = single_block(
            vec![Repr::Raw(RawKind::F64)],
            vec![MirInst::MakeGenerator {
                dst: LocalId::new(0),
                gen_id: 0,
                num_locals: 2,
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(
            verify(&f, &[]),
            Err(VerifyError::ReprMismatch { .. })
        ));
    }

    // ── exceptions (Phase 7) ──

    #[test]
    fn rejects_raise_not_last_in_block() {
        use crate::MirRaise;
        // A Raise followed by another instruction is malformed.
        let f = single_block(
            vec![Repr::Tagged],
            vec![
                MirInst::Raise(MirRaise::Reraise),
                MirInst::Const {
                    dst: LocalId::new(0),
                    val: Const::None,
                },
            ],
            MirTerminator::Unreachable,
        );
        assert!(matches!(verify(&f, &[]), Err(VerifyError::BadRaiseShape)));
    }

    #[test]
    fn rejects_raise_without_unreachable_term() {
        use crate::MirRaise;
        let f = single_block(
            vec![],
            vec![MirInst::Raise(MirRaise::Reraise)],
            MirTerminator::Return(None),
        );
        assert!(matches!(verify(&f, &[]), Err(VerifyError::BadRaiseShape)));
    }

    #[test]
    fn accepts_raise_shape_and_checks_msg_repr() {
        use crate::MirRaise;
        // Well-formed: Raise last + Unreachable, Tagged message operand.
        let ok = single_block(
            vec![Repr::Tagged],
            vec![MirInst::Raise(MirRaise::Builtin {
                tag: 3,
                msg: Some(Operand::Local(LocalId::new(0))),
            })],
            MirTerminator::Unreachable,
        );
        assert_eq!(verify(&ok, &[]), Ok(()));
        // A raw message operand is rejected.
        let bad = single_block(
            vec![Repr::Raw(RawKind::F64)],
            vec![MirInst::Raise(MirRaise::Builtin {
                tag: 3,
                msg: Some(Operand::Local(LocalId::new(0))),
            })],
            MirTerminator::Unreachable,
        );
        assert!(matches!(
            verify(&bad, &[]),
            Err(VerifyError::ReprMismatch { .. })
        ));
    }

    #[test]
    fn rejects_handler_at_entry() {
        let mut f = single_block(vec![], vec![], MirTerminator::Return(None));
        f.blocks[0].handler = Some(BlockId::new(0));
        assert!(matches!(
            verify(&f, &[]),
            Err(VerifyError::BadHandler { block: 0 })
        ));
    }

    #[test]
    fn rejects_handler_out_of_range() {
        let mut f = single_block(vec![], vec![], MirTerminator::Return(None));
        f.blocks[0].handler = Some(BlockId::new(7));
        assert!(matches!(
            verify(&f, &[]),
            Err(VerifyError::BadHandler { block: 0 })
        ));
    }

    #[test]
    fn exc_query_dst_reprs() {
        use crate::ExcQuery;
        // Current → Tagged dst; Matches* → Raw(I8) dst.
        let ok = single_block(
            vec![Repr::Tagged, Repr::Raw(RawKind::I8)],
            vec![
                MirInst::ExcQuery {
                    dst: LocalId::new(0),
                    query: ExcQuery::Current,
                },
                MirInst::ExcQuery {
                    dst: LocalId::new(1),
                    query: ExcQuery::MatchesBuiltin(3),
                },
            ],
            MirTerminator::Return(None),
        );
        assert_eq!(verify(&ok, &[]), Ok(()));
        let bad = single_block(
            vec![Repr::Raw(RawKind::I8)],
            vec![MirInst::ExcQuery {
                dst: LocalId::new(0),
                query: ExcQuery::Current,
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(
            verify(&bad, &[]),
            Err(VerifyError::ReprMismatch { .. })
        ));
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
        assert!(matches!(
            verify(&f, &[]),
            Err(VerifyError::ReprMismatch { .. })
        ));
    }

    /// An interprocedural raw-int callee `(Raw(I64)) -> Raw(I64)` — the shape the
    /// whole-program interval pass produces for a specializable bounded-int
    /// function (PLAN backlog #7, Part A).
    fn raw_i64_callee() -> MirFunction {
        MirFunction {
            name: interned("callee"),
            file: interned_file(),
            params: vec![Repr::Raw(RawKind::I64)],
            ret: Repr::Raw(RawKind::I64),
            locals: vec![LocalDecl {
                repr: Repr::Raw(RawKind::I64),
            }],
            blocks: vec![MirBlock {
                insts: vec![],
                term: MirTerminator::Return(Some(Operand::Local(LocalId::new(0)))),
                handler: None,
            }],
            entry: BlockId::new(0),
        }
    }

    #[test]
    fn accepts_raw_int_call_pair() {
        // Caller (fn 1) passes a Raw(I64) local into the Raw(I64) param and reads
        // the Raw(I64) result into a Raw(I64) dst — `Call.arg` ↔ `callee.params`
        // and `Call.dst` ↔ `callee.ret` both agree.
        let caller = single_block(
            vec![Repr::Raw(RawKind::I64), Repr::Raw(RawKind::I64)],
            vec![MirInst::Call {
                dst: Some(LocalId::new(1)),
                func: pyaot_utils::FuncId::new(0),
                args: vec![Operand::Local(LocalId::new(0))],
            }],
            MirTerminator::Return(None),
        );
        let funcs = vec![raw_i64_callee(), caller];
        assert_eq!(verify(&funcs[0], &funcs), Ok(()));
        assert_eq!(verify(&funcs[1], &funcs), Ok(()));
    }

    #[test]
    fn rejects_tagged_arg_into_raw_int_param() {
        // A Tagged arg into a Raw(I64) param is a `Call.arg` ↔ `callee.params`
        // mismatch — the verifier catches a desync of the three repr sources.
        let caller = single_block(
            vec![Repr::Tagged, Repr::Raw(RawKind::I64)],
            vec![MirInst::Call {
                dst: Some(LocalId::new(1)),
                func: pyaot_utils::FuncId::new(0),
                args: vec![Operand::Local(LocalId::new(0))],
            }],
            MirTerminator::Return(None),
        );
        let funcs = vec![raw_i64_callee(), caller];
        assert!(matches!(
            verify(&funcs[1], &funcs),
            Err(VerifyError::ReprMismatch { .. })
        ));
    }

    #[test]
    fn rejects_tagged_dst_from_raw_int_return() {
        // Reading a Raw(I64) return into a Tagged dst is a `Call.dst` ↔
        // `callee.ret` mismatch.
        let caller = single_block(
            vec![Repr::Raw(RawKind::I64), Repr::Tagged],
            vec![MirInst::Call {
                dst: Some(LocalId::new(1)),
                func: pyaot_utils::FuncId::new(0),
                args: vec![Operand::Local(LocalId::new(0))],
            }],
            MirTerminator::Return(None),
        );
        let funcs = vec![raw_i64_callee(), caller];
        assert!(matches!(
            verify(&funcs[1], &funcs),
            Err(VerifyError::ReprMismatch { .. })
        ));
    }
}
