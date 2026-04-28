//! Deque runtime support
//!
//! Double-ended queue implemented as a ring buffer.
//! Supports O(1) append/appendleft/pop/popleft and optional maxlen.

use crate::exceptions::ExceptionType;
use crate::gc;
use crate::object::{DequeObj, Obj, TypeTagKind};
use pyaot_core_defs::Value;

/// Minimum ring buffer capacity (power of 2)
const MIN_CAPACITY: usize = 8;

/// Create an empty deque with optional maxlen (-1 = unbounded)
pub fn rt_make_deque(maxlen: i64) -> *mut Obj {
    let deque_size = std::mem::size_of::<DequeObj>();
    let obj = gc::gc_alloc(deque_size, TypeTagKind::Deque as u8);

    unsafe {
        let deque = obj as *mut DequeObj;
        let capacity = MIN_CAPACITY;
        let data = alloc_ring_buffer(capacity);
        (*deque).data = data;
        (*deque).capacity = capacity;
        (*deque).head = 0;
        (*deque).len = 0;
        (*deque).maxlen = maxlen;
    }

    obj
}
#[export_name = "rt_make_deque"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_make_deque_abi(maxlen: i64) -> Value {
    Value::from_ptr(rt_make_deque(maxlen))
}

/// Create a deque from an iterator with optional maxlen
pub fn rt_deque_from_iter(iter: *mut Obj, maxlen: i64) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    let obj = rt_make_deque(maxlen);

    if iter.is_null() {
        return obj;
    }

    // Root the deque object across every rt_iter_next_no_exc call: the iterator's
    // next() may allocate (e.g., string iterator calls rt_str_getchar), triggering
    // a GC collection under gc_stress_test.  Without rooting, the deque would be
    // seen as unreachable and freed.
    let mut root: *mut Obj = obj;
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: &mut root as *mut *mut Obj,
    };
    unsafe { gc_push(&mut frame) };

    loop {
        let elem = crate::iterator::rt_iter_next_no_exc(iter);
        if elem.is_null() {
            break;
        }
        rt_deque_append(root, elem);
    }

    gc_pop();
    root
}
#[export_name = "rt_deque_from_iter"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_from_iter_abi(iter: Value, maxlen: i64) -> Value {
    Value::from_ptr(rt_deque_from_iter(iter.unwrap_ptr(), maxlen))
}

/// deque.append(elem) — add to the right end
pub fn rt_deque_append(deque: *mut Obj, elem: *mut Obj) {
    unsafe {
        let d = deque as *mut DequeObj;
        let maxlen = (*d).maxlen;

        if maxlen >= 0 && (*d).len >= maxlen as usize {
            // At maxlen: remove from left before adding to right
            if maxlen == 0 {
                return; // maxlen=0 means deque is always empty
            }
            // Drop leftmost element
            (*d).head = ((*d).head + 1) % (*d).capacity;
            (*d).len -= 1;
        }

        ensure_capacity(d);

        let idx = ((*d).head + (*d).len) % (*d).capacity;
        *(*d).data.add(idx) = pyaot_core_defs::Value(elem as u64);
        (*d).len += 1;
    }
}
#[export_name = "rt_deque_append"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_append_abi(deque: Value, elem: Value) {
    rt_deque_append(deque.unwrap_ptr(), elem.unwrap_ptr())
}

/// deque.appendleft(elem) — add to the left end
pub fn rt_deque_appendleft(deque: *mut Obj, elem: *mut Obj) {
    unsafe {
        let d = deque as *mut DequeObj;
        let maxlen = (*d).maxlen;

        if maxlen >= 0 && (*d).len >= maxlen as usize {
            if maxlen == 0 {
                return;
            }
            // Drop rightmost element
            (*d).len -= 1;
        }

        ensure_capacity(d);

        (*d).head = if (*d).head == 0 {
            (*d).capacity - 1
        } else {
            (*d).head - 1
        };
        *(*d).data.add((*d).head) = pyaot_core_defs::Value(elem as u64);
        (*d).len += 1;
    }
}
#[export_name = "rt_deque_appendleft"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_appendleft_abi(deque: Value, elem: Value) {
    rt_deque_appendleft(deque.unwrap_ptr(), elem.unwrap_ptr())
}

/// deque.pop() — remove and return from right end
pub fn rt_deque_pop(deque: *mut Obj) -> *mut Obj {
    unsafe {
        let d = deque as *mut DequeObj;
        if (*d).len == 0 {
            raise_exc!(ExceptionType::IndexError, "pop from an empty deque");
        }
        (*d).len -= 1;
        let idx = ((*d).head + (*d).len) % (*d).capacity;
        (*(*d).data.add(idx)).0 as *mut Obj
    }
}
#[export_name = "rt_deque_pop"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_pop_abi(deque: Value) -> Value {
    Value::from_ptr(rt_deque_pop(deque.unwrap_ptr()))
}

/// deque.popleft() — remove and return from left end
pub fn rt_deque_popleft(deque: *mut Obj) -> *mut Obj {
    unsafe {
        let d = deque as *mut DequeObj;
        if (*d).len == 0 {
            raise_exc!(ExceptionType::IndexError, "pop from an empty deque");
        }
        let elem = (*(*d).data.add((*d).head)).0 as *mut Obj;
        (*d).head = ((*d).head + 1) % (*d).capacity;
        (*d).len -= 1;
        elem
    }
}
#[export_name = "rt_deque_popleft"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_popleft_abi(deque: Value) -> Value {
    Value::from_ptr(rt_deque_popleft(deque.unwrap_ptr()))
}

/// deque.extend(iterable) — extend right side with elements from iterable
pub fn rt_deque_extend(deque: *mut Obj, iterable: *mut Obj) {
    if iterable.is_null() {
        return;
    }
    let iter = iterable_to_iterator(iterable);
    loop {
        let elem = crate::iterator::rt_iter_next_no_exc(iter);
        if elem.is_null() {
            break;
        }
        rt_deque_append(deque, elem);
    }
}
#[export_name = "rt_deque_extend"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_extend_abi(deque: Value, iterable: Value) {
    rt_deque_extend(deque.unwrap_ptr(), iterable.unwrap_ptr())
}

/// deque.extendleft(iterable) — extend left side
pub fn rt_deque_extendleft(deque: *mut Obj, iterable: *mut Obj) {
    if iterable.is_null() {
        return;
    }
    let iter = iterable_to_iterator(iterable);
    loop {
        let elem = crate::iterator::rt_iter_next_no_exc(iter);
        if elem.is_null() {
            break;
        }
        rt_deque_appendleft(deque, elem);
    }
}
#[export_name = "rt_deque_extendleft"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_extendleft_abi(deque: Value, iterable: Value) {
    rt_deque_extendleft(deque.unwrap_ptr(), iterable.unwrap_ptr())
}

/// Convert an iterable to an iterator. If already an iterator, return as-is.
fn iterable_to_iterator(obj: *mut Obj) -> *mut Obj {
    unsafe {
        match (*obj).type_tag() {
            TypeTagKind::Iterator | TypeTagKind::Generator => obj,
            TypeTagKind::List => crate::iterator::rt_iter_list(obj),
            TypeTagKind::Tuple => crate::iterator::rt_iter_tuple(obj),
            TypeTagKind::Dict | TypeTagKind::DefaultDict | TypeTagKind::Counter => {
                crate::iterator::rt_iter_dict(obj)
            }
            TypeTagKind::Set => crate::iterator::rt_iter_set(obj),
            TypeTagKind::Str => crate::iterator::rt_iter_str(obj),
            _ => obj, // Assume it's already an iterator
        }
    }
}

/// deque.rotate(n) — rotate n steps to the right (negative = left)
pub fn rt_deque_rotate(deque: *mut Obj, n: i64) {
    unsafe {
        let d = deque as *mut DequeObj;
        let len = (*d).len;
        if len <= 1 {
            return;
        }

        // Normalize n to [0, len)
        let steps = ((n % len as i64) + len as i64) as usize % len;
        if steps == 0 {
            return;
        }

        // Rotate right by moving head back
        (*d).head = ((*d).head + (*d).capacity - steps) % (*d).capacity;
    }
}
#[export_name = "rt_deque_rotate"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_rotate_abi(deque: Value, n: i64) {
    rt_deque_rotate(deque.unwrap_ptr(), n)
}

/// len(deque)
pub fn rt_deque_len(deque: *mut Obj) -> i64 {
    unsafe {
        let d = deque as *mut DequeObj;
        (*d).len as i64
    }
}
#[export_name = "rt_deque_len"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_len_abi(deque: Value) -> i64 {
    rt_deque_len(deque.unwrap_ptr())
}

/// deque[index] — get element by index
pub fn rt_deque_get(deque: *mut Obj, index: i64) -> *mut Obj {
    unsafe {
        let d = deque as *mut DequeObj;
        let len = (*d).len as i64;
        let actual_idx = if index < 0 { len + index } else { index };
        if actual_idx < 0 || actual_idx >= len {
            raise_exc!(ExceptionType::IndexError, "deque index out of range");
        }
        let ring_idx = ((*d).head + actual_idx as usize) % (*d).capacity;
        (*(*d).data.add(ring_idx)).0 as *mut Obj
    }
}
#[export_name = "rt_deque_get"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_get_abi(deque: Value, index: i64) -> Value {
    Value::from_ptr(rt_deque_get(deque.unwrap_ptr(), index))
}

/// deque[index] = value
pub fn rt_deque_set(deque: *mut Obj, index: i64, value: *mut Obj) {
    unsafe {
        let d = deque as *mut DequeObj;
        let len = (*d).len as i64;
        let actual_idx = if index < 0 { len + index } else { index };
        if actual_idx < 0 || actual_idx >= len {
            raise_exc!(ExceptionType::IndexError, "deque index out of range");
        }
        let ring_idx = ((*d).head + actual_idx as usize) % (*d).capacity;
        *(*d).data.add(ring_idx) = pyaot_core_defs::Value(value as u64);
    }
}
#[export_name = "rt_deque_set"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_set_abi(deque: Value, index: i64, value: Value) {
    rt_deque_set(deque.unwrap_ptr(), index, value.unwrap_ptr())
}

/// deque.clear()
pub fn rt_deque_clear(deque: *mut Obj) {
    unsafe {
        let d = deque as *mut DequeObj;
        (*d).head = 0;
        (*d).len = 0;
    }
}
#[export_name = "rt_deque_clear"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_clear_abi(deque: Value) {
    rt_deque_clear(deque.unwrap_ptr())
}

/// deque.reverse()
pub fn rt_deque_reverse(deque: *mut Obj) {
    unsafe {
        let d = deque as *mut DequeObj;
        let len = (*d).len;
        if len <= 1 {
            return;
        }
        for i in 0..len / 2 {
            let left = ((*d).head + i) % (*d).capacity;
            let right = ((*d).head + len - 1 - i) % (*d).capacity;
            let tmp = *(*d).data.add(left);
            *(*d).data.add(left) = *(*d).data.add(right);
            *(*d).data.add(right) = tmp;
        }
    }
}
#[export_name = "rt_deque_reverse"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_reverse_abi(deque: Value) {
    rt_deque_reverse(deque.unwrap_ptr())
}

/// deque.copy() -> new deque
pub fn rt_deque_copy(deque: *mut Obj) -> *mut Obj {
    unsafe {
        let d = deque as *mut DequeObj;
        let new_obj = rt_make_deque((*d).maxlen);
        let _new_d = new_obj as *mut DequeObj;
        for i in 0..(*d).len {
            let idx = ((*d).head + i) % (*d).capacity;
            let elem = (*(*d).data.add(idx)).0 as *mut Obj;
            rt_deque_append(new_obj, elem);
        }
        new_obj
    }
}
#[export_name = "rt_deque_copy"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_copy_abi(deque: Value) -> Value {
    Value::from_ptr(rt_deque_copy(deque.unwrap_ptr()))
}

/// deque.count(value) — count occurrences
pub fn rt_deque_count(deque: *mut Obj, value: *mut Obj) -> i64 {
    unsafe {
        let d = deque as *mut DequeObj;
        let mut count: i64 = 0;
        for i in 0..(*d).len {
            let idx = ((*d).head + i) % (*d).capacity;
            let elem = (*(*d).data.add(idx)).0 as *mut Obj;
            if crate::hash_table_utils::eq_hashable_obj(elem, value) {
                count += 1;
            }
        }
        count
    }
}
#[export_name = "rt_deque_count"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_count_abi(deque: Value, value: Value) -> i64 {
    rt_deque_count(deque.unwrap_ptr(), value.unwrap_ptr())
}

/// Finalize a deque (free the ring buffer)
pub unsafe fn deque_finalize(obj: *mut Obj) {
    use std::alloc::{dealloc, Layout};
    let d = obj as *mut DequeObj;
    let cap = (*d).capacity;
    if !(*d).data.is_null() && cap > 0 {
        let layout =
            Layout::array::<pyaot_core_defs::Value>(cap).expect("Allocation size overflow");
        dealloc((*d).data as *mut u8, layout);
        (*d).data = std::ptr::null_mut();
    }
}

// =============================================================================
// Internal helpers
// =============================================================================

/// Allocate a ring buffer of the given capacity
fn alloc_ring_buffer(capacity: usize) -> *mut pyaot_core_defs::Value {
    use std::alloc::{alloc_zeroed, Layout};
    let layout =
        Layout::array::<pyaot_core_defs::Value>(capacity).expect("Allocation size overflow");
    unsafe { alloc_zeroed(layout) as *mut pyaot_core_defs::Value }
}

/// Ensure the ring buffer has room for one more element
unsafe fn ensure_capacity(d: *mut DequeObj) {
    if (*d).len < (*d).capacity {
        return;
    }

    // Double the capacity
    let old_cap = (*d).capacity;
    let new_cap = old_cap * 2;
    let new_data = alloc_ring_buffer(new_cap);

    // Copy elements to new buffer (linearized from head)
    for i in 0..(*d).len {
        let old_idx = ((*d).head + i) % old_cap;
        *new_data.add(i) = *(*d).data.add(old_idx);
    }

    // Free old buffer
    if !(*d).data.is_null() && old_cap > 0 {
        use std::alloc::{dealloc, Layout};
        let layout =
            Layout::array::<pyaot_core_defs::Value>(old_cap).expect("Allocation size overflow");
        dealloc((*d).data as *mut u8, layout);
    }

    (*d).data = new_data;
    (*d).capacity = new_cap;
    (*d).head = 0;
}
