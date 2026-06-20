//! Character predicate operations: isdecimal, isdigit, isnumeric, isalpha,
//! isalnum, isspace, isupper, islower, isascii.
//!
//! §9 — Unicode-aware and byte-exact with CPython. Every codepoint predicate is
//! driven off the generated [`unicode_char_table`] (one u8 of packed flags per
//! range), NOT the Rust `char::is_*` family. The std family diverges from CPython
//! two ways the table fixes: (1) different property definitions —
//! `char::is_alphabetic` is the `Alphabetic` derived property (L* + Nl +
//! Other_Alphabetic) where CPython's `isalpha` is category L* only, and
//! `char::is_numeric` cannot tell Numeric_Type=Digit (`²`) from =Numeric (`½`);
//! (2) version skew — rustc's bundled UCD and the gate's CPython track different
//! Unicode releases. The table is generated FROM the gate's CPython, so both
//! vanish. A non-codepoint (invalid UTF-8) or empty string is `False` for every
//! predicate; `isascii` stays a pure byte test (no table needed).

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::object::{Obj, StrObj, TypeTagKind};
use crate::string::unicode_char_table::CHAR_TABLE;
use pyaot_core_defs::Value;

// Flag layout of a `CHAR_TABLE` entry (see tools/gen_unicode_tables.py).
const RANK_MASK: u8 = 0b11; // bits 0-1: 3=Decimal, 2=Digit, 1=Numeric, 0=none
const CASE_LOWER: u8 = 1; // bits 2-3 case class, after `>> 2`
const CASE_UPPER: u8 = 2;
#[allow(dead_code)] // documents the layout; titlecase is rejected via `class != want`
const CASE_TITLE: u8 = 3;
const FLAG_ALPHA: u8 = 0x10; // bit 4
const FLAG_SPACE: u8 = 0x20; // bit 5

/// Decode a `StrObj` to `&str`, or `None` for a null pointer / invalid UTF-8.
/// `who` is the caller name for the B18 type-tag seam guard.
///
/// # Safety
/// `str_obj` must be null or a valid `StrObj` pointer.
#[inline]
unsafe fn str_chars<'a>(str_obj: *mut Obj, who: &str) -> Option<&'a str> {
    if str_obj.is_null() {
        return None;
    }
    debug_assert_type_tag!(str_obj, TypeTagKind::Str, who);
    let src = str_obj as *mut StrObj;
    let len = (*src).len;
    let bytes = std::slice::from_raw_parts((*src).data.as_ptr(), len);
    std::str::from_utf8(bytes).ok()
}

/// `True` iff the string is non-empty and every codepoint satisfies `pred`.
#[inline]
unsafe fn all_chars(str_obj: *mut Obj, who: &str, pred: fn(char) -> bool) -> i8 {
    match str_chars(str_obj, who) {
        Some(s) if !s.is_empty() => s.chars().all(pred) as i8,
        _ => 0,
    }
}

/// Packed Unicode flags of `c` (0 = no tracked property). Binary search over the
/// generated, sorted range table (O(log n), no alloc).
#[inline]
fn char_flags(c: char) -> u8 {
    let cp = c as u32;
    let i = CHAR_TABLE.partition_point(|&(start, _, _)| start <= cp);
    match i.checked_sub(1) {
        Some(j) => {
            let (start, end, flags) = CHAR_TABLE[j];
            if cp >= start && cp <= end {
                flags
            } else {
                0
            }
        }
        None => 0,
    }
}

/// Unicode `Numeric_Type` rank of `c`: 3=Decimal, 2=Digit, 1=Numeric, 0=none.
#[inline]
fn numeric_rank(c: char) -> u8 {
    char_flags(c) & RANK_MASK
}

/// Per-char case class of `c`: 1=lower, 2=upper, 3=title, 0=uncased.
#[inline]
fn case_class(c: char) -> u8 {
    (char_flags(c) >> 2) & 0b11
}

/// Shared engine for `isupper`/`islower`: every cased character must be `want`
/// (a titlecase or opposite-case character fails), and at least one must be
/// `want`. Mirrors CPython's `unicode_isupper_impl`/`unicode_islower_impl`.
#[inline]
unsafe fn all_cased(str_obj: *mut Obj, who: &str, want: u8) -> i8 {
    let Some(s) = str_chars(str_obj, who) else {
        return 0;
    };
    let mut has_cased = false;
    for c in s.chars() {
        let class = case_class(c);
        if class != 0 && class != want {
            return 0; // a cased char that is the opposite case or titlecase
        }
        if class == want {
            has_cased = true;
        }
    }
    has_cased as i8
}

/// Check if all characters are decimal (`Numeric_Type=Decimal`, §9).
/// Returns: 1 (true) or 0 (false)
pub fn rt_str_isdecimal(str_obj: *mut Obj) -> i8 {
    unsafe { all_chars(str_obj, "rt_str_isdecimal", |c| numeric_rank(c) >= 3) }
}
#[export_name = "rt_str_isdecimal"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_isdecimal_abi(str_obj: Value) -> i8 {
    rt_str_isdecimal(str_obj.unwrap_ptr())
}

/// Check if all characters are digits (`Numeric_Type` ∈ {Decimal, Digit}, §9).
/// Returns: 1 (true) or 0 (false)
pub fn rt_str_isdigit(str_obj: *mut Obj) -> i8 {
    unsafe { all_chars(str_obj, "rt_str_isdigit", |c| numeric_rank(c) >= 2) }
}
#[export_name = "rt_str_isdigit"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_isdigit_abi(str_obj: Value) -> i8 {
    rt_str_isdigit(str_obj.unwrap_ptr())
}

/// Check if all characters are numeric (`Numeric_Type` ∈ {Decimal, Digit,
/// Numeric}, §9 — includes fractions `½`, Roman numerals, CJK numerals `一`).
/// Returns: 1 (true) or 0 (false)
pub fn rt_str_isnumeric(str_obj: *mut Obj) -> i8 {
    unsafe { all_chars(str_obj, "rt_str_isnumeric", |c| numeric_rank(c) >= 1) }
}
#[export_name = "rt_str_isnumeric"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_isnumeric_abi(str_obj: Value) -> i8 {
    rt_str_isnumeric(str_obj.unwrap_ptr())
}

/// Check if all characters are alphabetic (CPython category L* only, §9 — `Nl`
/// like Roman numerals and combining `Other_Alphabetic` marks are NOT alpha).
/// Returns: 1 (true) or 0 (false)
pub fn rt_str_isalpha(str_obj: *mut Obj) -> i8 {
    unsafe { all_chars(str_obj, "rt_str_isalpha", |c| char_flags(c) & FLAG_ALPHA != 0) }
}
#[export_name = "rt_str_isalpha"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_isalpha_abi(str_obj: Value) -> i8 {
    rt_str_isalpha(str_obj.unwrap_ptr())
}

/// Check if all characters are alphanumeric (§9 — `isalpha` OR any
/// `Numeric_Type`, exactly CPython's `isalnum`).
/// Returns: 1 (true) or 0 (false)
pub fn rt_str_isalnum(str_obj: *mut Obj) -> i8 {
    unsafe {
        all_chars(str_obj, "rt_str_isalnum", |c| {
            let f = char_flags(c);
            f & FLAG_ALPHA != 0 || f & RANK_MASK != 0
        })
    }
}
#[export_name = "rt_str_isalnum"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_isalnum_abi(str_obj: Value) -> i8 {
    rt_str_isalnum(str_obj.unwrap_ptr())
}

/// Check if all characters are whitespace (§9 — CPython's space set: Unicode
/// White_Space plus the file/group/record/unit separators U+001C..U+001F).
/// Returns: 1 (true) or 0 (false)
pub fn rt_str_isspace(str_obj: *mut Obj) -> i8 {
    unsafe { all_chars(str_obj, "rt_str_isspace", |c| char_flags(c) & FLAG_SPACE != 0) }
}
#[export_name = "rt_str_isspace"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_isspace_abi(str_obj: Value) -> i8 {
    rt_str_isspace(str_obj.unwrap_ptr())
}

/// Check if all cased characters are uppercase and at least one is (§9). A
/// lowercase or titlecase character fails.
/// Returns: 1 (true) or 0 (false)
pub fn rt_str_isupper(str_obj: *mut Obj) -> i8 {
    unsafe { all_cased(str_obj, "rt_str_isupper", CASE_UPPER) }
}
#[export_name = "rt_str_isupper"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_isupper_abi(str_obj: Value) -> i8 {
    rt_str_isupper(str_obj.unwrap_ptr())
}

/// Check if all cased characters are lowercase and at least one is (§9). An
/// uppercase or titlecase character fails.
/// Returns: 1 (true) or 0 (false)
pub fn rt_str_islower(str_obj: *mut Obj) -> i8 {
    unsafe { all_cased(str_obj, "rt_str_islower", CASE_LOWER) }
}
#[export_name = "rt_str_islower"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_islower_abi(str_obj: Value) -> i8 {
    rt_str_islower(str_obj.unwrap_ptr())
}

/// Check if all characters are ASCII (code points < 128)
/// Returns: 1 (true) or 0 (false)
/// Empty string returns 1 (Python behavior)
pub fn rt_str_isascii(str_obj: *mut Obj) -> i8 {
    if str_obj.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_isascii");
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        // Empty string is ASCII
        if len == 0 {
            return 1;
        }

        let data = (*src).data.as_ptr();
        for i in 0..len {
            if !(*data.add(i)).is_ascii() {
                return 0;
            }
        }
        1
    }
}
#[export_name = "rt_str_isascii"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_isascii_abi(str_obj: Value) -> i8 {
    rt_str_isascii(str_obj.unwrap_ptr())
}

#[cfg(test)]
mod tests {
    use super::{case_class, char_flags, numeric_rank, CASE_LOWER, CASE_TITLE, CASE_UPPER,
        FLAG_ALPHA};

    /// The numeric trio: ranks the codepoints the old `char::is_numeric` path
    /// got wrong, plus the boundaries and a non-numeric control.
    #[test]
    fn numeric_rank_matches_cpython() {
        assert_eq!(numeric_rank('0'), 3); // Decimal
        assert_eq!(numeric_rank('\u{0660}'), 3); // ARABIC-INDIC DIGIT ZERO
        assert_eq!(numeric_rank('\u{00B2}'), 2); // ² SUPERSCRIPT TWO (Digit)
        assert_eq!(numeric_rank('\u{00BD}'), 1); // ½ FRACTION ONE HALF (Numeric)
        assert_eq!(numeric_rank('\u{2167}'), 1); // Ⅷ ROMAN NUMERAL EIGHT
        assert_eq!(numeric_rank('\u{4E00}'), 1); // 一 CJK numeral one (category Lo)
        assert_eq!(numeric_rank('a'), 0);
        assert_eq!(numeric_rank('\u{4E01}'), 0); // 丁 — adjacent CJK, not numeric
    }

    /// isalpha is category L* only: Nl and combining `Other_Alphabetic` are not
    /// alpha, where `char::is_alphabetic` wrongly said yes.
    #[test]
    fn alpha_matches_cpython() {
        assert_ne!(char_flags('a') & FLAG_ALPHA, 0);
        assert_ne!(char_flags('\u{00E9}') & FLAG_ALPHA, 0); // é
        assert_eq!(char_flags('\u{2167}') & FLAG_ALPHA, 0); // Ⅷ Nl — NOT alpha
        assert_eq!(char_flags('\u{0345}') & FLAG_ALPHA, 0); // combining iota — NOT alpha
        assert_eq!(char_flags('0') & FLAG_ALPHA, 0);
    }

    /// Case class, incl. the titlecase digraph that blocks both isupper/islower.
    #[test]
    fn case_class_matches_cpython() {
        assert_eq!(case_class('A'), CASE_UPPER);
        assert_eq!(case_class('a'), CASE_LOWER);
        assert_eq!(case_class('\u{01C5}'), CASE_TITLE); // ǅ LATIN CAPITAL D WITH SMALL Z
        assert_eq!(case_class('5'), 0); // uncased
    }

    /// Drift guard: the committed table must still match the system CPython that
    /// the differential gate compares against. Skips where python3 is absent —
    /// the same dependency the gate already requires. One subprocess, then a
    /// byte-compare against the regenerated output.
    #[test]
    fn char_table_matches_system_python() {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/gen_unicode_tables.py");
        let Ok(out) = std::process::Command::new("python3").arg(script).output() else {
            return; // no python3 here — nothing to compare against
        };
        if !out.status.success() {
            return; // present but failed (e.g. too old) — don't fail unrelated builds
        }
        let regenerated = String::from_utf8(out.stdout).expect("generator output is utf-8");
        let committed = include_str!("unicode_char_table.rs");
        assert_eq!(
            regenerated, committed,
            "unicode_char_table.rs drifted from system CPython — \
             rerun tools/gen_unicode_tables.py",
        );
    }
}
