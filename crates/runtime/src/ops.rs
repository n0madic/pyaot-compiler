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
    if a == i64::MIN && b == -1 {
        unsafe {
            crate::exceptions::rt_exc_raise_overflow_error(
                b"integer overflow".as_ptr(),
                b"integer overflow".len(),
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
    if a == i64::MIN && b == -1 {
        unsafe {
            crate::exceptions::rt_exc_raise_overflow_error(
                b"integer overflow".as_ptr(),
                b"integer overflow".len(),
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
            TypeTagKind::DefaultDict => print_dict_repr(obj),
            TypeTagKind::Counter => print_dict_repr(obj),
            TypeTagKind::Deque => print!("<deque at {:p}>", obj),
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

        // Int/Bool cross-type equality (Python: 1 == True, 0 == False)
        if (tag_a == TypeTagKind::Int && tag_b == TypeTagKind::Bool)
            || (tag_a == TypeTagKind::Bool && tag_b == TypeTagKind::Int)
        {
            let va = if tag_a == TypeTagKind::Int {
                (*(a as *mut IntObj)).value
            } else {
                (*(a as *mut BoolObj)).value as i64
            };
            let vb = if tag_b == TypeTagKind::Int {
                (*(b as *mut IntObj)).value
            } else {
                (*(b as *mut BoolObj)).value as i64
            };
            return if va == vb { 1 } else { 0 };
        }

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
                // NaN sorts to the end (Greater) to provide a stable ordering for sorted()
                va.partial_cmp(&vb).unwrap_or(Ordering::Greater)
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
                crate::raise_exc!(
                    ExceptionType::TypeError,
                    "'<' not supported between instances of '{}' and '{}'",
                    type_name(tag_a),
                    type_name(tag_b)
                );
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
        // NaN sorts to the end (Greater) to provide a stable ordering for sorted()
        return va.partial_cmp(&vb).unwrap_or(Ordering::Greater);
    }

    // Incompatible types
    crate::raise_exc!(
        ExceptionType::TypeError,
        "'<' not supported between instances of '{}' and '{}'",
        type_name(tag_a),
        type_name(tag_b)
    );
}

/// Helper to get type name for error messages.
/// Delegates to TypeTagKind::type_name() from core-defs (single source of truth).
#[inline]
fn type_name(tag: TypeTagKind) -> &'static str {
    tag.type_name()
}

/// Check whether any float value involved in a comparison is NaN.
/// Returns true if a or b is a Float NaN, or if the mixed int/float case
/// produces a NaN operand (only possible when a Float is NaN).
/// Python semantics: all ordering comparisons involving NaN return False.
unsafe fn involves_nan(a: *mut Obj, b: *mut Obj) -> bool {
    if a.is_null() || b.is_null() {
        return false;
    }
    let tag_a = (*a).type_tag();
    let tag_b = (*b).type_tag();

    if tag_a == TypeTagKind::Float {
        let va = (*(a as *mut FloatObj)).value;
        if va.is_nan() {
            return true;
        }
    }
    if tag_b == TypeTagKind::Float {
        let vb = (*(b as *mut FloatObj)).value;
        if vb.is_nan() {
            return true;
        }
    }
    false
}

/// Compare two heap objects for less-than with runtime type dispatch
/// Returns 1 if a < b, 0 otherwise
/// Used for Union types where the actual type is determined at runtime
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_lt(a: *mut Obj, b: *mut Obj) -> i8 {
    unsafe {
        // NaN comparisons always return False in Python
        if involves_nan(a, b) {
            return 0;
        }
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
        // NaN comparisons always return False in Python
        if involves_nan(a, b) {
            return 0;
        }
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
        // NaN comparisons always return False in Python
        if involves_nan(a, b) {
            return 0;
        }
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
        // NaN comparisons always return False in Python
        if involves_nan(a, b) {
            return 0;
        }
        let ord = obj_cmp_ordering(a, b);
        if ord == std::cmp::Ordering::Greater || ord == std::cmp::Ordering::Equal {
            1
        } else {
            0
        }
    }
}

// ==================== Union Arithmetic Operations ====================
// Runtime dispatch for arithmetic on boxed Union values.
// Returns a boxed result (*mut Obj).

/// Extract numeric values from two boxed objects, promoting int→float if mixed.
/// Returns (left_f64, right_f64, both_int, left_int, right_int)
#[inline]
unsafe fn extract_numeric_pair(a: *mut Obj, b: *mut Obj) -> (f64, f64, bool, i64, i64) {
    let tag_a = (*a).type_tag();
    let tag_b = (*b).type_tag();
    let va_int = if tag_a == TypeTagKind::Int {
        (*(a as *mut IntObj)).value
    } else {
        0
    };
    let vb_int = if tag_b == TypeTagKind::Int {
        (*(b as *mut IntObj)).value
    } else {
        0
    };
    let va_f = if tag_a == TypeTagKind::Float {
        (*(a as *mut FloatObj)).value
    } else {
        va_int as f64
    };
    let vb_f = if tag_b == TypeTagKind::Float {
        (*(b as *mut FloatObj)).value
    } else {
        vb_int as f64
    };
    let both_int = tag_a == TypeTagKind::Int && tag_b == TypeTagKind::Int;
    (va_f, vb_f, both_int, va_int, vb_int)
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_add(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let tag_a = (*a).type_tag();
        let tag_b = (*b).type_tag();
        // String concatenation
        if tag_a == TypeTagKind::Str && tag_b == TypeTagKind::Str {
            return crate::string::rt_str_concat(a, b);
        }
        let (va, vb, both_int, vai, vbi) = extract_numeric_pair(a, b);
        if both_int {
            match vai.checked_add(vbi) {
                Some(v) => crate::boxing::rt_box_int(v),
                None => {
                    let msg = b"integer overflow";
                    rt_exc_raise(ExceptionType::OverflowError as u8, msg.as_ptr(), msg.len());
                }
            }
        } else if (tag_a == TypeTagKind::Int || tag_a == TypeTagKind::Float)
            && (tag_b == TypeTagKind::Int || tag_b == TypeTagKind::Float)
        {
            crate::boxing::rt_box_float(va + vb)
        } else {
            crate::raise_exc!(
                ExceptionType::TypeError,
                "unsupported operand type(s) for +: '{}' and '{}'",
                type_name(tag_a),
                type_name(tag_b)
            );
        }
    }
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_sub(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let (va, vb, both_int, vai, vbi) = extract_numeric_pair(a, b);
        if both_int {
            match vai.checked_sub(vbi) {
                Some(v) => crate::boxing::rt_box_int(v),
                None => {
                    let msg = b"integer overflow";
                    rt_exc_raise(ExceptionType::OverflowError as u8, msg.as_ptr(), msg.len());
                }
            }
        } else {
            crate::boxing::rt_box_float(va - vb)
        }
    }
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_mul(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let tag_a = (*a).type_tag();
        let tag_b = (*b).type_tag();
        // String repetition: str * int or int * str
        if tag_a == TypeTagKind::Str && tag_b == TypeTagKind::Int {
            return crate::string::rt_str_mul(a, (*(b as *mut IntObj)).value);
        }
        if tag_a == TypeTagKind::Int && tag_b == TypeTagKind::Str {
            return crate::string::rt_str_mul(b, (*(a as *mut IntObj)).value);
        }
        let (va, vb, both_int, vai, vbi) = extract_numeric_pair(a, b);
        if both_int {
            match vai.checked_mul(vbi) {
                Some(v) => crate::boxing::rt_box_int(v),
                None => {
                    let msg = b"integer overflow";
                    rt_exc_raise(ExceptionType::OverflowError as u8, msg.as_ptr(), msg.len());
                }
            }
        } else {
            crate::boxing::rt_box_float(va * vb)
        }
    }
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_div(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let (va, vb, _, _, _) = extract_numeric_pair(a, b);
        if vb == 0.0 {
            let msg = "division by zero";
            rt_exc_raise(
                ExceptionType::ZeroDivisionError as u8,
                msg.as_ptr(),
                msg.len(),
            );
        }
        crate::boxing::rt_box_float(va / vb) // Python 3: true division always float
    }
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_floordiv(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let (va, vb, both_int, vai, vbi) = extract_numeric_pair(a, b);
        if both_int {
            if vbi == 0 {
                let msg = "integer division or modulo by zero";
                rt_exc_raise(
                    ExceptionType::ZeroDivisionError as u8,
                    msg.as_ptr(),
                    msg.len(),
                );
            }
            if vai == i64::MIN && vbi == -1 {
                let msg = b"integer overflow";
                rt_exc_raise(ExceptionType::OverflowError as u8, msg.as_ptr(), msg.len());
            }
            let d = vai / vbi;
            let r = vai % vbi;
            let result = if r != 0 && (r ^ vbi) < 0 { d - 1 } else { d };
            crate::boxing::rt_box_int(result)
        } else {
            if vb == 0.0 {
                let msg = "integer division or modulo by zero";
                rt_exc_raise(
                    ExceptionType::ZeroDivisionError as u8,
                    msg.as_ptr(),
                    msg.len(),
                );
            }
            crate::boxing::rt_box_float((va / vb).floor())
        }
    }
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_mod(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let (va, vb, both_int, vai, vbi) = extract_numeric_pair(a, b);
        if both_int {
            if vbi == 0 {
                let msg = "integer division or modulo by zero";
                rt_exc_raise(
                    ExceptionType::ZeroDivisionError as u8,
                    msg.as_ptr(),
                    msg.len(),
                );
            }
            if vai == i64::MIN && vbi == -1 {
                let msg = b"integer overflow";
                rt_exc_raise(ExceptionType::OverflowError as u8, msg.as_ptr(), msg.len());
            }
            let r = vai % vbi;
            let result = if r != 0 && (r ^ vbi) < 0 { r + vbi } else { r };
            crate::boxing::rt_box_int(result)
        } else {
            if vb == 0.0 {
                let msg = "integer division or modulo by zero";
                rt_exc_raise(
                    ExceptionType::ZeroDivisionError as u8,
                    msg.as_ptr(),
                    msg.len(),
                );
            }
            crate::boxing::rt_box_float(va % vb)
        }
    }
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_pow(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let (va, vb, both_int, vai, vbi) = extract_numeric_pair(a, b);
        if both_int && vbi >= 0 {
            let exp = vbi as u32;
            let mut result: i64 = 1;
            let mut base = vai;
            let mut e = exp;
            let mut overflow = false;
            while e > 0 {
                if e & 1 == 1 {
                    match result.checked_mul(base) {
                        Some(v) => result = v,
                        None => {
                            overflow = true;
                            break;
                        }
                    }
                }
                e >>= 1;
                if e > 0 {
                    match base.checked_mul(base) {
                        Some(v) => base = v,
                        None => {
                            overflow = true;
                            break;
                        }
                    }
                }
            }
            if overflow {
                let msg = b"integer overflow";
                rt_exc_raise(ExceptionType::OverflowError as u8, msg.as_ptr(), msg.len());
            }
            crate::boxing::rt_box_int(result)
        } else {
            crate::boxing::rt_box_float(va.powf(vb))
        }
    }
}

/// Runtime-dispatched subscript: obj[index] where obj has unknown type at compile time.
/// Dispatches to the appropriate getter based on the object's type tag.
/// Returns boxed value (*mut Obj) for all types.
#[no_mangle]
pub extern "C" fn rt_any_getitem(obj: *mut Obj, index: i64) -> *mut Obj {
    use crate::object::*;

    if obj.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        match (*obj).type_tag() {
            TypeTagKind::List => {
                let list = obj as *mut ListObj;
                let len = (*list).len as i64;
                let actual_idx = if index < 0 { len + index } else { index };
                if actual_idx < 0 || actual_idx >= len {
                    let msg = b"IndexError: list index out of range";
                    crate::exceptions::rt_exc_raise(
                        crate::exceptions::ExceptionType::IndexError as u8,
                        msg.as_ptr(),
                        msg.len(),
                    );
                }
                let elem = *(*list).data.add(actual_idx as usize);
                // If list stores raw ints (ELEM_RAW_INT), box them
                if (*list).elem_tag == ELEM_RAW_INT {
                    crate::boxing::rt_box_int(elem as i64)
                } else {
                    elem
                }
            }
            TypeTagKind::Tuple => {
                let tuple = obj as *mut TupleObj;
                let len = (*tuple).len as i64;
                let actual_idx = if index < 0 { len + index } else { index };
                if actual_idx < 0 || actual_idx >= len {
                    let msg = b"IndexError: tuple index out of range";
                    crate::exceptions::rt_exc_raise(
                        crate::exceptions::ExceptionType::IndexError as u8,
                        msg.as_ptr(),
                        msg.len(),
                    );
                }
                let elem = *(*tuple).data.as_ptr().add(actual_idx as usize);
                // Check heap_field_mask: if this field is NOT a heap pointer, box it
                let is_heap = (*tuple).heap_field_mask & (1u64 << actual_idx as u64) != 0;
                if !is_heap && (*tuple).elem_tag == ELEM_RAW_INT {
                    crate::boxing::rt_box_int(elem as i64)
                } else {
                    elem
                }
            }
            TypeTagKind::Dict | TypeTagKind::DefaultDict | TypeTagKind::Counter => {
                // Dict subscript needs a boxed key
                let boxed_key = crate::boxing::rt_box_int(index);
                crate::dict::rt_dict_get(obj, boxed_key)
            }
            TypeTagKind::Str => {
                // String subscript returns single-char string
                crate::string::rt_str_getchar(obj, index)
            }
            _ => std::ptr::null_mut(),
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
        let msg = b"TypeError: argument of type 'NoneType' is not iterable";
        unsafe { rt_exc_raise(ExceptionType::TypeError as u8, msg.as_ptr(), msg.len()) }
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
            _ => {
                let tag_str = type_name((*container).type_tag());
                crate::raise_exc!(
                    ExceptionType::TypeError,
                    "argument of type '{}' is not iterable",
                    tag_str
                );
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn init_runtime() {
        let _guard = crate::RUNTIME_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::gc::init();
    }

    #[test]
    fn test_add_int_basic() {
        assert_eq!(rt_add_int(1, 2), 3);
        assert_eq!(rt_add_int(0, 0), 0);
        assert_eq!(rt_add_int(-5, 3), -2);
        assert_eq!(rt_add_int(i64::MAX - 1, 1), i64::MAX);
    }

    #[test]
    fn test_sub_int_basic() {
        assert_eq!(rt_sub_int(5, 3), 2);
        assert_eq!(rt_sub_int(0, 0), 0);
        assert_eq!(rt_sub_int(i64::MIN + 1, 1), i64::MIN);
    }

    #[test]
    fn test_mul_int_basic() {
        assert_eq!(rt_mul_int(3, 4), 12);
        assert_eq!(rt_mul_int(0, i64::MAX), 0);
        assert_eq!(rt_mul_int(-3, 5), -15);
    }

    #[test]
    fn test_div_int_floor() {
        // Python floor division: -7 // 2 = -4 (not -3)
        assert_eq!(rt_div_int(7, 2), 3);
        assert_eq!(rt_div_int(-7, 2), -4);
        assert_eq!(rt_div_int(7, -2), -4);
        assert_eq!(rt_div_int(-7, -2), 3);
        assert_eq!(rt_div_int(6, 3), 2);
    }

    #[test]
    fn test_mod_int() {
        assert_eq!(rt_mod_int(7, 3), 1);
        assert_eq!(rt_mod_int(-7, 3), 2); // Python: -7 % 3 = 2
        assert_eq!(rt_mod_int(7, -3), -2); // Python: 7 % -3 = -2
    }

    #[test]
    fn test_float_arithmetic() {
        assert_eq!(rt_add_float(1.5, 2.5), 4.0);
        assert_eq!(rt_sub_float(5.0, 3.0), 2.0);
        assert_eq!(rt_mul_float(2.0, 3.0), 6.0);
        assert_eq!(rt_div_float(7.0, 2.0), 3.5);
    }

    #[test]
    fn test_true_div_int() {
        assert_eq!(rt_true_div_int(7, 2), 3.5);
        assert_eq!(rt_true_div_int(6, 3), 2.0);
    }

    #[test]
    fn test_is_truthy_int() {
        init_runtime();
        let zero = crate::boxing::rt_box_int(0);
        assert_eq!(rt_is_truthy(zero), 0);
        let one = crate::boxing::rt_box_int(1);
        assert_eq!(rt_is_truthy(one), 1);
        let neg = crate::boxing::rt_box_int(-1);
        assert_eq!(rt_is_truthy(neg), 1);
    }
}

/// Check if bytes contains an integer value
unsafe fn rt_bytes_contains_value(bytes: *mut Obj, value: *mut Obj) -> i8 {
    use crate::object::{BytesObj, IntObj};

    // value should be an integer
    if value.is_null() || (*value).type_tag() != TypeTagKind::Int {
        let type_str = if value.is_null() {
            "NoneType"
        } else {
            type_name((*value).type_tag())
        };
        crate::raise_exc!(
            ExceptionType::TypeError,
            "a bytes-like object is required, not '{}'",
            type_str
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
