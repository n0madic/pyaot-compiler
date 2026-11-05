//! Hash operations for Python runtime.
//!
//! [`py_hash`] is the single source of `hash(x)`: it reproduces CPython's
//! numeric hash exactly (integers, bignums, bools and floats all reduce modulo
//! the Mersenne prime `2**61 - 1`, so `hash(10**30) == hash(1e30)` and equal
//! numbers hash equal across representations) and CPython's tuple hash
//! (xxHash-based, deterministic given the element hashes). `str`/`bytes`
//! hashes are FNV-1a (CPython's are SipHash with a per-process random key, so
//! no program can depend on their values). Class instances dispatch
//! `__hash__`; a class with `__eq__` but no `__hash__` is unhashable; any other
//! instance hashes by identity (CPython's `object.__hash__`).
//!
//! The hash tables (`hash_table_utils::hash_hashable_obj`) scramble this value
//! for bucket placement, so table equality and `hash()` can never disagree.

use crate::object::{BigIntObj, BytesObj, FloatObj, InstanceObj, Obj, StrObj, TypeTagKind};
use num_bigint::{BigInt, Sign};
use num_traits::ToPrimitive;
use pyaot_core_defs::Value;

// FNV-1a hash constants
const FNV_OFFSET_BASIS: u64 = 14695981039346656037;
const FNV_PRIME: u64 = 1099511628211;

/// CPython's `_PyHASH_MODULUS` (`2**61 - 1`, a Mersenne prime).
pub const HASH_MODULUS: u64 = (1u64 << 61) - 1;
const HASH_BITS: u32 = 61;
/// CPython's `_PyHASH_INF` (`hash(float('inf'))`).
const HASH_INF: i64 = 314159;
/// CPython's fixed `hash(None)` (3.12+).
pub const HASH_NONE: i64 = 0xFCA8_6420;

/// CPython reserves -1 as an error sentinel: a hash of -1 becomes -2.
#[inline]
fn fix_minus_one(h: i64) -> i64 {
    if h == -1 {
        -2
    } else {
        h
    }
}

/// `hash(n)` for a machine integer (CPython: `n mod (2**61-1)` with the sign
/// of `n`, `-1 → -2`).
#[inline]
pub fn py_hash_i64(value: i64) -> i64 {
    let m = (value.unsigned_abs() % HASH_MODULUS) as i64;
    fix_minus_one(if value < 0 { -m } else { m })
}

/// `hash(n)` for a bignum.
pub fn py_hash_bigint(value: &BigInt) -> i64 {
    let m = (value.magnitude() % HASH_MODULUS)
        .to_u64()
        .expect("residue fits in u64") as i64;
    fix_minus_one(if value.sign() == Sign::Minus { -m } else { m })
}

/// `frexp`: `v = m * 2**e` with `0.5 <= |m| < 1` (finite, non-zero `v`).
fn frexp(v: f64) -> (f64, i32) {
    let bits = v.to_bits();
    let exp_field = ((bits >> 52) & 0x7ff) as i32;
    if exp_field == 0 {
        // Subnormal: scale into the normal range first.
        let (m, e) = frexp(v * f64::from_bits(0x4350_0000_0000_0000)); // 2**54
        return (m, e - 54);
    }
    let e = exp_field - 1022;
    let m = f64::from_bits((bits & !(0x7ffu64 << 52)) | (1022u64 << 52));
    (m, e)
}

/// `hash(x)` for a float — CPython's `_Py_HashDouble`. `nan` hashes by the
/// identity of its object (`ident`), as in CPython 3.10+.
pub fn py_hash_f64(v: f64, ident: usize) -> i64 {
    if v.is_nan() {
        return py_hash_identity(ident);
    }
    if v.is_infinite() {
        return if v > 0.0 { HASH_INF } else { -HASH_INF };
    }
    if v == 0.0 {
        return 0;
    }
    let (mut m, mut e) = frexp(v);
    let mut sign = 1i64;
    if m < 0.0 {
        sign = -1;
        m = -m;
    }
    // Process 28 bits at a time, reducing modulo 2**61-1 as we go.
    let mut x: u64 = 0;
    while m != 0.0 {
        x = ((x << 28) & HASH_MODULUS) | (x >> (HASH_BITS - 28));
        m *= 268435456.0; // 2**28
        e -= 28;
        let y = m as u64;
        m -= y as f64;
        x += y;
        if x >= HASH_MODULUS {
            x -= HASH_MODULUS;
        }
    }
    // Multiply by 2**e modulo 2**61-1 (a rotation).
    let e = if e >= 0 {
        e % HASH_BITS as i32
    } else {
        HASH_BITS as i32 - 1 - ((-1 - e) % HASH_BITS as i32)
    } as u32;
    x = ((x << e) & HASH_MODULUS) | (x >> (HASH_BITS - e));
    fix_minus_one(x as i64 * sign)
}

/// CPython's default `object.__hash__` (`_Py_HashPointer`: the address rotated
/// right by 4 bits, so the always-zero alignment bits don't waste entropy).
pub fn py_hash_identity(ident: usize) -> i64 {
    fix_minus_one((ident as u64).rotate_right(4) as i64)
}

/// FNV-1a over raw bytes (the `str`/`bytes` hash).
#[inline]
fn fnv1a(data: &[u8]) -> i64 {
    let mut hash: u64 = FNV_OFFSET_BASIS;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    fix_minus_one(hash as i64)
}

/// The `hash()` of a value: `Ok(hash)`, or `Err(type name)` when the value is
/// unhashable (the caller raises `TypeError: unhashable type: '<name>'`).
///
/// # Safety
/// `obj` must be null, a tagged immediate, or a valid object pointer.
pub unsafe fn py_hash(obj: *mut Obj) -> Result<i64, String> {
    let v = Value(obj as u64);
    if obj.is_null() || v.is_none() {
        return Ok(HASH_NONE);
    }
    if v.is_int() {
        return Ok(py_hash_i64(v.unwrap_int()));
    }
    if v.is_bool() {
        return Ok(v.unwrap_bool() as i64);
    }
    if !v.is_ptr() {
        return Ok(py_hash_identity(obj as usize));
    }
    match (*obj).type_tag() {
        TypeTagKind::Float => Ok(py_hash_f64((*(obj as *mut FloatObj)).value, obj as usize)),
        TypeTagKind::BigInt => Ok(py_hash_bigint(&(*(obj as *mut BigIntObj)).value)),
        TypeTagKind::Str => {
            let s = obj as *mut StrObj;
            Ok(fnv1a(std::slice::from_raw_parts(
                (*s).data.as_ptr(),
                (*s).len,
            )))
        }
        TypeTagKind::Bytes => {
            let b = obj as *mut BytesObj;
            Ok(fnv1a(std::slice::from_raw_parts(
                (*b).data.as_ptr(),
                (*b).len,
            )))
        }
        TypeTagKind::Tuple => py_hash_tuple(obj),
        TypeTagKind::FrozenSet => Ok(fix_minus_one(
            crate::frozenset::frozenset_content_hash(obj) as i64
        )),
        TypeTagKind::None => Ok(HASH_NONE),
        TypeTagKind::Instance => py_hash_instance(obj),
        TypeTagKind::List => Err("list".into()),
        TypeTagKind::Dict => Err("dict".into()),
        TypeTagKind::DefaultDict => Err("collections.defaultdict".into()),
        TypeTagKind::Counter => Err("Counter".into()),
        TypeTagKind::Set => Err("set".into()),
        TypeTagKind::ByteArray => Err("bytearray".into()),
        TypeTagKind::Deque => Err("collections.deque".into()),
        // The set-like views define `__eq__` (so `__hash__` is None); a values
        // view keeps identity hashing (the default arm).
        TypeTagKind::DictKeys => Err("dict_keys".into()),
        TypeTagKind::DictItems => Err("dict_items".into()),
        // Everything else (files, closures, iterators, generators, …) has
        // identity hashing in CPython.
        _ => Ok(py_hash_identity(obj as usize)),
    }
}

/// `hash()` of a class instance: its `__hash__` (the result reduced like any
/// int); unhashable when the class defines `__eq__` without `__hash__`;
/// otherwise identity.
unsafe fn py_hash_instance(obj: *mut Obj) -> Result<i64, String> {
    if let Some(h) = crate::ops::try_hash_dunder(obj) {
        // CPython keeps a `__hash__` result that fits `Py_hash_t` as is (only
        // -1 → -2) and reduces a wider int like `hash(int)` does.
        let hv = Value(h as u64);
        if hv.is_int() {
            return Ok(fix_minus_one(hv.unwrap_int()));
        }
        if hv.is_bool() {
            return Ok(hv.unwrap_bool() as i64);
        }
        if hv.is_ptr() && !h.is_null() && (*h).type_tag() == TypeTagKind::BigInt {
            let big = &(*(h as *mut BigIntObj)).value;
            return Ok(match big.to_i64() {
                Some(i) => fix_minus_one(i),
                None => py_hash_bigint(big),
            });
        }
        crate::raise_exc!(
            crate::exceptions::ExceptionType::TypeError,
            "__hash__ method should return an integer"
        );
    }
    let class_id = (*(obj as *const InstanceObj)).class_id;
    if crate::ops::instance_defines_eq(class_id) {
        return Err(crate::ops::instance_class_name(class_id));
    }
    Ok(py_hash_identity(obj as usize))
}

/// [`py_hash`], raising `TypeError: unhashable type: '<name>'` for an
/// unhashable value.
///
/// # Safety
/// See [`py_hash`].
pub unsafe fn py_hash_or_raise(obj: *mut Obj) -> i64 {
    match py_hash(obj) {
        Ok(h) => h,
        Err(name) => crate::raise_exc!(
            crate::exceptions::ExceptionType::TypeError,
            "unhashable type: '{}'",
            name
        ),
    }
}

/// CPython's tuple hash (3.8+, xxHash-based), over the elements' hashes.
unsafe fn py_hash_tuple(tuple_obj: *mut Obj) -> Result<i64, String> {
    const XXPRIME_1: u64 = 11400714785074694791;
    const XXPRIME_2: u64 = 14029467366897019727;
    const XXPRIME_5: u64 = 2870177450012600261;
    let tuple = tuple_obj as *mut crate::object::TupleObj;
    let len = (*tuple).len;
    let data = (*tuple).data.as_ptr();
    let mut acc: u64 = XXPRIME_5;
    for i in 0..len {
        let lane = py_hash((*data.add(i)).0 as *mut Obj)? as u64;
        acc = acc.wrapping_add(lane.wrapping_mul(XXPRIME_2));
        acc = acc.rotate_left(31);
        acc = acc.wrapping_mul(XXPRIME_1);
    }
    acc = acc.wrapping_add((len as u64) ^ (XXPRIME_5 ^ 3527539));
    if acc == u64::MAX {
        return Ok(1546275796);
    }
    Ok(acc as i64)
}

/// Hash an integer value, matching CPython's `hash(int)`.
#[no_mangle]
pub extern "C" fn rt_hash_int(value: i64) -> i64 {
    py_hash_i64(value)
}

/// Hash a string object (FNV-1a; `-1 → -2`).
pub fn rt_hash_str(str_obj: *mut Obj) -> i64 {
    if str_obj.is_null() {
        return 0;
    }
    unsafe {
        let s = str_obj as *mut StrObj;
        fnv1a(std::slice::from_raw_parts((*s).data.as_ptr(), (*s).len))
    }
}
#[export_name = "rt_hash_str"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_hash_str_abi(str_obj: Value) -> i64 {
    rt_hash_str(str_obj.unwrap_ptr())
}

/// Hash a boolean value (`hash(True) == hash(1)`, `hash(False) == hash(0)`).
#[no_mangle]
pub extern "C" fn rt_hash_bool(value: i8) -> i64 {
    (value != 0) as i64
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

/// Hash a tuple object (CPython's algorithm); raises `TypeError` for an
/// unhashable element.
pub fn rt_hash_tuple(tuple_obj: *mut Obj) -> i64 {
    if tuple_obj.is_null() {
        return 0;
    }
    unsafe { py_hash_or_raise(tuple_obj) }
}
#[export_name = "rt_hash_tuple"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_hash_tuple_abi(tuple_obj: Value) -> i64 {
    rt_hash_tuple(tuple_obj.unwrap_ptr())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_hash_matches_cpython() {
        // Values from CPython 3.14.
        assert_eq!(py_hash_f64(1.5, 0), 1152921504606846977);
        let exact_1e30: BigInt = "1000000000000000019884624838656".parse().unwrap();
        assert_eq!(py_hash_f64(1e30, 0), py_hash_bigint(&exact_1e30));
        assert_eq!(py_hash_f64(2.0, 0), 2);
        assert_eq!(py_hash_f64(-1.0, 0), -2);
        assert_eq!(py_hash_f64(0.1, 0), 230584300921369408);
        assert_eq!(py_hash_f64(f64::INFINITY, 0), 314159);
    }

    #[test]
    fn int_hash_matches_cpython() {
        assert_eq!(py_hash_i64(-1), -2);
        assert_eq!(py_hash_i64((1 << 61) - 1), 0);
        assert_eq!(py_hash_i64(1 << 61), 1);
        assert_eq!(
            py_hash_bigint(&BigInt::from(10).pow(30u32)),
            465258685558744706
        );
    }
}
