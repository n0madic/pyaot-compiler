//! Thread-local exception state management.
//!
//! Holds the `ExceptionState` type and its thread-local `EXCEPTION_STATE` storage,
//! along with accessor helpers used throughout the exceptions module.

use std::cell::RefCell;

use super::core::ExceptionObject;

/// Thread-local exception state
pub(super) struct ExceptionState {
    /// Pointer to current exception object (owned, must be freed)
    pub current_exception: Option<Box<ExceptionObject>>,
    /// Stack of exceptions being handled (CPython's `exc_info` stack), one
    /// entry per active `except` / `finally`-exceptional / `with`-exit block,
    /// innermost last. `rt_exc_start_handling` pushes the in-flight exception
    /// (`None` when there is none, to keep pushes and pops balanced),
    /// `rt_exc_end_handling` pops. The compiler pops on EVERY exit of such a
    /// block — normal, early (`return`/`break`/`continue`) and exceptional (a
    /// cleanup landing pad around the block body) — so a bare `raise` after a
    /// nested, already-handled exception re-raises the right one. The top
    /// entry is what a bare `raise` re-raises and what a new raise captures as
    /// its `__context__`.
    pub handling_stack: Vec<Option<Box<ExceptionObject>>>,
    /// Pending explicit cause armed by `rt_exc_arm_cause_*` (`raise X from Y`,
    /// PEP 3134). The next raise builder takes this and attaches it as the new
    /// exception's `__cause__`. Live only on the straight-line arm→raise edge
    /// (no safepoint between), but scanned for GC roots defensively.
    pub pending_cause: Option<Box<ExceptionObject>>,
    /// Pending `suppress_context` armed by `rt_exc_arm_suppress` (`raise X from
    /// None`). The next raise builder applies it (cause stays `None`).
    pub pending_suppress: bool,
}

impl ExceptionState {
    pub const fn new() -> Self {
        Self {
            current_exception: None,
            handling_stack: Vec::new(),
            pending_cause: None,
            pending_suppress: false,
        }
    }
}

impl ExceptionState {
    /// The exception being handled by the innermost active handler, if any.
    pub fn handled(&self) -> Option<&ExceptionObject> {
        self.handling_stack.last().and_then(|e| e.as_deref())
    }

    /// Mutable access to the innermost handled exception.
    pub fn handled_mut(&mut self) -> Option<&mut ExceptionObject> {
        self.handling_stack
            .last_mut()
            .and_then(|e| e.as_deref_mut())
    }

    /// The in-flight exception, else the innermost handled one (handler
    /// bodies run after `rt_exc_start_handling` moved it to the stack).
    pub fn current_or_handled(&self) -> Option<&ExceptionObject> {
        self.current_exception.as_deref().or_else(|| self.handled())
    }

    /// Mutable form of [`Self::current_or_handled`].
    pub fn current_or_handled_mut(&mut self) -> Option<&mut ExceptionObject> {
        match self.current_exception {
            Some(ref mut e) => Some(&mut **e),
            None => self.handled_mut(),
        }
    }

    /// The implicit `__context__` for a NEW exception raised now: a copy of
    /// the innermost handled exception (it stays on the handling stack, since
    /// its handler is still active). `new_instance` is the instance being
    /// raised, if known: re-raising the handled exception itself (`except E
    /// as e: raise e`) must not chain it to itself, matching CPython.
    pub fn new_context(
        &self,
        new_instance: *mut crate::object::Obj,
    ) -> Option<Box<ExceptionObject>> {
        let handled = self.handled()?;
        if !new_instance.is_null() && handled.instance == new_instance {
            return None;
        }
        Some(Box::new(handled.deep_clone()))
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

/// Can the exception state be borrowed right now (no live `borrow_mut`)?
pub(super) fn exception_state_borrowable() -> bool {
    EXCEPTION_STATE.with(|state| state.try_borrow().is_ok())
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
