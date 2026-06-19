//! Iterator next() implementations
//!
//! Core iteration logic for all iterator types.

use super::EXHAUSTED_SENTINEL;
use crate::exceptions;
use crate::object::{GeneratorObj, Obj, TypeTagKind};
use pyaot_core_defs::Value;

use super::composite::{
    call_filter_with_captures, call_filter_with_captures_tagged, call_map_with_captures,
};

/// Internal implementation of iterator next()
/// Can optionally raise StopIteration or return sentinel
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub(crate) fn rt_iter_next_internal(iter_obj: *mut Obj, raise_on_exhausted: bool) -> *mut Obj {
    use crate::generator::rt_generator_next;
    use crate::object::{IteratorKind, IteratorObj, StrObj};
    use crate::string::rt_str_getchar;

    if iter_obj.is_null() {
        if raise_on_exhausted {
            unsafe {
                raise_exc!(
                    exceptions::ExceptionType::StopIteration,
                    "next() called on null iterator"
                );
            }
        }
        return EXHAUSTED_SENTINEL;
    }

    unsafe {
        // Check if this is a generator (generators are their own iterators)
        if (*iter_obj).header.type_tag == TypeTagKind::Generator {
            // For generators, we must use the normal path since they raise internally
            // This is OK because generators properly set exhausted flag before raising
            let result = rt_generator_next(iter_obj);
            // Match the non-generator branch (below): surface exhaustion as
            // StopIteration on the explicit next() path. The for-loop path passes
            // raise_on_exhausted=false and reads the flag via rt_iter_next_no_exc,
            // so it must NOT raise here. `exhausted` (not the 0 return) is the
            // discriminator — a legit `yield 0` leaves the flag false.
            if raise_on_exhausted && (*(iter_obj as *mut GeneratorObj)).exhausted {
                raise_exc!(exceptions::ExceptionType::StopIteration, "");
            }
            return result;
        }

        // A raw user-class instance passed directly to next() (a self-iterator
        // with __next__, e.g. `next(countup)` without an intervening iter()).
        // Dispatch its compiled <iternext> thunk; the IteratorObj-wrapped case
        // (for-loops / iter();next()) is handled by the IteratorKind::Instance
        // match arm below. The for-loop no-exc path never reaches here — it
        // always iter()-wraps the instance first (rt_iter_value_dyn).
        if (*iter_obj).header.type_tag == TypeTagKind::Instance {
            return match call_iternext_thunk(iter_obj) {
                None => {
                    if raise_on_exhausted {
                        raise_exc!(
                            exceptions::ExceptionType::TypeError,
                            "object is not an iterator"
                        );
                    }
                    EXHAUSTED_SENTINEL
                }
                Some(v) if v.is_unbound() => {
                    if raise_on_exhausted {
                        raise_exc!(exceptions::ExceptionType::StopIteration, "");
                    }
                    EXHAUSTED_SENTINEL
                }
                Some(v) => v.0 as *mut Obj,
            };
        }

        let iter = iter_obj as *mut IteratorObj;

        if (*iter).exhausted {
            if raise_on_exhausted {
                raise_exc!(exceptions::ExceptionType::StopIteration, "");
            }
            return EXHAUSTED_SENTINEL;
        }

        let kind = IteratorKind::try_from((*iter).kind)
            .expect("rt_iter_next_internal: invalid iterator kind");
        let reversed = (*iter).reversed;

        match kind {
            IteratorKind::List => iter_next_list(iter, reversed, raise_on_exhausted),

            IteratorKind::Tuple => iter_next_tuple(iter, reversed, raise_on_exhausted),

            IteratorKind::Dict => iter_next_dict(iter, reversed, raise_on_exhausted),

            IteratorKind::String => {
                let str_obj = (*iter).source as *mut StrObj;
                let len = (*str_obj).len as i64;
                let idx = (*iter).index;

                let out_of_bounds = if reversed { idx < 0 } else { idx >= len };

                if out_of_bounds {
                    (*iter).exhausted = true;
                    if raise_on_exhausted {
                        raise_exc!(exceptions::ExceptionType::StopIteration, "");
                    }
                    return EXHAUSTED_SENTINEL;
                }

                let result = rt_str_getchar((*iter).source, idx);
                let data = (*str_obj).data.as_ptr();
                if reversed {
                    // Step back to the start of the previous codepoint, skipping
                    // UTF-8 continuation bytes (10xxxxxx). Index -1 marks exhaustion.
                    let mut prev = idx - 1;
                    while prev > 0 && (*data.add(prev as usize) & 0xC0) == 0x80 {
                        prev -= 1;
                    }
                    (*iter).index = prev;
                } else {
                    // Advance by the full UTF-8 width of the current codepoint.
                    let width =
                        crate::string::slice::utf8_char_width(*data.add(idx as usize)) as i64;
                    (*iter).index = idx + width;
                }
                result
            }

            IteratorKind::Range => iter_next_range(iter, raise_on_exhausted),

            IteratorKind::Set => iter_next_set(iter, raise_on_exhausted),

            IteratorKind::Bytes => iter_next_bytes(iter, reversed, raise_on_exhausted),

            IteratorKind::Enumerate => iter_next_enumerate(iter, raise_on_exhausted),

            IteratorKind::Zip => iter_next_zip(iter_obj, raise_on_exhausted),

            IteratorKind::Map => iter_next_map(iter_obj, raise_on_exhausted),

            IteratorKind::Filter => iter_next_filter(iter_obj, raise_on_exhausted),

            IteratorKind::Chain => iter_next_chain(iter_obj, raise_on_exhausted),

            IteratorKind::ISlice => iter_next_islice(iter_obj, raise_on_exhausted),

            IteratorKind::Zip3 => iter_next_zip3(iter_obj, raise_on_exhausted),

            IteratorKind::ZipN => iter_next_zipn(iter_obj, raise_on_exhausted),

            IteratorKind::MapTagged => iter_next_map_tagged(iter_obj, raise_on_exhausted),

            IteratorKind::FilterTagged => iter_next_filter_tagged(iter_obj, raise_on_exhausted),

            IteratorKind::Instance => iter_next_instance(iter, raise_on_exhausted),
        }
    }
}

/// Invoke a user-class iterator instance's compiled `<iternext>` thunk (lazy
/// user-class iterator protocol). Returns the thunk's `Value` — the
/// `__next__()` result, or `Value::UNBOUND` when `__next__` raised
/// `StopIteration` (the thunk's `try/except` caught it in compiled code).
/// Returns `None` when the class registered no thunk (its class has no
/// `__next__`, so the instance is not an iterator).
///
/// `inst` must be a valid `InstanceObj`. The thunk call may allocate (the
/// user's `__next__` builds its result) and may unwind (a non-`StopIteration`
/// raise propagates through this fn); this fn allocates no Rust heap before the
/// call, so an unwind leaks nothing (PITFALLS B2).
unsafe fn call_iternext_thunk(inst: *mut Obj) -> Option<Value> {
    use crate::object::InstanceObj;
    let class_id = (*(inst as *const InstanceObj)).class_id;
    let thunk = crate::vtable::lookup_iternext(class_id);
    if thunk.is_null() {
        return None;
    }
    type IterNextFn = unsafe extern "C" fn(*mut Obj) -> Value;
    let f: IterNextFn = std::mem::transmute(thunk);
    Some(f(inst))
}

/// Next for a user-class iterator (`IteratorKind::Instance`): call the source
/// instance's `<iternext>` thunk and translate the `Value::UNBOUND` sentinel
/// (the thunk's StopIteration→sentinel) into the runtime's `exhausted`-flag
/// protocol — set `exhausted` and raise/return per `raise_on_exhausted`.
unsafe fn iter_next_instance(
    iter: *mut crate::object::IteratorObj,
    raise_on_exhausted: bool,
) -> *mut Obj {
    let inst = (*iter).source;
    match call_iternext_thunk(inst) {
        // `__iter__` returned a non-iterator instance (no `__next__`) — CPython
        // raises TypeError when `iter()` returns such an object. The wrap has
        // already happened, so surface it on the first `next()`.
        None => {
            if raise_on_exhausted {
                raise_exc!(
                    exceptions::ExceptionType::TypeError,
                    "iter() returned non-iterator"
                );
            }
            EXHAUSTED_SENTINEL
        }
        Some(v) if v.is_unbound() => {
            (*iter).exhausted = true;
            if raise_on_exhausted {
                raise_exc!(exceptions::ExceptionType::StopIteration, "");
            }
            EXHAUSTED_SENTINEL
        }
        Some(v) => v.0 as *mut Obj,
    }
}

/// Next for list iterator
unsafe fn iter_next_list(
    iter: *mut crate::object::IteratorObj,
    reversed: bool,
    raise_on_exhausted: bool,
) -> *mut Obj {
    use crate::object::ListObj;

    let list = (*iter).source as *mut ListObj;
    let len = (*list).len as i64;
    let idx = (*iter).index;

    let out_of_bounds = if reversed { idx < 0 } else { idx >= len };

    if out_of_bounds {
        (*iter).exhausted = true;
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    // After §F.7c BigBang: return tagged Value bits unchanged. Typed Int/Bool
    // consumers emit UnwrapValueInt/UnwrapValueBool in lowering; Union/Any/Heap
    // consumers pass through as *mut Obj.
    let result = (*(*list).data.add(idx as usize)).0 as *mut Obj;
    if reversed {
        (*iter).index -= 1;
    } else {
        (*iter).index += 1;
    }
    result
}

/// Next for tuple iterator
unsafe fn iter_next_tuple(
    iter: *mut crate::object::IteratorObj,
    reversed: bool,
    raise_on_exhausted: bool,
) -> *mut Obj {
    use crate::object::TupleObj;

    let tuple = (*iter).source as *mut TupleObj;
    let len = (*tuple).len as i64;
    let idx = (*iter).index;

    let out_of_bounds = if reversed { idx < 0 } else { idx >= len };

    if out_of_bounds {
        (*iter).exhausted = true;
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    // After §F.7c BigBang: return tagged Value bits unchanged. Typed Int/Bool
    // consumers emit UnwrapValueInt/UnwrapValueBool in lowering.
    let result = (*(*tuple).data.as_ptr().add(idx as usize)).0 as *mut Obj;
    if reversed {
        (*iter).index -= 1;
    } else {
        (*iter).index += 1;
    }
    result
}

/// Next for dict iterator
unsafe fn iter_next_dict(
    iter: *mut crate::object::IteratorObj,
    reversed: bool,
    raise_on_exhausted: bool,
) -> *mut Obj {
    use crate::object::ListObj;

    let keys_list = (*iter).source as *mut ListObj;
    let len = (*keys_list).len as i64;
    let idx = (*iter).index;

    let out_of_bounds = if reversed { idx < 0 } else { idx >= len };

    if out_of_bounds {
        (*iter).exhausted = true;
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    // After §F.7c BigBang: return tagged Value bits unchanged.
    let result = (*(*keys_list).data.add(idx as usize)).0 as *mut Obj;
    if reversed {
        (*iter).index -= 1;
    } else {
        (*iter).index += 1;
    }
    result
}

/// Next for range iterator
unsafe fn iter_next_range(
    iter: *mut crate::object::IteratorObj,
    raise_on_exhausted: bool,
) -> *mut Obj {
    let current = (*iter).index;
    let stop = (*iter).range_stop;
    let step = (*iter).range_step;

    let exhausted = if step > 0 {
        current >= stop
    } else {
        current <= stop
    };

    if exhausted {
        (*iter).exhausted = true;
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    (*iter).index += step;
    // After §F.7c BigBang: tag the integer so all iter_next_* return tagged Values.
    pyaot_core_defs::Value::from_int(current).0 as *mut Obj
}

/// Next for set iterator
unsafe fn iter_next_set(
    iter: *mut crate::object::IteratorObj,
    raise_on_exhausted: bool,
) -> *mut Obj {
    use crate::object::{SetObj, TOMBSTONE};

    let set = (*iter).source as *mut SetObj;
    let capacity = (*set).capacity;
    let entries = (*set).entries;

    let mut idx = (*iter).index as usize;
    while idx < capacity {
        let entry = entries.add(idx);
        let elem = (*entry).elem;
        if elem.0 != 0 && elem != TOMBSTONE {
            (*iter).index = (idx + 1) as i64;
            // After §F.7c BigBang: return tagged Value bits unchanged.
            return elem.0 as *mut Obj;
        }
        idx += 1;
    }

    (*iter).exhausted = true;
    if raise_on_exhausted {
        raise_exc!(exceptions::ExceptionType::StopIteration, "");
    }
    EXHAUSTED_SENTINEL
}

/// Next for bytes iterator
unsafe fn iter_next_bytes(
    iter: *mut crate::object::IteratorObj,
    reversed: bool,
    raise_on_exhausted: bool,
) -> *mut Obj {
    use crate::object::BytesObj;

    let bytes = (*iter).source as *mut BytesObj;
    let len = (*bytes).len as i64;
    let idx = (*iter).index;

    let out_of_bounds = if reversed { idx < 0 } else { idx >= len };

    if out_of_bounds {
        (*iter).exhausted = true;
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    let byte_val = *(*bytes).data.as_ptr().add(idx as usize) as i64;
    if reversed {
        (*iter).index -= 1;
    } else {
        (*iter).index += 1;
    }
    // After §F.7c BigBang: tag the byte so all iter_next_* return tagged Values.
    pyaot_core_defs::Value::from_int(byte_val).0 as *mut Obj
}

/// Next for enumerate iterator
unsafe fn iter_next_enumerate(
    iter: *mut crate::object::IteratorObj,
    raise_on_exhausted: bool,
) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::object::TupleObj;

    let inner = (*iter).source;
    // Use internal version for inner iterator to avoid raising
    let elem = rt_iter_next_internal(inner, false);

    if elem == EXHAUSTED_SENTINEL {
        (*iter).exhausted = true;
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    let counter = (*iter).index;
    (*iter).index += 1;

    // Value::from_int (counter > 256) and rt_make_tuple both call gc_alloc.
    // Root each intermediate result before the next allocation so GC stress
    // test cannot collect them.

    // Step 1: box the counter; root elem across this call.
    let mut roots1: [*mut Obj; 1] = [elem];
    let mut frame1 = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots1.as_mut_ptr(),
    };
    gc_push(&mut frame1);
    let boxed_counter = pyaot_core_defs::Value::from_int(counter).0 as *mut crate::object::Obj;
    gc_pop();

    // Step 2: box elem; root boxed_counter across this call.
    let mut roots2: [*mut Obj; 1] = [boxed_counter];
    let mut frame2 = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots2.as_mut_ptr(),
    };
    gc_push(&mut frame2);
    let boxed_elem = elem;
    gc_pop();

    // Step 3: allocate the tuple; root both boxed values.
    let mut roots3: [*mut Obj; 2] = [boxed_counter, boxed_elem];
    let mut frame3 = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 2,
        roots: roots3.as_mut_ptr(),
    };
    gc_push(&mut frame3);
    let tuple = crate::tuple::rt_make_tuple(2);
    gc_pop();

    let tuple_obj = tuple as *mut TupleObj;
    *(*tuple_obj).data.as_mut_ptr().add(0) = pyaot_core_defs::Value::from_ptr(boxed_counter);
    *(*tuple_obj).data.as_mut_ptr().add(1) = pyaot_core_defs::Value::from_ptr(boxed_elem);
    tuple
}

/// Next for zip iterator
unsafe fn iter_next_zip(iter_obj: *mut Obj, raise_on_exhausted: bool) -> *mut Obj {
    use crate::object::ZipIterObj;

    let zip_iter = iter_obj as *mut ZipIterObj;

    if (*zip_iter).exhausted {
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    // Use internal version to avoid raising StopIteration
    let item1 = rt_iter_next_internal((*zip_iter).iter1, false);
    if item1 == EXHAUSTED_SENTINEL {
        (*zip_iter).exhausted = true;
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    // One shadow frame, filled progressively: `item1` must survive the SECOND
    // inner next (a string / enumerate / map / generator source ALLOCATES its
    // element, so a collection there would free the fresh, otherwise-unrooted
    // `item1`), and both items must survive `rt_make_tuple`. Null slots are
    // fine — the GC mark skips non-pointers.
    let mut roots: [*mut Obj; 2] = [item1, std::ptr::null_mut()];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 2,
        roots: roots.as_mut_ptr(),
    };
    gc_push(&mut frame);

    let item2 = rt_iter_next_internal((*zip_iter).iter2, false);
    if item2 == EXHAUSTED_SENTINEL {
        gc_pop();
        (*zip_iter).exhausted = true;
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }
    roots[1] = item2;

    let tuple = crate::tuple::rt_make_tuple(2);
    gc_pop();

    // Read the items back THROUGH the roots array — the reads keep the root
    // stores live (a store the compiler deems dead would un-root the item
    // during `rt_make_tuple`'s collection).
    crate::tuple::rt_tuple_set(tuple, 0, roots[0]);
    crate::tuple::rt_tuple_set(tuple, 1, roots[1]);
    tuple
}

/// Next for zip3 iterator (3 iterables)
unsafe fn iter_next_zip3(iter_obj: *mut Obj, raise_on_exhausted: bool) -> *mut Obj {
    use crate::object::Zip3IterObj;

    let zip_iter = iter_obj as *mut Zip3IterObj;

    if (*zip_iter).exhausted {
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    let item1 = rt_iter_next_internal((*zip_iter).iter1, false);
    if item1 == EXHAUSTED_SENTINEL {
        (*zip_iter).exhausted = true;
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    // One shadow frame, filled progressively: each already-obtained item must
    // survive the remaining inner nexts (fresh-element sources allocate) and
    // `rt_make_tuple`. Null slots are skipped by the GC mark.
    let mut roots: [*mut Obj; 3] = [item1, std::ptr::null_mut(), std::ptr::null_mut()];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 3,
        roots: roots.as_mut_ptr(),
    };
    gc_push(&mut frame);

    let item2 = rt_iter_next_internal((*zip_iter).iter2, false);
    if item2 == EXHAUSTED_SENTINEL {
        gc_pop();
        (*zip_iter).exhausted = true;
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }
    roots[1] = item2;

    let item3 = rt_iter_next_internal((*zip_iter).iter3, false);
    if item3 == EXHAUSTED_SENTINEL {
        gc_pop();
        (*zip_iter).exhausted = true;
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }
    roots[2] = item3;

    let tuple = crate::tuple::rt_make_tuple(3);
    gc_pop();

    // Read the items back THROUGH the roots array — the reads keep the root
    // stores live (a store the compiler deems dead would un-root the item).
    crate::tuple::rt_tuple_set(tuple, 0, roots[0]);
    crate::tuple::rt_tuple_set(tuple, 1, roots[1]);
    crate::tuple::rt_tuple_set(tuple, 2, roots[2]);
    tuple
}

/// Next for zipN iterator (N iterables stored in a list)
unsafe fn iter_next_zipn(iter_obj: *mut Obj, raise_on_exhausted: bool) -> *mut Obj {
    use crate::object::{ListObj, ZipNIterObj};

    let zip_iter = iter_obj as *mut ZipNIterObj;

    if (*zip_iter).exhausted {
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    let count = (*zip_iter).count as usize;
    let iters_list = (*zip_iter).iters as *mut ListObj;
    // The collected items live only in this Rust frame, and every inner next
    // after the first — plus `rt_make_tuple` — may allocate (fresh-element
    // sources: string / enumerate / map / generator). Root the scratch area
    // itself as a shadow frame, filled progressively; null slots are skipped
    // by the GC mark. The Vec's buffer is malloc-backed, so its address is
    // stable for the frame's lifetime (no pushes after `gc_push`).
    let mut items: Vec<*mut Obj> = vec![std::ptr::null_mut(); count];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: count,
        roots: items.as_mut_ptr(),
    };
    gc_push(&mut frame);
    for (i, slot) in items.iter_mut().enumerate() {
        let iter_i = (*(*iters_list).data.add(i)).0 as *mut Obj;
        let item = rt_iter_next_internal(iter_i, false);
        if item == EXHAUSTED_SENTINEL {
            gc_pop();
            (*zip_iter).exhausted = true;
            if raise_on_exhausted {
                raise_exc!(exceptions::ExceptionType::StopIteration, "");
            }
            return EXHAUSTED_SENTINEL;
        }
        *slot = item;
    }

    let root_tuple: *mut Obj = crate::tuple::rt_make_tuple(count as i64);
    gc_pop();

    for (i, &item) in items.iter().enumerate() {
        crate::tuple::rt_tuple_set(root_tuple, i as i64, item);
    }

    root_tuple
}

/// Next for map iterator
unsafe fn iter_next_map(iter_obj: *mut Obj, raise_on_exhausted: bool) -> *mut Obj {
    use crate::object::{IteratorObj, MapIterObj};

    let map_iter = iter_obj as *mut MapIterObj;

    if (*map_iter).exhausted {
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    // Get next element from inner iterator
    // We call rt_iter_next_internal then check the inner iterator's exhausted flag
    // because EXHAUSTED_SENTINEL could collide with -1 as a raw int value
    let elem = rt_iter_next_internal((*map_iter).inner_iter, false);
    let inner_iter = (*map_iter).inner_iter;
    // Check if inner iterator is exhausted — must dispatch on type_tag because
    // GeneratorObj and IteratorObj have different layouts and the `exhausted`
    // field lives at a different offset in each struct.
    let inner_exhausted = if (*inner_iter).header.type_tag == TypeTagKind::Generator {
        (*(inner_iter as *mut GeneratorObj)).exhausted
    } else {
        (*(inner_iter as *mut IteratorObj)).exhausted
    };
    if inner_exhausted {
        (*map_iter).exhausted = true;
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    // capture_count encoding (after §F.7c BigBang):
    //   bits 0-7 of low byte: capture count (bit 7 is legacy needs_boxing — no-op now)
    //   elem_unbox_kind:      separate field, set by lowering based on lambda's
    //                         first non-capture param type
    let raw_cc = (*map_iter).capture_count;
    let actual_cc = raw_cc & 0x7F;

    let elem_for_call = match (*map_iter).elem_unbox_kind {
        1 => pyaot_core_defs::Value(elem as u64).unwrap_int() as *mut Obj,
        2 => i64::from(pyaot_core_defs::Value(elem as u64).unwrap_bool()) as *mut Obj,
        _ => elem,
    };

    // Call map function with captures (if any)
    // Captures are prepended to the argument list: func(c0, c1, ..., elem)
    let result = call_map_with_captures(
        (*map_iter).func_ptr,
        (*map_iter).captures,
        actual_cc,
        elem_for_call,
    );

    // After §F.7c BigBang: re-tag raw scalar return values so callers (for-loops,
    // chained iterators) see uniform tagged Value bits.
    match (*map_iter).result_box_kind {
        1 => pyaot_core_defs::Value::from_int(result as i64).0 as *mut Obj,
        2 => pyaot_core_defs::Value::from_bool((result as i64) != 0).0 as *mut Obj,
        _ => result,
    }
}

/// Next for filter iterator
unsafe fn iter_next_filter(iter_obj: *mut Obj, raise_on_exhausted: bool) -> *mut Obj {
    use crate::object::{FilterIterObj, IteratorObj};

    let filter_iter = iter_obj as *mut FilterIterObj;

    if (*filter_iter).exhausted {
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    // Loop until we find an element that passes the predicate
    loop {
        // Get next element from inner iterator
        // We call rt_iter_next_internal then check the inner iterator's exhausted flag
        // because EXHAUSTED_SENTINEL could collide with -1 as a raw int value
        let elem = rt_iter_next_internal((*filter_iter).inner_iter, false);
        let inner_iter = (*filter_iter).inner_iter;
        // Check if inner iterator is exhausted — must dispatch on type_tag because
        // GeneratorObj and IteratorObj have different layouts and the `exhausted`
        // field lives at a different offset in each struct.
        let inner_exhausted = if (*inner_iter).header.type_tag == TypeTagKind::Generator {
            (*(inner_iter as *mut GeneratorObj)).exhausted
        } else {
            (*(inner_iter as *mut IteratorObj)).exhausted
        };
        if inner_exhausted {
            (*filter_iter).exhausted = true;
            if raise_on_exhausted {
                raise_exc!(exceptions::ExceptionType::StopIteration, "");
            }
            return EXHAUSTED_SENTINEL;
        }

        // Check if we should use truthiness filtering (func_ptr == 0)
        // or call a predicate function
        let passes = if (*filter_iter).func_ptr == 0 {
            // filter(None, iterable) — elem is already a tagged Value after BigBang.
            crate::ops::rt_is_truthy(elem) != 0
        } else {
            // filter(func, iterable) - call predicate function with captures.
            // Unbox tagged Value for typed Int/Bool predicate params.
            let elem_for_call = match (*filter_iter).elem_unbox_kind {
                1 => pyaot_core_defs::Value(elem as u64).unwrap_int() as *mut Obj,
                2 => i64::from(pyaot_core_defs::Value(elem as u64).unwrap_bool()) as *mut Obj,
                _ => elem,
            };
            call_filter_with_captures(
                (*filter_iter).func_ptr,
                (*filter_iter).captures,
                (*filter_iter).capture_count & 0x7F,
                elem_for_call,
            )
        };

        if passes {
            return elem;
        }
        // If predicate returns false, continue to next element
    }
}

/// Phase 4+ Extension E2a: tagged-delivery variant of `iter_next_map`.
/// Asymmetric semantics:
///   - INPUT element is passed verbatim — callback's prologue does its
///     own `UnboxValue` for primitive-typed params (per Step E1).
///   - OUTPUT return is re-wrapped via `result_box_kind` if the
///     callback returns a raw primitive (lambdas are not return-ABI
///     flipped today, so this re-wrap is still required to keep
///     downstream consumers seeing uniform tagged Values).
unsafe fn iter_next_map_tagged(iter_obj: *mut Obj, raise_on_exhausted: bool) -> *mut Obj {
    use crate::object::{IteratorObj, MapIterObj};

    let map_iter = iter_obj as *mut MapIterObj;

    if (*map_iter).exhausted {
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    let elem = rt_iter_next_internal((*map_iter).inner_iter, false);
    let inner_iter = (*map_iter).inner_iter;
    let inner_exhausted = if (*inner_iter).header.type_tag == TypeTagKind::Generator {
        (*(inner_iter as *mut GeneratorObj)).exhausted
    } else {
        (*(inner_iter as *mut IteratorObj)).exhausted
    };
    if inner_exhausted {
        (*map_iter).exhausted = true;
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    // Pass element through verbatim — callback's prologue unboxes.
    let result = call_map_with_captures(
        (*map_iter).func_ptr,
        (*map_iter).captures,
        (*map_iter).capture_count & 0x7F,
        elem,
    );

    // Output re-wrap: callback returns raw primitive bits when the
    // declared return type is Int / Bool (lambdas are not return-flipped
    // today). Re-wrap into a tagged Value for the downstream consumer.
    match (*map_iter).result_box_kind {
        1 => pyaot_core_defs::Value::from_int(result as i64).0 as *mut Obj,
        2 => pyaot_core_defs::Value::from_bool((result as i64) != 0).0 as *mut Obj,
        _ => result,
    }
}

/// Phase 4+ Extension E2a: tagged-delivery variant of `iter_next_filter`.
///
/// Two key differences from the legacy `iter_next_filter`:
///   1. INPUT: element is passed verbatim (no `elem_unbox_kind` unboxing)
///      — the phase4-safe callback does its own `UnboxValue` in its prologue.
///   2. OUTPUT: predicate return is interpreted as a **tagged `Value`** (i64),
///      not raw i8. Phase4-return-flipped lambdas box their Bool/Int return
///      into a tagged Value; calling them as `-> i8` reads only the low byte
///      of the tagged representation. Crucially, tagged `false` is `0x03`
///      (BOOL_TAG), whose low byte is `3` — non-zero — so the legacy i8 path
///      would incorrectly admit elements where the predicate returned `false`.
///      `call_filter_with_captures_tagged` calls as `-> i64` and delegates to
///      `rt_is_truthy` for correct tagged-Value truthiness evaluation.
unsafe fn iter_next_filter_tagged(iter_obj: *mut Obj, raise_on_exhausted: bool) -> *mut Obj {
    use crate::object::{FilterIterObj, IteratorObj};

    let filter_iter = iter_obj as *mut FilterIterObj;

    if (*filter_iter).exhausted {
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    loop {
        let elem = rt_iter_next_internal((*filter_iter).inner_iter, false);
        let inner_iter = (*filter_iter).inner_iter;
        let inner_exhausted = if (*inner_iter).header.type_tag == TypeTagKind::Generator {
            (*(inner_iter as *mut GeneratorObj)).exhausted
        } else {
            (*(inner_iter as *mut IteratorObj)).exhausted
        };
        if inner_exhausted {
            (*filter_iter).exhausted = true;
            if raise_on_exhausted {
                raise_exc!(exceptions::ExceptionType::StopIteration, "");
            }
            return EXHAUSTED_SENTINEL;
        }

        // func_ptr == 0 means filter(None, iter) — truthiness on tagged
        // Value (same as legacy variant).
        let passes = if (*filter_iter).func_ptr == 0 {
            crate::ops::rt_is_truthy(elem) != 0
        } else {
            // Pass elem through verbatim — callback's prologue unboxes.
            // Use the tagged-return variant: phase4-safe predicates return a
            // tagged Value (i64). The legacy i8 path is wrong here because
            // tagged bool false (0x03) has a non-zero low byte.
            call_filter_with_captures_tagged(
                (*filter_iter).func_ptr,
                (*filter_iter).captures,
                (*filter_iter).capture_count & 0x7F,
                elem,
            )
        };

        if passes {
            return elem;
        }
    }
}

/// Next for chain iterator
/// Advances through the iterables sequentially, `iter()`-wrapping each lazily.
unsafe fn iter_next_chain(iter_obj: *mut Obj, raise_on_exhausted: bool) -> *mut Obj {
    use crate::iterator::factory::rt_iter_value_dyn;
    use crate::object::{ChainIterObj, ListObj};

    let chain_iter = iter_obj as *mut ChainIterObj;

    if (*chain_iter).exhausted {
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    let iters_list = (*chain_iter).iters as *mut ListObj;
    let num_iters = if iters_list.is_null() {
        0
    } else {
        (*iters_list).len as i64
    };

    // Try to get an element from the current iterable, advancing on exhaustion.
    while (*chain_iter).current_idx < num_iters {
        // Lazily `iter()`-wrap the current iterable on first use. `iter()` is
        // idempotent on generators/iterators and raises TypeError on a
        // non-iterable. The result is rooted in `current_iter` so the GC can
        // trace it across the inner `next()`'s allocations.
        if (*chain_iter).current_iter.is_null() {
            let iterable =
                (*(*iters_list).data.add((*chain_iter).current_idx as usize)).0 as *mut Obj;
            (*chain_iter).current_iter = rt_iter_value_dyn(iterable);
        }
        let current_iter = (*chain_iter).current_iter;

        let elem = rt_iter_next_internal(current_iter, false);
        if elem != EXHAUSTED_SENTINEL {
            // Also check exhausted flag since EXHAUSTED_SENTINEL can collide with -1.
            // The exhausted flag lives at different offsets in GeneratorObj
            // (offset 24) vs IteratorObj (offset 17), so dispatch on type_tag —
            // otherwise a chained generator's exhausted bit is read from the
            // wrong field and the chain truncates (mirrors iter_next_map_tagged).
            let inner_exhausted = if (*current_iter).header.type_tag == TypeTagKind::Generator {
                (*(current_iter as *mut crate::object::GeneratorObj)).exhausted
            } else {
                (*(current_iter as *mut crate::object::IteratorObj)).exhausted
            };
            if !inner_exhausted {
                return elem;
            }
        }

        // Current iterable exhausted: drop its iterator and move to the next.
        (*chain_iter).current_iter = std::ptr::null_mut();
        (*chain_iter).current_idx += 1;
    }

    // All iterables exhausted
    (*chain_iter).exhausted = true;
    if raise_on_exhausted {
        raise_exc!(exceptions::ExceptionType::StopIteration, "");
    }
    EXHAUSTED_SENTINEL
}

/// Next for islice iterator
/// Yields elements at positions [start, start+step, start+2*step, ...) up to stop
unsafe fn iter_next_islice(iter_obj: *mut Obj, raise_on_exhausted: bool) -> *mut Obj {
    use crate::object::ISliceIterObj;

    let islice_iter = iter_obj as *mut ISliceIterObj;

    if (*islice_iter).exhausted {
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    // Check if we've passed the stop point
    if (*islice_iter).stop >= 0 && (*islice_iter).next_yield >= (*islice_iter).stop {
        (*islice_iter).exhausted = true;
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }

    // Skip elements until we reach next_yield
    while (*islice_iter).current < (*islice_iter).next_yield {
        let elem = rt_iter_next_internal((*islice_iter).inner_iter, false);
        if elem == EXHAUSTED_SENTINEL {
            (*islice_iter).exhausted = true;
            if raise_on_exhausted {
                raise_exc!(exceptions::ExceptionType::StopIteration, "");
            }
            return EXHAUSTED_SENTINEL;
        }
        (*islice_iter).current += 1;
    }

    // Get the element at next_yield position
    let elem = rt_iter_next_internal((*islice_iter).inner_iter, false);
    if elem == EXHAUSTED_SENTINEL {
        (*islice_iter).exhausted = true;
        if raise_on_exhausted {
            raise_exc!(exceptions::ExceptionType::StopIteration, "");
        }
        return EXHAUSTED_SENTINEL;
    }
    (*islice_iter).current += 1;

    // Advance next_yield by step
    (*islice_iter).next_yield += (*islice_iter).step;

    elem
}

/// Get next element from iterator
/// Raises StopIteration when iterator is exhausted
/// Returns: pointer to next element
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_iter_next(iter_obj: *mut Obj) -> *mut Obj {
    // Delegate to internal implementation with raise_on_exhausted = true
    rt_iter_next_internal(iter_obj, true)
}
#[export_name = "rt_iter_next"]
pub extern "C" fn rt_iter_next_abi(iter_obj: Value) -> Value {
    Value::from_ptr(rt_iter_next(iter_obj.unwrap_ptr()))
}

/// Get next element from iterator WITHOUT raising exceptions
/// Sets the exhausted flag but returns a dummy value instead of raising
/// This is used by for-loops which check the exhausted flag after next()
/// Returns: pointer to next element, or 0 if exhausted
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_iter_next_no_exc(iter_obj: *mut Obj) -> *mut Obj {
    use crate::object::{GeneratorObj, IteratorObj};

    let result = rt_iter_next_internal(iter_obj, false);

    // Check if iterator is exhausted by looking at the exhausted flag
    // instead of comparing to EXHAUSTED_SENTINEL, because -1 as i64
    // has the same bit pattern as EXHAUSTED_SENTINEL (usize::MAX)
    let is_exhausted = unsafe {
        if iter_obj.is_null() {
            true
        } else if (*iter_obj).header.type_tag == TypeTagKind::Generator {
            let gen = iter_obj as *mut GeneratorObj;
            (*gen).exhausted
        } else {
            // All iterator types (IteratorObj, ZipIterObj, MapIterObj, FilterIterObj)
            // have `exhausted` at the same offset: after header and kind byte
            let iter = iter_obj as *mut IteratorObj;
            (*iter).exhausted
        }
    };

    if is_exhausted {
        // Return null as dummy value - caller will check exhausted flag
        std::ptr::null_mut()
    } else {
        result
    }
}
#[export_name = "rt_iter_next_no_exc"]
pub extern "C" fn rt_iter_next_no_exc_abi(iter_obj: Value) -> Value {
    Value::from_ptr(rt_iter_next_no_exc(iter_obj.unwrap_ptr()))
}

/// Check if an iterator or generator is exhausted
/// Works for both IteratorObj (lists, tuples, etc.) and GeneratorObj
/// Returns: 1 if exhausted, 0 if not
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_iter_is_exhausted(obj: *mut Obj) -> i8 {
    use crate::object::{GeneratorObj, IteratorObj};

    if obj.is_null() {
        return 1; // Null is considered exhausted
    }

    unsafe {
        let type_tag = (*obj).header.type_tag;

        if type_tag == TypeTagKind::Generator {
            // Generator object
            let gen = obj as *mut GeneratorObj;
            if (*gen).exhausted {
                1
            } else {
                0
            }
        } else if type_tag == TypeTagKind::Iterator {
            // Iterator object (list, tuple, dict, etc.)
            let iter = obj as *mut IteratorObj;
            if (*iter).exhausted {
                1
            } else {
                0
            }
        } else {
            // Unknown type - treat as exhausted
            1
        }
    }
}
#[export_name = "rt_iter_is_exhausted"]
pub extern "C" fn rt_iter_is_exhausted_abi(obj: Value) -> i8 {
    rt_iter_is_exhausted(obj.unwrap_ptr())
}
