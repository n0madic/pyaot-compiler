//! Boxing and unboxing operations for primitive types
//!
//! Includes a small integer pool for integers -5 to 256 (like CPython),
//! and boolean singletons for True and False, reducing GC pressure for
//! commonly used values.

use crate::gc;
use crate::object::{BoolObj, FloatObj, IntObj, Obj, TypeTagKind};
use std::cell::UnsafeCell;

/// Range of small integers to pre-allocate (inclusive)
const SMALL_INT_MIN: i64 = -5;
const SMALL_INT_MAX: i64 = 256;
const SMALL_INT_POOL_SIZE: usize = (SMALL_INT_MAX - SMALL_INT_MIN + 1) as usize; // 262

/// Lock-free pool storage for single-threaded access.
/// Safety: The runtime is single-threaded (AOT-compiled Python has no threading).
struct PoolStorage<const N: usize> {
    data: UnsafeCell<[*mut Obj; N]>,
    initialized: UnsafeCell<bool>,
}

unsafe impl<const N: usize> Sync for PoolStorage<N> {}

impl<const N: usize> PoolStorage<N> {
    const fn new() -> Self {
        Self {
            data: UnsafeCell::new([std::ptr::null_mut(); N]),
            initialized: UnsafeCell::new(false),
        }
    }

    #[inline(always)]
    fn is_initialized(&self) -> bool {
        unsafe { *self.initialized.get() }
    }

    fn set_initialized(&self) {
        unsafe {
            *self.initialized.get() = true;
        }
    }

    #[inline(always)]
    unsafe fn get(&self, index: usize) -> *mut Obj {
        (*self.data.get())[index]
    }

    unsafe fn set(&self, index: usize, val: *mut Obj) {
        (*self.data.get())[index] = val;
    }

    unsafe fn iter(&self) -> std::slice::Iter<'_, *mut Obj> {
        (*self.data.get()).iter()
    }
}

/// Pool of pre-allocated small integers
/// Initialized by init_small_int_pool() during rt_init()
static SMALL_INT_POOL: PoolStorage<SMALL_INT_POOL_SIZE> = PoolStorage::new();

/// Pre-allocated boolean singletons (index 0 = False, index 1 = True)
/// Initialized by init_bool_pool() during rt_init()
static BOOL_POOL: PoolStorage<2> = PoolStorage::new();

/// Initialize the small integer pool
/// Pre-allocates IntObj for integers -5 to 256
/// Called from rt_init()
pub fn init_small_int_pool() {
    if SMALL_INT_POOL.is_initialized() {
        return;
    }

    for i in 0..SMALL_INT_POOL_SIZE {
        let value = SMALL_INT_MIN + i as i64;
        let size = std::mem::size_of::<IntObj>();
        let obj = gc::gc_alloc(size, TypeTagKind::Int as u8);

        unsafe {
            let int_obj = obj as *mut IntObj;
            (*int_obj).value = value;
            SMALL_INT_POOL.set(i, obj);
        }
    }

    SMALL_INT_POOL.set_initialized();
}

/// Shutdown the small integer pool
/// Called from rt_shutdown()
pub fn shutdown_small_int_pool() {
    // Objects are freed by GC shutdown, just mark as uninitialized
    unsafe {
        *SMALL_INT_POOL.initialized.get() = false;
    }
}

/// Initialize the boolean singleton pool
/// Pre-allocates BoolObj for False (index 0) and True (index 1)
/// Called from rt_init()
pub fn init_bool_pool() {
    if BOOL_POOL.is_initialized() {
        return;
    }

    for i in 0..2 {
        let size = std::mem::size_of::<BoolObj>();
        let obj = gc::gc_alloc(size, TypeTagKind::Bool as u8);

        unsafe {
            let bool_obj = obj as *mut BoolObj;
            (*bool_obj).value = i != 0;
            BOOL_POOL.set(i, obj);
        }
    }

    BOOL_POOL.set_initialized();
}

/// Shutdown the boolean singleton pool
/// Called from rt_shutdown()
pub fn shutdown_bool_pool() {
    unsafe {
        *BOOL_POOL.initialized.get() = false;
    }
}

/// Mark all pool objects (small integers and boolean singletons) as reachable.
///
/// Called during the GC mark phase so that pool objects — which are not on the
/// shadow stack or in globals — are never swept.  The initial `marked = true`
/// set at allocation time is cleared by `sweep` at the end of every collection,
/// so we must re-mark them at the start of every mark phase instead.
pub fn mark_pools() {
    // Mark small integer pool.
    // Do NOT guard on is_initialized(): during pool initialization in gc_stress
    // mode, gc_alloc fires a collection before each IntObj is allocated. At that
    // point is_initialized() is false even though some slots are already filled.
    // Skipping those slots causes the partially-built pool objects to be swept
    // (freed), so every pool[i] ends up at the same address as pool[i-1] was
    // at — all pointing to the last-allocated object. Marking non-null slots
    // unconditionally fixes this.
    unsafe {
        for &obj in SMALL_INT_POOL.iter() {
            if !obj.is_null() {
                (*obj).set_marked(true);
            }
        }
    }

    // Mark boolean singleton pool (same reasoning applies).
    unsafe {
        for &obj in BOOL_POOL.iter() {
            if !obj.is_null() {
                (*obj).set_marked(true);
            }
        }
    }
}

/// Box an integer value as a heap-allocated IntObj
/// For small integers (-5 to 256), returns a pre-allocated object from the pool.
/// Used for dict keys when the key type is int
#[no_mangle]
pub extern "C" fn rt_box_int(value: i64) -> *mut Obj {
    // Check if value is in the small integer range
    if (SMALL_INT_MIN..=SMALL_INT_MAX).contains(&value) && SMALL_INT_POOL.is_initialized() {
        let index = (value - SMALL_INT_MIN) as usize;
        return unsafe { SMALL_INT_POOL.get(index) };
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
/// Returns a pre-allocated singleton from the bool pool (like CPython's True/False).
/// Used for dict keys when the key type is bool
#[no_mangle]
pub extern "C" fn rt_box_bool(value: i8) -> *mut Obj {
    if BOOL_POOL.is_initialized() {
        let index = if value != 0 { 1 } else { 0 };
        return unsafe { BOOL_POOL.get(index) };
    }

    // Fallback: pool not yet initialized (should not happen in normal operation)
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

/// Unbox an integer value from a heap-allocated IntObj
/// Used for dict keys and set elements when the element type is int.
/// Returns 0 for null pointers (safe default for .get() returning None).
#[no_mangle]
pub extern "C" fn rt_unbox_int(obj: *mut Obj) -> i64 {
    if obj.is_null() {
        return 0;
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
