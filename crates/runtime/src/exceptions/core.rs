//! Core exception types and internal raise machinery.
//!
//! Contains `ExceptionObject` and the internal functions
//! `dispatch_to_handler`, `raise_with_owned_message`, `dispatch_existing_exception`,
//! `copy_message_to_owned`, and exception printing utilities. The control
//! transfer itself (frame-pointer walk + jump) lives in [`super::unwind`].

use std::ptr;

use pyaot_core_defs::{BuiltinExceptionKind, Value};
use std::cell::UnsafeCell;

use super::state::{with_exception_state, with_exception_state_ref};

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
    /// Captured traceback at the point where the exception was raised: raw
    /// return PCs, innermost first (resolved to names/files/lines only when
    /// printed — see `crate::traceback`).
    pub traceback: Option<Vec<usize>>,
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
/// not in the GC shadow stack. When an exception unwind prunes the shadow stack, these
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
        // A pending `from CAUSE` value (`raise X from caught_var`) holds the
        // only reference to its instance until the next raise consumes it
        // (B5/B15) — scan it like the current/handling exceptions.
        if let Some(ref exc) = state.pending_cause {
            collect_from(exc, &mut ptrs);
        }
        ptrs
    })
}

// ==================== Internal raise machinery ====================

/// Core exception raise logic: stores the exception object, then unwinds to
/// the nearest handler (table-based: frame-pointer walk + jump).
///
/// Called by `rt_exc_raise` (after copying message), `rt_exc_raise_owned` (zero-copy),
/// and other raise variants after they build their ExceptionObject.
///
/// # Safety
/// `exc_obj` must be a valid, fully initialized ExceptionObject.
pub(super) unsafe fn dispatch_to_handler(exc_obj: Box<ExceptionObject>) -> ! {
    with_exception_state(|state| {
        state.current_exception = Some(exc_obj);
    });
    dispatch_existing_exception()
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

    let mut exc_obj = Box::new(ExceptionObject {
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

    // Attach any pending `from CAUSE` / `from None` (PEP 3134).
    apply_pending(&mut exc_obj);

    dispatch_to_handler(exc_obj)
}

// ==================== Pending cause (PEP 3134 `from`) ====================

/// Build a cause `ExceptionObject` for a builtin-exception cause (`raise X from
/// BuiltinError(...)` / `from BuiltinError`). Builtin-only: `instance` is null,
/// no traceback of its own. Extracted from the former `rt_exc_raise_from`
/// cause builder so the pending-cause mechanism reuses it.
///
/// # Safety
/// `msg_ptr`/`len` must form a valid owned-able buffer or be `(null, 0)`.
pub(super) unsafe fn build_builtin_cause(
    cause_tag: u8,
    msg_ptr: *const u8,
    len: usize,
) -> ExceptionObject {
    let cause_type = exception_type_from_tag(cause_tag);
    let (cause_msg_ptr, cause_msg_len, cause_msg_capacity) = copy_message_to_owned(msg_ptr, len);
    ExceptionObject {
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
    }
}

/// Copy the message of an exception instance — `.args[0]` when field 0 is an
/// args tuple whose first element is a string — into an owned buffer. Returns
/// `(null, 0, 0)` when there is no recoverable string message (a custom
/// instance without an `.args` tuple included: its cause *type name* still
/// renders correctly — documented boundary). Reads bytes directly (no
/// allocation), unlike `rt_exc_instance_str`.
///
/// # Safety
/// `instance` must be a valid non-null `InstanceObj` pointer.
unsafe fn instance_message_owned(
    instance: *mut crate::object::Obj,
) -> (*const u8, usize, usize) {
    let args_raw = crate::instance::rt_instance_get_field(instance, 0);
    let args_tuple = args_raw as *mut crate::object::Obj;
    if args_tuple.is_null() {
        return (ptr::null(), 0, 0);
    }
    let header = &*(args_tuple as *const crate::object::ObjHeader);
    if header.type_tag != crate::object::TypeTagKind::Tuple {
        return (ptr::null(), 0, 0);
    }
    let tuple_obj = args_tuple as *mut crate::object::TupleObj;
    if (*tuple_obj).len == 0 {
        return (ptr::null(), 0, 0);
    }
    let first_elem = *(*tuple_obj).data.as_ptr();
    if first_elem.0 == 0 {
        return (ptr::null(), 0, 0);
    }
    let elem_ptr = first_elem.0 as *const crate::object::ObjHeader;
    if (*elem_ptr).type_tag != crate::object::TypeTagKind::Str {
        return (ptr::null(), 0, 0);
    }
    let str_obj = elem_ptr as *mut crate::object::Obj;
    let data = crate::string::rt_str_data(str_obj);
    let len = crate::string::rt_str_len(str_obj);
    copy_message_to_owned(data, len)
}

/// Build a cause `ExceptionObject` from a Tagged exception-instance value
/// (`raise X from <value>`). Derives `exc_type`/`custom_class_id` from the
/// instance's `class_id` exactly as `rt_exc_raise_instance` does, and recovers
/// the message from `.args[0]`. Returns `Err(())` for a non-exception value —
/// the caller raises `TypeError` ("exception causes must derive from
/// BaseException"), matching CPython.
///
/// # Safety
/// `value`'s bits must be a valid Tagged `Value`.
pub(super) unsafe fn exc_object_from_value(value: Value) -> Result<ExceptionObject, ()> {
    // An immediate (int/bool/None) or a null pointer is never an exception.
    if !value.is_ptr() {
        return Err(());
    }
    let obj = value.unwrap_ptr::<crate::object::Obj>();
    if obj.is_null() {
        return Err(());
    }
    // Only an `InstanceObj` carries a `class_id` / `.args`; reject str/list/etc.
    let header = &*(obj as *const crate::object::ObjHeader);
    if header.type_tag != crate::object::TypeTagKind::Instance {
        return Err(());
    }
    let inst = obj as *const crate::object::InstanceObj;
    let class_id = (*inst).class_id;
    // The class must derive from BaseException (tag 28).
    let is_exc = BuiltinExceptionKind::from_tag(class_id).is_some()
        || crate::vtable::rt_class_inherits_from(
            class_id,
            BuiltinExceptionKind::BaseException.tag(),
        ) != 0;
    if !is_exc {
        return Err(());
    }
    let exc_type = exception_type_from_tag(class_id);
    let custom_class_id = if BuiltinExceptionKind::from_tag(class_id).is_some() {
        NOT_CUSTOM_CLASS
    } else {
        class_id
    };
    let (msg_ptr, msg_len, msg_capacity) = instance_message_owned(obj);
    Ok(ExceptionObject {
        exc_type,
        custom_class_id,
        message: msg_ptr,
        message_len: msg_len,
        message_capacity: msg_capacity,
        cause: None,
        context: None,
        suppress_context: false,
        traceback: None,
        instance: obj,
    })
}

/// Apply any pending `from CAUSE` (armed by `rt_exc_arm_*`) to a freshly-built
/// exception object: attach the explicit cause (suppressing the implicit
/// `__context__` chain) or apply a bare `from None` suppression. Every raise
/// builder calls this AFTER capturing `context` and BEFORE dispatching.
pub(super) fn apply_pending(exc_obj: &mut ExceptionObject) {
    with_exception_state(|state| {
        if let Some(cause) = state.pending_cause.take() {
            exc_obj.cause = Some(cause);
            exc_obj.suppress_context = true;
        }
        if std::mem::take(&mut state.pending_suppress) {
            exc_obj.suppress_context = true;
        }
    });
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

/// Dispatch the exception already stored in `current_exception` to the
/// nearest handler: walk the frame-pointer chain, prune the GC shadow stack
/// of the frames being abandoned, and jump. Exits with the CPython-style
/// unhandled print when no handler protects any live frame.
pub(super) unsafe fn dispatch_existing_exception() -> ! {
    if let Some(h) = super::unwind::find_handler() {
        // Shadow frames are stack slots of their owning functions; everything
        // below the handler frame's SP belongs to abandoned frames.
        crate::gc::unwind_below(h.sp);
        super::unwind::resume(h)
    }
    with_exception_state(|state| {
        if let Some(ref exc) = state.current_exception {
            print_unhandled_exception_full(exc);
        }
    });
    std::process::exit(1);
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
/// - field 0 = `.args` tuple: a single-element tuple containing the message
///   string, or — for a message-less raise like `ValueError()` — the EMPTY
///   tuple, matching CPython's `e.args == ()`. (Deliberate Phase-7 substrate
///   extension: the message pipeline cannot distinguish `ValueError()` from
///   `ValueError("")`, so the explicit-empty-string edge also yields `()` —
///   documented divergence, out of corpus.)
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

    let has_msg = exc.message_len > 0 && !exc.message.is_null();

    // Create the message string object (may trigger GC; instance is rooted)
    let msg_str = if has_msg {
        crate::string::rt_make_str(exc.message, exc.message_len)
    } else {
        std::ptr::null_mut()
    };
    // Root msg_str across rt_make_tuple
    roots[1] = msg_str;

    // Create the .args tuple (may trigger GC; both are rooted): 1 element with
    // a message, empty without one.
    let args_tuple = crate::tuple::rt_make_tuple(if has_msg { 1 } else { 0 });

    gc_pop();

    // Re-derive pointers through the (non-moving) GC heap
    let instance = roots[0];
    let msg_str = roots[1];

    if has_msg {
        crate::tuple::rt_tuple_set(args_tuple, 0, msg_str);
    }

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
