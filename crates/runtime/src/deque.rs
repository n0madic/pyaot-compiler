//! Deque runtime support
//!
//! Double-ended queue implemented as a ring buffer.
//! Supports O(1) append/appendleft/pop/popleft and optional maxlen.

use crate::exceptions::ExceptionType;
use crate::gc;
use crate::object::{DequeObj, Obj, TypeTagKind};

/// Minimum ring buffer capacity (power of 2)
const MIN_CAPACITY: usize = 8;

/// Create an empty deque with optional maxlen (-1 = unbounded)
#[no_mangle]
pub extern "C" fn rt_make_deque(maxlen: i64) -> *mut Obj {
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

/// Create a deque from an iterator with optional maxlen
#[no_mangle]
pub extern "C" fn rt_deque_from_iter(iter: *mut Obj, maxlen: i64) -> *mut Obj {
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

/// deque.append(elem) — add to the right end
#[no_mangle]
pub extern "C" fn rt_deque_append(deque: *mut Obj, elem: *mut Obj) {
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

/// deque.appendleft(elem) — add to the left end
#[no_mangle]
pub extern "C" fn rt_deque_appendleft(deque: *mut Obj, elem: *mut Obj) {
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

/// deque.pop() — remove and return from right end
#[no_mangle]
pub extern "C" fn rt_deque_pop(deque: *mut Obj) -> *mut Obj {
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

/// deque.popleft() — remove and return from left end
#[no_mangle]
pub extern "C" fn rt_deque_popleft(deque: *mut Obj) -> *mut Obj {
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

/// deque.extend(iterable) — extend right side with elements from iterable
#[no_mangle]
pub extern "C" fn rt_deque_extend(deque: *mut Obj, iterable: *mut Obj) {
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

/// deque.extendleft(iterable) — extend left side
#[no_mangle]
pub extern "C" fn rt_deque_extendleft(deque: *mut Obj, iterable: *mut Obj) {
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
#[no_mangle]
pub extern "C" fn rt_deque_rotate(deque: *mut Obj, n: i64) {
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

/// len(deque)
#[no_mangle]
pub extern "C" fn rt_deque_len(deque: *mut Obj) -> i64 {
    unsafe {
        let d = deque as *mut DequeObj;
        (*d).len as i64
    }
}

/// deque[index] — get element by index
#[no_mangle]
pub extern "C" fn rt_deque_get(deque: *mut Obj, index: i64) -> *mut Obj {
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

/// deque[index] = value
#[no_mangle]
pub extern "C" fn rt_deque_set(deque: *mut Obj, index: i64, value: *mut Obj) {
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

/// deque.clear()
#[no_mangle]
pub extern "C" fn rt_deque_clear(deque: *mut Obj) {
    unsafe {
        let d = deque as *mut DequeObj;
        (*d).head = 0;
        (*d).len = 0;
    }
}

/// deque.reverse()
#[no_mangle]
pub extern "C" fn rt_deque_reverse(deque: *mut Obj) {
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

/// deque.copy() -> new deque
#[no_mangle]
pub extern "C" fn rt_deque_copy(deque: *mut Obj) -> *mut Obj {
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

/// deque.count(value) — count occurrences
#[no_mangle]
pub extern "C" fn rt_deque_count(deque: *mut Obj, value: *mut Obj) -> i64 {
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
