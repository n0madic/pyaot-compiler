//! Class instance operations for Python runtime

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::gc;
use crate::object::{Obj, TypeTagKind};

/// Create a new instance of a class
/// class_id: ID of the class (used for type checking)
/// field_count: number of fields in the class
/// Returns: pointer to allocated InstanceObj
#[no_mangle]
pub extern "C" fn rt_make_instance(class_id: u8, field_count: i64) -> *mut Obj {
    use crate::object::{InstanceObj, TypeTagKind};
    use crate::vtable::rt_get_vtable;

    let field_count = field_count.max(0) as usize;

    // Pre-§F.7 the GC used a per-class 64-bit mask, so classes with
    // more than 64 fields lost precise tracing. With §F.7 the mark
    // walk is mask-free (`Value::is_ptr()` per slot), so this limit is
    // gone; the warning is retained as a soft hint that very wide
    // classes may still indicate a design smell.
    if field_count > 64 {
        eprintln!(
            "WARNING: class_id {} has {} fields. Consider splitting wide classes.",
            class_id, field_count
        );
    }

    // Calculate size using size_of::<InstanceObj>() so that struct padding between
    // fields (e.g., the 7 padding bytes after class_id: u8 before field_count: usize
    // on 64-bit targets) is accounted for correctly. The flexible array member
    // `fields: [*mut Obj; 0]` contributes 0 bytes, so we add the field storage
    // separately.
    let size = std::mem::size_of::<InstanceObj>()
        + field_count * std::mem::size_of::<pyaot_core_defs::Value>();

    // Allocate using GC
    let obj = gc::gc_alloc(size, TypeTagKind::Instance as u8);

    unsafe {
        let instance = obj as *mut InstanceObj;
        // Set vtable pointer from the global registry
        (*instance).vtable = rt_get_vtable(class_id);
        (*instance).class_id = class_id;
        (*instance).field_count = field_count;

        // Initialize all fields to empty (Value(0))
        let fields_ptr = (*instance).fields.as_mut_ptr();
        for i in 0..field_count {
            *fields_ptr.add(i) = pyaot_core_defs::Value(0);
        }
    }

    obj
}

/// Get a field from an instance by offset
/// inst: pointer to the instance
/// offset: field offset (0-based)
/// Returns: field value as i64 (raw value for primitives, pointer for heap types)
#[no_mangle]
pub extern "C" fn rt_instance_get_field(inst: *mut Obj, offset: i64) -> i64 {
    if inst.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(inst, TypeTagKind::Instance, "rt_instance_get_field");
        let instance = inst as *mut crate::object::InstanceObj;
        let field_count = (*instance).field_count as i64;

        // Bounds check
        if offset < 0 || offset >= field_count {
            debug_assert!(
                false,
                "rt_instance_get_field: offset {} out of bounds (field_count={})",
                offset, field_count
            );
            return 0;
        }

        let fields_ptr = (*instance).fields.as_ptr();
        (*fields_ptr.add(offset as usize)).0 as i64
    }
}

/// Set a field in an instance by offset
/// inst: pointer to the instance
/// offset: field offset (0-based)
/// value: the value to store (raw i64 for primitives, pointer cast to i64 for heap types)
#[no_mangle]
pub extern "C" fn rt_instance_set_field(inst: *mut Obj, offset: i64, value: i64) {
    if inst.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(inst, TypeTagKind::Instance, "rt_instance_set_field");
        let instance = inst as *mut crate::object::InstanceObj;
        let field_count = (*instance).field_count as i64;

        // Bounds check
        if offset < 0 || offset >= field_count {
            debug_assert!(
                false,
                "rt_instance_set_field: offset {} out of bounds (field_count={})",
                offset, field_count
            );
            return;
        }

        let fields_ptr = (*instance).fields.as_mut_ptr();
        *fields_ptr.add(offset as usize) = pyaot_core_defs::Value(value as u64);
    }
}

/// Get the class ID of an instance
/// Returns: class ID, or 0 if null
#[no_mangle]
pub extern "C" fn rt_instance_get_class_id(inst: *mut Obj) -> u8 {
    if inst.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(inst, TypeTagKind::Instance, "rt_instance_get_class_id");
        let instance = inst as *mut crate::object::InstanceObj;
        (*instance).class_id
    }
}

/// Get the type tag of an object
/// Returns: type tag as i64
///
/// Handles the case where `obj` might not be a valid heap pointer (e.g., a raw
/// function pointer from a closure/decorator). Non-aligned or obviously invalid
/// pointers return the Instance tag as a safe fallback.
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_get_type_tag(obj: *mut Obj) -> i64 {
    if obj.is_null() {
        return crate::object::TypeTagKind::None as i64;
    }
    let v = pyaot_core_defs::Value(obj as u64);
    if v.is_int() {
        return crate::object::TypeTagKind::Int as i64;
    }
    if v.is_bool() {
        return crate::object::TypeTagKind::Bool as i64;
    }
    if v.is_none() {
        return crate::object::TypeTagKind::None as i64;
    }
    unsafe { (*obj).type_tag() as i64 }
}

/// Check if an object is an instance of a specific class
/// obj: pointer to object
/// class_id: expected class ID
/// Returns: 1 if isinstance, 0 otherwise
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_isinstance_class(obj: *mut Obj, class_id: i64) -> i8 {
    if obj.is_null() {
        return 0;
    }
    // Validate alignment before dereferencing
    if !(obj as usize).is_multiple_of(std::mem::align_of::<Obj>()) {
        return 0;
    }

    unsafe {
        let type_tag = (*obj).type_tag();
        if type_tag != crate::object::TypeTagKind::Instance {
            return 0;
        }

        let instance = obj as *mut crate::object::InstanceObj;
        if (*instance).class_id == class_id as u8 {
            1
        } else {
            0
        }
    }
}

/// Check if an object is an instance of a specific class or any of its parent classes
/// This supports inheritance: isinstance(Dog(), Animal) returns True if Dog inherits from Animal
/// obj: pointer to object
/// target_class_id: the class ID to check against
/// Returns: 1 if isinstance (directly or through inheritance), 0 otherwise
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_isinstance_class_inherited(obj: *mut Obj, target_class_id: i64) -> i8 {
    if obj.is_null() {
        return 0;
    }
    // Validate alignment before dereferencing (mirrors rt_isinstance_class)
    if !(obj as usize).is_multiple_of(std::mem::align_of::<crate::object::Obj>()) {
        return 0;
    }

    unsafe {
        let type_tag = (*obj).type_tag();
        if type_tag != crate::object::TypeTagKind::Instance {
            return 0;
        }

        let instance = obj as *mut crate::object::InstanceObj;
        let obj_class_id = (*instance).class_id;

        // Use the vtable module to check inheritance
        crate::vtable::rt_class_inherits_from(obj_class_id, target_class_id as u8)
    }
}

/// Check if child_vtable is a subclass of parent_vtable
/// Returns: 1 (true) or 0 (false)
#[no_mangle]
pub extern "C" fn rt_issubclass(child_vtable: i64, parent_vtable: i64) -> i8 {
    // Use the vtable module to check inheritance
    // vtable IDs are class IDs as u8
    crate::vtable::rt_class_inherits_from(child_vtable as u8, parent_vtable as u8)
}
