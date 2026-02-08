//! Boxing and unboxing operations for primitive types
//!
//! Includes a small integer pool for integers -5 to 256 (like CPython),
//! reducing GC pressure for commonly used integers.

use crate::gc;
use crate::object::{BoolObj, FloatObj, IntObj, Obj, TypeTagKind};
use std::sync::Mutex;

/// Range of small integers to pre-allocate (inclusive)
const SMALL_INT_MIN: i64 = -5;
const SMALL_INT_MAX: i64 = 256;
const SMALL_INT_POOL_SIZE: usize = (SMALL_INT_MAX - SMALL_INT_MIN + 1) as usize; // 262

/// Wrapper to make raw pointer Send+Sync for use in static Mutex
/// Safety: The pool is only accessed while holding the mutex lock
struct PoolWrapper([*mut Obj; SMALL_INT_POOL_SIZE]);
unsafe impl Send for PoolWrapper {}
unsafe impl Sync for PoolWrapper {}

/// Pool of pre-allocated small integers
/// Initialized by init_small_int_pool() during rt_init()
static SMALL_INT_POOL: Mutex<Option<PoolWrapper>> = Mutex::new(None);

/// Initialize the small integer pool
/// Pre-allocates IntObj for integers -5 to 256
/// Called from rt_init()
pub fn init_small_int_pool() {
    let mut pool_guard = SMALL_INT_POOL
        .lock()
        .expect("SMALL_INT_POOL mutex poisoned - another thread panicked");

    if pool_guard.is_some() {
        return; // Already initialized
    }

    let mut pool: [*mut Obj; SMALL_INT_POOL_SIZE] = [std::ptr::null_mut(); SMALL_INT_POOL_SIZE];

    for (i, slot) in pool.iter_mut().enumerate() {
        let value = SMALL_INT_MIN + i as i64;
        let size = std::mem::size_of::<IntObj>();
        let obj = gc::gc_alloc(size, TypeTagKind::Int as u8);

        unsafe {
            let int_obj = obj as *mut IntObj;
            (*int_obj).value = value;

            // Mark as permanent GC root (never collect)
            (*obj).header.marked = true;
        }

        *slot = obj;
    }

    *pool_guard = Some(PoolWrapper(pool));
}

/// Shutdown the small integer pool
/// Called from rt_shutdown()
pub fn shutdown_small_int_pool() {
    let mut pool_guard = SMALL_INT_POOL
        .lock()
        .expect("SMALL_INT_POOL mutex poisoned - another thread panicked");
    *pool_guard = None;
}

/// Box an integer value as a heap-allocated IntObj
/// For small integers (-5 to 256), returns a pre-allocated object from the pool.
/// Used for dict keys when the key type is int
#[no_mangle]
pub extern "C" fn rt_box_int(value: i64) -> *mut Obj {
    // Check if value is in the small integer range
    if (SMALL_INT_MIN..=SMALL_INT_MAX).contains(&value) {
        let pool_guard = SMALL_INT_POOL
            .lock()
            .expect("SMALL_INT_POOL mutex poisoned - another thread panicked");

        if let Some(ref wrapper) = *pool_guard {
            let index = (value - SMALL_INT_MIN) as usize;
            return wrapper.0[index];
        }
    }

    // Fall back to regular allocation for integers outside the pool range
    let size = std::mem::size_of::<IntObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Int as u8);

    unsafe {
        let int_obj = obj as *mut IntObj;
        (*int_obj).value = value;
    }

    obj
}

/// Box a boolean value as a heap-allocated BoolObj
/// Used for dict keys when the key type is bool
#[no_mangle]
pub extern "C" fn rt_box_bool(value: i8) -> *mut Obj {
    let size = std::mem::size_of::<BoolObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Bool as u8);

    unsafe {
        let bool_obj = obj as *mut BoolObj;
        (*bool_obj).value = value != 0;
    }

    obj
}

/// Box a float value as a heap-allocated FloatObj
/// Used for list elements when the element type is float
#[no_mangle]
pub extern "C" fn rt_box_float(value: f64) -> *mut Obj {
    let size = std::mem::size_of::<FloatObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Float as u8);

    unsafe {
        let float_obj = obj as *mut FloatObj;
        (*float_obj).value = value;
    }

    obj
}

/// Unbox a float value from a heap-allocated FloatObj
/// Used for list elements when the element type is float
///
/// # Panics
/// Panics if `obj` is null or has wrong type tag. This catches type confusion
/// bugs in both debug and release builds.
#[no_mangle]
pub extern "C" fn rt_unbox_float(obj: *mut Obj) -> f64 {
    if obj.is_null() {
        panic!("rt_unbox_float: cannot unbox null pointer");
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

/// Unbox an integer value from a heap-allocated IntObj
/// Used for dict keys and set elements when the element type is int
///
/// # Panics
/// Panics if `obj` is null or has wrong type tag. This catches type confusion
/// bugs in both debug and release builds.
#[no_mangle]
pub extern "C" fn rt_unbox_int(obj: *mut Obj) -> i64 {
    if obj.is_null() {
        panic!("rt_unbox_int: cannot unbox null pointer");
    }

    unsafe {
        let actual_tag = (*obj).header.type_tag;
        if actual_tag != TypeTagKind::Int {
            panic!("rt_unbox_int: expected Int, got {:?}", actual_tag);
        }
        let int_obj = obj as *mut IntObj;
        (*int_obj).value
    }
}

/// Unbox a boolean value from a heap-allocated BoolObj
/// Used for dict keys and set elements when the element type is bool
///
/// # Panics
/// Panics if `obj` is null or has wrong type tag. This catches type confusion
/// bugs in both debug and release builds.
#[no_mangle]
pub extern "C" fn rt_unbox_bool(obj: *mut Obj) -> i8 {
    if obj.is_null() {
        panic!("rt_unbox_bool: cannot unbox null pointer");
    }

    unsafe {
        let actual_tag = (*obj).header.type_tag;
        if actual_tag != TypeTagKind::Bool {
            panic!("rt_unbox_bool: expected Bool, got {:?}", actual_tag);
        }
        let bool_obj = obj as *mut BoolObj;
        if (*bool_obj).value {
            1
        } else {
            0
        }
    }
}

/// Box None as a heap-allocated NoneObj
/// Used for Union types when the value is None
#[no_mangle]
pub extern "C" fn rt_box_none() -> *mut Obj {
    crate::object::none_obj()
}
