//! Constant expression evaluation
//!
//! Evaluates binary and unary operations on constant operands at compile time.
//! Returns `None` when folding is unsafe (overflow, division by zero, etc.).

use pyaot_mir::{BinOp, Constant, UnOp};
use pyaot_utils::StringInterner;

/// Try to fold a binary operation on two constants.
/// Returns `None` if the operation cannot be safely evaluated at compile time.
pub fn try_fold_binop(
    op: BinOp,
    left: &Constant,
    right: &Constant,
    interner: &mut StringInterner,
) -> Option<Constant> {
    match (left, right) {
        (Constant::Int(a), Constant::Int(b)) => fold_int_binop(op, *a, *b),
        (Constant::Float(a), Constant::Float(b)) => fold_float_binop(op, *a, *b),
        (Constant::Bool(a), Constant::Bool(b)) => fold_bool_binop(op, *a, *b),
        (Constant::Str(a), Constant::Str(b)) => fold_str_binop(op, *a, *b, interner),
        _ => None,
    }
}

/// Try to fold a unary operation on a constant.
pub fn try_fold_unop(op: UnOp, operand: &Constant) -> Option<Constant> {
    match operand {
        Constant::Int(v) => fold_int_unop(op, *v),
        Constant::Float(v) => fold_float_unop(op, *v),
        Constant::Bool(v) => fold_bool_unop(op, *v),
        _ => None,
    }
}

/// Try to fold a type conversion on a constant.
pub fn try_fold_bool_to_int(val: &Constant) -> Option<Constant> {
    match val {
        Constant::Bool(b) => Some(Constant::Int(if *b { 1 } else { 0 })),
        _ => None,
    }
}

pub fn try_fold_int_to_float(val: &Constant) -> Option<Constant> {
    match val {
        Constant::Int(n) => Some(Constant::Float(*n as f64)),
        _ => None,
    }
}

pub fn try_fold_float_to_int(val: &Constant) -> Option<Constant> {
    match val {
        Constant::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                None // Would trap at runtime
            } else {
                let truncated = f.trunc();
                if truncated < i64::MIN as f64 || truncated > i64::MAX as f64 {
                    None // Overflow
                } else {
                    Some(Constant::Int(truncated as i64))
                }
            }
        }
        _ => None,
    }
}

pub fn try_fold_float_abs(val: &Constant) -> Option<Constant> {
    match val {
        Constant::Float(f) => Some(Constant::Float(f.abs())),
        _ => None,
    }
}

// ==================== Integer operations ====================

fn fold_int_binop(op: BinOp, a: i64, b: i64) -> Option<Constant> {
    match op {
        BinOp::Add => a.checked_add(b).map(Constant::Int),
        BinOp::Sub => a.checked_sub(b).map(Constant::Int),
        BinOp::Mul => a.checked_mul(b).map(Constant::Int),
        BinOp::FloorDiv => {
            if b == 0 {
                return None; // ZeroDivisionError
            }
            // Special case: i64::MIN // -1 overflows
            if a == i64::MIN && b == -1 {
                return None;
            }
            Some(Constant::Int(python_floordiv(a, b)))
        }
        BinOp::Mod => {
            if b == 0 {
                return None; // ZeroDivisionError
            }
            if a == i64::MIN && b == -1 {
                return None;
            }
            Some(Constant::Int(python_mod(a, b)))
        }
        BinOp::Pow => python_pow(a, b).map(Constant::Int),
        BinOp::Div => {
            // Python int / int → float
            if b == 0 {
                return None;
            }
            Some(Constant::Float(a as f64 / b as f64))
        }

        // Comparisons
        BinOp::Eq => Some(Constant::Bool(a == b)),
        BinOp::NotEq => Some(Constant::Bool(a != b)),
        BinOp::Lt => Some(Constant::Bool(a < b)),
        BinOp::LtE => Some(Constant::Bool(a <= b)),
        BinOp::Gt => Some(Constant::Bool(a > b)),
        BinOp::GtE => Some(Constant::Bool(a >= b)),

        // Bitwise
        BinOp::BitAnd => Some(Constant::Int(a & b)),
        BinOp::BitOr => Some(Constant::Int(a | b)),
        BinOp::BitXor => Some(Constant::Int(a ^ b)),
        BinOp::LShift => {
            if !(0..64).contains(&b) {
                None // Negative or too-large shift
            } else {
                Some(Constant::Int(a << b))
            }
        }
        BinOp::RShift => {
            if !(0..64).contains(&b) {
                None
            } else {
                Some(Constant::Int(a >> b))
            }
        }

        // Logical ops on ints: not applicable at MIR level for ints
        BinOp::And | BinOp::Or => None,
    }
}

fn fold_int_unop(op: UnOp, v: i64) -> Option<Constant> {
    match op {
        UnOp::Neg => v.checked_neg().map(Constant::Int),
        UnOp::Not => Some(Constant::Bool(v == 0)),
        UnOp::Invert => Some(Constant::Int(!v)),
    }
}

// ==================== Float operations ====================

fn fold_float_binop(op: BinOp, a: f64, b: f64) -> Option<Constant> {
    match op {
        BinOp::Add => Some(Constant::Float(a + b)),
        BinOp::Sub => Some(Constant::Float(a - b)),
        BinOp::Mul => Some(Constant::Float(a * b)),
        BinOp::Div => {
            if b == 0.0 {
                None // ZeroDivisionError
            } else {
                Some(Constant::Float(a / b))
            }
        }
        BinOp::FloorDiv => {
            if b == 0.0 {
                None
            } else {
                Some(Constant::Float((a / b).floor()))
            }
        }
        BinOp::Mod => {
            if b == 0.0 {
                None
            } else {
                // Python float mod: result has sign of divisor
                let r = a % b;
                let result = if r != 0.0 && r.is_sign_negative() != b.is_sign_negative() {
                    r + b
                } else {
                    r
                };
                Some(Constant::Float(result))
            }
        }
        BinOp::Pow => Some(Constant::Float(a.powf(b))),

        // Comparisons
        BinOp::Eq => Some(Constant::Bool(a == b)),
        BinOp::NotEq => Some(Constant::Bool(a != b)),
        BinOp::Lt => Some(Constant::Bool(a < b)),
        BinOp::LtE => Some(Constant::Bool(a <= b)),
        BinOp::Gt => Some(Constant::Bool(a > b)),
        BinOp::GtE => Some(Constant::Bool(a >= b)),

        _ => None,
    }
}

fn fold_float_unop(op: UnOp, v: f64) -> Option<Constant> {
    match op {
        UnOp::Neg => Some(Constant::Float(-v)),
        UnOp::Not => Some(Constant::Bool(v == 0.0)),
        _ => None,
    }
}

// ==================== Boolean operations ====================

fn fold_bool_binop(op: BinOp, a: bool, b: bool) -> Option<Constant> {
    match op {
        BinOp::And => Some(Constant::Bool(a && b)),
        BinOp::Or => Some(Constant::Bool(a || b)),
        BinOp::Eq => Some(Constant::Bool(a == b)),
        BinOp::NotEq => Some(Constant::Bool(a != b)),
        _ => None,
    }
}

fn fold_bool_unop(op: UnOp, v: bool) -> Option<Constant> {
    match op {
        UnOp::Not => Some(Constant::Bool(!v)),
        _ => None,
    }
}

// ==================== String operations ====================

fn fold_str_binop(
    op: BinOp,
    a: pyaot_utils::InternedString,
    b: pyaot_utils::InternedString,
    interner: &mut StringInterner,
) -> Option<Constant> {
    match op {
        BinOp::Add => {
            let a_str = interner.resolve(a);
            let b_str = interner.resolve(b);
            let concatenated = format!("{}{}", a_str, b_str);
            let interned = interner.intern(&concatenated);
            Some(Constant::Str(interned))
        }
        BinOp::Eq => {
            // Same interned string ID means equal
            Some(Constant::Bool(a == b))
        }
        BinOp::NotEq => Some(Constant::Bool(a != b)),
        _ => None,
    }
}

// ==================== Python-compatible arithmetic helpers ====================

/// Python floor division: rounds toward negative infinity
fn python_floordiv(a: i64, b: i64) -> i64 {
    let d = a / b;
    let r = a % b;
    if r != 0 && ((r ^ b) < 0) {
        d - 1
    } else {
        d
    }
}

/// Python modulo: result has sign of divisor
fn python_mod(a: i64, b: i64) -> i64 {
    let r = a % b;
    if r != 0 && ((r ^ b) < 0) {
        r + b
    } else {
        r
    }
}

/// Python integer power with overflow checking.
/// Returns None for negative exponents (would produce float) or overflow.
fn python_pow(base: i64, exp: i64) -> Option<i64> {
    if exp < 0 {
        return None; // Would produce float in Python
    }
    let mut result: i64 = 1;
    let mut b = base;
    let mut e = exp as u64;
    while e > 0 {
        if e & 1 == 1 {
            result = result.checked_mul(b)?;
        }
        e >>= 1;
        if e > 0 {
            b = b.checked_mul(b)?;
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_int_add() {
        assert_eq!(fold_int_binop(BinOp::Add, 2, 3), Some(Constant::Int(5)));
    }

    #[test]
    fn test_int_add_overflow() {
        assert_eq!(fold_int_binop(BinOp::Add, i64::MAX, 1), None);
    }

    #[test]
    fn test_int_floordiv() {
        assert_eq!(
            fold_int_binop(BinOp::FloorDiv, 7, 2),
            Some(Constant::Int(3))
        );
        assert_eq!(
            fold_int_binop(BinOp::FloorDiv, -7, 2),
            Some(Constant::Int(-4))
        );
        assert_eq!(
            fold_int_binop(BinOp::FloorDiv, 7, -2),
            Some(Constant::Int(-4))
        );
    }

    #[test]
    fn test_int_mod() {
        assert_eq!(fold_int_binop(BinOp::Mod, -7, 2), Some(Constant::Int(1)));
        assert_eq!(fold_int_binop(BinOp::Mod, 7, -2), Some(Constant::Int(-1)));
    }

    #[test]
    fn test_div_by_zero() {
        assert_eq!(fold_int_binop(BinOp::FloorDiv, 5, 0), None);
        assert_eq!(fold_int_binop(BinOp::Mod, 5, 0), None);
        assert_eq!(fold_int_binop(BinOp::Div, 5, 0), None);
    }

    #[test]
    fn test_int_pow() {
        assert_eq!(python_pow(2, 10), Some(1024));
        assert_eq!(python_pow(2, -1), None);
        assert_eq!(python_pow(i64::MAX, 2), None); // Overflow
    }

    #[test]
    fn test_float_ops() {
        assert_eq!(
            fold_float_binop(BinOp::Add, 1.5, 2.5),
            Some(Constant::Float(4.0))
        );
        assert_eq!(fold_float_binop(BinOp::Div, 1.0, 0.0), None);
    }

    #[test]
    fn test_bool_ops() {
        assert_eq!(
            fold_bool_binop(BinOp::And, true, false),
            Some(Constant::Bool(false))
        );
        assert_eq!(
            fold_bool_binop(BinOp::Or, false, true),
            Some(Constant::Bool(true))
        );
    }

    #[test]
    fn test_unop_neg() {
        assert_eq!(fold_int_unop(UnOp::Neg, 5), Some(Constant::Int(-5)));
        assert_eq!(fold_int_unop(UnOp::Neg, i64::MIN), None);
    }

    #[test]
    fn test_unop_invert() {
        assert_eq!(fold_int_unop(UnOp::Invert, 0), Some(Constant::Int(-1)));
    }

    #[test]
    fn test_conversions() {
        assert_eq!(
            try_fold_bool_to_int(&Constant::Bool(true)),
            Some(Constant::Int(1))
        );
        assert_eq!(
            try_fold_int_to_float(&Constant::Int(42)),
            Some(Constant::Float(42.0))
        );
        assert_eq!(
            try_fold_float_to_int(&Constant::Float(3.7)),
            Some(Constant::Int(3))
        );
        assert_eq!(try_fold_float_to_int(&Constant::Float(f64::NAN)), None);
        assert_eq!(
            try_fold_float_abs(&Constant::Float(-3.0)),
            Some(Constant::Float(3.0))
        );
    }

    #[test]
    fn test_str_concat() {
        let mut interner = StringInterner::new();
        let a = interner.intern("hello");
        let b = interner.intern(" world");
        let result = fold_str_binop(BinOp::Add, a, b, &mut interner);
        match result {
            Some(Constant::Str(s)) => {
                assert_eq!(interner.resolve(s), "hello world");
            }
            other => panic!("Expected Str constant, got {:?}", other),
        }
    }

    #[test]
    fn test_int_div_produces_float() {
        assert_eq!(fold_int_binop(BinOp::Div, 5, 2), Some(Constant::Float(2.5)));
    }

    #[test]
    fn test_shift_bounds() {
        assert_eq!(fold_int_binop(BinOp::LShift, 1, -1), None);
        assert_eq!(fold_int_binop(BinOp::LShift, 1, 64), None);
        assert_eq!(fold_int_binop(BinOp::LShift, 1, 3), Some(Constant::Int(8)));
    }
}
