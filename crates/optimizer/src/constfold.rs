//! Constant folding: a block-local forward scan over [`ConstLattice`].
//!
//! The fold table is a STRICT ALLOWLIST. What folds:
//! * `BinOp{Add,Sub,Mul}` on two `Const::Int`s into a `Tagged` dst — iff the
//!   `checked_*` arithmetic succeeds AND the result still fits the runtime's
//!   fixnum range (folding a bignum promotion would change the value's
//!   dynamic type at a `Const::Int`, a silent miscompile);
//!   `BitAnd`/`BitOr`/`BitXor` likewise (they cannot overflow).
//! * float `Add`/`Sub`/`Mul` on two `Raw(F64)` constants — exact IEEE,
//!   the same bits the runtime would compute.
//! * `Compare` of two constants → `Const::Int(0|1)` into the `Raw(I8)` dst
//!   (the verifier explicitly admits `Const::Int` at `Raw(I8)`).
//! * `Truthy` / `Unary::Not` of a constant → `Const::Int(0|1)`.
//! * `Branch` on a known `Raw(I8)` constant → `Jump` (DCE then blanks the
//!   dead arm).
//!
//! What NEVER folds: `Div`/`FloorDiv`/`Mod`/`Pow`/`Shl`/`Shr` (they raise —
//! `1 // 0` must still raise at runtime; the corpus depends on it) and any
//! float division (IEEE special cases stay at runtime).

use pyaot_core_defs::int_fits;
use pyaot_mir::{BinOp, CmpOp, Const, MirFunction, MirInst, MirProgram, MirTerminator, UnaryOp};
use pyaot_types::{RawKind, Repr};

use crate::analysis::ConstLattice;
use crate::OptimizationPass;

pub struct ConstFold;

impl OptimizationPass for ConstFold {
    fn name(&self) -> &'static str {
        "constfold"
    }

    fn run(&self, program: &mut MirProgram) {
        for func in &mut program.funcs {
            run_func(func);
        }
    }
}

fn run_func(f: &mut MirFunction) {
    for block in &mut f.blocks {
        let mut env = ConstLattice::new();
        for inst in &mut block.insts {
            if let Some(folded) = fold_inst(inst, &env, &f.locals) {
                *inst = folded;
            }
            // Update the lattice AFTER the (possibly rewritten) instruction:
            // a Const records its value; any other write kills its dst.
            match inst {
                MirInst::Const { dst, val } => env.set(*dst, val.clone()),
                other => {
                    if let Some(dst) = other.dst() {
                        env.kill(dst);
                    }
                }
            }
        }
        // Branch on a known I8 constant → Jump.
        if let MirTerminator::Branch { cond, then, else_ } = &block.term {
            if let Some(Const::Int(v)) = env.get_operand(cond) {
                let target = if *v != 0 { *then } else { *else_ };
                block.term = MirTerminator::Jump(target);
            }
        }
    }
}

/// The fold table. Returns the replacement instruction, or `None` to keep.
fn fold_inst(
    inst: &MirInst,
    env: &ConstLattice,
    locals: &[pyaot_mir::LocalDecl],
) -> Option<MirInst> {
    match inst {
        MirInst::BinOp { dst, op, l, r } => {
            let lc = env.get_operand(l)?;
            let rc = env.get_operand(r)?;
            match (lc, rc) {
                (Const::Int(a), Const::Int(b)) if locals[dst.index()].repr == Repr::Tagged => {
                    let v = match op {
                        BinOp::Add => a.checked_add(*b)?,
                        BinOp::Sub => a.checked_sub(*b)?,
                        BinOp::Mul => a.checked_mul(*b)?,
                        BinOp::BitAnd => a & b,
                        BinOp::BitOr => a | b,
                        BinOp::BitXor => a ^ b,
                        // Raising ops never fold.
                        BinOp::Div
                        | BinOp::FloorDiv
                        | BinOp::Mod
                        | BinOp::Pow
                        | BinOp::Shl
                        | BinOp::Shr => return None,
                    };
                    if !int_fits(v) {
                        return None; // would promote to BigInt at runtime
                    }
                    Some(MirInst::Const {
                        dst: *dst,
                        val: Const::Int(v),
                    })
                }
                (Const::Float(a), Const::Float(b))
                    if locals[dst.index()].repr == Repr::Raw(RawKind::F64) =>
                {
                    let v = match op {
                        BinOp::Add => a + b,
                        BinOp::Sub => a - b,
                        BinOp::Mul => a * b,
                        _ => return None, // float Div & friends stay at runtime
                    };
                    Some(MirInst::Const {
                        dst: *dst,
                        val: Const::Float(v),
                    })
                }
                _ => None,
            }
        }
        MirInst::Compare { dst, op, l, r } => {
            if locals[dst.index()].repr != Repr::Raw(RawKind::I8) {
                return None;
            }
            let lc = env.get_operand(l)?;
            let rc = env.get_operand(r)?;
            let res = match (lc, rc) {
                (Const::Int(a), Const::Int(b)) => cmp(op, a, b),
                (Const::Float(a), Const::Float(b)) => cmp_f(op, *a, *b),
                (Const::Bool(a), Const::Bool(b)) => cmp(op, &(*a as i64), &(*b as i64)),
                _ => return None,
            };
            Some(MirInst::Const {
                dst: *dst,
                val: Const::Int(res as i64),
            })
        }
        MirInst::Truthy { dst, operand } => {
            let truth = const_truthiness(env.get_operand(operand)?)?;
            Some(MirInst::Const {
                dst: *dst,
                val: Const::Int(truth as i64),
            })
        }
        MirInst::Unary {
            dst,
            op: UnaryOp::Not,
            operand,
        } => {
            if locals[dst.index()].repr != Repr::Raw(RawKind::I8) {
                return None;
            }
            let truth = const_truthiness(env.get_operand(operand)?)?;
            Some(MirInst::Const {
                dst: *dst,
                val: Const::Int(!truth as i64),
            })
        }
        _ => None,
    }
}

fn cmp(op: &CmpOp, a: &i64, b: &i64) -> bool {
    match op {
        CmpOp::Eq => a == b,
        CmpOp::NotEq => a != b,
        CmpOp::Lt => a < b,
        CmpOp::LtE => a <= b,
        CmpOp::Gt => a > b,
        CmpOp::GtE => a >= b,
    }
}

fn cmp_f(op: &CmpOp, a: f64, b: f64) -> bool {
    match op {
        CmpOp::Eq => a == b,
        CmpOp::NotEq => a != b,
        CmpOp::Lt => a < b,
        CmpOp::LtE => a <= b,
        CmpOp::Gt => a > b,
        CmpOp::GtE => a >= b,
    }
}

/// Python truthiness of a scalar constant. Heap constants (Str/Bytes/BigInt)
/// are left alone — their truthiness is a length check the runtime owns.
fn const_truthiness(c: &Const) -> Option<bool> {
    match c {
        Const::Int(v) => Some(*v != 0),
        Const::Float(v) => Some(*v != 0.0),
        Const::Bool(b) => Some(*b),
        Const::None => Some(false),
        Const::Str(_) | Const::Bytes(_) | Const::BigIntStr(_) | Const::NullPtr => None,
    }
}

#[cfg(test)]
mod tests {
    use pyaot_mir::{BinOp, CmpOp, Const, MirInst, MirTerminator, UnaryOp};
    use pyaot_types::{RawKind, Repr};
    use pyaot_utils::BlockId;

    use super::run_func;
    use crate::testutil::{function, l, op, single_block, verify_ok};

    fn tagged_int_binop(op_kind: BinOp, a: i64, b: i64) -> pyaot_mir::MirFunction {
        single_block(
            vec![Repr::Tagged, Repr::Tagged, Repr::Tagged],
            vec![
                MirInst::Const {
                    dst: l(0),
                    val: Const::Int(a),
                },
                MirInst::Const {
                    dst: l(1),
                    val: Const::Int(b),
                },
                MirInst::BinOp {
                    dst: l(2),
                    op: op_kind,
                    l: op(0),
                    r: op(1),
                },
            ],
            MirTerminator::Return(Some(op(2))),
        )
    }

    #[test]
    fn folds_tagged_int_add() {
        let mut f = tagged_int_binop(BinOp::Add, 2, 3);
        run_func(&mut f);
        assert!(
            matches!(
                f.blocks[0].insts[2],
                MirInst::Const {
                    val: Const::Int(5),
                    ..
                }
            ),
            "2 + 3 must fold to Const::Int(5), got {:?}",
            f.blocks[0].insts[2]
        );
        verify_ok(&f);
    }

    #[test]
    fn fixnum_overflow_does_not_fold() {
        // (1<<60)-1 + 1 escapes the 61-bit fixnum payload — runtime would
        // promote to BigInt, so the BinOp must survive.
        let mut f = tagged_int_binop(BinOp::Add, (1 << 60) - 1, 1);
        run_func(&mut f);
        assert!(matches!(f.blocks[0].insts[2], MirInst::BinOp { .. }));
        verify_ok(&f);
    }

    #[test]
    fn raising_ops_never_fold() {
        // 1 // 0 must keep raising ZeroDivisionError at runtime.
        for op_kind in [
            BinOp::Div,
            BinOp::FloorDiv,
            BinOp::Mod,
            BinOp::Pow,
            BinOp::Shl,
            BinOp::Shr,
        ] {
            let mut f = tagged_int_binop(op_kind, 1, 0);
            run_func(&mut f);
            assert!(
                matches!(f.blocks[0].insts[2], MirInst::BinOp { .. }),
                "{op_kind:?} must not fold"
            );
        }
    }

    #[test]
    fn folds_raw_float_mul() {
        let mut f = single_block(
            vec![
                Repr::Raw(RawKind::F64),
                Repr::Raw(RawKind::F64),
                Repr::Raw(RawKind::F64),
            ],
            vec![
                MirInst::Const {
                    dst: l(0),
                    val: Const::Float(1.5),
                },
                MirInst::Const {
                    dst: l(1),
                    val: Const::Float(2.0),
                },
                MirInst::BinOp {
                    dst: l(2),
                    op: BinOp::Mul,
                    l: op(0),
                    r: op(1),
                },
            ],
            MirTerminator::Return(None),
        );
        run_func(&mut f);
        assert!(
            matches!(f.blocks[0].insts[2], MirInst::Const { val: Const::Float(v), .. } if v == 3.0)
        );
        verify_ok(&f);
    }

    #[test]
    fn folds_compare_and_branch_to_jump() {
        let mut f = function(
            vec![Repr::Tagged, Repr::Tagged, Repr::Raw(RawKind::I8)],
            vec![
                (
                    vec![
                        MirInst::Const {
                            dst: l(0),
                            val: Const::Int(1),
                        },
                        MirInst::Const {
                            dst: l(1),
                            val: Const::Int(2),
                        },
                        MirInst::Compare {
                            dst: l(2),
                            op: CmpOp::Lt,
                            l: op(0),
                            r: op(1),
                        },
                    ],
                    MirTerminator::Branch {
                        cond: op(2),
                        then: BlockId::new(1),
                        else_: BlockId::new(2),
                    },
                ),
                (vec![], MirTerminator::Return(None)),
                (vec![], MirTerminator::Return(None)),
            ],
        );
        run_func(&mut f);
        assert!(matches!(
            f.blocks[0].insts[2],
            MirInst::Const {
                val: Const::Int(1),
                ..
            }
        ));
        assert!(
            matches!(f.blocks[0].term, MirTerminator::Jump(t) if t == BlockId::new(1)),
            "1 < 2 branch must become Jump(then)"
        );
        verify_ok(&f);
    }

    #[test]
    fn folds_truthy_and_not() {
        let mut f = single_block(
            vec![Repr::Tagged, Repr::Raw(RawKind::I8), Repr::Raw(RawKind::I8)],
            vec![
                MirInst::Const {
                    dst: l(0),
                    val: Const::Int(0),
                },
                MirInst::Truthy {
                    dst: l(1),
                    operand: op(0),
                },
                MirInst::Unary {
                    dst: l(2),
                    op: UnaryOp::Not,
                    operand: op(0),
                },
            ],
            MirTerminator::Return(None),
        );
        run_func(&mut f);
        assert!(matches!(
            f.blocks[0].insts[1],
            MirInst::Const {
                val: Const::Int(0),
                ..
            }
        ));
        assert!(matches!(
            f.blocks[0].insts[2],
            MirInst::Const {
                val: Const::Int(1),
                ..
            }
        ));
        verify_ok(&f);
    }

    #[test]
    fn redefinition_kills_the_constant() {
        // l0 = 1; l0 = <runtime value>; l2 = l0 + 1 must NOT fold.
        let mut f = single_block(
            vec![Repr::Tagged, Repr::Tagged, Repr::Tagged],
            vec![
                MirInst::Const {
                    dst: l(0),
                    val: Const::Int(1),
                },
                // Overwrite l0 with a runtime value (Div never folds).
                MirInst::BinOp {
                    dst: l(0),
                    op: BinOp::Div,
                    l: op(0),
                    r: op(0),
                },
                MirInst::Const {
                    dst: l(1),
                    val: Const::Int(1),
                },
                MirInst::BinOp {
                    dst: l(2),
                    op: BinOp::Add,
                    l: op(0),
                    r: op(1),
                },
            ],
            MirTerminator::Return(Some(op(2))),
        );
        run_func(&mut f);
        assert!(
            matches!(f.blocks[0].insts[3], MirInst::BinOp { .. }),
            "use of a killed constant must not fold"
        );
        verify_ok(&f);
    }
}
