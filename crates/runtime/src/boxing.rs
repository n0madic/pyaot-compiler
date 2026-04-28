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
/// # Panics
/// Panics if `obj` is null or has wrong type tag. This catches type confusion
/// bugs in both debug and release builds.
pub fn rt_unbox_float(obj: *mut Obj) -> f64 {
    if obj.is_null() {
        return 0.0;
    }

    unsafe {
        let actual_tag = (*obj).header.type_tag;
        if actual_tag != TypeTagKind::Float {
            panic!("rt_unbox_float: expected Float, got {:?}", actual_tag);
        }
        let float_obj = obj as *mut FloatObj;
        (*float_obj).value
    }
}
#[export_name = "rt_unbox_float"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_unbox_float_abi(obj: Value) -> f64 {
    rt_unbox_float(obj.unwrap_ptr())
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

