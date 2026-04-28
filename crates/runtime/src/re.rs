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

use crate::gc::{self, ShadowFrame};
use crate::object::{MatchObj, Obj, ObjHeader, TupleObj, TypeTagKind};
use crate::utils::make_str_from_rust;
use pyaot_core_defs::Value;
use regex_lite::Regex;

/// Create a MatchObj from regex match result
unsafe fn create_match_obj(
    matched: bool,
    start: i64,
    end: i64,
    groups: Vec<Option<&str>>,
    original: *mut Obj,
) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push};
    use crate::tuple::{rt_make_tuple, rt_tuple_set};

    if !matched {
        // Return None for no match
        return crate::object::none_obj();
    }

    let groups_count = groups.len();

    // Allocate the groups tuple via rt_make_tuple so the slots are
    // tagged Values that the GC walk can follow (str pointers).
    // Pre-§F.7 we needed a per-tuple heap mask to make the walk
    // safe; post-§F.7 every tuple slot is uniformly tagged.
    //
    // Root both the groups tuple and original across all allocating calls:
    //   - make_str_from_rust / gc_alloc for each group string
    //   - gc_alloc for the MatchObj
    let groups_tuple = rt_make_tuple(groups_count as i64);

    // roots[0] = groups_tuple, roots[1] = original (the source string)
    let mut roots: [*mut Obj; 2] = [groups_tuple, original];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 2,
        roots: roots.as_mut_ptr(),
    };
    gc_push(&mut frame);

    // Fill tuple with group strings (None for unmatched groups).
    // Re-derive the tuple pointer from roots[0] after each allocating call so
    // we always use the live pointer (non-moving GC — address is stable, but
    // being explicit guards against future changes).
    for (i, group) in groups.iter().enumerate() {
        let group_str = match group {
            Some(s) => make_str_from_rust(s),
            None => crate::object::none_obj(),
        };
        rt_tuple_set(roots[0], i as i64, group_str);
    }

    // Create MatchObj — tuple and original are still rooted.
    let match_size = std::mem::size_of::<MatchObj>();
    let match_ptr = gc::gc_alloc(match_size, TypeTagKind::Match as u8) as *mut MatchObj;

    gc_pop();

    (*match_ptr).header = ObjHeader {
        type_tag: TypeTagKind::Match,
        marked: false,
        size: match_size,
    };
    (*match_ptr).matched = matched;
    (*match_ptr).start = start;
    (*match_ptr).end = end;
    (*match_ptr).groups = roots[0]; // live tuple pointer after GC
    (*match_ptr).original = roots[1]; // live original pointer after GC

    match_ptr as *mut Obj
}

/// Search for pattern anywhere in string
/// Returns Match object if found, None otherwise
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_re_search(pattern: *mut Obj, string: *mut Obj) -> *mut Obj {
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
                crate::raise_exc!(
                    crate::exceptions::ExceptionType::ValueError,
                    "re.error: {}",
                    e
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
#[export_name = "rt_re_search"]
pub extern "C" fn rt_re_search_abi(pattern: Value, string: Value) -> Value {
    Value::from_ptr(rt_re_search(pattern.unwrap_ptr(), string.unwrap_ptr()))
}

/// Match pattern at start of string
/// Returns Match object if found at start, None otherwise
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_re_match(pattern: *mut Obj, string: *mut Obj) -> *mut Obj {
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
                crate::raise_exc!(
                    crate::exceptions::ExceptionType::ValueError,
                    "re.error: {}",
                    e
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
#[export_name = "rt_re_match"]
pub extern "C" fn rt_re_match_abi(pattern: Value, string: Value) -> Value {
    Value::from_ptr(rt_re_match(pattern.unwrap_ptr(), string.unwrap_ptr()))
}

/// Substitute all occurrences of pattern with replacement
/// Returns new string with substitutions
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_re_sub(pattern: *mut Obj, repl: *mut Obj, string: *mut Obj) -> *mut Obj {
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
                crate::raise_exc!(
                    crate::exceptions::ExceptionType::ValueError,
                    "re.error: {}",
                    e
                );
            }
        };

        // Translate Python replacement backreferences (\1..\9) to regex-crate syntax ($1..$9).
        // Python's \0 is the null byte, NOT a backreference — do not translate it.
        let translated_repl = repl_str
            .replace("\\1", "$1")
            .replace("\\2", "$2")
            .replace("\\3", "$3")
            .replace("\\4", "$4")
            .replace("\\5", "$5")
            .replace("\\6", "$6")
            .replace("\\7", "$7")
            .replace("\\8", "$8")
            .replace("\\9", "$9");
        // Replace all occurrences
        let result = re.replace_all(&string_str, translated_repl.as_str());

        make_str_from_rust(&result)
    }
}
#[export_name = "rt_re_sub"]
pub extern "C" fn rt_re_sub_abi(pattern: Value, repl: Value, string: Value) -> Value {
    Value::from_ptr(rt_re_sub(
        pattern.unwrap_ptr(),
        repl.unwrap_ptr(),
        string.unwrap_ptr(),
    ))
}

/// Get a match group by index
/// Returns string for that group, or None if group didn't participate
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_match_group(m: *mut Obj, n: i64) -> *mut Obj {
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

        let val = *(*tuple).data.as_ptr().add(n as usize);
        val.0 as *mut Obj
    }
}
#[export_name = "rt_match_group"]
pub extern "C" fn rt_match_group_abi(m: Value, n: i64) -> Value {
    Value::from_ptr(rt_match_group(m.unwrap_ptr(), n))
}

/// Get start position of match
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_match_start(m: *mut Obj) -> i64 {
    unsafe {
        if m.is_null() || (*m).header.type_tag != TypeTagKind::Match {
            return -1;
        }

        let match_obj = m as *const MatchObj;
        (*match_obj).start
    }
}
#[export_name = "rt_match_start"]
pub extern "C" fn rt_match_start_abi(m: Value) -> i64 {
    rt_match_start(m.unwrap_ptr())
}

/// Get end position of match
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_match_end(m: *mut Obj) -> i64 {
    unsafe {
        if m.is_null() || (*m).header.type_tag != TypeTagKind::Match {
            return -1;
        }

        let match_obj = m as *const MatchObj;
        (*match_obj).end
    }
}
#[export_name = "rt_match_end"]
pub extern "C" fn rt_match_end_abi(m: Value) -> i64 {
    rt_match_end(m.unwrap_ptr())
}

/// Get all groups as a tuple (excluding group 0)
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_match_groups(m: *mut Obj) -> *mut Obj {
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
#[export_name = "rt_match_groups"]
pub extern "C" fn rt_match_groups_abi(m: Value) -> Value {
    Value::from_ptr(rt_match_groups(m.unwrap_ptr()))
}

/// Get span (start, end) of match as a tuple of two integers
/// Returns a tuple (start, end) using raw int storage
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_match_span(m: *mut Obj) -> *mut Obj {
    unsafe {
        let (start, end) = if m.is_null() || (*m).header.type_tag != TypeTagKind::Match {
            (-1i64, -1i64)
        } else {
            let match_obj = m as *const MatchObj;
            ((*match_obj).start, (*match_obj).end)
        };

        // Create a 2-element tuple storing tagged int Values
        let tuple_size = std::mem::size_of::<TupleObj>() + 2 * std::mem::size_of::<*mut Obj>();
        let tuple_ptr = gc::gc_alloc(tuple_size, TypeTagKind::Tuple as u8) as *mut TupleObj;

        (*tuple_ptr).header = ObjHeader {
            type_tag: TypeTagKind::Tuple,
            marked: false,
            size: tuple_size,
        };
        (*tuple_ptr).len = 2;

        // Store start and end as tagged Value::from_int
        *(*tuple_ptr).data.as_mut_ptr().add(0) = pyaot_core_defs::Value::from_int(start);
        *(*tuple_ptr).data.as_mut_ptr().add(1) = pyaot_core_defs::Value::from_int(end);

        tuple_ptr as *mut Obj
    }
}
#[export_name = "rt_match_span"]
pub extern "C" fn rt_match_span_abi(m: Value) -> Value {
    Value::from_ptr(rt_match_span(m.unwrap_ptr()))
}
