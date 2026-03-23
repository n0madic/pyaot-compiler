//! re (regex) module runtime support
//!
//! Provides:
//! - re.search(pattern, string) -> Match | None
//! - re.match(pattern, string) -> Match | None
//! - re.sub(pattern, repl, string) -> str
//!
//! Match object methods:
//! - match.group(n) -> str | None
//! - match.start() -> int
//! - match.end() -> int
//! - match.groups() -> tuple[str, ...]
//! - match.span() -> tuple[int, int]

use crate::gc;
use crate::object::{MatchObj, Obj, ObjHeader, TupleObj, TypeTagKind};
use crate::utils::make_str_from_rust;
use regex_lite::Regex;

/// Create a MatchObj from regex match result
unsafe fn create_match_obj(
    matched: bool,
    start: i64,
    end: i64,
    groups: Vec<Option<&str>>,
    original: *mut Obj,
) -> *mut Obj {
    if !matched {
        // Return None for no match
        return crate::object::none_obj();
    }

    // Create tuple of groups
    let groups_count = groups.len();
    let tuple_size =
        std::mem::size_of::<TupleObj>() + groups_count * std::mem::size_of::<*mut Obj>();
    let tuple_ptr = gc::gc_alloc(tuple_size, TypeTagKind::Tuple as u8) as *mut TupleObj;

    (*tuple_ptr).header = ObjHeader {
        type_tag: TypeTagKind::Tuple,
        marked: false,
        size: tuple_size,
    };
    (*tuple_ptr).len = groups_count;

    // Fill tuple with group strings (None for unmatched groups)
    for (i, group) in groups.iter().enumerate() {
        let group_str = match group {
            Some(s) => make_str_from_rust(s),
            None => crate::object::none_obj(),
        };
        *(*tuple_ptr).data.as_mut_ptr().add(i) = group_str;
    }

    // Create MatchObj
    let match_size = std::mem::size_of::<MatchObj>();
    let match_ptr = gc::gc_alloc(match_size, TypeTagKind::Match as u8) as *mut MatchObj;

    (*match_ptr).header = ObjHeader {
        type_tag: TypeTagKind::Match,
        marked: false,
        size: match_size,
    };
    (*match_ptr).matched = matched;
    (*match_ptr).start = start;
    (*match_ptr).end = end;
    (*match_ptr).groups = tuple_ptr as *mut Obj;
    (*match_ptr).original = original;

    match_ptr as *mut Obj
}

/// Search for pattern anywhere in string
/// Returns Match object if found, None otherwise
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_re_search(pattern: *mut Obj, string: *mut Obj) -> *mut Obj {
    unsafe {
        let pattern_str = match crate::utils::extract_str_checked(pattern) {
            Some(s) => s,
            None => return crate::object::none_obj(),
        };

        let string_str = match crate::utils::extract_str_checked(string) {
            Some(s) => s,
            None => return crate::object::none_obj(),
        };

        // Compile regex
        let re = match Regex::new(&pattern_str) {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("re.error: {}", e);
                crate::exceptions::rt_exc_raise(
                    crate::exceptions::ExceptionType::ValueError as u8,
                    msg.as_ptr(),
                    msg.len(),
                );
            }
        };

        // Search for match
        match re.captures(&string_str) {
            Some(caps) => {
                let m = caps.get(0).expect("regex capture must have group 0");
                // Convert byte offsets to character offsets for CPython compatibility
                let start_chars = string_str[..m.start()].chars().count() as i64;
                let end_chars = string_str[..m.end()].chars().count() as i64;

                // Collect all groups (including group 0 = full match)
                let groups: Vec<Option<&str>> = (0..caps.len())
                    .map(|i| caps.get(i).map(|m| m.as_str()))
                    .collect();

                create_match_obj(true, start_chars, end_chars, groups, string)
            }
            None => crate::object::none_obj(),
        }
    }
}

/// Match pattern at start of string
/// Returns Match object if found at start, None otherwise
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_re_match(pattern: *mut Obj, string: *mut Obj) -> *mut Obj {
    unsafe {
        let pattern_str = match crate::utils::extract_str_checked(pattern) {
            Some(s) => s,
            None => return crate::object::none_obj(),
        };

        let string_str = match crate::utils::extract_str_checked(string) {
            Some(s) => s,
            None => return crate::object::none_obj(),
        };

        // Compile regex with anchor at start
        let anchored_pattern = format!("^(?:{})", pattern_str);
        let re = match Regex::new(&anchored_pattern) {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("re.error: {}", e);
                crate::exceptions::rt_exc_raise(
                    crate::exceptions::ExceptionType::ValueError as u8,
                    msg.as_ptr(),
                    msg.len(),
                );
            }
        };

        // Match at start
        match re.captures(&string_str) {
            Some(caps) => {
                let m = caps.get(0).expect("regex capture must have group 0");
                // Convert byte offsets to character offsets for CPython compatibility
                let start_chars = string_str[..m.start()].chars().count() as i64;
                let end_chars = string_str[..m.end()].chars().count() as i64;

                // Collect all groups
                let groups: Vec<Option<&str>> = (0..caps.len())
                    .map(|i| caps.get(i).map(|m| m.as_str()))
                    .collect();

                create_match_obj(true, start_chars, end_chars, groups, string)
            }
            None => crate::object::none_obj(),
        }
    }
}

/// Substitute all occurrences of pattern with replacement
/// Returns new string with substitutions
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_re_sub(pattern: *mut Obj, repl: *mut Obj, string: *mut Obj) -> *mut Obj {
    unsafe {
        let pattern_str = match crate::utils::extract_str_checked(pattern) {
            Some(s) => s,
            None => return string, // Return original on error
        };

        let repl_str = match crate::utils::extract_str_checked(repl) {
            Some(s) => s,
            None => return string,
        };

        let string_str = match crate::utils::extract_str_checked(string) {
            Some(s) => s,
            None => return string,
        };

        // Compile regex
        let re = match Regex::new(&pattern_str) {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("re.error: {}", e);
                crate::exceptions::rt_exc_raise(
                    crate::exceptions::ExceptionType::ValueError as u8,
                    msg.as_ptr(),
                    msg.len(),
                );
            }
        };

        // Translate Python replacement syntax (\1, \2, \g<name>) to regex syntax ($1, $2)
        let translated_repl = repl_str
            .replace("\\1", "$1")
            .replace("\\2", "$2")
            .replace("\\3", "$3")
            .replace("\\4", "$4")
            .replace("\\5", "$5")
            .replace("\\6", "$6")
            .replace("\\7", "$7")
            .replace("\\8", "$8")
            .replace("\\9", "$9")
            .replace("\\0", "$0");
        // Replace all occurrences
        let result = re.replace_all(&string_str, translated_repl.as_str());

        make_str_from_rust(&result)
    }
}

/// Get a match group by index
/// Returns string for that group, or None if group didn't participate
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_match_group(m: *mut Obj, n: i64) -> *mut Obj {
    unsafe {
        if m.is_null() || (*m).header.type_tag != TypeTagKind::Match {
            return crate::object::none_obj();
        }

        let match_obj = m as *const MatchObj;

        if !(*match_obj).matched {
            return crate::object::none_obj();
        }

        let groups = (*match_obj).groups;
        if groups.is_null() {
            return crate::object::none_obj();
        }

        let tuple = groups as *const TupleObj;
        let groups_len = (*tuple).len;

        if n < 0 || n >= groups_len as i64 {
            return crate::object::none_obj();
        }

        *(*tuple).data.as_ptr().add(n as usize)
    }
}

/// Get start position of match
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_match_start(m: *mut Obj) -> i64 {
    unsafe {
        if m.is_null() || (*m).header.type_tag != TypeTagKind::Match {
            return -1;
        }

        let match_obj = m as *const MatchObj;
        (*match_obj).start
    }
}

/// Get end position of match
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_match_end(m: *mut Obj) -> i64 {
    unsafe {
        if m.is_null() || (*m).header.type_tag != TypeTagKind::Match {
            return -1;
        }

        let match_obj = m as *const MatchObj;
        (*match_obj).end
    }
}

/// Get all groups as a tuple (excluding group 0)
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_match_groups(m: *mut Obj) -> *mut Obj {
    unsafe {
        if m.is_null() || (*m).header.type_tag != TypeTagKind::Match {
            // Return empty tuple
            let tuple_size = std::mem::size_of::<TupleObj>();
            let tuple_ptr = gc::gc_alloc(tuple_size, TypeTagKind::Tuple as u8) as *mut TupleObj;

            (*tuple_ptr).header = ObjHeader {
                type_tag: TypeTagKind::Tuple,
                marked: false,
                size: tuple_size,
            };
            (*tuple_ptr).len = 0;

            return tuple_ptr as *mut Obj;
        }

        let match_obj = m as *const MatchObj;

        if !(*match_obj).matched {
            // Return empty tuple
            let tuple_size = std::mem::size_of::<TupleObj>();
            let tuple_ptr = gc::gc_alloc(tuple_size, TypeTagKind::Tuple as u8) as *mut TupleObj;

            (*tuple_ptr).header = ObjHeader {
                type_tag: TypeTagKind::Tuple,
                marked: false,
                size: tuple_size,
            };
            (*tuple_ptr).len = 0;

            return tuple_ptr as *mut Obj;
        }

        let groups = (*match_obj).groups;
        if groups.is_null() {
            // Return empty tuple
            let tuple_size = std::mem::size_of::<TupleObj>();
            let tuple_ptr = gc::gc_alloc(tuple_size, TypeTagKind::Tuple as u8) as *mut TupleObj;

            (*tuple_ptr).header = ObjHeader {
                type_tag: TypeTagKind::Tuple,
                marked: false,
                size: tuple_size,
            };
            (*tuple_ptr).len = 0;

            return tuple_ptr as *mut Obj;
        }

        let original_tuple = groups as *const TupleObj;
        let original_len = (*original_tuple).len;

        // Create new tuple excluding group 0
        if original_len <= 1 {
            // Return empty tuple
            let tuple_size = std::mem::size_of::<TupleObj>();
            let tuple_ptr = gc::gc_alloc(tuple_size, TypeTagKind::Tuple as u8) as *mut TupleObj;

            (*tuple_ptr).header = ObjHeader {
                type_tag: TypeTagKind::Tuple,
                marked: false,
                size: tuple_size,
            };
            (*tuple_ptr).len = 0;

            return tuple_ptr as *mut Obj;
        }

        let new_len = original_len - 1;
        let tuple_size =
            std::mem::size_of::<TupleObj>() + new_len * std::mem::size_of::<*mut Obj>();
        let tuple_ptr = gc::gc_alloc(tuple_size, TypeTagKind::Tuple as u8) as *mut TupleObj;

        (*tuple_ptr).header = ObjHeader {
            type_tag: TypeTagKind::Tuple,
            marked: false,
            size: tuple_size,
        };
        (*tuple_ptr).len = new_len;

        // Copy groups 1..n (skip group 0)
        for i in 0..new_len {
            *(*tuple_ptr).data.as_mut_ptr().add(i) = *(*original_tuple).data.as_ptr().add(i + 1);
        }

        tuple_ptr as *mut Obj
    }
}

/// Get span (start, end) of match as a tuple of two integers
/// Returns a tuple (start, end) using raw int storage
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_match_span(m: *mut Obj) -> *mut Obj {
    use crate::object::ELEM_RAW_INT;

    unsafe {
        let (start, end) = if m.is_null() || (*m).header.type_tag != TypeTagKind::Match {
            (-1i64, -1i64)
        } else {
            let match_obj = m as *const MatchObj;
            ((*match_obj).start, (*match_obj).end)
        };

        // Create a 2-element tuple with raw int storage
        let tuple_size = std::mem::size_of::<TupleObj>() + 2 * std::mem::size_of::<*mut Obj>();
        let tuple_ptr = gc::gc_alloc(tuple_size, TypeTagKind::Tuple as u8) as *mut TupleObj;

        (*tuple_ptr).header = ObjHeader {
            type_tag: TypeTagKind::Tuple,
            marked: false,
            size: tuple_size,
        };
        (*tuple_ptr).len = 2;
        (*tuple_ptr).elem_tag = ELEM_RAW_INT;

        // Store start and end as raw i64 values
        *(*tuple_ptr).data.as_mut_ptr().add(0) = start as *mut Obj;
        *(*tuple_ptr).data.as_mut_ptr().add(1) = end as *mut Obj;

        tuple_ptr as *mut Obj
    }
}
