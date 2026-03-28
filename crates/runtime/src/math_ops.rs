//! Mathematical operations for Python runtime

/// Power function: pow(base, exp) -> f64
/// Returns: base raised to the power of exp
#[no_mangle]
pub extern "C" fn rt_pow_float(base: f64, exp: f64) -> f64 {
    base.powf(exp)
}

/// Integer power function: base ** exp -> i64
/// For negative exponents, returns 0 (truncated toward zero like Python's int() does)
/// For very large results, behavior is wrapping (overflow)
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
        // Use exponentiation by squaring for efficiency
        let mut result: i64 = 1;
        let mut base = base;
        let mut exp = exp as u64;

        while exp > 0 {
            if exp & 1 == 1 {
                result = result.wrapping_mul(base);
            }
            base = base.wrapping_mul(base);
            exp >>= 1;
        }
        result
    }
}

/// Round float to nearest integer using banker's rounding (round half to even): round(x) -> i64
/// Returns: rounded value as integer
#[no_mangle]
pub extern "C" fn rt_round_to_int(x: f64) -> i64 {
    if x.is_nan() {
        let msg = b"ValueError: cannot convert float NaN to integer";
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::ValueError as u8,
                msg.as_ptr(),
                msg.len(),
            )
        }
    }
    if x.is_infinite() || x > i64::MAX as f64 || x < i64::MIN as f64 {
        let msg = b"OverflowError: cannot convert float infinity to integer";
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::OverflowError as u8,
                msg.as_ptr(),
                msg.len(),
            )
        }
    }
    // Banker's rounding: round half to even
    let floor = x.floor();
    let ceil = x.ceil();
    let diff_floor = (x - floor).abs();
    let diff_ceil = (ceil - x).abs();

    if diff_floor < diff_ceil {
        floor as i64
    } else if diff_ceil < diff_floor {
        ceil as i64
    } else {
        // Exactly halfway - round to even
        let floor_int = floor as i64;
        if floor_int % 2 == 0 {
            floor_int
        } else {
            ceil as i64
        }
    }
}

/// Round float to N decimal places using banker's rounding: round(x, ndigits) -> f64
/// Returns: rounded value as float
#[no_mangle]
pub extern "C" fn rt_round_to_digits(x: f64, ndigits: i64) -> f64 {
    if ndigits == 0 {
        // Use banker's rounding for ndigits=0 too
        return rt_round_to_int(x) as f64;
    }
    let multiplier = 10_f64.powi(ndigits as i32);
    let scaled = x * multiplier;

    // Apply banker's rounding to scaled value
    let floor = scaled.floor();
    let ceil = scaled.ceil();
    let diff_floor = (scaled - floor).abs();
    let diff_ceil = (ceil - scaled).abs();

    let rounded = if diff_floor < diff_ceil {
        floor
    } else if diff_ceil < diff_floor {
        ceil
    } else {
        // Exactly halfway - round to even
        let floor_int = floor as i64;
        if floor_int % 2 == 0 {
            floor
        } else {
            ceil
        }
    };

    rounded / multiplier
}

// ============================================================================
// math module functions
// ============================================================================

/// Square root: math.sqrt(x) -> f64
#[no_mangle]
pub extern "C" fn rt_math_sqrt(x: f64) -> f64 {
    if x < 0.0 {
        let msg = b"ValueError: math domain error";
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::ValueError as u8,
                msg.as_ptr(),
                msg.len(),
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
        let msg = b"ValueError: cannot convert float NaN to integer";
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::ValueError as u8,
                msg.as_ptr(),
                msg.len(),
            )
        }
    }
    if v.is_infinite() || v > i64::MAX as f64 || v < i64::MIN as f64 {
        let msg = b"OverflowError: cannot convert float infinity to integer";
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::OverflowError as u8,
                msg.as_ptr(),
                msg.len(),
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
        let msg = b"ValueError: cannot convert float NaN to integer";
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::ValueError as u8,
                msg.as_ptr(),
                msg.len(),
            )
        }
    }
    if v.is_infinite() || v > i64::MAX as f64 || v < i64::MIN as f64 {
        let msg = b"OverflowError: cannot convert float infinity to integer";
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::OverflowError as u8,
                msg.as_ptr(),
                msg.len(),
            )
        }
    }
    v as i64
}

/// Factorial: math.factorial(n) -> i64
/// Returns n! for non-negative integers, raises ValueError for negative values.
#[no_mangle]
pub extern "C" fn rt_math_factorial(n: i64) -> i64 {
    if n < 0 {
        let msg = "factorial() not defined for negative values";
        unsafe {
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
    }
    if n <= 1 {
        return 1;
    }
    let mut result: i64 = 1;
    for i in 2..=n {
        result = match result.checked_mul(i) {
            Some(v) => v,
            None => {
                let msg = b"OverflowError: int too large to convert to C long";
                unsafe {
                    crate::exceptions::rt_exc_raise(
                        crate::exceptions::ExceptionType::OverflowError as u8,
                        msg.as_ptr(),
                        msg.len(),
                    );
                }
            }
        };
    }
    result
}

/// Logarithm: math.log(x[, base]) -> f64
/// When base is NaN (sentinel), computes natural logarithm.
/// Otherwise computes log(x) / log(base) like CPython.
#[no_mangle]
pub extern "C" fn rt_math_log(x: f64, base: f64) -> f64 {
    if x <= 0.0 {
        let msg = b"ValueError: math domain error";
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::ValueError as u8,
                msg.as_ptr(),
                msg.len(),
            )
        }
    }
    if base.is_nan() {
        x.ln()
    } else {
        x.ln() / base.ln()
    }
}

/// Logarithm base 2: math.log2(x) -> f64
#[no_mangle]
pub extern "C" fn rt_math_log2(x: f64) -> f64 {
    if x <= 0.0 {
        let msg = b"ValueError: math domain error";
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::ValueError as u8,
                msg.as_ptr(),
                msg.len(),
            )
        }
    }
    x.log2()
}

/// Logarithm base 10: math.log10(x) -> f64
#[no_mangle]
pub extern "C" fn rt_math_log10(x: f64) -> f64 {
    if x <= 0.0 {
        let msg = b"ValueError: math domain error";
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::ValueError as u8,
                msg.as_ptr(),
                msg.len(),
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
        let msg = b"ValueError: math domain error";
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::ValueError as u8,
                msg.as_ptr(),
                msg.len(),
            )
        }
    }
    x.asin()
}

/// Arc cosine: math.acos(x) -> f64 (result in radians)
#[no_mangle]
pub extern "C" fn rt_math_acos(x: f64) -> f64 {
    if !(-1.0..=1.0).contains(&x) {
        let msg = b"ValueError: math domain error";
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::ValueError as u8,
                msg.as_ptr(),
                msg.len(),
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
        let msg = b"ValueError: cannot convert float NaN to integer";
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::ValueError as u8,
                msg.as_ptr(),
                msg.len(),
            )
        }
    }
    if v.is_infinite() || v > i64::MAX as f64 || v < i64::MIN as f64 {
        let msg = b"OverflowError: cannot convert float infinity to integer";
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::OverflowError as u8,
                msg.as_ptr(),
                msg.len(),
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

/// Greatest common divisor: math.gcd(a, b) -> i64
#[no_mangle]
pub extern "C" fn rt_math_gcd(a: i64, b: i64) -> i64 {
    let mut a = (a as i128).unsigned_abs() as u64;
    let mut b = (b as i128).unsigned_abs() as u64;

    while b != 0 {
        let temp = b;
        b = a % b;
        a = temp;
    }
    // gcd(i64::MIN, 0) == 2^63 which exceeds i64::MAX
    if a > i64::MAX as u64 {
        let msg = b"OverflowError: math.gcd result too large to convert to int";
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::OverflowError as u8,
                msg.as_ptr(),
                msg.len(),
            )
        }
    }
    a as i64
}

/// Least common multiple: math.lcm(a, b) -> i64
#[no_mangle]
pub extern "C" fn rt_math_lcm(a: i64, b: i64) -> i64 {
    if a == 0 || b == 0 {
        return 0;
    }
    let g = rt_math_gcd(a, b);
    let aa = (a as i128).abs();
    let bb = (b as i128).abs();
    let result = (aa / g as i128) * bb;
    if result > i64::MAX as i128 {
        let msg = b"OverflowError: int too large to convert to C long";
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::OverflowError as u8,
                msg.as_ptr(),
                msg.len(),
            );
        }
    }
    result as i64
}

/// Binomial coefficient: math.comb(n, k) -> i64
/// Returns n! / (k! * (n-k)!)
#[no_mangle]
pub extern "C" fn rt_math_comb(n: i64, k: i64) -> i64 {
    if k < 0 || n < 0 {
        let msg = "comb() requires non-negative arguments";
        unsafe {
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
    }

    if k > n {
        return 0;
    }

    // Optimize by using min(k, n-k)
    let k = if k > n - k { n - k } else { k };

    if k == 0 {
        return 1;
    }

    let mut result: i128 = 1;
    for i in 0..k {
        result = result * (n - i) as i128 / (i + 1) as i128;
        if result > i64::MAX as i128 {
            let msg = b"OverflowError: int too large to convert to C long";
            unsafe {
                crate::exceptions::rt_exc_raise(
                    crate::exceptions::ExceptionType::OverflowError as u8,
                    msg.as_ptr(),
                    msg.len(),
                );
            }
        }
    }
    result as i64
}

/// Permutation count: math.perm(n, k) -> i64
/// Returns n! / (n-k)!
#[no_mangle]
pub extern "C" fn rt_math_perm(n: i64, k: i64) -> i64 {
    if k < 0 || n < 0 {
        let msg = "perm() requires non-negative arguments";
        unsafe {
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
    }

    if k > n {
        return 0;
    }

    let mut result: i128 = 1;
    for i in 0..k {
        result *= (n - i) as i128;
        if result > i64::MAX as i128 {
            let msg = b"OverflowError: int too large to convert to C long";
            unsafe {
                crate::exceptions::rt_exc_raise(
                    crate::exceptions::ExceptionType::OverflowError as u8,
                    msg.as_ptr(),
                    msg.len(),
                );
            }
        }
    }
    result as i64
}
