//! Print operations for Python runtime

use crate::object::{DictObj, FloatObj, ListObj, Obj, SetObj, StrObj, TupleObj, TypeTagKind};
use crate::print::{rt_print_bytes_obj, rt_print_str_obj};
use pyaot_core_defs::Value;

// === Print functions for print() builtin ===

/// Print an integer value (no newline)
#[no_mangle]
pub extern "C" fn rt_print_int_value(value: i64) {
    print!("{}", value);
}

/// Print a float value (no newline)
#[no_mangle]
pub extern "C" fn rt_print_float_value(value: f64) {
    print!("{}", crate::utils::format_float_python(value));
}

/// Print a boolean value (no newline)
#[no_mangle]
pub extern "C" fn rt_print_bool_value(value: bool) {
    print!("{}", if value { "True" } else { "False" });
}

/// Print None value (no newline)
#[no_mangle]
pub extern "C" fn rt_print_none_value() {
    print!("None");
}

/// Print a C string (no newline)
/// str_ptr is a pointer to a null-terminated C string
///
/// # Safety
/// `str_ptr` must be null or a valid pointer to a null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn rt_print_str_value(str_ptr: *const i8) {
    if !str_ptr.is_null() {
        let c_str = std::ffi::CStr::from_ptr(str_ptr);
        print!("{}", c_str.to_string_lossy());
    }
}

/// Print newline and flush stdout
#[no_mangle]
pub extern "C" fn rt_print_newline() {
    println!();
}

/// Print default separator (space)
#[no_mangle]
pub extern "C" fn rt_print_sep() {
    print!(" ");
}

/// Flush stdout (useful after print without newline)
#[no_mangle]
pub extern "C" fn rt_flush_stdout() {
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

// === Runtime type dispatch printing operations for Union types ===

/// Print a single element from a container, dispatching on Value tag
unsafe fn print_elem_repr(elem: *mut Obj) {
    print_obj_repr(elem);
}

/// Print Python repr of any boxed object (strings get quotes).
///
/// NOTE: This function intentionally duplicates much of `rt_print_obj` below.
/// The key difference is that `print_obj_repr` renders strings in repr mode
/// (with single quotes and escape sequences), while `rt_print_obj` renders
/// strings in str mode (raw content, no quotes). All other type arms are
/// identical. We keep them separate rather than adding a `repr: bool` parameter
/// to avoid branching overhead on a hot path that is called recursively for
/// every element in nested containers.
unsafe fn print_obj_repr(obj: *mut Obj) {
    if obj.is_null() {
        print!("None");
        return;
    }
    // Check Value-tagged primitives before heap pointer dereference.
    let v = Value(obj as u64);
    if v.is_int() {
        print!("{}", v.unwrap_int());
        return;
    }
    if v.is_bool() {
        print!("{}", if v.unwrap_bool() { "True" } else { "False" });
        return;
    }
    if v.is_none() {
        print!("None");
        return;
    }
    match (*obj).type_tag() {
        // Int/Bool handled above via Value tag checks; these arms are unreachable
        // for correctly-tagged values but required for exhaustiveness.
        TypeTagKind::Int => print!("{}", Value(obj as u64).unwrap_int()),
        TypeTagKind::Bool => {
            print!(
                "{}",
                if Value(obj as u64).unwrap_bool() {
                    "True"
                } else {
                    "False"
                }
            )
        }
        TypeTagKind::Float => {
            let float_obj = obj as *mut FloatObj;
            print!("{}", crate::utils::format_float_python((*float_obj).value));
        }
        TypeTagKind::Str => {
            // In repr mode, strings get single quotes with proper escaping
            let str_obj = obj as *mut StrObj;
            let len = (*str_obj).len;
            let data = (*str_obj).data.as_ptr();
            let bytes = std::slice::from_raw_parts(data, len);
            if let Ok(text) = std::str::from_utf8(bytes) {
                let mut s = String::with_capacity(len + 2);
                s.push('\'');
                crate::conversions::repr_escape_into(&mut s, text);
                s.push('\'');
                print!("{}", s);
            }
        }
        TypeTagKind::None => print!("None"),
        TypeTagKind::List => print_list_repr(obj),
        TypeTagKind::Tuple => print_tuple_repr(obj),
        TypeTagKind::Dict => print_dict_repr(obj),
        TypeTagKind::Set => print_set_repr(obj),
        TypeTagKind::Bytes => rt_print_bytes_obj(obj),
        TypeTagKind::Instance => print!("<object at {:p}>", obj),
        TypeTagKind::Iterator => print!("<iterator>"),
        TypeTagKind::Cell => print!("<cell>"),
        // For these types, use type_name() from core-defs (single source of truth)
        TypeTagKind::Generator => print!(
            "<{} object at {:p}>",
            TypeTagKind::Generator.type_name(),
            obj
        ),
        TypeTagKind::Match => print!("<{} object at {:p}>", TypeTagKind::Match.type_name(), obj),
        TypeTagKind::File => print!("<{} at {:p}>", TypeTagKind::File.type_name(), obj),
        TypeTagKind::StringBuilder => {
            print!("<{} at {:p}>", TypeTagKind::StringBuilder.type_name(), obj)
        }
        TypeTagKind::StructTime => print!("<{} at {:p}>", TypeTagKind::StructTime.type_name(), obj),
        TypeTagKind::CompletedProcess => print!(
            "<{} at {:p}>",
            TypeTagKind::CompletedProcess.type_name(),
            obj
        ),
        TypeTagKind::ParseResult => {
            print!("<{} at {:p}>", TypeTagKind::ParseResult.type_name(), obj)
        }
        TypeTagKind::HttpResponse => {
            print!("<{} at {:p}>", TypeTagKind::HttpResponse.type_name(), obj)
        }
        TypeTagKind::Hash => print!("<{} object>", TypeTagKind::Hash.type_name()),
        TypeTagKind::StringIO => print!("<_io.StringIO object>"),
        TypeTagKind::BytesIO => print!("<_io.BytesIO object>"),
        TypeTagKind::DefaultDict => print_dict_repr(obj), // Same repr as dict
        TypeTagKind::Counter => print_dict_repr(obj),     // Same repr as dict
        TypeTagKind::Deque => print!("<deque at {:p}>", obj),
        TypeTagKind::Request => print!("<{} at {:p}>", TypeTagKind::Request.type_name(), obj),
        TypeTagKind::NotImplemented => print!("NotImplemented"),
    }
}

/// Print list repr: [elem, elem, ...]
unsafe fn print_list_repr(obj: *mut Obj) {
    let list = obj as *mut ListObj;
    let len = (*list).len;
    let data = (*list).data;

    print!("[");
    for i in 0..len {
        if i > 0 {
            print!(", ");
        }
        let elem = (*data.add(i)).0 as *mut Obj;
        print_elem_repr(elem);
    }
    print!("]");
}

/// Print tuple repr: (elem, elem, ...) with trailing comma for single-element
unsafe fn print_tuple_repr(obj: *mut Obj) {
    let tuple = obj as *mut TupleObj;
    let len = (*tuple).len;
    let data = (*tuple).data.as_ptr();

    print!("(");
    for i in 0..len {
        if i > 0 {
            print!(", ");
        }
        let elem = (*data.add(i)).0 as *mut Obj;
        print_elem_repr(elem);
    }
    if len == 1 {
        print!(",");
    }
    print!(")");
}

/// Print a value that may be a heap object or a Value-tagged primitive.
unsafe fn print_maybe_raw_repr(ptr: *mut Obj) {
    print_obj_repr(ptr);
}

/// Print dict repr: {key: value, ...}
/// Dict keys are always boxed, but values may be raw primitives
unsafe fn print_dict_repr(obj: *mut Obj) {
    let dict = obj as *mut DictObj;
    let entries_len = (*dict).entries_len;
    let entries = (*dict).entries;

    print!("{{");
    let mut first = true;
    for i in 0..entries_len {
        let entry = entries.add(i);
        let key = (*entry).key;
        if key.0 != 0 {
            if !first {
                print!(", ");
            }
            first = false;
            print_obj_repr(key.0 as *mut Obj);
            print!(": ");
            print_maybe_raw_repr((*entry).value.0 as *mut Obj);
        }
    }
    print!("}}");
}

/// Print set repr: {elem, ...} or set() for empty
/// Set elements are always boxed heap objects
unsafe fn print_set_repr(obj: *mut Obj) {
    let set = obj as *mut SetObj;
    let len = (*set).len;
    let capacity = (*set).capacity;
    let entries = (*set).entries;
    use crate::object::TOMBSTONE;

    if len == 0 {
        print!("set()");
        return;
    }

    print!("{{");
    let mut first = true;
    for i in 0..capacity {
        let entry = entries.add(i);
        let elem = (*entry).elem;
        if elem.0 != 0 && elem != TOMBSTONE {
            if !first {
                print!(", ");
            }
            first = false;
            print_obj_repr(elem.0 as *mut Obj);
        }
    }
    print!("}}");
}

/// Print any heap object with runtime type dispatch
/// Used for Union types where the actual type is determined at runtime
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_print_obj(obj: *mut Obj) {
    if obj.is_null() {
        print!("None");
        return;
    }
    // Check Value-tagged primitives before heap pointer dereference.
    let v = Value(obj as u64);
    if v.is_int() {
        print!("{}", v.unwrap_int());
        return;
    }
    if v.is_bool() {
        print!("{}", if v.unwrap_bool() { "True" } else { "False" });
        return;
    }
    if v.is_none() {
        print!("None");
        return;
    }
    unsafe {
        match (*obj).type_tag() {
            // Int/Bool handled above via Value tag checks; arms required for exhaustiveness.
            TypeTagKind::Int => print!("{}", Value(obj as u64).unwrap_int()),
            TypeTagKind::Bool => {
                print!(
                    "{}",
                    if Value(obj as u64).unwrap_bool() {
                        "True"
                    } else {
                        "False"
                    }
                )
            }
            TypeTagKind::Float => {
                let float_obj = obj as *mut FloatObj;
                print!("{}", crate::utils::format_float_python((*float_obj).value));
            }
            TypeTagKind::Str => rt_print_str_obj(obj),
            TypeTagKind::Bytes => rt_print_bytes_obj(obj),
            TypeTagKind::None => print!("None"),
            TypeTagKind::List => print_list_repr(obj),
            TypeTagKind::Tuple => print_tuple_repr(obj),
            TypeTagKind::Dict => print_dict_repr(obj),
            TypeTagKind::Set => print_set_repr(obj),
            TypeTagKind::Instance => print!("<object at {:p}>", obj),
            TypeTagKind::Iterator => print!("<iterator>"),
            TypeTagKind::Cell => print!("<cell>"),
            // For these types, use type_name() from core-defs (single source of truth)
            TypeTagKind::Generator => print!(
                "<{} object at {:p}>",
                TypeTagKind::Generator.type_name(),
                obj
            ),
            TypeTagKind::Match => {
                print!("<{} object at {:p}>", TypeTagKind::Match.type_name(), obj)
            }
            TypeTagKind::File => print!("<{} at {:p}>", TypeTagKind::File.type_name(), obj),
            TypeTagKind::StringBuilder => {
                print!("<{} at {:p}>", TypeTagKind::StringBuilder.type_name(), obj)
            }
            TypeTagKind::StructTime => {
                print!("<{} at {:p}>", TypeTagKind::StructTime.type_name(), obj)
            }
            TypeTagKind::CompletedProcess => print!(
                "<{} at {:p}>",
                TypeTagKind::CompletedProcess.type_name(),
                obj
            ),
            TypeTagKind::ParseResult => {
                print!("<{} at {:p}>", TypeTagKind::ParseResult.type_name(), obj)
            }
            TypeTagKind::HttpResponse => {
                print!("<{} at {:p}>", TypeTagKind::HttpResponse.type_name(), obj)
            }
            TypeTagKind::Hash => print!("<{} object>", TypeTagKind::Hash.type_name()),
            TypeTagKind::StringIO => print!("<_io.StringIO object>"),
            TypeTagKind::BytesIO => print!("<_io.BytesIO object>"),
            TypeTagKind::DefaultDict => print_dict_repr(obj),
            TypeTagKind::Counter => print_dict_repr(obj),
            TypeTagKind::Deque => print!("<deque at {:p}>", obj),
            TypeTagKind::Request => {
                print!("<{} at {:p}>", TypeTagKind::Request.type_name(), obj)
            }
            TypeTagKind::NotImplemented => print!("NotImplemented"),
        }
    }
}
