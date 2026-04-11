use crate::gc::{gc_alloc, gc_pop, gc_push, ShadowFrame};
use crate::object::{DictObj, InstanceObj, ListObj, Obj, SetObj, TupleObj};
use pyaot_core_defs::TypeTagKind;
use std::collections::HashMap;

/// Shallow copy of an object
///
/// Immutable types (Int, Float, Bool, Str, None, Bytes, Tuple) are returned as-is.
/// Mutable containers (List, Dict, Set, Instance) get a new container with copied references.
#[no_mangle]
pub unsafe extern "C" fn rt_copy_copy(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return obj;
    }

    let header = &(*obj).header;
    match header.type_tag {
        // Immutable types - return as-is
        TypeTagKind::Int
        | TypeTagKind::Float
        | TypeTagKind::Bool
        | TypeTagKind::Str
        | TypeTagKind::None
        | TypeTagKind::Bytes
        | TypeTagKind::Tuple => obj,

        // List - shallow copy
        TypeTagKind::List => {
            let orig = obj as *mut ListObj;
            let len = (*orig).len;
            let capacity = (*orig).capacity;
            let elem_tag = (*orig).elem_tag;

            let new_list = crate::list::rt_make_list(capacity as i64, elem_tag) as *mut ListObj;
            if len > 0 {
                std::ptr::copy_nonoverlapping((*orig).data, (*new_list).data, len);
            }
            (*new_list).len = len;

            new_list as *mut Obj
        }

        // Dict - shallow copy (preserves insertion order)
        TypeTagKind::Dict => {
            let orig = obj as *mut DictObj;
            let len = (*orig).len;

            let new_dict = crate::dict::rt_make_dict(len as i64);

            if len > 0 {
                // Root new_dict while rt_dict_set may trigger GC on resize
                let mut roots: [*mut Obj; 1] = [new_dict];
                let mut frame = ShadowFrame {
                    prev: std::ptr::null_mut(),
                    nroots: 1,
                    roots: roots.as_mut_ptr(),
                };
                gc_push(&mut frame);

                for i in 0..(*orig).entries_len {
                    let entry = (*orig).entries.add(i);
                    let key = (*entry).key;
                    if !key.is_null() {
                        crate::dict::rt_dict_set(roots[0], key, (*entry).value);
                    }
                }

                gc_pop();

                roots[0]
            } else {
                new_dict
            }
        }

        // Set - shallow copy
        TypeTagKind::Set => {
            let orig = obj as *mut SetObj;
            let len = (*orig).len;
            let capacity = (*orig).capacity;

            // Create new set with same capacity
            let new_set = crate::set::rt_make_set(capacity as i64);

            if len > 0 {
                // Root new_set while rt_set_add may trigger GC on resize
                let mut roots: [*mut Obj; 1] = [new_set];
                let mut frame = ShadowFrame {
                    prev: std::ptr::null_mut(),
                    nroots: 1,
                    roots: roots.as_mut_ptr(),
                };
                gc_push(&mut frame);

                // Iterate through original entries and copy to new set
                for i in 0..capacity {
                    let entry = (*orig).entries.add(i);
                    let elem = (*entry).elem;

                    // Skip empty slots (TOMBSTONE or null)
                    if elem.is_null() || elem == crate::object::TOMBSTONE {
                        continue;
                    }

                    crate::set::rt_set_add(roots[0], elem);
                }

                gc_pop();

                roots[0]
            } else {
                new_set
            }
        }

        // Instance - check for __copy__ dunder, then fall back to shallow copy
        TypeTagKind::Instance => {
            let orig = obj as *mut InstanceObj;
            let class_id = (*orig).class_id;

            // Check for user-defined __copy__
            let copy_fn = crate::vtable::get_copy_func(class_id);
            if !copy_fn.is_null() {
                let copy_fn: extern "C" fn(i64) -> *mut Obj = std::mem::transmute(copy_fn);
                return copy_fn(obj as i64);
            }

            // Default: shallow copy fields
            let field_count = (*orig).field_count;
            let vtable = (*orig).vtable;

            let size =
                std::mem::size_of::<InstanceObj>() + field_count * std::mem::size_of::<*mut Obj>();
            let new_inst = gc_alloc(size, TypeTagKind::Instance.tag()) as *mut InstanceObj;

            (*new_inst).class_id = class_id;
            (*new_inst).field_count = field_count;
            (*new_inst).vtable = vtable;

            if field_count > 0 {
                let orig_fields = (*orig).fields.as_ptr();
                let new_fields = (*new_inst).fields.as_mut_ptr();
                std::ptr::copy_nonoverlapping(orig_fields, new_fields, field_count);
            }

            new_inst as *mut Obj
        }

        // File, Generator, Iterator — raise TypeError (CPython does the same)
        TypeTagKind::File => {
            raise_exc!(
                pyaot_core_defs::BuiltinExceptionKind::TypeError,
                "cannot copy file objects"
            );
        }
        TypeTagKind::Generator => {
            raise_exc!(
                pyaot_core_defs::BuiltinExceptionKind::TypeError,
                "cannot copy generator objects"
            );
        }
        TypeTagKind::Iterator => {
            raise_exc!(
                pyaot_core_defs::BuiltinExceptionKind::TypeError,
                "cannot copy iterator objects"
            );
        }

        // Other types (Cell, Match, StringBuilder, StructTime, CompletedProcess,
        // Hash, StringIO, BytesIO) - return as-is
        _ => obj,
    }
}

/// Deep copy of an object with cycle detection
///
/// Immutable types (Int, Float, Bool, Str, None, Bytes) are returned as-is.
/// Mutable containers and tuples are recursively deep copied.
#[no_mangle]
pub unsafe extern "C" fn rt_copy_deepcopy(obj: *mut Obj) -> *mut Obj {
    let mut memo: HashMap<usize, *mut Obj> = HashMap::new();
    deep_copy_recursive(obj, &mut memo)
}

/// Internal recursive helper for deep copy with cycle detection
unsafe fn deep_copy_recursive(obj: *mut Obj, memo: &mut HashMap<usize, *mut Obj>) -> *mut Obj {
    if obj.is_null() {
        return obj;
    }

    let header = &(*obj).header;
    match header.type_tag {
        // Immutable types - return as-is
        TypeTagKind::Int
        | TypeTagKind::Float
        | TypeTagKind::Bool
        | TypeTagKind::Str
        | TypeTagKind::None
        | TypeTagKind::Bytes => obj,

        // Tuple - deep copy each element
        TypeTagKind::Tuple => {
            let obj_addr = obj as usize;
            if let Some(&cached) = memo.get(&obj_addr) {
                return cached;
            }

            let orig = obj as *mut TupleObj;
            let len = (*orig).len;
            let elem_tag = (*orig).elem_tag;

            // Allocate new tuple
            let size = std::mem::size_of::<TupleObj>() + len * std::mem::size_of::<*mut Obj>();
            let new_tuple = gc_alloc(size, TypeTagKind::Tuple.tag()) as *mut TupleObj;
            (*new_tuple).len = len;
            (*new_tuple).elem_tag = elem_tag;

            // Register in memo before recursing (for cycle detection)
            memo.insert(obj_addr, new_tuple as *mut Obj);

            // Deep copy each element; root new_tuple across recursive allocs
            if len > 0 {
                let mut roots: [*mut Obj; 1] = [new_tuple as *mut Obj];
                let mut frame = ShadowFrame {
                    prev: std::ptr::null_mut(),
                    nroots: 1,
                    roots: roots.as_mut_ptr(),
                };
                gc_push(&mut frame);

                let orig_data = (*(obj as *mut TupleObj)).data.as_ptr();
                for i in 0..len {
                    let elem = *orig_data.add(i);
                    let new_elem = deep_copy_recursive(elem, memo);
                    let nt = roots[0] as *mut TupleObj;
                    *(*nt).data.as_mut_ptr().add(i) = new_elem;
                    // Keep memo entry current after the GC may have moved nothing
                    // (mark-sweep doesn't move, but keep memo consistent)
                    memo.insert(obj_addr, roots[0]);
                }

                gc_pop();

                roots[0]
            } else {
                new_tuple as *mut Obj
            }
        }

        // List - deep copy each element
        TypeTagKind::List => {
            let obj_addr = obj as usize;
            if let Some(&cached) = memo.get(&obj_addr) {
                return cached;
            }

            let orig = obj as *mut ListObj;
            let len = (*orig).len;
            let elem_tag = (*orig).elem_tag;

            // Create new list with same capacity
            let new_list = crate::list::rt_make_list(len as i64, elem_tag);

            // Register in memo before recursing
            memo.insert(obj_addr, new_list);

            // Deep copy each element; root new_list across recursive allocs
            if len > 0 {
                let mut roots: [*mut Obj; 1] = [new_list];
                let mut frame = ShadowFrame {
                    prev: std::ptr::null_mut(),
                    nroots: 1,
                    roots: roots.as_mut_ptr(),
                };
                gc_push(&mut frame);
                memo.insert(obj_addr, roots[0]);

                let orig_data = (*(obj as *mut ListObj)).data;
                for i in 0..len {
                    let elem = *orig_data.add(i);
                    let new_elem = deep_copy_recursive(elem, memo);
                    crate::list::rt_list_push(roots[0], new_elem);
                    memo.insert(obj_addr, roots[0]);
                }

                gc_pop();

                roots[0]
            } else {
                new_list
            }
        }

        // Dict - deep copy each key and value
        TypeTagKind::Dict => {
            let obj_addr = obj as usize;
            if let Some(&cached) = memo.get(&obj_addr) {
                return cached;
            }

            let orig = obj as *mut DictObj;
            let len = (*orig).len;

            let new_dict = crate::dict::rt_make_dict(len as i64);

            // Register in memo before recursing
            memo.insert(obj_addr, new_dict);

            if len > 0 {
                // Root new_dict across recursive allocs and rt_dict_set
                let mut roots: [*mut Obj; 1] = [new_dict];
                let mut frame = ShadowFrame {
                    prev: std::ptr::null_mut(),
                    nroots: 1,
                    roots: roots.as_mut_ptr(),
                };
                gc_push(&mut frame);
                memo.insert(obj_addr, roots[0]);

                for i in 0..(*orig).entries_len {
                    let entry = (*orig).entries.add(i);
                    let key = (*entry).key;
                    if !key.is_null() {
                        let value = (*entry).value;
                        let new_key = deep_copy_recursive(key, memo);
                        let new_value = deep_copy_recursive(value, memo);
                        crate::dict::rt_dict_set(roots[0], new_key, new_value);
                        memo.insert(obj_addr, roots[0]);
                    }
                }

                gc_pop();

                roots[0]
            } else {
                new_dict
            }
        }

        // Set - deep copy each element
        TypeTagKind::Set => {
            let obj_addr = obj as usize;
            if let Some(&cached) = memo.get(&obj_addr) {
                return cached;
            }

            let orig = obj as *mut SetObj;
            let len = (*orig).len;
            let capacity = (*orig).capacity;

            // Create new set with same capacity
            let new_set = crate::set::rt_make_set(capacity as i64);

            // Register in memo before recursing
            memo.insert(obj_addr, new_set);

            if len > 0 {
                // Root new_set across recursive allocs and rt_set_add
                let mut roots: [*mut Obj; 1] = [new_set];
                let mut frame = ShadowFrame {
                    prev: std::ptr::null_mut(),
                    nroots: 1,
                    roots: roots.as_mut_ptr(),
                };
                gc_push(&mut frame);
                memo.insert(obj_addr, roots[0]);

                // Iterate through original entries and deep copy
                for i in 0..capacity {
                    let entry = (*orig).entries.add(i);
                    let elem = (*entry).elem;

                    // Skip empty slots
                    if elem.is_null() || elem == crate::object::TOMBSTONE {
                        continue;
                    }

                    let new_elem = deep_copy_recursive(elem, memo);
                    crate::set::rt_set_add(roots[0], new_elem);
                    memo.insert(obj_addr, roots[0]);
                }

                gc_pop();

                roots[0]
            } else {
                new_set
            }
        }

        // Instance - deep copy each field
        TypeTagKind::Instance => {
            let obj_addr = obj as usize;
            if let Some(&cached) = memo.get(&obj_addr) {
                return cached;
            }

            let orig = obj as *mut InstanceObj;
            let class_id = (*orig).class_id;

            // Check for user-defined __deepcopy__ (simplified: no memo arg)
            let deepcopy_fn = crate::vtable::get_deepcopy_func(class_id);
            if !deepcopy_fn.is_null() {
                let deepcopy_fn: extern "C" fn(i64) -> *mut Obj = std::mem::transmute(deepcopy_fn);
                let result = deepcopy_fn(obj as i64);
                memo.insert(obj_addr, result);
                return result;
            }

            let field_count = (*orig).field_count;
            let vtable = (*orig).vtable;

            // Allocate new instance
            let size =
                std::mem::size_of::<InstanceObj>() + field_count * std::mem::size_of::<*mut Obj>();
            let new_inst = gc_alloc(size, TypeTagKind::Instance.tag()) as *mut InstanceObj;

            (*new_inst).class_id = class_id;
            (*new_inst).field_count = field_count;
            (*new_inst).vtable = vtable;

            // Register in memo before recursing
            memo.insert(obj_addr, new_inst as *mut Obj);

            // Deep copy each field, using heap_field_mask to distinguish
            // pointer fields from raw int/float/bool fields. Bits set in the
            // mask correspond to fields that hold heap pointers; unset bits
            // hold raw scalar bit-patterns (int/float/bool) that must be
            // copied verbatim — interpreting them as pointers would be UB.
            if field_count > 0 {
                // Root new_inst across recursive allocs
                let mut roots: [*mut Obj; 1] = [new_inst as *mut Obj];
                let mut frame = ShadowFrame {
                    prev: std::ptr::null_mut(),
                    nroots: 1,
                    roots: roots.as_mut_ptr(),
                };
                gc_push(&mut frame);
                memo.insert(obj_addr, roots[0]);

                let heap_mask = crate::vtable::get_class_heap_field_mask(class_id);
                let orig_fields = (*(obj as *mut InstanceObj)).fields.as_ptr();
                for i in 0..field_count {
                    let raw: *mut Obj = *orig_fields.add(i);
                    let copied: *mut Obj = if heap_mask & (1u64 << i) != 0 {
                        // Heap pointer field: recurse to produce a deep copy
                        deep_copy_recursive(raw, memo)
                    } else {
                        // Raw scalar field (int / float bits / bool): copy the
                        // bit-pattern verbatim without treating it as a pointer
                        raw
                    };
                    let ni = roots[0] as *mut InstanceObj;
                    *(*ni).fields.as_mut_ptr().add(i) = copied;
                    memo.insert(obj_addr, roots[0]);
                }

                gc_pop();

                roots[0]
            } else {
                new_inst as *mut Obj
            }
        }

        // Stateful I/O and opaque types that cannot be meaningfully deep-copied.
        // CPython raises TypeError for these as well.
        TypeTagKind::StringIO
        | TypeTagKind::BytesIO
        | TypeTagKind::File
        | TypeTagKind::Generator
        | TypeTagKind::Hash => {
            raise_exc!(
                pyaot_core_defs::BuiltinExceptionKind::TypeError,
                "cannot deepcopy this object"
            )
        }

        // Genuinely immutable / value-like types (Match, StructTime, CompletedProcess,
        // Iterator, Cell, StringBuilder) - returning the same pointer is acceptable.
        _ => obj,
    }
}
