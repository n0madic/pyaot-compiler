//! FFI-exported exception functions (`extern "C"`).
//!
//! All `pub extern "C" fn rt_exc_*` symbols live here. These are called from
//! Cranelift-generated code and from the rest of the runtime.

use pyaot_core_defs::BuiltinExceptionKind;

use super::core::{
    apply_pending, build_builtin_cause, copy_message_to_owned, create_builtin_exception_instance,
    dispatch_existing_exception, dispatch_to_handler, exc_object_from_value, exception_type_from_tag,
    get_custom_exception_name, print_unhandled_exception_full, raise_with_owned_message,
    register_class_name, ExceptionObject, ExceptionType, NOT_CUSTOM_CLASS,
};
use super::state::with_exception_state;

// Frame push/pop is gone: protected regions are static table entries
// (see `super::unwind`); the happy path of a `try` block emits no runtime
// calls at all.

/// Raise an exception with the given type and message
/// This function does not return - it unwinds to the nearest handler
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
/// ownership of the caller's buffer, avoiding the leak that occurs when the unwind
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
        (std::ptr::null(), 0, 0)
    };
    raise_with_owned_message(exc_type, ptr, len, cap)
}

/// Arm a `from None` for the immediately-following raise (`raise X from None`,
/// PEP 3134): suppress the implicit `__context__` chain without an explicit
/// cause. The next raise builder consumes this via `apply_pending`.
#[no_mangle]
pub extern "C" fn rt_exc_arm_suppress() {
    with_exception_state(|state| {
        state.pending_suppress = true;
    });
}

/// Arm a builtin-exception cause for the immediately-following raise
/// (`raise X from BuiltinError(...)` / `from BuiltinError`). Stores a scalar
/// `(tag, message)` cause object; the next raise consumes it via
/// `apply_pending`.
///
/// # Safety
/// If `len > 0`, `message` must be a valid pointer to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_arm_cause_builtin(cause_tag: u8, message: *const u8, len: usize) {
    let cause = build_builtin_cause(cause_tag, message, len);
    with_exception_state(|state| {
        state.pending_cause = Some(Box::new(cause));
    });
}

/// Arm an arbitrary value cause for the immediately-following raise
/// (`raise X from <value>` — a caught variable or a constructed custom/stdlib
/// exception). Reads `(class_id, message)` from the Tagged instance; a
/// non-exception value raises `TypeError` ("exception causes must derive from
/// BaseException"), matching CPython.
///
/// # Safety
/// `cause`'s bits must be a valid Tagged `Value`.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_arm_cause_value(cause: pyaot_core_defs::Value) {
    match exc_object_from_value(cause) {
        Ok(obj) => {
            with_exception_state(|state| {
                state.pending_cause = Some(Box::new(obj));
            });
        }
        Err(()) => {
            let msg = b"exception causes must derive from BaseException";
            rt_exc_raise(ExceptionType::TypeError as u8, msg.as_ptr(), msg.len());
        }
    }
}

/// Called when entering an except handler block (and the exceptional edge of
/// `finally` / `with`). Pushes the in-flight exception onto the handling stack
/// (a `None` entry when there is none, so pushes and pops stay balanced): a
/// new exception raised during handling captures the top entry as its
/// `__context__`, and a bare `raise` re-raises it.
#[no_mangle]
pub extern "C" fn rt_exc_start_handling() {
    with_exception_state(|state| {
        let exc = state.current_exception.take();
        state.handling_stack.push(exc);
    });
}

/// Called on every exit of a handler block entered by
/// [`rt_exc_start_handling`] — normal, early (`return`/`break`/`continue`) and
/// exceptional (the compiler's cleanup landing pad). Pops the handling stack.
#[no_mangle]
pub extern "C" fn rt_exc_end_handling() {
    with_exception_state(|state| {
        let _ = state.handling_stack.pop();
    });
}

/// Re-raise the current exception (bare `raise` statement)
/// This is used when an except block wants to propagate the exception
///
/// # Safety
/// This function unwinds the stack to the nearest exception handler (FP walk + jump).
/// The caller must ensure a valid exception handler frame exists on the handler stack.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_reraise() -> ! {
    // In flight: propagate it (the no-match tail of a handler chain, or a
    // cleanup landing pad). Otherwise re-raise the innermost HANDLED exception
    // (a bare `raise` in an `except` body, or the end of a `finally` run on
    // the exceptional edge) — a copy: its handling-stack entry is popped by
    // the enclosing block's cleanup.
    let has_exception = with_exception_state(|state| {
        if state.current_exception.is_none() {
            state.current_exception = state.handled().map(|e| Box::new(e.deep_clone()));
        }
        state.current_exception.is_some()
    });
    if !has_exception {
        // No active exception - raise RuntimeError (matches CPython behavior)
        crate::utils::raise_runtime_error("No active exception to reraise");
    }

    // Re-dispatch the existing exception (already stored in current_exception)
    dispatch_existing_exception()
}

/// Get the current exception type tag
/// Returns the type tag of current exception, or -1 if no exception.
/// Checks both current_exception and the handling stack (the latter is set
/// after ExcStartHandling in except handlers, e.g. context manager __exit__).
#[no_mangle]
pub extern "C" fn rt_exc_get_type() -> i32 {
    with_exception_state(|state| {
        let exc = state.current_or_handled();
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
            std::ptr::null()
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

/// Get the current exception as a heap-allocated exception instance.
/// Returns an `InstanceObj` with `.args` tuple and exception class_id.
/// For built-in exceptions, the instance is created lazily on first access.
/// For custom exceptions with a pre-created instance, returns that instance.
/// Returns null if no exception is pending.
///
/// This function checks both current_exception and the handling stack, as the
/// exception may have been moved there by rt_exc_start_handling().
#[no_mangle]
pub extern "C" fn rt_exc_get_current() -> *mut crate::object::Obj {
    // Fast path: return an already-created instance (no allocation needed).
    let existing = with_exception_state(|state| {
        let exc = state.current_or_handled();
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
        let exc = state.current_or_handled();
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
        let exc = state.current_or_handled_mut();
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
        let exc = state.current_or_handled();
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
        let exc = state.current_or_handled();

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
    // Copy the message location out of the borrow and allocate AFTER
    // releasing it: `rt_make_str` may collect, and the GC's root scan cannot
    // read the exception state while it is mutably borrowed.
    let msg = with_exception_state(|state| {
        let exc = state.current_or_handled();
        match exc {
            // Same instance: its owned message buffer (stable across a GC —
            // the collector never frees exception-state buffers).
            Some(exc) if !instance.is_null() && exc.instance == instance => {
                if exc.message_len > 0 && !exc.message.is_null() {
                    Some((exc.message, exc.message_len))
                } else {
                    Some((std::ptr::null(), 0))
                }
            }
            _ => None,
        }
    });

    if let Some((ptr, len)) = msg {
        return unsafe { crate::string::rt_make_str(ptr, len) };
    }

    // Fallback: try to read .args from instance (for lazy-created built-in exception instances)
    if instance.is_null() {
        return unsafe { crate::string::rt_make_str(std::ptr::null(), 0) };
    }
    // A user exception whose message `BaseException.__init__` recorded.
    if let Some(m) = super::core::instance_message(instance) {
        return unsafe { crate::string::rt_make_str(m.as_ptr(), m.len()) };
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

        if first_elem.0 == 0 {
            return crate::string::rt_make_str(std::ptr::null(), 0);
        }

        let elem_ptr = first_elem.0 as *const crate::object::ObjHeader;
        let elem_header = &*elem_ptr;
        if elem_header.type_tag == crate::object::TypeTagKind::Str {
            return elem_ptr as *mut crate::object::Obj;
        }

        crate::conversions::rt_obj_to_str(elem_ptr as *mut crate::object::Obj)
    }
}

/// Check if the current exception is an instance of the builtin exception
/// `type_tag` (subclasses included — builtin tags ARE runtime class ids, so this
/// is the same registry walk as [`rt_exc_isinstance_class`]).
/// Returns 1 if it matches, 0 otherwise.
#[no_mangle]
pub extern "C" fn rt_exc_isinstance(type_tag: u8) -> i8 {
    rt_exc_isinstance_class(type_tag)
}

// ==================== Custom Exception Class Support ====================

/// Raise a custom exception with the given class ID and message.
/// This function does not return - it unwinds to the nearest handler.
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
    let context = with_exception_state(|state| state.new_context(std::ptr::null_mut()));

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

/// Raise a stdlib exception (Phase 8D): one whose builtin PARENT is
/// `exc_type_tag` (e.g. `HTTPError`/`URLError` → `OSError`). Sets BOTH
/// `exc_type` (so `except OSError` / `except Exception` match by tag) and
/// `custom_class_id` (so `except HTTPError` matches by its own class id). This
/// is the seam `rt_exc_raise_custom` cannot serve — that one hardcodes
/// `exc_type = Exception`, which a bare `except OSError` would miss.
///
/// # Safety
/// If `len > 0`, `message` must be a valid pointer to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_raise_stdlib(
    exc_type_tag: u8,
    class_id: u8,
    message: *const u8,
    len: usize,
) -> ! {
    // Register this stdlib exception's parent in the class registry so the
    // hierarchy walker (`rt_class_inherits_from`, used by `except OSError:` /
    // `except Exception:`) resolves `class_id → parent → … → Exception`. The
    // builtin chain (`OSError → Exception → BaseException`) is already wired by
    // `rt_init_builtin_exception_classes`. Idempotent.
    crate::vtable::rt_register_class(class_id, exc_type_tag);

    let exc_type = exception_type_from_tag(exc_type_tag);
    let (msg_ptr, msg_len, msg_capacity) = copy_message_to_owned(message, len);
    let context = with_exception_state(|state| state.new_context(std::ptr::null_mut()));
    let traceback = Some(crate::traceback::capture_traceback());

    let mut exc_obj = Box::new(ExceptionObject {
        exc_type,
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

    // Attach any pending `from CAUSE` / `from None` (PEP 3134).
    apply_pending(&mut exc_obj);

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
    // No raise-site message: use the one `BaseException.__init__` recorded
    // (`super().__init__(msg)`), if any.
    let recorded = if len == 0 && !instance.is_null() {
        super::core::instance_message(instance)
    } else {
        None
    };
    let (msg_ptr, msg_len, msg_capacity) = match &recorded {
        Some(m) => copy_message_to_owned(m.as_ptr(), m.len()),
        None => copy_message_to_owned(message, len),
    };
    let context = with_exception_state(|state| state.new_context(instance));
    let traceback = Some(crate::traceback::capture_traceback());

    let mut exc_obj = Box::new(ExceptionObject {
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

    // Attach any pending `from CAUSE` / `from None` (PEP 3134).
    apply_pending(&mut exc_obj);

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
    // Defensive null-check at the public C-ABI boundary: a null instance would
    // otherwise dereference below. Mirrors CPython's `raise None` behaviour.
    if instance.is_null() {
        let msg = b"exceptions must derive from BaseException";
        rt_exc_raise(ExceptionType::TypeError as u8, msg.as_ptr(), msg.len());
    }
    let inst = instance as *const crate::object::InstanceObj;
    let class_id = (*inst).class_id;

    // Try to recover the original message from the thread-local exception state.
    // The exception may be on the handling stack if we're inside an except block.
    // For built-in exceptions, field 0 is .args tuple containing the message.
    // For custom exceptions, fields are user-defined (not .args), so we don't read them.
    let (msg_ptr, msg_len, msg_capacity) = {
        // First try: get message from thread-local state (if exception is still there)
        let from_state = with_exception_state(|state| {
            let exc = state.current_or_handled();
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

    let context = with_exception_state(|state| state.new_context(instance));
    let traceback = Some(crate::traceback::capture_traceback());

    // Determine exc_type from class_id
    let exc_type = exception_type_from_tag(class_id);
    let custom_class_id = if BuiltinExceptionKind::from_tag(class_id).is_some() {
        NOT_CUSTOM_CLASS
    } else {
        class_id
    };

    let mut exc_obj = Box::new(ExceptionObject {
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

    // Attach any pending `from CAUSE` / `from None` (PEP 3134).
    apply_pending(&mut exc_obj);

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

            // Registry inheritance walk. Builtin parents are seeded from the
            // core-defs parent column (CPython's exception tree); user and
            // stdlib exception classes register their own parents. User class
            // ids never overlap builtin tags.
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

/// Register a custom exception class name for display purposes.
/// This is called during module initialization to register exception class names.
#[no_mangle]
pub extern "C" fn rt_exc_register_class_name(class_id: u8, name: *const u8, len: usize) {
    register_class_name(class_id, name, len);
}

/// `BaseException.__init__(self[, msg])` for a user exception instance —
/// `super().__init__(msg)` in an exception class whose parents define no
/// `__init__`: records `str(msg)` (empty without an argument) as the
/// instance's message, which `str(e)` and an uncaught traceback then show.
///
/// # Safety
/// `instance` must be a valid instance pointer; `msg` null (no argument), a
/// tagged immediate, or a valid object pointer.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_set_instance_message(
    instance: pyaot_core_defs::Value,
    msg: pyaot_core_defs::Value,
) {
    let inst = instance.0 as *mut crate::object::Obj;
    if inst.is_null() {
        return;
    }
    let text = if msg.0 == 0 {
        String::new()
    } else {
        let s = crate::conversions::rt_obj_to_str(msg.0 as *mut crate::object::Obj);
        crate::utils::extract_str_unchecked(s)
    };
    super::core::set_instance_message(inst, text);
}

// ==================== Exception instance str / repr / type name ====================

/// The class name of an exception instance's class (`ValueError`, a user
/// `MyErr`, a stdlib `HTTPError`), or `None` when `obj` is not an exception
/// instance. Second value: is it a builtin exception class.
///
/// # Safety
/// `obj` must be null, a tagged immediate or a valid object pointer.
pub(crate) unsafe fn exception_class_name(obj: *mut crate::object::Obj) -> Option<(String, bool)> {
    let v = pyaot_core_defs::Value(obj as u64);
    if !v.is_ptr() || obj.is_null() || (*obj).type_tag() != crate::object::TypeTagKind::Instance {
        return None;
    }
    let class_id = (*(obj as *const crate::object::InstanceObj)).class_id;
    if let Some(kind) = BuiltinExceptionKind::from_tag(class_id) {
        return Some((kind.name().to_string(), true));
    }
    if crate::vtable::rt_class_inherits_from(class_id, BuiltinExceptionKind::BaseException.tag())
        == 0
    {
        return None;
    }
    let name = match super::core::get_registered_exception_name(class_id) {
        Some(n) => n,
        None => crate::ops::instance_class_name(class_id),
    };
    Some((name, false))
}

/// `str(e)` of an exception instance as a Rust string (`None` if `obj` is not
/// an exception instance).
///
/// # Safety
/// See [`exception_class_name`].
pub(crate) unsafe fn exception_str(obj: *mut crate::object::Obj) -> Option<String> {
    exception_class_name(obj)?;
    let s = rt_exc_instance_str(obj);
    Some(crate::utils::extract_str_unchecked(s))
}

/// `repr(e)` of an exception instance: `ValueError('bad')`, `ValueError()`,
/// `MyErr('m')` (`None` if `obj` is not an exception instance). A builtin
/// exception renders its `.args`; a user exception renders its message (the
/// constructor's full argument list is not retained).
///
/// # Safety
/// See [`exception_class_name`].
pub(crate) unsafe fn exception_repr(obj: *mut crate::object::Obj) -> Option<String> {
    let (name, builtin) = exception_class_name(obj)?;
    let mut parts: Vec<String> = Vec::new();
    let args = if builtin {
        crate::instance::rt_instance_get_field(obj, 0) as *mut crate::object::Obj
    } else {
        std::ptr::null_mut()
    };
    let av = pyaot_core_defs::Value(args as u64);
    if av.is_ptr() && !args.is_null() && (*args).type_tag() == crate::object::TypeTagKind::Tuple {
        let t = args as *mut crate::object::TupleObj;
        // A KeyError's message is already `repr(key)` (its `__str__`).
        let key_error = name == BuiltinExceptionKind::KeyError.name();
        for i in 0..(*t).len {
            let elem = (*(*t).data.as_ptr().add(i)).0 as *mut crate::object::Obj;
            if key_error && (*t).len == 1 {
                parts.push(crate::utils::extract_str_unchecked(
                    crate::conversions::rt_obj_to_str(elem),
                ));
                continue;
            }
            let r = crate::builtins::rt_builtin_repr(elem);
            parts.push(crate::utils::extract_str_unchecked(r));
        }
    } else {
        let msg = crate::utils::extract_str_unchecked(rt_exc_instance_str(obj));
        if !msg.is_empty() {
            let mut q = String::new();
            crate::conversions::repr_str_into(&mut q, &msg);
            parts.push(q);
        }
    }
    Some(format!("{}({})", name, parts.join(", ")))
}
