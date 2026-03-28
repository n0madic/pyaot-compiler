//! Generator runtime support
//!
//! Provides runtime functions for generator objects (functions with yield).

use crate::exceptions::{rt_exc_raise, ExceptionType};
use crate::gc::gc_alloc;
use crate::object::{GeneratorObj, Obj, TypeTagKind};
use std::mem::size_of;

// External function provided by compiled code - dispatches to the appropriate resume function
extern "C" {
    fn __pyaot_generator_resume(gen: *mut Obj) -> *mut Obj;
}

/// Call next() on a generator - invokes the resume function
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
#[no_mangle]
pub unsafe extern "C" fn rt_generator_next(gen: *mut Obj) -> *mut Obj {
    // Check if it's actually a generator
    if gen.is_null() || (*gen).header.type_tag != TypeTagKind::Generator {
        let msg = b"next() called on non-generator";
        rt_exc_raise(ExceptionType::StopIteration as u8, msg.as_ptr(), msg.len());
    }

    // Call the dispatcher function (provided by compiled code)
    __pyaot_generator_resume(gen)
}

/// RAII guard for type_tags allocation to prevent memory leaks on panic
struct TypeTagKindsGuard {
    ptr: *mut u8,
    layout: std::alloc::Layout,
}

impl TypeTagKindsGuard {
    fn new(ptr: *mut u8, layout: std::alloc::Layout) -> Self {
        Self { ptr, layout }
    }

    /// Transfer ownership - prevents deallocation on drop
    fn into_raw(mut self) -> *mut u8 {
        let ptr = self.ptr;
        self.ptr = std::ptr::null_mut();
        std::mem::forget(self);
        ptr
    }
}

impl Drop for TypeTagKindsGuard {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { std::alloc::dealloc(self.ptr, self.layout) }
        }
    }
}

/// Create a new generator object
///
/// # Safety
/// The returned pointer must be treated as a GC-managed object.
#[no_mangle]
pub unsafe extern "C" fn rt_make_generator(func_id: u32, num_locals: u32) -> *mut Obj {
    use std::alloc::{alloc, Layout};

    // Calculate size: header + fixed fields + locals array
    // Use checked arithmetic to prevent overflow
    let locals_size = (num_locals as usize)
        .checked_mul(size_of::<i64>())
        .unwrap_or_else(|| {
            let msg = b"MemoryError: generator locals size overflow";
            rt_exc_raise(ExceptionType::MemoryError as u8, msg.as_ptr(), msg.len());
        });
    let total_size = size_of::<GeneratorObj>()
        .checked_add(locals_size)
        .unwrap_or_else(|| {
            let msg = b"MemoryError: generator size overflow";
            rt_exc_raise(ExceptionType::MemoryError as u8, msg.as_ptr(), msg.len());
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
    (*gen).sent_value = 0; // No sent value initially (0 = None)

    // Allocate type tags array (not GC-managed, just raw allocation)
    // Use RAII guard to ensure deallocation if panic occurs during initialization
    if num_locals > 0 {
        let layout =
            Layout::array::<u8>(num_locals as usize).expect("Invalid layout for type_tags");
        let type_tags = alloc(layout);
        if type_tags.is_null() {
            std::process::abort();
        }

        // Guard takes ownership - will dealloc on panic
        let guard = TypeTagKindsGuard::new(type_tags, layout);

        // Initialize all type tags to LOCAL_TYPE_RAW_INT (safest default - won't be traced)
        use crate::object::LOCAL_TYPE_RAW_INT;
        for i in 0..num_locals as usize {
            *type_tags.add(i) = LOCAL_TYPE_RAW_INT;
        }

        // Zero-initialize locals (gc_alloc should have zeroed, but be explicit)
        let locals_ptr = (*gen).locals.as_mut_ptr();
        for i in 0..num_locals as usize {
            *locals_ptr.add(i) = 0;
        }

        // All initialization successful - transfer ownership to generator
        (*gen).type_tags = guard.into_raw();
    } else {
        (*gen).type_tags = std::ptr::null_mut();

        // Zero-initialize locals (gc_alloc should have zeroed, but be explicit)
        let locals_ptr = (*gen).locals.as_mut_ptr();
        for i in 0..num_locals as usize {
            *locals_ptr.add(i) = 0;
        }
    }

    obj
}

/// Get the current state of a generator
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
#[no_mangle]
pub unsafe extern "C" fn rt_generator_get_state(gen: *mut Obj) -> u32 {
    let gen = gen as *mut GeneratorObj;
    (*gen).state
}

/// Set the current state of a generator
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
#[no_mangle]
pub unsafe extern "C" fn rt_generator_set_state(gen: *mut Obj, state: u32) {
    let gen = gen as *mut GeneratorObj;
    (*gen).state = state;
}

/// Get a local variable from the generator (as i64 for int/float bits/bool/ptr)
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
/// `index` must be less than the generator's num_locals.
#[no_mangle]
pub unsafe extern "C" fn rt_generator_get_local(gen: *mut Obj, index: u32) -> i64 {
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
    *locals_ptr.add(index as usize)
}

/// Set a local variable in the generator (as i64)
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
/// `index` must be less than the generator's num_locals.
#[no_mangle]
pub unsafe extern "C" fn rt_generator_set_local(gen: *mut Obj, index: u32, value: i64) {
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
    *locals_ptr.add(index as usize) = value;
}

/// Get a local variable from the generator as a pointer
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
/// `index` must be less than the generator's num_locals.
#[no_mangle]
pub unsafe extern "C" fn rt_generator_get_local_ptr(gen: *mut Obj, index: u32) -> *mut Obj {
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
    *locals_ptr.add(index as usize) as *mut Obj
}

/// Set a local variable in the generator as a pointer
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
/// `index` must be less than the generator's num_locals.
#[no_mangle]
pub unsafe extern "C" fn rt_generator_set_local_ptr(gen: *mut Obj, index: u32, value: *mut Obj) {
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
    *locals_ptr.add(index as usize) = value as i64;
}

/// Set the type tag for a generator local variable (for precise GC tracking)
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
/// `index` must be less than the generator's num_locals.
/// `type_tag` should be one of LOCAL_TYPE_RAW_INT, LOCAL_TYPE_RAW_FLOAT, LOCAL_TYPE_RAW_BOOL, or LOCAL_TYPE_PTR.
#[no_mangle]
pub unsafe extern "C" fn rt_generator_set_local_type(gen: *mut Obj, index: u32, type_tag: u8) {
    let gen = gen as *mut GeneratorObj;
    if !(*gen).type_tags.is_null() && index < (*gen).num_locals {
        *(*gen).type_tags.add(index as usize) = type_tag;
    }
}

/// Mark the generator as exhausted
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
#[no_mangle]
pub unsafe extern "C" fn rt_generator_set_exhausted(gen: *mut Obj) {
    let gen_obj = gen as *mut GeneratorObj;
    (*gen_obj).exhausted = true;
    (*gen_obj).state = u32::MAX;
}

/// Check if the generator is exhausted
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj or IteratorObj.
#[no_mangle]
pub unsafe extern "C" fn rt_generator_is_exhausted(gen: *mut Obj) -> i8 {
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

/// Raise StopIteration exception (called when generator is exhausted)
///
/// # Safety
/// This function will not return normally.
#[no_mangle]
pub unsafe extern "C" fn rt_generator_stop_iteration() -> ! {
    rt_exc_raise(ExceptionType::StopIteration as u8, std::ptr::null(), 0)
}

/// Send a value to a generator and resume execution
///
/// `send(value)` resumes the generator and "sends" a value that becomes
/// the result of the current yield expression.
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
/// `value` is the value to send (stored as i64).
#[no_mangle]
pub unsafe extern "C" fn rt_generator_send(gen: *mut Obj, value: i64) -> *mut Obj {
    // Check if it's actually a generator
    if gen.is_null() || (*gen).header.type_tag != TypeTagKind::Generator {
        let msg = b"send() called on non-generator";
        rt_exc_raise(ExceptionType::TypeError as u8, msg.as_ptr(), msg.len());
    }

    let gen_obj = gen as *mut GeneratorObj;

    // Check if generator is exhausted
    if (*gen_obj).exhausted {
        let msg = b"generator already exhausted";
        rt_exc_raise(ExceptionType::StopIteration as u8, msg.as_ptr(), msg.len());
    }

    // CPython: can't send non-None value to a just-started generator
    // State 0 means the generator hasn't started yet
    // In our representation, value 0 is None
    if (*gen_obj).state == 0 && value != 0 {
        let msg = b"can't send non-None value to a just-started generator";
        rt_exc_raise(ExceptionType::TypeError as u8, msg.as_ptr(), msg.len());
    }

    // Store the sent value
    (*gen_obj).sent_value = value;

    // Call the resume function
    __pyaot_generator_resume(gen)
}

/// Get the sent value from a generator (called from resume function)
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
#[no_mangle]
pub unsafe extern "C" fn rt_generator_get_sent_value(gen: *mut Obj) -> i64 {
    let gen_obj = gen as *mut GeneratorObj;
    (*gen_obj).sent_value
}

/// Close a generator
///
/// `close()` raises GeneratorExit at the point where the generator was paused.
/// If the generator function catches GeneratorExit (or doesn't catch), close() returns.
/// If the generator yields a value, RuntimeError is raised.
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
#[no_mangle]
pub unsafe extern "C" fn rt_generator_close(gen: *mut Obj) {
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
        let msg = b"generator ignored GeneratorExit";
        rt_exc_raise(ExceptionType::RuntimeError as u8, msg.as_ptr(), msg.len());
    }

    // Mark as exhausted
    (*gen_obj).exhausted = true;
    (*gen_obj).closing = false;
}

/// Check if the generator is in closing state
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj.
#[no_mangle]
pub unsafe extern "C" fn rt_generator_is_closing(gen: *mut Obj) -> i8 {
    let gen_obj = gen as *mut GeneratorObj;
    if (*gen_obj).closing {
        1
    } else {
        0
    }
}

/// Finalize a generator object (free type_tags array)
/// Called by GC during sweep phase before freeing the generator object
///
/// # Safety
/// `gen` must be a valid pointer to a GeneratorObj that will be freed.
pub(crate) unsafe fn finalize_generator(gen: *mut Obj) {
    use std::alloc::{dealloc, Layout};

    let gen_obj = gen as *mut GeneratorObj;
    let num_locals = (*gen_obj).num_locals;

    // Free the type_tags array if it exists
    if !(*gen_obj).type_tags.is_null() && num_locals > 0 {
        let layout =
            Layout::array::<u8>(num_locals as usize).expect("Invalid layout for type_tags");
        dealloc((*gen_obj).type_tags, layout);
        (*gen_obj).type_tags = std::ptr::null_mut();
    }
}
