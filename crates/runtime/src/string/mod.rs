//! String operations for Python runtime
//!
//! This module provides string manipulation functions for the Python AOT compiler runtime.
//! Functions are organized into submodules by functionality.

mod align;
pub mod builder;
mod case;
mod core;
pub mod interning;
mod modify;
mod predicates;
mod search;
pub mod slice;
mod split_join;
mod trim;

// Re-export all public functions
pub use align::{rt_str_center, rt_str_ljust, rt_str_rjust, rt_str_zfill};
pub use case::{rt_str_capitalize, rt_str_lower, rt_str_swapcase, rt_str_title, rt_str_upper};
pub(crate) use core::{count_codepoints, str_alloc_size};
pub use core::{
    rt_make_str, rt_make_str_impl, rt_str_concat, rt_str_data, rt_str_encode, rt_str_len,
    rt_str_len_int,
};
pub use modify::{rt_str_mul, rt_str_replace};
pub use predicates::{
    rt_str_isalnum, rt_str_isalpha, rt_str_isascii, rt_str_isdigit, rt_str_islower, rt_str_isspace,
    rt_str_isupper,
};
pub use search::{
    rt_str_contains, rt_str_count, rt_str_endswith, rt_str_eq, rt_str_find, rt_str_rfind,
    rt_str_search, rt_str_startswith,
};
pub(crate) use slice::utf8_char_width;
pub use slice::{rt_str_getchar, rt_str_slice, rt_str_slice_step};
pub use split_join::{rt_str_join, rt_str_rsplit, rt_str_split};
pub use trim::{rt_str_lstrip, rt_str_rstrip, rt_str_strip};

// Re-export interning functions
pub use interning::{
    init_string_pool, prune_string_pool, rt_make_str_interned, shutdown_string_pool,
};

// Re-export builder functions
pub use builder::{
    rt_make_string_builder, rt_string_builder_append, rt_string_builder_to_str,
    string_builder_finalize,
};

/// Tests for the `StrObj::char_len` cache: every rewritten operation is
/// exercised with a multi-byte string ("приветx😀y") and a pure-ASCII twin,
/// checking both the result content and the cached codepoint count against
/// the canonical `count_codepoints` rule.
#[cfg(test)]
mod char_len_tests {
    use super::modify::{rt_str_expandtabs, rt_str_removeprefix, rt_str_removesuffix};
    use super::*;
    use crate::gc;
    use crate::list::{rt_list_push, rt_make_list};
    use crate::object::{Obj, StrObj};

    fn lock_and_init() -> std::sync::MutexGuard<'static, ()> {
        let guard = crate::RUNTIME_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        gc::init();
        guard
    }

    unsafe fn mk(s: &str) -> *mut Obj {
        rt_make_str(s.as_ptr(), s.len())
    }

    /// Assert content AND the char_len invariant (cache == canonical count
    /// == Rust's chars().count(), since test data is valid UTF-8).
    unsafe fn assert_str(obj: *mut Obj, expected: &str, ctx: &str) {
        assert!(!obj.is_null(), "{ctx}: null result");
        let so = obj as *mut StrObj;
        let bytes = std::slice::from_raw_parts((*so).data.as_ptr(), (*so).len);
        assert_eq!(bytes, expected.as_bytes(), "{ctx}: content mismatch");
        assert_eq!(
            (*so).char_len,
            count_codepoints((*so).data.as_ptr(), (*so).len),
            "{ctx}: char_len out of sync with data"
        );
        assert_eq!(
            (*so).char_len,
            expected.chars().count(),
            "{ctx}: char_len value"
        );
    }

    const UNI: &str = "приветx😀y"; // 9 chars, 18 bytes
    const ASC: &str = "privet_xy"; // 9 chars, 9 bytes

    #[test]
    fn test_len_subscript_getchar() {
        let _guard = lock_and_init();
        unsafe {
            let u = mk(UNI);
            let a = mk(ASC);
            assert_str(u, UNI, "mk uni");
            assert_str(a, ASC, "mk ascii");
            assert_eq!(rt_str_len_int(u), 9);
            assert_eq!(rt_str_len_int(a), 9);
            assert_eq!(rt_str_len(u), 18);

            assert_str(slice::rt_str_subscript(u, 1), "р", "u[1]");
            assert_str(slice::rt_str_subscript(u, -2), "😀", "u[-2]");
            assert_str(slice::rt_str_subscript(a, 0), "p", "a[0]");
            assert_str(slice::rt_str_subscript(a, -1), "y", "a[-1]");

            // getchar takes a BYTE offset (string iteration protocol);
            // the emoji starts at byte 13.
            assert_str(rt_str_getchar(u, 13), "😀", "getchar emoji");
            assert_str(rt_str_getchar(a, 3), "v", "getchar ascii");
        }
    }

    #[test]
    fn test_slice_and_step() {
        let _guard = lock_and_init();
        unsafe {
            let u = mk(UNI);
            let a = mk(ASC);
            assert_str(rt_str_slice(u, 2, 7), "иветx", "u[2:7]");
            assert_str(rt_str_slice(u, 1, -1), "риветx😀", "u[1:-1]");
            assert_str(rt_str_slice(u, i64::MIN, i64::MAX), UNI, "u[:]");
            assert_str(rt_str_slice(u, 5, 2), "", "u[5:2]");
            assert_str(rt_str_slice(a, 2, 7), "ivet_", "a[2:7]");

            assert_str(
                rt_str_slice_step(u, i64::MIN, i64::MAX, -1),
                "y😀xтевирп",
                "u[::-1]",
            );
            assert_str(rt_str_slice_step(u, i64::MIN, i64::MAX, 2), "пиеxy", "u[::2]");
            assert_str(
                rt_str_slice_step(a, i64::MIN, i64::MAX, -1),
                "yx_tevirp",
                "a[::-1]",
            );
            assert_str(rt_str_slice_step(a, 1, 6, 2), "rvt", "a[1:6:2]");
        }
    }

    #[test]
    fn test_concat_mul_join() {
        let _guard = lock_and_init();
        unsafe {
            let u = mk(UNI);
            let abc = mk("abc");
            assert_str(rt_str_concat(u, abc), "приветx😀yabc", "u+abc");
            assert_str(rt_str_concat(abc, abc), "abcabc", "abc+abc");

            let px = mk("пx");
            assert_str(rt_str_mul(px, 3), "пxпxпx", "пx*3");
            assert_str(rt_str_mul(px, 0), "", "пx*0");
            assert_str(rt_str_mul(px, -2), "", "пx*-2");

            let list = rt_make_list(0);
            rt_list_push(list, mk("a"));
            rt_list_push(list, mk("b"));
            let sep = mk("😀");
            assert_str(rt_str_join(sep, list), "a😀b", "😀-join");
            let list2 = rt_make_list(0);
            rt_list_push(list2, mk("пр"));
            rt_list_push(list2, mk("ив"));
            assert_str(rt_str_join(mk(","), list2), "пр,ив", "comma-join");
        }
    }

    #[test]
    fn test_strip_family() {
        let _guard = lock_and_init();
        unsafe {
            let padded = mk("  привет\t\n");
            assert_str(rt_str_strip(padded), "привет", "strip uni");
            let padded_a = mk(" ab \t");
            assert_str(rt_str_strip(padded_a), "ab", "strip ascii");

            assert_str(
                rt_str_lstrip(mk(" \tпривет"), std::ptr::null_mut()),
                "привет",
                "lstrip ws",
            );
            assert_str(
                rt_str_rstrip(mk("привет \n"), std::ptr::null_mut()),
                "привет",
                "rstrip ws",
            );
            assert_str(
                rt_str_lstrip(mk("xyпривет"), mk("yx")),
                "привет",
                "lstrip chars",
            );
            assert_str(
                rt_str_rstrip(mk("приветxy"), mk("yx")),
                "привет",
                "rstrip chars",
            );
        }
    }

    #[test]
    fn test_replace_remove_expandtabs() {
        let _guard = lock_and_init();
        unsafe {
            let u = mk(UNI);
            assert_str(rt_str_replace(u, mk("x"), mk("XY")), "приветXY😀y", "replace");
            assert_str(rt_str_replace(mk("ab"), mk(""), mk("X")), "XaXbX", "replace empty old");
            assert_str(
                rt_str_replace(mk("aбa"), mk("a"), mk("ю")),
                "юбю",
                "replace uni new",
            );

            assert_str(rt_str_removeprefix(u, mk("при")), "ветx😀y", "removeprefix");
            assert_str(rt_str_removesuffix(u, mk("😀y")), "приветx", "removesuffix");
            assert_str(rt_str_removeprefix(mk("ab"), mk("zz")), "ab", "removeprefix miss");

            assert_str(rt_str_expandtabs(mk("a\tб"), 4), "a   б", "expandtabs");
            assert_str(rt_str_expandtabs(mk("\tб"), 2), "  б", "expandtabs leading");
        }
    }

    #[test]
    fn test_find_rfind_count() {
        let _guard = lock_and_init();
        unsafe {
            let u = mk(UNI);
            let a = mk(ASC);
            assert_eq!(rt_str_find(u, mk("вет")), 3, "find uni");
            assert_eq!(rt_str_find(a, mk("vet")), 3, "find ascii");
            assert_eq!(rt_str_find(u, mk("zz")), -1, "find miss");
            assert_eq!(rt_str_rfind(u, mk("😀")), 7, "rfind uni");
            assert_eq!(rt_str_rfind(a, mk("v")), 3, "rfind ascii");
            assert_eq!(rt_str_rfind(u, mk("")), 9, "rfind empty");
            assert_eq!(rt_str_count(u, mk("")), 10, "count empty");
            assert_eq!(rt_str_count(u, mk("п")), 1, "count п");
            assert_eq!(rt_str_count(mk("aaaa"), mk("aa")), 2, "count overlap");
        }
    }

    #[test]
    fn test_align_family() {
        let _guard = lock_and_init();
        unsafe {
            let pr = mk("пр");
            let dash = mk("-");
            let dot = mk(".");
            assert_str(rt_str_center(pr, 5, dash), "--пр-", "center uni");
            assert_str(rt_str_center(mk("ab"), 5, dash), "--ab-", "center ascii");
            assert_str(rt_str_ljust(pr, 4, dot), "пр..", "ljust");
            assert_str(rt_str_rjust(pr, 4, dot), "..пр", "rjust");
            assert_str(rt_str_zfill(mk("-пр"), 5), "-00пр", "zfill");

            // No-op paths must return the original object (early return on
            // cached char_len, no decode/copy).
            let long = mk("привет");
            assert_eq!(rt_str_center(long, 3, dash), long, "center no-op identity");
            assert_eq!(rt_str_rjust(long, 6, dot), long, "rjust no-op identity");
            assert_eq!(rt_str_zfill(long, 2), long, "zfill no-op identity");
        }
    }
}
