//! Exception handling support using setjmp/longjmp
//!
//! This module provides exception handling infrastructure for the AOT compiler.
//! It uses setjmp/longjmp for control flow with thread-local exception state.
//!
//! # Exception Type Definitions
//!
//! Exception types are defined in `pyaot-core-defs` crate, which serves as the
//! single source of truth shared between the compiler and runtime.

use pyaot_core_defs::BuiltinExceptionKind;
use std::cell::RefCell;
use std::ptr;

/// Size of jmp_buf varies by platform
/// macOS/iOS arm64: 192 bytes (defined as int[48] in <setjmp.h>)
/// macOS/iOS x86_64: 148 bytes
/// Linux x86_64: 200 bytes (defined as __jmp_buf_tag with __jmp_buf = long[8], __saved_mask = sigset_t)
/// We use 200 bytes to be safe across platforms
pub const JMP_BUF_SIZE: usize = 200;

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

/// Runtime exception type - wraps `BuiltinExceptionKind` with runtime-specific behavior.
/// The main difference is that `from_tag` returns `Exception` for invalid tags instead of `None`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExceptionType {
    Exception = 0,
    AssertionError = 1,
    IndexError = 2,
    ValueError = 3,
    StopIteration = 4,
    TypeError = 5,
    RuntimeError = 6,
    GeneratorExit = 7,
    KeyError = 8,
    AttributeError = 9,
    IOError = 10,
    ZeroDivisionError = 11,
    OverflowError = 12,
    MemoryError = 13,
    NameError = 14,
    NotImplementedError = 15,
    FileNotFoundError = 16,
    PermissionError = 17,
    RecursionError = 18,
    EOFError = 19,
    SystemExit = 20,
    KeyboardInterrupt = 21,
    FileExistsError = 22,
    ImportError = 23,
    OSError = 24,
    ConnectionError = 25,
    TimeoutError = 26,
    SyntaxError = 27,
}

impl ExceptionType {
    /// Create from a numeric tag.
    /// Returns `Exception` if the tag is invalid (runtime fallback behavior).
    #[inline]
    pub const fn from_tag(tag: u8) -> Self {
        match BuiltinExceptionKind::from_tag(tag) {
            Some(kind) => Self::from_kind(kind),
            None => Self::Exception,
        }
    }

    /// Create from a `BuiltinExceptionKind`.
    #[inline]
    pub const fn from_kind(kind: BuiltinExceptionKind) -> Self {
        // Direct conversion using the shared tag values
        match kind {
            BuiltinExceptionKind::Exception => Self::Exception,
            BuiltinExceptionKind::AssertionError => Self::AssertionError,
            BuiltinExceptionKind::IndexError => Self::IndexError,
            BuiltinExceptionKind::ValueError => Self::ValueError,
            BuiltinExceptionKind::StopIteration => Self::StopIteration,
            BuiltinExceptionKind::TypeError => Self::TypeError,
            BuiltinExceptionKind::RuntimeError => Self::RuntimeError,
            BuiltinExceptionKind::GeneratorExit => Self::GeneratorExit,
            BuiltinExceptionKind::KeyError => Self::KeyError,
            BuiltinExceptionKind::AttributeError => Self::AttributeError,
            BuiltinExceptionKind::IOError => Self::IOError,
            BuiltinExceptionKind::ZeroDivisionError => Self::ZeroDivisionError,
            BuiltinExceptionKind::OverflowError => Self::OverflowError,
            BuiltinExceptionKind::MemoryError => Self::MemoryError,
            BuiltinExceptionKind::NameError => Self::NameError,
            BuiltinExceptionKind::NotImplementedError => Self::NotImplementedError,
            BuiltinExceptionKind::FileNotFoundError => Self::FileNotFoundError,
            BuiltinExceptionKind::PermissionError => Self::PermissionError,
            BuiltinExceptionKind::RecursionError => Self::RecursionError,
            BuiltinExceptionKind::EOFError => Self::EOFError,
            BuiltinExceptionKind::SystemExit => Self::SystemExit,
            BuiltinExceptionKind::KeyboardInterrupt => Self::KeyboardInterrupt,
            BuiltinExceptionKind::FileExistsError => Self::FileExistsError,
            BuiltinExceptionKind::ImportError => Self::ImportError,
            BuiltinExceptionKind::OSError => Self::OSError,
            BuiltinExceptionKind::ConnectionError => Self::ConnectionError,
            BuiltinExceptionKind::TimeoutError => Self::TimeoutError,
            BuiltinExceptionKind::SyntaxError => Self::SyntaxError,
        }
    }

    /// Get the Python name for this exception type.
    #[inline]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Exception => "Exception",
            Self::AssertionError => "AssertionError",
            Self::IndexError => "IndexError",
            Self::ValueError => "ValueError",
            Self::StopIteration => "StopIteration",
            Self::TypeError => "TypeError",
            Self::RuntimeError => "RuntimeError",
            Self::GeneratorExit => "GeneratorExit",
            Self::KeyError => "KeyError",
            Self::AttributeError => "AttributeError",
            Self::IOError => "IOError",
            Self::ZeroDivisionError => "ZeroDivisionError",
            Self::OverflowError => "OverflowError",
            Self::MemoryError => "MemoryError",
            Self::NameError => "NameError",
            Self::NotImplementedError => "NotImplementedError",
            Self::FileNotFoundError => "FileNotFoundError",
            Self::PermissionError => "PermissionError",
            Self::RecursionError => "RecursionError",
            Self::EOFError => "EOFError",
            Self::SystemExit => "SystemExit",
            Self::KeyboardInterrupt => "KeyboardInterrupt",
            Self::FileExistsError => "FileExistsError",
            Self::ImportError => "ImportError",
            Self::OSError => "OSError",
            Self::ConnectionError => "ConnectionError",
            Self::TimeoutError => "TimeoutError",
            Self::SyntaxError => "SyntaxError",
        }
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
    /// Values 0-26 are reserved for built-in exceptions mapped to class IDs.
    /// Values 27+ are user-defined exception classes.
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
        // Note: self.cause and self.context are Option<Box<ExceptionObject>> which will be
        // automatically dropped, recursively freeing the cause and context chains
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

/// Helper to access exception state
fn with_exception_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut ExceptionState) -> R,
{
    EXCEPTION_STATE.with(|state| f(&mut state.borrow_mut()))
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

/// Raise an exception with the given type and message
/// This function does not return - it longjmps to the nearest handler
///
/// # Safety
/// If `len > 0`, `message` must be a valid pointer to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_raise(exc_type_tag: u8, message: *const u8, len: usize) -> ! {
    // Convert type tag to ExceptionType using macro-generated from_tag
    let exc_type = ExceptionType::from_tag(exc_type_tag);

    // Copy message to owned buffer if present.
    // The Vec is forgotten, and ownership is transferred to ExceptionObject.
    // The Drop impl for ExceptionObject will reconstruct the Vec with matching
    // (ptr, len, capacity) values to properly deallocate the buffer.
    let (msg_ptr, msg_len, msg_capacity) = if !message.is_null() && len > 0 {
        let mut msg_buf = vec![0u8; len];
        ptr::copy_nonoverlapping(message, msg_buf.as_mut_ptr(), len);
        let ptr = msg_buf.as_ptr();
        let capacity = msg_buf.capacity();
        std::mem::forget(msg_buf);
        (ptr, len, capacity)
    } else {
        (ptr::null(), 0, 0)
    };

    // Capture implicit context from any exception being handled
    // This implements Python's __context__ (PEP 3134)
    let context = with_exception_state(|state| state.handling_exception.take());

    // Capture traceback at the point of raise
    let traceback = Some(crate::traceback::capture_traceback());

    // Store exception object
    let exc_obj = Box::new(ExceptionObject {
        exc_type,
        custom_class_id: NOT_CUSTOM_CLASS,
        message: msg_ptr,
        message_len: msg_len,
        message_capacity: msg_capacity,
        cause: None,
        context,
        suppress_context: false, // Plain raise doesn't suppress context
        traceback,
    });

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
    let exc_type = ExceptionType::from_tag(exc_type_tag);
    let cause_type = ExceptionType::from_tag(cause_type_tag);

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
    });

    let handler_frame = with_exception_state(|state| {
        state.current_exception = Some(exc_obj);
        state.handler_stack
    });

    // If no handler, print chained error and abort
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

    // Pop the handler frame
    with_exception_state(|state| {
        state.handler_stack = (*handler_frame).prev;
    });

    // Jump to handler
    longjmp((*handler_frame).jmp_buf.as_mut_ptr(), 1);
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
    let exc_type = ExceptionType::from_tag(exc_type_tag);
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
    });

    let handler_frame = with_exception_state(|state| {
        state.current_exception = Some(exc_obj);
        state.handler_stack
    });

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

    // Unwind traceback stack to saved position
    crate::traceback::unwind_to((*handler_frame).traceback_depth);

    with_exception_state(|state| {
        state.handler_stack = (*handler_frame).prev;
    });

    longjmp((*handler_frame).jmp_buf.as_mut_ptr(), 1);
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

/// Get the display name for an exception type
/// Uses the macro-generated name() method.
#[inline]
fn exception_type_name(exc_type: ExceptionType) -> &'static str {
    exc_type.name()
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
        let type_name = exception_type_name(exc.exc_type);
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

    let handler_frame = with_exception_state(|state| state.handler_stack);

    // If no handler, print current exception and abort
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

    // Pop the handler frame
    with_exception_state(|state| {
        state.handler_stack = (*handler_frame).prev;
    });

    // Jump to handler
    longjmp((*handler_frame).jmp_buf.as_mut_ptr(), 1);
}

/// Get the current exception type tag
/// Returns the type tag of current exception, or -1 if no exception
#[no_mangle]
pub extern "C" fn rt_exc_get_type() -> i32 {
    with_exception_state(|state| {
        if let Some(ref exc) = state.current_exception {
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

/// Get current exception message as a pointer and length
/// Returns null pointer and 0 length if no exception
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

/// Get the current exception as a string object (for `except Exception as e:`)
/// Returns a heap-allocated StrObj with the exception message, or an empty string if no message.
/// Returns null if no exception is pending.
///
/// This function checks both current_exception and handling_exception, as the exception
/// may have been moved to handling_exception by rt_exc_start_handling().
#[no_mangle]
pub extern "C" fn rt_exc_get_current() -> *mut crate::object::Obj {
    with_exception_state(|state| {
        // Check current_exception first, then handling_exception
        // (handling_exception is set when we're in an except block)
        let exc = state
            .current_exception
            .as_ref()
            .or(state.handling_exception.as_ref());

        if let Some(exc) = exc {
            // Create a string object with the exception message
            if exc.message_len > 0 && !exc.message.is_null() {
                unsafe { crate::string::rt_make_str(exc.message, exc.message_len) }
            } else {
                // Return empty string
                unsafe { crate::string::rt_make_str(std::ptr::null(), 0) }
            }
        } else {
            std::ptr::null_mut()
        }
    })
}

/// Check if current exception matches the given type tag
/// Returns 1 if it matches, 0 otherwise
#[no_mangle]
pub extern "C" fn rt_exc_isinstance(type_tag: u8) -> i8 {
    with_exception_state(|state| {
        if let Some(ref exc) = state.current_exception {
            // Exception type 0 (base Exception) matches all exceptions
            if type_tag == 0 {
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
    });

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
            // Target class ID 0 (Exception) catches all exceptions
            if target_class_id == 0 {
                return 1;
            }

            // Get the class ID of the current exception
            let exc_class_id = if exc.custom_class_id == NOT_CUSTOM_CLASS {
                // Built-in exception: use the type tag as class ID (0-12)
                exc.exc_type as u8
            } else {
                // Custom exception: use the custom class ID
                exc.custom_class_id
            };

            // Use inheritance check from vtable module
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

use std::sync::RwLock;

/// Maximum number of exception classes that can be registered
const MAX_EXCEPTION_CLASSES: usize = 256;

/// Registry for custom exception class names
static EXCEPTION_NAME_REGISTRY: RwLock<[Option<String>; MAX_EXCEPTION_CLASSES]> =
    RwLock::new([const { None }; MAX_EXCEPTION_CLASSES]);

/// Register a custom exception class name for display purposes.
/// This is called during module initialization to register exception class names.
#[no_mangle]
pub extern "C" fn rt_exc_register_class_name(class_id: u8, name: *const u8, len: usize) {
    if let Ok(mut registry) = EXCEPTION_NAME_REGISTRY.write() {
        if !name.is_null() && len > 0 {
            let name_slice = unsafe { std::slice::from_raw_parts(name, len) };
            if let Ok(name_str) = std::str::from_utf8(name_slice) {
                registry[class_id as usize] = Some(name_str.to_string());
            }
        }
    }
}

/// Get the registered name for an exception class
fn get_registered_exception_name(class_id: u8) -> Option<String> {
    if let Ok(registry) = EXCEPTION_NAME_REGISTRY.read() {
        registry[class_id as usize].clone()
    } else {
        None
    }
}
