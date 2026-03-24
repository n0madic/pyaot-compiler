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
use std::alloc::{alloc, dealloc, realloc, Layout};

/// Growth factor for StringBuilder buffer (2x growth strategy)
const GROWTH_FACTOR: usize = 2;

/// Minimum initial capacity for StringBuilder
const MIN_CAPACITY: usize = 64;

/// Create a new StringBuilder with estimated capacity
/// capacity_hint: estimated total length of all strings to be appended
/// Returns: pointer to allocated StringBuilderObj
#[no_mangle]
pub extern "C" fn rt_make_string_builder(capacity_hint: i64) -> *mut Obj {
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
            let msg = b"MemoryError: cannot allocate StringBuilder buffer";
            crate::exceptions::rt_exc_raise(
                pyaot_core_defs::BuiltinExceptionKind::MemoryError.tag(),
                msg.as_ptr(),
                msg.len(),
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

/// Append a string to the StringBuilder
/// builder: pointer to StringBuilderObj
/// str_obj: pointer to StrObj to append
#[no_mangle]
pub extern "C" fn rt_string_builder_append(builder: *mut Obj, str_obj: *mut Obj) {
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
                let msg = b"MemoryError: cannot reallocate StringBuilder buffer";
                crate::exceptions::rt_exc_raise(
                    pyaot_core_defs::BuiltinExceptionKind::MemoryError.tag(),
                    msg.as_ptr(),
                    msg.len(),
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

/// Finalize StringBuilder and return the resulting StrObj
/// builder: pointer to StringBuilderObj
/// Returns: pointer to new StrObj with the concatenated string
///
/// After this call, the StringBuilder's buffer is freed and should not be used again.
#[no_mangle]
pub extern "C" fn rt_string_builder_to_str(builder: *mut Obj) -> *mut Obj {
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

        // Create the result string
        let result = crate::string::core::rt_make_str_impl(data, len);

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

            let s1 = rt_make_str_impl(b"Hello".as_ptr(), 5);
            let s2 = rt_make_str_impl(b", ".as_ptr(), 2);
            let s3 = rt_make_str_impl(b"World".as_ptr(), 5);
            let s4 = rt_make_str_impl(b"!".as_ptr(), 1);

            rt_string_builder_append(builder, s1);
            rt_string_builder_append(builder, s2);
            rt_string_builder_append(builder, s3);
            rt_string_builder_append(builder, s4);

            let result = rt_string_builder_to_str(builder);
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

            // Append strings that exceed initial capacity
            for _ in 0..20 {
                let s = rt_make_str_impl(b"0123456789".as_ptr(), 10);
                rt_string_builder_append(builder, s);
            }

            let result = rt_string_builder_to_str(builder);
            let result_str = result as *mut StrObj;

            // Should have 200 bytes
            assert_eq!((*result_str).len, 200);
        }

        teardown();
    }
}
