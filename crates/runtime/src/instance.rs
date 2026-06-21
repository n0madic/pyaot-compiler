//! Class instance operations for Python runtime

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::gc;
use crate::object::{Obj, TypeTagKind};
use pyaot_core_defs::Value;

/// Create a new instance of a class
/// class_id: ID of the class (used for type checking)
/// field_count: number of fields in the class
/// Returns: pointer to allocated InstanceObj
pub fn rt_make_instance(class_id: u8, field_count: i64) -> *mut Obj {
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
#[export_name = "rt_make_instance"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_make_instance_abi(class_id: u8, field_count: i64) -> Value {
    Value::from_ptr(rt_make_instance(class_id, field_count))
}

/// Get a field from an instance by offset
/// inst: pointer to the instance
/// offset: field offset (0-based)
/// Returns: field value as i64 (raw value for primitives, pointer for heap types)
pub fn rt_instance_get_field(inst: *mut Obj, offset: i64) -> i64 {
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
#[export_name = "rt_instance_get_field"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_instance_get_field_abi(inst: Value, offset: i64) -> i64 {
    unsafe {
        rt_instance_get_field(
            crate::utils::expect_ptr_or_type_error(inst, "rt_instance_get_field"),
            offset,
        )
    }
}

/// Set a field in an instance by offset
/// inst: pointer to the instance
/// offset: field offset (0-based)
/// value: the value to store (raw i64 for primitives, pointer cast to i64 for heap types)
pub fn rt_instance_set_field(inst: *mut Obj, offset: i64, value: i64) {
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
#[export_name = "rt_instance_set_field"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_instance_set_field_abi(inst: Value, offset: i64, value: i64) {
    unsafe {
        rt_instance_set_field(
            crate::utils::expect_ptr_or_type_error(inst, "rt_instance_set_field"),
            offset,
            value,
        )
    }
}

/// Get the class ID of an instance
/// Returns: class ID, or 0 if null
pub fn rt_instance_get_class_id(inst: *mut Obj) -> u8 {
    if inst.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(inst, TypeTagKind::Instance, "rt_instance_get_class_id");
        let instance = inst as *mut crate::object::InstanceObj;
        (*instance).class_id
    }
}
#[export_name = "rt_instance_get_class_id"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_instance_get_class_id_abi(inst: Value) -> u8 {
    if !inst.is_ptr() {
        return 0;
    }
    rt_instance_get_class_id(inst.unwrap_ptr())
}

/// Get the type tag of an object
/// Returns: type tag as i64
///
/// Handles the case where `obj` might not be a valid heap pointer (e.g., a raw
/// function pointer from a closure/decorator). Non-aligned or obviously invalid
/// pointers return the Instance tag as a safe fallback.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_get_type_tag(obj: *mut Obj) -> i64 {
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
#[export_name = "rt_get_type_tag"]
pub extern "C" fn rt_get_type_tag_abi(obj: Value) -> i64 {
    rt_get_type_tag(obj.unwrap_ptr())
}

/// Check if an object is an instance of a specific class
/// obj: pointer to object
/// class_id: expected class ID
/// Returns: 1 if isinstance, 0 otherwise
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_isinstance_class(obj: *mut Obj, class_id: i64) -> i8 {
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
#[export_name = "rt_isinstance_class"]
pub extern "C" fn rt_isinstance_class_abi(obj: Value, class_id: i64) -> i8 {
    if !obj.is_ptr() {
        return 0;
    }
    rt_isinstance_class(obj.unwrap_ptr(), class_id)
}

/// Check if an object is an instance of a specific class or any of its parent classes
/// This supports inheritance: isinstance(Dog(), Animal) returns True if Dog inherits from Animal
/// obj: pointer to object
/// target_class_id: the class ID to check against
/// Returns: 1 if isinstance (directly or through inheritance), 0 otherwise
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_isinstance_class_inherited(obj: *mut Obj, target_class_id: i64) -> i8 {
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
#[export_name = "rt_isinstance_class_inherited"]
pub extern "C" fn rt_isinstance_class_inherited_abi(obj: Value, target_class_id: i64) -> i8 {
    if !obj.is_ptr() {
        return 0;
    }
    rt_isinstance_class_inherited(obj.unwrap_ptr(), target_class_id)
}

/// True iff `obj` is a heap pointer whose type tag is `tag` (null- and
/// alignment-checked, like `rt_isinstance_class_inherited`). Immediates
/// (`int`/`bool`/`None`) are never heap pointers, so they return `false`.
fn heap_tag_is(obj: Value, tag: TypeTagKind) -> bool {
    if !obj.is_ptr() {
        return false;
    }
    let ptr: *mut Obj = obj.unwrap_ptr();
    if ptr.is_null() || !(ptr as usize).is_multiple_of(std::mem::align_of::<Obj>()) {
        return false;
    }
    unsafe { (*ptr).type_tag() == tag }
}

/// `isinstance(obj, T)` against a builtin type `T` (by KIND) for a gradual
/// (`Dyn`/`Union`) value, inspecting the runtime tag. `kind` is a
/// [`pyaot_core_defs::isinstance_kind`] code. Used when the verdict cannot be
/// folded statically (e.g. a dunder param typed `Dyn`); a statically-typed
/// receiver still folds at lowering. Matches by Python `type` KIND: `bool ⊂
/// int` (a `bool` value satisfies `isinstance(x, int)`), big integers are
/// `int`, container element types are ignored.
#[no_mangle]
pub extern "C" fn rt_isinstance_builtin(obj: Value, kind: i64) -> i8 {
    use pyaot_core_defs::isinstance_kind as k;
    let verdict = match kind {
        k::INT => {
            obj.is_int()
                || obj.is_bool()
                || heap_tag_is(obj, TypeTagKind::Int)
                || heap_tag_is(obj, TypeTagKind::BigInt)
        }
        k::BOOL => obj.is_bool() || heap_tag_is(obj, TypeTagKind::Bool),
        k::FLOAT => heap_tag_is(obj, TypeTagKind::Float),
        k::STR => heap_tag_is(obj, TypeTagKind::Str),
        k::BYTES => heap_tag_is(obj, TypeTagKind::Bytes),
        k::LIST => heap_tag_is(obj, TypeTagKind::List),
        k::DICT => heap_tag_is(obj, TypeTagKind::Dict),
        k::SET => heap_tag_is(obj, TypeTagKind::Set),
        k::TUPLE => heap_tag_is(obj, TypeTagKind::Tuple),
        k::FROZENSET => heap_tag_is(obj, TypeTagKind::FrozenSet),
        k::BYTEARRAY => heap_tag_is(obj, TypeTagKind::ByteArray),
        _ => false,
    };
    verdict as i8
}

/// The Python `type_name` of a runtime `Value`, for guard error messages.
/// Immediates (`int`/`bool`/`None`) report their immediate kind; a heap pointer
/// reports its tag's `type_name` (a null pointer reports `NoneType`).
fn value_type_name(v: Value) -> &'static str {
    if v.is_int() {
        return "int";
    }
    if v.is_bool() {
        return "bool";
    }
    if v.is_none() {
        return "NoneType";
    }
    let obj: *mut Obj = v.unwrap_ptr_or_null();
    if obj.is_null() {
        return "NoneType";
    }
    unsafe { (*obj).type_tag().type_name() }
}

/// The gradual builtin-container shape guard (the `Heap` analogue of
/// `rt_unbox_float`). When a genuinely-`Dyn` value flows into a typed
/// `str`/`bytes`/`list`/`dict`/`set`/`tuple` parameter / return / slot, lowering
/// routes it through this CHECKED coercion: if the runtime tag matches `kind` (a
/// [`pyaot_core_defs::isinstance_kind`] code) the same tagged `Value` is
/// returned untouched; otherwise a `TypeError` is raised AT THE BOUNDARY instead
/// of a deferred SIGSEGV at the first container op. `dict` is family-aware
/// (`Dict`/`DefaultDict`/`Counter` share one layout); the rest are subtype-free
/// singletons (user classes cannot subclass builtins). An unrecognised `kind`
/// passes through unchanged — defensive only; lowering never emits a checked
/// heap coerce for a guard-less shape (PITFALLS B18).
#[export_name = "rt_check_heap_kind"]
pub extern "C" fn rt_check_heap_kind_abi(value: Value, kind: i64) -> Value {
    use pyaot_core_defs::isinstance_kind as k;
    let (ok, expected) = match kind {
        k::STR => (heap_tag_is(value, TypeTagKind::Str), "str"),
        k::BYTES => (heap_tag_is(value, TypeTagKind::Bytes), "bytes"),
        k::LIST => (heap_tag_is(value, TypeTagKind::List), "list"),
        k::SET => (heap_tag_is(value, TypeTagKind::Set), "set"),
        k::TUPLE => (heap_tag_is(value, TypeTagKind::Tuple), "tuple"),
        k::DICT => (
            heap_tag_is(value, TypeTagKind::Dict)
                || heap_tag_is(value, TypeTagKind::DefaultDict)
                || heap_tag_is(value, TypeTagKind::Counter),
            "dict",
        ),
        // No guard for this kind — pass through (should be unreachable).
        _ => return value,
    };
    if ok {
        return value;
    }
    unsafe {
        raise_exc!(
            crate::exceptions::ExceptionType::TypeError,
            "expected {}, got {}",
            expected,
            value_type_name(value)
        );
    }
}

/// The gradual user-class instance shape guard (subclass-aware sibling
/// of `rt_check_heap_kind`). A genuinely-`Dyn` value flowing into a typed
/// class-instance parameter / return / slot is routed here: it must be a heap
/// `Instance` whose class is `class_id` OR a subclass of it
/// (`rt_class_inherits_from`), so a `Dog` legitimately passes an `Animal` param.
/// On a match the same tagged `Value` is returned untouched; otherwise a
/// `TypeError` is raised at the boundary.
#[export_name = "rt_check_instance"]
pub extern "C" fn rt_check_instance_abi(value: Value, class_id: i64) -> Value {
    if rt_isinstance_class_inherited_abi(value, class_id) != 0 {
        return value;
    }
    let expected = lookup_class_qualname(class_id as u8).unwrap_or_else(|| "object".to_string());
    unsafe {
        raise_exc!(
            crate::exceptions::ExceptionType::TypeError,
            "expected {}, got {}",
            expected,
            value_type_name(value)
        );
    }
}

// =============================================================================
// Class qualified-name registry (for the default object repr)
// =============================================================================

use std::cell::UnsafeCell;

/// Max class_id range (class_id is u8); mirrors `vtable::MAX_CLASSES`.
const MAX_QUALNAME_CLASSES: usize = 256;

struct ClassQualnameRegistry(UnsafeCell<[Option<String>; MAX_QUALNAME_CLASSES]>);
// Safety: the AOT runtime is single-threaded; the registry is populated once
// at module init before any instance is rendered.
unsafe impl Sync for ClassQualnameRegistry {}

static CLASS_QUALNAME_REGISTRY: ClassQualnameRegistry =
    ClassQualnameRegistry(UnsafeCell::new([const { None }; MAX_QUALNAME_CLASSES]));

/// Register a class's qualified name (e.g. `"__main__.Widget"`) for the
/// default object repr. Emitted once per class at module init. `name` is a
/// `StrObj` pointer (the `Value::from_ptr` encoding is identity for aligned
/// pointers, so the raw operand bits are the pointer).
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_register_class_qualname(class_id: i64, name: *mut Obj) {
    use crate::object::StrObj;
    if name.is_null() || class_id < 0 || class_id >= MAX_QUALNAME_CLASSES as i64 {
        return;
    }
    unsafe {
        let s = name as *mut StrObj;
        let len = (*s).len;
        let bytes = std::slice::from_raw_parts((*s).data.as_ptr(), len);
        if let Ok(text) = std::str::from_utf8(bytes) {
            (*CLASS_QUALNAME_REGISTRY.0.get())[class_id as usize] = Some(text.to_string());
        }
    }
}

/// Look up a class's registered qualified name by class id.
pub(crate) fn lookup_class_qualname(class_id: u8) -> Option<String> {
    unsafe { (*CLASS_QUALNAME_REGISTRY.0.get())[class_id as usize].clone() }
}

/// Default repr string for a class instance — `<{qualname} object at 0x..>`,
/// matching CPython's `<__main__.Widget object at 0x..>`. Falls back to
/// `<object at 0x..>` when the class registered no qualified name. Callers
/// have already confirmed the instance has no user `__repr__`/`__str__`.
///
/// # Safety
/// `obj` must be a valid object pointer or a tagged primitive `Value`.
pub(crate) unsafe fn instance_default_repr(obj: *mut Obj) -> String {
    use crate::object::InstanceObj;
    let v = Value(obj as u64);
    if v.is_ptr() && !obj.is_null() && (*obj).type_tag() == TypeTagKind::Instance {
        let class_id = (*(obj as *const InstanceObj)).class_id;
        if let Some(qn) = lookup_class_qualname(class_id) {
            return format!("<{} object at {:p}>", qn, obj);
        }
    }
    format!("<object at {:p}>", obj)
}

#[cfg(test)]
mod plan1_guard_tests {
    //! The gradual heap-arg shape guards (`rt_check_heap_kind` /
    //! `rt_check_instance`).
    //!
    //! Only the ACCEPT paths and the reject DECISION predicates are exercised
    //! here: a guard's reject path calls `raise_exc!` → `std::process::exit(1)`
    //! when no handler protects the frame (an uncaught runtime exception), which
    //! cannot be caught by `#[should_panic]` (it is an unwind/exit, not a Rust
    //! panic). The heap-kind reject path is covered end-to-end by the
    //! differential gate (`corpus/p46_heap_arg_guard.py` — a `Dyn` int into a
    //! `list` param → `TypeError`); the class reject path, which diverges from
    //! CPython (it raises `AttributeError` at `.name`, not `TypeError`), is
    //! pinned here by its decision predicate instead.

    use super::*;
    use crate::{counter, dict, gc, list, set};
    use pyaot_core_defs::isinstance_kind as k;

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        crate::RUNTIME_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn value_type_name_reports_python_names() {
        let _g = lock();
        gc::init();
        assert_eq!(value_type_name(Value::from_int(7)), "int");
        assert_eq!(value_type_name(Value::TRUE), "bool");
        assert_eq!(value_type_name(Value::NONE), "NoneType");
        let lst = Value::from_ptr(list::rt_make_list(2));
        assert_eq!(value_type_name(lst), "list");
    }

    #[test]
    fn check_heap_kind_accepts_matching_shape() {
        let _g = lock();
        gc::init();
        // Root the containers so GC-stress mode cannot sweep them across the
        // sequence of allocations below.
        let lst = list::rt_make_list(2);
        let st = set::rt_make_set(2);
        let dct = dict::rt_make_dict(2);
        let ctr = counter::rt_make_counter_empty();
        let mut roots: [*mut Obj; 4] = [lst, st, dct, ctr];
        let mut frame = gc::ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 4,
            roots: roots.as_mut_ptr(),
        };
        unsafe { gc::gc_push(&mut frame) };

        let lst_v = Value::from_ptr(roots[0]);
        assert_eq!(rt_check_heap_kind_abi(lst_v, k::LIST), lst_v);
        let set_v = Value::from_ptr(roots[1]);
        assert_eq!(rt_check_heap_kind_abi(set_v, k::SET), set_v);
        // `dict` is family-aware: a plain Dict AND a Counter (shared layout)
        // both satisfy the DICT guard.
        let dct_v = Value::from_ptr(roots[2]);
        assert_eq!(rt_check_heap_kind_abi(dct_v, k::DICT), dct_v);
        let ctr_v = Value::from_ptr(roots[3]);
        assert_eq!(rt_check_heap_kind_abi(ctr_v, k::DICT), ctr_v);

        gc::gc_pop();
    }

    #[test]
    fn check_heap_kind_decision_rejects_wrong_shape() {
        let _g = lock();
        gc::init();
        // The predicate that drives the raise: an `int` is not a `list`, so the
        // guard WOULD raise `TypeError("expected list, got int")`.
        let int_v = Value::from_int(42);
        assert!(!heap_tag_is(int_v, TypeTagKind::List));
        assert!(!heap_tag_is(int_v, TypeTagKind::Dict));
        assert_eq!(value_type_name(int_v), "int");
    }

    #[test]
    fn check_instance_accepts_self_and_subclass() {
        let _g = lock();
        gc::init();
        // Animal(201) is a base class; Dog(200) inherits from it.
        const ANIMAL: u8 = 201;
        const DOG: u8 = 200;
        const UNRELATED: i64 = 202;
        crate::vtable::rt_register_class(ANIMAL, 255 /* NO_PARENT */);
        crate::vtable::rt_register_class(DOG, ANIMAL);

        let dog = rt_make_instance(DOG, 1);
        let mut roots: [*mut Obj; 1] = [dog];
        let mut frame = gc::ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        unsafe { gc::gc_push(&mut frame) };
        let dog_v = Value::from_ptr(roots[0]);

        // A Dog passes its own class and its Animal base (subclass-aware).
        assert_eq!(rt_check_instance_abi(dog_v, DOG as i64), dog_v);
        assert_eq!(rt_check_instance_abi(dog_v, ANIMAL as i64), dog_v);

        // The reject decisions (would raise): a Dog is not an UNRELATED class,
        // and a non-instance immediate is not any class.
        assert_eq!(rt_isinstance_class_inherited_abi(dog_v, UNRELATED), 0);
        assert_eq!(
            rt_isinstance_class_inherited_abi(Value::from_int(1), ANIMAL as i64),
            0
        );

        gc::gc_pop();
    }
}
