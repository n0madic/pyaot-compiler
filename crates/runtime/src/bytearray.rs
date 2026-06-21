//! bytearray runtime support — a MUTABLE byte sequence.
//!
//! Backed by [`ByteArrayObj`](crate::object::ByteArrayObj): a growable object
//! mirroring [`ListObj`](crate::object::ListObj) (header + len + capacity +
//! separately-allocated `*mut u8` buffer), so `append`/`extend` grow in place
//! without moving the object. The buffer is a separate allocation freed by
//! `bytearray_finalize` (slab) and holds no `Value`s, so the GC does not trace
//! into it (a leaf, like `BytesObj`).
//!
//! Read-only methods that mirror `bytes` (`hex`/`decode`/`find`/`rfind`/`count`/
//! `startswith`/`endswith`) take a cheap immutable `BytesObj` snapshot of the
//! live bytes and delegate to the existing `rt_bytes_*` machinery — DRY, at one
//! allocation per call.

use crate::exceptions::ExceptionType;
use crate::gc::{self, gc_pop, gc_push, ShadowFrame};
use crate::object::{ByteArrayObj, BytesObj, Obj, TypeTagKind};
use crate::slice_utils::{normalize_slice_indices, slice_length};
use pyaot_core_defs::Value;
use std::alloc::{alloc_zeroed, dealloc, realloc, Layout};

// =============================================================================
// Allocation / growth
// =============================================================================

/// Allocate an empty `ByteArrayObj` with the given buffer capacity.
fn alloc_bytearray(capacity: usize) -> *mut Obj {
    let obj = gc::gc_alloc(std::mem::size_of::<ByteArrayObj>(), TypeTagKind::ByteArray as u8);
    unsafe {
        let ba = obj as *mut ByteArrayObj;
        (*ba).len = 0;
        (*ba).capacity = capacity;
        (*ba).data = if capacity > 0 {
            let layout = Layout::array::<u8>(capacity).expect("bytearray capacity overflow");
            alloc_zeroed(layout)
        } else {
            std::ptr::null_mut()
        };
    }
    obj
}

/// Ensure `ba` has room for at least `needed` bytes, growing the buffer with the
/// list growth strategy (`realloc`, ~12.5% for large buffers). New bytes are
/// zeroed. The object pointer never moves (only the buffer is reallocated).
unsafe fn ensure_capacity(ba: *mut ByteArrayObj, needed: usize) {
    let capacity = (*ba).capacity;
    if needed <= capacity {
        return;
    }
    let mut new_capacity = crate::list::list_grow_capacity(capacity);
    if new_capacity < needed {
        new_capacity = needed;
    }
    let new_layout = Layout::array::<u8>(new_capacity).expect("bytearray capacity overflow");
    let data = (*ba).data;
    let new_data = if data.is_null() {
        alloc_zeroed(new_layout)
    } else {
        let old_layout = Layout::array::<u8>(capacity).expect("bytearray capacity overflow");
        let p = realloc(data, old_layout, new_layout.size());
        if !p.is_null() {
            // Zero the freshly-grown tail.
            std::ptr::write_bytes(p.add(capacity), 0, new_capacity - capacity);
        }
        p
    };
    if new_data.is_null() {
        raise_exc!(ExceptionType::MemoryError, "cannot allocate memory for bytearray");
    }
    (*ba).data = new_data;
    (*ba).capacity = new_capacity;
}

/// Finalize a bytearray by freeing its data buffer (called by the GC sweep
/// before the object is collected).
///
/// # Safety
/// `ba` must be a valid `ByteArrayObj` about to be deallocated.
pub unsafe fn bytearray_finalize(ba: *mut Obj) {
    if ba.is_null() {
        return;
    }
    let ba = ba as *mut ByteArrayObj;
    let data = (*ba).data;
    let capacity = (*ba).capacity;
    if !data.is_null() && capacity > 0 {
        let layout = Layout::array::<u8>(capacity).expect("bytearray capacity overflow");
        dealloc(data, layout);
    }
}

/// Build a `ByteArrayObj` directly from a byte slice (internal constructor).
unsafe fn bytearray_from_slice(src: *const u8, len: usize) -> *mut Obj {
    let obj = alloc_bytearray(len);
    let ba = obj as *mut ByteArrayObj;
    if len > 0 && !src.is_null() {
        std::ptr::copy_nonoverlapping(src, (*ba).data, len);
    }
    (*ba).len = len;
    obj
}

// =============================================================================
// Constructors
// =============================================================================

/// `bytearray()` — empty.
pub fn rt_make_bytearray_empty() -> *mut Obj {
    alloc_bytearray(0)
}
#[export_name = "rt_make_bytearray_empty"]
pub extern "C" fn rt_make_bytearray_empty_abi() -> Value {
    Value::from_ptr(rt_make_bytearray_empty())
}

/// `bytearray(n)` — `n` zero bytes (n is a RAW i64).
pub fn rt_make_bytearray_zero(n: i64) -> *mut Obj {
    let len = n.max(0) as usize;
    let obj = alloc_bytearray(len);
    unsafe {
        (*(obj as *mut ByteArrayObj)).len = len;
        // Buffer is already zeroed by alloc_zeroed.
    }
    obj
}
#[export_name = "rt_make_bytearray_zero"]
pub extern "C" fn rt_make_bytearray_zero_abi(n: i64) -> Value {
    Value::from_ptr(rt_make_bytearray_zero(n))
}

/// `bytearray(b"…")` — a mutable copy of a bytes object.
pub fn rt_make_bytearray_from_bytes(bytes: *mut Obj) -> *mut Obj {
    if bytes.is_null() {
        return rt_make_bytearray_empty();
    }
    unsafe {
        let src = bytes as *mut BytesObj;
        bytearray_from_slice((*src).data.as_ptr(), (*src).len)
    }
}
#[export_name = "rt_make_bytearray_from_bytes"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_make_bytearray_from_bytes_abi(bytes: Value) -> Value {
    Value::from_ptr(rt_make_bytearray_from_bytes(bytes.unwrap_ptr()))
}

/// `bytearray(str, encoding)` — UTF-8 bytes of the string (the encoding argument
/// is accepted for signature compatibility; only UTF-8 is supported).
pub fn rt_make_bytearray_from_str(str_obj: *mut Obj, _encoding: *mut Obj) -> *mut Obj {
    use crate::object::StrObj;
    if str_obj.is_null() {
        return rt_make_bytearray_empty();
    }
    unsafe {
        let src = str_obj as *mut StrObj;
        bytearray_from_slice((*src).data.as_ptr(), (*src).len)
    }
}
#[export_name = "rt_make_bytearray_from_str"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_make_bytearray_from_str_abi(str_obj: Value, encoding: Value) -> Value {
    Value::from_ptr(rt_make_bytearray_from_str(
        str_obj.unwrap_ptr(),
        encoding.0 as *mut Obj,
    ))
}

/// `bytearray([ints])` — from a list of ints (0-255).
pub fn rt_make_bytearray_from_list(list: *mut Obj) -> *mut Obj {
    use crate::object::ListObj;
    if list.is_null() {
        return rt_make_bytearray_empty();
    }
    unsafe {
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len;
        // Root the list across the allocation.
        let mut roots: [*mut Obj; 1] = [list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);
        let obj = alloc_bytearray(len);
        gc_pop();

        let ba = obj as *mut ByteArrayObj;
        let list_obj = roots[0] as *mut ListObj;
        for i in 0..len {
            let v = (*(*list_obj).data.add(i)).unwrap_int();
            check_byte_value(v);
            *(*ba).data.add(i) = (v & 0xFF) as u8;
        }
        (*ba).len = len;
        obj
    }
}
#[export_name = "rt_make_bytearray_from_list"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_make_bytearray_from_list_abi(list: Value) -> Value {
    Value::from_ptr(rt_make_bytearray_from_list(list.unwrap_ptr()))
}

/// `bytearray(iterable)` — from any iterable of ints. The frontend passes the
/// raw iterable; it is normalized to an iterator internally.
pub fn rt_make_bytearray_from_iter(iterable: *mut Obj) -> *mut Obj {
    let obj = rt_make_bytearray_empty();
    if iterable.is_null() {
        return obj;
    }
    unsafe {
        let mut roots: [*mut Obj; 3] = [obj, iterable, std::ptr::null_mut()];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 3,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);
        roots[1] = crate::iterator::rt_iter_value_dyn(roots[1]);
        loop {
            let elem = crate::iterator::rt_iter_next_no_exc(roots[1]);
            if elem.is_null() {
                break;
            }
            let v = Value(elem as u64).unwrap_int();
            check_byte_value(v);
            bytearray_push_byte(roots[0] as *mut ByteArrayObj, (v & 0xFF) as u8);
        }
        gc_pop();
    }
    obj
}
#[export_name = "rt_make_bytearray_from_iter"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_make_bytearray_from_iter_abi(iterable: Value) -> Value {
    Value::from_ptr(rt_make_bytearray_from_iter(iterable.unwrap_ptr()))
}

/// `bytearray(arg)` — the unified 1-argument constructor. Dispatches on the
/// argument at runtime (so a statically-`Dyn` argument works too): an int → that
/// many zero bytes; a bytes/bytearray → a mutable copy; any other iterable → its
/// int elements; a `str` without an encoding → `TypeError` (use the 2-arg
/// `bytearray(str, encoding)` form).
pub fn rt_make_bytearray(arg: *mut Obj) -> *mut Obj {
    let v = Value(arg as u64);
    if arg.is_null() || v.is_none() {
        return rt_make_bytearray_empty();
    }
    if v.is_int() {
        return rt_make_bytearray_zero(v.unwrap_int());
    }
    if v.is_bool() {
        return rt_make_bytearray_zero(i64::from(v.unwrap_bool()));
    }
    unsafe {
        match (*arg).type_tag() {
            TypeTagKind::Bytes => rt_make_bytearray_from_bytes(arg),
            TypeTagKind::ByteArray => rt_bytearray_slice(arg, i64::MIN, i64::MAX),
            TypeTagKind::Str => raise_exc!(
                ExceptionType::TypeError,
                "string argument without an encoding"
            ),
            // list / tuple / set / range / generator / … of ints.
            _ => rt_make_bytearray_from_iter(arg),
        }
    }
}
#[export_name = "rt_make_bytearray"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_make_bytearray_abi(arg: Value) -> Value {
    Value::from_ptr(rt_make_bytearray(arg.0 as *mut Obj))
}

/// Raise `ValueError` if `v` is not a valid byte (0..=255), matching CPython.
unsafe fn check_byte_value(v: i64) {
    if !(0..=255).contains(&v) {
        raise_exc!(ExceptionType::ValueError, "byte must be in range(0, 256)");
    }
}

/// Append a single (already-validated) byte, growing the buffer if needed.
unsafe fn bytearray_push_byte(ba: *mut ByteArrayObj, byte: u8) {
    let len = (*ba).len;
    ensure_capacity(ba, len + 1);
    *(*ba).data.add(len) = byte;
    (*ba).len = len + 1;
}

// =============================================================================
// Protocol: len / get / set / slice / eq / concat / contains / iter
// =============================================================================

/// Length in bytes.
pub fn rt_bytearray_len(ba: *mut Obj) -> i64 {
    if ba.is_null() {
        return 0;
    }
    unsafe { (*(ba as *mut ByteArrayObj)).len as i64 }
}
#[export_name = "rt_bytearray_len"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytearray_len_abi(ba: Value) -> i64 {
    rt_bytearray_len(ba.unwrap_ptr())
}

/// `ba[i]` → the byte (0-255) as a TAGGED int Value (supports negative index).
pub fn rt_bytearray_get(ba: *mut Obj, index: i64) -> *mut Obj {
    if ba.is_null() {
        unsafe { raise_exc!(ExceptionType::IndexError, "bytearray index out of range") }
    }
    unsafe {
        let ba = ba as *mut ByteArrayObj;
        let len = (*ba).len as i64;
        let idx = if index < 0 { len + index } else { index };
        if idx < 0 || idx >= len {
            raise_exc!(ExceptionType::IndexError, "bytearray index out of range");
        }
        let byte = *(*ba).data.add(idx as usize) as i64;
        Value::from_int(byte).0 as *mut Obj
    }
}
#[export_name = "rt_bytearray_get"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytearray_get_abi(ba: Value, index: i64) -> Value {
    Value::from_ptr(rt_bytearray_get(ba.unwrap_ptr(), index))
}

/// `ba[i] = v` — element assignment (v is a tagged int; supports negative index).
pub fn rt_bytearray_set(ba: *mut Obj, index: i64, value: *mut Obj) {
    if ba.is_null() {
        return;
    }
    unsafe {
        let v = Value(value as u64).unwrap_int();
        check_byte_value(v);
        let ba = ba as *mut ByteArrayObj;
        let len = (*ba).len as i64;
        let idx = if index < 0 { len + index } else { index };
        if idx < 0 || idx >= len {
            raise_exc!(ExceptionType::IndexError, "bytearray index out of range");
        }
        *(*ba).data.add(idx as usize) = (v & 0xFF) as u8;
    }
}
#[export_name = "rt_bytearray_set"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytearray_set_abi(ba: Value, index: i64, value: Value) {
    rt_bytearray_set(ba.unwrap_ptr(), index, value.0 as *mut Obj)
}

/// `ba[start:end]` → a new bytearray.
pub fn rt_bytearray_slice(ba: *mut Obj, start: i64, end: i64) -> *mut Obj {
    if ba.is_null() {
        return rt_make_bytearray_empty();
    }
    unsafe {
        let src = ba as *mut ByteArrayObj;
        let len = (*src).len as i64;
        let (start, end) = normalize_slice_indices(start, end, len, 1);
        let slice_len = slice_length(start, end);
        bytearray_from_slice((*src).data.add(start as usize), slice_len)
    }
}
#[export_name = "rt_bytearray_slice"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytearray_slice_abi(ba: Value, start: i64, end: i64) -> Value {
    Value::from_ptr(rt_bytearray_slice(ba.unwrap_ptr(), start, end))
}

/// `ba[start:end:step]` → a new bytearray.
pub fn rt_bytearray_slice_step(ba: *mut Obj, start: i64, end: i64, step: i64) -> *mut Obj {
    if step == 0 {
        unsafe { raise_exc!(ExceptionType::ValueError, "slice step cannot be zero") }
    }
    if ba.is_null() {
        return rt_make_bytearray_empty();
    }
    unsafe {
        let src = ba as *mut ByteArrayObj;
        let len = (*src).len as i64;
        let (start, end) = normalize_slice_indices(start, end, len, step);
        let data = (*src).data;
        let mut bytes: Vec<u8> = Vec::new();
        if step > 0 {
            let mut i = start;
            while i < end {
                bytes.push(*data.add(i as usize));
                i += step;
            }
        } else {
            let mut i = start;
            while i > end {
                bytes.push(*data.add(i as usize));
                i += step;
            }
        }
        bytearray_from_slice(bytes.as_ptr(), bytes.len())
    }
}
#[export_name = "rt_bytearray_slice_step"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytearray_slice_step_abi(ba: Value, start: i64, end: i64, step: i64) -> Value {
    Value::from_ptr(rt_bytearray_slice_step(ba.unwrap_ptr(), start, end, step))
}

/// A `(ptr, len)` view of any bytes-like (`Bytes` or `ByteArray`) operand.
unsafe fn byte_view(obj: *mut Obj) -> Option<(*const u8, usize)> {
    if obj.is_null() {
        return None;
    }
    match (*obj).type_tag() {
        TypeTagKind::Bytes => {
            let b = obj as *mut BytesObj;
            Some(((*b).data.as_ptr(), (*b).len))
        }
        TypeTagKind::ByteArray => {
            let b = obj as *mut ByteArrayObj;
            Some(((*b).data as *const u8, (*b).len))
        }
        _ => None,
    }
}

/// `bytearray == bytes-like` by content. (Two bytearrays, or a bytearray and a
/// bytes — both compare by byte content.)
pub fn rt_bytearray_eq(a: *mut Obj, b: *mut Obj) -> i8 {
    unsafe {
        match (byte_view(a), byte_view(b)) {
            (Some((pa, la)), Some((pb, lb))) => {
                if la != lb {
                    return 0;
                }
                (std::slice::from_raw_parts(pa, la) == std::slice::from_raw_parts(pb, lb)) as i8
            }
            (None, None) => 1,
            _ => 0,
        }
    }
}
#[export_name = "rt_bytearray_eq"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytearray_eq_abi(a: Value, b: Value) -> i8 {
    rt_bytearray_eq(a.unwrap_ptr(), b.unwrap_ptr())
}

/// `ba + other` (other is bytes-like) → a new bytearray of the concatenation.
pub fn rt_bytearray_concat(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe {
        let (pa, la) = byte_view(a).unwrap_or((std::ptr::null(), 0));
        let (pb, lb) = byte_view(b).unwrap_or((std::ptr::null(), 0));
        let total = la + lb;
        let obj = alloc_bytearray(total);
        let ba = obj as *mut ByteArrayObj;
        if la > 0 {
            std::ptr::copy_nonoverlapping(pa, (*ba).data, la);
        }
        if lb > 0 {
            std::ptr::copy_nonoverlapping(pb, (*ba).data.add(la), lb);
        }
        (*ba).len = total;
        obj
    }
}
#[export_name = "rt_bytearray_concat"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytearray_concat_abi(a: Value, b: Value) -> Value {
    Value::from_ptr(rt_bytearray_concat(a.unwrap_ptr(), b.unwrap_ptr()))
}

/// `elem in ba` — an int byte (membership), or a bytes-like (subsequence).
pub fn rt_bytearray_contains(ba: *mut Obj, elem: *mut Obj) -> i8 {
    unsafe {
        let (pa, la) = match byte_view(ba) {
            Some(v) => v,
            None => return 0,
        };
        let hay = std::slice::from_raw_parts(pa, la);
        let v = Value(elem as u64);
        if v.is_int() {
            let n = v.unwrap_int();
            return hay.iter().any(|&b| b as i64 == n) as i8;
        }
        // bytes-like sub-sequence membership.
        if let Some((pb, lb)) = byte_view(elem) {
            let needle = std::slice::from_raw_parts(pb, lb);
            if needle.is_empty() {
                return 1;
            }
            return hay.windows(needle.len()).any(|w| w == needle) as i8;
        }
        0
    }
}
#[export_name = "rt_bytearray_contains"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytearray_contains_abi(ba: Value, elem: Value) -> i8 {
    rt_bytearray_contains(ba.unwrap_ptr(), elem.0 as *mut Obj)
}

/// Create a bytearray iterator (yields each byte as a tagged int).
pub fn rt_iter_bytearray(ba: *mut Obj) -> *mut Obj {
    use crate::object::{IteratorKind, IteratorObj};
    let obj = gc::gc_alloc(std::mem::size_of::<IteratorObj>(), TypeTagKind::Iterator as u8);
    unsafe {
        let iter = obj as *mut IteratorObj;
        (*iter).kind = IteratorKind::ByteArray as u8;
        (*iter).exhausted = false;
        (*iter).reversed = false;
        (*iter).source = ba;
        (*iter).index = 0;
        (*iter).range_stop = 0;
        (*iter).range_step = 0;
    }
    obj
}
#[export_name = "rt_iter_bytearray"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_bytearray_abi(ba: Value) -> Value {
    Value::from_ptr(rt_iter_bytearray(ba.unwrap_ptr()))
}

// =============================================================================
// Mutators
// =============================================================================

/// `ba.append(int)` — append one byte (0-255), growing in place.
pub fn rt_bytearray_append(ba: *mut Obj, value: *mut Obj) {
    if ba.is_null() {
        return;
    }
    unsafe {
        let v = Value(value as u64).unwrap_int();
        check_byte_value(v);
        bytearray_push_byte(ba as *mut ByteArrayObj, (v & 0xFF) as u8);
    }
}
#[export_name = "rt_bytearray_append"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytearray_append_abi(ba: Value, value: Value) {
    rt_bytearray_append(ba.unwrap_ptr(), value.0 as *mut Obj)
}

/// `ba.extend(iterable)` — append each int from the iterable (a bytes-like
/// iterates as its byte ints, so `ba.extend(b"…")` works through this seam too).
pub fn rt_bytearray_extend(ba: *mut Obj, iterable: *mut Obj) {
    if ba.is_null() || iterable.is_null() {
        return;
    }
    unsafe {
        let mut roots: [*mut Obj; 3] = [ba, iterable, std::ptr::null_mut()];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 3,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);
        roots[1] = crate::iterator::rt_iter_value_dyn(roots[1]);
        loop {
            let elem = crate::iterator::rt_iter_next_no_exc(roots[1]);
            if elem.is_null() {
                break;
            }
            let v = Value(elem as u64).unwrap_int();
            check_byte_value(v);
            bytearray_push_byte(roots[0] as *mut ByteArrayObj, (v & 0xFF) as u8);
        }
        gc_pop();
    }
}
#[export_name = "rt_bytearray_extend"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytearray_extend_abi(ba: Value, iterable: Value) {
    rt_bytearray_extend(ba.unwrap_ptr(), iterable.unwrap_ptr())
}

// =============================================================================
// Read-only methods (delegate to bytes machinery via a snapshot)
// =============================================================================

/// Take a cheap immutable `BytesObj` snapshot of the live bytes.
unsafe fn snapshot_bytes(ba: *mut Obj) -> *mut Obj {
    match byte_view(ba) {
        Some((ptr, len)) => crate::bytes::rt_make_bytes(ptr, len),
        None => crate::bytes::rt_make_bytes(std::ptr::null(), 0),
    }
}

/// Run `f` over a rooted bytes snapshot of `ba` (so a GC during `f`'s own
/// allocation cannot free the snapshot mid-call).
unsafe fn with_snapshot<R>(ba: *mut Obj, f: impl FnOnce(*mut Obj) -> R) -> R {
    let snap = snapshot_bytes(ba);
    let mut roots: [*mut Obj; 1] = [snap];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    gc_push(&mut frame);
    let r = f(roots[0]);
    gc_pop();
    r
}

/// `ba.hex()` → lowercase hex string of the bytes.
pub fn rt_bytearray_hex(ba: *mut Obj) -> *mut Obj {
    unsafe {
        let (ptr, len) = byte_view(ba).unwrap_or((std::ptr::null(), 0));
        let mut s = String::with_capacity(len * 2);
        use std::fmt::Write;
        for i in 0..len {
            let _ = write!(s, "{:02x}", *ptr.add(i));
        }
        crate::string::rt_make_str(s.as_ptr(), s.len())
    }
}
#[export_name = "rt_bytearray_hex"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytearray_hex_abi(ba: Value) -> Value {
    Value::from_ptr(rt_bytearray_hex(ba.unwrap_ptr()))
}

/// `ba.decode(encoding, errors)` → str (delegates to the shared codec).
pub fn rt_bytearray_decode(ba: *mut Obj, encoding: *mut Obj, errors: *mut Obj) -> *mut Obj {
    unsafe { with_snapshot(ba, |snap| crate::bytes::rt_bytes_decode(snap, encoding, errors)) }
}
#[export_name = "rt_bytearray_decode"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytearray_decode_abi(ba: Value, encoding: Value, errors: Value) -> Value {
    Value::from_ptr(rt_bytearray_decode(
        ba.unwrap_ptr(),
        encoding.0 as *mut Obj,
        errors.0 as *mut Obj,
    ))
}

/// `ba.find(sub)` → first index of the bytes-like `sub`, or -1.
pub fn rt_bytearray_find(ba: *mut Obj, sub: *mut Obj) -> i64 {
    unsafe { with_snapshot(ba, |snap| crate::bytes::rt_bytes_find(snap, sub, i64::MIN, i64::MAX)) }
}
#[export_name = "rt_bytearray_find"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytearray_find_abi(ba: Value, sub: Value) -> i64 {
    rt_bytearray_find(ba.unwrap_ptr(), sub.unwrap_ptr())
}

/// `ba.rfind(sub)` → last index of the bytes-like `sub`, or -1.
pub fn rt_bytearray_rfind(ba: *mut Obj, sub: *mut Obj) -> i64 {
    unsafe { with_snapshot(ba, |snap| crate::bytes::rt_bytes_rfind(snap, sub, i64::MIN, i64::MAX)) }
}
#[export_name = "rt_bytearray_rfind"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytearray_rfind_abi(ba: Value, sub: Value) -> i64 {
    rt_bytearray_rfind(ba.unwrap_ptr(), sub.unwrap_ptr())
}

/// `ba.count(sub)` → number of non-overlapping occurrences of `sub`.
pub fn rt_bytearray_count(ba: *mut Obj, sub: *mut Obj) -> i64 {
    unsafe { with_snapshot(ba, |snap| crate::bytes::rt_bytes_count(snap, sub)) }
}
#[export_name = "rt_bytearray_count"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytearray_count_abi(ba: Value, sub: Value) -> i64 {
    rt_bytearray_count(ba.unwrap_ptr(), sub.unwrap_ptr())
}

/// `ba.startswith(prefix)` → bool (as i8).
pub fn rt_bytearray_startswith(ba: *mut Obj, prefix: *mut Obj) -> i8 {
    unsafe { with_snapshot(ba, |snap| crate::bytes::rt_bytes_startswith(snap, prefix) as i8) }
}
#[export_name = "rt_bytearray_startswith"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytearray_startswith_abi(ba: Value, prefix: Value) -> i8 {
    rt_bytearray_startswith(ba.unwrap_ptr(), prefix.unwrap_ptr())
}

/// `ba.endswith(suffix)` → bool (as i8).
pub fn rt_bytearray_endswith(ba: *mut Obj, suffix: *mut Obj) -> i8 {
    unsafe { with_snapshot(ba, |snap| crate::bytes::rt_bytes_endswith(snap, suffix) as i8) }
}
#[export_name = "rt_bytearray_endswith"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytearray_endswith_abi(ba: Value, suffix: Value) -> i8 {
    rt_bytearray_endswith(ba.unwrap_ptr(), suffix.unwrap_ptr())
}

// =============================================================================
// repr / str
// =============================================================================

/// Render `bytearray(b'…')` (used by print/repr/str/ascii dispatch).
pub fn bytearray_repr_string(ba: *mut Obj) -> String {
    unsafe {
        let (ptr, len) = byte_view(ba).unwrap_or((std::ptr::null(), 0));
        let inner = crate::print::format_bytes_repr(ptr, len);
        format!("bytearray({inner})")
    }
}
