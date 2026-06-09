//! MIR verifier — checks `Repr` consistency at every pass boundary.
//!
//! `Repr` is the sole representation type, so there are no widening exceptions to
//! bridge a second (logical) type field. The verifier runs in debug builds at
//! every pass boundary (see `pyaot-optimizer`); it is the executable form of the
//! invariant that representation never silently drifts.
//!
//! Phase 1 interprets the `print("hello")` instruction set: [`Const::Str`],
//! [`MirInst::Coerce`], and [`MirInst::Print`]. Reserved instruction/operand
//! kinds are bounds-checked but otherwise accepted, so later phases extend the
//! checks without reshaping this function.

use crate::{
    classify_coercion, is_heap_str, Const, MirFunction, MirInst, MirTerminator, Operand, PrintKind,
};
use pyaot_types::Repr;
use pyaot_utils::LocalId;

/// A representation-consistency violation found by [`verify`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// A function has no blocks (no entry to verify).
    EmptyFunction,
    /// An operand or destination names a local outside the Repr table.
    LocalOutOfRange { local: usize, count: usize },
    /// `Const::Str` must produce a `Heap(Str)` value.
    ConstStrReprMismatch { dst: usize, got: Repr },
    /// `Print { StrObj }` requires a `Tagged` operand (everything reaches print
    /// through the uniform tagged repr — see `lowering::legalize`).
    PrintStrObjNotTagged { got: Repr },
    /// A print kind that takes no argument (`Newline` / `Sep`) was given one.
    PrintUnexpectedArg { kind: PrintKind },
    /// A print kind that needs an argument was given none.
    PrintMissingArg { kind: PrintKind },
    /// `Coerce.from` disagrees with the source operand's actual representation.
    CoerceFromMismatch { expected: Repr, actual: Repr },
    /// `Coerce.to` disagrees with the destination local's declared representation.
    CoerceToMismatch { expected: Repr, actual: Repr },
    /// `(from, to)` is not an accepted coercion (`classify_coercion` rejected it).
    IllegalCoercion { from: Repr, to: Repr },
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::EmptyFunction => write!(f, "MIR function has no blocks"),
            VerifyError::LocalOutOfRange { local, count } => {
                write!(f, "local {local} out of range (function has {count} locals)")
            }
            VerifyError::ConstStrReprMismatch { dst, got } => {
                write!(f, "Const::Str into local {dst} must be Heap(Str), got {got:?}")
            }
            VerifyError::PrintStrObjNotTagged { got } => {
                write!(f, "Print(StrObj) operand must be Tagged, got {got:?}")
            }
            VerifyError::PrintUnexpectedArg { kind } => {
                write!(f, "Print({kind:?}) takes no argument but one was supplied")
            }
            VerifyError::PrintMissingArg { kind } => {
                write!(f, "Print({kind:?}) requires an argument but none was supplied")
            }
            VerifyError::CoerceFromMismatch { expected, actual } => {
                write!(f, "Coerce.from is {expected:?} but the source operand is {actual:?}")
            }
            VerifyError::CoerceToMismatch { expected, actual } => {
                write!(f, "Coerce.to is {expected:?} but the destination local is {actual:?}")
            }
            VerifyError::IllegalCoercion { from, to } => {
                write!(f, "illegal coercion {from:?} -> {to:?} (not in the legality table)")
            }
        }
    }
}

impl std::error::Error for VerifyError {}

/// Verify a single MIR function's representation consistency.
///
/// "Every block ends in exactly one terminator" is enforced structurally:
/// [`crate::MirBlock`] holds exactly one `term` field, so it cannot end in zero
/// or two terminators by construction.
pub fn verify(f: &MirFunction) -> Result<(), VerifyError> {
    if f.blocks.is_empty() {
        return Err(VerifyError::EmptyFunction);
    }

    let count = f.locals.len();
    let check_local = |id: LocalId| -> Result<(), VerifyError> {
        if id.index() >= count {
            Err(VerifyError::LocalOutOfRange { local: id.index(), count })
        } else {
            Ok(())
        }
    };
    let check_operand = |op: &Operand| -> Result<(), VerifyError> {
        match op {
            Operand::Local(id) => check_local(*id),
        }
    };

    for block in &f.blocks {
        for inst in &block.insts {
            match inst {
                MirInst::Const { dst, val } => {
                    check_local(*dst)?;
                    match val {
                        Const::Str(_) => {
                            let repr = f.local_repr(*dst);
                            if !is_heap_str(repr) {
                                return Err(VerifyError::ConstStrReprMismatch {
                                    dst: dst.index(),
                                    got: repr.clone(),
                                });
                            }
                        }
                    }
                }
                MirInst::Coerce { dst, src, from, to } => {
                    check_local(*dst)?;
                    check_operand(src)?;
                    let src_repr = f.operand_repr(src);
                    if src_repr != from {
                        return Err(VerifyError::CoerceFromMismatch {
                            expected: from.clone(),
                            actual: src_repr.clone(),
                        });
                    }
                    let dst_repr = f.local_repr(*dst);
                    if dst_repr != to {
                        return Err(VerifyError::CoerceToMismatch {
                            expected: to.clone(),
                            actual: dst_repr.clone(),
                        });
                    }
                    if classify_coercion(from, to).is_none() {
                        return Err(VerifyError::IllegalCoercion {
                            from: from.clone(),
                            to: to.clone(),
                        });
                    }
                }
                MirInst::Print { kind, arg } => match kind {
                    PrintKind::Newline | PrintKind::Sep => {
                        if arg.is_some() {
                            return Err(VerifyError::PrintUnexpectedArg { kind: *kind });
                        }
                    }
                    PrintKind::StrObj => {
                        let op = arg
                            .as_ref()
                            .ok_or(VerifyError::PrintMissingArg { kind: *kind })?;
                        check_operand(op)?;
                        let repr = f.operand_repr(op);
                        if *repr != Repr::Tagged {
                            return Err(VerifyError::PrintStrObjNotTagged { got: repr.clone() });
                        }
                    }
                    // Int / Float / Bool / None_ / Obj are not produced in Phase 1.
                    // Bounds-check any operand and defer richer checks to Phase 2+.
                    _ => {
                        if let Some(op) = arg {
                            check_operand(op)?;
                        }
                    }
                },
            }
        }

        if let MirTerminator::Return(Some(op)) = &block.term {
            check_operand(op)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Const, LocalDecl, MirBlock, MirFunction};
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

    /// The exact shape `lowering` produces for `print("hello")`.
    fn well_formed_print() -> MirFunction {
        single_block(
            vec![Repr::Heap(HeapShape::Str), Repr::Tagged],
            vec![
                MirInst::Const {
                    dst: LocalId::new(0),
                    val: Const::Str(interned("hello")),
                },
                MirInst::Coerce {
                    dst: LocalId::new(1),
                    src: Operand::Local(LocalId::new(0)),
                    from: Repr::Heap(HeapShape::Str),
                    to: Repr::Tagged,
                },
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
        assert_eq!(verify(&well_formed_print()), Ok(()));
    }

    #[test]
    fn rejects_const_str_into_non_heap_str() {
        let mut f = well_formed_print();
        f.locals[0].repr = Repr::Tagged; // Const::Str must land in Heap(Str).
        assert!(matches!(
            verify(&f),
            Err(VerifyError::ConstStrReprMismatch { .. })
        ));
    }

    #[test]
    fn rejects_print_strobj_non_tagged_operand() {
        // Print(StrObj) directly off a Heap(Str) local — never coerced to Tagged.
        let f = single_block(
            vec![Repr::Heap(HeapShape::Str)],
            vec![MirInst::Print {
                kind: PrintKind::StrObj,
                arg: Some(Operand::Local(LocalId::new(0))),
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(
            verify(&f),
            Err(VerifyError::PrintStrObjNotTagged { .. })
        ));
    }

    #[test]
    fn rejects_newline_with_argument() {
        let f = single_block(
            vec![Repr::Tagged],
            vec![MirInst::Print {
                kind: PrintKind::Newline,
                arg: Some(Operand::Local(LocalId::new(0))),
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(
            verify(&f),
            Err(VerifyError::PrintUnexpectedArg { .. })
        ));
    }

    #[test]
    fn rejects_illegal_coercion() {
        // Tagged -> Heap(Str) is not in the legality table.
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
        assert!(matches!(verify(&f), Err(VerifyError::IllegalCoercion { .. })));
    }

    #[test]
    fn rejects_operand_out_of_range() {
        let f = single_block(
            vec![Repr::Tagged],
            vec![MirInst::Print {
                kind: PrintKind::StrObj,
                arg: Some(Operand::Local(LocalId::new(5))),
            }],
            MirTerminator::Return(None),
        );
        assert!(matches!(
            verify(&f),
            Err(VerifyError::LocalOutOfRange { .. })
        ));
    }
}
