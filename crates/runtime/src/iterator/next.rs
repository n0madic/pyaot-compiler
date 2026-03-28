//! Iterator next() implementations
//!
//! Core iteration logic for all iterator types.

use super::{box_if_raw_int_iterator, EXHAUSTED_SENTINEL};
use crate::exceptions;
use crate::object::{GeneratorObj, Obj, TypeTagKind, ELEM_HEAP_OBJ};

use super::composite::{call_filter_with_captures, call_map_with_captures};

/// Internal implementation of iterator next()
/// Can optionally raise StopIteration or return sentinel
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub(crate) fn rt_iter_next_internal(iter_obj: *mut Obj, raise_on_exhausted: bool) -> *mut Obj {
    use crate::generator::rt_generator_next;
    use crate::object::{IteratorKind, IteratorObj, StrObj};
    use crate::string::rt_str_getchar;

    if iter_obj.is_null() {
        if raise_on_exhausted {
            let msg = b"next() called on null iterator";
            unsafe {
                exceptions::rt_exc_raise(
                    exceptions::ExceptionType::StopIteration as u8,
                    msg.as_ptr(),
                    msg.len(),
                );
            }
        }
        return EXHAUSTED_SENTINEL;
    }

    unsafe {
        // Check if this is a generator (generators are their own iterators)
        if (*iter_obj).header.type_tag == TypeTagKind::Generator {
            // For generators, we must use the normal path since they use longjmp internally
            // This is OK because generators properly set exhausted flag before raising
            return rt_generator_next(iter_obj);
        }

        let iter = iter_obj as *mut IteratorObj;

        if (*iter).exhausted {
            if raise_on_exhausted {
                exceptions::rt_exc_raise(
                    exceptions::ExceptionType::StopIteration as u8,
                    std::ptr::null(),
                    0,
                );
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
                        exceptions::rt_exc_raise(
                            exceptions::ExceptionType::StopIteration as u8,
                            std::ptr::null(),
                            0,
                        );
                    }
                    return EXHAUSTED_SENTINEL;
                }

                let result = rt_str_getchar((*iter).source, idx);
                if reversed {
                    (*iter).index -= 1;
                } else {
                    (*iter).index += 1;
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
        }
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
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    let result = *(*list).data.add(idx as usize);
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
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    let result = *(*tuple).data.as_ptr().add(idx as usize);
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
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    let result = *(*keys_list).data.add(idx as usize);
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
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    (*iter).index += step;
    current as *mut Obj
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
        if !elem.is_null() && elem != TOMBSTONE {
            (*iter).index = (idx + 1) as i64;
            return elem;
        }
        idx += 1;
    }

    (*iter).exhausted = true;
    if raise_on_exhausted {
        exceptions::rt_exc_raise(
            exceptions::ExceptionType::StopIteration as u8,
            std::ptr::null(),
            0,
        );
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
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    let byte_val = *(*bytes).data.as_ptr().add(idx as usize) as i64;
    if reversed {
        (*iter).index -= 1;
    } else {
        (*iter).index += 1;
    }
    byte_val as *mut Obj
}

/// Next for enumerate iterator
unsafe fn iter_next_enumerate(
    iter: *mut crate::object::IteratorObj,
    raise_on_exhausted: bool,
) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::object::TupleObj;

    let inner = (*iter).source;
    // Use internal version for inner iterator to avoid longjmp
    let elem = rt_iter_next_internal(inner, false);

    if elem == EXHAUSTED_SENTINEL {
        (*iter).exhausted = true;
        if raise_on_exhausted {
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    let counter = (*iter).index;
    (*iter).index += 1;

    // rt_box_int (counter > 256) and box_if_raw_int_iterator both call gc_alloc.
    // rt_make_tuple also calls gc_alloc.  Root each intermediate result before
    // the next allocation so GC stress test cannot collect them.

    // Step 1: box the counter; root elem across this call.
    let mut roots1: [*mut Obj; 1] = [elem];
    let mut frame1 = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots1.as_mut_ptr(),
    };
    gc_push(&mut frame1);
    let boxed_counter = crate::boxing::rt_box_int(counter);
    gc_pop();

    // Step 2: box elem; root boxed_counter across this call.
    let mut roots2: [*mut Obj; 1] = [boxed_counter];
    let mut frame2 = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots2.as_mut_ptr(),
    };
    gc_push(&mut frame2);
    let boxed_elem = box_if_raw_int_iterator(inner, elem);
    gc_pop();

    // Step 3: allocate the tuple; root both boxed values.
    let mut roots3: [*mut Obj; 2] = [boxed_counter, boxed_elem];
    let mut frame3 = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 2,
        roots: roots3.as_mut_ptr(),
    };
    gc_push(&mut frame3);
    let tuple = crate::tuple::rt_make_tuple(2, ELEM_HEAP_OBJ);
    gc_pop();

    let tuple_obj = tuple as *mut TupleObj;
    *(*tuple_obj).data.as_mut_ptr().add(0) = boxed_counter;
    *(*tuple_obj).data.as_mut_ptr().add(1) = boxed_elem;
    tuple
}

/// Next for zip iterator
unsafe fn iter_next_zip(iter_obj: *mut Obj, raise_on_exhausted: bool) -> *mut Obj {
    use crate::object::ZipIterObj;

    let zip_iter = iter_obj as *mut ZipIterObj;

    if (*zip_iter).exhausted {
        if raise_on_exhausted {
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    // Use internal version to avoid longjmp issues
    let item1 = rt_iter_next_internal((*zip_iter).iter1, false);
    if item1 == EXHAUSTED_SENTINEL {
        (*zip_iter).exhausted = true;
        if raise_on_exhausted {
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    let item2 = rt_iter_next_internal((*zip_iter).iter2, false);
    if item2 == EXHAUSTED_SENTINEL {
        (*zip_iter).exhausted = true;
        if raise_on_exhausted {
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    // box_if_raw_int_iterator and rt_make_tuple both call gc_alloc — root all
    // live heap pointers before each allocation.
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    // Step 1: box item1; root item2 (item1 itself may be raw, boxed result needs rooting
    // before next alloc, but item2 is the unrooted heap value that matters).
    let mut r1: [*mut Obj; 1] = [item2];
    let mut f1 = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: r1.as_mut_ptr(),
    };
    gc_push(&mut f1);
    let boxed_item1 = box_if_raw_int_iterator((*zip_iter).iter1, item1);
    gc_pop();

    // Step 2: box item2; root boxed_item1.
    let mut r2: [*mut Obj; 1] = [boxed_item1];
    let mut f2 = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: r2.as_mut_ptr(),
    };
    gc_push(&mut f2);
    let boxed_item2 = box_if_raw_int_iterator((*zip_iter).iter2, item2);
    gc_pop();

    // Step 3: allocate tuple; root both boxed items.
    let mut r3: [*mut Obj; 2] = [boxed_item1, boxed_item2];
    let mut f3 = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 2,
        roots: r3.as_mut_ptr(),
    };
    gc_push(&mut f3);
    let tuple = crate::tuple::rt_make_tuple(2, ELEM_HEAP_OBJ);
    gc_pop();

    crate::tuple::rt_tuple_set(tuple, 0, boxed_item1);
    crate::tuple::rt_tuple_set(tuple, 1, boxed_item2);
    tuple
}

/// Next for zip3 iterator (3 iterables)
unsafe fn iter_next_zip3(iter_obj: *mut Obj, raise_on_exhausted: bool) -> *mut Obj {
    use crate::object::Zip3IterObj;

    let zip_iter = iter_obj as *mut Zip3IterObj;

    if (*zip_iter).exhausted {
        if raise_on_exhausted {
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    let item1 = rt_iter_next_internal((*zip_iter).iter1, false);
    if item1 == EXHAUSTED_SENTINEL {
        (*zip_iter).exhausted = true;
        if raise_on_exhausted {
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    let item2 = rt_iter_next_internal((*zip_iter).iter2, false);
    if item2 == EXHAUSTED_SENTINEL {
        (*zip_iter).exhausted = true;
        if raise_on_exhausted {
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    let item3 = rt_iter_next_internal((*zip_iter).iter3, false);
    if item3 == EXHAUSTED_SENTINEL {
        (*zip_iter).exhausted = true;
        if raise_on_exhausted {
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    // box_if_raw_int_iterator and rt_make_tuple both call gc_alloc — root all
    // live heap pointers before each allocation.
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    // Step 1: box item1; root the two remaining raw items.
    let mut r1: [*mut Obj; 2] = [item2, item3];
    let mut f1 = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 2,
        roots: r1.as_mut_ptr(),
    };
    gc_push(&mut f1);
    let boxed_item1 = box_if_raw_int_iterator((*zip_iter).iter1, item1);
    gc_pop();

    // Step 2: box item2; root boxed_item1 and the remaining raw item.
    let mut r2: [*mut Obj; 2] = [boxed_item1, item3];
    let mut f2 = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 2,
        roots: r2.as_mut_ptr(),
    };
    gc_push(&mut f2);
    let boxed_item2 = box_if_raw_int_iterator((*zip_iter).iter2, item2);
    gc_pop();

    // Step 3: box item3; root the two already-boxed items.
    let mut r3: [*mut Obj; 2] = [boxed_item1, boxed_item2];
    let mut f3 = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 2,
        roots: r3.as_mut_ptr(),
    };
    gc_push(&mut f3);
    let boxed_item3 = box_if_raw_int_iterator((*zip_iter).iter3, item3);
    gc_pop();

    // Step 4: allocate tuple; root all three boxed items.
    let mut r4: [*mut Obj; 3] = [boxed_item1, boxed_item2, boxed_item3];
    let mut f4 = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 3,
        roots: r4.as_mut_ptr(),
    };
    gc_push(&mut f4);
    let tuple = crate::tuple::rt_make_tuple(3, ELEM_HEAP_OBJ);
    gc_pop();

    crate::tuple::rt_tuple_set(tuple, 0, boxed_item1);
    crate::tuple::rt_tuple_set(tuple, 1, boxed_item2);
    crate::tuple::rt_tuple_set(tuple, 2, boxed_item3);
    tuple
}

/// Next for zipN iterator (N iterables stored in a list)
unsafe fn iter_next_zipn(iter_obj: *mut Obj, raise_on_exhausted: bool) -> *mut Obj {
    use crate::object::{ListObj, ZipNIterObj};

    let zip_iter = iter_obj as *mut ZipNIterObj;

    if (*zip_iter).exhausted {
        if raise_on_exhausted {
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    let count = (*zip_iter).count as usize;
    let iters_list = (*zip_iter).iters as *mut ListObj;
    let iters_data = (*iters_list).data;

    // Collect all items first (no allocations during rt_iter_next_internal for raw iterators).
    // We need a fixed-size scratch area; use a Vec for simplicity — it lives on the heap
    // but is not a GC object, so it won't be collected.
    let mut items: Vec<(*mut Obj, *mut Obj)> = Vec::with_capacity(count); // (iter, item)
    for i in 0..count {
        let iter_i = *iters_data.add(i);
        let item = rt_iter_next_internal(iter_i, false);
        if item == EXHAUSTED_SENTINEL {
            (*zip_iter).exhausted = true;
            if raise_on_exhausted {
                exceptions::rt_exc_raise(
                    exceptions::ExceptionType::StopIteration as u8,
                    std::ptr::null(),
                    0,
                );
            }
            return EXHAUSTED_SENTINEL;
        }
        items.push((iter_i, item));
    }

    // Allocate the result tuple, then box and store each item.
    // box_if_raw_int_iterator calls gc_alloc, so the tuple must be rooted while we
    // box each element.  We use a single-element root slot and keep it updated.
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    let mut root_tuple: *mut Obj = crate::tuple::rt_make_tuple(count as i64, ELEM_HEAP_OBJ);
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: &mut root_tuple as *mut *mut Obj,
    };
    gc_push(&mut frame);

    for (i, &(iter_i, item)) in items.iter().enumerate() {
        let boxed_item = box_if_raw_int_iterator(iter_i, item);
        crate::tuple::rt_tuple_set(root_tuple, i as i64, boxed_item);
    }

    gc_pop();
    root_tuple
}

/// Next for map iterator
unsafe fn iter_next_map(iter_obj: *mut Obj, raise_on_exhausted: bool) -> *mut Obj {
    use crate::object::{IteratorObj, MapIterObj};

    let map_iter = iter_obj as *mut MapIterObj;

    if (*map_iter).exhausted {
        if raise_on_exhausted {
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
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
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    // Bit 7 of capture_count encodes whether raw int elements need boxing
    // before calling the map function (set for builtins that expect *mut Obj).
    let raw_cc = (*map_iter).capture_count;
    let needs_boxing = raw_cc & 0x80 != 0;
    let actual_cc = raw_cc & 0x7F;

    let elem_for_call = if needs_boxing {
        box_if_raw_int_iterator((*map_iter).inner_iter, elem)
    } else {
        elem
    };

    // Call map function with captures (if any)
    // Captures are prepended to the argument list: func(c0, c1, ..., elem)
    call_map_with_captures(
        (*map_iter).func_ptr,
        (*map_iter).captures,
        actual_cc,
        elem_for_call,
    )
}

/// Next for filter iterator
unsafe fn iter_next_filter(iter_obj: *mut Obj, raise_on_exhausted: bool) -> *mut Obj {
    use crate::object::{FilterIterObj, IteratorObj, ELEM_RAW_INT};

    let filter_iter = iter_obj as *mut FilterIterObj;

    if (*filter_iter).exhausted {
        if raise_on_exhausted {
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
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
                exceptions::rt_exc_raise(
                    exceptions::ExceptionType::StopIteration as u8,
                    std::ptr::null(),
                    0,
                );
            }
            return EXHAUSTED_SENTINEL;
        }

        // Check if we should use truthiness filtering (func_ptr == 0)
        // or call a predicate function
        let passes = if (*filter_iter).func_ptr == 0 {
            // filter(None, iterable) - use truthiness check
            // Handle raw values vs heap objects based on elem_tag
            match (*filter_iter).elem_tag {
                ELEM_RAW_INT => {
                    // Raw i64: truthy if non-zero
                    (elem as i64) != 0
                }
                _ => {
                    // Heap object (ELEM_HEAP_OBJ): use full truthiness check
                    // Note: Bool is boxed in lists, so uses this path
                    crate::ops::rt_is_truthy(elem) != 0
                }
            }
        } else {
            // filter(func, iterable) - call predicate function with captures
            call_filter_with_captures(
                (*filter_iter).func_ptr,
                (*filter_iter).captures,
                (*filter_iter).capture_count,
                elem,
            )
        };

        if passes {
            return elem;
        }
        // If predicate returns false, continue to next element
    }
}

/// Next for chain iterator
/// Advances through iterators sequentially
unsafe fn iter_next_chain(iter_obj: *mut Obj, raise_on_exhausted: bool) -> *mut Obj {
    use crate::object::{ChainIterObj, ListObj};

    let chain_iter = iter_obj as *mut ChainIterObj;

    if (*chain_iter).exhausted {
        if raise_on_exhausted {
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    // Try to get an element from the current iterator, advancing to next on exhaustion
    while (*chain_iter).current_idx < (*chain_iter).num_iters {
        let iters_list = (*chain_iter).iters as *mut ListObj;
        let current_iter = *(*iters_list).data.add((*chain_iter).current_idx as usize);

        let elem = rt_iter_next_internal(current_iter, false);
        if elem != EXHAUSTED_SENTINEL {
            // Also check exhausted flag since EXHAUSTED_SENTINEL can collide with -1
            let inner_iter = current_iter as *mut crate::object::IteratorObj;
            if !(*inner_iter).exhausted {
                return elem;
            }
        }

        // Current iterator exhausted, move to next
        (*chain_iter).current_idx += 1;
    }

    // All iterators exhausted
    (*chain_iter).exhausted = true;
    if raise_on_exhausted {
        exceptions::rt_exc_raise(
            exceptions::ExceptionType::StopIteration as u8,
            std::ptr::null(),
            0,
        );
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
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    // Check if we've passed the stop point
    if (*islice_iter).stop >= 0 && (*islice_iter).next_yield >= (*islice_iter).stop {
        (*islice_iter).exhausted = true;
        if raise_on_exhausted {
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
        }
        return EXHAUSTED_SENTINEL;
    }

    // Skip elements until we reach next_yield
    while (*islice_iter).current < (*islice_iter).next_yield {
        let elem = rt_iter_next_internal((*islice_iter).inner_iter, false);
        if elem == EXHAUSTED_SENTINEL {
            (*islice_iter).exhausted = true;
            if raise_on_exhausted {
                exceptions::rt_exc_raise(
                    exceptions::ExceptionType::StopIteration as u8,
                    std::ptr::null(),
                    0,
                );
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
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::StopIteration as u8,
                std::ptr::null(),
                0,
            );
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
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_next(iter_obj: *mut Obj) -> *mut Obj {
    // Delegate to internal implementation with raise_on_exhausted = true
    rt_iter_next_internal(iter_obj, true)
}

/// Get next element from iterator WITHOUT raising exceptions
/// Sets the exhausted flag but returns a dummy value instead of raising
/// This is used by for-loops which check the exhausted flag after next()
/// Returns: pointer to next element, or 0 if exhausted
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_next_no_exc(iter_obj: *mut Obj) -> *mut Obj {
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

/// Check if an iterator or generator is exhausted
/// Works for both IteratorObj (lists, tuples, etc.) and GeneratorObj
/// Returns: 1 if exhausted, 0 if not
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_is_exhausted(obj: *mut Obj) -> i8 {
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
