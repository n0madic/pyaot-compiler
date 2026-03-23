//! VTable (Virtual Table) support for class inheritance
//!
//! This module provides the vtable infrastructure for virtual method dispatch
//! and inheritance-aware isinstance checks.

use std::sync::RwLock;

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

/// Global class registry for inheritance information
/// Index is class_id, value is ClassInfo
static CLASS_REGISTRY: RwLock<[ClassInfo; MAX_CLASSES]> = RwLock::new(
    [ClassInfo {
        parent_class_id: NO_PARENT,
        heap_field_mask: u64::MAX, // Default: treat all fields as heap (conservative)
    }; MAX_CLASSES],
);

/// Wrapper for raw pointer that implements Send + Sync
/// Safety: The vtable pointers are only written during initialization
/// and read during method dispatch. The RwLock provides synchronization.
#[derive(Copy, Clone)]
struct VtablePtr(*const u8);
unsafe impl Send for VtablePtr {}
unsafe impl Sync for VtablePtr {}

impl VtablePtr {
    const fn null() -> Self {
        VtablePtr(std::ptr::null())
    }
}

/// Global vtable registry: maps class_id to vtable pointer
/// Each vtable is an array of function pointers for virtual methods
static VTABLE_REGISTRY: RwLock<[VtablePtr; MAX_CLASSES]> =
    RwLock::new([VtablePtr::null(); MAX_CLASSES]);

/// Register a class with its parent
/// class_id: The class being registered
/// parent_class_id: The parent class ID (0 if no parent)
#[no_mangle]
pub extern "C" fn rt_register_class(class_id: u8, parent_class_id: u8) {
    match CLASS_REGISTRY.write() {
        Ok(mut registry) => {
            registry[class_id as usize].parent_class_id = parent_class_id;
        }
        Err(_) => {
            eprintln!(
                "WARNING: rt_register_class: CLASS_REGISTRY lock poisoned, class {} not registered",
                class_id
            );
        }
    }
}

/// Register the heap field mask for a class (tells GC which fields are heap pointers)
/// heap_field_mask: bitmask where bit i = 1 means field i is a heap object pointer
#[no_mangle]
pub extern "C" fn rt_register_class_fields(class_id: u8, heap_field_mask: i64) {
    match CLASS_REGISTRY.write() {
        Ok(mut registry) => {
            registry[class_id as usize].heap_field_mask = heap_field_mask as u64;
        }
        Err(_) => {
            eprintln!(
                "WARNING: rt_register_class_fields: CLASS_REGISTRY lock poisoned, class {} fields not registered",
                class_id
            );
        }
    }
}

/// Get the heap field mask for a class (used by GC during marking)
pub fn get_class_heap_field_mask(class_id: u8) -> u64 {
    if let Ok(registry) = CLASS_REGISTRY.read() {
        registry[class_id as usize].heap_field_mask
    } else {
        u64::MAX // Conservative: treat all fields as heap
    }
}

/// Register a vtable for a class
/// class_id: The class ID
/// vtable_ptr: Pointer to the vtable (array of function pointers)
#[no_mangle]
pub extern "C" fn rt_register_vtable(class_id: u8, vtable_ptr: *const u8) {
    match VTABLE_REGISTRY.write() {
        Ok(mut registry) => {
            registry[class_id as usize] = VtablePtr(vtable_ptr);
        }
        Err(_) => {
            eprintln!(
                "WARNING: rt_register_vtable: VTABLE_REGISTRY lock poisoned, class {} vtable not registered",
                class_id
            );
        }
    }
}

/// Get the vtable pointer for a class
/// Returns the vtable pointer, or null if not registered
#[no_mangle]
pub extern "C" fn rt_get_vtable(class_id: u8) -> *const u8 {
    if let Ok(registry) = VTABLE_REGISTRY.read() {
        registry[class_id as usize].0
    } else {
        std::ptr::null()
    }
}

/// Lookup a method in the vtable by slot index
/// vtable_ptr: Pointer to the vtable
/// slot: The slot index in the vtable
/// Returns the function pointer at that slot
///
/// # Safety
/// The caller must ensure that vtable_ptr is a valid pointer to a vtable
/// structure and that slot is within bounds of the vtable.
#[no_mangle]
pub unsafe extern "C" fn rt_vtable_lookup(vtable_ptr: *const u8, slot: usize) -> *const u8 {
    if vtable_ptr.is_null() {
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
/// Returns 0 if no parent (base class) or class not found
#[no_mangle]
pub extern "C" fn rt_get_parent_class(class_id: u8) -> u8 {
    if let Ok(registry) = CLASS_REGISTRY.read() {
        registry[class_id as usize].parent_class_id
    } else {
        0
    }
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
    if let Ok(registry) = CLASS_REGISTRY.read() {
        // Limit iterations to MAX_CLASSES to prevent infinite loops from circular inheritance
        for _ in 0..MAX_CLASSES {
            let parent = registry[current as usize].parent_class_id;
            if parent == NO_PARENT {
                // Reached the top of the hierarchy without finding target
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
/// This maps exception type tags (0-12) to class IDs with proper inheritance:
/// - Exception (0) is the root (no parent)
/// - All other built-in exceptions inherit from Exception (0)
///
/// This function is idempotent and thread-safe.
#[no_mangle]
pub extern "C" fn rt_init_builtin_exception_classes() {
    INIT_BUILTIN_EXCEPTIONS.call_once(|| {
        if let Ok(mut registry) = CLASS_REGISTRY.write() {
            // Exception (0) - base class, no parent
            registry[0] = ClassInfo {
                parent_class_id: NO_PARENT,
                heap_field_mask: u64::MAX,
            };
            // All other built-in exceptions inherit from Exception (0)
            for i in 1..=12 {
                registry[i] = ClassInfo {
                    parent_class_id: 0,
                    heap_field_mask: u64::MAX,
                };
            }
        }
    });
}

/// Get the first available class ID for user classes.
/// Built-in exception classes use IDs 0-12, so user classes start at 13.
pub const FIRST_USER_CLASS_ID: u8 = 13;
