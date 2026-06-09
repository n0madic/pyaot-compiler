//! Tests for debug_assert_type_tag! macro.
//!
//! These tests verify that FFI functions panic with clear error messages
//! when called with incorrect object types in debug builds.
//!
//! Note: FFI functions (`extern "C"`) cannot unwind, so panics abort the process.
//! This means we cannot use `#[should_panic]` for testing FFI functions directly.
//! Instead, we test the macro directly without going through the FFI boundary.
//!
//! **Important:** Tests that expect panics from `debug_assert_type_tag!` only work
//! in debug builds. In release builds, the assertions are compiled away.

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::{dict, gc, list, object, set, tuple};

/// Test that the debug_assert_type_tag! macro panics with correct message
/// when given a Dict but expecting a List.
/// Only runs in debug builds (release builds compile away debug_assert).
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "test_macro: expected List, got Dict")]
fn test_debug_assert_type_tag_macro() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gc::init();

    // Create a Dict object
    let dict_obj = dict::rt_make_dict(8);

    // Use the macro directly - this should panic
    unsafe {
        debug_assert_type_tag!(dict_obj, object::TypeTagKind::List, "test_macro");
    }
}

/// Test that the macro does NOT panic when types match.
#[test]
fn test_debug_assert_type_tag_macro_correct_type() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gc::init();

    // Create a List object
    let list_obj = list::rt_make_list(8);

    // Use the macro - this should NOT panic
    unsafe {
        debug_assert_type_tag!(list_obj, object::TypeTagKind::List, "test_macro");
    }
    // If we reach here, the test passed
}

/// Test type mismatch: Set passed where Tuple expected.
/// Only runs in debug builds (release builds compile away debug_assert).
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "expected Tuple, got Set")]
fn test_debug_assert_type_tag_tuple_vs_set() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gc::init();

    let set_obj = set::rt_make_set(8);

    unsafe {
        debug_assert_type_tag!(set_obj, object::TypeTagKind::Tuple, "test");
    }
}

/// Test type mismatch: List passed where Dict expected.
/// Only runs in debug builds (release builds compile away debug_assert).
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "expected Dict, got List")]
fn test_debug_assert_type_tag_dict_vs_list() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gc::init();

    let list_obj = list::rt_make_list(4);

    unsafe {
        debug_assert_type_tag!(list_obj, object::TypeTagKind::Dict, "test");
    }
}

/// Test type mismatch: Dict passed where Set expected.
/// Only runs in debug builds (release builds compile away debug_assert).
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "expected Set, got Dict")]
fn test_debug_assert_type_tag_set_vs_dict() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gc::init();

    let dict_obj = dict::rt_make_dict(4);

    unsafe {
        debug_assert_type_tag!(dict_obj, object::TypeTagKind::Set, "test");
    }
}

/// Verify all correct types don't trigger assertions.
#[test]
fn test_correct_types_no_panic() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gc::init();

    unsafe {
        // Allocate all objects and root them together so GC stress mode does not
        // sweep earlier allocations when later ones trigger a collection.
        let list_obj = list::rt_make_list(4);
        let mut roots: [*mut object::Obj; 4] = [
            list_obj,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        ];
        let mut frame = gc::ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 4,
            roots: roots.as_mut_ptr(),
        };
        gc::gc_push(&mut frame);

        roots[1] = dict::rt_make_dict(4);
        roots[2] = set::rt_make_set(4);
        roots[3] = tuple::rt_make_tuple(2);

        gc::gc_pop();

        // All of these should pass without panic
        debug_assert_type_tag!(roots[0], object::TypeTagKind::List, "list");
        debug_assert_type_tag!(roots[1], object::TypeTagKind::Dict, "dict");
        debug_assert_type_tag!(roots[2], object::TypeTagKind::Set, "set");
        debug_assert_type_tag!(roots[3], object::TypeTagKind::Tuple, "tuple");
    }
}
