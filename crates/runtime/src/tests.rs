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
use crate::{dict, gc, iterator, list, object, set, string, tuple};

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

// ---------------------------------------------------------------------------
// Defence-in-depth guards at the proof-trusted `Tagged → Heap` stdlib seam.
//
// These exercise the `debug_assert_type_tag!` guards added to the blind-cast
// `rt_str_*` / `rt_iter_*` entry points: a wrong-shape `Value` (here a Dict
// built via `rt_make_dict`) reaching one of them now surfaces as a clear debug
// panic ("expected <Shape>, got Dict") instead of a raw SEGV in the frozen
// runtime (the Phase 8B–8F gradual-seam family). We call the plain `pub fn`
// (NOT the `extern "C"` `_abi` shims, which cannot unwind — see the file
// header), so `#[should_panic]` works. They only fire in debug builds; the
// linked release staticlib compiles the guard away, so this is zero-cost there.
// ---------------------------------------------------------------------------

/// `str.join` on a non-list receiver — the named SEGV ("join on a non-list").
/// `rt_str_join(sep, list_obj)`: sep=null, list_obj=a Dict (wrong shape).
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "rt_str_join: expected List, got Dict")]
fn test_guard_str_join_wrong_list() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gc::init();

    let dict_obj = dict::rt_make_dict(8);
    string::rt_str_join(std::ptr::null_mut(), dict_obj);
}

/// `str.split` receiver guard wiring (split_join.rs).
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "rt_str_split: expected Str, got Dict")]
fn test_guard_str_split_wrong_receiver() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gc::init();

    let dict_obj = dict::rt_make_dict(8);
    string::rt_str_split(dict_obj, std::ptr::null_mut(), -1);
}

/// `str.upper` receiver guard wiring (case.rs).
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "rt_str_upper: expected Str, got Dict")]
fn test_guard_str_upper_wrong_receiver() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gc::init();

    let dict_obj = dict::rt_make_dict(8);
    string::rt_str_upper(dict_obj);
}

/// `str.strip` receiver guard wiring (trim.rs).
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "rt_str_strip: expected Str, got Dict")]
fn test_guard_str_strip_wrong_receiver() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gc::init();

    let dict_obj = dict::rt_make_dict(8);
    string::rt_str_strip(dict_obj);
}

/// `str.find` receiver guard wiring (search.rs). Both args are the Dict: the
/// joint `is_null` early-return is bypassed (both non-null), then the receiver
/// guard fires before `sub` is ever dereferenced.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "rt_str_find: expected Str, got Dict")]
fn test_guard_str_find_wrong_receiver() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gc::init();

    let dict_obj = dict::rt_make_dict(8);
    string::rt_str_find(dict_obj, dict_obj);
}

/// Iterator factory receiver guard wiring (iterator/factory.rs).
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "rt_iter_list: expected List, got Dict")]
fn test_guard_iter_list_wrong_receiver() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gc::init();

    let dict_obj = dict::rt_make_dict(8);
    iterator::rt_iter_list(dict_obj);
}

/// Correct shapes flow through the new seam guards without panic — proves the
/// guards do not fire on the legitimate path.
#[test]
fn test_guard_correct_shapes_no_panic() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gc::init();

    // A real List flows through rt_iter_list. Root it across the iterator
    // allocation so GC stress mode cannot sweep it.
    let list_obj = list::rt_make_list(4);
    let mut roots: [*mut object::Obj; 1] = [list_obj];
    let mut frame = gc::ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    unsafe { gc::gc_push(&mut frame) };
    let _iter = iterator::rt_iter_list(roots[0]);
    gc::gc_pop();

    // A real String flows through rt_str_upper (it copies the bytes before
    // allocating, so the source needs no rooting across the call).
    unsafe {
        let s = string::rt_make_str(std::ptr::null(), 0);
        let _upper = string::rt_str_upper(s);
    }
}

// ── Backlog §3: the `del` runtime helpers (happy paths) ─────────────────────
//
// The raise paths (IndexError/KeyError on miss, UnboundLocalError/… on the
// sentinel) unwind via the table-based unwinder rather than `panic!`, so they
// cannot be `#[should_panic]`-tested here — the compiled corpus probe exercises
// them end-to-end. These cover the success paths.

#[test]
fn rt_list_delete_removes_and_shifts() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gc::init();
    use pyaot_core_defs::Value;
    let li = list::rt_make_list(4);
    for v in [10i64, 20, 30] {
        list::rt_list_push(li, Value::from_int(v).0 as *mut object::Obj);
    }
    // del li[1] → [10, 30]
    list::rt_list_delete(li, 1);
    assert_eq!(list::rt_list_len(li), 2);
    assert_eq!(Value(list::rt_list_get(li, 0) as u64).unwrap_int(), 10);
    assert_eq!(Value(list::rt_list_get(li, 1) as u64).unwrap_int(), 30);
    // del li[-1] → [10] (negative index)
    list::rt_list_delete(li, -1);
    assert_eq!(list::rt_list_len(li), 1);
    assert_eq!(Value(list::rt_list_get(li, 0) as u64).unwrap_int(), 10);
}

#[test]
fn rt_dict_delete_removes_entry() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gc::init();
    use pyaot_core_defs::Value;
    let d = dict::rt_make_dict(8);
    let k1 = Value::from_int(1).0 as *mut object::Obj;
    let k2 = Value::from_int(2).0 as *mut object::Obj;
    dict::rt_dict_set(d, k1, Value::from_int(100).0 as *mut object::Obj);
    dict::rt_dict_set(d, k2, Value::from_int(200).0 as *mut object::Obj);
    assert_eq!(dict::rt_dict_len(d), 2);
    dict::rt_dict_delete(d, k1);
    assert_eq!(dict::rt_dict_len(d), 1);
    assert_eq!(dict::rt_dict_contains(d, k1), 0);
    assert_eq!(dict::rt_dict_contains(d, k2), 1);
}

#[test]
fn rt_any_delitem_dispatches_to_list() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gc::init();
    use pyaot_core_defs::Value;
    let li = list::rt_make_list(4);
    for v in [1i64, 2, 3] {
        list::rt_list_push(li, Value::from_int(v).0 as *mut object::Obj);
    }
    // Runtime-dispatched del container[0] → list delete.
    crate::ops::rt_any_delitem(li, 0);
    assert_eq!(list::rt_list_len(li), 2);
    assert_eq!(Value(list::rt_list_get(li, 0) as u64).unwrap_int(), 2);
}

#[test]
fn rt_check_bound_passes_through_a_bound_value() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gc::init();
    use pyaot_core_defs::Value;
    // A real value flows through unchanged for every kind (no raise).
    for kind in [0i64, 1, 2] {
        let v = Value::from_int(42);
        assert_eq!(crate::ops::rt_check_bound(v, kind, std::ptr::null_mut()), v);
    }
    // `None` is a bound value too — only the UNBOUND sentinel raises.
    assert_eq!(
        crate::ops::rt_check_bound(Value::NONE, 0, std::ptr::null_mut()),
        Value::NONE
    );
    // The sentinel itself is detected (the raise path unwinds, so it is only
    // exercised end-to-end by the corpus probe, not here).
    assert!(Value::UNBOUND.is_unbound());
}

#[test]
fn rt_unbox_bool_unboxes_tagged_bools() {
    let _guard = crate::RUNTIME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    use pyaot_core_defs::Value;
    // The success path of the third checked-unbox shape (Tagged -> Raw(I8)): a
    // tagged True/False unboxes to 1/0. The strict wrong-shape guard (TypeError
    // on any non-bool tag — int, float, None, heap) longjmps via `raise_exc!`,
    // so like the other raise paths it is exercised end-to-end by the corpus
    // (an `extern "C"` raise cannot unwind into a `#[test]`), not here.
    assert_eq!(crate::boxing::rt_unbox_bool_abi(Value::TRUE), 1);
    assert_eq!(crate::boxing::rt_unbox_bool_abi(Value::FALSE), 0);
}
