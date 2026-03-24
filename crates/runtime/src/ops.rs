//! Runtime operations (arithmetic, comparisons, etc.)

/// Add two integers
#[no_mangle]
pub extern "C" fn rt_add_int(a: i64, b: i64) -> i64 {
    match a.checked_add(b) {
        Some(value) => value,
        None => unsafe {
            crate::exceptions::rt_exc_raise_overflow_error(
                b"integer overflow".as_ptr(),
                b"integer overflow".len(),
            )
        },
    }
}

/// Subtract two integers
#[no_mangle]
pub extern "C" fn rt_sub_int(a: i64, b: i64) -> i64 {
    match a.checked_sub(b) {
        Some(value) => value,
        None => unsafe {
            crate::exceptions::rt_exc_raise_overflow_error(
                b"integer overflow".as_ptr(),
                b"integer overflow".len(),
            )
        },
    }
}

/// Multiply two integers
#[no_mangle]
pub extern "C" fn rt_mul_int(a: i64, b: i64) -> i64 {
    match a.checked_mul(b) {
        Some(value) => value,
        None => unsafe {
            crate::exceptions::rt_exc_raise_overflow_error(
                b"integer overflow".as_ptr(),
                b"integer overflow".len(),
            )
        },
    }
}

/// Divide two integers (Python-style floor division)
#[no_mangle]
pub extern "C" fn rt_div_int(a: i64, b: i64) -> i64 {
    if b == 0 {
        unsafe {
            crate::exceptions::rt_exc_raise_zero_division_error(
                b"division by zero".as_ptr(),
                b"division by zero".len(),
            )
        }
    }
    // Python floor division: rounds toward negative infinity
    let d = a / b;
    let r = a % b;
    // Adjust when remainder has different sign than divisor
    if r != 0 && (r ^ b) < 0 {
        d - 1
    } else {
        d
    }
}

/// True division of two integers (Python 3 `/` operator)
/// Always returns float, even for integer operands
#[no_mangle]
pub extern "C" fn rt_true_div_int(a: i64, b: i64) -> f64 {
    if b == 0 {
        unsafe {
            crate::exceptions::rt_exc_raise_zero_division_error(
                b"division by zero".as_ptr(),
                b"division by zero".len(),
            )
        }
    }
    (a as f64) / (b as f64)
}

/// Modulo two integers
#[no_mangle]
pub extern "C" fn rt_mod_int(a: i64, b: i64) -> i64 {
    if b == 0 {
        unsafe {
            crate::exceptions::rt_exc_raise_zero_division_error(
                b"integer modulo by zero".as_ptr(),
                b"integer modulo by zero".len(),
            )
        }
    }
    // Python modulo: result has same sign as divisor
    let r = a % b;
    if r != 0 && (r ^ b) < 0 {
        r + b
    } else {
        r
    }
}

/// Add two floats
#[no_mangle]
pub extern "C" fn rt_add_float(a: f64, b: f64) -> f64 {
    a + b
}

/// Subtract two floats
#[no_mangle]
pub extern "C" fn rt_sub_float(a: f64, b: f64) -> f64 {
    a - b
}

/// Multiply two floats
#[no_mangle]
pub extern "C" fn rt_mul_float(a: f64, b: f64) -> f64 {
    a * b
}

/// Divide two floats
#[no_mangle]
pub extern "C" fn rt_div_float(a: f64, b: f64) -> f64 {
    a / b
}

/// Print an integer (legacy - with newline)
#[no_mangle]
pub extern "C" fn rt_print_int(value: i64) {
    println!("{}", value);
}

/// Print a float (legacy - with newline)
#[no_mangle]
pub extern "C" fn rt_print_float(value: f64) {
    println!("{}", crate::utils::format_float_python(value));
}

/// Print a boolean (legacy - with newline)
#[no_mangle]
pub extern "C" fn rt_print_bool(value: bool) {
    println!("{}", if value { "True" } else { "False" });
}

/// Print None (legacy - with newline)
#[no_mangle]
pub extern "C" fn rt_print_none() {
    println!("None");
}

// === New print functions for print() builtin ===

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

// === Runtime type dispatch operations for Union types ===

use crate::object::{
    BoolObj, DictObj, FloatObj, IntObj, ListObj, Obj, SetObj, StrObj, TupleObj, TypeTagKind,
    ELEM_RAW_BOOL, ELEM_RAW_INT,
};
use crate::print::{rt_print_bytes_obj, rt_print_str_obj};
use crate::{exceptions::rt_exc_raise, exceptions::ExceptionType};

/// Print a single element from a container, respecting elem_tag
/// For repr mode, strings get quotes
unsafe fn print_elem_repr(elem: *mut Obj, elem_tag: u8) {
    match elem_tag {
        ELEM_RAW_INT => {
            // Element is a raw i64 value stored as pointer
            print!("{}", elem as i64);
        }
        ELEM_RAW_BOOL => {
            // Element is a raw bool stored as pointer
            let val = elem as i64;
            print!("{}", if val != 0 { "True" } else { "False" });
        }
        _ => {
            // ELEM_HEAP_OBJ: element is a *mut Obj with valid header
            print_obj_repr(elem);
        }
    }
}

/// Print Python repr of any boxed object (strings get quotes)
unsafe fn print_obj_repr(obj: *mut Obj) {
    if obj.is_null() {
        print!("None");
        return;
    }
    match (*obj).type_tag() {
        TypeTagKind::Int => {
            let int_obj = obj as *mut IntObj;
            print!("{}", (*int_obj).value);
        }
        TypeTagKind::Float => {
            let float_obj = obj as *mut FloatObj;
            print!("{}", crate::utils::format_float_python((*float_obj).value));
        }
        TypeTagKind::Bool => {
            let bool_obj = obj as *mut BoolObj;
            print!("{}", if (*bool_obj).value { "True" } else { "False" });
        }
        TypeTagKind::Str => {
            // In repr mode, strings get single quotes
            let str_obj = obj as *mut StrObj;
            let len = (*str_obj).len;
            let data = (*str_obj).data.as_ptr();
            let bytes = std::slice::from_raw_parts(data, len);
            if let Ok(s) = std::str::from_utf8(bytes) {
                print!("'{}'", s);
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
    }
}

/// Print list repr: [elem, elem, ...]
unsafe fn print_list_repr(obj: *mut Obj) {
    let list = obj as *mut ListObj;
    let len = (*list).len;
    let data = (*list).data;
    let elem_tag = (*list).elem_tag;

    print!("[");
    for i in 0..len {
        if i > 0 {
            print!(", ");
        }
        let elem = *data.add(i);
        print_elem_repr(elem, elem_tag);
    }
    print!("]");
}

/// Print tuple repr: (elem, elem, ...) with trailing comma for single-element
unsafe fn print_tuple_repr(obj: *mut Obj) {
    let tuple = obj as *mut TupleObj;
    let len = (*tuple).len;
    let data = (*tuple).data.as_ptr();
    let elem_tag = (*tuple).elem_tag;

    print!("(");
    for i in 0..len {
        if i > 0 {
            print!(", ");
        }
        let elem = *data.add(i);
        print_elem_repr(elem, elem_tag);
    }
    if len == 1 {
        print!(",");
    }
    print!(")");
}

/// Print a value that may be a heap object or a raw primitive (for dict/set values)
unsafe fn print_maybe_raw_repr(ptr: *mut Obj) {
    if crate::utils::is_heap_obj(ptr) {
        print_obj_repr(ptr);
    } else {
        // Raw integer value stored as pointer
        print!("{}", ptr as i64);
    }
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
        if !key.is_null() {
            if !first {
                print!(", ");
            }
            first = false;
            print_obj_repr(key);
            print!(": ");
            print_maybe_raw_repr((*entry).value);
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
    const TOMBSTONE: *mut Obj = std::ptr::dangling_mut::<Obj>();

    if len == 0 {
        print!("set()");
        return;
    }

    print!("{{");
    let mut first = true;
    for i in 0..capacity {
        let entry = entries.add(i);
        let elem = (*entry).elem;
        if !elem.is_null() && elem != TOMBSTONE {
            if !first {
                print!(", ");
            }
            first = false;
            print_obj_repr(elem);
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
    unsafe {
        match (*obj).type_tag() {
            TypeTagKind::Int => {
                let int_obj = obj as *mut IntObj;
                print!("{}", (*int_obj).value);
            }
            TypeTagKind::Float => {
                let float_obj = obj as *mut FloatObj;
                print!("{}", crate::utils::format_float_python((*float_obj).value));
            }
            TypeTagKind::Bool => {
                let bool_obj = obj as *mut BoolObj;
                print!("{}", if (*bool_obj).value { "True" } else { "False" });
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
        }
    }
}

/// Compare two heap objects for equality with runtime type dispatch
/// Returns 1 if equal, 0 if not equal
/// Used for Union types where the actual type is determined at runtime
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_eq(a: *mut Obj, b: *mut Obj) -> i8 {
    // Handle null (None)
    if a.is_null() && b.is_null() {
        return 1;
    }
    if a.is_null() || b.is_null() {
        let non_null = if a.is_null() { b } else { a };
        unsafe {
            return if (*non_null).type_tag() == TypeTagKind::None {
                1
            } else {
                0
            };
        }
    }

    unsafe {
        let tag_a = (*a).type_tag();
        let tag_b = (*b).type_tag();

        // Different types → not equal
        if tag_a != tag_b {
            return 0;
        }

        match tag_a {
            TypeTagKind::Int => {
                let va = (*(a as *mut IntObj)).value;
                let vb = (*(b as *mut IntObj)).value;
                if va == vb {
                    1
                } else {
                    0
                }
            }
            TypeTagKind::Float => {
                let va = (*(a as *mut FloatObj)).value;
                let vb = (*(b as *mut FloatObj)).value;
                if va == vb {
                    1
                } else {
                    0
                }
            }
            TypeTagKind::Bool => {
                let va = (*(a as *mut BoolObj)).value;
                let vb = (*(b as *mut BoolObj)).value;
                if va == vb {
                    1
                } else {
                    0
                }
            }
            TypeTagKind::Str => crate::string::rt_str_eq(a, b),
            TypeTagKind::Bytes => crate::bytes::rt_bytes_eq(a, b),
            TypeTagKind::None => 1,
            // For containers and other types, use identity comparison
            _ => {
                if a == b {
                    1
                } else {
                    0
                }
            }
        }
    }
}

/// Helper function to compare two orderable heap objects
/// Returns Ordering or panics with TypeError for incompatible types
unsafe fn obj_cmp_ordering(a: *mut Obj, b: *mut Obj) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    // Handle null (None) - None is not orderable
    if a.is_null() || b.is_null() {
        let msg = b"'<' not supported between instances of 'NoneType' and other types";
        rt_exc_raise(ExceptionType::TypeError as u8, msg.as_ptr(), msg.len());
    }

    let tag_a = (*a).type_tag();
    let tag_b = (*b).type_tag();

    // Check for None type tag
    if tag_a == TypeTagKind::None || tag_b == TypeTagKind::None {
        let msg = b"'<' not supported between instances of 'NoneType' and other types";
        rt_exc_raise(ExceptionType::TypeError as u8, msg.as_ptr(), msg.len());
    }

    // Same type comparisons
    if tag_a == tag_b {
        return match tag_a {
            TypeTagKind::Int => {
                let va = (*(a as *mut IntObj)).value;
                let vb = (*(b as *mut IntObj)).value;
                va.cmp(&vb)
            }
            TypeTagKind::Float => {
                let va = (*(a as *mut FloatObj)).value;
                let vb = (*(b as *mut FloatObj)).value;
                va.partial_cmp(&vb).unwrap_or(Ordering::Equal)
            }
            TypeTagKind::Bool => {
                let va = (*(a as *mut BoolObj)).value;
                let vb = (*(b as *mut BoolObj)).value;
                va.cmp(&vb)
            }
            TypeTagKind::Str => {
                let str_a = a as *mut StrObj;
                let str_b = b as *mut StrObj;
                let len_a = (*str_a).len;
                let len_b = (*str_b).len;
                let data_a = std::slice::from_raw_parts((*str_a).data.as_ptr(), len_a);
                let data_b = std::slice::from_raw_parts((*str_b).data.as_ptr(), len_b);
                data_a.cmp(data_b)
            }
            _ => {
                let msg = format!(
                    "'<' not supported between instances of '{}' and '{}'",
                    type_name(tag_a),
                    type_name(tag_b)
                );
                rt_exc_raise(ExceptionType::TypeError as u8, msg.as_ptr(), msg.len());
            }
        };
    }

    // Mixed int/float - promote int to float
    if (tag_a == TypeTagKind::Int && tag_b == TypeTagKind::Float)
        || (tag_a == TypeTagKind::Float && tag_b == TypeTagKind::Int)
    {
        let va = if tag_a == TypeTagKind::Int {
            (*(a as *mut IntObj)).value as f64
        } else {
            (*(a as *mut FloatObj)).value
        };
        let vb = if tag_b == TypeTagKind::Int {
            (*(b as *mut IntObj)).value as f64
        } else {
            (*(b as *mut FloatObj)).value
        };
        return va.partial_cmp(&vb).unwrap_or(Ordering::Equal);
    }

    // Incompatible types
    let msg = format!(
        "'<' not supported between instances of '{}' and '{}'",
        type_name(tag_a),
        type_name(tag_b)
    );
    rt_exc_raise(ExceptionType::TypeError as u8, msg.as_ptr(), msg.len());
}

/// Helper to get type name for error messages.
/// Delegates to TypeTagKind::type_name() from core-defs (single source of truth).
#[inline]
fn type_name(tag: TypeTagKind) -> &'static str {
    tag.type_name()
}

/// Compare two heap objects for less-than with runtime type dispatch
/// Returns 1 if a < b, 0 otherwise
/// Used for Union types where the actual type is determined at runtime
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_lt(a: *mut Obj, b: *mut Obj) -> i8 {
    unsafe {
        if obj_cmp_ordering(a, b) == std::cmp::Ordering::Less {
            1
        } else {
            0
        }
    }
}

/// Compare two heap objects for less-than-or-equal with runtime type dispatch
/// Returns 1 if a <= b, 0 otherwise
/// Used for Union types where the actual type is determined at runtime
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_lte(a: *mut Obj, b: *mut Obj) -> i8 {
    unsafe {
        let ord = obj_cmp_ordering(a, b);
        if ord == std::cmp::Ordering::Less || ord == std::cmp::Ordering::Equal {
            1
        } else {
            0
        }
    }
}

/// Compare two heap objects for greater-than with runtime type dispatch
/// Returns 1 if a > b, 0 otherwise
/// Used for Union types where the actual type is determined at runtime
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_gt(a: *mut Obj, b: *mut Obj) -> i8 {
    unsafe {
        if obj_cmp_ordering(a, b) == std::cmp::Ordering::Greater {
            1
        } else {
            0
        }
    }
}

/// Compare two heap objects for greater-than-or-equal with runtime type dispatch
/// Returns 1 if a >= b, 0 otherwise
/// Used for Union types where the actual type is determined at runtime
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_gte(a: *mut Obj, b: *mut Obj) -> i8 {
    unsafe {
        let ord = obj_cmp_ordering(a, b);
        if ord == std::cmp::Ordering::Greater || ord == std::cmp::Ordering::Equal {
            1
        } else {
            0
        }
    }
}

/// Check if element is in container with runtime type dispatch
/// Returns 1 if element is in container, 0 otherwise
/// Used for Union container types where the actual type is determined at runtime
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_contains(container: *mut Obj, elem: *mut Obj) -> i8 {
    if container.is_null() {
        panic!("TypeError: argument of type 'NoneType' is not iterable");
    }

    unsafe {
        match (*container).type_tag() {
            TypeTagKind::Dict => crate::dict::rt_dict_contains(container, elem),
            TypeTagKind::Set => crate::set::rt_set_contains(container, elem),
            TypeTagKind::List => {
                // Use linear search with value equality
                rt_list_contains_value(container, elem)
            }
            TypeTagKind::Str => crate::string::rt_str_contains(elem, container),
            TypeTagKind::Tuple => {
                // Use linear search with value equality
                rt_tuple_contains_value(container, elem)
            }
            TypeTagKind::Bytes => {
                // Check if integer is in bytes
                rt_bytes_contains_value(container, elem)
            }
            _ => panic!(
                "TypeError: argument of type '{}' is not iterable",
                type_name((*container).type_tag())
            ),
        }
    }
}

/// Check if list contains value using value equality (not pointer equality)
unsafe fn rt_list_contains_value(list: *mut Obj, value: *mut Obj) -> i8 {
    use crate::object::ListObj;

    let list_obj = list as *mut ListObj;
    let len = (*list_obj).len;
    let data = (*list_obj).data;

    if data.is_null() {
        return 0;
    }

    for i in 0..len {
        let elem = *data.add(i);
        if rt_obj_eq(elem, value) == 1 {
            return 1;
        }
    }

    0
}

/// Check if tuple contains value using value equality
unsafe fn rt_tuple_contains_value(tuple: *mut Obj, value: *mut Obj) -> i8 {
    use crate::object::{BoolObj, IntObj, TupleObj, ELEM_RAW_BOOL, ELEM_RAW_INT};

    let tuple_obj = tuple as *mut TupleObj;
    let len = (*tuple_obj).len;
    let data = (*tuple_obj).data.as_ptr();
    let elem_tag = (*tuple_obj).elem_tag;

    match elem_tag {
        ELEM_RAW_INT => {
            // Elements are raw i64 values — unbox the search value to compare
            if value.is_null() {
                return 0;
            }
            let search_val = match (*value).header.type_tag {
                TypeTagKind::Int => (*(value as *mut IntObj)).value,
                TypeTagKind::Bool => (*(value as *mut BoolObj)).value as i8 as i64,
                _ => return 0,
            };
            for i in 0..len {
                let elem_raw = *data.add(i) as i64;
                if elem_raw == search_val {
                    return 1;
                }
            }
            0
        }
        ELEM_RAW_BOOL => {
            // Elements are raw i8 values cast to pointer
            if value.is_null() {
                return 0;
            }
            let search_val: i8 = match (*value).header.type_tag {
                TypeTagKind::Bool => (*(value as *mut BoolObj)).value as i8,
                TypeTagKind::Int => {
                    let v = (*(value as *mut IntObj)).value;
                    if v == 0 {
                        0
                    } else {
                        1
                    }
                }
                _ => return 0,
            };
            for i in 0..len {
                let elem_raw = *data.add(i) as i8;
                if elem_raw == search_val {
                    return 1;
                }
            }
            0
        }
        _ => {
            // Elements are *mut Obj pointers — use value equality
            for i in 0..len {
                let elem = *data.add(i);
                if rt_obj_eq(elem, value) == 1 {
                    return 1;
                }
            }
            0
        }
    }
}

/// Check truthiness of any value with runtime type dispatch
/// Returns 1 if truthy, 0 if falsy
/// Falsy values: None, False, 0, 0.0, empty str/list/tuple/dict/set/bytes
/// Used for filter(None, iterable) to filter out falsy values
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_is_truthy(obj: *mut Obj) -> i8 {
    // None is falsy
    if obj.is_null() {
        return 0;
    }

    unsafe {
        match (*obj).type_tag() {
            TypeTagKind::None => 0,
            TypeTagKind::Bool => {
                let bool_obj = obj as *mut BoolObj;
                if (*bool_obj).value {
                    1
                } else {
                    0
                }
            }
            TypeTagKind::Int => {
                let int_obj = obj as *mut IntObj;
                if (*int_obj).value != 0 {
                    1
                } else {
                    0
                }
            }
            TypeTagKind::Float => {
                let float_obj = obj as *mut FloatObj;
                if (*float_obj).value != 0.0 {
                    1
                } else {
                    0
                }
            }
            TypeTagKind::Str => {
                let str_obj = obj as *mut StrObj;
                if (*str_obj).len > 0 {
                    1
                } else {
                    0
                }
            }
            TypeTagKind::List => {
                let list_obj = obj as *mut ListObj;
                if (*list_obj).len > 0 {
                    1
                } else {
                    0
                }
            }
            TypeTagKind::Tuple => {
                let tuple_obj = obj as *mut TupleObj;
                if (*tuple_obj).len > 0 {
                    1
                } else {
                    0
                }
            }
            TypeTagKind::Dict => {
                let dict_obj = obj as *mut DictObj;
                if (*dict_obj).len > 0 {
                    1
                } else {
                    0
                }
            }
            TypeTagKind::Set => {
                let set_obj = obj as *mut SetObj;
                if (*set_obj).len > 0 {
                    1
                } else {
                    0
                }
            }
            TypeTagKind::Bytes => {
                use crate::object::BytesObj;
                let bytes_obj = obj as *mut BytesObj;
                if (*bytes_obj).len > 0 {
                    1
                } else {
                    0
                }
            }
            // All other types (Instance, Iterator, Cell, Generator, Match, File) are truthy
            _ => 1,
        }
    }
}

/// Check if bytes contains an integer value
unsafe fn rt_bytes_contains_value(bytes: *mut Obj, value: *mut Obj) -> i8 {
    use crate::object::{BytesObj, IntObj};

    // value should be an integer
    if value.is_null() || (*value).type_tag() != TypeTagKind::Int {
        panic!(
            "TypeError: a bytes-like object is required, not '{}'",
            if value.is_null() {
                "NoneType"
            } else {
                type_name((*value).type_tag())
            }
        );
    }

    let int_val = (*(value as *mut IntObj)).value;
    if !(0..=255).contains(&int_val) {
        return 0; // Not a valid byte value
    }
    let byte_to_find = int_val as u8;

    let bytes_obj = bytes as *mut BytesObj;
    let len = (*bytes_obj).len;
    let data = (*bytes_obj).data.as_ptr();

    for i in 0..len {
        if *data.add(i) == byte_to_find {
            return 1;
        }
    }

    0
}
