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

/// Convert an operand to `float` for a mixed int/float operation. Like
/// CPython, an int too large for a double raises `OverflowError` instead of
/// silently becoming `inf`.
fn to_f64(n: &Num) -> f64 {
    match n {
        Num::Int(i) => *i as f64,
        Num::Big(b) => bigint_to_f64(b),
        Num::Float(f) => *f,
    }
}

/// `float(n)` for a bignum: correctly rounded (via [`int_true_divide`] by
/// one); `OverflowError` beyond the double range, as CPython.
pub fn bigint_to_f64(b: &BigInt) -> f64 {
    let f = int_true_divide(b, &BigInt::from(1));
    if f.is_infinite() {
        unsafe {
            crate::raise_exc!(
                ExceptionType::OverflowError,
                "int too large to convert to float"
            )
        }
    }
    f
}

/// `int / int` correctly rounded (CPython's `long_true_divide`): exact when
/// both fit in 53 bits, else an exact big-integer quotient with >= 55
/// significant bits plus a sticky bit, rounded once to a double.
fn int_true_divide(a: &BigInt, b: &BigInt) -> f64 {
    const DBL_MANT_DIG: i64 = 53;
    if let (Some(x), Some(y)) = (a.to_i64(), b.to_i64()) {
        if x.unsigned_abs() < (1u64 << 53) && y.unsigned_abs() < (1u64 << 53) {
            return x as f64 / y as f64;
        }
    }
    let negative = a.is_negative() != b.is_negative();
    let (na, nb) = (a.abs(), b.abs());
    if na.is_zero() {
        return if negative { -0.0 } else { 0.0 };
    }
    let diff = na.bits() as i64 - nb.bits() as i64;
    // Scale so the quotient has DBL_MANT_DIG + 2 or + 3 bits.
    let shift = DBL_MANT_DIG + 2 - diff;
    let (num, den) = if shift >= 0 {
        (na << (shift as usize), nb)
    } else {
        (na, nb << ((-shift) as usize))
    };
    let (q, r) = num.div_rem(&den);
    let mut q = q.to_u64().expect("scaled quotient fits in 64 bits");
    if !r.is_zero() {
        q |= 1; // sticky bit: the true quotient lies strictly above q
    }
    // Result = q * 2**-shift. Its binary exponent decides normal vs overflow.
    let q_bits = 64 - q.leading_zeros() as i64;
    let exp = q_bits - shift; // result is in [2**(exp-1), 2**exp)
    if exp > 1024 {
        // Out of the double range: the caller raises its own OverflowError.
        return if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let mag = if exp >= -1021 {
        // Normal range: round q to 53 bits once (u64 -> f64 is correctly
        // rounded, ties-to-even), then scale exactly by a power of two.
        ldexp(q as f64, -shift)
    } else {
        // Subnormal: round to the coarser subnormal grid directly.
        let drop = (-1074 + shift) as u32; // bits below 2**-1074
        let keep = if drop >= 64 { 0 } else { q >> drop };
        let rem = if drop >= 64 {
            q
        } else {
            q & ((1u64 << drop) - 1)
        };
        let half = if drop == 0 || drop > 64 {
            0
        } else {
            1u64 << (drop - 1)
        };
        let up = drop > 0 && drop <= 64 && (rem > half || (rem == half && keep & 1 == 1));
        ldexp((keep + up as u64) as f64, -1074)
    };
    if negative {
        -mag
    } else {
        mag
    }
}

/// `x * 2**e` without intermediate overflow/underflow (for the exponent
/// ranges [`int_true_divide`] produces).
fn ldexp(mut x: f64, mut e: i64) -> f64 {
    while e > 1000 {
        x *= 2f64.powi(1000);
        e -= 1000;
    }
    while e < -1000 {
        x *= 2f64.powi(-1000);
        e += 1000;
    }
    x * 2f64.powi(e as i32)
}

/// CPython's `float_divmod`: `(floor(x / y), x mod y)` computed from `fmod`
/// so the quotient is consistent with the remainder (`1.0 // 0.1 == 9.0`).
pub fn float_divmod(vx: f64, wx: f64) -> (f64, f64) {
    let mut m = vx % wx; // fmod
    let mut div = (vx - m) / wx;
    if m != 0.0 {
        if (wx < 0.0) != (m < 0.0) {
            m += wx;
            div -= 1.0;
        }
    } else {
        m = 0.0f64.copysign(wx);
    }
    let floordiv = if div != 0.0 {
        let mut f = div.floor();
        if div - f > 0.5 {
            f += 1.0;
        }
        f
    } else {
        0.0f64.copysign(vx / wx)
    };
    (floordiv, m)
}

/// Exact ordering of an integer against a float (CPython compares int vs
/// float exactly, never through a lossy conversion). `None` for NaN.
pub fn cmp_int_float(i: &BigInt, f: f64) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    if f.is_infinite() {
        return Some(if f > 0.0 {
            Ordering::Less
        } else {
            Ordering::Greater
        });
    }
    if let Some(x) = i.to_i64() {
        if x.unsigned_abs() <= (1u64 << 53) {
            return (x as f64).partial_cmp(&f);
        }
    }
    let fl = f.floor();
    let fl_big = float_to_bigint_exact(fl);
    match i.cmp(&fl_big) {
        Ordering::Equal if f > fl => Some(Ordering::Less),
        ord => Some(ord),
    }
}

/// The exact integer value of a finite, integral `f`.
fn float_to_bigint_exact(f: f64) -> BigInt {
    if f == 0.0 {
        return BigInt::zero();
    }
    let bits = f.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i64;
    let frac = bits & ((1u64 << 52) - 1);
    let (mant, e) = if exp == 0 {
        (frac, -1074)
    } else {
        (frac | (1u64 << 52), exp - 1075)
    };
    let mut v = BigInt::from(mant);
    if e >= 0 {
        v <<= e as usize;
    } else {
        v >>= (-e) as usize; // exact for an integral float
    }
    if f < 0.0 {
        -v
    } else {
        v
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
    if !is_float(&a) && !is_float(&b) {
        // int / int: correctly rounded, never via two lossy conversions.
        let bb = to_big(&b);
        if bb.is_zero() {
            unsafe { crate::raise_exc!(ExceptionType::ZeroDivisionError, "division by zero") };
        }
        let q = int_true_divide(&to_big(&a), &bb);
        if q.is_infinite() {
            unsafe {
                crate::raise_exc!(
                    ExceptionType::OverflowError,
                    "integer division result too large for a float"
                )
            }
        }
        return crate::boxing::rt_box_float(q);
    }
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
            unsafe { crate::raise_exc!(ExceptionType::ZeroDivisionError, "division by zero") };
        }
        return crate::boxing::rt_box_float(float_divmod(to_f64(&a), fb).0);
    }
    let bb = to_big(&b);
    if bb.is_zero() {
        unsafe { crate::raise_exc!(ExceptionType::ZeroDivisionError, "division by zero") };
    }
    // `div_floor` rounds toward -inf (CPython floor division).
    make_int_value(to_big(&a).div_floor(&bb))
}

pub fn num_mod(a: Num, b: Num) -> *mut Obj {
    if is_float(&a) || is_float(&b) {
        let fb = to_f64(&b);
        if fb == 0.0 {
            unsafe { crate::raise_exc!(ExceptionType::ZeroDivisionError, "division by zero") };
        }
        return crate::boxing::rt_box_float(float_divmod(to_f64(&a), fb).1);
    }
    let bb = to_big(&b);
    if bb.is_zero() {
        unsafe { crate::raise_exc!(ExceptionType::ZeroDivisionError, "division by zero") };
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

/// Compare two numerics exactly (int vs float included, as CPython). Returns
/// `None` only on NaN.
pub fn num_cmp(a: &Num, b: &Num) -> Option<Ordering> {
    match (a, b) {
        (Num::Float(x), Num::Float(y)) => x.partial_cmp(y),
        (Num::Float(x), i) => cmp_int_float(&to_big(i), *x).map(Ordering::reverse),
        (i, Num::Float(y)) => cmp_int_float(&to_big(i), *y),
        _ => Some(to_big(a).cmp(&to_big(b))),
    }
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

/// [`cmp_int_float`] for a machine integer (no allocation on the common
/// `|i| <= 2**53` path).
pub fn cmp_i64_float(i: i64, f: f64) -> Option<Ordering> {
    if i.unsigned_abs() <= (1u64 << 53) {
        return (i as f64).partial_cmp(&f);
    }
    cmp_int_float(&BigInt::from(i), f)
}
