//! Generator runtime support
//!
//! Provides runtime functions for generator objects (functions with yield).

use crate::exceptions::ExceptionType;
use crate::gc::gc_alloc;
use crate::object::{GeneratorObj, Obj, TypeTagKind};
use pyaot_core_defs::Value;
use std::mem::size_of;

// External function provided by compiled code - dispatches to the appropriate resume function
extern "C" {
    fn __pyaot_generator_resume(gen: *mut Obj) -> *mut Obj;
}

/// Call next() on a generator - invokes the resume function
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
pub unsafe fn rt_generator_next(gen: *mut Obj) -> *mut Obj {
    // Check if it's actually a generator
    if gen.is_null() || (*gen).header.type_tag != TypeTagKind::Generator {
        raise_exc!(
            ExceptionType::StopIteration,
            "next() called on non-generator"
        );
    }

    // Call the dispatcher function (provided by compiled code)
    __pyaot_generator_resume(gen)
}
#[export_name = "rt_generator_next"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_generator_next_abi(gen: Value) -> Value {
    Value::from_ptr(unsafe { rt_generator_next(gen.unwrap_ptr()) })
}


// §F.7b: TypeTagKindsGuard removed — per-slot tag side-array deleted.

/// Create a new generator object
///
/// # Safety
/// The returned pointer must be treated as a GC-managed object.
pub unsafe fn rt_make_generator(func_id: u32, num_locals: u32) -> *mut Obj {
    // Calculate size: header + fixed fields + locals array
    // Use checked arithmetic to prevent overflow
    let locals_size = (num_locals as usize)
        .checked_mul(size_of::<pyaot_core_defs::Value>())
        .unwrap_or_else(|| {
            raise_exc!(ExceptionType::MemoryError, "generator locals size overflow");
        });
    let total_size = size_of::<GeneratorObj>()
        .checked_add(locals_size)
        .unwrap_or_else(|| {
            raise_exc!(ExceptionType::MemoryError, "generator size overflow");
        });

    let obj = gc_alloc(total_size, TypeTagKind::Generator as u8);
    if obj.is_null() {
        std::process::abort();
    }

    let gen = obj as *mut GeneratorObj;
    // gc_alloc already sets up header
    (*gen).func_id = func_id;
    (*gen).state = 0; // Initial state
    (*gen).exhausted = false;
    (*gen).closing = false;
    (*gen).num_locals = num_locals;
    // §F.7b: sent_value is a tagged Value; Value(0) encodes None (int tag, value 0).
    (*gen).sent_value = pyaot_core_defs::Value(0);

    // Zero-initialize locals (Value(0) = tagged-None / zero int)
    let locals_ptr = (*gen).locals.as_mut_ptr();
    for i in 0..num_locals as usize {
        *locals_ptr.add(i) = pyaot_core_defs::Value(0);
    }

    obj
}
#[export_name = "rt_make_generator"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_make_generator_abi(func_id: u32, num_locals: u32) -> Value {
    Value::from_ptr(unsafe { rt_make_generator(func_id, num_locals) })
}


/// Get the current state of a generator
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
pub unsafe fn rt_generator_get_state(gen: *mut Obj) -> u32 {
    let gen = gen as *mut GeneratorObj;
    (*gen).state
}
#[export_name = "rt_generator_get_state"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_generator_get_state_abi(gen: Value) -> u32 {
    unsafe { rt_generator_get_state(gen.unwrap_ptr()) }
}


/// Set the current state of a generator
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
pub unsafe fn rt_generator_set_state(gen: *mut Obj, state: u32) {
    let gen = gen as *mut GeneratorObj;
    (*gen).state = state;
}
#[export_name = "rt_generator_set_state"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_generator_set_state_abi(gen: Value, state: u32) {
    unsafe { rt_generator_set_state(gen.unwrap_ptr(), state) }
}


/// Get a local variable from the generator (as i64 for int/float bits/bool/ptr)
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
/// `index` must be less than the generator's num_locals.
pub unsafe fn rt_generator_get_local(gen: *mut Obj, index: u32) -> i64 {
    let gen = gen as *mut GeneratorObj;
    if index >= (*gen).num_locals {
        eprintln!(
            "FATAL: Generator local index {} out of bounds (num_locals={})",
            index,
            (*gen).num_locals
        );
        std::process::abort();
    }
    let locals_ptr = (*gen).locals.as_ptr();
    (*locals_ptr.add(index as usize)).0 as i64
}
#[export_name = "rt_generator_get_local"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_generator_get_local_abi(gen: Value, index: u32) -> i64 {
    unsafe { rt_generator_get_local(gen.unwrap_ptr(), index) }
}


/// Set a local variable in the generator (as i64)
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
/// `index` must be less than the generator's num_locals.
pub unsafe fn rt_generator_set_local(gen: *mut Obj, index: u32, value: i64) {
    let gen = gen as *mut GeneratorObj;
    if index >= (*gen).num_locals {
        eprintln!(
            "FATAL: Generator local index {} out of bounds (num_locals={})",
            index,
            (*gen).num_locals
        );
        std::process::abort();
    }
    let locals_ptr = (*gen).locals.as_mut_ptr();
    *locals_ptr.add(index as usize) = pyaot_core_defs::Value(value as u64);
}
#[export_name = "rt_generator_set_local"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_generator_set_local_abi(gen: Value, index: u32, value: i64) {
    unsafe { rt_generator_set_local(gen.unwrap_ptr(), index, value) }
}


/// Get a local variable from the generator as a pointer
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
/// `index` must be less than the generator's num_locals.
pub unsafe fn rt_generator_get_local_ptr(gen: *mut Obj, index: u32) -> *mut Obj {
    let gen = gen as *mut GeneratorObj;
    if index >= (*gen).num_locals {
        eprintln!(
            "FATAL: Generator local index {} out of bounds (num_locals={})",
            index,
            (*gen).num_locals
        );
        std::process::abort();
    }
    let locals_ptr = (*gen).locals.as_ptr();
    (*locals_ptr.add(index as usize)).0 as *mut Obj
}
#[export_name = "rt_generator_get_local_ptr"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_generator_get_local_ptr_abi(gen: Value, index: u32) -> Value {
    Value::from_ptr(unsafe { rt_generator_get_local_ptr(gen.unwrap_ptr(), index) })
}


/// Set a local variable in the generator as a pointer
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
/// `index` must be less than the generator's num_locals.
pub unsafe fn rt_generator_set_local_ptr(gen: *mut Obj, index: u32, value: *mut Obj) {
    let gen = gen as *mut GeneratorObj;
    if index >= (*gen).num_locals {
        eprintln!(
            "FATAL: Generator local index {} out of bounds (num_locals={})",
            index,
            (*gen).num_locals
        );
        std::process::abort();
    }
    let locals_ptr = (*gen).locals.as_mut_ptr();
    *locals_ptr.add(index as usize) = pyaot_core_defs::Value(value as u64);
}
#[export_name = "rt_generator_set_local_ptr"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_generator_set_local_ptr_abi(gen: Value, index: u32, value: Value) {
    unsafe { rt_generator_set_local_ptr(gen.unwrap_ptr(), index, value.unwrap_ptr()) }
}


// §F.7b: rt_generator_set_local_type removed — per-slot tag side-array deleted.

/// Mark the generator as exhausted
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
pub unsafe fn rt_generator_set_exhausted(gen: *mut Obj) {
    let gen_obj = gen as *mut GeneratorObj;
    (*gen_obj).exhausted = true;
    (*gen_obj).state = u32::MAX;
}
#[export_name = "rt_generator_set_exhausted"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_generator_set_exhausted_abi(gen: Value) {
    unsafe { rt_generator_set_exhausted(gen.unwrap_ptr()) }
}


/// Check if the generator is exhausted
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj or IteratorObj.
pub unsafe fn rt_generator_is_exhausted(gen: *mut Obj) -> i8 {
    use crate::object::{IteratorObj, TypeTagKind};

    if gen.is_null() {
        return 1;
    }

    // Check the type tag to determine the object type
    let type_tag = (*gen).header.type_tag;

    if type_tag == TypeTagKind::Generator {
        // It's a generator object
        let gen_obj = gen as *mut GeneratorObj;
        if (*gen_obj).exhausted {
            1
        } else {
            0
        }
    } else if type_tag == TypeTagKind::Iterator {
        // It's an iterator object (including Zip, Map, Filter iterators)
        // All these have compatible layout for kind and exhausted fields
        let iter_obj = gen as *mut IteratorObj;
        if (*iter_obj).exhausted {
            1
        } else {
            0
        }
    } else {
        // Unknown type, assume not exhausted
        0
    }
}
#[export_name = "rt_generator_is_exhausted"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_generator_is_exhausted_abi(gen: Value) -> i8 {
    unsafe { rt_generator_is_exhausted(gen.unwrap_ptr()) }
}


/// Raise StopIteration exception (called when generator is exhausted)
///
/// # Safety
/// This function will not return normally.
#[no_mangle]
pub unsafe extern "C" fn rt_generator_stop_iteration() -> ! {
    raise_exc!(ExceptionType::StopIteration, "")
}

/// Send a value to a generator and resume execution
///
/// `send(value)` resumes the generator and "sends" a value that becomes
/// the result of the current yield expression.
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
/// `value` is the value to send (stored as i64).
pub unsafe fn rt_generator_send(gen: *mut Obj, value: i64) -> *mut Obj {
    // Check if it's actually a generator
    if gen.is_null() || (*gen).header.type_tag != TypeTagKind::Generator {
        raise_exc!(ExceptionType::TypeError, "send() called on non-generator");
    }

    let gen_obj = gen as *mut GeneratorObj;

    // Check if generator is exhausted
    if (*gen_obj).exhausted {
        raise_exc!(ExceptionType::StopIteration, "generator already exhausted");
    }

    // CPython: can't send non-None value to a just-started generator
    // State 0 means the generator hasn't started yet
    // In our representation, value 0 is None
    if (*gen_obj).state == 0 && value != 0 {
        raise_exc!(
            ExceptionType::TypeError,
            "can't send non-None value to a just-started generator"
        );
    }

    // Store the sent value (wire format i64 reinterpreted as tagged Value bits)
    (*gen_obj).sent_value = pyaot_core_defs::Value(value as u64);

    // Call the resume function
    __pyaot_generator_resume(gen)
}
#[export_name = "rt_generator_send"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_generator_send_abi(gen: Value, value: i64) -> Value {
    Value::from_ptr(unsafe { rt_generator_send(gen.unwrap_ptr(), value) })
}


/// Get the sent value from a generator (called from resume function)
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
pub unsafe fn rt_generator_get_sent_value(gen: *mut Obj) -> i64 {
    let gen_obj = gen as *mut GeneratorObj;
    (*gen_obj).sent_value.0 as i64
}
#[export_name = "rt_generator_get_sent_value"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_generator_get_sent_value_abi(gen: Value) -> i64 {
    unsafe { rt_generator_get_sent_value(gen.unwrap_ptr()) }
}


/// Close a generator
///
/// `close()` raises GeneratorExit at the point where the generator was paused.
/// If the generator function catches GeneratorExit (or doesn't catch), close() returns.
/// If the generator yields a value, RuntimeError is raised.
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
pub unsafe fn rt_generator_close(gen: *mut Obj) {
    // Check if it's actually a generator
    if gen.is_null() || (*gen).header.type_tag != TypeTagKind::Generator {
        return; // Silently ignore
    }

    let gen_obj = gen as *mut GeneratorObj;

    // If already exhausted, do nothing
    if (*gen_obj).exhausted {
        return;
    }

    // Mark as closing - the resume function will check this and raise GeneratorExit
    (*gen_obj).closing = true;

    // Try to resume - if it yields instead of exiting, that's an error
    __pyaot_generator_resume(gen);

    // If the generator is not exhausted after resuming, it yielded instead of
    // returning/raising GeneratorExit — that is an error per the Python spec.
    // Checking `exhausted` is correct because None yields (null/0 result) would
    // otherwise be conflated with "did not yield" when testing the result pointer.
    if !(*gen_obj).exhausted {
        // Generator yielded instead of returning/raising - this is an error
        raise_exc!(
            ExceptionType::RuntimeError,
            "generator ignored GeneratorExit"
        );
    }

    // Mark as exhausted
    (*gen_obj).exhausted = true;
    (*gen_obj).closing = false;
}
#[export_name = "rt_generator_close"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_generator_close_abi(gen: Value) {
    unsafe { rt_generator_close(gen.unwrap_ptr()) }
}


/// Check if the generator is in closing state
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
pub unsafe fn rt_generator_is_closing(gen: *mut Obj) -> i8 {
    let gen_obj = gen as *mut GeneratorObj;
    if (*gen_obj).closing {
        1
    } else {
        0
    }
}
#[export_name = "rt_generator_is_closing"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_generator_is_closing_abi(gen: Value) -> i8 {
    unsafe { rt_generator_is_closing(gen.unwrap_ptr()) }
}


// §F.7b: finalize_generator removed — per-slot tag side-array deleted; no
// separate heap allocation to free on sweep.
