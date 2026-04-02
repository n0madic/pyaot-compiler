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
// InstanceObj layout (vtable-based dispatch)
// =============================================================================

/// Offset of the vtable pointer inside `InstanceObj` (right after `ObjHeader`).
pub const INSTANCE_VTABLE_OFFSET: i32 = OBJ_HEADER_SIZE as i32;

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
// ExceptionFrame layout
// =============================================================================

/// Size of `jmp_buf` in bytes. Must be large enough for all supported
/// platforms (macOS arm64: 192, macOS x86_64: 148, Linux x86_64: 200).
pub const JMP_BUF_SIZE: usize = 200;

/// Offset of `jmp_buf` field within `ExceptionFrame` (after `prev` pointer).
pub const EXCEPTION_JMP_BUF_OFFSET: i32 = PTR_SIZE as i32;

/// Total size of `ExceptionFrame` in bytes.
///
/// ```text
/// ExceptionFrame {
///     prev: *mut ExceptionFrame,   // 8 bytes
///     jmp_buf: [u8; JMP_BUF_SIZE], // JMP_BUF_SIZE bytes
///     gc_stack_top: *mut u8,       // 8 bytes
///     traceback_depth: usize,      // 8 bytes
/// }
/// ```
pub const EXCEPTION_FRAME_SIZE: u32 =
    PTR_SIZE as u32 + JMP_BUF_SIZE as u32 + PTR_SIZE as u32 + PTR_SIZE as u32;
