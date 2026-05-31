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

/// Convert a deque to a list, preserving left-to-right order. Backs
/// `list(deque)` and the for-loop iteration path (a deque is not an
/// `IteratorObj`, so it cannot be fed to `rt_list_from_iter` directly —
/// lowering converts to a list first, mirroring set/dict iteration).
/// Elements are tagged `Value`s, copied verbatim from the ring buffer.
pub fn rt_list_from_deque(deque: *mut Obj) -> *mut Obj {
    use crate::object::ListObj;
    use std::alloc::{alloc_zeroed, Layout};

    let alloc_empty_list = || -> *mut Obj {
        let size = std::mem::size_of::<ListObj>();
        let obj = gc::gc_alloc(size, TypeTagKind::List as u8);
        unsafe {
            let list = obj as *mut ListObj;
            (*list).len = 0;
            (*list).capacity = 0;
            (*list).data = std::ptr::null_mut();
        }
        obj
    };

    if deque.is_null() {
        return alloc_empty_list();
    }

    unsafe {
        crate::debug_assert_type_tag!(deque, TypeTagKind::Deque, "rt_list_from_deque");
        let len = (*(deque as *mut DequeObj)).len;
        if len == 0 {
            return alloc_empty_list();
        }

        // Root the deque across gc_alloc, which may trigger a collection.
        let mut roots: [*mut Obj; 1] = [deque];
        let mut frame = gc::ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc::gc_push(&mut frame);

        let list_size = std::mem::size_of::<ListObj>();
        let list_obj = gc::gc_alloc(list_size, TypeTagKind::List as u8);

        gc::gc_pop();

        // Re-derive the deque pointer through the rooted slot after allocation.
        let d = roots[0] as *mut DequeObj;
        let list = list_obj as *mut ListObj;

        // Raw allocator (does not trigger GC); deque slots are tagged Values
        // with the same layout as list storage, so a verbatim copy is sound.
        let data_layout =
            Layout::array::<Value>(len).expect("Allocation size overflow - capacity too large");
        let data = alloc_zeroed(data_layout) as *mut Value;

        let capacity = (*d).capacity;
        let head = (*d).head;
        for i in 0..len {
            let ring_idx = (head + i) % capacity;
            *data.add(i) = *(*d).data.add(ring_idx);
        }

        (*list).len = len;
        (*list).capacity = len;
        (*list).data = data;

        list_obj
    }
}
#[export_name = "rt_list_from_deque"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_from_deque_abi(deque: Value) -> Value {
    Value::from_ptr(rt_list_from_deque(deque.unwrap_ptr()))
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
    // `elem` is a stored slot that may be a tagged immediate (int/bool/None);
    // pass raw bits so the tag survives instead of tripping `unwrap_ptr`'s
    // debug `is_ptr` assertion. `deque` is always a heap pointer.
    rt_deque_append(deque.unwrap_ptr(), elem.0 as *mut Obj)
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
    // `elem` may be a tagged immediate; pass raw bits (see `rt_deque_append_abi`).
    rt_deque_appendleft(deque.unwrap_ptr(), elem.0 as *mut Obj)
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
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if iterable.is_null() {
        return;
    }
    // Root the deque, the (possibly freshly-allocated) iterator and the scratch
    // element across every allocating `rt_iter_next_no_exc` call: next() may
    // trigger a GC that would otherwise free the unrooted iterator/deque.
    // Mirrors `rt_deque_from_iter`'s rooting recipe.
    let mut roots: [*mut Obj; 3] = [deque, iterable, std::ptr::null_mut()];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 3,
        roots: roots.as_mut_ptr(),
    };
    unsafe { gc_push(&mut frame) };

    let iter = iterable_to_iterator(iterable);
    roots[1] = iter;
    loop {
        let elem = crate::iterator::rt_iter_next_no_exc(iter);
        if elem.is_null() {
            break;
        }
        roots[2] = elem;
        rt_deque_append(deque, elem);
    }

    gc_pop();
}
#[export_name = "rt_deque_extend"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_extend_abi(deque: Value, iterable: Value) {
    rt_deque_extend(deque.unwrap_ptr(), iterable.unwrap_ptr())
}

/// deque.extendleft(iterable) — extend left side
pub fn rt_deque_extendleft(deque: *mut Obj, iterable: *mut Obj) {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if iterable.is_null() {
        return;
    }
    // Root deque + iterator + scratch element across allocating next() calls
    // (see `rt_deque_extend` for the rationale).
    let mut roots: [*mut Obj; 3] = [deque, iterable, std::ptr::null_mut()];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 3,
        roots: roots.as_mut_ptr(),
    };
    unsafe { gc_push(&mut frame) };

    let iter = iterable_to_iterator(iterable);
    roots[1] = iter;
    loop {
        let elem = crate::iterator::rt_iter_next_no_exc(iter);
        if elem.is_null() {
            break;
        }
        roots[2] = elem;
        rt_deque_appendleft(deque, elem);
    }

    gc_pop();
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
    // `value` may be a tagged immediate; pass raw bits (see `rt_deque_append_abi`).
    rt_deque_set(deque.unwrap_ptr(), index, value.0 as *mut Obj)
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
    // `value` is the search element, possibly a tagged immediate (int/bool/None);
    // pass raw bits so the tag survives instead of tripping `unwrap_ptr`'s debug
    // `is_ptr` assertion. `deque` is always a heap pointer.
    rt_deque_count(deque.unwrap_ptr(), value.0 as *mut Obj)
}

/// `del dq[index]` — remove the element at `index`, ring-aware.
/// Negative indices count from the right (mirrors `rt_deque_get`).
pub fn rt_deque_delete(deque: *mut Obj, index: i64) {
    unsafe {
        let d = deque as *mut DequeObj;
        let len_i = (*d).len as i64;
        let actual_idx = if index < 0 { len_i + index } else { index };
        if actual_idx < 0 || actual_idx >= len_i {
            raise_exc!(ExceptionType::IndexError, "deque index out of range");
        }
        let idx = actual_idx as usize;
        let len = (*d).len;
        let cap = (*d).capacity;
        let left_count = idx;
        let right_count = len - 1 - idx;
        if left_count <= right_count {
            // Shift the left block one step toward the gap, then drop the front.
            let mut j = idx;
            while j > 0 {
                let dst = ((*d).head + j) % cap;
                let src = ((*d).head + j - 1) % cap;
                *(*d).data.add(dst) = *(*d).data.add(src);
                j -= 1;
            }
            (*d).head = ((*d).head + 1) % cap;
        } else {
            // Shift the right block one step toward the gap, then drop the tail.
            let mut j = idx;
            while j < len - 1 {
                let dst = ((*d).head + j) % cap;
                let src = ((*d).head + j + 1) % cap;
                *(*d).data.add(dst) = *(*d).data.add(src);
                j += 1;
            }
        }
        (*d).len -= 1;
    }
}
#[export_name = "rt_deque_delete"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_delete_abi(deque: Value, index: i64) {
    rt_deque_delete(deque.unwrap_ptr(), index)
}

/// deque.index(value) -> i64 — first logical position of `value`.
/// Raises `ValueError: deque.index(x): x not in deque` if absent (matches
/// CPython 3.14's message).
pub fn rt_deque_index(deque: *mut Obj, value: *mut Obj) -> i64 {
    unsafe {
        let d = deque as *mut DequeObj;
        for i in 0..(*d).len {
            let idx = ((*d).head + i) % (*d).capacity;
            let elem = (*(*d).data.add(idx)).0 as *mut Obj;
            if crate::hash_table_utils::eq_hashable_obj(elem, value) {
                return i as i64;
            }
        }
        raise_exc!(ExceptionType::ValueError, "deque.index(x): x not in deque");
    }
}
#[export_name = "rt_deque_index"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_index_abi(deque: Value, value: Value) -> i64 {
    // `value` is the search element, possibly a tagged immediate; pass raw bits
    // (see `rt_deque_count_abi`). `deque` is always a heap pointer.
    rt_deque_index(deque.unwrap_ptr(), value.0 as *mut Obj)
}

/// deque.insert(index, value) — insert `value` before logical `index`.
/// Raises `IndexError` when the deque is already at `maxlen` (CPython 3.5+).
pub fn rt_deque_insert(deque: *mut Obj, index: i64, value: *mut Obj) {
    unsafe {
        let d = deque as *mut DequeObj;
        let maxlen = (*d).maxlen;
        if maxlen >= 0 && (*d).len >= maxlen as usize {
            raise_exc!(
                ExceptionType::IndexError,
                "deque already at its maximum size"
            );
        }
        // Clamp `index` CPython-style: negatives count from the right, then the
        // result is pinned to [0, len].
        let len_i = (*d).len as i64;
        let mut idx = index;
        if idx < 0 {
            idx += len_i;
            if idx < 0 {
                idx = 0;
            }
        }
        if idx > len_i {
            idx = len_i;
        }
        let idx = idx as usize;

        ensure_capacity(d);
        let cap = (*d).capacity;
        let len = (*d).len; // unchanged by ensure_capacity
        let left_count = idx;
        let right_count = len - idx;
        if left_count <= right_count {
            // Open the slot by shifting the left block toward a new head.
            let new_head = ((*d).head + cap - 1) % cap;
            for p in 0..idx {
                let src = ((*d).head + p) % cap;
                let dst = (new_head + p) % cap;
                *(*d).data.add(dst) = *(*d).data.add(src);
            }
            (*d).head = new_head;
            let slot = (new_head + idx) % cap;
            *(*d).data.add(slot) = pyaot_core_defs::Value(value as u64);
        } else {
            // Open the slot by shifting the right block one step right.
            let mut k = len;
            while k > idx {
                let src = ((*d).head + k - 1) % cap;
                let dst = ((*d).head + k) % cap;
                *(*d).data.add(dst) = *(*d).data.add(src);
                k -= 1;
            }
            let slot = ((*d).head + idx) % cap;
            *(*d).data.add(slot) = pyaot_core_defs::Value(value as u64);
        }
        (*d).len += 1;
    }
}
#[export_name = "rt_deque_insert"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_insert_abi(deque: Value, index: i64, value: Value) {
    // `index` arrives raw (TypeSpec::Int, unboxed); `value` may be a tagged
    // immediate — pass raw bits (see `rt_deque_count_abi`).
    rt_deque_insert(deque.unwrap_ptr(), index, value.0 as *mut Obj)
}

/// deque.remove(value) — remove the first occurrence of `value`.
/// Raises `ValueError` if absent (CPython message).
pub fn rt_deque_remove(deque: *mut Obj, value: *mut Obj) {
    unsafe {
        let d = deque as *mut DequeObj;
        for i in 0..(*d).len {
            let idx = ((*d).head + i) % (*d).capacity;
            let elem = (*(*d).data.add(idx)).0 as *mut Obj;
            if crate::hash_table_utils::eq_hashable_obj(elem, value) {
                rt_deque_delete(deque, i as i64);
                return;
            }
        }
        raise_exc!(ExceptionType::ValueError, "deque.remove(x): x not in deque");
    }
}
#[export_name = "rt_deque_remove"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_deque_remove_abi(deque: Value, value: Value) {
    // `value` may be a tagged immediate; pass raw bits (see `rt_deque_count_abi`).
    rt_deque_remove(deque.unwrap_ptr(), value.0 as *mut Obj)
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
