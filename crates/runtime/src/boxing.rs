//! Boxing and unboxing operations for primitive types.
//!
//! Int, Bool, None are immediate tagged Values (no heap allocation),
//! produced inline at MIR-codegen via `ValueFromInt` / `ValueFromBool`
//! and unwrapped via `UnwrapValueInt` / `UnwrapValueBool`. Float
//! remains heap-boxed as `FloatObj` and uses the extern shims
//! `rt_box_float` / `rt_unbox_float`. None uses the singleton
//! `rt_box_none`.

use crate::gc;
use crate::object::{FloatObj, Obj, TypeTagKind};
use pyaot_core_defs::Value;

/// Box a float value as a heap-allocated FloatObj
/// Used for list elements when the element type is float
pub fn rt_box_float(value: f64) -> *mut Obj {
    let size = std::mem::size_of::<FloatObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Float as u8);

    unsafe {
        let float_obj = obj as *mut FloatObj;
        (*float_obj).value = value;
    }

    obj
}
#[export_name = "rt_box_float"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_box_float_abi(value: f64) -> Value {
    Value::from_ptr(rt_box_float(value))
}

/// Unbox a float value from a heap-allocated FloatObj
/// Used for list elements when the element type is float
///
/// Raises `TypeError` if `obj` has the wrong type tag. Returns 0.0 for null
/// (matching zero-initialisation semantics for empty Union/Optional slots).
pub fn rt_unbox_float(obj: *mut Obj) -> f64 {
    if obj.is_null() {
        return 0.0;
    }

    unsafe {
        let actual_tag = (*obj).header.type_tag;
        if actual_tag != TypeTagKind::Float {
            raise_exc!(
                crate::exceptions::ExceptionType::TypeError,
                "rt_unbox_float: expected Float, got {}",
                actual_tag.type_name()
            );
        }
        let float_obj = obj as *mut FloatObj;
        (*float_obj).value
    }
}
#[export_name = "rt_unbox_float"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_unbox_float_abi(obj: Value) -> f64 {
    // Union slots carry tagged Values: int/bool arrive tagged, floats as *mut FloatObj.
    if obj.is_int() {
        return obj.unwrap_int() as f64;
    }
    if obj.is_bool() {
        return if obj.unwrap_bool() { 1.0 } else { 0.0 };
    }
    rt_unbox_float(obj.unwrap_ptr())
}

/// Unbox an int from a Tagged Value at a CHECKED stdlib raw-ABI boundary
/// (Phase 8H, D3): a tagged fixnum unwraps, a bool converts to 0/1, a heap
/// `BigInt` truncates to its low 64 bits (the same wrapping semantics a
/// fixnum `UntagInt` shift would give), anything else raises `TypeError`.
#[export_name = "rt_unbox_int"]
pub extern "C" fn rt_unbox_int_abi(v: Value) -> i64 {
    use crate::object::BigIntObj;
    if v.is_int() {
        return v.unwrap_int();
    }
    if v.is_bool() {
        return if v.unwrap_bool() { 1 } else { 0 };
    }
    let obj: *mut Obj = v.unwrap_ptr();
    unsafe {
        if obj.is_null() {
            raise_exc!(
                crate::exceptions::ExceptionType::TypeError,
                "an integer is required, got NoneType"
            );
        }
        let tag = (*obj).header.type_tag;
        if tag == TypeTagKind::BigInt {
            let big = &(*(obj as *mut BigIntObj)).value;
            let (sign, digits) = big.to_u64_digits();
            let low = digits.first().copied().unwrap_or(0) as i64;
            return if sign == num_bigint::Sign::Minus { low.wrapping_neg() } else { low };
        }
        raise_exc!(
            crate::exceptions::ExceptionType::TypeError,
            "an integer is required, got {}",
            tag.type_name()
        );
    }
}

/// Box None as a heap-allocated NoneObj
/// Used for Union types when the value is None
pub fn rt_box_none() -> *mut Obj {
    crate::object::none_obj()
}
#[export_name = "rt_box_none"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_box_none_abi() -> Value {
    Value::from_ptr(rt_box_none())
}
