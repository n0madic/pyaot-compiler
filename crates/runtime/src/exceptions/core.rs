//! Core exception types and internal raise machinery.
//!
//! Contains `ExceptionObject`, `ExceptionFrame`, and the internal functions
//! `dispatch_to_handler`, `raise_with_owned_message`, `dispatch_existing_exception`,
//! `copy_message_to_owned`, and exception printing utilities.

use std::ptr;

use pyaot_core_defs::layout;
use pyaot_core_defs::BuiltinExceptionKind;
use std::cell::UnsafeCell;

use super::state::{with_exception_state, with_exception_state_ref};

/// Re-export from core-defs for backwards compatibility within the runtime crate.
pub const JMP_BUF_SIZE: usize = layout::JMP_BUF_SIZE;

// Compile-time assertions that JMP_BUF_SIZE is large enough for the platform's jmp_buf.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const _: () = assert!(
    JMP_BUF_SIZE >= 192,
    "JMP_BUF_SIZE too small for macOS arm64 jmp_buf (192 bytes)"
);
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const _: () = assert!(
    JMP_BUF_SIZE >= 148,
    "JMP_BUF_SIZE too small for macOS x86_64 jmp_buf (148 bytes)"
);
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const _: () = assert!(
    JMP_BUF_SIZE >= 200,
    "JMP_BUF_SIZE too small for Linux x86_64 jmp_buf (200 bytes)"
);

/// Assert at runtime that `JMP_BUF_SIZE` is large enough for the current platform's
/// `jmp_buf`. Called from `rt_init` on startup so any mismatch fails loudly rather
/// than silently corrupting the stack.
///
/// The platform-specific sizes match the compile-time `const` assertions above;
/// this function covers any platform not handled by those `#[cfg]` guards.
pub fn assert_jmp_buf_size() {
    // Known platform sizes (bytes), mirroring the documented comment on JMP_BUF_SIZE.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    let platform_size: usize = 192;
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    let platform_size: usize = 148;
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    let platform_size: usize = 200;
    // On unrecognized platforms, skip the check rather than refuse to compile.
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
    )))]
    let platform_size: usize = 0;

    assert!(
        JMP_BUF_SIZE >= platform_size,
        "JMP_BUF_SIZE ({JMP_BUF_SIZE}) is smaller than the platform jmp_buf size \
         ({platform_size} bytes); update JMP_BUF_SIZE in exceptions.rs"
    );
}

extern "C" {
    // Note: setjmp is called directly from Cranelift-generated code, not from Rust.
    // Only longjmp is called from the runtime.

    /// longjmp: restore execution context saved by setjmp
    /// val should be non-zero (typically 1)
    pub(super) fn longjmp(env: *mut u8, val: i32) -> !;
}

/// Type alias: the runtime uses `BuiltinExceptionKind` directly from `core-defs`
/// as its exception type enum. Both share the same `#[repr(u8)]` discriminant
/// values and variant names, so there is no need for a separate runtime mirror.
pub type ExceptionType = BuiltinExceptionKind;

/// Convert a numeric tag to an exception kind, falling back to `Exception`
/// for invalid tags. This is the runtime-specific fallback behavior: instead
/// of returning `None` for unknown tags (as `BuiltinExceptionKind::from_tag`
/// does), the runtime defaults to a generic `Exception`.
#[inline]
pub const fn exception_type_from_tag(tag: u8) -> BuiltinExceptionKind {
    match BuiltinExceptionKind::from_tag(tag) {
        Some(kind) => kind,
        None => BuiltinExceptionKind::Exception,
    }
}

/// Exception handler frame (linked list on stack)
/// This structure is allocated on the stack in each function that has a try block
#[repr(C)]
pub struct ExceptionFrame {
    /// Pointer to previous exception frame in the chain
    pub prev: *mut ExceptionFrame,
    /// Jump buffer for setjmp/longjmp
    pub jmp_buf: [u8; JMP_BUF_SIZE],
    /// Saved GC shadow stack top - restored when unwinding
    pub gc_stack_top: *mut u8,
    /// Saved traceback call stack depth - restored when unwinding
    pub traceback_depth: usize,
}

// Compile-time assertions: ExceptionFrame layout must match codegen constants
const _: () = assert!(
    std::mem::size_of::<ExceptionFrame>() == layout::EXCEPTION_FRAME_SIZE as usize,
    "ExceptionFrame size does not match layout::EXCEPTION_FRAME_SIZE"
);
const _: () = assert!(
    std::mem::offset_of!(ExceptionFrame, jmp_buf) == layout::EXCEPTION_JMP_BUF_OFFSET as usize,
    "ExceptionFrame jmp_buf offset does not match layout::EXCEPTION_JMP_BUF_OFFSET"
);

impl ExceptionFrame {
    /// Create a new zeroed exception frame
    pub const fn new() -> Self {
        Self {
            prev: ptr::null_mut(),
            jmp_buf: [0u8; JMP_BUF_SIZE],
            gc_stack_top: ptr::null_mut(),
            traceback_depth: 0,
        }
    }
}

impl Default for ExceptionFrame {
    fn default() -> Self {
        Self::new()
    }
}

/// Sentinel value indicating not a custom exception class
pub const NOT_CUSTOM_CLASS: u8 = 255;

/// Exception object - stored when an exception is raised
#[repr(C)]
pub struct ExceptionObject {
    /// Type of the exception (for built-in exceptions)
    pub exc_type: ExceptionType,
    /// Custom class ID for user-defined exception classes.
    /// NOT_CUSTOM_CLASS (255) means this is a built-in exception.
    /// Values 0-28 are reserved for built-in exceptions mapped to class IDs.
    /// Values 29+ are user-defined exception classes.
    pub custom_class_id: u8,
    /// Exception message (heap-allocated string, or null)
    pub message: *const u8,
    /// Length of message in bytes
    pub message_len: usize,
    /// Capacity of the message buffer (for correct Vec reconstruction in Drop)
    pub message_capacity: usize,
    /// Explicit cause exception for `raise X from Y` (PEP 3134 __cause__)
    pub cause: Option<Box<ExceptionObject>>,
    /// Implicit context exception - captured when raising during handling (PEP 3134 __context__)
    pub context: Option<Box<ExceptionObject>>,
    /// Whether to suppress context display (True for `raise X from Y` including `from None`)
    pub suppress_context: bool,
    /// Captured traceback at the point where the exception was raised
    pub traceback: Option<Vec<crate::traceback::TracebackEntry>>,
    /// Heap-allocated exception instance (for full exception objects).
    /// Scanned by GC via `get_exception_pointers()`.
    /// null if no instance has been created yet (lazy creation for built-ins).
    pub instance: *mut crate::object::Obj,
}

impl Drop for ExceptionObject {
    fn drop(&mut self) {
        // Free the message buffer if allocated.
        // SAFETY: The message buffer is allocated via Vec in `rt_exc_raise`
        // and `copy_message_to_owned`. We reconstruct the Vec with the original
        // capacity to properly deallocate.
        if !self.message.is_null() && self.message_capacity > 0 {
            unsafe {
                let _ = Vec::from_raw_parts(
                    self.message as *mut u8,
                    self.message_len,
                    self.message_capacity,
                );
            }
        }
        // Iteratively drop cause and context chains to prevent stack overflow
        // on deeply nested exception chains (e.g., thousands of `raise ... from ...`).
        let mut cause = self.cause.take();
        while let Some(mut exc) = cause {
            cause = exc.cause.take();
            // `exc` drops here with cause=None, so no recursion
        }
        let mut context = self.context.take();
        while let Some(mut exc) = context {
            context = exc.context.take();
        }
    }
}

// ==================== GC Integration ====================

/// Collect all heap object pointers from exception state for GC root scanning.
///
/// Exception instances are stored in thread-local `ExceptionState` (Rust heap),
/// not in the GC shadow stack. When `longjmp` unwinds the shadow stack, these
/// pointers would be lost without explicit scanning. This function walks the
/// current and handling exceptions (including cause/context chains) and returns
/// all non-null instance pointers so the GC can mark them as roots.
pub fn get_exception_pointers() -> Vec<*mut crate::object::Obj> {
    with_exception_state_ref(|state| {
        let mut ptrs = Vec::new();
        fn collect_from(exc: &ExceptionObject, ptrs: &mut Vec<*mut crate::object::Obj>) {
            if !exc.instance.is_null() {
                ptrs.push(exc.instance);
            }
            if let Some(ref cause) = exc.cause {
                collect_from(cause, ptrs);
            }
            if let Some(ref context) = exc.context {
                collect_from(context, ptrs);
            }
        }
        if let Some(ref exc) = state.current_exception {
            collect_from(exc, &mut ptrs);
        }
        if let Some(ref exc) = state.handling_exception {
            collect_from(exc, &mut ptrs);
        }
        ptrs
    })
}

// ==================== Internal raise machinery ====================

/// Core exception raise logic: stores exception object and longjmps to nearest handler.
///
/// Called by `rt_exc_raise` (after copying message), `rt_exc_raise_owned` (zero-copy),
/// and other raise variants after they build their ExceptionObject.
///
/// # Safety
/// `exc_obj` must be a valid, fully initialized ExceptionObject.
pub(super) unsafe fn dispatch_to_handler(exc_obj: Box<ExceptionObject>) -> ! {
    let handler_frame = with_exception_state(|state| {
        state.current_exception = Some(exc_obj);
        state.handler_stack
    });

    // If no handler, print error and abort
    if handler_frame.is_null() {
        with_exception_state(|state| {
            if let Some(ref exc) = state.current_exception {
                print_unhandled_exception_full(exc);
            }
        });
        std::process::exit(1);
    }

    // Unwind GC stack to saved position
    let gc_stack_top = (*handler_frame).gc_stack_top;
    if !gc_stack_top.is_null() {
        crate::gc::unwind_to(gc_stack_top as *mut crate::gc::ShadowFrame);
    }

    // Unwind traceback stack to saved position
    crate::traceback::unwind_to((*handler_frame).traceback_depth);

    // Pop the handler frame (we're jumping to it)
    with_exception_state(|state| {
        state.handler_stack = (*handler_frame).prev;
    });

    // Jump to handler
    longjmp((*handler_frame).jmp_buf.as_mut_ptr(), 1);
}

/// Build an ExceptionObject from message parts and raise it.
///
/// # Safety
/// msg_ptr/msg_len/msg_capacity must form a valid owned buffer (from Vec::forget)
/// or be (null, 0, 0). Ownership of the buffer is transferred to ExceptionObject.
pub(super) unsafe fn raise_with_owned_message(
    exc_type: ExceptionType,
    msg_ptr: *const u8,
    msg_len: usize,
    msg_capacity: usize,
) -> ! {
    // Capture implicit context from any exception being handled
    // This implements Python's __context__ (PEP 3134)
    let context = with_exception_state(|state| state.handling_exception.take());

    // Capture traceback at the point of raise
    let traceback = Some(crate::traceback::capture_traceback());

    let exc_obj = Box::new(ExceptionObject {
        exc_type,
        custom_class_id: NOT_CUSTOM_CLASS,
        message: msg_ptr,
        message_len: msg_len,
        message_capacity: msg_capacity,
        cause: None,
        context,
        suppress_context: false,
        traceback,
        instance: std::ptr::null_mut(),
    });

    dispatch_to_handler(exc_obj)
}

/// Copy a message to an owned buffer, returning (ptr, len, capacity)
///
/// # Safety
/// If `len > 0`, `message` must point to valid memory of at least `len` bytes.
///
/// The returned pointer is owned and must eventually be freed by reconstructing
/// the Vec with `Vec::from_raw_parts(ptr, len, capacity)`.
pub(super) unsafe fn copy_message_to_owned(
    message: *const u8,
    len: usize,
) -> (*const u8, usize, usize) {
    if !message.is_null() && len > 0 {
        let mut msg_buf = vec![0u8; len];
        ptr::copy_nonoverlapping(message, msg_buf.as_mut_ptr(), len);
        let ptr = msg_buf.as_ptr();
        let capacity = msg_buf.capacity();
        std::mem::forget(msg_buf);
        (ptr, len, capacity)
    } else {
        (ptr::null(), 0, 0)
    }
}

/// Dispatch an already-stored exception to the nearest handler.
/// Used by rt_exc_reraise where the exception is already in current_exception.
pub(super) unsafe fn dispatch_existing_exception() -> ! {
    let handler_frame = with_exception_state(|state| state.handler_stack);

    if handler_frame.is_null() {
        with_exception_state(|state| {
            if let Some(ref exc) = state.current_exception {
                print_unhandled_exception_full(exc);
            }
        });
        std::process::exit(1);
    }

    let gc_stack_top = (*handler_frame).gc_stack_top;
    if !gc_stack_top.is_null() {
        crate::gc::unwind_to(gc_stack_top as *mut crate::gc::ShadowFrame);
    }

    crate::traceback::unwind_to((*handler_frame).traceback_depth);

    with_exception_state(|state| {
        state.handler_stack = (*handler_frame).prev;
    });

    longjmp((*handler_frame).jmp_buf.as_mut_ptr(), 1);
}

// ==================== Exception printing ====================

/// Print an exception to stderr (type name and optional message)
///
/// # Safety
/// If `msg_len > 0`, `msg_ptr` must be valid for `msg_len` bytes.
pub(super) unsafe fn print_exception_line(type_name: &str, msg_ptr: *const u8, msg_len: usize) {
    if msg_len > 0 && !msg_ptr.is_null() {
        let msg = std::slice::from_raw_parts(msg_ptr, msg_len);
        if let Ok(s) = std::str::from_utf8(msg) {
            eprintln!("{}: {}", type_name, s);
        } else {
            eprintln!("{}", type_name);
        }
    } else {
        eprintln!("{}", type_name);
    }
}

/// Print an unhandled exception with full chain support (cause and context).
/// Implements Python's exception chaining display (PEP 3134).
///
/// Display order:
/// 1. If __cause__ exists: print cause first, then "direct cause" message
/// 2. Else if __context__ exists AND !suppress_context: print context first, then "during handling" message
/// 3. Print current exception
///
/// # Safety
/// Message pointers must be valid for their indicated lengths.
pub(super) unsafe fn print_unhandled_exception_full(exc: &ExceptionObject) {
    // First, print any chained exceptions (cause takes precedence over context)
    if let Some(ref cause) = exc.cause {
        // Print the cause chain recursively
        print_unhandled_exception_full(cause);
        eprintln!();
        eprintln!("The above exception was the direct cause of the following exception:");
        eprintln!();
    } else if let Some(ref context) = exc.context {
        // Only show context if suppress_context is false
        if !exc.suppress_context {
            print_unhandled_exception_full(context);
            eprintln!();
            eprintln!("During handling of the above exception, another exception occurred:");
            eprintln!();
        }
    }

    // Print traceback before the exception line
    if let Some(ref tb) = exc.traceback {
        crate::traceback::format_traceback(tb);
    }

    // Print this exception
    if exc.custom_class_id == NOT_CUSTOM_CLASS {
        let type_name = exc.exc_type.name();
        print_exception_line(type_name, exc.message, exc.message_len);
    } else {
        let type_name = get_custom_exception_name(exc.custom_class_id);
        print_exception_line(&type_name, exc.message, exc.message_len);
    }
}

// ==================== Exception instance creation ====================

/// Number of fields in a built-in exception instance (just `.args`).
const BUILTIN_EXC_FIELD_COUNT: i64 = 1;

/// Create a heap-allocated exception instance for a built-in exception.
///
/// The instance is a regular `InstanceObj` with:
/// - class_id = exception type tag (0-27)
/// - field 0 = `.args` tuple (single-element tuple containing the message string)
///
/// # Safety
/// The `exc` must have valid message pointer/length if non-null.
pub(super) unsafe fn create_builtin_exception_instance(
    exc: &ExceptionObject,
) -> *mut crate::object::Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::object::Obj;

    // Determine class_id: use custom_class_id if set, otherwise use exc_type tag
    let class_id = if exc.custom_class_id != NOT_CUSTOM_CLASS {
        exc.custom_class_id
    } else {
        exc.exc_type as u8
    };

    // Allocate instance with 1 field (.args)
    let instance = crate::instance::rt_make_instance(class_id, BUILTIN_EXC_FIELD_COUNT);

    // Root `instance` across the subsequent allocations.  rt_make_str and
    // rt_make_tuple call gc_alloc which fires a full GC in gc_stress mode.
    // Without rooting, `instance` would be swept before we finish building it.
    // msg_str also needs rooting across rt_make_tuple.
    let mut roots: [*mut Obj; 2] = [instance, std::ptr::null_mut()];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 2,
        roots: roots.as_mut_ptr(),
    };
    gc_push(&mut frame);

    // Create the message string object (may trigger GC; instance is rooted)
    let msg_str = if exc.message_len > 0 && !exc.message.is_null() {
        crate::string::rt_make_str(exc.message, exc.message_len)
    } else {
        crate::string::rt_make_str(std::ptr::null(), 0)
    };
    // Root msg_str across rt_make_tuple
    roots[1] = msg_str;

    // Create a 1-element tuple for .args (may trigger GC; both are rooted)
    let args_tuple = crate::tuple::rt_make_tuple(1);

    gc_pop();

    // Re-derive pointers through the (non-moving) GC heap
    let instance = roots[0];
    let msg_str = roots[1];

    crate::tuple::rt_tuple_set(args_tuple, 0, msg_str);

    // Set field 0 = .args tuple
    crate::instance::rt_instance_set_field(instance, 0, args_tuple as i64);

    instance
}

// ==================== Exception Class Name Registry ====================

/// Maximum number of exception classes that can be registered
const MAX_EXCEPTION_CLASSES: usize = 256;

/// Lock-free registry for single-threaded access
struct ExcNameRegistry(UnsafeCell<[Option<String>; MAX_EXCEPTION_CLASSES]>);

// Safety: The runtime is single-threaded (AOT-compiled Python has no threading)
unsafe impl Sync for ExcNameRegistry {}

static EXCEPTION_NAME_REGISTRY: ExcNameRegistry =
    ExcNameRegistry(UnsafeCell::new([const { None }; MAX_EXCEPTION_CLASSES]));

/// Get the display name for a custom exception class.
/// Returns "CustomException<id>" if name is not registered.
pub(super) fn get_custom_exception_name(class_id: u8) -> String {
    // Try to get the registered name, fall back to generic name
    if let Some(name) = get_registered_exception_name(class_id) {
        name
    } else {
        format!("CustomException<{}>", class_id)
    }
}

/// Register a custom exception class name for display purposes.
/// This is called during module initialization to register exception class names.
pub fn register_class_name(class_id: u8, name: *const u8, len: usize) {
    if !name.is_null() && len > 0 {
        let name_slice = unsafe { std::slice::from_raw_parts(name, len) };
        if let Ok(name_str) = std::str::from_utf8(name_slice) {
            unsafe {
                (*EXCEPTION_NAME_REGISTRY.0.get())[class_id as usize] = Some(name_str.to_string());
            }
        }
    }
}

/// Get the registered name for an exception class
pub(super) fn get_registered_exception_name(class_id: u8) -> Option<String> {
    unsafe { (*EXCEPTION_NAME_REGISTRY.0.get())[class_id as usize].clone() }
}
