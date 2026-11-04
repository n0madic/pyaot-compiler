//! Mathematical operations for Python runtime

use pyaot_core_defs::Value;

use crate::bigint::{classify_num, make_int_value, Num};

/// `int.bit_length()` — number of bits needed to represent the absolute
/// value, ignoring sign and leading zeros. `(0).bit_length() == 0`,
/// `(255).bit_length() == 8`, `(-7).bit_length() == 3`. The argument is a
/// tagged `int` / `bool` `Value` (the receiver baseline, §9); a heap `BigInt`
/// uses `BigInt::bits()`, a fixnum/bool `unsigned_abs().leading_zeros()` (so
/// `i64::MIN` does not overflow). The bit count itself always fits a fixnum.
#[export_name = "rt_int_bit_length"]
pub extern "C" fn rt_int_bit_length(n: Value) -> i64 {
    // SAFETY: `n` is an int/bool tagged value or a live heap `BigInt` (typeck
    // gates the receiver to `int`/`bool`); `classify_num` only derefs a ptr
    // after confirming its type tag.
    match unsafe { classify_num(n) } {
        Some(Num::Int(i)) => (64 - i.unsigned_abs().leading_zeros()) as i64,
        Some(Num::Big(b)) => b.bits() as i64,
        // Unreachable for an int/bool receiver; a defensive 0 (not a SEGV).
        _ => 0,
    }
}

/// `int.bit_count()` (Python 3.10+) — number of set bits in the absolute
/// value. `(255).bit_count() == 8`, `(-7).bit_count() == 3`. Bignum-aware via
/// `BigUint::count_ones()` on the magnitude; the argument is a tagged
/// `int` / `bool` `Value`.
#[export_name = "rt_int_bit_count"]
pub extern "C" fn rt_int_bit_count(n: Value) -> i64 {
    // SAFETY: see `rt_int_bit_length`.
    match unsafe { classify_num(n) } {
        Some(Num::Int(i)) => i.unsigned_abs().count_ones() as i64,
        Some(Num::Big(b)) => b.magnitude().count_ones() as i64,
        _ => 0,
    }
}

/// `int.conjugate()` / `int.__index__()` — an int is its own conjugate and its
/// own index, so this returns the receiver's integer VALUE. Normalizes a
/// `bool` receiver to its int value (`True.conjugate() == 1`) and preserves a
/// bignum (re-boxed via `make_int_value`, demoting to a fixnum when it fits).
/// The argument is a tagged `int` / `bool` `Value`; the result is a tagged int.
#[export_name = "rt_int_index"]
pub extern "C" fn rt_int_index(n: Value) -> Value {
    // SAFETY: see `rt_int_bit_length`.
    match unsafe { classify_num(n) } {
        Some(Num::Int(i)) => Value::from_int(i),
        Some(Num::Big(b)) => Value::from_ptr(make_int_value(b)),
        // Unreachable for an int/bool receiver; return the value verbatim.
        _ => n,
    }
}

/// Power function: pow(base, exp) -> f64
/// Returns: base raised to the power of exp
#[no_mangle]
pub extern "C" fn rt_pow_float(base: f64, exp: f64) -> f64 {
    base.powf(exp)
}

/// Integer power function: base ** exp -> i64
/// For negative exponents, returns 0 (truncated toward zero like Python's int() does)
/// Raises OverflowError on overflow (consistent with rt_mul_int)
#[no_mangle]
pub extern "C" fn rt_pow_int(base: i64, exp: i64) -> i64 {
    if exp < 0 {
        // Negative exponent: result is a fraction, truncate to 0
        // (except for base == 1 or -1)
        match base {
            1 => 1,
            -1 => {
                if exp % 2 == 0 {
                    1
                } else {
                    -1
                }
            }
            _ => 0,
        }
    } else if exp == 0 {
        1
    } else {
        // Use exponentiation by squaring with overflow checking
        let mut result: i64 = 1;
        let mut base = base;
        let mut exp = exp as u64;

        while exp > 0 {
            if exp & 1 == 1 {
                result = match result.checked_mul(base) {
                    Some(v) => v,
                    None => unsafe {
                        raise_exc!(
                            crate::exceptions::ExceptionType::OverflowError,
                            "integer overflow"
                        )
                    },
                };
            }
            if exp > 1 {
                base = match base.checked_mul(base) {
                    Some(v) => v,
                    // Only overflow if we'll actually use this value
                    None => unsafe {
                        raise_exc!(
                            crate::exceptions::ExceptionType::OverflowError,
                            "integer overflow"
                        )
                    },
                };
            }
            exp >>= 1;
        }
        result
    }
}


/// Safe float-to-int conversion: int(x) where x is float.
/// Raises ValueError for NaN, OverflowError for infinity or out-of-range values.
/// Called from codegen instead of raw fcvt_to_sint which traps on NaN/Inf.
#[no_mangle]
pub extern "C" fn rt_float_to_int(x: f64) -> i64 {
    if x.is_nan() {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "cannot convert float NaN to integer"
            )
        }
    }
    if x.is_infinite() || x > i64::MAX as f64 || x < i64::MIN as f64 {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::OverflowError,
                "cannot convert float infinity to integer"
            )
        }
    }
    x as i64
}

// ============================================================================
// math module functions
// ============================================================================

/// Square root: math.sqrt(x) -> f64
#[no_mangle]
pub extern "C" fn rt_math_sqrt(x: f64) -> f64 {
    if x < 0.0 {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "math domain error"
            )
        }
    }
    x.sqrt()
}

/// Sine: math.sin(x) -> f64 (x in radians)
#[no_mangle]
pub extern "C" fn rt_math_sin(x: f64) -> f64 {
    x.sin()
}

/// Cosine: math.cos(x) -> f64 (x in radians)
#[no_mangle]
pub extern "C" fn rt_math_cos(x: f64) -> f64 {
    x.cos()
}

/// Tangent: math.tan(x) -> f64 (x in radians)
#[no_mangle]
pub extern "C" fn rt_math_tan(x: f64) -> f64 {
    x.tan()
}

/// Ceiling: math.ceil(x) -> i64
#[no_mangle]
pub extern "C" fn rt_math_ceil(x: f64) -> i64 {
    let v = x.ceil();
    if v.is_nan() {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "cannot convert float NaN to integer"
            )
        }
    }
    if v.is_infinite() || v > i64::MAX as f64 || v < i64::MIN as f64 {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::OverflowError,
                "cannot convert float infinity to integer"
            )
        }
    }
    v as i64
}

/// Floor: math.floor(x) -> i64
#[no_mangle]
pub extern "C" fn rt_math_floor(x: f64) -> i64 {
    let v = x.floor();
    if v.is_nan() {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "cannot convert float NaN to integer"
            )
        }
    }
    if v.is_infinite() || v > i64::MAX as f64 || v < i64::MIN as f64 {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::OverflowError,
                "cannot convert float infinity to integer"
            )
        }
    }
    v as i64
}

/// Convert a math-function argument to an exact integer (CPython's
/// `__index__` rule for the builtin numeric types): an `int` / `bool` /
/// big `int`, else `TypeError`.
///
/// # Safety
/// `v`'s pointer (if any) must reference a live object.
unsafe fn index_arg(v: Value) -> num_bigint::BigInt {
    match classify_num(v) {
        Some(Num::Int(i)) => num_bigint::BigInt::from(i),
        Some(Num::Big(b)) => b,
        Some(Num::Float(_)) => raise_exc!(
            crate::exceptions::ExceptionType::TypeError,
            "'float' object cannot be interpreted as an integer"
        ),
        None => raise_exc!(
            crate::exceptions::ExceptionType::TypeError,
            "'{}' object cannot be interpreted as an integer",
            crate::instance::value_type_name(v)
        ),
    }
}

/// `math.factorial(n)` — exact, arbitrary precision (contract change: the
/// argument and result are tagged `int` Values, so `factorial(25)` is a big
/// int instead of an `OverflowError` / a wrapped fixnum).
#[export_name = "rt_math_factorial"]
pub extern "C" fn rt_math_factorial(n: Value) -> Value {
    use num_traits::{One, Signed, ToPrimitive};
    let n = unsafe { index_arg(n) };
    if n.is_negative() {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "factorial() not defined for negative values"
            );
        }
    }
    let n = n.to_u64().unwrap_or_else(|| unsafe {
        raise_exc!(
            crate::exceptions::ExceptionType::OverflowError,
            "factorial() argument should not exceed {}",
            i64::MAX
        )
    });
    let mut acc = num_bigint::BigInt::one();
    for i in 2..=n {
        acc *= i;
    }
    Value(make_int_value(acc) as u64)
}

/// `math.gcd(a, b)` — exact, arbitrary precision (tagged `int` Values).
#[export_name = "rt_math_gcd"]
pub extern "C" fn rt_math_gcd(a: Value, b: Value) -> Value {
    use num_integer::Integer;
    let (a, b) = unsafe { (index_arg(a), index_arg(b)) };
    Value(make_int_value(a.gcd(&b)) as u64)
}

/// `math.lcm(a, b)` — exact, arbitrary precision (tagged `int` Values).
#[export_name = "rt_math_lcm"]
pub extern "C" fn rt_math_lcm(a: Value, b: Value) -> Value {
    use num_integer::Integer;
    let (a, b) = unsafe { (index_arg(a), index_arg(b)) };
    Value(make_int_value(a.lcm(&b)) as u64)
}

/// Shared argument check of `comb` / `perm`: two non-negative ints.
fn comb_args(n: Value, k: Value, fname: &str) -> (num_bigint::BigInt, num_bigint::BigInt) {
    use num_traits::Signed;
    let (n, k) = unsafe { (index_arg(n), index_arg(k)) };
    if n.is_negative() {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "n must be a non-negative integer"
            );
        }
    }
    if k.is_negative() {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "k must be a non-negative integer"
            );
        }
    }
    let _ = fname;
    (n, k)
}

/// `math.comb(n, k)` — the binomial coefficient, exact (tagged `int` Values).
#[export_name = "rt_math_comb"]
pub extern "C" fn rt_math_comb(n: Value, k: Value) -> Value {
    use num_traits::{One, Zero};
    let (n, k) = comb_args(n, k, "comb");
    if k > n {
        return Value(make_int_value(num_bigint::BigInt::zero()) as u64);
    }
    let k = std::cmp::min(k.clone(), &n - &k);
    let mut acc = num_bigint::BigInt::one();
    let mut i = num_bigint::BigInt::zero();
    while i < k {
        acc = acc * (&n - &i) / (&i + 1u32);
        i += 1u32;
    }
    Value(make_int_value(acc) as u64)
}

/// `math.perm(n, k)` — `n! / (n-k)!`, exact (tagged `int` Values).
#[export_name = "rt_math_perm"]
pub extern "C" fn rt_math_perm(n: Value, k: Value) -> Value {
    use num_traits::{One, Zero};
    let (n, k) = comb_args(n, k, "perm");
    if k > n {
        return Value(make_int_value(num_bigint::BigInt::zero()) as u64);
    }
    let mut acc = num_bigint::BigInt::one();
    let mut i = num_bigint::BigInt::zero();
    while i < k {
        acc *= &n - &i;
        i += 1u32;
    }
    Value(make_int_value(acc) as u64)
}


/// Logarithm: math.log(x[, base]) -> f64
/// When base is NaN (sentinel), computes natural logarithm.
/// Otherwise computes log(x) / log(base) like CPython.
#[no_mangle]
pub extern "C" fn rt_math_log(x: f64, base: f64, arg_count: i64) -> f64 {
    // `arg_count` (user-written argument count) tells `log(x)` from
    // `log(x, base)` — no in-band sentinel, so `log(x, nan)` is `nan`.
    let domain_error = || unsafe {
        raise_exc!(
            crate::exceptions::ExceptionType::ValueError,
            "expected a positive input"
        )
    };
    if x <= 0.0 {
        domain_error();
    }
    if arg_count < 2 {
        return x.ln();
    }
    if base <= 0.0 {
        domain_error();
    }
    let den = base.ln();
    if den == 0.0 {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ZeroDivisionError,
                "division by zero"
            )
        }
    }
    x.ln() / den
}

/// Logarithm base 2: math.log2(x) -> f64
#[no_mangle]
pub extern "C" fn rt_math_log2(x: f64) -> f64 {
    if x <= 0.0 {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "math domain error"
            )
        }
    }
    x.log2()
}

/// Logarithm base 10: math.log10(x) -> f64
#[no_mangle]
pub extern "C" fn rt_math_log10(x: f64) -> f64 {
    if x <= 0.0 {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "math domain error"
            )
        }
    }
    x.log10()
}

/// Exponential: math.exp(x) -> f64
#[no_mangle]
pub extern "C" fn rt_math_exp(x: f64) -> f64 {
    x.exp()
}

/// Arc sine: math.asin(x) -> f64 (result in radians)
#[no_mangle]
pub extern "C" fn rt_math_asin(x: f64) -> f64 {
    if !(-1.0..=1.0).contains(&x) {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "math domain error"
            )
        }
    }
    x.asin()
}

/// Arc cosine: math.acos(x) -> f64 (result in radians)
#[no_mangle]
pub extern "C" fn rt_math_acos(x: f64) -> f64 {
    if !(-1.0..=1.0).contains(&x) {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "math domain error"
            )
        }
    }
    x.acos()
}

/// Arc tangent: math.atan(x) -> f64 (result in radians)
#[no_mangle]
pub extern "C" fn rt_math_atan(x: f64) -> f64 {
    x.atan()
}

/// Hyperbolic sine: math.sinh(x) -> f64
#[no_mangle]
pub extern "C" fn rt_math_sinh(x: f64) -> f64 {
    x.sinh()
}

/// Hyperbolic cosine: math.cosh(x) -> f64
#[no_mangle]
pub extern "C" fn rt_math_cosh(x: f64) -> f64 {
    x.cosh()
}

/// Hyperbolic tangent: math.tanh(x) -> f64
#[no_mangle]
pub extern "C" fn rt_math_tanh(x: f64) -> f64 {
    x.tanh()
}

/// Absolute value: math.fabs(x) -> f64
#[no_mangle]
pub extern "C" fn rt_math_fabs(x: f64) -> f64 {
    x.abs()
}

/// Convert radians to degrees: math.degrees(x) -> f64
#[no_mangle]
pub extern "C" fn rt_math_degrees(x: f64) -> f64 {
    x.to_degrees()
}

/// Convert degrees to radians: math.radians(x) -> f64
#[no_mangle]
pub extern "C" fn rt_math_radians(x: f64) -> f64 {
    x.to_radians()
}

/// Truncate to integer: math.trunc(x) -> i64
#[no_mangle]
pub extern "C" fn rt_math_trunc(x: f64) -> i64 {
    let v = x.trunc();
    if v.is_nan() {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "cannot convert float NaN to integer"
            )
        }
    }
    if v.is_infinite() || v > i64::MAX as f64 || v < i64::MIN as f64 {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::OverflowError,
                "cannot convert float infinity to integer"
            )
        }
    }
    v as i64
}

/// Test if NaN: math.isnan(x) -> i8
#[no_mangle]
pub extern "C" fn rt_math_isnan(x: f64) -> i8 {
    x.is_nan() as i8
}

/// Test if infinite: math.isinf(x) -> i8
#[no_mangle]
pub extern "C" fn rt_math_isinf(x: f64) -> i8 {
    x.is_infinite() as i8
}

/// Test if finite: math.isfinite(x) -> i8
#[no_mangle]
pub extern "C" fn rt_math_isfinite(x: f64) -> i8 {
    x.is_finite() as i8
}

/// Arc tangent of y/x: math.atan2(y, x) -> f64 (result in radians)
#[no_mangle]
pub extern "C" fn rt_math_atan2(y: f64, x: f64) -> f64 {
    y.atan2(x)
}

/// Floating point remainder: math.fmod(x, y) -> f64
#[no_mangle]
pub extern "C" fn rt_math_fmod(x: f64, y: f64) -> f64 {
    x % y
}

/// Copy sign: math.copysign(x, y) -> f64
#[no_mangle]
pub extern "C" fn rt_math_copysign(x: f64, y: f64) -> f64 {
    x.copysign(y)
}

/// Euclidean distance: math.hypot(x, y) -> f64
#[no_mangle]
pub extern "C" fn rt_math_hypot(x: f64, y: f64) -> f64 {
    x.hypot(y)
}

/// Power: math.pow(x, y) -> f64
#[no_mangle]
pub extern "C" fn rt_math_pow(x: f64, y: f64) -> f64 {
    x.powf(y)
}





#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pow_float() {
        assert_eq!(rt_pow_float(2.0, 3.0), 8.0);
        assert_eq!(rt_pow_float(2.0, 0.0), 1.0);
    }

    #[test]
    fn test_pow_int_basic() {
        assert_eq!(rt_pow_int(2, 10), 1024);
        assert_eq!(rt_pow_int(3, 0), 1);
        assert_eq!(rt_pow_int(0, 5), 0);
        assert_eq!(rt_pow_int(1, 1000), 1);
        assert_eq!(rt_pow_int(-1, 3), -1);
        assert_eq!(rt_pow_int(-1, 4), 1);
    }

    #[test]
    fn test_pow_int_negative_exp() {
        assert_eq!(rt_pow_int(2, -1), 0);
        assert_eq!(rt_pow_int(1, -5), 1);
        assert_eq!(rt_pow_int(-1, -3), -1);
        assert_eq!(rt_pow_int(-1, -4), 1);
    }

    #[test]
    fn test_float_to_int_basic() {
        assert_eq!(rt_float_to_int(3.7), 3);
        assert_eq!(rt_float_to_int(-3.7), -3);
        assert_eq!(rt_float_to_int(0.0), 0);
        assert_eq!(rt_float_to_int(1.0), 1);
    }

    #[test]
    fn test_math_sqrt() {
        assert_eq!(rt_math_sqrt(4.0), 2.0);
        assert_eq!(rt_math_sqrt(0.0), 0.0);
    }

    #[test]
    fn test_math_floor_ceil() {
        assert_eq!(rt_math_floor(1.7), 1);
        assert_eq!(rt_math_floor(-1.7), -2);
        assert_eq!(rt_math_ceil(1.3), 2);
        assert_eq!(rt_math_ceil(-1.3), -1);
    }

    #[test]
    fn test_math_gcd() {
        let g = |a: i64, b: i64| rt_math_gcd(Value::from_int(a), Value::from_int(b)).unwrap_int();
        assert_eq!(g(12, 8), 4);
        assert_eq!(g(7, 13), 1);
        assert_eq!(g(0, 5), 5);
        assert_eq!(g(-12, 8), 4);
    }

    #[test]
    fn test_math_factorial() {
        assert_eq!(rt_math_factorial(Value::from_int(0)).unwrap_int(), 1);
        assert_eq!(rt_math_factorial(Value::from_int(1)).unwrap_int(), 1);
        assert_eq!(rt_math_factorial(Value::from_int(5)).unwrap_int(), 120);
        assert_eq!(rt_math_factorial(Value::from_int(10)).unwrap_int(), 3628800);
    }

    #[test]
    fn test_math_comb() {
        assert_eq!(rt_math_comb(Value::from_int(5), Value::from_int(2)).unwrap_int(), 10);
        assert_eq!(rt_math_comb(Value::from_int(10), Value::from_int(0)).unwrap_int(), 1);
        assert_eq!(rt_math_comb(Value::from_int(10), Value::from_int(10)).unwrap_int(), 1);
        assert_eq!(rt_math_comb(Value::from_int(5), Value::from_int(6)).unwrap_int(), 0);
    }

    #[test]
    fn test_math_perm() {
        assert_eq!(rt_math_perm(Value::from_int(5), Value::from_int(2)).unwrap_int(), 20);
        assert_eq!(rt_math_perm(Value::from_int(5), Value::from_int(0)).unwrap_int(), 1);
        assert_eq!(rt_math_perm(Value::from_int(5), Value::from_int(6)).unwrap_int(), 0);
    }

    #[test]
    fn test_math_trig() {
        assert!((rt_math_sin(0.0)).abs() < 1e-10);
        assert!((rt_math_cos(0.0) - 1.0).abs() < 1e-10);
        assert!((rt_math_tan(0.0)).abs() < 1e-10);
    }

    #[test]
    fn test_math_abs_degrees_radians() {
        assert_eq!(rt_math_fabs(-2.5), 2.5);
        assert!((rt_math_degrees(std::f64::consts::PI) - 180.0).abs() < 1e-10);
        assert!((rt_math_radians(180.0) - std::f64::consts::PI).abs() < 1e-10);
    }
}
