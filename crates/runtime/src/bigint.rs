//! Arbitrary-precision integers — the one sanctioned runtime extension (A6).
//!
//! `int` is a tagged fixnum on the fast path and promotes to a heap [`BigIntObj`]
//! on overflow. This module owns the numeric tower used by the `rt_obj_*` ops:
//! it classifies operands into [`Num`] (fixnum / bignum / float), performs each
//! operation with CPython semantics (floor-division/modulo via
//! [`num_integer::Integer`]; int↔float promotion), and **demotes** any integer
//! result back to a tagged fixnum when it fits ([`make_int_value`]). Results that
//! do not fit are boxed; the `BigInt` they own is dropped in
//! `slab::finalize_object_by_tag`.

use std::cmp::Ordering;

use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{Pow, Signed, ToPrimitive, Zero};

use crate::exceptions::ExceptionType;
use crate::gc;
use crate::object::{BigIntObj, FloatObj, Obj, TypeTagKind};
use pyaot_core_defs::{int_fits, Value};

/// A classified numeric operand.
pub enum Num {
    Int(i64),
    Big(BigInt),
    Float(f64),
}

/// Classify an int-like / float operand, or `None` for anything else (str,
/// class instance, container, …) — the caller falls back to its own handling.
///
/// # Safety
/// `v`'s pointer (if any) must reference a live object.
pub unsafe fn classify_num(v: Value) -> Option<Num> {
    if v.is_int() {
        return Some(Num::Int(v.unwrap_int()));
    }
    if v.is_bool() {
        return Some(Num::Int(v.unwrap_bool() as i64));
    }
    if v.is_ptr() {
        let p = v.0 as *mut Obj;
        if !p.is_null() {
            match (*p).type_tag() {
                TypeTagKind::Float => return Some(Num::Float((*(p as *mut FloatObj)).value)),
                TypeTagKind::BigInt => {
                    return Some(Num::Big((*(p as *mut BigIntObj)).value.clone()))
                }
                _ => {}
            }
        }
    }
    None
}

fn to_f64(n: &Num) -> f64 {
    match n {
        Num::Int(i) => *i as f64,
        Num::Big(b) => b.to_f64().unwrap_or(f64::INFINITY),
        Num::Float(f) => *f,
    }
}

fn to_big(n: &Num) -> BigInt {
    match n {
        Num::Int(i) => BigInt::from(*i),
        Num::Big(b) => b.clone(),
        Num::Float(_) => unreachable!("to_big on a float operand"),
    }
}

fn is_float(n: &Num) -> bool {
    matches!(n, Num::Float(_))
}

/// Allocate a heap `BigIntObj` holding `value`.
pub fn rt_box_bigint(value: BigInt) -> *mut Obj {
    let size = std::mem::size_of::<BigIntObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::BigInt as u8);
    // The slot is zeroed by `gc_alloc`; `BigInt` is not valid all-zeros, so write
    // (do not assign, which would drop the bogus "old" value).
    unsafe {
        std::ptr::write(&mut (*(obj as *mut BigIntObj)).value, value);
    }
    obj
}

/// Box an integer `BigInt` as an `int` Value, demoting to a tagged fixnum when it
/// fits the 61-bit range (Python normalizes small results).
pub fn make_int_value(big: BigInt) -> *mut Obj {
    if let Some(i) = big.to_i64() {
        if int_fits(i) {
            return Value::from_int(i).0 as *mut Obj;
        }
    }
    rt_box_bigint(big)
}

// ── binary numeric ops (CPython semantics) ──────────────────────────────────

pub fn num_add(a: Num, b: Num) -> *mut Obj {
    if is_float(&a) || is_float(&b) {
        return crate::boxing::rt_box_float(to_f64(&a) + to_f64(&b));
    }
    if let (Num::Int(x), Num::Int(y)) = (&a, &b) {
        if let Some(v) = x.checked_add(*y) {
            if int_fits(v) {
                return Value::from_int(v).0 as *mut Obj;
            }
        }
    }
    make_int_value(to_big(&a) + to_big(&b))
}

pub fn num_sub(a: Num, b: Num) -> *mut Obj {
    if is_float(&a) || is_float(&b) {
        return crate::boxing::rt_box_float(to_f64(&a) - to_f64(&b));
    }
    if let (Num::Int(x), Num::Int(y)) = (&a, &b) {
        if let Some(v) = x.checked_sub(*y) {
            if int_fits(v) {
                return Value::from_int(v).0 as *mut Obj;
            }
        }
    }
    make_int_value(to_big(&a) - to_big(&b))
}

pub fn num_mul(a: Num, b: Num) -> *mut Obj {
    if is_float(&a) || is_float(&b) {
        return crate::boxing::rt_box_float(to_f64(&a) * to_f64(&b));
    }
    if let (Num::Int(x), Num::Int(y)) = (&a, &b) {
        if let Some(v) = x.checked_mul(*y) {
            if int_fits(v) {
                return Value::from_int(v).0 as *mut Obj;
            }
        }
    }
    make_int_value(to_big(&a) * to_big(&b))
}

/// `/` — Python 3 true division always yields a float.
pub fn num_truediv(a: Num, b: Num) -> *mut Obj {
    let fb = to_f64(&b);
    if fb == 0.0 {
        unsafe { crate::raise_exc!(ExceptionType::ZeroDivisionError, "division by zero") };
    }
    crate::boxing::rt_box_float(to_f64(&a) / fb)
}

pub fn num_floordiv(a: Num, b: Num) -> *mut Obj {
    if is_float(&a) || is_float(&b) {
        let fb = to_f64(&b);
        if fb == 0.0 {
            unsafe {
                crate::raise_exc!(
                    ExceptionType::ZeroDivisionError,
                    "float floor division by zero"
                )
            };
        }
        return crate::boxing::rt_box_float((to_f64(&a) / fb).floor());
    }
    let bb = to_big(&b);
    if bb.is_zero() {
        unsafe {
            crate::raise_exc!(
                ExceptionType::ZeroDivisionError,
                "integer division or modulo by zero"
            )
        };
    }
    // `div_floor` rounds toward -inf (CPython floor division).
    make_int_value(to_big(&a).div_floor(&bb))
}

pub fn num_mod(a: Num, b: Num) -> *mut Obj {
    if is_float(&a) || is_float(&b) {
        let fb = to_f64(&b);
        if fb == 0.0 {
            unsafe { crate::raise_exc!(ExceptionType::ZeroDivisionError, "float modulo") };
        }
        let fa = to_f64(&a);
        let mut r = fa % fb;
        if r != 0.0 && (r < 0.0) != (fb < 0.0) {
            r += fb;
        }
        return crate::boxing::rt_box_float(r);
    }
    let bb = to_big(&b);
    if bb.is_zero() {
        unsafe {
            crate::raise_exc!(
                ExceptionType::ZeroDivisionError,
                "integer division or modulo by zero"
            )
        };
    }
    // `mod_floor` gives a result with the divisor's sign (CPython modulo).
    make_int_value(to_big(&a).mod_floor(&bb))
}

pub fn num_pow(a: Num, b: Num) -> *mut Obj {
    if is_float(&a) || is_float(&b) {
        return crate::boxing::rt_box_float(to_f64(&a).powf(to_f64(&b)));
    }
    let exp = to_big(&b);
    if exp.is_negative() {
        // int ** negative int → float in Python.
        return crate::boxing::rt_box_float(to_f64(&a).powf(to_f64(&b)));
    }
    match exp.to_u32() {
        Some(e) => make_int_value(Pow::pow(to_big(&a), e)),
        None => unsafe { crate::raise_exc!(ExceptionType::OverflowError, "exponent too large") },
    }
}

// ── bitwise / shift (integer-only, bignum-aware) ─────────────────────────────
//
// Routed through the Tagged baseline (not raw `i64`) because an `int` operand may
// dynamically be a heap `BigInt`: unboxing a bignum pointer to a raw `i64` would
// be a silent miscompile (Invariant 2). A raw fast path is a future range-proven
// optimization, not the correct default.

fn bitwise_type_error(op: &str, a: &Num, b: &Num) -> ! {
    let name = |n: &Num| if is_float(n) { "float" } else { "int" };
    unsafe {
        crate::raise_exc!(
            ExceptionType::TypeError,
            "unsupported operand type(s) for {}: '{}' and '{}'",
            op,
            name(a),
            name(b)
        )
    }
}

pub fn num_bitand(a: Num, b: Num) -> *mut Obj {
    if is_float(&a) || is_float(&b) {
        bitwise_type_error("&", &a, &b);
    }
    if let (Num::Int(x), Num::Int(y)) = (&a, &b) {
        let v = x & y;
        if int_fits(v) {
            return Value::from_int(v).0 as *mut Obj;
        }
    }
    make_int_value(to_big(&a) & to_big(&b))
}

pub fn num_bitor(a: Num, b: Num) -> *mut Obj {
    if is_float(&a) || is_float(&b) {
        bitwise_type_error("|", &a, &b);
    }
    if let (Num::Int(x), Num::Int(y)) = (&a, &b) {
        let v = x | y;
        if int_fits(v) {
            return Value::from_int(v).0 as *mut Obj;
        }
    }
    make_int_value(to_big(&a) | to_big(&b))
}

pub fn num_bitxor(a: Num, b: Num) -> *mut Obj {
    if is_float(&a) || is_float(&b) {
        bitwise_type_error("^", &a, &b);
    }
    if let (Num::Int(x), Num::Int(y)) = (&a, &b) {
        let v = x ^ y;
        if int_fits(v) {
            return Value::from_int(v).0 as *mut Obj;
        }
    }
    make_int_value(to_big(&a) ^ to_big(&b))
}

pub fn num_shl(a: Num, b: Num) -> *mut Obj {
    if is_float(&a) || is_float(&b) {
        bitwise_type_error("<<", &a, &b);
    }
    let sb = to_big(&b);
    if sb.is_negative() {
        unsafe { crate::raise_exc!(ExceptionType::ValueError, "negative shift count") };
    }
    if let (Num::Int(x), Num::Int(s)) = (&a, &b) {
        if *s < 62 {
            if let Some(v) = x.checked_shl(*s as u32) {
                // `checked_shl` only rejects shift counts >= 64; it does NOT
                // detect significant bits shifted out of the 64-bit word. Guard
                // with a round-trip check so the fixnum fast path fires only
                // when no bits were lost — otherwise fall through to the bignum
                // path (never silently wrap; cf. A6).
                if int_fits(v) && (v >> *s) == *x {
                    return Value::from_int(v).0 as *mut Obj;
                }
            }
        }
    }
    let shift = match sb.to_usize() {
        Some(s) => s,
        None => unsafe { crate::raise_exc!(ExceptionType::OverflowError, "shift count too large") },
    };
    make_int_value(to_big(&a) << shift)
}

pub fn num_shr(a: Num, b: Num) -> *mut Obj {
    if is_float(&a) || is_float(&b) {
        bitwise_type_error(">>", &a, &b);
    }
    let sb = to_big(&b);
    if sb.is_negative() {
        unsafe { crate::raise_exc!(ExceptionType::ValueError, "negative shift count") };
    }
    // A shift past the value's width yields 0 (>=0) or -1 (<0); `BigInt >> usize`
    // already does this, so saturating an out-of-range shift is correct.
    let shift = sb.to_usize().unwrap_or(usize::MAX);
    make_int_value(to_big(&a) >> shift)
}

// ── unary ops ────────────────────────────────────────────────────────────────

pub fn num_neg(a: Num) -> *mut Obj {
    match a {
        Num::Int(i) => match i.checked_neg() {
            Some(v) if int_fits(v) => Value::from_int(v).0 as *mut Obj,
            _ => make_int_value(-BigInt::from(i)),
        },
        Num::Big(b) => make_int_value(-b),
        Num::Float(f) => crate::boxing::rt_box_float(-f),
    }
}

pub fn num_invert(a: Num) -> *mut Obj {
    match a {
        Num::Int(i) => make_int_value(!BigInt::from(i)),
        Num::Big(b) => make_int_value(!b),
        Num::Float(_) => unsafe {
            crate::raise_exc!(
                ExceptionType::TypeError,
                "bad operand type for unary ~: 'float'"
            )
        },
    }
}

// ── comparisons ──────────────────────────────────────────────────────────────

/// Compare two numerics (int/bignum exact, float-involved via `f64`). Returns
/// `None` only on NaN.
pub fn num_cmp(a: &Num, b: &Num) -> Option<Ordering> {
    if is_float(a) || is_float(b) {
        return to_f64(a).partial_cmp(&to_f64(b));
    }
    Some(to_big(a).cmp(&to_big(b)))
}

pub fn num_eq(a: &Num, b: &Num) -> bool {
    matches!(num_cmp(a, b), Some(Ordering::Equal))
}

/// `int(s)` / large literal: parse decimal text into an `int` Value.
///
/// # Safety
/// `ptr`..`ptr+len` must be a valid readable range.
#[export_name = "rt_bigint_from_str"]
pub unsafe extern "C" fn rt_bigint_from_str(ptr: *const u8, len: usize) -> Value {
    let bytes = std::slice::from_raw_parts(ptr, len);
    let text = std::str::from_utf8(bytes).unwrap_or("0").trim();
    let big = text.parse::<BigInt>().unwrap_or_else(|_| BigInt::zero());
    Value::from_ptr(make_int_value(big))
}
