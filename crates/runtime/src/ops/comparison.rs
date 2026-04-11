//! Comparison, containment, truthiness, and subscript operations for Python runtime

use crate::exceptions::ExceptionType;
use crate::object::{
    BoolObj, BytesObj, DictObj, FloatObj, IntObj, ListObj, Obj, SetObj, StrObj, TupleObj,
    TypeTagKind, ELEM_RAW_BOOL, ELEM_RAW_INT,
};

/// Helper to get type name for error messages.
/// Delegates to TypeTagKind::type_name() from core-defs (single source of truth).
#[inline]
pub(super) fn type_name(tag: TypeTagKind) -> &'static str {
    tag.type_name()
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
pub(super) unsafe fn obj_cmp_ordering(a: *mut Obj, b: *mut Obj) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    // Handle null (None) - None is not orderable
    if a.is_null() || b.is_null() {
        raise_exc!(
            ExceptionType::TypeError,
            "'<' not supported between instances of 'NoneType' and other types"
        );
    }

    let tag_a = (*a).type_tag();
    let tag_b = (*b).type_tag();

    // Check for None type tag
    if tag_a == TypeTagKind::None || tag_b == TypeTagKind::None {
        raise_exc!(
            ExceptionType::TypeError,
            "'<' not supported between instances of 'NoneType' and other types"
        );
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

/// Compare two heap objects for ordering with runtime type dispatch.
///
/// `op_tag` encodes the comparison operator: 0=Lt, 1=Lte, 2=Gt, 3=Gte
/// (matches `mir::ComparisonOp::to_tag()`).
///
/// Returns 1 if the comparison is true, 0 otherwise. NaN comparisons always
/// return false (0) per Python semantics.
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_cmp(a: *mut Obj, b: *mut Obj, op_tag: u8) -> i8 {
    unsafe {
        // NaN comparisons always return False in Python
        if involves_nan(a, b) {
            return 0;
        }
        let ord = obj_cmp_ordering(a, b);
        let result = match op_tag {
            0 => ord == std::cmp::Ordering::Less, // Lt
            1 => ord == std::cmp::Ordering::Less || ord == std::cmp::Ordering::Equal, // Lte
            2 => ord == std::cmp::Ordering::Greater, // Gt
            3 => ord == std::cmp::Ordering::Greater || ord == std::cmp::Ordering::Equal, // Gte
            _ => false,
        };
        result as i8
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
                    raise_exc!(
                        crate::exceptions::ExceptionType::IndexError,
                        "list index out of range"
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
                    raise_exc!(
                        crate::exceptions::ExceptionType::IndexError,
                        "tuple index out of range"
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
        unsafe {
            raise_exc!(
                ExceptionType::TypeError,
                "argument of type 'NoneType' is not iterable"
            )
        }
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

/// Check if bytes contains an integer value
pub(super) unsafe fn rt_bytes_contains_value(bytes: *mut Obj, value: *mut Obj) -> i8 {
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
