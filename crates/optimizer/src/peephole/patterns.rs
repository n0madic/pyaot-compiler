//! Peephole pattern matchers and transformations

use pyaot_mir::{BinOp, Constant, Instruction, InstructionKind, Operand, RuntimeFunc, UnOp};

/// Simplify a single instruction in-place. Returns true if changed.
pub fn simplify_instruction(kind: &mut InstructionKind) -> bool {
    // Extract what we need before mutating to avoid borrow conflicts
    let replacement = match kind {
        InstructionKind::BinOp {
            dest,
            op,
            left,
            right,
        } => try_simplify_binop(*dest, *op, left, right),
        _ => None,
    };
    if let Some(r) = replacement {
        *kind = r;
        return true;
    }
    false
}

/// Simplify adjacent instruction pairs. Returns true if any changes were made.
///
/// Patterns detected:
/// - `BoxInt(x)` then `UnboxInt(box_result)` → replace unbox with `Copy { dest, src: x }`
/// - `FloatBits(x)` then `IntBitsToFloat(bits_result)` → replace second with `Copy`
/// - Double negation: `Neg(Neg(x))`, `Not(Not(x))`, `Invert(Invert(x))`
pub fn simplify_pairs(instructions: &mut [Instruction]) -> bool {
    let mut changed = false;
    // We can't easily remove instructions from a Vec while iterating,
    // so we replace the second instruction with a Copy instead.
    // DCE will clean up the now-unused first instruction.

    let len = instructions.len();
    if len < 2 {
        return false;
    }

    for i in 0..len - 1 {
        // Get both instructions immutably first to check the pattern
        let (first_kind, second_kind) = {
            let (first_slice, second_slice) = instructions.split_at(i + 1);
            (&first_slice[i].kind, &second_slice[0].kind)
        };

        if let Some(replacement) = match_pair_pattern(first_kind, second_kind) {
            instructions[i + 1].kind = replacement;
            changed = true;
        }
    }

    changed
}

/// Check if two adjacent instructions form a reducible pair.
/// Returns a replacement for the second instruction if so.
fn match_pair_pattern(
    first: &InstructionKind,
    second: &InstructionKind,
) -> Option<InstructionKind> {
    // Box/Unbox elimination: UnboxT(BoxT(x)) → Copy(x)
    if let (
        InstructionKind::RuntimeCall {
            dest: box_dest,
            func: box_func,
            args: box_args,
        },
        InstructionKind::RuntimeCall {
            dest: unbox_dest,
            func: unbox_func,
            args: unbox_args,
        },
    ) = (first, second)
    {
        // Check: unbox takes box's result as its sole argument
        if box_args.len() == 1
            && unbox_args.len() == 1
            && matches!(&unbox_args[0], Operand::Local(id) if *id == *box_dest)
            && is_matching_box_unbox(box_func, unbox_func)
        {
            return Some(InstructionKind::Copy {
                dest: *unbox_dest,
                src: box_args[0].clone(),
            });
        }
    }

    // FloatBits then IntBitsToFloat (or vice versa): roundtrip bitcast elimination
    if let (
        InstructionKind::FloatBits {
            dest: bits_dest,
            src: orig_src,
        },
        InstructionKind::IntBitsToFloat {
            dest: float_dest,
            src: bits_src,
        },
    ) = (first, second)
    {
        if matches!(bits_src, Operand::Local(id) if *id == *bits_dest) {
            return Some(InstructionKind::Copy {
                dest: *float_dest,
                src: orig_src.clone(),
            });
        }
    }
    if let (
        InstructionKind::IntBitsToFloat {
            dest: float_dest,
            src: orig_src,
        },
        InstructionKind::FloatBits {
            dest: bits_dest,
            src: float_src,
        },
    ) = (first, second)
    {
        if matches!(float_src, Operand::Local(id) if *id == *float_dest) {
            return Some(InstructionKind::Copy {
                dest: *bits_dest,
                src: orig_src.clone(),
            });
        }
    }

    // Double negation: UnOp(op, UnOp(op, x)) → Copy(x)
    if let (
        InstructionKind::UnOp {
            dest: first_dest,
            op: first_op,
            operand: orig_operand,
        },
        InstructionKind::UnOp {
            dest: second_dest,
            op: second_op,
            operand: inner_src,
        },
    ) = (first, second)
    {
        if first_op == second_op
            && matches!(inner_src, Operand::Local(id) if *id == *first_dest)
            && matches!(first_op, UnOp::Neg | UnOp::Not | UnOp::Invert)
        {
            return Some(InstructionKind::Copy {
                dest: *second_dest,
                src: orig_operand.clone(),
            });
        }
    }

    // BoolToInt then IntToFloat: keep as-is, not a simplification target
    // (these are semantically different types, can't shortcut)

    None
}

/// Check if a box/unbox RuntimeFunc pair cancel out.
fn is_matching_box_unbox(box_func: &RuntimeFunc, unbox_func: &RuntimeFunc) -> bool {
    use pyaot_core_defs::runtime_func_def::*;
    matches!(
        (box_func, unbox_func),
        (RuntimeFunc::Call(b), RuntimeFunc::Call(u))
            if (std::ptr::eq(*b, &RT_BOX_INT) && std::ptr::eq(*u, &RT_UNBOX_INT))
            || (std::ptr::eq(*b, &RT_BOX_FLOAT) && std::ptr::eq(*u, &RT_UNBOX_FLOAT))
            || (std::ptr::eq(*b, &RT_BOX_BOOL) && std::ptr::eq(*u, &RT_UNBOX_BOOL))
    )
}

// ==================== Single-instruction simplifications ====================

/// Try to simplify a BinOp. Returns replacement instruction if a pattern matches.
fn try_simplify_binop(
    dest: pyaot_utils::LocalId,
    op: BinOp,
    left: &Operand,
    right: &Operand,
) -> Option<InstructionKind> {
    // Patterns with right operand constant
    if let Operand::Constant(rc) = right {
        if let Some(r) = match_binop_right_const(dest, op, left, rc) {
            return Some(r);
        }
    }

    // Patterns with left operand constant
    if let Operand::Constant(lc) = left {
        if let Some(r) = match_binop_left_const(dest, op, lc, right) {
            return Some(r);
        }
    }

    // Same operand patterns: x - x → 0, x ^ x → 0 (only for locals, not constants)
    if let (Operand::Local(l), Operand::Local(r)) = (left, right) {
        if l == r {
            return match_binop_same_operand(dest, op);
        }
    }

    None
}

/// Match patterns where the RIGHT operand is a known constant.
fn match_binop_right_const(
    dest: pyaot_utils::LocalId,
    op: BinOp,
    left: &Operand,
    rc: &Constant,
) -> Option<InstructionKind> {
    match (op, rc) {
        // x + 0 → x, x - 0 → x, x | 0 → x, x ^ 0 → x, x << 0 → x, x >> 0 → x
        (
            BinOp::Add | BinOp::Sub | BinOp::BitOr | BinOp::BitXor | BinOp::LShift | BinOp::RShift,
            Constant::Int(0),
        ) => Some(InstructionKind::Copy {
            dest,
            src: left.clone(),
        }),
        // x + 0.0 → x, x - 0.0 → x
        (BinOp::Add | BinOp::Sub, Constant::Float(f)) if *f == 0.0 => Some(InstructionKind::Copy {
            dest,
            src: left.clone(),
        }),
        // x * 1 → x, x // 1 → x
        (BinOp::Mul | BinOp::FloorDiv, Constant::Int(1)) => Some(InstructionKind::Copy {
            dest,
            src: left.clone(),
        }),
        // x * 1.0 → x, x / 1.0 → x
        (BinOp::Mul | BinOp::Div, Constant::Float(f)) if *f == 1.0 => Some(InstructionKind::Copy {
            dest,
            src: left.clone(),
        }),
        // x * 0 → 0
        // Safety: replacing x * 0 -> 0 is safe because MIR operands are locals/constants,
        // and DCE preserves the instruction that defined the local if it has side effects
        // (RuntimeCalls are not considered pure by DCE).
        (BinOp::Mul, Constant::Int(0)) => Some(InstructionKind::Const {
            dest,
            value: Constant::Int(0),
        }),
        // x & 0 → 0
        (BinOp::BitAnd, Constant::Int(0)) => Some(InstructionKind::Const {
            dest,
            value: Constant::Int(0),
        }),
        // x & -1 → x (all bits set)
        (BinOp::BitAnd, Constant::Int(-1)) => Some(InstructionKind::Copy {
            dest,
            src: left.clone(),
        }),
        // x | -1 → -1 (all bits set)
        (BinOp::BitOr, Constant::Int(-1)) => Some(InstructionKind::Const {
            dest,
            value: Constant::Int(-1),
        }),
        // Strength reduction: x * 2 → x + x
        (BinOp::Mul, Constant::Int(2)) => Some(InstructionKind::BinOp {
            dest,
            op: BinOp::Add,
            left: left.clone(),
            right: left.clone(),
        }),
        // Strength reduction: x * 2^n → x << n (for small positive powers of 2)
        (BinOp::Mul, Constant::Int(n)) if *n > 2 && (*n as u64).is_power_of_two() => {
            let shift = n.trailing_zeros() as i64;
            Some(InstructionKind::BinOp {
                dest,
                op: BinOp::LShift,
                left: left.clone(),
                right: Operand::Constant(Constant::Int(shift)),
            })
        }
        // Strength reduction: x // 2^n → x >> n (only for positive divisors, Python floor division)
        // NOTE: only valid when x >= 0. For negative x, Python floor div rounds down
        // while right shift rounds toward negative infinity — they happen to agree!
        // Python: -7 // 4 = -2, Rust: -7 >> 2 = -2 (arithmetic shift). Safe.
        (BinOp::FloorDiv, Constant::Int(n)) if *n > 1 && (*n as u64).is_power_of_two() => {
            let shift = n.trailing_zeros() as i64;
            Some(InstructionKind::BinOp {
                dest,
                op: BinOp::RShift,
                left: left.clone(),
                right: Operand::Constant(Constant::Int(shift)),
            })
        }
        // x ** 0 → 1 (Python: anything ** 0 == 1, including 0 ** 0)
        (BinOp::Pow, Constant::Int(0)) => Some(InstructionKind::Const {
            dest,
            value: Constant::Int(1),
        }),
        // x ** 1 → x
        (BinOp::Pow, Constant::Int(1)) => Some(InstructionKind::Copy {
            dest,
            src: left.clone(),
        }),
        // x ** 2 → x * x
        (BinOp::Pow, Constant::Int(2)) => Some(InstructionKind::BinOp {
            dest,
            op: BinOp::Mul,
            left: left.clone(),
            right: left.clone(),
        }),
        _ => None,
    }
}

/// Match patterns where the LEFT operand is a known constant.
fn match_binop_left_const(
    dest: pyaot_utils::LocalId,
    op: BinOp,
    lc: &Constant,
    right: &Operand,
) -> Option<InstructionKind> {
    match (op, lc) {
        // 0 + x → x, 0 | x → x, 0 ^ x → x
        (BinOp::Add | BinOp::BitOr | BinOp::BitXor, Constant::Int(0)) => {
            Some(InstructionKind::Copy {
                dest,
                src: right.clone(),
            })
        }
        // 0.0 + x → x
        (BinOp::Add, Constant::Float(f)) if *f == 0.0 => Some(InstructionKind::Copy {
            dest,
            src: right.clone(),
        }),
        // 1 * x → x
        (BinOp::Mul, Constant::Int(1)) => Some(InstructionKind::Copy {
            dest,
            src: right.clone(),
        }),
        // 1.0 * x → x
        (BinOp::Mul, Constant::Float(f)) if *f == 1.0 => Some(InstructionKind::Copy {
            dest,
            src: right.clone(),
        }),
        // 0 * x → 0
        // Safety: replacing 0 * x -> 0 is safe because MIR operands are locals/constants,
        // and DCE preserves the instruction that defined the local if it has side effects
        // (RuntimeCalls are not considered pure by DCE).
        (BinOp::Mul, Constant::Int(0)) => Some(InstructionKind::Const {
            dest,
            value: Constant::Int(0),
        }),
        // 0 & x → 0
        (BinOp::BitAnd, Constant::Int(0)) => Some(InstructionKind::Const {
            dest,
            value: Constant::Int(0),
        }),
        // -1 & x → x
        (BinOp::BitAnd, Constant::Int(-1)) => Some(InstructionKind::Copy {
            dest,
            src: right.clone(),
        }),
        // -1 | x → -1
        (BinOp::BitOr, Constant::Int(-1)) => Some(InstructionKind::Const {
            dest,
            value: Constant::Int(-1),
        }),
        // 2 * x → x + x (strength reduction, commutative)
        (BinOp::Mul, Constant::Int(2)) => Some(InstructionKind::BinOp {
            dest,
            op: BinOp::Add,
            left: right.clone(),
            right: right.clone(),
        }),
        // 2^n * x → x << n (strength reduction, commutative)
        (BinOp::Mul, Constant::Int(n)) if *n > 2 && (*n as u64).is_power_of_two() => {
            let shift = n.trailing_zeros() as i64;
            Some(InstructionKind::BinOp {
                dest,
                op: BinOp::LShift,
                left: right.clone(),
                right: Operand::Constant(Constant::Int(shift)),
            })
        }
        _ => None,
    }
}

/// Match patterns where both operands are the same local: x op x
fn match_binop_same_operand(dest: pyaot_utils::LocalId, op: BinOp) -> Option<InstructionKind> {
    match op {
        // x - x → 0, x ^ x → 0
        BinOp::Sub | BinOp::BitXor => Some(InstructionKind::Const {
            dest,
            value: Constant::Int(0),
        }),
        _ => None,
    }
}
