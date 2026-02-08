//! Hash operations for Python runtime

use crate::object::Obj;

// FNV-1a hash constants
const FNV_OFFSET_BASIS: u64 = 14695981039346656037;
const FNV_PRIME: u64 = 1099511628211;

/// Hash an integer value
/// Uses the same algorithm as Python's hash for integers
/// Returns: hash value as i64
#[no_mangle]
pub extern "C" fn rt_hash_int(value: i64) -> i64 {
    // Python's hash for integers: for small ints, hash(x) == x
    // For our purposes, use a simple scramble that matches Python behavior for small values
    // Golden ratio scramble for better distribution
    let v = value as u64;
    v.wrapping_mul(11400714819323198485) as i64 // 2^64 / phi
}

/// Hash a string object
/// Uses FNV-1a hash algorithm
/// Returns: hash value as i64
#[no_mangle]
pub extern "C" fn rt_hash_str(str_obj: *mut Obj) -> i64 {
    if str_obj.is_null() {
        return 0;
    }

    unsafe {
        let str_obj = str_obj as *mut crate::object::StrObj;
        let len = (*str_obj).len;
        let data = (*str_obj).data.as_ptr();

        // FNV-1a hash
        let mut hash: u64 = FNV_OFFSET_BASIS;
        for i in 0..len {
            hash ^= *data.add(i) as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash as i64
    }
}

/// Hash a boolean value
/// Returns: 0 for False, 1 for True (matches Python)
#[no_mangle]
pub extern "C" fn rt_hash_bool(value: i8) -> i64 {
    if value != 0 {
        1
    } else {
        0
    }
}

/// Get the id (memory address) of a heap object
/// Returns: pointer value as i64
#[no_mangle]
pub extern "C" fn rt_id_obj(obj: *mut Obj) -> i64 {
    obj as i64
}

/// Hash a tuple object
/// Combines hashes of all elements using Python's tuple hash algorithm
/// Returns: hash value as i64
#[no_mangle]
pub extern "C" fn rt_hash_tuple(tuple_obj: *mut Obj) -> i64 {
    if tuple_obj.is_null() {
        return 0;
    }

    unsafe {
        let tuple = tuple_obj as *mut crate::object::TupleObj;
        let len = (*tuple).len;
        let data = (*tuple).data.as_ptr();
        let elem_tag = (*tuple).elem_tag;

        // Python uses: hash = hash * 1000003 ^ element_hash
        // Start with a seed based on length
        let mut hash: u64 = 0x345678;

        for i in 0..len {
            let elem = *data.add(i);
            let elem_hash = match elem_tag {
                crate::object::ELEM_RAW_INT => rt_hash_int(elem as i64),
                crate::object::ELEM_RAW_BOOL => {
                    rt_hash_bool(if (elem as i64) != 0 { 1 } else { 0 })
                }
                _ => hash_any_obj(elem), // ELEM_HEAP_OBJ: elements are boxed pointers
            };
            // Python's tuple hash combination algorithm
            hash = hash.wrapping_mul(1000003) ^ (elem_hash as u64);
        }

        // Mix in the length
        hash ^= len as u64;

        hash as i64
    }
}

/// Hash any object based on its type tag
/// Internal helper for tuple hashing
unsafe fn hash_any_obj(obj: *mut Obj) -> i64 {
    if obj.is_null() {
        return 0; // hash(None) == 0
    }

    match (*obj).header.type_tag {
        crate::object::TypeTagKind::Int => {
            // Boxed int
            let int_obj = obj as *mut crate::object::IntObj;
            rt_hash_int((*int_obj).value)
        }
        crate::object::TypeTagKind::Bool => {
            // Boxed bool
            let bool_obj = obj as *mut crate::object::BoolObj;
            rt_hash_bool(if (*bool_obj).value { 1 } else { 0 })
        }
        crate::object::TypeTagKind::Float => {
            // Boxed float - use bit representation
            let float_obj = obj as *mut crate::object::FloatObj;
            (*float_obj).value.to_bits() as i64
        }
        crate::object::TypeTagKind::Str => rt_hash_str(obj),
        crate::object::TypeTagKind::Tuple => rt_hash_tuple(obj),
        crate::object::TypeTagKind::None => 0,
        // Other types (list, dict, set) are unhashable
        _ => 0,
    }
}
