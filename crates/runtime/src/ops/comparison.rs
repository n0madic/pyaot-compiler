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
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_eq(a: *mut Obj, b: *mut Obj) -> i8 {
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

    // Bignum-aware: when a heap BigInt is involved, compare as numbers
    // (int/bignum exact, float via f64). Only entered when a BigInt is present,
    // so non-bignum behavior is unchanged.
    unsafe {
        let a_big = va.is_ptr() && !a.is_null() && (*a).type_tag() == TypeTagKind::BigInt;
        let b_big = vb.is_ptr() && !b.is_null() && (*b).type_tag() == TypeTagKind::BigInt;
        if a_big || b_big {
            return match (crate::bigint::classify_num(va), crate::bigint::classify_num(vb)) {
                (Some(x), Some(y)) => crate::bigint::num_eq(&x, &y) as i8,
                _ => 0, // BigInt vs non-number → not equal
            };
        }
    }

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

    // One primitive, one heap pointer → different types in Python's runtime
    // type sense. The exception is the numeric tower: `8 == 8.0`, `True ==
    // 1.0`, etc. A tagged Int/Bool on one side and a heap-boxed FloatObj on
    // the other should compare as f64 to match CPython.
    if !va.is_ptr() || !vb.is_ptr() {
        let primitive_as_f64 = |v: Value| -> Option<f64> {
            if v.is_int() {
                Some(v.unwrap_int() as f64)
            } else if v.is_bool() {
                Some(if v.unwrap_bool() { 1.0 } else { 0.0 })
            } else {
                None
            }
        };
        let heap_float_as_f64 = |p: *mut Obj| -> Option<f64> {
            if p.is_null() {
                return None;
            }
            unsafe {
                if (*p).type_tag() == TypeTagKind::Float {
                    Some((*(p as *mut FloatObj)).value)
                } else {
                    None
                }
            }
        };
        let f_a = if va.is_ptr() {
            heap_float_as_f64(a)
        } else {
            primitive_as_f64(va)
        };
        let f_b = if vb.is_ptr() {
            heap_float_as_f64(b)
        } else {
            primitive_as_f64(vb)
        };
        return match (f_a, f_b) {
            (Some(x), Some(y)) if x == y => 1,
            _ => 0,
        };
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
            // Containers compare structurally (Python `==`), not by
            // identity. `rt_tuple_eq` / `rt_list_eq` recurse element-wise
            // via `eq_hashable_obj`. Without these arms, comparing two
            // distinct-but-equal tuples/lists through the generic
            // `rt_obj_eq` path (an `Any`/`Union` operand vs a container)
            // would fall to the identity check below and wrongly report
            // inequality.
            TypeTagKind::Tuple => crate::tuple::rt_tuple_eq(a, b),
            TypeTagKind::List => crate::list::rt_list_eq(a, b),
            // Dict, DefaultDict and Counter all share the `DictObj`
            // memory layout — `rt_dict_eq` reads only the common fields
            // (`len` / `entries` / `entries_len` / the index table), so
            // it compares any of them structurally.
            TypeTagKind::Dict | TypeTagKind::DefaultDict | TypeTagKind::Counter => {
                crate::dict::rt_dict_eq(a, b)
            }
            TypeTagKind::Set => crate::set::rt_set_eq(a, b),
            // Class instances and other types: identity comparison. Class
            // instances without `__eq__` use identity, matching Python's
            // default object equality.
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
#[export_name = "rt_obj_eq"]
pub extern "C" fn rt_obj_eq_abi(a: Value, b: Value) -> i8 {
    rt_obj_eq(a.unwrap_ptr(), b.unwrap_ptr())
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
    // From here both operands must be real heap pointers. A tagged
    // immediate (int/bool) that survived every numeric/None arm above is
    // paired with a non-Float heap object — genuinely incomparable in
    // Python. Raise instead of dereferencing the immediate as a pointer.
    if !va.is_ptr() || !vb.is_ptr() {
        let name_a = if va.is_ptr() {
            type_name((*a).type_tag())
        } else if va.is_int() {
            "int"
        } else {
            "bool"
        };
        let name_b = if vb.is_ptr() {
            type_name((*b).type_tag())
        } else if vb.is_int() {
            "int"
        } else {
            "bool"
        };
        raise_exc!(
            ExceptionType::TypeError,
            "'<' not supported between instances of '{}' and '{}'",
            name_a,
            name_b
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
            TypeTagKind::Bytes => {
                // Lexicographic byte comparison — mirrors CPython `b"a" < b"b"`.
                let bytes_a = a as *mut BytesObj;
                let bytes_b = b as *mut BytesObj;
                let len_a = (*bytes_a).len;
                let len_b = (*bytes_b).len;
                let data_a = std::slice::from_raw_parts((*bytes_a).data.as_ptr(), len_a);
                let data_b = std::slice::from_raw_parts((*bytes_b).data.as_ptr(), len_b);
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
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_cmp(a: *mut Obj, b: *mut Obj, op_tag: u8) -> i8 {
    unsafe {
        // Bignum-aware ordering when a heap BigInt is involved.
        let va = Value(a as u64);
        let vb = Value(b as u64);
        let a_big = va.is_ptr() && !a.is_null() && (*a).type_tag() == TypeTagKind::BigInt;
        let b_big = vb.is_ptr() && !b.is_null() && (*b).type_tag() == TypeTagKind::BigInt;
        if a_big || b_big {
            if let (Some(x), Some(y)) = (crate::bigint::classify_num(va), crate::bigint::classify_num(vb)) {
                return match crate::bigint::num_cmp(&x, &y) {
                    Some(ord) => {
                        let r = match op_tag {
                            0 => ord == std::cmp::Ordering::Less,
                            1 => ord != std::cmp::Ordering::Greater,
                            2 => ord == std::cmp::Ordering::Greater,
                            3 => ord != std::cmp::Ordering::Less,
                            _ => false,
                        };
                        r as i8
                    }
                    None => 0, // NaN
                };
            }
        }
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
#[export_name = "rt_obj_cmp"]
pub extern "C" fn rt_obj_cmp_abi(a: Value, b: Value, op_tag: u8) -> i8 {
    rt_obj_cmp(a.unwrap_ptr(), b.unwrap_ptr(), op_tag)
}

/// Runtime-dispatched subscript: obj[index] where obj has unknown type at compile time.
/// Dispatches to the appropriate getter based on the object's type tag.
/// Returns boxed value (*mut Obj) for all types.
pub fn rt_any_getitem(obj: *mut Obj, index: i64) -> *mut Obj {
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
#[export_name = "rt_any_getitem"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_any_getitem_abi(obj: Value, index: i64) -> Value {
    Value::from_ptr(rt_any_getitem(obj.unwrap_ptr(), index))
}

/// Runtime-dispatched slicing: obj[start:end] where obj has unknown type at
/// compile time. Mirrors `rt_any_getitem`'s dispatch table for Index, but
/// for Slice (delegates to the type-specific slice-runtime). Required when
/// lowering sees `obj_type == Any | HeapAny` — without this, `lower_slice`
/// silently falls through and returns a `None` constant, producing
/// empty-list shapes for downstream consumers and triggering null-deref
/// SEGVs in compiled patterns like microgpt's `[ki[hs:hs+head_dim] for ki
/// in keys[li]]` where `ki`'s element type collapses to `Any` because of
/// gpt's (unannotated) `keys` / `values` params.
///
/// Sentinel handling: `i64::MIN` for unspecified start, `i64::MAX` for
/// unspecified end — matches the codegen-side defaults emitted by
/// `lower_slice`. Each typed runtime slicer already understands these
/// sentinels, so we just forward.
pub fn rt_obj_slice(obj: *mut Obj, start: i64, end: i64) -> *mut Obj {
    if obj.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        match (*obj).type_tag() {
            TypeTagKind::List => crate::list::rt_list_slice(obj, start, end),
            TypeTagKind::Tuple => crate::tuple::rt_tuple_slice(obj, start, end),
            TypeTagKind::Str => crate::string::rt_str_slice(obj, start, end),
            TypeTagKind::Bytes => crate::bytes::rt_bytes_slice(obj, start, end),
            _ => std::ptr::null_mut(),
        }
    }
}
#[export_name = "rt_obj_slice"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_slice_abi(obj: Value, start: i64, end: i64) -> Value {
    Value::from_ptr(rt_obj_slice(obj.unwrap_ptr(), start, end))
}

/// Runtime-dispatched slicing with step: `obj[start:end:step]`. See
/// `rt_obj_slice` for the Any-typed motivation.
pub fn rt_obj_slice_step(obj: *mut Obj, start: i64, end: i64, step: i64) -> *mut Obj {
    if step == 0 {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "slice step cannot be zero"
            );
        }
    }
    if obj.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        match (*obj).type_tag() {
            TypeTagKind::List => crate::list::rt_list_slice_step(obj, start, end, step),
            TypeTagKind::Tuple => crate::tuple::rt_tuple_slice_step(obj, start, end, step),
            TypeTagKind::Str => crate::string::rt_str_slice_step(obj, start, end, step),
            TypeTagKind::Bytes => crate::bytes::rt_bytes_slice_step(obj, start, end, step),
            _ => std::ptr::null_mut(),
        }
    }
}
/// Runtime-dispatched length: `len(obj)` where obj has unknown type at
/// compile time. Mirrors `rt_obj_slice`'s dispatch, but for `len()`.
/// Without this, `len(obj)` for `obj: Any | HeapAny` falls into the
/// `lower_len` Any-arm fallback that emits `Const(0)`, silently
/// reporting zero length for legitimate non-empty containers — common
/// for `len(out[0])` where `out` is `list[Any]` because its element
/// expression's type collapsed to `Any` somewhere in the type-planning
/// fixpoint.
pub fn rt_obj_len(obj: *mut Obj) -> i64 {
    if obj.is_null() {
        return 0;
    }
    unsafe {
        match (*obj).type_tag() {
            TypeTagKind::List => crate::list::rt_list_len(obj),
            TypeTagKind::Tuple => crate::tuple::rt_tuple_len(obj),
            TypeTagKind::Dict | TypeTagKind::DefaultDict | TypeTagKind::Counter => {
                crate::dict::rt_dict_len(obj)
            }
            TypeTagKind::Set => crate::set::rt_set_len(obj),
            TypeTagKind::Str => crate::string::rt_str_len_int(obj),
            TypeTagKind::Bytes => crate::bytes::rt_bytes_len(obj),
            TypeTagKind::Deque => crate::deque::rt_deque_len(obj),
            _ => 0,
        }
    }
}
#[export_name = "rt_obj_len"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_len_abi(obj: Value) -> i64 {
    rt_obj_len(obj.unwrap_ptr())
}

#[export_name = "rt_obj_slice_step"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_slice_step_abi(obj: Value, start: i64, end: i64, step: i64) -> Value {
    Value::from_ptr(rt_obj_slice_step(obj.unwrap_ptr(), start, end, step))
}

/// Check if element is in container with runtime type dispatch
/// Returns 1 if element is in container, 0 otherwise
/// Used for Union container types where the actual type is determined at runtime
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_contains(container: *mut Obj, elem: *mut Obj) -> i8 {
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
#[export_name = "rt_obj_contains"]
pub extern "C" fn rt_obj_contains_abi(container: Value, elem: Value) -> i8 {
    rt_obj_contains(container.unwrap_ptr(), elem.unwrap_ptr())
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
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_is_none(obj: *mut Obj) -> i8 {
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
#[export_name = "rt_is_none"]
pub extern "C" fn rt_is_none_abi(obj: Value) -> i8 {
    rt_is_none(obj.unwrap_ptr())
}

/// Check truthiness of any value with runtime type dispatch
/// Returns 1 if truthy, 0 if falsy
/// Falsy values: None, False, 0, 0.0, empty str/list/tuple/dict/set/bytes
/// Used for filter(None, iterable) to filter out falsy values
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_is_truthy(obj: *mut Obj) -> i8 {
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
#[export_name = "rt_is_truthy"]
pub extern "C" fn rt_is_truthy_abi(obj: Value) -> i8 {
    rt_is_truthy(obj.unwrap_ptr())
}
