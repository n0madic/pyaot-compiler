//! Type cast and numeric formatting conversions for Python runtime

use crate::bigint::{classify_num, make_int_value, Num};
use crate::boxing::rt_box_float;
use crate::object::{Obj, ObjHeader, StrObj, TypeTagKind};
use crate::string::rt_make_str;
use num_bigint::BigInt;
use num_traits::FromPrimitive;
use pyaot_core_defs::Value;

// ==================== Number formatting ====================

/// Convert integer to binary string (e.g., '0b1010')
pub fn rt_int_to_bin(n: i64) -> *mut Obj {
    let s = if n < 0 {
        format!("-0b{:b}", n.unsigned_abs())
    } else {
        format!("0b{:b}", n)
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_int_to_bin"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_int_to_bin_abi(n: i64) -> Value {
    Value::from_ptr(rt_int_to_bin(n))
}

/// Convert integer to hexadecimal string (e.g., '0xff')
pub fn rt_int_to_hex(n: i64) -> *mut Obj {
    let s = if n < 0 {
        format!("-0x{:x}", n.unsigned_abs())
    } else {
        format!("0x{:x}", n)
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_int_to_hex"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_int_to_hex_abi(n: i64) -> Value {
    Value::from_ptr(rt_int_to_hex(n))
}

/// Convert integer to octal string (e.g., '0o10')
pub fn rt_int_to_oct(n: i64) -> *mut Obj {
    let s = if n < 0 {
        format!("-0o{:o}", n.unsigned_abs())
    } else {
        format!("0o{:o}", n)
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_int_to_oct"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_int_to_oct_abi(n: i64) -> Value {
    Value::from_ptr(rt_int_to_oct(n))
}

// ============ Bignum-aware scalar builtins: bin / hex / oct / round ==========
//
// PITFALLS B16: `bin`/`hex`/`oct` MUST take a TAGGED `Value`, not a raw `i64` —
// the receiver may be a heap `BigInt` (`bin(2 ** 100)`), and unboxing a bignum
// pointer to a raw word would format garbage. The fixnum / bool fast path reuses
// the raw `rt_int_to_*` formatters; the bignum path uses `BigInt::to_str_radix`.

/// Format a heap `BigInt` with a leading sign and base prefix (`0b`/`0o`/`0x`).
/// `to_str_radix` already emits the sign for negatives, so re-attach the prefix
/// after the `-` to match CPython (`bin(-5) == "-0b101"`).
fn bignum_radix_str(b: &BigInt, radix: u32, prefix: &str) -> String {
    let s = b.to_str_radix(radix);
    match s.strip_prefix('-') {
        Some(rest) => format!("-{prefix}{rest}"),
        None => format!("{prefix}{s}"),
    }
}

/// Allocate a `StrObj` from a Rust string.
fn str_from_string(s: &str) -> *mut Obj {
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// Raise the CPython `TypeError` for a non-integer argument to bin/hex/oct.
fn raise_not_an_integer(what: &str) -> ! {
    unsafe {
        raise_exc!(
            crate::exceptions::ExceptionType::TypeError,
            "'{}' object cannot be interpreted as an integer",
            what
        )
    }
}

/// `bin(n)` — binary string with a `0b` prefix. Bignum-aware (B16); `bool`
/// formats as its int value (`bin(True) == "0b1"`).
pub fn rt_builtin_bin(n: Value) -> *mut Obj {
    match unsafe { classify_num(n) } {
        Some(Num::Int(i)) => rt_int_to_bin(i),
        Some(Num::Big(b)) => str_from_string(&bignum_radix_str(&b, 2, "0b")),
        Some(Num::Float(_)) => raise_not_an_integer("float"),
        None => raise_not_an_integer("object"),
    }
}
#[export_name = "rt_builtin_bin"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_builtin_bin_abi(n: Value) -> Value {
    Value::from_ptr(rt_builtin_bin(n))
}

/// `hex(n)` — hexadecimal string with a `0x` prefix. Bignum-aware (B16).
pub fn rt_builtin_hex(n: Value) -> *mut Obj {
    match unsafe { classify_num(n) } {
        Some(Num::Int(i)) => rt_int_to_hex(i),
        Some(Num::Big(b)) => str_from_string(&bignum_radix_str(&b, 16, "0x")),
        Some(Num::Float(_)) => raise_not_an_integer("float"),
        None => raise_not_an_integer("object"),
    }
}
#[export_name = "rt_builtin_hex"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_builtin_hex_abi(n: Value) -> Value {
    Value::from_ptr(rt_builtin_hex(n))
}

/// `oct(n)` — octal string with a `0o` prefix. Bignum-aware (B16).
pub fn rt_builtin_oct(n: Value) -> *mut Obj {
    match unsafe { classify_num(n) } {
        Some(Num::Int(i)) => rt_int_to_oct(i),
        Some(Num::Big(b)) => str_from_string(&bignum_radix_str(&b, 8, "0o")),
        Some(Num::Float(_)) => raise_not_an_integer("float"),
        None => raise_not_an_integer("object"),
    }
}
#[export_name = "rt_builtin_oct"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_builtin_oct_abi(n: Value) -> Value {
    Value::from_ptr(rt_builtin_oct(n))
}

/// Build an `int` Value from a float that `round_ties_even` has already made a
/// whole number. Demotes to a tagged fixnum when it fits, else a heap `BigInt`
/// (CPython returns an exact int for huge floats). NaN/Inf cannot occur here —
/// the caller only invokes this for a finite result.
fn int_value_from_whole_f64(r: f64) -> *mut Obj {
    if (i64::MIN as f64..=i64::MAX as f64).contains(&r) {
        // Exact: `r` is integral and within i64, so the cast is lossless.
        make_int_value(BigInt::from(r as i64))
    } else {
        // Huge whole float → exact heap BigInt (truncation is exact: `r` is
        // already integral).
        match BigInt::from_f64(r) {
            Some(b) => make_int_value(b),
            None => unsafe {
                raise_exc!(
                    crate::exceptions::ExceptionType::OverflowError,
                    "cannot convert float to integer"
                )
            },
        }
    }
}

/// `round(x, ndigits)` for a float `x` and a non-`None` `ndigits` ≥ 0:
/// correctly-rounded to `ndigits` decimal places via decimal formatting, which
/// is round-half-to-even on the TRUE stored value — matching CPython exactly
/// (`round(2.675, 2) == 2.67`, since `2.675` is `2.67499…` as a double). A
/// naive `(x * 10ⁿ).round() / 10ⁿ` diverges here because the scaled product
/// rounds up to `267.5` (PITFALLS B1). Negative `ndigits` (round to tens/…)
/// has no formatter form, so it uses scaling — an accepted divergence in rare
/// half-way cases (unprobed).
fn round_to_digits_decimal(x: f64, n: i64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    if n >= 0 {
        // Beyond ~17 fractional significant digits a double can't change, so
        // clamp the formatted precision to bound the string (parse-back is
        // still the identity for larger `n`).
        let prec = (n as usize).min(323);
        format!("{:.*}", prec, x).parse::<f64>().unwrap_or(x)
    } else {
        let scale = 10f64.powi((-n).min(308) as i32);
        (x / scale).round_ties_even() * scale
    }
}

/// `round(x[, ndigits])` — banker's rounding (round-half-to-even, CPython B1).
///
/// The **presence** of `ndigits` selects the result type, not its value:
/// `round(2.5)` → `2` (int), `round(2.5, 0)` → `2.0` (float). The frontend
/// passes the absent / explicit-`None` second argument as the null sentinel
/// (`Value(0)`) or `None`, both of which mean "no ndigits" (matching CPython,
/// where `round(x, None)` returns an int). An int `x` stays int; a float `x`
/// with no ndigits rounds to an int (heap `BigInt` when huge), and with ndigits
/// rounds to a float.
pub fn rt_builtin_round(x: Value, ndigits: Value) -> *mut Obj {
    let has_ndigits = ndigits.0 != 0 && !ndigits.is_none();
    match unsafe { classify_num(x) } {
        // Float receiver: ndigits decides int-vs-float result.
        Some(Num::Float(f)) => {
            if has_ndigits {
                let n = match unsafe { classify_num(ndigits) } {
                    Some(Num::Int(i)) => i,
                    // `ndigits` beyond i64 (a bignum) is far past any meaningful
                    // decimal place; saturate.
                    Some(Num::Big(_)) => i64::MAX,
                    _ => unsafe {
                        raise_exc!(
                            crate::exceptions::ExceptionType::TypeError,
                            "'...' object cannot be interpreted as an integer"
                        )
                    },
                };
                rt_box_float(round_to_digits_decimal(f, n))
            } else {
                int_value_from_whole_f64(f.round_ties_even())
            }
        }
        // `bool` rounds to its int value (`round(True) == 1`, an int).
        Some(Num::Int(_)) if x.is_bool() => make_int_value(BigInt::from(x.unwrap_bool() as i64)),
        // Int / bignum receiver: returned unchanged for absent / non-negative
        // ndigits (integer rounding to >= 0 places is the identity). Negative
        // ndigits on an int is an accepted divergence (unprobed) — returned as-is.
        // Preserve the exact tagged bits (a fixnum is not a pointer): the ABI
        // wrapper re-wraps `Value(self.0)`, round-tripping the original value.
        Some(Num::Int(_)) | Some(Num::Big(_)) => x.0 as *mut Obj,
        None => unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::TypeError,
                "type cannot be interpreted as a number"
            )
        },
    }
}
#[export_name = "rt_builtin_round"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_builtin_round_abi(x: Value, ndigits: Value) -> Value {
    Value::from_ptr(rt_builtin_round(x, ndigits))
}

// ==================== Type name functions ====================

/// Get type name for type() builtin
/// Uses type_class() from core-defs as the single source of truth.
pub fn rt_type_name(obj: *mut Obj) -> *mut Obj {
    let v = pyaot_core_defs::Value(obj as u64);
    if obj.is_null() || v.is_none() {
        let s = TypeTagKind::None.type_class();
        let bytes = s.as_bytes();
        return unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) };
    }
    if v.is_int() {
        let s = TypeTagKind::Int.type_class();
        let bytes = s.as_bytes();
        return unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) };
    }
    if v.is_bool() {
        let s = TypeTagKind::Bool.type_class();
        let bytes = s.as_bytes();
        return unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) };
    }

    unsafe {
        // Class instances: use the registered qualified name so `type(w)`
        // gives `<class '__main__.Widget'>` (CPython compatible) instead of
        // the generic `<class 'object'>` from the `Instance` type tag.
        if (*obj).header.type_tag == TypeTagKind::Instance {
            let class_id = (*(obj as *const crate::object::InstanceObj)).class_id;
            if let Some(qn) = crate::instance::lookup_class_qualname(class_id) {
                let s = format!("<class '{}'>", qn);
                let bytes = s.as_bytes();
                return rt_make_str(bytes.as_ptr(), bytes.len());
            }
        }
        // Get type_class directly from the type tag - single source of truth in core-defs
        let type_class_str = (*obj).header.type_tag.type_class();
        let bytes = type_class_str.as_bytes();
        rt_make_str(bytes.as_ptr(), bytes.len())
    }
}
#[export_name = "rt_type_name"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_type_name_abi(obj: Value) -> Value {
    Value::from_ptr(rt_type_name(obj.unwrap_ptr()))
}

/// Extract type name from type string for __name__ attribute access
/// Extracts "int" from "<class 'int'>" for type(x).__name__
/// Input: string object like "<class 'int'>"
/// Output: string object like "int"
///
/// `__name__` is the bare class name, NOT module-qualified: CPython's
/// `type(w).__name__` is `"Widget"` (not `"__main__.Widget"`) and
/// `time.struct_time.__name__` is `"struct_time"`. So the last dotted
/// segment of the quoted name is taken.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_type_name_extract(type_str: *mut Obj) -> *mut Obj {
    let full_str = unsafe { crate::utils::str_obj_to_rust_string(type_str) };

    // Parse "<class 'module.typename'>" → bare "typename" (last `.` segment).
    if let Some(start) = full_str.find("'") {
        if let Some(end) = full_str.rfind("'") {
            if start < end {
                let qualified = &full_str[start + 1..end];
                let name = qualified.rsplit('.').next().unwrap_or(qualified);
                let bytes = name.as_bytes();
                return unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) };
            }
        }
    }

    // Fallback: return the full string if parsing fails
    let bytes = full_str.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_type_name_extract"]
pub extern "C" fn rt_type_name_extract_abi(type_str: Value) -> Value {
    Value::from_ptr(rt_type_name_extract(type_str.unwrap_ptr()))
}

/// Default repr for objects without __str__ or __repr__.
/// Returns: pointer to a string like "<__main__.Cls object at 0x...>"
/// (CPython compatible; falls back to "<object at 0x...>" when the class
/// registered no qualified name).
pub fn rt_obj_default_repr(obj: *mut Obj) -> *mut Obj {
    unsafe {
        let s = crate::instance::instance_default_repr(obj);
        let bytes = s.as_bytes();
        rt_make_str(bytes.as_ptr(), bytes.len())
    }
}
#[export_name = "rt_obj_default_repr"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_default_repr_abi(obj: Value) -> Value {
    Value::from_ptr(rt_obj_default_repr(obj.unwrap_ptr()))
}

/// Strip CPython-style digit-group underscores from a numeric literal body.
///
/// CPython allows a single `_` only *between* two digits (it may not lead,
/// trail, double up, or sit next to a sign). Returns `None` on a malformed
/// placement so the caller raises `ValueError`, matching CPython.
pub(crate) fn strip_int_underscores(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'_' {
            let prev_ok =
                i > 0 && bytes[i - 1] != b'_' && bytes[i - 1] != b'+' && bytes[i - 1] != b'-';
            let next_ok = i + 1 < bytes.len() && bytes[i + 1] != b'_';
            if !prev_ok || !next_ok {
                return None;
            }
            // skip the underscore
        } else {
            out.push(b as char);
        }
    }
    Some(out)
}

/// Convert string to integer with given base.
/// s: pointer to StrObj
/// base: numeric base (0 for prefix auto-detect, or 2..=36)
/// Returns: a tagged int `Value` — a fixnum when it fits, else a heap `BigInt`
/// (so large literals like `int("1" * 100)` honour arbitrary precision, A6).
pub fn rt_str_to_int_with_base(s: *mut Obj, base: i64) -> *mut Obj {
    if s.is_null() {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "invalid literal for int()"
            );
        }
    }

    unsafe {
        let str_obj = s as *mut StrObj;
        let len = (*str_obj).len;
        let data = (*str_obj).data.as_ptr();
        let bytes = std::slice::from_raw_parts(data, len);

        if let Ok(s_str) = std::str::from_utf8(bytes) {
            let trimmed = s_str.trim();

            // Handle prefixes for auto-detection when base is 0
            let (actual_base, trimmed_str) = if base == 0 {
                if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
                    (16, &trimmed[2..])
                } else if trimmed.starts_with("0b") || trimmed.starts_with("0B") {
                    (2, &trimmed[2..])
                } else if trimmed.starts_with("0o") || trimmed.starts_with("0O") {
                    (8, &trimmed[2..])
                } else {
                    (10, trimmed)
                }
            } else {
                // Only strip a prefix when it matches the requested base.
                // CPython raises ValueError for mismatches such as int("0b10", 16).
                let trimmed_str = match base {
                    16 if trimmed.starts_with("0x") || trimmed.starts_with("0X") => &trimmed[2..],
                    2 if trimmed.starts_with("0b") || trimmed.starts_with("0B") => &trimmed[2..],
                    8 if trimmed.starts_with("0o") || trimmed.starts_with("0O") => &trimmed[2..],
                    _ => trimmed,
                };
                (base as u32, trimmed_str)
            };

            if !(2..=36).contains(&actual_base) {
                raise_exc!(
                    crate::exceptions::ExceptionType::ValueError,
                    "int() base must be >= 2 and <= 36, or 0"
                );
            }

            // Strip digit-group underscores, then parse as a BigInt so values
            // beyond i64 range succeed (CPython int is arbitrary precision).
            let cleaned = strip_int_underscores(trimmed_str);
            let parsed = cleaned
                .as_deref()
                .filter(|c| !c.is_empty())
                .and_then(|c| BigInt::parse_bytes(c.as_bytes(), actual_base));
            match parsed {
                Some(big) => make_int_value(big),
                None => {
                    raise_exc!(
                        crate::exceptions::ExceptionType::ValueError,
                        "invalid literal for int() with base"
                    );
                }
            }
        } else {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "invalid literal for int()"
            );
        }
    }
}
#[export_name = "rt_str_to_int_with_base"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_to_int_with_base_abi(s: Value, base: i64) -> Value {
    Value::from_ptr(rt_str_to_int_with_base(s.unwrap_ptr(), base))
}

// Suppress unused import warning
#[allow(unused_imports)]
use ObjHeader as _;
