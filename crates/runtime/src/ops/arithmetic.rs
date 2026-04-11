//! Arithmetic operations for Python runtime (int, float, and boxed Union arithmetic)

use crate::exceptions::rt_exc_raise;
use crate::exceptions::ExceptionType;
use crate::object::{FloatObj, IntObj, Obj, TypeTagKind};

// ==================== Primitive int arithmetic ====================

/// Add two integers
#[no_mangle]
pub extern "C" fn rt_add_int(a: i64, b: i64) -> i64 {
    match a.checked_add(b) {
        Some(value) => value,
        None => unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::OverflowError as u8,
                b"integer overflow".as_ptr(),
                b"integer overflow".len(),
            )
        },
    }
}

/// Subtract two integers
#[no_mangle]
pub extern "C" fn rt_sub_int(a: i64, b: i64) -> i64 {
    match a.checked_sub(b) {
        Some(value) => value,
        None => unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::OverflowError as u8,
                b"integer overflow".as_ptr(),
                b"integer overflow".len(),
            )
        },
    }
}

/// Multiply two integers
#[no_mangle]
pub extern "C" fn rt_mul_int(a: i64, b: i64) -> i64 {
    match a.checked_mul(b) {
        Some(value) => value,
        None => unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::OverflowError as u8,
                b"integer overflow".as_ptr(),
                b"integer overflow".len(),
            )
        },
    }
}

/// Divide two integers (Python-style floor division)
#[no_mangle]
pub extern "C" fn rt_div_int(a: i64, b: i64) -> i64 {
    if b == 0 {
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::ZeroDivisionError as u8,
                b"division by zero".as_ptr(),
                b"division by zero".len(),
            )
        }
    }
    if a == i64::MIN && b == -1 {
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::OverflowError as u8,
                b"integer overflow".as_ptr(),
                b"integer overflow".len(),
            )
        }
    }
    // Python floor division: rounds toward negative infinity
    let d = a / b;
    let r = a % b;
    // Adjust when remainder has different sign than divisor
    if r != 0 && (r ^ b) < 0 {
        d - 1
    } else {
        d
    }
}

/// True division of two integers (Python 3 `/` operator)
/// Always returns float, even for integer operands
#[no_mangle]
pub extern "C" fn rt_true_div_int(a: i64, b: i64) -> f64 {
    if b == 0 {
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::ZeroDivisionError as u8,
                b"division by zero".as_ptr(),
                b"division by zero".len(),
            )
        }
    }
    (a as f64) / (b as f64)
}

/// Modulo two integers
#[no_mangle]
pub extern "C" fn rt_mod_int(a: i64, b: i64) -> i64 {
    if b == 0 {
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::ZeroDivisionError as u8,
                b"integer modulo by zero".as_ptr(),
                b"integer modulo by zero".len(),
            )
        }
    }
    if a == i64::MIN && b == -1 {
        unsafe {
            crate::exceptions::rt_exc_raise(
                crate::exceptions::ExceptionType::OverflowError as u8,
                b"integer overflow".as_ptr(),
                b"integer overflow".len(),
            )
        }
    }
    // Python modulo: result has same sign as divisor
    let r = a % b;
    if r != 0 && (r ^ b) < 0 {
        r + b
    } else {
        r
    }
}

// ==================== Primitive float arithmetic ====================

/// Add two floats
#[no_mangle]
pub extern "C" fn rt_add_float(a: f64, b: f64) -> f64 {
    a + b
}

/// Subtract two floats
#[no_mangle]
pub extern "C" fn rt_sub_float(a: f64, b: f64) -> f64 {
    a - b
}

/// Multiply two floats
#[no_mangle]
pub extern "C" fn rt_mul_float(a: f64, b: f64) -> f64 {
    a * b
}

/// Divide two floats
#[no_mangle]
pub extern "C" fn rt_div_float(a: f64, b: f64) -> f64 {
    a / b
}

// ==================== Union Arithmetic Operations ====================
// Runtime dispatch for arithmetic on boxed Union values.
// Returns a boxed result (*mut Obj).

/// Extract numeric values from two boxed objects, promoting int→float if mixed.
/// Returns (left_f64, right_f64, both_int, left_int, right_int)
#[inline]
pub(super) unsafe fn extract_numeric_pair(a: *mut Obj, b: *mut Obj) -> (f64, f64, bool, i64, i64) {
    let tag_a = (*a).type_tag();
    let tag_b = (*b).type_tag();
    let va_int = if tag_a == TypeTagKind::Int {
        (*(a as *mut IntObj)).value
    } else {
        0
    };
    let vb_int = if tag_b == TypeTagKind::Int {
        (*(b as *mut IntObj)).value
    } else {
        0
    };
    let va_f = if tag_a == TypeTagKind::Float {
        (*(a as *mut FloatObj)).value
    } else {
        va_int as f64
    };
    let vb_f = if tag_b == TypeTagKind::Float {
        (*(b as *mut FloatObj)).value
    } else {
        vb_int as f64
    };
    let both_int = tag_a == TypeTagKind::Int && tag_b == TypeTagKind::Int;
    (va_f, vb_f, both_int, va_int, vb_int)
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_add(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let tag_a = (*a).type_tag();
        let tag_b = (*b).type_tag();
        // String concatenation
        if tag_a == TypeTagKind::Str && tag_b == TypeTagKind::Str {
            return crate::string::rt_str_concat(a, b);
        }
        let (va, vb, both_int, vai, vbi) = extract_numeric_pair(a, b);
        if both_int {
            match vai.checked_add(vbi) {
                Some(v) => crate::boxing::rt_box_int(v),
                None => {
                    let msg = b"integer overflow";
                    rt_exc_raise(ExceptionType::OverflowError as u8, msg.as_ptr(), msg.len());
                }
            }
        } else if (tag_a == TypeTagKind::Int || tag_a == TypeTagKind::Float)
            && (tag_b == TypeTagKind::Int || tag_b == TypeTagKind::Float)
        {
            crate::boxing::rt_box_float(va + vb)
        } else {
            crate::raise_exc!(
                ExceptionType::TypeError,
                "unsupported operand type(s) for +: '{}' and '{}'",
                super::comparison::type_name(tag_a),
                super::comparison::type_name(tag_b)
            );
        }
    }
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_sub(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let (va, vb, both_int, vai, vbi) = extract_numeric_pair(a, b);
        if both_int {
            match vai.checked_sub(vbi) {
                Some(v) => crate::boxing::rt_box_int(v),
                None => {
                    let msg = b"integer overflow";
                    rt_exc_raise(ExceptionType::OverflowError as u8, msg.as_ptr(), msg.len());
                }
            }
        } else {
            crate::boxing::rt_box_float(va - vb)
        }
    }
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_mul(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let tag_a = (*a).type_tag();
        let tag_b = (*b).type_tag();
        // String repetition: str * int or int * str
        if tag_a == TypeTagKind::Str && tag_b == TypeTagKind::Int {
            return crate::string::rt_str_mul(a, (*(b as *mut IntObj)).value);
        }
        if tag_a == TypeTagKind::Int && tag_b == TypeTagKind::Str {
            return crate::string::rt_str_mul(b, (*(a as *mut IntObj)).value);
        }
        let (va, vb, both_int, vai, vbi) = extract_numeric_pair(a, b);
        if both_int {
            match vai.checked_mul(vbi) {
                Some(v) => crate::boxing::rt_box_int(v),
                None => {
                    let msg = b"integer overflow";
                    rt_exc_raise(ExceptionType::OverflowError as u8, msg.as_ptr(), msg.len());
                }
            }
        } else {
            crate::boxing::rt_box_float(va * vb)
        }
    }
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_div(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let (va, vb, _, _, _) = extract_numeric_pair(a, b);
        if vb == 0.0 {
            let msg = "division by zero";
            rt_exc_raise(
                ExceptionType::ZeroDivisionError as u8,
                msg.as_ptr(),
                msg.len(),
            );
        }
        crate::boxing::rt_box_float(va / vb) // Python 3: true division always float
    }
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_floordiv(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let (va, vb, both_int, vai, vbi) = extract_numeric_pair(a, b);
        if both_int {
            if vbi == 0 {
                let msg = "integer division or modulo by zero";
                rt_exc_raise(
                    ExceptionType::ZeroDivisionError as u8,
                    msg.as_ptr(),
                    msg.len(),
                );
            }
            if vai == i64::MIN && vbi == -1 {
                let msg = b"integer overflow";
                rt_exc_raise(ExceptionType::OverflowError as u8, msg.as_ptr(), msg.len());
            }
            let d = vai / vbi;
            let r = vai % vbi;
            let result = if r != 0 && (r ^ vbi) < 0 { d - 1 } else { d };
            crate::boxing::rt_box_int(result)
        } else {
            if vb == 0.0 {
                let msg = "integer division or modulo by zero";
                rt_exc_raise(
                    ExceptionType::ZeroDivisionError as u8,
                    msg.as_ptr(),
                    msg.len(),
                );
            }
            crate::boxing::rt_box_float((va / vb).floor())
        }
    }
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_mod(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let (va, vb, both_int, vai, vbi) = extract_numeric_pair(a, b);
        if both_int {
            if vbi == 0 {
                let msg = "integer division or modulo by zero";
                rt_exc_raise(
                    ExceptionType::ZeroDivisionError as u8,
                    msg.as_ptr(),
                    msg.len(),
                );
            }
            if vai == i64::MIN && vbi == -1 {
                let msg = b"integer overflow";
                rt_exc_raise(ExceptionType::OverflowError as u8, msg.as_ptr(), msg.len());
            }
            let r = vai % vbi;
            let result = if r != 0 && (r ^ vbi) < 0 { r + vbi } else { r };
            crate::boxing::rt_box_int(result)
        } else {
            if vb == 0.0 {
                let msg = "integer division or modulo by zero";
                rt_exc_raise(
                    ExceptionType::ZeroDivisionError as u8,
                    msg.as_ptr(),
                    msg.len(),
                );
            }
            crate::boxing::rt_box_float(va % vb)
        }
    }
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_pow(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let (va, vb, both_int, vai, vbi) = extract_numeric_pair(a, b);
        if both_int && vbi >= 0 {
            let exp = vbi as u32;
            let mut result: i64 = 1;
            let mut base = vai;
            let mut e = exp;
            let mut overflow = false;
            while e > 0 {
                if e & 1 == 1 {
                    match result.checked_mul(base) {
                        Some(v) => result = v,
                        None => {
                            overflow = true;
                            break;
                        }
                    }
                }
                e >>= 1;
                if e > 0 {
                    match base.checked_mul(base) {
                        Some(v) => base = v,
                        None => {
                            overflow = true;
                            break;
                        }
                    }
                }
            }
            if overflow {
                let msg = b"integer overflow";
                rt_exc_raise(ExceptionType::OverflowError as u8, msg.as_ptr(), msg.len());
            }
            crate::boxing::rt_box_int(result)
        } else {
            crate::boxing::rt_box_float(va.powf(vb))
        }
    }
}
