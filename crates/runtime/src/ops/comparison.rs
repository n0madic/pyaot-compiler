//! Comparison, containment, truthiness, and subscript operations for Python runtime

use crate::exceptions::ExceptionType;
use crate::object::{
    BytesObj, DictObj, FloatObj, ListObj, Obj, SetObj, StrObj, TupleObj, TypeTagKind,
};
use pyaot_core_defs::Value;

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

    // Apply Value tag dispatch first before any pointer dereference.
    let va = Value(a as u64);
    let vb = Value(b as u64);

    // Both tagged primitives: compare by type + value.
    if !va.is_ptr() && !vb.is_ptr() {
        // Same tagged kind → bit-for-bit comparison covers Int==Int and Bool==Bool.
        // Cross-kind (Int vs Bool): unwrap both as integers (True==1, False==0).
        if va.is_int() && vb.is_int() {
            return if va.unwrap_int() == vb.unwrap_int() {
                1
            } else {
                0
            };
        }
        if va.is_bool() && vb.is_bool() {
            return if va.unwrap_bool() == vb.unwrap_bool() {
                1
            } else {
                0
            };
        }
        // Int/Bool cross-type equality (Python: 1 == True, 0 == False)
        if (va.is_int() && vb.is_bool()) || (va.is_bool() && vb.is_int()) {
            let ia = if va.is_int() {
                va.unwrap_int()
            } else {
                va.unwrap_bool() as i64
            };
            let ib = if vb.is_int() {
                vb.unwrap_int()
            } else {
                vb.unwrap_bool() as i64
            };
            return if ia == ib { 1 } else { 0 };
        }
        // None == None
        if va.is_none() && vb.is_none() {
            return 1;
        }
        return 0;
    }

    // One primitive, one heap pointer → different types, cannot be equal
    // (unless the primitive is None-tagged and the heap object is a NoneObj,
    //  but that case is irrelevant for Stage C since both still arrive as *mut Obj).
    if !va.is_ptr() || !vb.is_ptr() {
        return 0;
    }

    // Both are real heap pointers — safe to dereference.
    unsafe {
        let tag_a = (*a).type_tag();
        let tag_b = (*b).type_tag();

        // Different types → not equal (float/int cross-type not handled here;
        // the Union equality path goes through hash_table_utils::eq_hashable_obj)
        if tag_a != tag_b {
            return 0;
        }

        match tag_a {
            TypeTagKind::Float => {
                let fa = (*(a as *mut FloatObj)).value;
                let fb = (*(b as *mut FloatObj)).value;
                if fa == fb {
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

    // Check Value tags first — Int and Bool are tagged primitives, not heap pointers.
    let va = Value(a as u64);
    let vb = Value(b as u64);

    // Int/Int ordering
    if va.is_int() && vb.is_int() {
        return va.unwrap_int().cmp(&vb.unwrap_int());
    }
    // Bool/Bool ordering
    if va.is_bool() && vb.is_bool() {
        return va.unwrap_bool().cmp(&vb.unwrap_bool());
    }
    // Int/Bool or Bool/Int cross-type ordering (Python: bool is int subtype)
    if (va.is_int() || va.is_bool()) && (vb.is_int() || vb.is_bool()) {
        let ia: i64 = if va.is_int() {
            va.unwrap_int()
        } else {
            va.unwrap_bool() as i64
        };
        let ib: i64 = if vb.is_int() {
            vb.unwrap_int()
        } else {
            vb.unwrap_bool() as i64
        };
        return ia.cmp(&ib);
    }
    // Int (tagged) vs Float (heap) — promote int to float
    if va.is_int() && vb.is_ptr() && (*b).type_tag() == TypeTagKind::Float {
        let fa = va.unwrap_int() as f64;
        let fb = (*(b as *mut FloatObj)).value;
        return fa.partial_cmp(&fb).unwrap_or(Ordering::Greater);
    }
    if vb.is_int() && va.is_ptr() && (*a).type_tag() == TypeTagKind::Float {
        let fa = (*(a as *mut FloatObj)).value;
        let fb = vb.unwrap_int() as f64;
        return fa.partial_cmp(&fb).unwrap_or(Ordering::Greater);
    }
    // Bool (tagged) vs Float (heap) — same as Int vs Float
    if va.is_bool() && vb.is_ptr() && (*b).type_tag() == TypeTagKind::Float {
        let fa = va.unwrap_bool() as i64 as f64;
        let fb = (*(b as *mut FloatObj)).value;
        return fa.partial_cmp(&fb).unwrap_or(Ordering::Greater);
    }
    if vb.is_bool() && va.is_ptr() && (*a).type_tag() == TypeTagKind::Float {
        let fa = (*(a as *mut FloatObj)).value;
        let fb = vb.unwrap_bool() as i64 as f64;
        return fa.partial_cmp(&fb).unwrap_or(Ordering::Greater);
    }
    // Tagged None is not orderable
    if va.is_none() || vb.is_none() {
        raise_exc!(
            ExceptionType::TypeError,
            "'<' not supported between instances of 'NoneType' and other types"
        );
    }
    // Both must be real heap pointers from here on.
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
            TypeTagKind::Float => {
                let fa = (*(a as *mut FloatObj)).value;
                let fb = (*(b as *mut FloatObj)).value;
                // NaN sorts to the end (Greater) to provide a stable ordering for sorted()
                fa.partial_cmp(&fb).unwrap_or(Ordering::Greater)
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

    // Mixed heap types (e.g., Float vs something else)
    if (tag_a == TypeTagKind::Float) || (tag_b == TypeTagKind::Float) {
        let fa = if tag_a == TypeTagKind::Float {
            (*(a as *mut FloatObj)).value
        } else {
            // tag_b is Float; tag_a is something non-numeric — fall through to error
            crate::raise_exc!(
                ExceptionType::TypeError,
                "'<' not supported between instances of '{}' and '{}'",
                type_name(tag_a),
                type_name(tag_b)
            );
        };
        let fb = (*(b as *mut FloatObj)).value;
        return fa.partial_cmp(&fb).unwrap_or(Ordering::Greater);
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
    // Tagged Int/Bool are never NaN.
    let va = Value(a as u64);
    let vb = Value(b as u64);
    if va.is_ptr()
        && (*a).type_tag() == TypeTagKind::Float
        && (*(a as *mut FloatObj)).value.is_nan()
    {
        return true;
    }
    if vb.is_ptr()
        && (*b).type_tag() == TypeTagKind::Float
        && (*(b as *mut FloatObj)).value.is_nan()
    {
        return true;
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
                (*(*list).data.add(actual_idx as usize)).0 as *mut Obj
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
                // Slots are uniformly tagged Values; return as-is.
                let elem = *(*tuple).data.as_ptr().add(actual_idx as usize);
                elem.0 as *mut Obj
            }
            TypeTagKind::Dict | TypeTagKind::DefaultDict | TypeTagKind::Counter => {
                // Dict subscript needs a boxed key
                let boxed_key =
                    pyaot_core_defs::Value::from_int(index).0 as *mut crate::object::Obj;
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
        let elem = (*(*list_obj).data.add(i)).0 as *mut Obj;
        if rt_obj_eq(elem, value) == 1 {
            return 1;
        }
    }

    0
}

/// Check if tuple contains value using value equality
unsafe fn rt_tuple_contains_value(tuple: *mut Obj, value: *mut Obj) -> i8 {
    use crate::hash_table_utils::eq_hashable_obj;
    let tuple_obj = tuple as *mut TupleObj;
    let len = (*tuple_obj).len;
    let data = (*tuple_obj).data.as_ptr();

    for i in 0..len {
        let elem = *data.add(i);
        if eq_hashable_obj(elem.0 as *mut Obj, value) {
            return 1;
        }
    }
    0
}

/// Check if bytes contains an integer value
pub(super) unsafe fn rt_bytes_contains_value(bytes: *mut Obj, value: *mut Obj) -> i8 {
    // value should be an integer — check the Value tag first.
    let int_val: i64 = if value.is_null() {
        crate::raise_exc!(
            ExceptionType::TypeError,
            "a bytes-like object is required, not 'NoneType'"
        );
    } else {
        let vv = Value(value as u64);
        if vv.is_int() {
            vv.unwrap_int()
        } else if vv.is_bool() {
            vv.unwrap_bool() as i64
        } else if vv.is_none() {
            crate::raise_exc!(
                ExceptionType::TypeError,
                "a bytes-like object is required, not 'NoneType'"
            );
        } else {
            // Heap pointer — must be Int type tag for bytes containment check.
            let tag = (*value).type_tag();
            if tag != TypeTagKind::Int {
                crate::raise_exc!(
                    ExceptionType::TypeError,
                    "a bytes-like object is required, not '{}'",
                    type_name(tag)
                );
            }
            // Heap Int is no longer valid post-Stage B, but handle defensively.
            // TODO stageD: remove heap-Int fallback once boxing is fully eliminated.
            crate::raise_exc!(
                ExceptionType::TypeError,
                "a bytes-like object is required, not 'int'"
            );
        }
    };
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

/// Check whether `obj` represents Python `None`.
///
/// Returns 1 if obj is a null pointer (runtime's unset / default-filled
/// optional representation) OR a `NoneObj` singleton (what the compiler
/// boxes a user-level `None` literal into when it crosses into an
/// `Optional[Heap]` slot). Used by the `is None` / `is not None` lowering
/// to sidestep the null-vs-NoneObj ABI asymmetry.
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_is_none(obj: *mut Obj) -> i8 {
    let v = pyaot_core_defs::Value(obj as u64);
    if obj.is_null() || v.is_none() {
        return 1;
    }
    if !v.is_ptr() {
        return 0;
    }
    unsafe {
        if (*obj).type_tag() == TypeTagKind::None {
            1
        } else {
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

    // Check Value-tagged primitives before any heap pointer dereference.
    let v = Value(obj as u64);
    if v.is_int() {
        return if v.unwrap_int() != 0 { 1 } else { 0 };
    }
    if v.is_bool() {
        return if v.unwrap_bool() { 1 } else { 0 };
    }
    if v.is_none() {
        return 0;
    }

    unsafe {
        match (*obj).type_tag() {
            TypeTagKind::None => 0,
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
