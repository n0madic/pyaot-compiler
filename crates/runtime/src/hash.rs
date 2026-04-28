//! Hash operations for Python runtime

use crate::object::Obj;
use pyaot_core_defs::Value;

// FNV-1a hash constants
const FNV_OFFSET_BASIS: u64 = 14695981039346656037;
const FNV_PRIME: u64 = 1099511628211;

/// Hash an integer value, matching CPython's behavior.
/// CPython: hash(n) == n for all integers except hash(-1) == -2
/// (since -1 is reserved as an error indicator in CPython's C API).
/// Returns: hash value as i64
#[no_mangle]
pub extern "C" fn rt_hash_int(value: i64) -> i64 {
    if value == -1 {
        -2
    } else {
        value
    }
}

/// Hash a string object
/// Uses FNV-1a hash algorithm
/// Returns: hash value as i64
pub fn rt_hash_str(str_obj: *mut Obj) -> i64 {
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
        let mut result = hash as i64;
        // CPython reserves -1 as an error sentinel; map it to -2
        if result == -1 {
            result = -2;
        }
        result
    }
}
#[export_name = "rt_hash_str"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_hash_str_abi(str_obj: Value) -> i64 {
    rt_hash_str(str_obj.unwrap_ptr())
}


/// Hash a boolean value
/// Returns the same value as hashing the equivalent integer (True == 1, False == 0),
/// satisfying the CPython invariant hash(True) == hash(1) and hash(False) == hash(0).
#[no_mangle]
pub extern "C" fn rt_hash_bool(value: i8) -> i64 {
    rt_hash_int(if value != 0 { 1 } else { 0 })
}

/// Get the id (memory address) of a heap object
/// Returns: pointer value as i64
pub fn rt_id_obj(obj: *mut Obj) -> i64 {
    obj as i64
}
#[export_name = "rt_id_obj"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_id_obj_abi(obj: Value) -> i64 {
    rt_id_obj(obj.unwrap_ptr())
}


/// Hash a tuple object
/// Combines hashes of all elements using Python's tuple hash algorithm
/// Returns: hash value as i64
pub fn rt_hash_tuple(tuple_obj: *mut Obj) -> i64 {
    if tuple_obj.is_null() {
        return 0;
    }

    unsafe {
        let tuple = tuple_obj as *mut crate::object::TupleObj;
        let len = (*tuple).len;
        let data = (*tuple).data.as_ptr();

        // Python uses: hash = hash * 1000003 ^ element_hash
        // Start with a seed based on length
        let mut hash: u64 = 0x345678;

        for i in 0..len {
            let val = *data.add(i);
            let elem_hash = if val.is_int() {
                rt_hash_int(val.unwrap_int())
            } else if val.is_bool() {
                rt_hash_bool(if val.unwrap_bool() { 1 } else { 0 })
            } else if val.is_none() {
                0 // hash(None) == 0
            } else {
                hash_any_obj(val.0 as *mut Obj)
            };
            // Python's tuple hash combination algorithm
            hash = hash.wrapping_mul(1000003) ^ (elem_hash as u64);
        }

        // Mix in the length
        hash ^= len as u64;

        let mut result = hash as i64;
        // CPython reserves -1 as an error sentinel; map it to -2
        if result == -1 {
            result = -2;
        }
        result
    }
}
#[export_name = "rt_hash_tuple"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_hash_tuple_abi(tuple_obj: Value) -> i64 {
    rt_hash_tuple(tuple_obj.unwrap_ptr())
}


/// Hash any object based on its type tag
/// Internal helper for tuple hashing
unsafe fn hash_any_obj(obj: *mut Obj) -> i64 {
    if obj.is_null() {
        return 0; // hash(None) == 0
    }

    // Check Value-tagged primitives before heap pointer dereference.
    let val = pyaot_core_defs::Value(obj as u64);
    if val.is_int() {
        return rt_hash_int(val.unwrap_int());
    }
    if val.is_bool() {
        return rt_hash_int(if val.unwrap_bool() { 1 } else { 0 });
    }
    if val.is_none() {
        return 0;
    }

    match (*obj).header.type_tag {
        crate::object::TypeTagKind::Float => {
            // Boxed float — integer-valued floats must hash identically to the equivalent int
            let float_obj = obj as *mut crate::object::FloatObj;
            let v = (*float_obj).value;
            if v == 0.0 {
                0 // hash(-0.0) == hash(0.0) == 0
            } else if v.fract() == 0.0 && v.is_finite() {
                // Integer-valued float: hash must equal hash of the equivalent integer
                // Only cast to i64 if within safe range to avoid overflow/UB
                if v >= i64::MIN as f64 && v <= i64::MAX as f64 {
                    rt_hash_int(v as i64)
                } else {
                    // Large integer-valued float outside i64 range: hash the bit representation
                    rt_hash_int(v.to_bits() as i64)
                }
            } else {
                // Non-integer float: use bit representation as input to the scramble
                rt_hash_int(v.to_bits() as i64)
            }
        }
        crate::object::TypeTagKind::Str => rt_hash_str(obj),
        crate::object::TypeTagKind::Tuple => rt_hash_tuple(obj),
        crate::object::TypeTagKind::None => 0,
        // Other types (list, dict, set) are unhashable
        _ => 0,
    }
}
