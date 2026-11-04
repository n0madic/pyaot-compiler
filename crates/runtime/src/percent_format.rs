//! printf-style string formatting: `str % args` (CPython's
//! `PyUnicode_Format`).
//!
//! Supports the full conversion-spec grammar `%[(key)][flags][width][.prec]type`
//! with `*` width/precision, the flags `#0- +`, and the conversions
//! `d i u o x X e E f F g G c r s a %`. `args` is a tuple (positional), a
//! dict (when the format uses `%(key)` mappings), or a single value.

use crate::exceptions::ExceptionType;
use crate::object::{BigIntObj, FloatObj, Obj, TupleObj, TypeTagKind};
use num_bigint::BigInt;
use num_traits::{Signed, Zero};
use pyaot_core_defs::Value;

/// Read a `StrObj` as a Rust `String`.
unsafe fn str_of(obj: *mut Obj) -> String {
    let data = crate::string::rt_str_data(obj);
    let len = crate::string::rt_str_len(obj);
    String::from_utf8_lossy(std::slice::from_raw_parts(data, len)).into_owned()
}

unsafe fn type_err(msg: String) -> ! {
    crate::raise_exc!(ExceptionType::TypeError, "{}", msg)
}

/// The Python type name of a value (for error messages).
unsafe fn type_name_of(v: *mut Obj) -> &'static str {
    let val = Value(v as u64);
    let tag = if val.is_ptr() && !v.is_null() {
        (*v).type_tag()
    } else if v.is_null() {
        TypeTagKind::None
    } else {
        val.primitive_type().unwrap_or(TypeTagKind::None)
    };
    crate::ops::comparison_type_name(tag)
}

/// An integer operand for `%d`/`%x`/…: ints, bools, bignums; floats are
/// truncated for the decimal conversions only (CPython).
unsafe fn int_operand(v: *mut Obj, conv: char) -> BigInt {
    let val = Value(v as u64);
    if val.is_int() {
        return BigInt::from(val.unwrap_int());
    }
    if val.is_bool() {
        return BigInt::from(val.unwrap_bool() as i64);
    }
    if val.is_ptr() && !v.is_null() {
        match (*v).type_tag() {
            TypeTagKind::BigInt => return (*(v as *mut BigIntObj)).value.clone(),
            TypeTagKind::Float if matches!(conv, 'd' | 'i' | 'u') => {
                let f = (*(v as *mut FloatObj)).value;
                if !f.is_finite() {
                    crate::raise_exc!(
                        if f.is_nan() {
                            ExceptionType::ValueError
                        } else {
                            ExceptionType::OverflowError
                        },
                        "cannot convert float {} to integer",
                        if f.is_nan() { "NaN" } else { "infinity" }
                    );
                }
                let i = crate::bigint::classify_num(Value::from_ptr(
                    crate::builtins::rt_builtin_int(v),
                ));
                if let Some(crate::bigint::Num::Int(x)) = i {
                    return BigInt::from(x);
                }
                if let Some(crate::bigint::Num::Big(b)) = i {
                    return b;
                }
            }
            TypeTagKind::Instance => {
                if let Some(r) = crate::ops::try_int_dunder(v) {
                    return int_operand(r, conv);
                }
            }
            _ => {}
        }
    }
    if matches!(conv, 'd' | 'i' | 'u') {
        type_err(format!(
            "%{} format: a real number is required, not {}",
            conv,
            type_name_of(v)
        ))
    }
    type_err(format!(
        "%{} format: an integer is required, not {}",
        conv,
        type_name_of(v)
    ))
}

unsafe fn float_operand(v: *mut Obj, conv: char) -> f64 {
    let val = Value(v as u64);
    if val.is_int() {
        return val.unwrap_int() as f64;
    }
    if val.is_bool() {
        return val.unwrap_bool() as i64 as f64;
    }
    if val.is_ptr() && !v.is_null() {
        match (*v).type_tag() {
            TypeTagKind::Float => return (*(v as *mut FloatObj)).value,
            TypeTagKind::BigInt => {
                return crate::bigint::bigint_to_f64(&(*(v as *mut BigIntObj)).value)
            }
            TypeTagKind::Instance => {
                if let Some(r) = crate::ops::try_float_dunder(v) {
                    return float_operand(r, conv);
                }
            }
            _ => {}
        }
    }
    type_err(format!("must be real number, not {}", type_name_of(v)))
}

/// One parsed conversion spec.
struct Spec {
    left: bool,
    zero: bool,
    plus: bool,
    space: bool,
    alt: bool,
    width: usize,
    prec: Option<usize>,
}

/// Pad `body` (whose sign/prefix is `lead`) to the spec width.
fn pad(lead: &str, body: &str, spec: &Spec, zero_ok: bool) -> String {
    let len = lead.chars().count() + body.chars().count();
    if len >= spec.width {
        return format!("{}{}", lead, body);
    }
    let fill = spec.width - len;
    if spec.left {
        format!("{}{}{}", lead, body, " ".repeat(fill))
    } else if spec.zero && zero_ok {
        format!("{}{}{}", lead, "0".repeat(fill), body)
    } else {
        format!("{}{}{}", " ".repeat(fill), lead, body)
    }
}

fn sign_lead(negative: bool, spec: &Spec) -> &'static str {
    if negative {
        "-"
    } else if spec.plus {
        "+"
    } else if spec.space {
        " "
    } else {
        ""
    }
}

unsafe fn format_one(conv: char, arg: *mut Obj, spec: &Spec) -> String {
    match conv {
        's' | 'r' | 'a' => {
            let s_obj = match conv {
                's' => crate::conversions::rt_obj_to_str(arg),
                'r' => crate::builtins::rt_builtin_repr(arg),
                _ => crate::builtins::rt_builtin_ascii(arg),
            };
            let mut s = str_of(s_obj);
            if let Some(p) = spec.prec {
                s = s.chars().take(p).collect();
            }
            pad("", &s, spec, false)
        }
        'd' | 'i' | 'u' | 'o' | 'x' | 'X' => {
            let n = int_operand(arg, conv);
            let radix = match conv {
                'o' => 8,
                'x' | 'X' => 16,
                _ => 10,
            };
            let mut digits = n.abs().to_str_radix(radix);
            if conv == 'X' {
                digits = digits.to_uppercase();
            }
            if let Some(p) = spec.prec {
                while digits.len() < p {
                    digits.insert(0, '0');
                }
            }
            let prefix = match (spec.alt, conv) {
                (true, 'o') => "0o",
                (true, 'x') => "0x",
                (true, 'X') => "0X",
                _ => "",
            };
            let lead = format!(
                "{}{}",
                sign_lead(n.is_negative() && !n.is_zero(), spec),
                prefix
            );
            pad(&lead, &digits, spec, true)
        }
        'e' | 'E' | 'f' | 'F' | 'g' | 'G' => {
            let f = float_operand(arg, conv);
            let negative = f.is_sign_negative() && !f.is_nan();
            let body = if f.is_nan() {
                "nan".to_string()
            } else if f.is_infinite() {
                "inf".to_string()
            } else {
                pyaot_format_shared::float_body(
                    f.abs(),
                    conv,
                    spec.prec.unwrap_or(6),
                    spec.alt,
                    false,
                )
            };
            let body = if conv.is_ascii_uppercase() {
                body.to_uppercase()
            } else {
                body
            };
            pad(sign_lead(negative, spec), &body, spec, f.is_finite())
        }
        'c' => {
            let val = Value(arg as u64);
            let s = if val.is_ptr() && !arg.is_null() && (*arg).type_tag() == TypeTagKind::Str {
                let s = str_of(arg);
                if s.chars().count() != 1 {
                    type_err(
                        "%c requires an int or a unicode character, not a string of length "
                            .to_string()
                            + &s.chars().count().to_string(),
                    )
                }
                s
            } else {
                let n = int_operand(arg, 'c');
                match num_traits::ToPrimitive::to_u32(&n).and_then(char::from_u32) {
                    Some(c) => c.to_string(),
                    None => crate::raise_exc!(
                        ExceptionType::OverflowError,
                        "%c arg not in range(0x110000)"
                    ),
                }
            };
            pad("", &s, spec, false)
        }
        _ => crate::raise_exc!(
            ExceptionType::ValueError,
            "unsupported format character '{}' (0x{:x})",
            conv,
            conv as u32
        ),
    }
}

/// Is `obj` a mapping usable for `%(key)` lookups?
unsafe fn is_mapping(obj: *mut Obj) -> bool {
    let v = Value(obj as u64);
    v.is_ptr()
        && !obj.is_null()
        && matches!(
            (*obj).type_tag(),
            TypeTagKind::Dict | TypeTagKind::DefaultDict | TypeTagKind::Counter
        )
}

/// `fmt % args` → the formatted string.
///
/// # Safety
/// `fmt` must be a valid `StrObj`; `args` null, a tagged immediate, or a valid
/// object pointer. Both must stay rooted by the caller.
pub unsafe fn percent_format(fmt: *mut Obj, args: *mut Obj) -> *mut Obj {
    let f: Vec<char> = str_of(fmt).chars().collect();
    let av = Value(args as u64);
    let is_tuple = av.is_ptr() && !args.is_null() && (*args).type_tag() == TypeTagKind::Tuple;
    let (items, n_items): (*const Value, usize) = if is_tuple {
        let t = args as *mut TupleObj;
        ((*t).data.as_ptr(), (*t).len)
    } else {
        (std::ptr::null(), 1)
    };
    let mapping = !is_tuple && is_mapping(args);
    let mut next = 0usize;
    let mut used_mapping = false;
    let mut out = String::with_capacity(f.len() + 16);

    let take_arg = |next: &mut usize| -> *mut Obj {
        if *next >= n_items {
            type_err("not enough arguments for format string".to_string())
        }
        let a = if is_tuple {
            (*items.add(*next)).0 as *mut Obj
        } else {
            args
        };
        *next += 1;
        a
    };

    let mut i = 0;
    while i < f.len() {
        let c = f[i];
        if c != '%' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        if i >= f.len() {
            crate::raise_exc!(ExceptionType::ValueError, "incomplete format");
        }
        // %(key)
        let mut keyed: Option<*mut Obj> = None;
        if f[i] == '(' {
            if !mapping {
                type_err("format requires a mapping".to_string())
            }
            let mut depth = 1;
            let start = i + 1;
            i += 1;
            while i < f.len() && depth > 0 {
                match f[i] {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                i += 1;
            }
            if depth > 0 {
                crate::raise_exc!(ExceptionType::ValueError, "incomplete format key");
            }
            let key: String = f[start..i - 1].iter().collect();
            let key_obj = crate::string::rt_make_str(key.as_ptr(), key.len());
            let v = crate::dict::rt_dict_get(args, key_obj);
            if v.is_null() {
                crate::raise_exc!(ExceptionType::KeyError, "'{}'", key);
            }
            keyed = Some(v);
            used_mapping = true;
        }
        let mut spec = Spec {
            left: false,
            zero: false,
            plus: false,
            space: false,
            alt: false,
            width: 0,
            prec: None,
        };
        while i < f.len() {
            match f[i] {
                '-' => spec.left = true,
                '0' => spec.zero = true,
                '+' => spec.plus = true,
                ' ' => spec.space = true,
                '#' => spec.alt = true,
                _ => break,
            }
            i += 1;
        }
        // width
        if i < f.len() && f[i] == '*' {
            let w = int_operand(take_arg(&mut next), 'd');
            let w = num_traits::ToPrimitive::to_i64(&w).unwrap_or(0);
            if w < 0 {
                spec.left = true;
            }
            spec.width = w.unsigned_abs() as usize;
            i += 1;
        } else {
            while i < f.len() && f[i].is_ascii_digit() {
                spec.width = spec.width * 10 + f[i].to_digit(10).unwrap() as usize;
                i += 1;
            }
        }
        // precision
        if i < f.len() && f[i] == '.' {
            i += 1;
            if i < f.len() && f[i] == '*' {
                let p = int_operand(take_arg(&mut next), 'd');
                spec.prec = Some(num_traits::ToPrimitive::to_i64(&p).unwrap_or(0).max(0) as usize);
                i += 1;
            } else {
                let mut p = 0usize;
                while i < f.len() && f[i].is_ascii_digit() {
                    p = p * 10 + f[i].to_digit(10).unwrap() as usize;
                    i += 1;
                }
                spec.prec = Some(p);
            }
        }
        // length modifiers (ignored, as CPython)
        while i < f.len() && matches!(f[i], 'h' | 'l' | 'L') {
            i += 1;
        }
        if i >= f.len() {
            crate::raise_exc!(ExceptionType::ValueError, "incomplete format");
        }
        let conv = f[i];
        i += 1;
        if conv == '%' {
            out.push('%');
            continue;
        }
        // A mapping used positionally (`'%s' % {...}`) is the single argument.
        let arg = match keyed {
            Some(v) => v,
            None => take_arg(&mut next),
        };
        out.push_str(&format_one(conv, arg, &spec));
    }
    if next < n_items && !used_mapping && !(mapping && next == 0) {
        type_err("not all arguments converted during string formatting".to_string())
    }
    crate::string::rt_make_str(out.as_ptr(), out.len())
}
