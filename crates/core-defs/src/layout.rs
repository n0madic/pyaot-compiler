//! Object layout constants shared between compiler and runtime.
//!
//! These constants define the memory layout of runtime objects and must match
//! the `#[repr(C)]` struct definitions in the runtime crate. Compile-time
//! assertions in the runtime verify agreement.

/// Pointer/word size on the target platform (64-bit only).
pub const PTR_SIZE: usize = 8;

// =============================================================================
// ObjHeader layout
// =============================================================================

/// Size of `ObjHeader` in bytes.
///
/// ```text
/// ObjHeader {
///     type_tag: TypeTagKind,  // 1 byte
///     marked: bool,           // 1 byte
///     /* padding */           // 6 bytes
///     size: usize,            // 8 bytes
/// }
/// ```
pub const OBJ_HEADER_SIZE: usize = 16;

// =============================================================================
// FloatObj layout
// =============================================================================

/// Byte offset of the `value: f64` field inside `FloatObj`.
///
/// ```text
/// FloatObj { header: ObjHeader(16 bytes), value: f64(8 bytes at offset 16) }
/// ```
pub const FLOAT_OBJ_VALUE_OFFSET: i32 = OBJ_HEADER_SIZE as i32;

// =============================================================================
// InstanceObj layout (vtable-based dispatch)
// =============================================================================

/// Offset of the vtable pointer inside `InstanceObj` (right after `ObjHeader`).
pub const INSTANCE_VTABLE_OFFSET: i32 = OBJ_HEADER_SIZE as i32;

/// Byte offset of the flexible `fields: [Value; 0]` array inside `InstanceObj`.
///
/// ```text
/// InstanceObj {
///     header:      ObjHeader   (16 bytes, offset 0)
///     vtable:      *const u8   ( 8 bytes, offset 16)
///     class_id:    u8          ( 1 byte,  offset 24)
///     /* padding               ( 7 bytes, offset 25) */
///     field_count: usize       ( 8 bytes, offset 32)
///     fields:      [Value; 0]  ( 0 bytes, offset 40)  ← this constant
/// }
/// ```
pub const INSTANCE_FIELDS_OFFSET: i32 = OBJ_HEADER_SIZE as i32 + 3 * PTR_SIZE as i32; // 16 + 24 = 40

/// Offset of the first method pointer inside a vtable data section.
///
/// Vtable layout: `[num_slots: u64, method_ptrs: [*const (); num_slots]]`
pub const VTABLE_METHODS_OFFSET: usize = PTR_SIZE;

/// Byte offset of a specific method slot within a vtable.
pub const fn vtable_slot_offset(slot: usize) -> i32 {
    (VTABLE_METHODS_OFFSET + slot * PTR_SIZE) as i32
}

/// Total byte size of a vtable data section with `num_slots` method pointers.
pub const fn vtable_data_size(num_slots: usize) -> usize {
    PTR_SIZE + num_slots * PTR_SIZE
}

// =============================================================================
// GeneratorObj layout
// =============================================================================

/// Offset of `func_id` field inside `GeneratorObj` (right after `ObjHeader`).
///
/// ```text
/// GeneratorObj {
///     header: ObjHeader,  // OBJ_HEADER_SIZE bytes
///     func_id: u32,       // at offset OBJ_HEADER_SIZE
///     ...
/// }
/// ```
pub const GENERATOR_FUNC_ID_OFFSET: i32 = OBJ_HEADER_SIZE as i32;

// =============================================================================
// GC ShadowFrame layout
// =============================================================================

/// Size of the `ShadowFrame` struct on the stack.
///
/// ```text
/// ShadowFrame {
///     prev: *mut ShadowFrame,    // 8 bytes (offset 0)
///     nroots: usize,             // 8 bytes (offset 8)
///     roots: *mut *mut Obj,      // 8 bytes (offset 16)
/// }
/// ```
pub const SHADOW_FRAME_SIZE: u32 = 3 * PTR_SIZE as u32;

/// Offset of `nroots` field in `ShadowFrame`.
pub const SHADOW_FRAME_NROOTS_OFFSET: i32 = PTR_SIZE as i32;

/// Offset of `roots` pointer field in `ShadowFrame`.
pub const SHADOW_FRAME_ROOTS_OFFSET: i32 = 2 * PTR_SIZE as i32;

/// Byte size of the GC roots array for `nroots` entries.
pub const fn gc_roots_array_size(nroots: usize) -> u32 {
    (nroots * PTR_SIZE) as u32
}

/// Byte offset of a specific root slot in the roots array.
pub const fn gc_root_offset(root_idx: usize) -> i32 {
    (root_idx * PTR_SIZE) as i32
}

// =============================================================================
// Unwind-table record layout (table-based exception handling)
// =============================================================================
//
// Codegen emits one data object per program: `count` records, each describing
// one protected machine call site. The runtime resolves `func_addr + site_off`
// to the absolute return-PC at registration, sorts, and binary-searches at
// raise time.
//
// ```text
// ExcTableRecord {
//     func_addr: *const u8,  // 8 bytes — relocated function base address
//     site_off: u32,         // return-address offset within the function
//     handler_off: u32,      // handler entry offset within the function
//     frame_off: u32,        // FP-to-SP distance at the call site
//     _pad: u32,
// }
// ```

/// Size of one unwind-table record in bytes.
pub const EXC_TABLE_RECORD_SIZE: u32 = 24;
/// Offset of `site_off` within a record (after the function pointer).
pub const EXC_RECORD_SITE_OFF_OFFSET: u32 = 8;
/// Offset of `handler_off` within a record.
pub const EXC_RECORD_HANDLER_OFF_OFFSET: u32 = 12;
/// Offset of `frame_off` within a record.
pub const EXC_RECORD_FRAME_OFF_OFFSET: u32 = 16;
