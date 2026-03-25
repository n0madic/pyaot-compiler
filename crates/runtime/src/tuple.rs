//! Tuple operations for Python runtime

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::gc;
use crate::object::{Obj, TypeTagKind, ELEM_HEAP_OBJ};
use crate::slice_utils::{collect_step_indices, normalize_slice_indices, slice_length};

/// Create a new tuple with given size and element tag
/// elem_tag: ELEM_HEAP_OBJ (0), ELEM_RAW_INT (1), or ELEM_RAW_BOOL (2)
/// Returns: pointer to allocated TupleObj
#[no_mangle]
pub extern "C" fn rt_make_tuple(size: i64, elem_tag: u8) -> *mut Obj {
    use crate::object::{TupleObj, TypeTagKind};

    let size = size.max(0) as usize;

    // Calculate size: base struct size + inline data array
    // TupleObj has: ObjHeader(16) + len(8) + elem_tag(1) + padding(7) + data[0]
    // Use size_of::<TupleObj> for the base size (includes alignment padding)
    let tuple_size = std::mem::size_of::<TupleObj>() + size * std::mem::size_of::<*mut Obj>();

    // Allocate TupleObj using GC
    let obj = gc::gc_alloc(tuple_size, TypeTagKind::Tuple as u8);

    unsafe {
        let tuple = obj as *mut TupleObj;
        (*tuple).len = size;
        (*tuple).elem_tag = elem_tag;
        // Default heap_field_mask: all fields are heap objects when ELEM_HEAP_OBJ,
        // no fields are heap objects when ELEM_RAW_INT/ELEM_RAW_BOOL.
        (*tuple).heap_field_mask = if elem_tag == ELEM_HEAP_OBJ {
            u64::MAX
        } else {
            0
        };

        // Initialize all elements to null
        let data_ptr = (*tuple).data.as_mut_ptr();
        for i in 0..size {
            *data_ptr.add(i) = std::ptr::null_mut();
        }
    }

    obj
}

/// Set the heap_field_mask on a tuple.
/// Called after rt_make_tuple when the caller knows per-field GC tracing info.
/// mask: bitmask where bit i = 1 means field i is a heap pointer.
#[no_mangle]
pub extern "C" fn rt_tuple_set_heap_mask(tuple: *mut Obj, mask: i64) {
    if tuple.is_null() {
        return;
    }
    unsafe {
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        (*tuple_obj).heap_field_mask = mask as u64;
    }
}

/// Set element in tuple at given index (used during tuple construction)
#[no_mangle]
pub extern "C" fn rt_tuple_set(tuple: *mut Obj, index: i64, value: *mut Obj) {
    if tuple.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_set");
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        let len = (*tuple_obj).len as i64;

        // Only positive indices during construction
        if index < 0 || index >= len {
            return;
        }

        // Note: no validate_elem_tag! for tuples — the GC uses heap_field_mask for
        // precise per-field tracing, making the elem_tag-based validation unnecessary.
        // Mixed-type tuples (captures, *args) are safely handled by the mask.

        let data_ptr = (*tuple_obj).data.as_mut_ptr();
        *data_ptr.add(index as usize) = value;
    }
}

/// Get element from tuple at given index
/// Supports negative indexing
/// Returns: pointer to element or null if out of bounds
#[no_mangle]
pub extern "C" fn rt_tuple_get(tuple: *mut Obj, index: i64) -> *mut Obj {
    if tuple.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_get");
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        let len = (*tuple_obj).len as i64;

        // Handle negative index
        let idx = if index < 0 { len + index } else { index };

        // Bounds check
        if idx < 0 || idx >= len {
            return std::ptr::null_mut();
        }

        let data_ptr = (*tuple_obj).data.as_ptr();
        *data_ptr.add(idx as usize)
    }
}

/// Get length of tuple
#[no_mangle]
pub extern "C" fn rt_tuple_len(tuple: *mut Obj) -> i64 {
    if tuple.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_len");
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        (*tuple_obj).len as i64
    }
}

/// Slice a tuple: tuple[start:end]
/// Negative indices are supported (counted from end)
/// Uses i64::MIN as sentinel for "default start" (0) and i64::MAX for "default end" (len)
/// Returns: pointer to new allocated TupleObj (shallow copy)
#[no_mangle]
pub extern "C" fn rt_tuple_slice(tuple: *mut Obj, start: i64, end: i64) -> *mut Obj {
    if tuple.is_null() {
        return rt_make_tuple(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_slice");
        let src = tuple as *mut crate::object::TupleObj;
        let len = (*src).len as i64;

        // Normalize indices using shared utility (step=1 for simple slice)
        let (start, end) = normalize_slice_indices(start, end, len, 1);
        let slice_len = slice_length(start, end);

        // Create new tuple
        let new_tuple = rt_make_tuple(slice_len as i64, (*src).elem_tag);
        let new_tuple_obj = new_tuple as *mut crate::object::TupleObj;

        if slice_len > 0 {
            let src_data = (*src).data.as_ptr();
            let dst_data = (*new_tuple_obj).data.as_mut_ptr();

            // Copy element pointers (shallow copy)
            for i in 0..slice_len {
                *dst_data.add(i) = *src_data.add(start as usize + i);
            }
        }

        new_tuple
    }
}

/// Slice a tuple and return as a list: used for starred unpacking
/// In Python, `a, *rest = (1, 2, 3)` makes rest a list, not a tuple
/// Negative indices are supported
/// Uses i64::MIN as sentinel for "default start" (0) and i64::MAX for "default end" (len)
/// Returns: pointer to new allocated ListObj (shallow copy of elements)
#[no_mangle]
pub extern "C" fn rt_tuple_slice_to_list(tuple: *mut Obj, start: i64, end: i64) -> *mut Obj {
    use crate::list::rt_make_list;

    if tuple.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_slice_to_list");
        let src = tuple as *mut crate::object::TupleObj;
        let len = (*src).len as i64;

        // Normalize indices using shared utility (step=1 for simple slice)
        let (start, end) = normalize_slice_indices(start, end, len, 1);
        let slice_len = slice_length(start, end);

        // Create new list with capacity for slice_len elements, preserving elem_tag
        let new_list = rt_make_list(slice_len as i64, (*src).elem_tag);
        let new_list_obj = new_list as *mut crate::object::ListObj;

        if slice_len > 0 {
            let src_data = (*src).data.as_ptr();
            let dst_data = (*new_list_obj).data;

            // Copy element pointers (shallow copy)
            for i in 0..slice_len {
                *dst_data.add(i) = *src_data.add(start as usize + i);
            }
            // Set the actual length
            (*new_list_obj).len = slice_len;
        }

        new_list
    }
}

/// Compare two tuples for equality
/// Handles heterogeneous tuples by dispatching based on element type at runtime
/// Returns 1 if equal, 0 if not equal
#[no_mangle]
pub extern "C" fn rt_tuple_eq(a: *mut Obj, b: *mut Obj) -> i8 {
    use crate::object::{TupleObj, ELEM_HEAP_OBJ, ELEM_RAW_BOOL, ELEM_RAW_INT};

    // Both null => equal
    if a.is_null() && b.is_null() {
        return 1;
    }
    // One null => not equal
    if a.is_null() || b.is_null() {
        return 0;
    }

    unsafe {
        let tuple_a = a as *mut TupleObj;
        let tuple_b = b as *mut TupleObj;

        // Compare lengths
        if (*tuple_a).len != (*tuple_b).len {
            return 0;
        }

        let len = (*tuple_a).len;

        // Empty tuples are equal
        if len == 0 {
            return 1;
        }

        let data_a = (*tuple_a).data.as_ptr();
        let data_b = (*tuple_b).data.as_ptr();
        let elem_tag_a = (*tuple_a).elem_tag;
        let elem_tag_b = (*tuple_b).elem_tag;

        // Compare each element
        for i in 0..len {
            let val_a = *data_a.add(i);
            let val_b = *data_b.add(i);

            // Determine element types for comparison
            // If both tuples have same element tag, use optimized path
            if elem_tag_a == elem_tag_b {
                match elem_tag_a {
                    ELEM_RAW_INT => {
                        // Raw integers
                        if val_a as i64 != val_b as i64 {
                            return 0;
                        }
                    }
                    ELEM_RAW_BOOL => {
                        // Raw bools
                        if val_a as i8 != val_b as i8 {
                            return 0;
                        }
                    }
                    ELEM_HEAP_OBJ => {
                        // Heap objects - need runtime type dispatch
                        if !compare_heap_objects(val_a, val_b) {
                            return 0;
                        }
                    }
                    _ => {
                        // Unknown tag, fall back to pointer comparison
                        if val_a != val_b {
                            return 0;
                        }
                    }
                }
            } else {
                // Mixed element tags - box raw values before comparing
                let boxed_a = box_if_raw(val_a, elem_tag_a);
                let boxed_b = box_if_raw(val_b, elem_tag_b);
                if !compare_heap_objects(boxed_a, boxed_b) {
                    return 0;
                }
            }
        }

        1
    }
}

/// Box a raw value as a heap object based on its elem_tag.
/// If already a heap object (ELEM_HEAP_OBJ), returns as-is.
#[inline]
unsafe fn box_if_raw(val: *mut Obj, elem_tag: u8) -> *mut Obj {
    use crate::object::{ELEM_RAW_BOOL, ELEM_RAW_INT};
    match elem_tag {
        ELEM_RAW_INT => crate::boxing::rt_box_int(val as i64),
        ELEM_RAW_BOOL => crate::boxing::rt_box_bool(val as i8),
        _ => val, // ELEM_HEAP_OBJ or unknown — already a heap pointer
    }
}

/// Compare two heap objects for equality
/// Returns true if equal, false otherwise
/// Both arguments must be valid heap object pointers (or null).
/// Use `box_if_raw` to convert raw values before calling this.
unsafe fn compare_heap_objects(a: *mut Obj, b: *mut Obj) -> bool {
    use crate::object::{BoolObj, FloatObj, IntObj, ObjHeader, TypeTagKind};
    use crate::string::rt_str_eq;

    // Both null => equal
    if a.is_null() && b.is_null() {
        return true;
    }
    // One null => not equal
    if a.is_null() || b.is_null() {
        return false;
    }

    // Both are heap objects - safe to dereference
    let header_a = a as *mut ObjHeader;
    let header_b = b as *mut ObjHeader;
    let type_a = (*header_a).type_tag;
    let type_b = (*header_b).type_tag;

    // Different types => check for numeric cross-type equality (CPython: 1 == 1.0 == True)
    if type_a != type_b {
        return crate::hash_table_utils::eq_hashable_obj(a, b);
    }

    match type_a {
        TypeTagKind::Int => {
            let int_a = a as *mut IntObj;
            let int_b = b as *mut IntObj;
            (*int_a).value == (*int_b).value
        }
        TypeTagKind::Float => {
            let float_a = a as *mut FloatObj;
            let float_b = b as *mut FloatObj;
            (*float_a).value == (*float_b).value
        }
        TypeTagKind::Bool => {
            let bool_a = a as *mut BoolObj;
            let bool_b = b as *mut BoolObj;
            (*bool_a).value == (*bool_b).value
        }
        TypeTagKind::Str => rt_str_eq(a, b) == 1,
        TypeTagKind::None => true, // All None values are equal
        TypeTagKind::Tuple => rt_tuple_eq(a, b) == 1, // Recursive for nested tuples
        _ => {
            // For other types, fall back to pointer comparison
            a == b
        }
    }
}

/// Tuple less-than comparison - returns i8 (bool)
/// Implements element-wise lexicographic comparison following Python semantics
#[no_mangle]
pub extern "C" fn rt_tuple_lt(a: *mut Obj, b: *mut Obj) -> i8 {
    unsafe {
        // Handle null cases
        if a.is_null() && b.is_null() {
            return 0; // null == null, not <
        }
        if a.is_null() {
            return 1; // null < non-null
        }
        if b.is_null() {
            return 0; // non-null not < null
        }

        let tuple_a = a as *mut crate::object::TupleObj;
        let tuple_b = b as *mut crate::object::TupleObj;
        let len_a = (*tuple_a).len;
        let len_b = (*tuple_b).len;
        let min_len = len_a.min(len_b);
        let elem_tag_a = (*tuple_a).elem_tag;
        let elem_tag_b = (*tuple_b).elem_tag;

        let data_a = (*tuple_a).data.as_ptr();
        let data_b = (*tuple_b).data.as_ptr();

        // Compare element-by-element
        for i in 0..min_len {
            let elem_a = *data_a.add(i);
            let elem_b = *data_b.add(i);

            use crate::sorted::compare_list_elements;
            let (cmp_a, cmp_b, cmp_tag) = if elem_tag_a == elem_tag_b {
                (elem_a, elem_b, elem_tag_a)
            } else {
                let boxed_a = box_if_raw(elem_a, elem_tag_a);
                let boxed_b = box_if_raw(elem_b, elem_tag_b);
                (boxed_a, boxed_b, crate::object::ELEM_HEAP_OBJ)
            };
            match compare_list_elements(cmp_a, cmp_b, cmp_tag) {
                std::cmp::Ordering::Less => return 1,    // a < b
                std::cmp::Ordering::Greater => return 0, // a > b
                std::cmp::Ordering::Equal => continue,   // check next element
            }
        }

        // All compared elements are equal - shorter tuple is less
        if len_a < len_b {
            1
        } else {
            0
        }
    }
}

/// Tuple less-than-or-equal comparison - returns i8 (bool)
/// Implements element-wise lexicographic comparison following Python semantics
#[no_mangle]
pub extern "C" fn rt_tuple_lte(a: *mut Obj, b: *mut Obj) -> i8 {
    unsafe {
        // Handle null cases
        if a.is_null() && b.is_null() {
            return 1; // null == null, so <=
        }
        if a.is_null() {
            return 1; // null < non-null, so <=
        }
        if b.is_null() {
            return 0; // non-null not <= null
        }

        let tuple_a = a as *mut crate::object::TupleObj;
        let tuple_b = b as *mut crate::object::TupleObj;
        let len_a = (*tuple_a).len;
        let len_b = (*tuple_b).len;
        let min_len = len_a.min(len_b);
        let elem_tag_a = (*tuple_a).elem_tag;
        let elem_tag_b = (*tuple_b).elem_tag;

        let data_a = (*tuple_a).data.as_ptr();
        let data_b = (*tuple_b).data.as_ptr();

        // Compare element-by-element
        for i in 0..min_len {
            let elem_a = *data_a.add(i);
            let elem_b = *data_b.add(i);

            use crate::sorted::compare_list_elements;
            let (cmp_a, cmp_b, cmp_tag) = if elem_tag_a == elem_tag_b {
                (elem_a, elem_b, elem_tag_a)
            } else {
                let boxed_a = box_if_raw(elem_a, elem_tag_a);
                let boxed_b = box_if_raw(elem_b, elem_tag_b);
                (boxed_a, boxed_b, crate::object::ELEM_HEAP_OBJ)
            };
            match compare_list_elements(cmp_a, cmp_b, cmp_tag) {
                std::cmp::Ordering::Less => return 1,    // a < b, so <=
                std::cmp::Ordering::Greater => return 0, // a > b, so not <=
                std::cmp::Ordering::Equal => continue,   // check next element
            }
        }

        // All compared elements are equal - shorter or equal length satisfies <=
        if len_a <= len_b {
            1
        } else {
            0
        }
    }
}

/// Tuple greater-than comparison - returns i8 (bool)
/// Implements element-wise lexicographic comparison following Python semantics
#[no_mangle]
pub extern "C" fn rt_tuple_gt(a: *mut Obj, b: *mut Obj) -> i8 {
    unsafe {
        // Handle null cases
        if a.is_null() && b.is_null() {
            return 0; // null == null, not >
        }
        if a.is_null() {
            return 0; // null < non-null, not >
        }
        if b.is_null() {
            return 1; // non-null > null
        }

        let tuple_a = a as *mut crate::object::TupleObj;
        let tuple_b = b as *mut crate::object::TupleObj;
        let len_a = (*tuple_a).len;
        let len_b = (*tuple_b).len;
        let min_len = len_a.min(len_b);
        let elem_tag_a = (*tuple_a).elem_tag;
        let elem_tag_b = (*tuple_b).elem_tag;

        let data_a = (*tuple_a).data.as_ptr();
        let data_b = (*tuple_b).data.as_ptr();

        // Compare element-by-element
        for i in 0..min_len {
            let elem_a = *data_a.add(i);
            let elem_b = *data_b.add(i);

            use crate::sorted::compare_list_elements;
            let (cmp_a, cmp_b, cmp_tag) = if elem_tag_a == elem_tag_b {
                (elem_a, elem_b, elem_tag_a)
            } else {
                let boxed_a = box_if_raw(elem_a, elem_tag_a);
                let boxed_b = box_if_raw(elem_b, elem_tag_b);
                (boxed_a, boxed_b, crate::object::ELEM_HEAP_OBJ)
            };
            match compare_list_elements(cmp_a, cmp_b, cmp_tag) {
                std::cmp::Ordering::Less => return 0,    // a < b, not >
                std::cmp::Ordering::Greater => return 1, // a > b
                std::cmp::Ordering::Equal => continue,   // check next element
            }
        }

        // All compared elements are equal - longer tuple is greater
        if len_a > len_b {
            1
        } else {
            0
        }
    }
}

/// Tuple greater-than-or-equal comparison - returns i8 (bool)
/// Implements element-wise lexicographic comparison following Python semantics
#[no_mangle]
pub extern "C" fn rt_tuple_gte(a: *mut Obj, b: *mut Obj) -> i8 {
    unsafe {
        // Handle null cases
        if a.is_null() && b.is_null() {
            return 1; // null == null, so >=
        }
        if a.is_null() {
            return 0; // null < non-null, not >=
        }
        if b.is_null() {
            return 1; // non-null > null, so >=
        }

        let tuple_a = a as *mut crate::object::TupleObj;
        let tuple_b = b as *mut crate::object::TupleObj;
        let len_a = (*tuple_a).len;
        let len_b = (*tuple_b).len;
        let min_len = len_a.min(len_b);
        let elem_tag_a = (*tuple_a).elem_tag;
        let elem_tag_b = (*tuple_b).elem_tag;

        let data_a = (*tuple_a).data.as_ptr();
        let data_b = (*tuple_b).data.as_ptr();

        // Compare element-by-element
        for i in 0..min_len {
            let elem_a = *data_a.add(i);
            let elem_b = *data_b.add(i);

            use crate::sorted::compare_list_elements;
            let (cmp_a, cmp_b, cmp_tag) = if elem_tag_a == elem_tag_b {
                (elem_a, elem_b, elem_tag_a)
            } else {
                let boxed_a = box_if_raw(elem_a, elem_tag_a);
                let boxed_b = box_if_raw(elem_b, elem_tag_b);
                (boxed_a, boxed_b, crate::object::ELEM_HEAP_OBJ)
            };
            match compare_list_elements(cmp_a, cmp_b, cmp_tag) {
                std::cmp::Ordering::Less => return 0,    // a < b, not >=
                std::cmp::Ordering::Greater => return 1, // a > b, so >=
                std::cmp::Ordering::Equal => continue,   // check next element
            }
        }

        // All compared elements are equal - longer or equal length satisfies >=
        if len_a >= len_b {
            1
        } else {
            0
        }
    }
}

/// Get integer element from tuple, unboxing if necessary
/// Handles both raw integer storage and boxed IntObj storage transparently
#[no_mangle]
pub extern "C" fn rt_tuple_get_int(tuple: *mut Obj, index: i64) -> i64 {
    use crate::object::{IntObj, ELEM_HEAP_OBJ, ELEM_RAW_INT};

    if tuple.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_get_int");
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        let len = (*tuple_obj).len as i64;

        // Handle negative index
        let idx = if index < 0 { len + index } else { index };

        // Bounds check
        if idx < 0 || idx >= len {
            return 0;
        }

        let data_ptr = (*tuple_obj).data.as_ptr();
        let elem = *data_ptr.add(idx as usize);
        let elem_tag = (*tuple_obj).elem_tag;

        match elem_tag {
            ELEM_RAW_INT => {
                // Element is stored as raw i64
                elem as i64
            }
            ELEM_HEAP_OBJ => {
                // Element is boxed - unbox it
                if elem.is_null() {
                    return 0;
                }
                let int_obj = elem as *mut IntObj;
                (*int_obj).value
            }
            _ => {
                // Unknown tag, treat as raw
                elem as i64
            }
        }
    }
}

/// Get float element from tuple, unboxing if necessary
/// Handles both raw float storage (as bitcast i64) and boxed FloatObj storage
#[no_mangle]
pub extern "C" fn rt_tuple_get_float(tuple: *mut Obj, index: i64) -> f64 {
    use crate::object::{FloatObj, ELEM_HEAP_OBJ};

    if tuple.is_null() {
        return 0.0;
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_get_float");
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        let len = (*tuple_obj).len as i64;

        // Handle negative index
        let idx = if index < 0 { len + index } else { index };

        // Bounds check
        if idx < 0 || idx >= len {
            return 0.0;
        }

        let data_ptr = (*tuple_obj).data.as_ptr();
        let elem = *data_ptr.add(idx as usize);
        let elem_tag = (*tuple_obj).elem_tag;

        match elem_tag {
            ELEM_HEAP_OBJ => {
                // Element is boxed - unbox it
                if elem.is_null() {
                    return 0.0;
                }
                let float_obj = elem as *mut FloatObj;
                (*float_obj).value
            }
            _ => {
                // Raw storage: element is f64 bitcast to pointer
                f64::from_bits(elem as u64)
            }
        }
    }
}

/// Get bool element from tuple, unboxing if necessary
/// Handles both raw bool storage (as i8 cast to pointer) and boxed BoolObj storage
#[no_mangle]
pub extern "C" fn rt_tuple_get_bool(tuple: *mut Obj, index: i64) -> i8 {
    use crate::object::{BoolObj, ELEM_HEAP_OBJ, ELEM_RAW_BOOL};

    if tuple.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_get_bool");
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        let len = (*tuple_obj).len as i64;

        // Handle negative index
        let idx = if index < 0 { len + index } else { index };

        // Bounds check
        if idx < 0 || idx >= len {
            return 0;
        }

        let data_ptr = (*tuple_obj).data.as_ptr();
        let elem = *data_ptr.add(idx as usize);
        let elem_tag = (*tuple_obj).elem_tag;

        match elem_tag {
            ELEM_RAW_BOOL => {
                // Element is stored as raw i8
                elem as i8
            }
            ELEM_HEAP_OBJ => {
                // Element is boxed - unbox it
                if elem.is_null() {
                    return 0;
                }
                let bool_obj = elem as *mut BoolObj;
                if (*bool_obj).value {
                    1
                } else {
                    0
                }
            }
            _ => {
                // Unknown tag, treat as raw
                elem as i8
            }
        }
    }
}

/// Find minimum element in an integer tuple
#[no_mangle]
pub extern "C" fn rt_tuple_min_int(tuple: *mut Obj) -> i64 {
    use crate::minmax_utils::find_extremum_int;
    if tuple.is_null() {
        return 0;
    }
    unsafe {
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        if (*tuple_obj).len == 0 {
            let msg = b"min() arg is an empty sequence";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
        find_extremum_int(
            (*tuple_obj).data.as_ptr() as *const usize,
            (*tuple_obj).len,
            true,
        )
    }
}

/// Find maximum element in an integer tuple
#[no_mangle]
pub extern "C" fn rt_tuple_max_int(tuple: *mut Obj) -> i64 {
    use crate::minmax_utils::find_extremum_int;
    if tuple.is_null() {
        return 0;
    }
    unsafe {
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        if (*tuple_obj).len == 0 {
            let msg = b"max() arg is an empty sequence";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
        find_extremum_int(
            (*tuple_obj).data.as_ptr() as *const usize,
            (*tuple_obj).len,
            false,
        )
    }
}

/// Find minimum element in a float tuple
#[no_mangle]
pub extern "C" fn rt_tuple_min_float(tuple: *mut Obj) -> f64 {
    use crate::minmax_utils::find_extremum_float;
    if tuple.is_null() {
        return 0.0;
    }
    unsafe {
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        if (*tuple_obj).len == 0 {
            let msg = b"min() arg is an empty sequence";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
        find_extremum_float(
            (*tuple_obj).data.as_ptr() as *const usize,
            (*tuple_obj).len,
            true,
        )
    }
}

/// Find maximum element in a float tuple
#[no_mangle]
pub extern "C" fn rt_tuple_max_float(tuple: *mut Obj) -> f64 {
    use crate::minmax_utils::find_extremum_float;
    if tuple.is_null() {
        return 0.0;
    }
    unsafe {
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        if (*tuple_obj).len == 0 {
            let msg = b"max() arg is an empty sequence";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
        find_extremum_float(
            (*tuple_obj).data.as_ptr() as *const usize,
            (*tuple_obj).len,
            false,
        )
    }
}

// Type alias for key function pointer
type KeyFn = extern "C" fn(*mut Obj) -> *mut Obj;

/// Find minimum element in a tuple with key function
/// elem_tag: element storage type (0=ELEM_HEAP_OBJ, 1=ELEM_RAW_INT, 2=ELEM_RAW_BOOL)
///           Used to box raw elements before passing to key function
#[no_mangle]
pub extern "C" fn rt_tuple_min_with_key(tuple: *mut Obj, key_fn: KeyFn, elem_tag: i64) -> *mut Obj {
    unsafe { find_tuple_extremum_with_key(tuple, key_fn, elem_tag, true) }
}

/// Find maximum element in a tuple with key function
/// elem_tag: element storage type (0=ELEM_HEAP_OBJ, 1=ELEM_RAW_INT, 2=ELEM_RAW_BOOL)
///           Used to box raw elements before passing to key function
#[no_mangle]
pub extern "C" fn rt_tuple_max_with_key(tuple: *mut Obj, key_fn: KeyFn, elem_tag: i64) -> *mut Obj {
    unsafe { find_tuple_extremum_with_key(tuple, key_fn, elem_tag, false) }
}

/// Find extremum (min or max) element in a tuple using a key function
unsafe fn find_tuple_extremum_with_key(
    tuple: *mut Obj,
    key_fn: KeyFn,
    elem_tag: i64,
    is_min: bool,
) -> *mut Obj {
    use crate::object::ELEM_RAW_INT;
    use crate::sorted::compare_key_values;

    if tuple.is_null() {
        return std::ptr::null_mut();
    }

    let tuple_obj = tuple as *mut crate::object::TupleObj;
    let len = (*tuple_obj).len;

    if len == 0 {
        let msg = if is_min {
            b"min() arg is an empty sequence"
        } else {
            b"max() arg is an empty sequence"
        };
        crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
    }

    let data = (*tuple_obj).data.as_ptr();

    // Apply key function to first element
    let mut extremum_elem = *data;
    // Box raw elements before passing to key function
    let boxed_elem = if elem_tag == ELEM_RAW_INT as i64 {
        crate::boxing::rt_box_int(extremum_elem as i64)
    } else {
        extremum_elem
    };
    let mut extremum_key = key_fn(boxed_elem);

    // Compare remaining elements
    for i in 1..len {
        let elem = *data.add(i);
        // Box raw elements before passing to key function
        let boxed_elem = if elem_tag == ELEM_RAW_INT as i64 {
            crate::boxing::rt_box_int(elem as i64)
        } else {
            elem
        };
        let key = key_fn(boxed_elem);

        let cmp = compare_key_values(key, extremum_key);
        let is_better = if is_min {
            cmp == std::cmp::Ordering::Less
        } else {
            cmp == std::cmp::Ordering::Greater
        };

        if is_better {
            extremum_elem = elem;
            extremum_key = key;
        }
    }

    extremum_elem // Return original element, not key!
}

/// Slice a tuple with step: tuple[start:end:step]
/// Uses i64::MIN as sentinel for "default start" and i64::MAX for "default end"
/// Defaults depend on step direction:
///   - Positive step: start=0, end=len
///   - Negative step: start=len-1, end=-1 (before index 0)
///
/// Returns: pointer to new allocated TupleObj (shallow copy)
#[no_mangle]
pub extern "C" fn rt_tuple_slice_step(
    tuple: *mut Obj,
    start: i64,
    end: i64,
    step: i64,
) -> *mut Obj {
    if tuple.is_null() || step == 0 {
        return rt_make_tuple(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let src = tuple as *mut crate::object::TupleObj;
        let len = (*src).len as i64;

        // Normalize indices using shared utility
        let (start, end) = normalize_slice_indices(start, end, len, step);

        // Collect indices using shared utility
        let indices = collect_step_indices(start, end, step);
        let result_len = indices.len();
        let new_tuple = rt_make_tuple(result_len as i64, (*src).elem_tag);
        let new_tuple_obj = new_tuple as *mut crate::object::TupleObj;

        if result_len > 0 {
            let src_data = (*src).data.as_ptr();
            let dst_data = (*new_tuple_obj).data.as_mut_ptr();

            for (dst_i, src_i) in indices.iter().enumerate() {
                *dst_data.add(dst_i) = *src_data.add(*src_i);
            }
        }

        new_tuple
    }
}

/// Create a tuple from a list
/// Returns: pointer to new TupleObj
#[no_mangle]
pub extern "C" fn rt_tuple_from_list(list: *mut Obj) -> *mut Obj {
    use crate::object::ListObj;

    if list.is_null() {
        return rt_make_tuple(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len;
        let elem_tag = (*list_obj).elem_tag;

        let tuple = rt_make_tuple(len as i64, elem_tag);
        let tuple_obj = tuple as *mut crate::object::TupleObj;

        if len > 0 {
            let src_data = (*list_obj).data;
            let dst_data = (*tuple_obj).data.as_mut_ptr();

            for i in 0..len {
                *dst_data.add(i) = *src_data.add(i);
            }
        }

        tuple
    }
}

/// Create a tuple from a string (each character becomes an element)
/// Returns: pointer to new TupleObj
#[no_mangle]
pub extern "C" fn rt_tuple_from_str(str_obj: *mut Obj) -> *mut Obj {
    use crate::object::StrObj;
    use crate::string::rt_make_str;

    if str_obj.is_null() {
        return rt_make_tuple(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let str = str_obj as *mut StrObj;
        let len = (*str).len;
        let data = (*str).data.as_ptr();

        let tuple = rt_make_tuple(len as i64, ELEM_HEAP_OBJ);

        for i in 0..len {
            let ch = *data.add(i);
            // Create single-character string
            let char_str = rt_make_str(&ch, 1);
            rt_tuple_set(tuple, i as i64, char_str);
        }

        tuple
    }
}

/// Create a tuple from a range
/// Returns: pointer to new TupleObj
#[no_mangle]
pub extern "C" fn rt_tuple_from_range(start: i64, stop: i64, step: i64) -> *mut Obj {
    use crate::object::ELEM_RAW_INT;

    if step == 0 {
        return rt_make_tuple(0, ELEM_RAW_INT);
    }

    let len = if step > 0 {
        if stop > start {
            ((stop - start + step - 1) / step) as usize
        } else {
            0
        }
    } else if start > stop {
        ((start - stop - step - 1) / (-step)) as usize
    } else {
        0
    };

    let tuple = rt_make_tuple(len as i64, ELEM_RAW_INT);

    unsafe {
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        let data = (*tuple_obj).data.as_mut_ptr();

        let mut current = start;
        for i in 0..len {
            *data.add(i) = current as *mut Obj;
            current += step;
        }
    }

    tuple
}

/// Create a tuple by consuming an iterator
/// Returns: pointer to new TupleObj
#[no_mangle]
pub extern "C" fn rt_tuple_from_iter(iter: *mut Obj) -> *mut Obj {
    use crate::iterator::rt_iter_next_no_exc;
    use crate::list::{rt_list_push, rt_make_list};

    if iter.is_null() {
        return rt_make_tuple(0, ELEM_HEAP_OBJ);
    }

    // First collect into a list (since we don't know the size)
    let list = rt_make_list(8, ELEM_HEAP_OBJ);

    loop {
        let elem = rt_iter_next_no_exc(iter);
        if elem.is_null() {
            break;
        }
        rt_list_push(list, elem);
    }

    // Convert list to tuple
    rt_tuple_from_list(list)
}

/// Create a tuple from a set
/// Returns: pointer to new TupleObj
#[no_mangle]
pub extern "C" fn rt_tuple_from_set(set: *mut Obj) -> *mut Obj {
    use crate::set::rt_set_to_list;

    // First convert set to list, then list to tuple
    let list = rt_set_to_list(set);
    rt_tuple_from_list(list)
}

/// Create a tuple from a dict (keys only)
/// Returns: pointer to new TupleObj
#[no_mangle]
pub extern "C" fn rt_tuple_from_dict(dict: *mut Obj) -> *mut Obj {
    use crate::dict::rt_dict_keys;

    // First get keys as list, then convert to tuple
    let list = rt_dict_keys(dict, crate::object::ELEM_HEAP_OBJ);
    rt_tuple_from_list(list)
}

/// Concatenate two tuples into a new tuple
/// Used for combining extra positional args with list-unpacked varargs
/// Returns: pointer to new TupleObj containing elements from tuple1 followed by tuple2
#[no_mangle]
pub extern "C" fn rt_tuple_concat(tuple1: *mut Obj, tuple2: *mut Obj) -> *mut Obj {
    use crate::object::TupleObj;

    // Handle null cases
    if tuple1.is_null() && tuple2.is_null() {
        return rt_make_tuple(0, ELEM_HEAP_OBJ);
    }
    if tuple1.is_null() {
        return tuple2;
    }
    if tuple2.is_null() {
        return tuple1;
    }

    unsafe {
        let t1 = tuple1 as *mut TupleObj;
        let t2 = tuple2 as *mut TupleObj;
        let len1 = (*t1).len;
        let len2 = (*t2).len;
        let total_len = len1 + len2;

        // Use elem_tag from the first tuple (or HEAP_OBJ if first is empty)
        let elem_tag = if len1 > 0 {
            (*t1).elem_tag
        } else {
            (*t2).elem_tag
        };

        // Create new tuple
        let result = rt_make_tuple(total_len as i64, elem_tag);
        let result_obj = result as *mut TupleObj;

        // Copy elements from tuple1
        if len1 > 0 {
            let src_data = (*t1).data.as_ptr();
            let dst_data = (*result_obj).data.as_mut_ptr();
            for i in 0..len1 {
                *dst_data.add(i) = *src_data.add(i);
            }
        }

        // Copy elements from tuple2
        if len2 > 0 {
            let src_data = (*t2).data.as_ptr();
            let dst_data = (*result_obj).data.as_mut_ptr();
            for i in 0..len2 {
                *dst_data.add(len1 + i) = *src_data.add(i);
            }
        }

        result
    }
}

/// Find index of value in tuple
/// Raises ValueError if not found
/// Returns: index (0-based)
#[no_mangle]
pub extern "C" fn rt_tuple_index(tuple: *mut Obj, value: *mut Obj) -> i64 {
    if tuple.is_null() {
        unsafe {
            let msg = b"tuple.index(x): x not in tuple";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_index");
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        let len = (*tuple_obj).len;
        let data = (*tuple_obj).data.as_ptr();

        // Search for value using value equality
        for i in 0..len {
            let elem = *data.add(i);
            if crate::ops::rt_obj_eq(elem, value) == 1 {
                return i as i64;
            }
        }

        // Not found - raise ValueError
        let msg = b"tuple.index(x): x not in tuple";
        crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
    }
}

/// Count occurrences of value in tuple
/// Returns: count
#[no_mangle]
pub extern "C" fn rt_tuple_count(tuple: *mut Obj, value: *mut Obj) -> i64 {
    if tuple.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_count");
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        let len = (*tuple_obj).len;
        let data = (*tuple_obj).data.as_ptr();

        let mut count = 0i64;
        // Count occurrences using value equality
        for i in 0..len {
            let elem = *data.add(i);
            if crate::ops::rt_obj_eq(elem, value) == 1 {
                count += 1;
            }
        }

        count
    }
}

/// Maximum number of arguments supported for *args forwarding via indirect call.
const MAX_CALL_ARGS: usize = 8;

/// Call a function pointer with arguments unpacked from a tuple.
/// Used for *args forwarding in decorator wrappers: `func(*args)`.
///
/// All arguments are passed as i64 (raw ints stay as i64, heap objects as pointers cast to i64).
/// The function pointer must use the SystemV calling convention.
#[no_mangle]
pub extern "C" fn rt_call_with_tuple_args(func_ptr: i64, args_tuple: *mut Obj) -> i64 {
    use crate::object::TupleObj;

    if func_ptr == 0 {
        return 0;
    }

    unsafe {
        let len = if args_tuple.is_null() {
            0
        } else {
            let tuple_obj = args_tuple as *mut TupleObj;
            (*tuple_obj).len
        };

        // Extract arguments from tuple
        let mut call_args = [0i64; MAX_CALL_ARGS];
        if !args_tuple.is_null() && len > 0 {
            let tuple_obj = args_tuple as *mut TupleObj;
            let data_ptr = (*tuple_obj).data.as_ptr();
            for (slot, i) in (0..len.min(MAX_CALL_ARGS)).enumerate() {
                call_args[slot] = *data_ptr.add(i) as i64;
            }
        }

        // Dispatch based on argument count
        type F0 = extern "C" fn() -> i64;
        type F1 = extern "C" fn(i64) -> i64;
        type F2 = extern "C" fn(i64, i64) -> i64;
        type F3 = extern "C" fn(i64, i64, i64) -> i64;
        type F4 = extern "C" fn(i64, i64, i64, i64) -> i64;
        type F5 = extern "C" fn(i64, i64, i64, i64, i64) -> i64;
        type F6 = extern "C" fn(i64, i64, i64, i64, i64, i64) -> i64;
        type F7 = extern "C" fn(i64, i64, i64, i64, i64, i64, i64) -> i64;
        type F8 = extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64) -> i64;

        match len {
            0 => {
                let f: F0 = std::mem::transmute(func_ptr as usize);
                f()
            }
            1 => {
                let f: F1 = std::mem::transmute(func_ptr as usize);
                f(call_args[0])
            }
            2 => {
                let f: F2 = std::mem::transmute(func_ptr as usize);
                f(call_args[0], call_args[1])
            }
            3 => {
                let f: F3 = std::mem::transmute(func_ptr as usize);
                f(call_args[0], call_args[1], call_args[2])
            }
            4 => {
                let f: F4 = std::mem::transmute(func_ptr as usize);
                f(call_args[0], call_args[1], call_args[2], call_args[3])
            }
            5 => {
                let f: F5 = std::mem::transmute(func_ptr as usize);
                f(
                    call_args[0],
                    call_args[1],
                    call_args[2],
                    call_args[3],
                    call_args[4],
                )
            }
            6 => {
                let f: F6 = std::mem::transmute(func_ptr as usize);
                f(
                    call_args[0],
                    call_args[1],
                    call_args[2],
                    call_args[3],
                    call_args[4],
                    call_args[5],
                )
            }
            7 => {
                let f: F7 = std::mem::transmute(func_ptr as usize);
                f(
                    call_args[0],
                    call_args[1],
                    call_args[2],
                    call_args[3],
                    call_args[4],
                    call_args[5],
                    call_args[6],
                )
            }
            8 => {
                let f: F8 = std::mem::transmute(func_ptr as usize);
                f(
                    call_args[0],
                    call_args[1],
                    call_args[2],
                    call_args[3],
                    call_args[4],
                    call_args[5],
                    call_args[6],
                    call_args[7],
                )
            }
            _ => 0, // Unsupported arity
        }
    }
}
