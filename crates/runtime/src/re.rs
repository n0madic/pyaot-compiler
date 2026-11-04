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

/// One piece of a parsed `re.sub` replacement template.
enum TemplatePart {
    Literal(String),
    Group(usize),
}

/// `re.error` for a bad replacement template (raised as `ValueError`, the
/// base class of CPython's `re.PatternError`).
unsafe fn template_error(msg: String) -> ! {
    crate::raise_exc!(crate::exceptions::ExceptionType::ValueError, "{}", msg)
}

/// Parse a Python replacement template (CPython `sre_parse.parse_template`):
/// `\n \t \r \f \v \a \b \\` escapes, octal escapes (`\0`, `\012`),
/// numeric group references (`\1`..`\99`), `\g<name>` / `\g<N>`; an unknown
/// ASCII-letter escape is an error; any other escaped character keeps its
/// backslash (`\.` stays `\.`).
unsafe fn parse_template(repl: &str, re: &Regex) -> Vec<TemplatePart> {
    let chars: Vec<char> = repl.chars().collect();
    let ngroups = re.captures_len();
    let mut parts: Vec<TemplatePart> = Vec::new();
    let mut lit = String::new();
    let flush = |lit: &mut String, parts: &mut Vec<TemplatePart>| {
        if !lit.is_empty() {
            parts.push(TemplatePart::Literal(std::mem::take(lit)));
        }
    };
    let group_ref = |g: usize, pos: usize| -> usize {
        if g >= ngroups {
            template_error(format!("invalid group reference {} at position {}", g, pos))
        }
        g
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '\\' {
            lit.push(c);
            i += 1;
            continue;
        }
        let pos = i;
        i += 1;
        let Some(&e) = chars.get(i) else {
            template_error(format!("bad escape (end of pattern) at position {}", pos))
        };
        i += 1;
        match e {
            'g' => {
                if chars.get(i) != Some(&'<') {
                    template_error(format!("missing < at position {}", i))
                }
                let start = i + 1;
                let Some(len) = chars[start..].iter().position(|&ch| ch == '>') else {
                    template_error(format!(
                        "missing >, unterminated name at position {}",
                        start
                    ))
                };
                let name: String = chars[start..start + len].iter().collect();
                i = start + len + 1;
                let g = if !name.is_empty() && name.chars().all(|ch| ch.is_ascii_digit()) {
                    group_ref(name.parse().unwrap_or(usize::MAX), start)
                } else {
                    match re.capture_names().position(|n| n == Some(name.as_str())) {
                        Some(g) => g,
                        None if name.is_empty() => {
                            template_error(format!("missing group name at position {}", start))
                        }
                        None => template_error(format!("unknown group name '{}'", name)),
                    }
                };
                flush(&mut lit, &mut parts);
                parts.push(TemplatePart::Group(g));
            }
            '0' => {
                // Octal escape: up to two more octal digits.
                let mut v = 0u32;
                let mut n = 0;
                while n < 2 && i < chars.len() && ('0'..='7').contains(&chars[i]) {
                    v = v * 8 + chars[i].to_digit(8).unwrap();
                    i += 1;
                    n += 1;
                }
                lit.push(char::from_u32(v).unwrap_or('\0'));
            }
            '1'..='9' => {
                // Two-digit group reference, or a three-digit octal escape.
                let d1 = e.to_digit(10).unwrap() as usize;
                if i < chars.len() && chars[i].is_ascii_digit() {
                    let d2 = chars[i].to_digit(10).unwrap() as usize;
                    if e <= '7'
                        && ('0'..='7').contains(&chars[i])
                        && i + 1 < chars.len()
                        && ('0'..='7').contains(&chars[i + 1])
                    {
                        let v =
                            (d1 * 64 + d2 * 8 + chars[i + 1].to_digit(8).unwrap() as usize) as u32;
                        if v > 0o377 {
                            template_error(format!(
                                "octal escape value \\{}{}{} outside of range 0-0o377 at position {}",
                                e, chars[i], chars[i + 1], pos
                            ))
                        }
                        lit.push(char::from_u32(v).unwrap());
                        i += 2;
                        continue;
                    }
                    i += 1;
                    flush(&mut lit, &mut parts);
                    parts.push(TemplatePart::Group(group_ref(d1 * 10 + d2, pos + 1)));
                } else {
                    flush(&mut lit, &mut parts);
                    parts.push(TemplatePart::Group(group_ref(d1, pos + 1)));
                }
            }
            'n' => lit.push('\n'),
            't' => lit.push('\t'),
            'r' => lit.push('\r'),
            'f' => lit.push('\x0c'),
            'v' => lit.push('\x0b'),
            'a' => lit.push('\x07'),
            'b' => lit.push('\x08'),
            '\\' => lit.push('\\'),
            ch if ch.is_ascii_alphabetic() => {
                template_error(format!("bad escape \\{} at position {}", ch, pos))
            }
            ch => {
                lit.push('\\');
                lit.push(ch);
            }
        }
    }
    flush(&mut lit, &mut parts);
    parts
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

        // Python template semantics (sre `parse_template`): escapes, `\N`
        // and `\g<name>` / `\g<N>` group references; `$` is literal.
        let template = parse_template(&repl_str, &re);
        let mut result = String::with_capacity(string_str.len());
        let mut last = 0;
        for caps in re.captures_iter(&string_str) {
            let m = caps.get(0).expect("group 0 always participates");
            result.push_str(&string_str[last..m.start()]);
            for part in &template {
                match part {
                    TemplatePart::Literal(t) => result.push_str(t),
                    TemplatePart::Group(g) => {
                        if let Some(gm) = caps.get(*g) {
                            result.push_str(gm.as_str());
                        }
                    }
                }
            }
            last = m.end();
        }
        result.push_str(&string_str[last..]);

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
/// Returns string for that group, or None if group didn't participate;
/// `IndexError: no such group` for an index outside the pattern's groups.
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
            crate::raise_exc!(
                crate::exceptions::ExceptionType::IndexError,
                "no such group"
            );
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
