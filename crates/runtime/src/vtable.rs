//! VTable (Virtual Table) support for class inheritance
//!
//! This module provides the vtable infrastructure for virtual method dispatch
//! and inheritance-aware isinstance checks.

use std::cell::UnsafeCell;

/// Maximum number of classes supported
const MAX_CLASSES: usize = 256;

/// Sentinel value for "no parent" (class is at the root of hierarchy)
const NO_PARENT: u8 = 255;

/// Class inheritance information for runtime type checks
/// This is a simplified structure for tracking parent relationships
#[repr(C)]
#[derive(Copy, Clone)]
pub struct ClassInfo {
    /// Parent class ID (NO_PARENT = no parent, i.e., base class)
    pub parent_class_id: u8,
    /// Bitmask of which fields are heap objects (pointers) that the GC must trace.
    /// Bit i is set if field i is a heap type (str, list, dict, tuple, set, class instance, etc.)
    /// Bit i is clear if field i is a raw value (int, float, bool, None).
    /// Supports up to 64 fields per class.
    pub heap_field_mask: u64,
}

/// Lock-free registry storage for single-threaded access
struct RegistryStorage<T: Copy, const N: usize>(UnsafeCell<[T; N]>);

// Safety: The runtime is single-threaded (AOT-compiled Python has no threading)
unsafe impl<T: Copy, const N: usize> Sync for RegistryStorage<T, N> {}

/// Global class registry for inheritance information
/// Index is class_id, value is ClassInfo
static CLASS_REGISTRY: RegistryStorage<ClassInfo, MAX_CLASSES> = RegistryStorage(UnsafeCell::new(
    [ClassInfo {
        parent_class_id: NO_PARENT,
        heap_field_mask: 0, // Default: treat no fields as heap (fail-safe; classes must register)
    }; MAX_CLASSES],
));

/// Wrapper for raw pointer that implements Copy
#[derive(Copy, Clone)]
struct VtablePtr(*const u8);

// Safety: Vtable pointers are written once during init, then read-only
unsafe impl Send for VtablePtr {}
unsafe impl Sync for VtablePtr {}

impl VtablePtr {
    const fn null() -> Self {
        VtablePtr(std::ptr::null())
    }
}

/// Global vtable registry: maps class_id to vtable pointer
/// Each vtable is an array of function pointers for virtual methods
static VTABLE_REGISTRY: RegistryStorage<VtablePtr, MAX_CLASSES> =
    RegistryStorage(UnsafeCell::new([VtablePtr::null(); MAX_CLASSES]));

/// Register a class with its parent
#[no_mangle]
pub extern "C" fn rt_register_class(class_id: u8, parent_class_id: u8) {
    unsafe {
        (*CLASS_REGISTRY.0.get())[class_id as usize].parent_class_id = parent_class_id;
    }
}

/// Register the heap field mask for a class (tells GC which fields are heap pointers)
/// Passed as i64 for Cranelift ABI compatibility; bit pattern is preserved as u64.
#[no_mangle]
pub extern "C" fn rt_register_class_fields(class_id: u8, heap_field_mask: i64) {
    unsafe {
        (*CLASS_REGISTRY.0.get())[class_id as usize].heap_field_mask = heap_field_mask as u64;
    }
}

/// Get the heap field mask for a class (used by GC during marking)
#[inline]
pub fn get_class_heap_field_mask(class_id: u8) -> u64 {
    unsafe { (*CLASS_REGISTRY.0.get())[class_id as usize].heap_field_mask }
}

/// Register a vtable for a class
#[no_mangle]
pub extern "C" fn rt_register_vtable(class_id: u8, vtable_ptr: *const u8) {
    unsafe {
        (*VTABLE_REGISTRY.0.get())[class_id as usize] = VtablePtr(vtable_ptr);
    }
}

/// Get the vtable pointer for a class
#[no_mangle]
pub extern "C" fn rt_get_vtable(class_id: u8) -> *const u8 {
    unsafe { (*VTABLE_REGISTRY.0.get())[class_id as usize].0 }
}

/// Lookup a method in the vtable by slot index
///
/// # Safety
/// The caller must ensure that vtable_ptr is a valid pointer to a vtable
/// structure and that slot is within bounds of the vtable.
#[no_mangle]
pub unsafe extern "C" fn rt_vtable_lookup(vtable_ptr: *const u8, slot: usize) -> *const u8 {
    if vtable_ptr.is_null() {
        return std::ptr::null();
    }
    // Validate alignment for reading a usize
    if !(vtable_ptr as usize).is_multiple_of(std::mem::align_of::<usize>()) {
        return std::ptr::null();
    }
    // Vtable layout: [num_slots: usize, method_ptrs: [*const (); num_slots]]
    let num_slots = *(vtable_ptr as *const usize);
    if slot >= num_slots {
        return std::ptr::null();
    }
    // Skip the num_slots field (8 bytes) and index into method_ptrs
    let methods_ptr = vtable_ptr.add(std::mem::size_of::<usize>()) as *const *const u8;
    *methods_ptr.add(slot)
}

/// Get the parent class ID for a given class
#[no_mangle]
pub extern "C" fn rt_get_parent_class(class_id: u8) -> u8 {
    unsafe { (*CLASS_REGISTRY.0.get())[class_id as usize].parent_class_id }
}

/// Check if a class inherits from another class (directly or indirectly)
/// Returns 1 if child_class_id is or inherits from target_class_id, 0 otherwise
#[no_mangle]
pub extern "C" fn rt_class_inherits_from(child_class_id: u8, target_class_id: u8) -> i8 {
    // Same class is considered "inherits from" itself
    if child_class_id == target_class_id {
        return 1;
    }

    // Walk up the parent chain
    let mut current = child_class_id;
    unsafe {
        let registry = &*CLASS_REGISTRY.0.get();
        // Limit iterations to MAX_CLASSES to prevent infinite loops from circular inheritance
        for _ in 0..MAX_CLASSES {
            let parent = registry[current as usize].parent_class_id;
            if parent == NO_PARENT {
                return 0;
            }
            if parent == target_class_id {
                return 1;
            }
            current = parent;
        }
    }
    0
}

// ==================== Built-in Exception Class Registration ====================

use std::sync::Once;

static INIT_BUILTIN_EXCEPTIONS: Once = Once::new();

/// Initialize built-in exception classes in the class registry.
/// This maps exception type tags (0-27) to class IDs with proper inheritance:
/// - Exception (0) is the root (no parent)
/// - All other built-in exceptions inherit from Exception (0)
///
/// This function is idempotent and thread-safe.
#[no_mangle]
pub extern "C" fn rt_init_builtin_exception_classes() {
    INIT_BUILTIN_EXCEPTIONS.call_once(|| unsafe {
        let registry = &mut *CLASS_REGISTRY.0.get();
        // Exception (0) - base class, no parent
        registry[0] = ClassInfo {
            parent_class_id: NO_PARENT,
            heap_field_mask: u64::MAX,
        };
        // All other built-in exceptions (tags 1-27) inherit from Exception (0)
        for entry in registry
            .iter_mut()
            .take(pyaot_core_defs::BUILTIN_EXCEPTION_COUNT as usize)
            .skip(1)
        {
            *entry = ClassInfo {
                parent_class_id: 0,
                heap_field_mask: u64::MAX,
            };
        }
    });
}

/// Get the first available class ID for user classes.
/// Built-in exception classes use IDs 0-27, so user classes start at 28.
pub const FIRST_USER_CLASS_ID: u8 = pyaot_core_defs::BUILTIN_EXCEPTION_COUNT;

// ==================== Method Name Registry (for Protocol dispatch) ====================
// Maps (class_id, method_name_hash) → vtable_slot for name-based virtual dispatch.

const MAX_METHODS_PER_CLASS: usize = 64;

/// Entry in the per-class method name table
#[derive(Copy, Clone)]
struct MethodNameEntry {
    name_hash: u64,
    slot: usize,
}

/// Per-class method name table: array of (hash, slot) pairs
#[derive(Copy, Clone)]
struct MethodNameTable {
    entries: [MethodNameEntry; MAX_METHODS_PER_CLASS],
    count: usize,
}

impl MethodNameTable {
    const fn new() -> Self {
        Self {
            entries: [MethodNameEntry {
                name_hash: 0,
                slot: 0,
            }; MAX_METHODS_PER_CLASS],
            count: 0,
        }
    }
}

/// Global method name registry: maps class_id → method name table
static METHOD_NAME_REGISTRY: RegistryStorage<MethodNameTable, MAX_CLASSES> =
    RegistryStorage(UnsafeCell::new({
        // const array initialization
        const EMPTY: MethodNameTable = MethodNameTable::new();
        [EMPTY; MAX_CLASSES]
    }));

/// Register a method name → vtable slot mapping for a class.
/// Called during class initialization for every instance method.
#[no_mangle]
pub extern "C" fn rt_register_method_name(class_id: i64, name_hash: i64, slot: i64) {
    unsafe {
        let registry = &mut *METHOD_NAME_REGISTRY.0.get();
        let table = &mut registry[class_id as usize];
        if table.count >= MAX_METHODS_PER_CLASS {
            eprintln!(
                "WARNING: class {} exceeds maximum methods per class ({}), method with hash {} dropped",
                class_id, MAX_METHODS_PER_CLASS, name_hash
            );
            return;
        }
        table.entries[table.count] = MethodNameEntry {
            name_hash: name_hash as u64,
            slot: slot as usize,
        };
        table.count += 1;
    }
}

/// Look up a vtable slot by method name hash on the ACTUAL runtime object.
/// Returns the function pointer for the method, or null if not found.
/// This is used for Protocol dispatch where the compile-time Protocol's vtable
/// slots may differ from the concrete class's slots.
#[no_mangle]
pub extern "C" fn rt_vtable_lookup_by_name(obj_ptr: *mut u8, name_hash: i64) -> *const u8 {
    if obj_ptr.is_null() {
        return std::ptr::null();
    }
    unsafe {
        // Validate that this is actually an InstanceObj by checking the type tag.
        // The type_tag field is the first byte of ObjHeader, which is at offset 0.
        let type_tag_byte = *obj_ptr;
        if pyaot_core_defs::TypeTagKind::from_tag(type_tag_byte)
            != Some(pyaot_core_defs::TypeTagKind::Instance)
        {
            return std::ptr::null();
        }

        // Get class_id from the InstanceObj (offset 25 from obj start)
        // InstanceObj layout: ObjHeader(16) + vtable_ptr(8) + class_id(1)
        let class_id = *obj_ptr.add(24) as usize;

        // Look up slot by name hash in the method name table
        let registry = &*METHOD_NAME_REGISTRY.0.get();
        let table = &registry[class_id];
        let target_hash = name_hash as u64;
        for i in 0..table.count {
            if table.entries[i].name_hash == target_hash {
                let slot = table.entries[i].slot;
                // Now do the standard vtable lookup
                let vtable_ptr = *(obj_ptr.add(16) as *const *const u8); // vtable at offset 16
                return rt_vtable_lookup(vtable_ptr, slot);
            }
        }
        std::ptr::null()
    }
}
