//! Thread-local exception state management.
//!
//! Holds the `ExceptionState` type and its thread-local `EXCEPTION_STATE` storage,
//! along with accessor helpers used throughout the exceptions module.

use std::cell::RefCell;

use super::core::ExceptionObject;
use super::core::ExceptionFrame;

/// Thread-local exception state
pub(super) struct ExceptionState {
    /// Pointer to current exception object (owned, must be freed)
    pub current_exception: Option<Box<ExceptionObject>>,
    /// Stack of exception handler frames (linked list)
    pub handler_stack: *mut ExceptionFrame,
    /// Exception being handled in current except block (for __context__ capture)
    /// When we enter an except handler, we save the current exception here.
    /// If a new exception is raised during handling, this becomes its __context__.
    pub handling_exception: Option<Box<ExceptionObject>>,
}

impl ExceptionState {
    pub const fn new() -> Self {
        Self {
            current_exception: None,
            handler_stack: std::ptr::null_mut(),
            handling_exception: None,
        }
    }
}

// Thread-local storage for exception state
thread_local! {
    pub(super) static EXCEPTION_STATE: RefCell<ExceptionState> = const { RefCell::new(ExceptionState::new()) };
}

/// Helper to access exception state (mutable)
pub(super) fn with_exception_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut ExceptionState) -> R,
{
    EXCEPTION_STATE.with(|state| f(&mut state.borrow_mut()))
}

/// Helper to access exception state (read-only, non-panicking).
/// Used by GC to collect exception pointers. Uses `try_borrow` so it returns a
/// default value instead of panicking when the RefCell is already mutably borrowed
/// (a re-entrant scenario that can theoretically occur during gc_stress testing).
pub(super) fn with_exception_state_ref<F, R: Default>(f: F) -> R
where
    F: FnOnce(&ExceptionState) -> R,
{
    EXCEPTION_STATE.with(|state| match state.try_borrow() {
        Ok(guard) => f(&guard),
        Err(_) => R::default(),
    })
}
