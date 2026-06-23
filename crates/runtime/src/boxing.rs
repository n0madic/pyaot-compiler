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
    // The immediate None tag is not a pointer; `unwrap_ptr` on it would
    // fabricate a garbage address. The CHECKED-unbox contract (Phase 8H,
    // D3) is TypeError on any non-numeric tag — same as `rt_unbox_int`.
    if obj.is_none() {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::TypeError,
                "must be real number, not NoneType"
            );
        }
    }
    // A heap `BigInt` is the int→float numeric tower above fixnum range:
    // `float(huge_int)` rounds to the nearest f64, returning ±inf on
    // overflow — exactly `num_bigint::BigInt::to_f64`'s semantics (matches
    // CPython). Mirrors `rt_unbox_int_abi`'s `BigIntObj` access; without this
    // arm `rt_unbox_float` below would TypeError on a legitimate big int.
    // Use `unwrap_ptr_or_null` (not `unwrap_ptr`): the only remaining immediate
    // tag here is RESERVED/UNBOUND (0b111), which `unwrap_ptr` would fabricate
    // into a wild address `0x7` and then dereference. Map it to null and raise
    // a clean `TypeError`, matching the checked-unbox contract (B18).
    let ptr: *mut Obj = obj.unwrap_ptr_or_null();
    unsafe {
        if ptr.is_null() {
            raise_exc!(
                crate::exceptions::ExceptionType::TypeError,
                "must be real number, not NoneType"
            );
        }
        if (*ptr).header.type_tag == TypeTagKind::BigInt {
            use crate::object::BigIntObj;
            use num_traits::ToPrimitive;
            let big = &(*(ptr as *mut BigIntObj)).value;
            return big.to_f64().unwrap_or(f64::INFINITY);
        }
    }
    rt_unbox_float(ptr)
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
    // `unwrap_ptr_or_null` maps the RESERVED/UNBOUND immediate (0b111) to null
    // instead of fabricating a wild `0x7` address that the deref below would
    // SEGV on; the null branch then raises the checked-unbox TypeError (B18).
    let obj: *mut Obj = v.unwrap_ptr_or_null();
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
            return if sign == num_bigint::Sign::Minus {
                low.wrapping_neg()
            } else {
                low
            };
        }
        raise_exc!(
            crate::exceptions::ExceptionType::TypeError,
            "an integer is required, got {}",
            tag.type_name()
        );
    }
}

/// Unbox a bool from a Tagged Value at a CHECKED raw-ABI boundary — the third
/// checked-unbox shape (`Tagged → Raw(I8)`), completing the
/// `rt_unbox_float` / `rt_unbox_int` family (Phase 1 of the `test_functions.py`
/// lift; PITFALLS B18). STRICT, unlike `rt_unbox_int` (which coerces bool→int):
/// a `: bool` slot is a contract, so only a tagged bool unboxes — an int, float,
/// None, or any heap object raises `TypeError` rather than silently truncating
/// (a `Dyn` int into a `: bool` slot is a contract violation, not a coercion;
/// the gradual `Dyn` success case at run time is genuinely a bool).
#[export_name = "rt_unbox_bool"]
pub extern "C" fn rt_unbox_bool_abi(v: Value) -> i8 {
    if v.is_bool() {
        return if v.unwrap_bool() { 1 } else { 0 };
    }
    let got = if v.is_int() {
        "int"
    } else if v.is_none() {
        "NoneType"
    } else {
        let obj: *mut Obj = v.unwrap_ptr_or_null();
        if obj.is_null() {
            "NoneType"
        } else {
            unsafe { (*obj).header.type_tag.type_name() }
        }
    };
    unsafe {
        raise_exc!(
            crate::exceptions::ExceptionType::TypeError,
            "a bool is required, got {}",
            got
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
