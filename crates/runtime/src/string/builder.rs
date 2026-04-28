//! StringBuilder for efficient string concatenation
//!
//! This module implements a StringBuilder pattern for O(n) string concatenation
//! instead of O(n²) when concatenating multiple strings with nested rt_str_concat calls.
//!
//! ## Usage Pattern
//!
//! For `a + b + c + d`:
//! 1. Create StringBuilder with estimated capacity
//! 2. Append each string in order
//! 3. Finalize to produce a StrObj
//!
//! ## Memory Management
//!
//! StringBuilder allocates a growing buffer that is separate from the GC heap.
//! The buffer is freed when the StringBuilder is finalized or collected.

use crate::gc;
use crate::object::{Obj, StrObj, StringBuilderObj, TypeTagKind};
use pyaot_core_defs::Value;
use std::alloc::{alloc, dealloc, realloc, Layout};

/// Growth factor for StringBuilder buffer (2x growth strategy)
const GROWTH_FACTOR: usize = 2;

/// Minimum initial capacity for StringBuilder
const MIN_CAPACITY: usize = 64;

/// Create a new StringBuilder with estimated capacity
/// capacity_hint: estimated total length of all strings to be appended
/// Returns: pointer to allocated StringBuilderObj
pub fn rt_make_string_builder(capacity_hint: i64) -> *mut Obj {
    let capacity = if capacity_hint > 0 {
        capacity_hint as usize
    } else {
        MIN_CAPACITY
    };

    // Allocate the StringBuilder header via GC
    let size = std::mem::size_of::<StringBuilderObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::StringBuilder as u8);

    // Allocate the data buffer separately (not via GC)
    let buffer = unsafe {
        let layout = Layout::array::<u8>(capacity).expect("StringBuilder capacity overflow");
        let ptr = alloc(layout);
        if ptr.is_null() {
            raise_exc!(
                pyaot_core_defs::BuiltinExceptionKind::MemoryError,
                "cannot allocate StringBuilder buffer"
            );
        }
        ptr
    };

    unsafe {
        let sb = obj as *mut StringBuilderObj;
        (*sb).len = 0;
        (*sb).capacity = capacity;
        (*sb).data = buffer;
    }

    obj
}
#[export_name = "rt_make_string_builder"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_make_string_builder_abi(capacity_hint: i64) -> Value {
    Value::from_ptr(rt_make_string_builder(capacity_hint))
}


/// Append a string to the StringBuilder
/// builder: pointer to StringBuilderObj
/// str_obj: pointer to StrObj to append
pub fn rt_string_builder_append(builder: *mut Obj, str_obj: *mut Obj) {
    if builder.is_null() || str_obj.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(
            builder,
            TypeTagKind::StringBuilder,
            "rt_string_builder_append"
        );
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_string_builder_append");

        let sb = builder as *mut StringBuilderObj;
        let s = str_obj as *mut StrObj;

        let str_len = (*s).len;
        if str_len == 0 {
            return;
        }

        let new_len = (*sb).len + str_len;

        // Grow buffer if needed
        if new_len > (*sb).capacity {
            let new_capacity = std::cmp::max(new_len, (*sb).capacity * GROWTH_FACTOR);
            let old_capacity = (*sb).capacity;
            let old_data = (*sb).data;

            let old_layout =
                Layout::array::<u8>(old_capacity).expect("StringBuilder old layout overflow");
            let new_layout =
                Layout::array::<u8>(new_capacity).expect("StringBuilder new layout overflow");

            let new_data = realloc(old_data, old_layout, new_layout.size());
            if new_data.is_null() {
                raise_exc!(
                    pyaot_core_defs::BuiltinExceptionKind::MemoryError,
                    "cannot reallocate StringBuilder buffer"
                );
            }

            (*sb).data = new_data;
            (*sb).capacity = new_capacity;
        }

        // Copy the string data
        std::ptr::copy_nonoverlapping((*s).data.as_ptr(), (*sb).data.add((*sb).len), str_len);

        (*sb).len = new_len;
    }
}
#[export_name = "rt_string_builder_append"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_string_builder_append_abi(builder: Value, str_obj: Value) {
    rt_string_builder_append(builder.unwrap_ptr(), str_obj.unwrap_ptr())
}


/// Finalize StringBuilder and return the resulting StrObj
/// builder: pointer to StringBuilderObj
/// Returns: pointer to new StrObj with the concatenated string
///
/// After this call, the StringBuilder's buffer is freed and should not be used again.
pub fn rt_string_builder_to_str(builder: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if builder.is_null() {
        // Return empty string for null builder
        return unsafe { crate::string::core::rt_make_str_impl(std::ptr::null(), 0) };
    }

    unsafe {
        debug_assert_type_tag!(
            builder,
            TypeTagKind::StringBuilder,
            "rt_string_builder_to_str"
        );

        let sb = builder as *mut StringBuilderObj;
        let len = (*sb).len;
        let data = (*sb).data;

        // Root `builder` across rt_make_str_impl → gc_alloc.  The builder's
        // raw buffer (`data`) is a std::alloc allocation invisible to the GC,
        // so GC cannot free the bytes.  But the StringBuilderObj header itself
        // is GC-managed: if it is not reachable it will be swept, and the
        // post-alloc accesses to (*sb).capacity / (*sb).data would be
        // use-after-free.  Rooting builder keeps the header alive.
        let mut roots: [*mut Obj; 1] = [builder];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Create the result string (may trigger GC)
        let result = crate::string::core::rt_make_str_impl(data, len);

        gc_pop();

        // Free the StringBuilder's buffer (the GC will collect the StringBuilder header)
        if !data.is_null() && (*sb).capacity > 0 {
            let layout =
                Layout::array::<u8>((*sb).capacity).expect("StringBuilder layout overflow");
            dealloc(data, layout);
        }

        // Mark buffer as freed
        (*sb).data = std::ptr::null_mut();
        (*sb).capacity = 0;
        (*sb).len = 0;

        result
    }
}
#[export_name = "rt_string_builder_to_str"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_string_builder_to_str_abi(builder: Value) -> Value {
    Value::from_ptr(rt_string_builder_to_str(builder.unwrap_ptr()))
}


/// Finalize StringBuilder (called by GC during collection)
/// Frees the internal buffer if not already freed
pub fn string_builder_finalize(obj: *mut Obj) {
    if obj.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(obj, TypeTagKind::StringBuilder, "string_builder_finalize");

        let sb = obj as *mut StringBuilderObj;
        let data = (*sb).data;
        let capacity = (*sb).capacity;

        if !data.is_null() && capacity > 0 {
            let layout = Layout::array::<u8>(capacity).expect("StringBuilder layout overflow");
            dealloc(data, layout);
            (*sb).data = std::ptr::null_mut();
            (*sb).capacity = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::string::core::rt_make_str_impl;

    fn setup() {
        crate::gc::init();
        crate::string::init_string_pool();
    }

    fn teardown() {
        crate::string::shutdown_string_pool();
        crate::gc::shutdown();
    }

    #[test]
    fn test_string_builder_basic() {
        let _guard = crate::RUNTIME_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        setup();

        unsafe {
            let builder = rt_make_string_builder(100);

            // Root builder across all rt_make_str_impl / rt_string_builder_append calls
            // so GC stress mode does not sweep it between allocations.
            let mut roots: [*mut Obj; 1] = [builder];
            let mut frame = crate::gc::ShadowFrame {
                prev: std::ptr::null_mut(),
                nroots: 1,
                roots: roots.as_mut_ptr(),
            };
            crate::gc::gc_push(&mut frame);

            let s1 = rt_make_str_impl(b"Hello".as_ptr(), 5);
            rt_string_builder_append(roots[0], s1);
            let s2 = rt_make_str_impl(b", ".as_ptr(), 2);
            rt_string_builder_append(roots[0], s2);
            let s3 = rt_make_str_impl(b"World".as_ptr(), 5);
            rt_string_builder_append(roots[0], s3);
            let s4 = rt_make_str_impl(b"!".as_ptr(), 1);
            rt_string_builder_append(roots[0], s4);

            let result = rt_string_builder_to_str(roots[0]);
            crate::gc::gc_pop();

            let result_str = result as *mut StrObj;

            assert_eq!((*result_str).len, 13);

            let result_data =
                std::slice::from_raw_parts((*result_str).data.as_ptr(), (*result_str).len);
            assert_eq!(result_data, b"Hello, World!");
        }

        teardown();
    }

    #[test]
    fn test_string_builder_empty() {
        let _guard = crate::RUNTIME_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        setup();

        unsafe {
            let builder = rt_make_string_builder(0);
            let result = rt_string_builder_to_str(builder);
            let result_str = result as *mut StrObj;

            assert_eq!((*result_str).len, 0);
        }

        teardown();
    }

    #[test]
    fn test_string_builder_growth() {
        let _guard = crate::RUNTIME_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        setup();

        unsafe {
            // Start with small capacity
            let builder = rt_make_string_builder(10);

            // Root builder across rt_make_str_impl / rt_string_builder_append calls.
            let mut roots: [*mut Obj; 1] = [builder];
            let mut frame = crate::gc::ShadowFrame {
                prev: std::ptr::null_mut(),
                nroots: 1,
                roots: roots.as_mut_ptr(),
            };
            crate::gc::gc_push(&mut frame);

            // Append strings that exceed initial capacity
            for _ in 0..20 {
                let s = rt_make_str_impl(b"0123456789".as_ptr(), 10);
                rt_string_builder_append(roots[0], s);
            }

            let result = rt_string_builder_to_str(roots[0]);
            crate::gc::gc_pop();

            let result_str = result as *mut StrObj;

            // Should have 200 bytes
            assert_eq!((*result_str).len, 200);
        }

        teardown();
    }
}
