use crate::gc::gc_alloc;
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
                for i in 0..(*orig).entries_len {
                    let entry = (*orig).entries.add(i);
                    let key = (*entry).key;
                    if !key.is_null() {
                        crate::dict::rt_dict_set(new_dict, key, (*entry).value);
                    }
                }
            }

            new_dict
        }

        // Set - shallow copy
        TypeTagKind::Set => {
            let orig = obj as *mut SetObj;
            let len = (*orig).len;
            let capacity = (*orig).capacity;

            // Create new set with same capacity
            let new_set = crate::set::rt_make_set(capacity as i64);

            if len > 0 {
                // Iterate through original entries and copy to new set
                for i in 0..capacity {
                    let entry = (*orig).entries.add(i);
                    let elem = (*entry).elem;

                    // Skip empty slots (TOMBSTONE or null)
                    if elem.is_null() || elem == crate::object::TOMBSTONE {
                        continue;
                    }

                    crate::set::rt_set_add(new_set, elem);
                }
            }

            new_set
        }

        // Instance - shallow copy fields
        TypeTagKind::Instance => {
            let orig = obj as *mut InstanceObj;
            let field_count = (*orig).field_count;
            let class_id = (*orig).class_id;
            let vtable = (*orig).vtable;

            // Allocate new instance
            let size =
                std::mem::size_of::<InstanceObj>() + field_count * std::mem::size_of::<*mut Obj>();
            let new_inst = gc_alloc(size, TypeTagKind::Instance.tag()) as *mut InstanceObj;

            (*new_inst).class_id = class_id;
            (*new_inst).field_count = field_count;
            (*new_inst).vtable = vtable;

            // Copy field pointers
            if field_count > 0 {
                let orig_fields = (*orig).fields.as_ptr();
                let new_fields = (*new_inst).fields.as_mut_ptr();
                std::ptr::copy_nonoverlapping(orig_fields, new_fields, field_count);
            }

            new_inst as *mut Obj
        }

        // Other types (Iterator, Cell, Generator, Match, File, StringBuilder,
        // StructTime, CompletedProcess, Hash, StringIO, BytesIO) - return as-is
        // These types either have internal state that shouldn't be copied or
        // are immutable/stateful objects
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

            // Deep copy each element
            if len > 0 {
                let orig_data = (*orig).data.as_ptr();
                let new_data = (*new_tuple).data.as_mut_ptr();
                for i in 0..len {
                    let elem = *orig_data.add(i);
                    let new_elem = deep_copy_recursive(elem, memo);
                    *new_data.add(i) = new_elem;
                }
            }

            new_tuple as *mut Obj
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

            // Deep copy each element
            if len > 0 {
                let orig_data = (*orig).data;
                for i in 0..len {
                    let elem = *orig_data.add(i);
                    let new_elem = deep_copy_recursive(elem, memo);
                    crate::list::rt_list_push(new_list, new_elem);
                }
            }

            new_list
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
                for i in 0..(*orig).entries_len {
                    let entry = (*orig).entries.add(i);
                    let key = (*entry).key;
                    if !key.is_null() {
                        let value = (*entry).value;
                        let new_key = deep_copy_recursive(key, memo);
                        let new_value = deep_copy_recursive(value, memo);
                        crate::dict::rt_dict_set(new_dict, new_key, new_value);
                    }
                }
            }

            new_dict
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
                // Iterate through original entries and deep copy
                for i in 0..capacity {
                    let entry = (*orig).entries.add(i);
                    let elem = (*entry).elem;

                    // Skip empty slots
                    if elem.is_null() || elem == crate::object::TOMBSTONE {
                        continue;
                    }

                    let new_elem = deep_copy_recursive(elem, memo);
                    crate::set::rt_set_add(new_set, new_elem);
                }
            }

            new_set
        }

        // Instance - deep copy each field
        TypeTagKind::Instance => {
            let obj_addr = obj as usize;
            if let Some(&cached) = memo.get(&obj_addr) {
                return cached;
            }

            let orig = obj as *mut InstanceObj;
            let field_count = (*orig).field_count;
            let class_id = (*orig).class_id;
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

            // Deep copy each field
            if field_count > 0 {
                let orig_fields = (*orig).fields.as_ptr();
                let new_fields = (*new_inst).fields.as_mut_ptr();
                for i in 0..field_count {
                    let field = *orig_fields.add(i);
                    let new_field = deep_copy_recursive(field, memo);
                    *new_fields.add(i) = new_field;
                }
            }

            new_inst as *mut Obj
        }

        // Other types (Iterator, Cell, Generator, Match, File, StringBuilder,
        // StructTime, CompletedProcess, Hash, StringIO, BytesIO) - return as-is
        // These types either have internal state that shouldn't be copied or
        // are immutable/stateful objects
        _ => obj,
    }
}
