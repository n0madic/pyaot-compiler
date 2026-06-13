//! Arithmetic operations for Python runtime (int, float, and boxed Union arithmetic)

use super::dunder_dispatch::{
    either_is_instance, try_class_dunder, try_class_unary_dunder, FNV_ADD, FNV_FLOORDIV,
    FNV_INVERT, FNV_MATMUL, FNV_MOD, FNV_MUL, FNV_NEG, FNV_POS, FNV_POW, FNV_RADD, FNV_RFLOORDIV,
    FNV_RMATMUL, FNV_RMOD, FNV_RMUL, FNV_RPOW, FNV_RSUB, FNV_RTRUEDIV, FNV_SUB, FNV_TRUEDIV,
};
use crate::exceptions::ExceptionType;
use crate::object::{Obj, TypeTagKind};
use pyaot_core_defs::Value;

/// The runtime type tag of a value (reading the header for heap pointers).
#[inline]
unsafe fn tag_of(v: Value, p: *mut Obj) -> TypeTagKind {
    if v.is_ptr() {
        (*p).type_tag()
    } else {
        v.primitive_type().unwrap()
    }
}

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
// Runtime dispatch for arithmetic on boxed Union values. The numeric tower
// (fixnum / bignum / float, with overflow promotion) lives in `crate::bigint`;
// these ops add the dunder / str-concat / str-repeat handling around it.

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_add(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        // Class instance operand: route through user-defined dunders (CPython
        // §3.3.8 protocol). Polymorphic-`other` parameters in user dunders
        // (typed Union[Self, int, float, bool] by the planner) reach here
        // when the runtime value is actually another Class instance.
        if either_is_instance(a, b) {
            if let Some(result) = try_class_dunder(a, b, FNV_ADD, FNV_RADD) {
                return result;
            }
            raise_exc!(
                ExceptionType::TypeError,
                "unsupported operand type(s) for +: '{}' and '{}'",
                super::comparison::type_name((*a).type_tag()),
                super::comparison::type_name((*b).type_tag())
            );
        }
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
        // Sequence concatenation through the gradual `+` path (CPython `+` on
        // two same-type sequences). The statically-typed paths emit dedicated
        // concat ops directly; this covers the dynamic path (e.g. `+` on two
        // `Dyn`/gradual operands, or inside a lambda with untyped params).
        // Mismatched sequence types (`list + tuple`) fall through to the
        // TypeError below, matching CPython.
        if tag_a == TypeTagKind::List && tag_b == TypeTagKind::List {
            return crate::list::rt_list_concat(a, b);
        }
        if tag_a == TypeTagKind::Tuple && tag_b == TypeTagKind::Tuple {
            return crate::tuple::rt_tuple_concat(a, b);
        }
        if tag_a == TypeTagKind::Bytes && tag_b == TypeTagKind::Bytes {
            return crate::bytes::rt_bytes_concat(a, b);
        }
        match (
            crate::bigint::classify_num(va),
            crate::bigint::classify_num(vb),
        ) {
            (Some(x), Some(y)) => crate::bigint::num_add(x, y),
            _ => raise_exc!(
                ExceptionType::TypeError,
                "unsupported operand type(s) for +: '{}' and '{}'",
                super::comparison::type_name(tag_a),
                super::comparison::type_name(tag_b)
            ),
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
        if either_is_instance(a, b) {
            if let Some(result) = try_class_dunder(a, b, FNV_SUB, FNV_RSUB) {
                return result;
            }
            raise_exc!(
                ExceptionType::TypeError,
                "unsupported operand type(s) for -: '{}' and '{}'",
                super::comparison::type_name((*a).type_tag()),
                super::comparison::type_name((*b).type_tag())
            );
        }
        let va = Value(a as u64);
        let vb = Value(b as u64);
        match (
            crate::bigint::classify_num(va),
            crate::bigint::classify_num(vb),
        ) {
            (Some(x), Some(y)) => crate::bigint::num_sub(x, y),
            _ => raise_exc!(
                ExceptionType::TypeError,
                "unsupported operand type(s) for -: '{}' and '{}'",
                super::comparison::type_name(tag_of(va, a)),
                super::comparison::type_name(tag_of(vb, b))
            ),
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
        if either_is_instance(a, b) {
            if let Some(result) = try_class_dunder(a, b, FNV_MUL, FNV_RMUL) {
                return result;
            }
            raise_exc!(
                ExceptionType::TypeError,
                "unsupported operand type(s) for *: '{}' and '{}'",
                super::comparison::type_name((*a).type_tag()),
                super::comparison::type_name((*b).type_tag())
            );
        }
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
        match (
            crate::bigint::classify_num(va_tagged),
            crate::bigint::classify_num(vb_tagged),
        ) {
            (Some(x), Some(y)) => crate::bigint::num_mul(x, y),
            _ => raise_exc!(
                ExceptionType::TypeError,
                "unsupported operand type(s) for *: '{}' and '{}'",
                super::comparison::type_name(tag_a),
                super::comparison::type_name(tag_b)
            ),
        }
    }
}
#[export_name = "rt_obj_mul"]
pub extern "C" fn rt_obj_mul_abi(a: Value, b: Value) -> Value {
    Value::from_ptr(rt_obj_mul(a.unwrap_ptr(), b.unwrap_ptr()))
}

/// Matrix multiply `a @ b` (PEP 465). There is no built-in numeric `@`, so this
/// only dispatches the user `__matmul__` / `__rmatmul__` dunder; any other operand
/// pair is a `TypeError`, matching CPython.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_matmul(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        if either_is_instance(a, b) {
            if let Some(result) = try_class_dunder(a, b, FNV_MATMUL, FNV_RMATMUL) {
                return result;
            }
        }
        raise_exc!(
            ExceptionType::TypeError,
            "unsupported operand type(s) for @: '{}' and '{}'",
            super::comparison::type_name(tag_of(Value(a as u64), a)),
            super::comparison::type_name(tag_of(Value(b as u64), b))
        );
    }
}
#[export_name = "rt_obj_matmul"]
pub extern "C" fn rt_obj_matmul_abi(a: Value, b: Value) -> Value {
    Value::from_ptr(rt_obj_matmul(a.unwrap_ptr(), b.unwrap_ptr()))
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_div(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        if either_is_instance(a, b) {
            if let Some(result) = try_class_dunder(a, b, FNV_TRUEDIV, FNV_RTRUEDIV) {
                return result;
            }
            raise_exc!(
                ExceptionType::TypeError,
                "unsupported operand type(s) for /: '{}' and '{}'",
                super::comparison::type_name((*a).type_tag()),
                super::comparison::type_name((*b).type_tag())
            );
        }
        let va = Value(a as u64);
        let vb = Value(b as u64);
        match (
            crate::bigint::classify_num(va),
            crate::bigint::classify_num(vb),
        ) {
            (Some(x), Some(y)) => crate::bigint::num_truediv(x, y),
            _ => raise_exc!(
                ExceptionType::TypeError,
                "unsupported operand type(s) for /: '{}' and '{}'",
                super::comparison::type_name(tag_of(va, a)),
                super::comparison::type_name(tag_of(vb, b))
            ),
        }
    }
}
#[export_name = "rt_obj_div"]
pub extern "C" fn rt_obj_div_abi(a: Value, b: Value) -> Value {
    Value::from_ptr(rt_obj_div(a.unwrap_ptr(), b.unwrap_ptr()))
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_floordiv(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        if either_is_instance(a, b) {
            if let Some(result) = try_class_dunder(a, b, FNV_FLOORDIV, FNV_RFLOORDIV) {
                return result;
            }
            raise_exc!(
                ExceptionType::TypeError,
                "unsupported operand type(s) for //: '{}' and '{}'",
                super::comparison::type_name((*a).type_tag()),
                super::comparison::type_name((*b).type_tag())
            );
        }
        let va = Value(a as u64);
        let vb = Value(b as u64);
        match (
            crate::bigint::classify_num(va),
            crate::bigint::classify_num(vb),
        ) {
            (Some(x), Some(y)) => crate::bigint::num_floordiv(x, y),
            _ => raise_exc!(
                ExceptionType::TypeError,
                "unsupported operand type(s) for //: '{}' and '{}'",
                super::comparison::type_name(tag_of(va, a)),
                super::comparison::type_name(tag_of(vb, b))
            ),
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
        if either_is_instance(a, b) {
            if let Some(result) = try_class_dunder(a, b, FNV_MOD, FNV_RMOD) {
                return result;
            }
            raise_exc!(
                ExceptionType::TypeError,
                "unsupported operand type(s) for %: '{}' and '{}'",
                super::comparison::type_name((*a).type_tag()),
                super::comparison::type_name((*b).type_tag())
            );
        }
        let va = Value(a as u64);
        let vb = Value(b as u64);
        match (
            crate::bigint::classify_num(va),
            crate::bigint::classify_num(vb),
        ) {
            (Some(x), Some(y)) => crate::bigint::num_mod(x, y),
            _ => raise_exc!(
                ExceptionType::TypeError,
                "unsupported operand type(s) for %: '{}' and '{}'",
                super::comparison::type_name(tag_of(va, a)),
                super::comparison::type_name(tag_of(vb, b))
            ),
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
        if either_is_instance(a, b) {
            if let Some(result) = try_class_dunder(a, b, FNV_POW, FNV_RPOW) {
                return result;
            }
            raise_exc!(
                ExceptionType::TypeError,
                "unsupported operand type(s) for **: '{}' and '{}'",
                super::comparison::type_name((*a).type_tag()),
                super::comparison::type_name((*b).type_tag())
            );
        }
        let va = Value(a as u64);
        let vb = Value(b as u64);
        match (
            crate::bigint::classify_num(va),
            crate::bigint::classify_num(vb),
        ) {
            (Some(x), Some(y)) => crate::bigint::num_pow(x, y),
            _ => raise_exc!(
                ExceptionType::TypeError,
                "unsupported operand type(s) for **: '{}' and '{}'",
                super::comparison::type_name(tag_of(va, a)),
                super::comparison::type_name(tag_of(vb, b))
            ),
        }
    }
}
#[export_name = "rt_obj_pow"]
pub extern "C" fn rt_obj_pow_abi(a: Value, b: Value) -> Value {
    Value::from_ptr(rt_obj_pow(a.unwrap_ptr(), b.unwrap_ptr()))
}

// ==================== Bitwise / shift (bignum-aware, Tagged baseline) ==========
//
// `& | ^ << >>` route through here (not a raw `i64` op) because an `int` operand
// may dynamically be a heap `BigInt` — unboxing it to raw bits would be a silent
// miscompile. A range-proven raw fast path is a future optimization.

macro_rules! bitwise_op {
    ($name:ident, $abi:ident, $export:literal, $num:ident) => {
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub fn $name(a: *mut Obj, b: *mut Obj) -> *mut Obj {
            unsafe {
                let va = Value(a as u64);
                let vb = Value(b as u64);
                match (
                    crate::bigint::classify_num(va),
                    crate::bigint::classify_num(vb),
                ) {
                    (Some(x), Some(y)) => crate::bigint::$num(x, y),
                    _ => raise_exc!(
                        ExceptionType::TypeError,
                        "unsupported operand type(s): '{}' and '{}'",
                        super::comparison::type_name(tag_of(va, a)),
                        super::comparison::type_name(tag_of(vb, b))
                    ),
                }
            }
        }
        #[export_name = $export]
        pub extern "C" fn $abi(a: Value, b: Value) -> Value {
            Value::from_ptr($name(a.unwrap_ptr(), b.unwrap_ptr()))
        }
    };
}

bitwise_op!(
    rt_obj_bitand,
    rt_obj_bitand_abi,
    "rt_obj_bitand",
    num_bitand
);
bitwise_op!(rt_obj_bitor, rt_obj_bitor_abi, "rt_obj_bitor", num_bitor);
bitwise_op!(
    rt_obj_bitxor,
    rt_obj_bitxor_abi,
    "rt_obj_bitxor",
    num_bitxor
);
bitwise_op!(rt_obj_lshift, rt_obj_lshift_abi, "rt_obj_lshift", num_shl);
bitwise_op!(rt_obj_rshift, rt_obj_rshift_abi, "rt_obj_rshift", num_shr);

// ==================== Unary obj operations (Union dispatch) ====================
//
// Generic unary helpers for Union-typed operands. The lowering routes
// `-x` / `+x` / `~x` here when `x` has a static type that can hold either
// a primitive (Int / Bool / Float) or a class instance — typical for
// `-self.data` inside dunders where the planner widens `data` to
// `Union[Float, Self]`. CPython's protocol: dispatch to the class's
// `__neg__` / `__pos__` / `__invert__` when the runtime value is a class
// instance, otherwise apply primitive negation/identity/bitwise-not.

unsafe fn classify_unary(a: *mut Obj) -> TypeTagKind {
    let va = Value(a as u64);
    if va.is_int() {
        TypeTagKind::Int
    } else if va.is_bool() {
        TypeTagKind::Bool
    } else if va.is_ptr() && !a.is_null() {
        (*a).type_tag()
    } else {
        TypeTagKind::None
    }
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_neg(a: *mut Obj) -> *mut Obj {
    unsafe {
        if let Some(result) = try_class_unary_dunder(a, FNV_NEG) {
            return result;
        }
        match crate::bigint::classify_num(Value(a as u64)) {
            Some(n) => crate::bigint::num_neg(n),
            None => raise_exc!(
                ExceptionType::TypeError,
                "bad operand type for unary -: '{}'",
                super::comparison::type_name(classify_unary(a))
            ),
        }
    }
}
#[export_name = "rt_obj_neg"]
pub extern "C" fn rt_obj_neg_abi(a: Value) -> Value {
    Value::from_ptr(rt_obj_neg(a.unwrap_ptr()))
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_pos(a: *mut Obj) -> *mut Obj {
    unsafe {
        if let Some(result) = try_class_unary_dunder(a, FNV_POS) {
            return result;
        }
        let tag = classify_unary(a);
        match tag {
            // CPython: unary `+` on a bool yields an int (`+True == 1`,
            // `+False == 0`) — mirror `rt_obj_neg`, which already promotes via the
            // numeric tower. Int / Float / BigInt are returned unchanged (`+x == x`).
            TypeTagKind::Bool => {
                Value::from_int(Value(a as u64).unwrap_bool() as i64).0 as *mut Obj
            }
            TypeTagKind::Int | TypeTagKind::Float | TypeTagKind::BigInt => a,
            other => raise_exc!(
                ExceptionType::TypeError,
                "bad operand type for unary +: '{}'",
                super::comparison::type_name(other)
            ),
        }
    }
}
#[export_name = "rt_obj_pos"]
pub extern "C" fn rt_obj_pos_abi(a: Value) -> Value {
    Value::from_ptr(rt_obj_pos(a.unwrap_ptr()))
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_invert(a: *mut Obj) -> *mut Obj {
    unsafe {
        if let Some(result) = try_class_unary_dunder(a, FNV_INVERT) {
            return result;
        }
        match crate::bigint::classify_num(Value(a as u64)) {
            Some(n) => crate::bigint::num_invert(n),
            None => raise_exc!(
                ExceptionType::TypeError,
                "bad operand type for unary ~: '{}'",
                super::comparison::type_name(classify_unary(a))
            ),
        }
    }
}
#[export_name = "rt_obj_invert"]
pub extern "C" fn rt_obj_invert_abi(a: Value) -> Value {
    Value::from_ptr(rt_obj_invert(a.unwrap_ptr()))
}
