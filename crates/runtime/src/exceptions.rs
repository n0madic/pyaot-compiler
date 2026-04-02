//! Exception handling support using setjmp/longjmp
//!
//! This module provides exception handling infrastructure for the AOT compiler.
//! It uses setjmp/longjmp for control flow with thread-local exception state.
//!
//! # Exception Type Definitions
//!
//! Exception types are defined in `pyaot-core-defs` crate, which serves as the
//! single source of truth shared between the compiler and runtime.

use pyaot_core_defs::layout;
use pyaot_core_defs::BuiltinExceptionKind;
use std::cell::RefCell;
use std::ptr;

/// Re-export from core-defs for backwards compatibility within the runtime crate.
pub const JMP_BUF_SIZE: usize = layout::JMP_BUF_SIZE;

// Compile-time assertions that JMP_BUF_SIZE is large enough for the platform's jmp_buf.
// Sizes are taken from each platform's <setjmp.h>. The libc crate does not expose
// jmp_buf on all platforms, so we use known sizes from the documented comment above.
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
    fn longjmp(env: *mut u8, val: i32) -> !;
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

/// Thread-local exception state
struct ExceptionState {
    /// Pointer to current exception object (owned, must be freed)
    current_exception: Option<Box<ExceptionObject>>,
    /// Stack of exception handler frames (linked list)
    handler_stack: *mut ExceptionFrame,
    /// Exception being handled in current except block (for __context__ capture)
    /// When we enter an except handler, we save the current exception here.
    /// If a new exception is raised during handling, this becomes its __context__.
    handling_exception: Option<Box<ExceptionObject>>,
}

impl ExceptionState {
    const fn new() -> Self {
        Self {
            current_exception: None,
            handler_stack: ptr::null_mut(),
            handling_exception: None,
        }
    }
}

// Thread-local storage for exception state
thread_local! {
    static EXCEPTION_STATE: RefCell<ExceptionState> = const { RefCell::new(ExceptionState::new()) };
}

/// Helper to access exception state (mutable)
fn with_exception_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut ExceptionState) -> R,
{
    EXCEPTION_STATE.with(|state| f(&mut state.borrow_mut()))
}

/// Helper to access exception state (read-only, non-panicking).
/// Used by GC to collect exception pointers. Uses `try_borrow` so it returns a
/// default value instead of panicking when the RefCell is already mutably borrowed
/// (a re-entrant scenario that can theoretically occur during gc_stress testing).
fn with_exception_state_ref<F, R: Default>(f: F) -> R
where
    F: FnOnce(&ExceptionState) -> R,
{
    EXCEPTION_STATE.with(|state| match state.try_borrow() {
        Ok(guard) => f(&guard),
        Err(_) => R::default(),
    })
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

// ==================== Public API (C ABI) ====================

/// Push an exception frame onto the handler stack
/// Called at the start of a try block
///
/// # Safety
/// `frame` must be a valid pointer to an ExceptionFrame that will remain
/// valid until the corresponding rt_exc_pop_frame is called.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_push_frame(frame: *mut ExceptionFrame) {
    if frame.is_null() {
        return;
    }

    with_exception_state(|state| {
        // Link to previous frame
        (*frame).prev = state.handler_stack;
        state.handler_stack = frame;

        // Save current GC stack top for unwinding
        // This is obtained from the GC module
        (*frame).gc_stack_top = crate::gc::get_stack_top() as *mut u8;

        // Save current traceback stack depth for unwinding
        (*frame).traceback_depth = crate::traceback::current_depth();
    });
}

/// Pop an exception frame from the handler stack
/// Called at normal exit from a try block (no exception occurred)
#[no_mangle]
pub extern "C" fn rt_exc_pop_frame() {
    with_exception_state(|state| {
        if !state.handler_stack.is_null() {
            unsafe {
                state.handler_stack = (*state.handler_stack).prev;
            }
        }
    });
}

// Note: setjmp is now called directly from Cranelift-generated code (not through
// a Rust wrapper) to avoid UB. When setjmp is called from a wrapper function that
// returns, the later longjmp tries to restore a dead stack frame — causing SIGILL
// in debug builds. The codegen computes jmp_buf address as frame_ptr + 8 and calls
// setjmp directly.

/// Core exception raise logic: stores exception object and longjmps to nearest handler.
///
/// Called by `rt_exc_raise` (after copying message), `rt_exc_raise_owned` (zero-copy),
/// and other raise variants after they build their ExceptionObject.
///
/// # Safety
/// `exc_obj` must be a valid, fully initialized ExceptionObject.
unsafe fn dispatch_to_handler(exc_obj: Box<ExceptionObject>) -> ! {
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
unsafe fn raise_with_owned_message(
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

/// Raise an exception with the given type and message
/// This function does not return - it longjmps to the nearest handler
///
/// # Safety
/// If `len > 0`, `message` must be a valid pointer to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_raise(exc_type_tag: u8, message: *const u8, len: usize) -> ! {
    let exc_type = exception_type_from_tag(exc_type_tag);
    let (msg_ptr, msg_len, msg_capacity) = copy_message_to_owned(message, len);
    raise_with_owned_message(exc_type, msg_ptr, msg_len, msg_capacity)
}

/// Raise an exception, taking ownership of a heap-allocated message buffer.
///
/// Unlike `rt_exc_raise` which copies the message, this function takes direct
/// ownership of the caller's buffer, avoiding the leak that occurs when longjmp
/// skips Rust destructors. This eliminates both the leak AND an unnecessary copy.
///
/// # Safety
/// - `msg_ptr` must have been obtained from `String::as_mut_ptr()` on a valid String
/// - `msg_len` and `msg_capacity` must be the String's len() and capacity()
/// - The caller must have called `std::mem::forget()` on the String before this call
/// - If msg_ptr is null, msg_len and msg_capacity must both be 0
pub unsafe fn rt_exc_raise_owned(
    exc_type_tag: u8,
    msg_ptr: *mut u8,
    msg_len: usize,
    msg_capacity: usize,
) -> ! {
    let exc_type = exception_type_from_tag(exc_type_tag);
    let (ptr, len, cap) = if !msg_ptr.is_null() && msg_len > 0 {
        (msg_ptr as *const u8, msg_len, msg_capacity)
    } else {
        (ptr::null(), 0, 0)
    };
    raise_with_owned_message(exc_type, ptr, len, cap)
}

/// Copy a message to an owned buffer, returning (ptr, len, capacity)
///
/// # Safety
/// If `len > 0`, `message` must point to valid memory of at least `len` bytes.
///
/// The returned pointer is owned and must eventually be freed by reconstructing
/// the Vec with `Vec::from_raw_parts(ptr, len, capacity)`.
unsafe fn copy_message_to_owned(message: *const u8, len: usize) -> (*const u8, usize, usize) {
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

/// Raise an exception with a cause (`raise X from Y`)
/// When an explicit cause is provided, suppress_context is set to True.
///
/// # Safety
/// If message/cause_message lengths are non-zero, their pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_raise_from(
    exc_type_tag: u8,
    message: *const u8,
    len: usize,
    cause_type_tag: u8,
    cause_message: *const u8,
    cause_len: usize,
) -> ! {
    // Convert type tags to ExceptionType using macro-generated from_tag
    let exc_type = exception_type_from_tag(exc_type_tag);
    let cause_type = exception_type_from_tag(cause_type_tag);

    let (msg_ptr, msg_len, msg_capacity) = copy_message_to_owned(message, len);
    let (cause_msg_ptr, cause_msg_len, cause_msg_capacity) =
        copy_message_to_owned(cause_message, cause_len);

    // Capture implicit context (may still be relevant for debugging)
    let context = with_exception_state(|state| state.handling_exception.take());

    // Build cause exception object (no traceback of its own)
    let cause_obj = Box::new(ExceptionObject {
        exc_type: cause_type,
        custom_class_id: NOT_CUSTOM_CLASS,
        message: cause_msg_ptr,
        message_len: cause_msg_len,
        message_capacity: cause_msg_capacity,
        cause: None,
        context: None,
        suppress_context: false,
        traceback: None,
        instance: std::ptr::null_mut(),
    });

    // Capture traceback at the point of raise
    let traceback = Some(crate::traceback::capture_traceback());

    // Build main exception object with cause
    // When explicit cause is provided, suppress_context = true
    let exc_obj = Box::new(ExceptionObject {
        exc_type,
        custom_class_id: NOT_CUSTOM_CLASS,
        message: msg_ptr,
        message_len: msg_len,
        message_capacity: msg_capacity,
        cause: Some(cause_obj),
        context,
        suppress_context: true, // Explicit cause suppresses context display
        traceback,
        instance: std::ptr::null_mut(),
    });

    dispatch_to_handler(exc_obj)
}

/// Raise an exception with context suppressed (`raise X from None`)
/// This sets suppress_context = true and cause = None, effectively hiding the context chain.
///
/// # Safety
/// If `len > 0`, `message` must be a valid pointer to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_raise_from_none(
    exc_type_tag: u8,
    message: *const u8,
    len: usize,
) -> ! {
    let exc_type = exception_type_from_tag(exc_type_tag);
    let (msg_ptr, msg_len, msg_capacity) = copy_message_to_owned(message, len);

    // Still capture context for debugging (it's stored but not displayed)
    let context = with_exception_state(|state| state.handling_exception.take());

    // Capture traceback at the point of raise
    let traceback = Some(crate::traceback::capture_traceback());

    // Build exception with suppressed context (no cause, context suppressed)
    let exc_obj = Box::new(ExceptionObject {
        exc_type,
        custom_class_id: NOT_CUSTOM_CLASS,
        message: msg_ptr,
        message_len: msg_len,
        message_capacity: msg_capacity,
        cause: None,
        context,
        suppress_context: true, // "from None" suppresses context display
        traceback,
        instance: std::ptr::null_mut(),
    });

    dispatch_to_handler(exc_obj)
}

/// Called when entering an except handler block.
/// Preserves the current exception in handling_exception so if a new exception
/// is raised during handling, the original becomes its __context__.
#[no_mangle]
pub extern "C" fn rt_exc_start_handling() {
    with_exception_state(|state| {
        // Take the current exception and save it as the one being handled
        // A new exception raised during handling will capture this as __context__
        if let Some(exc) = state.current_exception.take() {
            state.handling_exception = Some(exc);
        }
    });
}

/// Called when exiting an except handler block normally (not via exception).
/// Clears the handling_exception since we're done handling.
#[no_mangle]
pub extern "C" fn rt_exc_end_handling() {
    with_exception_state(|state| {
        // Clear the handled exception since we're done with the handler
        let _ = state.handling_exception.take();
    });
}

/// Print an exception to stderr (type name and optional message)
///
/// # Safety
/// If `msg_len > 0`, `msg_ptr` must be valid for `msg_len` bytes.
unsafe fn print_exception_line(type_name: &str, msg_ptr: *const u8, msg_len: usize) {
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
unsafe fn print_unhandled_exception_full(exc: &ExceptionObject) {
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

/// Re-raise the current exception (bare `raise` statement)
/// This is used when an except block wants to propagate the exception
///
/// # Safety
/// This function uses longjmp to unwind the stack to the nearest exception handler.
/// The caller must ensure a valid exception handler frame exists on the handler stack.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_reraise() -> ! {
    // First, restore the exception from handling_exception if it exists
    // This handles the case where we're reraising after calling rt_exc_start_handling
    with_exception_state(|state| {
        if state.current_exception.is_none() {
            if let Some(exc) = state.handling_exception.take() {
                state.current_exception = Some(exc);
            }
        }
    });

    // Check if there's actually an exception to re-raise
    let has_exception = with_exception_state(|state| state.current_exception.is_some());
    if !has_exception {
        // No active exception - raise RuntimeError (matches CPython behavior)
        crate::utils::raise_runtime_error("No active exception to re-raise");
    }

    // Re-dispatch the existing exception (already stored in current_exception)
    dispatch_existing_exception()
}

/// Dispatch an already-stored exception to the nearest handler.
/// Used by rt_exc_reraise where the exception is already in current_exception.
unsafe fn dispatch_existing_exception() -> ! {
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

/// Get the current exception type tag
/// Returns the type tag of current exception, or -1 if no exception.
/// Checks both current_exception and handling_exception (the latter is set
/// after ExcStartHandling in except handlers, e.g. context manager __exit__).
#[no_mangle]
pub extern "C" fn rt_exc_get_type() -> i32 {
    with_exception_state(|state| {
        let exc = state
            .current_exception
            .as_ref()
            .or(state.handling_exception.as_ref());
        if let Some(exc) = exc {
            exc.exc_type as i32
        } else {
            -1
        }
    })
}

/// Check if there is a current exception
/// Returns 1 if exception is pending, 0 otherwise
#[no_mangle]
pub extern "C" fn rt_exc_has_exception() -> i8 {
    with_exception_state(|state| {
        if state.current_exception.is_some() {
            1
        } else {
            0
        }
    })
}

/// Clear the current exception (after it has been handled)
#[no_mangle]
pub extern "C" fn rt_exc_clear() {
    with_exception_state(|state| {
        // Taking the exception out of the Option and dropping it
        // will trigger Drop::drop which frees the message buffer
        // and recursively drops the cause chain
        let _ = state.current_exception.take();
    });
}

/// Get current exception message as a pointer and length.
/// Returns null pointer and 0 length if no exception.
///
/// # Lifetime constraint
/// The returned pointer borrows the message buffer that is owned by the
/// `ExceptionObject` stored in thread-local `EXCEPTION_STATE.current_exception`.
/// The pointer is valid only for as long as the current exception remains set —
/// i.e., until `rt_exc_clear`, `rt_exc_reraise`, or any call that mutates
/// `current_exception` is made.
///
/// In generated AOT code this function is called in the immediate preamble of an
/// `except` handler, before any call that could clear or replace the exception,
/// so the pointer is always valid at the point of use. **Do not store the returned
/// pointer across a call that may raise or clear an exception.**
///
/// If a longer-lived copy of the message is required, copy the bytes into an
/// owned buffer before calling any other runtime function that might disturb the
/// exception state.
///
/// # Safety
/// The caller must ensure `out_len` is either null or points to valid writable memory.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_get_message(out_len: *mut usize) -> *const u8 {
    with_exception_state(|state| {
        if let Some(ref exc) = state.current_exception {
            unsafe {
                if !out_len.is_null() {
                    *out_len = exc.message_len;
                }
            }
            exc.message
        } else {
            unsafe {
                if !out_len.is_null() {
                    *out_len = 0;
                }
            }
            ptr::null()
        }
    })
}

/// Print the current exception (for debugging or unhandled exceptions)
#[no_mangle]
pub extern "C" fn rt_exc_print_current() {
    with_exception_state(|state| {
        if let Some(ref exc) = state.current_exception {
            unsafe {
                print_unhandled_exception_full(exc);
            }
        }
    });
}

/// Helper function to raise a ValueError with a message
/// This is a convenience wrapper around rt_exc_raise for ValueError exceptions
///
/// # Safety
/// If `len > 0`, `message` must be a valid pointer to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_raise_value_error(message: *const u8, len: usize) -> ! {
    rt_exc_raise(ExceptionType::ValueError as u8, message, len)
}

/// Helper function to raise a TypeError with a message
///
/// # Safety
/// If `len > 0`, `message` must be a valid pointer to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_raise_type_error(message: *const u8, len: usize) -> ! {
    rt_exc_raise(ExceptionType::TypeError as u8, message, len)
}

/// Helper function to raise an IndexError with a message
///
/// # Safety
/// If `len > 0`, `message` must be a valid pointer to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_raise_index_error(message: *const u8, len: usize) -> ! {
    rt_exc_raise(ExceptionType::IndexError as u8, message, len)
}

/// Helper function to raise a KeyError with a message
///
/// # Safety
/// If `len > 0`, `message` must be a valid pointer to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_raise_key_error(message: *const u8, len: usize) -> ! {
    rt_exc_raise(ExceptionType::KeyError as u8, message, len)
}

/// Helper function to raise an AttributeError with a message
///
/// # Safety
/// If `len > 0`, `message` must be a valid pointer to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_raise_attr_error(message: *const u8, len: usize) -> ! {
    rt_exc_raise(ExceptionType::AttributeError as u8, message, len)
}

/// Helper function to raise an IOError with a message
///
/// # Safety
/// If `len > 0`, `message` must be a valid pointer to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_raise_io_error(message: *const u8, len: usize) -> ! {
    rt_exc_raise(ExceptionType::IOError as u8, message, len)
}

/// Helper function to raise a ZeroDivisionError with a message
///
/// # Safety
/// If `len > 0`, `message` must be a valid pointer to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_raise_zero_division_error(message: *const u8, len: usize) -> ! {
    rt_exc_raise(ExceptionType::ZeroDivisionError as u8, message, len)
}

/// Helper function to raise an OverflowError with a message
///
/// # Safety
/// If `len > 0`, `message` must be a valid pointer to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_raise_overflow_error(message: *const u8, len: usize) -> ! {
    rt_exc_raise(ExceptionType::OverflowError as u8, message, len)
}

/// Helper function to raise a MemoryError with a message
///
/// # Safety
/// If `len > 0`, `message` must be a valid pointer to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_raise_memory_error(message: *const u8, len: usize) -> ! {
    rt_exc_raise(ExceptionType::MemoryError as u8, message, len)
}

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
unsafe fn create_builtin_exception_instance(exc: &ExceptionObject) -> *mut crate::object::Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::object::{Obj, ELEM_HEAP_OBJ};

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
    let args_tuple = crate::tuple::rt_make_tuple(1, ELEM_HEAP_OBJ);

    gc_pop();

    // Re-derive pointers through the (non-moving) GC heap
    let instance = roots[0];
    let msg_str = roots[1];

    crate::tuple::rt_tuple_set(args_tuple, 0, msg_str);

    // Set field 0 = .args tuple
    crate::instance::rt_instance_set_field(instance, 0, args_tuple as i64);

    instance
}

/// Get the current exception as a heap-allocated exception instance.
/// Returns an `InstanceObj` with `.args` tuple and exception class_id.
/// For built-in exceptions, the instance is created lazily on first access.
/// For custom exceptions with a pre-created instance, returns that instance.
/// Returns null if no exception is pending.
///
/// This function checks both current_exception and handling_exception, as the exception
/// may have been moved to handling_exception by rt_exc_start_handling().
#[no_mangle]
pub extern "C" fn rt_exc_get_current() -> *mut crate::object::Obj {
    // Fast path: return an already-created instance (no allocation needed).
    let existing = with_exception_state(|state| {
        let exc = state
            .current_exception
            .as_ref()
            .or(state.handling_exception.as_ref());
        exc.map(|e| e.instance)
    });

    match existing {
        None => return std::ptr::null_mut(),
        Some(ptr) if !ptr.is_null() => return ptr,
        _ => {} // Need to create instance lazily
    }

    // Collect the exception data needed for instance creation.
    // Release the RefCell borrow BEFORE calling any allocating functions.
    // create_builtin_exception_instance calls rt_make_instance / rt_make_str /
    // rt_make_tuple, all of which call gc_alloc.  In gc_stress mode gc_alloc
    // fires a full GC that calls get_exception_pointers() → with_exception_state
    // → borrow_mut() — which panics with BorrowMutError if we are still holding
    // the outer borrow_mut here.
    let (exc_type, custom_class_id, msg_ptr, msg_len) = with_exception_state(|state| {
        let exc = state
            .current_exception
            .as_ref()
            .or(state.handling_exception.as_ref());
        match exc {
            Some(e) => (e.exc_type, e.custom_class_id, e.message, e.message_len),
            None => (
                ExceptionType::Exception,
                NOT_CUSTOM_CLASS,
                std::ptr::null(),
                0,
            ),
        }
    });

    // Borrow is fully released here; safe to call gc_alloc inside.
    let tmp_exc = ExceptionObject {
        exc_type,
        custom_class_id,
        message: msg_ptr,
        message_len: msg_len,
        message_capacity: 0,
        cause: None,
        context: None,
        suppress_context: false,
        traceback: None,
        instance: std::ptr::null_mut(),
    };
    let instance = unsafe { create_builtin_exception_instance(&tmp_exc) };

    // Store the created instance back into the exception state.
    with_exception_state(|state| {
        let exc = state
            .current_exception
            .as_mut()
            .or(state.handling_exception.as_mut());
        if let Some(exc) = exc {
            if exc.instance.is_null() {
                exc.instance = instance;
            }
        }
    });

    instance
}

/// Get the class name of an exception instance as a `"<class 'ExcName'>"` string.
/// For built-in exceptions, returns `"<class 'ValueError'>"` etc.
/// For custom exceptions, looks up the registered name.
/// Returns a heap-allocated StrObj.
///
/// # Safety
/// `instance` must be a valid pointer to an InstanceObj.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_class_name(
    instance: *mut crate::object::Obj,
) -> *mut crate::object::Obj {
    if instance.is_null() {
        let s = "<class 'Exception'>";
        return crate::string::rt_make_str(s.as_ptr(), s.len());
    }

    // Get the exception name from thread-local state to correctly distinguish
    // built-in vs custom exceptions (their class_ids overlap in the vtable).
    let name = with_exception_state(|state| {
        let exc = state
            .current_exception
            .as_ref()
            .or(state.handling_exception.as_ref());
        if let Some(exc) = exc {
            if exc.custom_class_id == NOT_CUSTOM_CLASS {
                return exc.exc_type.name().to_string();
            } else {
                return get_custom_exception_name(exc.custom_class_id);
            }
        }
        // Fallback: read class_id from instance header
        let inst = instance as *const crate::object::InstanceObj;
        let class_id = (*inst).class_id;
        if let Some(kind) = BuiltinExceptionKind::from_tag(class_id) {
            kind.name().to_string()
        } else {
            get_custom_exception_name(class_id)
        }
    });

    let class_str = format!("<class '{}'>", name);
    crate::string::rt_make_str(class_str.as_ptr(), class_str.len())
}

/// Get the current exception message as a string object.
/// This is the backward-compatible version for internal use (traceback printing, etc.).
/// Returns a heap-allocated StrObj with the exception message, or an empty string.
#[no_mangle]
pub extern "C" fn rt_exc_get_current_message() -> *mut crate::object::Obj {
    // Read message pointer and length inside the borrow, then allocate outside.
    // rt_make_str calls gc_alloc which fires a full collection in gc_stress mode.
    // Doing that inside the borrow_mut would trigger get_exception_pointers() →
    // borrow_mut() → BorrowMutError panic.
    let (msg_ptr, msg_len) = with_exception_state(|state| {
        let exc = state
            .current_exception
            .as_ref()
            .or(state.handling_exception.as_ref());

        if let Some(exc) = exc {
            if exc.message_len > 0 && !exc.message.is_null() {
                (exc.message, exc.message_len)
            } else {
                (std::ptr::null(), 0)
            }
        } else {
            (std::ptr::null(), 0)
        }
    });
    // Borrow is released here; safe to call gc_alloc.
    unsafe { crate::string::rt_make_str(msg_ptr, msg_len) }
}

/// Convert an exception instance to its string representation.
///
/// First tries to get the message from the thread-local ExceptionState (which stores
/// the original message from the raise site). Falls back to reading field 0 (.args tuple)
/// from the instance for built-in exception instances created lazily.
///
/// This implements `str(e)` for exception objects, matching CPython behavior
/// where `str(ValueError("msg"))` returns `"msg"`.
#[no_mangle]
pub extern "C" fn rt_exc_instance_str(
    instance: *mut crate::object::Obj,
) -> *mut crate::object::Obj {
    // Try to get message from ExceptionState first
    // This works for both built-in and custom exceptions
    let msg = with_exception_state(|state| {
        let exc = state
            .current_exception
            .as_ref()
            .or(state.handling_exception.as_ref());
        if let Some(exc) = exc {
            // Check if this is the same instance
            if !instance.is_null() && exc.instance == instance {
                if exc.message_len > 0 && !exc.message.is_null() {
                    return Some(unsafe {
                        crate::string::rt_make_str(exc.message, exc.message_len)
                    });
                }
                return Some(unsafe { crate::string::rt_make_str(std::ptr::null(), 0) });
            }
        }
        None
    });

    if let Some(msg) = msg {
        return msg;
    }

    // Fallback: try to read .args from instance (for lazy-created built-in exception instances)
    if instance.is_null() {
        return unsafe { crate::string::rt_make_str(std::ptr::null(), 0) };
    }

    unsafe {
        // Get field 0 (.args tuple) from the instance
        let args_raw = crate::instance::rt_instance_get_field(instance, 0);
        let args_tuple = args_raw as *mut crate::object::Obj;

        if args_tuple.is_null() {
            return crate::string::rt_make_str(std::ptr::null(), 0);
        }

        // Verify it's actually a tuple (not a custom field)
        let header = &*(args_tuple as *const crate::object::ObjHeader);
        if header.type_tag != crate::object::TypeTagKind::Tuple {
            return crate::string::rt_make_str(std::ptr::null(), 0);
        }

        let tuple_obj = args_tuple as *mut crate::object::TupleObj;
        let len = (*tuple_obj).len;
        if len == 0 {
            return crate::string::rt_make_str(std::ptr::null(), 0);
        }

        let data_ptr = (*tuple_obj).data.as_ptr();
        let first_elem = *data_ptr;

        if first_elem.is_null() {
            return crate::string::rt_make_str(std::ptr::null(), 0);
        }

        let elem_header = &*(first_elem as *const crate::object::ObjHeader);
        if elem_header.type_tag == crate::object::TypeTagKind::Str {
            return first_elem;
        }

        crate::conversions::rt_obj_to_str(first_elem)
    }
}

/// Check if current exception matches the given type tag
/// Returns 1 if it matches, 0 otherwise
#[no_mangle]
pub extern "C" fn rt_exc_isinstance(type_tag: u8) -> i8 {
    with_exception_state(|state| {
        if let Some(ref exc) = state.current_exception {
            // BaseException (tag 28) catches ALL exceptions
            if type_tag == BuiltinExceptionKind::BaseException.tag() {
                return 1;
            }
            // Exception (tag 0) catches all EXCEPT SystemExit, KeyboardInterrupt, GeneratorExit
            if type_tag == BuiltinExceptionKind::Exception.tag() {
                let exc_tag = exc.exc_type as u8;
                if exc_tag == BuiltinExceptionKind::SystemExit.tag()
                    || exc_tag == BuiltinExceptionKind::KeyboardInterrupt.tag()
                    || exc_tag == BuiltinExceptionKind::GeneratorExit.tag()
                {
                    return 0;
                }
                return 1;
            }
            // Otherwise, check for exact type match
            if exc.exc_type as u8 == type_tag {
                1
            } else {
                0
            }
        } else {
            0
        }
    })
}

// ==================== Custom Exception Class Support ====================

/// Raise a custom exception with the given class ID and message.
/// This function does not return - it longjmps to the nearest handler.
///
/// Custom exception classes use class IDs 27+ (0-26 are reserved for built-in exceptions).
/// The class hierarchy is looked up via rt_class_inherits_from() for exception matching.
///
/// # Safety
/// If `len > 0`, `message` must be a valid pointer to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_raise_custom(class_id: u8, message: *const u8, len: usize) -> ! {
    // Copy message to owned buffer if present
    let (msg_ptr, msg_len, msg_capacity) = copy_message_to_owned(message, len);

    // Capture implicit context from any exception being handled
    let context = with_exception_state(|state| state.handling_exception.take());

    // Capture traceback at the point of raise
    let traceback = Some(crate::traceback::capture_traceback());

    // Store exception object with custom class ID
    // Use Exception as the base exc_type for custom exceptions
    let exc_obj = Box::new(ExceptionObject {
        exc_type: ExceptionType::Exception,
        custom_class_id: class_id,
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

/// Raise a custom exception with a pre-created instance.
/// The instance was allocated and __init__ called at the raise site.
/// This function stores the instance pointer in the ExceptionObject so that
/// `rt_exc_get_current()` returns it directly without lazy creation.
///
/// # Safety
/// - If `len > 0`, `message` must be a valid pointer to `len` bytes.
/// - `instance` must be a valid heap-allocated Obj pointer or null.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_raise_custom_with_instance(
    class_id: u8,
    message: *const u8,
    len: usize,
    instance: *mut crate::object::Obj,
) -> ! {
    let (msg_ptr, msg_len, msg_capacity) = copy_message_to_owned(message, len);
    let context = with_exception_state(|state| state.handling_exception.take());
    let traceback = Some(crate::traceback::capture_traceback());

    let exc_obj = Box::new(ExceptionObject {
        exc_type: ExceptionType::Exception,
        custom_class_id: class_id,
        message: msg_ptr,
        message_len: msg_len,
        message_capacity: msg_capacity,
        cause: None,
        context,
        suppress_context: false,
        traceback,
        instance, // Store pre-created instance
    });

    dispatch_to_handler(exc_obj)
}

/// Raise an exception from an existing exception instance pointer.
/// Used for `raise e` where `e` is a caught exception variable.
/// Reads the class_id from the InstanceObj header and extracts the message
/// from `.args[0]` (field 0 is an args tuple, element 0 is the message string).
///
/// # Safety
/// `instance` must be a valid pointer to a heap-allocated InstanceObj.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_raise_instance(instance: *mut crate::object::Obj) -> ! {
    let inst = instance as *const crate::object::InstanceObj;
    let class_id = (*inst).class_id;

    // Try to recover the original message from the thread-local exception state.
    // The exception may still be in handling_exception if we're inside an except block.
    // For built-in exceptions, field 0 is .args tuple containing the message.
    // For custom exceptions, fields are user-defined (not .args), so we don't read them.
    let (msg_ptr, msg_len, msg_capacity) = {
        // First try: get message from thread-local state (if exception is still there)
        let from_state = with_exception_state(|state| {
            let exc = state
                .current_exception
                .as_ref()
                .or(state.handling_exception.as_ref());
            if let Some(exc) = exc {
                if exc.message_len > 0 && !exc.message.is_null() {
                    let slice = std::slice::from_raw_parts(exc.message, exc.message_len);
                    let v = slice.to_vec();
                    return Some(v);
                }
            }
            None
        });
        if let Some(mut v) = from_state {
            let ptr = v.as_mut_ptr();
            let len = v.len();
            let cap = v.capacity();
            std::mem::forget(v);
            (ptr as *const u8, len, cap)
        } else {
            (std::ptr::null(), 0, 0)
        }
    };

    let context = with_exception_state(|state| state.handling_exception.take());
    let traceback = Some(crate::traceback::capture_traceback());

    // Determine exc_type from class_id
    let exc_type = exception_type_from_tag(class_id);
    let custom_class_id = if BuiltinExceptionKind::from_tag(class_id).is_some() {
        NOT_CUSTOM_CLASS
    } else {
        class_id
    };

    let exc_obj = Box::new(ExceptionObject {
        exc_type,
        custom_class_id,
        message: msg_ptr,
        message_len: msg_len,
        message_capacity: msg_capacity,
        cause: None,
        context,
        suppress_context: false,
        traceback,
        instance, // Preserve the original instance
    });

    dispatch_to_handler(exc_obj)
}

/// Check if current exception is an instance of the given class (with inheritance).
/// Uses rt_class_inherits_from to walk the class hierarchy.
/// Returns 1 if the current exception's class inherits from target_class_id, 0 otherwise.
///
/// For built-in exceptions (class IDs 0-26), this checks if:
/// - The exception's type tag matches the target class ID, OR
/// - The target is Exception (class ID 0), which catches all.
///
/// For custom exceptions (class IDs 27+), this walks the class hierarchy.
#[no_mangle]
pub extern "C" fn rt_exc_isinstance_class(target_class_id: u8) -> i8 {
    with_exception_state(|state| {
        if let Some(ref exc) = state.current_exception {
            // BaseException (tag 28) catches ALL exceptions
            if target_class_id == BuiltinExceptionKind::BaseException.tag() {
                return 1;
            }

            // Get the class ID of the current exception
            let exc_class_id = if exc.custom_class_id == NOT_CUSTOM_CLASS {
                // Built-in exception: use the type tag as class ID (0-28)
                exc.exc_type as u8
            } else {
                // Custom exception: use the custom class ID (29+)
                exc.custom_class_id
            };

            // Use vtable inheritance check.
            // The vtable hierarchy correctly models CPython's exception tree:
            // - BaseException (28) is the root (no parent)
            // - Exception (0) inherits from BaseException
            // - SystemExit/KeyboardInterrupt/GeneratorExit inherit from BaseException (NOT Exception)
            // - All other built-in exceptions inherit from Exception
            // - User class IDs start at BUILTIN_EXCEPTION_COUNT (29+), never overlapping built-in tags
            crate::vtable::rt_class_inherits_from(exc_class_id, target_class_id)
        } else {
            0
        }
    })
}

/// Get the class ID of the current exception.
/// Returns NOT_CUSTOM_CLASS (255) if no exception is pending.
/// For built-in exceptions, returns the type tag (0-12).
/// For custom exceptions, returns the custom class ID (13+).
#[no_mangle]
pub extern "C" fn rt_exc_get_class_id() -> u8 {
    with_exception_state(|state| {
        if let Some(ref exc) = state.current_exception {
            if exc.custom_class_id == NOT_CUSTOM_CLASS {
                exc.exc_type as u8
            } else {
                exc.custom_class_id
            }
        } else {
            NOT_CUSTOM_CLASS
        }
    })
}

/// Get the display name for a custom exception class.
/// Returns "CustomException<id>" if name is not registered.
fn get_custom_exception_name(class_id: u8) -> String {
    // Try to get the registered name, fall back to generic name
    if let Some(name) = get_registered_exception_name(class_id) {
        name
    } else {
        format!("CustomException<{}>", class_id)
    }
}

// ==================== Exception Class Name Registry ====================

use std::cell::UnsafeCell;

/// Maximum number of exception classes that can be registered
const MAX_EXCEPTION_CLASSES: usize = 256;

/// Lock-free registry for single-threaded access
struct ExcNameRegistry(UnsafeCell<[Option<String>; MAX_EXCEPTION_CLASSES]>);

// Safety: The runtime is single-threaded (AOT-compiled Python has no threading)
unsafe impl Sync for ExcNameRegistry {}

static EXCEPTION_NAME_REGISTRY: ExcNameRegistry =
    ExcNameRegistry(UnsafeCell::new([const { None }; MAX_EXCEPTION_CLASSES]));

/// Register a custom exception class name for display purposes.
/// This is called during module initialization to register exception class names.
#[no_mangle]
pub extern "C" fn rt_exc_register_class_name(class_id: u8, name: *const u8, len: usize) {
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
fn get_registered_exception_name(class_id: u8) -> Option<String> {
    unsafe { (*EXCEPTION_NAME_REGISTRY.0.get())[class_id as usize].clone() }
}
