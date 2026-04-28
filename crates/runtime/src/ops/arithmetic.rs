//! Arithmetic operations for Python runtime (int, float, and boxed Union arithmetic)

use crate::exceptions::ExceptionType;
use crate::object::{FloatObj, Obj, TypeTagKind};
use pyaot_core_defs::Value;

// ==================== Primitive int arithmetic ====================

/// Add two integers
#[no_mangle]
pub extern "C" fn rt_add_int(a: i64, b: i64) -> i64 {
    match a.checked_add(b) {
        Some(value) => value,
        None => unsafe { raise_exc!(ExceptionType::OverflowError, "integer overflow") },
    }
}

/// Subtract two integers
#[no_mangle]
pub extern "C" fn rt_sub_int(a: i64, b: i64) -> i64 {
    match a.checked_sub(b) {
        Some(value) => value,
        None => unsafe { raise_exc!(ExceptionType::OverflowError, "integer overflow") },
    }
}

/// Multiply two integers
#[no_mangle]
pub extern "C" fn rt_mul_int(a: i64, b: i64) -> i64 {
    match a.checked_mul(b) {
        Some(value) => value,
        None => unsafe { raise_exc!(ExceptionType::OverflowError, "integer overflow") },
    }
}

/// Divide two integers (Python-style floor division)
#[no_mangle]
pub extern "C" fn rt_div_int(a: i64, b: i64) -> i64 {
    if b == 0 {
        unsafe { raise_exc!(ExceptionType::ZeroDivisionError, "division by zero") }
    }
    if a == i64::MIN && b == -1 {
        unsafe { raise_exc!(ExceptionType::OverflowError, "integer overflow") }
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
        unsafe { raise_exc!(ExceptionType::ZeroDivisionError, "division by zero") }
    }
    (a as f64) / (b as f64)
}

/// Modulo two integers
#[no_mangle]
pub extern "C" fn rt_mod_int(a: i64, b: i64) -> i64 {
    if b == 0 {
        unsafe { raise_exc!(ExceptionType::ZeroDivisionError, "integer modulo by zero") }
    }
    if a == i64::MIN && b == -1 {
        unsafe { raise_exc!(ExceptionType::OverflowError, "integer overflow") }
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
    let va = Value(a as u64);
    let vb = Value(b as u64);
    let tag_a = if va.is_ptr() {
        (*a).type_tag()
    } else {
        va.primitive_type().unwrap()
    };
    let tag_b = if vb.is_ptr() {
        (*b).type_tag()
    } else {
        vb.primitive_type().unwrap()
    };
    let va_int = if tag_a == TypeTagKind::Int {
        if va.is_int() {
            va.unwrap_int()
        } else {
            0
        }
    } else {
        0
    };
    let vb_int = if tag_b == TypeTagKind::Int {
        if vb.is_int() {
            vb.unwrap_int()
        } else {
            0
        }
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

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_add(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let (va_f, vb_f, both_int, vai, vbi) = extract_numeric_pair(a, b);
        // Reconstruct type tags using Value so tagged primitives are handled correctly.
        let va = Value(a as u64);
        let vb = Value(b as u64);
        let tag_a = if va.is_ptr() {
            (*a).type_tag()
        } else {
            va.primitive_type().unwrap()
        };
        let tag_b = if vb.is_ptr() {
            (*b).type_tag()
        } else {
            vb.primitive_type().unwrap()
        };
        // String concatenation (only possible for heap Str objects)
        if tag_a == TypeTagKind::Str && tag_b == TypeTagKind::Str {
            return crate::string::rt_str_concat(a, b);
        }
        if both_int {
            match vai.checked_add(vbi) {
                Some(v) => Value::from_int(v).0 as *mut crate::object::Obj,
                None => {
                    raise_exc!(ExceptionType::OverflowError, "integer overflow");
                }
            }
        } else if (tag_a == TypeTagKind::Int || tag_a == TypeTagKind::Float)
            && (tag_b == TypeTagKind::Int || tag_b == TypeTagKind::Float)
        {
            crate::boxing::rt_box_float(va_f + vb_f)
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
#[export_name = "rt_obj_add"]
pub extern "C" fn rt_obj_add_abi(a: Value, b: Value) -> Value {
    Value::from_ptr(rt_obj_add(a.unwrap_ptr(), b.unwrap_ptr()))
}


#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_sub(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let (va, vb, both_int, vai, vbi) = extract_numeric_pair(a, b);
        if both_int {
            match vai.checked_sub(vbi) {
                Some(v) => Value::from_int(v).0 as *mut crate::object::Obj,
                None => {
                    raise_exc!(ExceptionType::OverflowError, "integer overflow");
                }
            }
        } else {
            crate::boxing::rt_box_float(va - vb)
        }
    }
}
#[export_name = "rt_obj_sub"]
pub extern "C" fn rt_obj_sub_abi(a: Value, b: Value) -> Value {
    Value::from_ptr(rt_obj_sub(a.unwrap_ptr(), b.unwrap_ptr()))
}


#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_mul(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let va_tagged = Value(a as u64);
        let vb_tagged = Value(b as u64);
        let tag_a = if va_tagged.is_ptr() {
            (*a).type_tag()
        } else {
            va_tagged.primitive_type().unwrap()
        };
        let tag_b = if vb_tagged.is_ptr() {
            (*b).type_tag()
        } else {
            vb_tagged.primitive_type().unwrap()
        };
        // String repetition: str * int or int * str
        if tag_a == TypeTagKind::Str && tag_b == TypeTagKind::Int {
            let count = if vb_tagged.is_int() {
                vb_tagged.unwrap_int()
            } else {
                0
            };
            return crate::string::rt_str_mul(a, count);
        }
        if tag_a == TypeTagKind::Int && tag_b == TypeTagKind::Str {
            let count = if va_tagged.is_int() {
                va_tagged.unwrap_int()
            } else {
                0
            };
            return crate::string::rt_str_mul(b, count);
        }
        let (va, vb, both_int, vai, vbi) = extract_numeric_pair(a, b);
        if both_int {
            match vai.checked_mul(vbi) {
                Some(v) => Value::from_int(v).0 as *mut crate::object::Obj,
                None => {
                    raise_exc!(ExceptionType::OverflowError, "integer overflow");
                }
            }
        } else {
            crate::boxing::rt_box_float(va * vb)
        }
    }
}
#[export_name = "rt_obj_mul"]
pub extern "C" fn rt_obj_mul_abi(a: Value, b: Value) -> Value {
    Value::from_ptr(rt_obj_mul(a.unwrap_ptr(), b.unwrap_ptr()))
}


#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_div(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let (va, vb, _, _, _) = extract_numeric_pair(a, b);
        if vb == 0.0 {
            raise_exc!(ExceptionType::ZeroDivisionError, "division by zero");
        }
        crate::boxing::rt_box_float(va / vb) // Python 3: true division always float
    }
}
#[export_name = "rt_obj_div"]
pub extern "C" fn rt_obj_div_abi(a: Value, b: Value) -> Value {
    Value::from_ptr(rt_obj_div(a.unwrap_ptr(), b.unwrap_ptr()))
}


#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_floordiv(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let (va, vb, both_int, vai, vbi) = extract_numeric_pair(a, b);
        if both_int {
            if vbi == 0 {
                raise_exc!(
                    ExceptionType::ZeroDivisionError,
                    "integer division or modulo by zero"
                );
            }
            if vai == i64::MIN && vbi == -1 {
                raise_exc!(ExceptionType::OverflowError, "integer overflow");
            }
            let d = vai / vbi;
            let r = vai % vbi;
            let result = if r != 0 && (r ^ vbi) < 0 { d - 1 } else { d };
            Value::from_int(result).0 as *mut crate::object::Obj
        } else {
            if vb == 0.0 {
                raise_exc!(
                    ExceptionType::ZeroDivisionError,
                    "integer division or modulo by zero"
                );
            }
            crate::boxing::rt_box_float((va / vb).floor())
        }
    }
}
#[export_name = "rt_obj_floordiv"]
pub extern "C" fn rt_obj_floordiv_abi(a: Value, b: Value) -> Value {
    Value::from_ptr(rt_obj_floordiv(a.unwrap_ptr(), b.unwrap_ptr()))
}


#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_mod(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let (va, vb, both_int, vai, vbi) = extract_numeric_pair(a, b);
        if both_int {
            if vbi == 0 {
                raise_exc!(
                    ExceptionType::ZeroDivisionError,
                    "integer division or modulo by zero"
                );
            }
            if vai == i64::MIN && vbi == -1 {
                raise_exc!(ExceptionType::OverflowError, "integer overflow");
            }
            let r = vai % vbi;
            let result = if r != 0 && (r ^ vbi) < 0 { r + vbi } else { r };
            Value::from_int(result).0 as *mut crate::object::Obj
        } else {
            if vb == 0.0 {
                raise_exc!(
                    ExceptionType::ZeroDivisionError,
                    "integer division or modulo by zero"
                );
            }
            crate::boxing::rt_box_float(va % vb)
        }
    }
}
#[export_name = "rt_obj_mod"]
pub extern "C" fn rt_obj_mod_abi(a: Value, b: Value) -> Value {
    Value::from_ptr(rt_obj_mod(a.unwrap_ptr(), b.unwrap_ptr()))
}


#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_pow(a: *mut Obj, b: *mut Obj) -> *mut Obj {
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
                raise_exc!(ExceptionType::OverflowError, "integer overflow");
            }
            Value::from_int(result).0 as *mut crate::object::Obj
        } else {
            crate::boxing::rt_box_float(va.powf(vb))
        }
    }
}
#[export_name = "rt_obj_pow"]
pub extern "C" fn rt_obj_pow_abi(a: Value, b: Value) -> Value {
    Value::from_ptr(rt_obj_pow(a.unwrap_ptr(), b.unwrap_ptr()))
}

