//! Call stack tracking for Python-style tracebacks.
//!
//! Maintains a lightweight call stack at runtime. Each compiled function
//! pushes an entry on entry and pops on return. When an exception is raised,
//! the stack is snapshot into a `Vec<TracebackEntry>` and stored in the
//! `ExceptionObject`. This is the only allocation — push/pop are pure
//! pointer bumps on a pre-allocated array.

use std::ptr;

/// Maximum call stack depth tracked for tracebacks.
/// Deeper calls silently stop recording (the program still runs).
const MAX_STACK_DEPTH: usize = 256;

/// One slot in the global call stack. Pointers reference static data
/// sections baked into the compiled binary, so they are valid for the
/// entire program lifetime.
#[repr(C)]
struct StackEntry {
    func_name: *const u8,
    func_name_len: usize,
    file_name: *const u8,
    file_name_len: usize,
    line_number: u32,
}

impl StackEntry {
    const EMPTY: Self = Self {
        func_name: ptr::null(),
        func_name_len: 0,
        file_name: ptr::null(),
        file_name_len: 0,
        line_number: 0,
    };
}

/// Global call stack (single-threaded program — no synchronisation needed).
static mut CALL_STACK: [StackEntry; MAX_STACK_DEPTH] = [StackEntry::EMPTY; MAX_STACK_DEPTH];
static mut STACK_DEPTH: usize = 0;

/// Owned snapshot of one stack frame, stored inside `ExceptionObject`.
#[derive(Debug, Clone)]
pub struct TracebackEntry {
    pub func_name: String,
    pub file_name: String,
    pub line_number: u32,
}

// ---------------------------------------------------------------------------
// Public C ABI — called from compiled code
// ---------------------------------------------------------------------------

/// Push a frame onto the call stack.
///
/// # Safety
/// `func_name` must point to `func_name_len` valid bytes.
/// `file_name` must point to `file_name_len` valid bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_stack_push(
    func_name: *const u8,
    func_name_len: usize,
    file_name: *const u8,
    file_name_len: usize,
    line_number: u32,
) {
    let depth = STACK_DEPTH;
    if depth >= MAX_STACK_DEPTH {
        return; // silently drop — program still runs
    }
    CALL_STACK[depth] = StackEntry {
        func_name,
        func_name_len,
        file_name,
        file_name_len,
        line_number,
    };
    STACK_DEPTH = depth + 1;
}

/// Pop the top frame from the call stack.
#[no_mangle]
pub extern "C" fn rt_stack_pop() {
    unsafe {
        STACK_DEPTH = STACK_DEPTH.saturating_sub(1);
    }
}

// ---------------------------------------------------------------------------
// Internal helpers — called from exception.rs
// ---------------------------------------------------------------------------

/// Current stack depth (for saving in `ExceptionFrame`).
pub fn current_depth() -> usize {
    unsafe { STACK_DEPTH }
}

/// Restore depth after longjmp (mirrors `gc::unwind_to`).
pub fn unwind_to(depth: usize) {
    unsafe {
        STACK_DEPTH = depth;
    }
}

/// Snapshot the current call stack into an owned `Vec`.
/// Only called when an exception is raised — not on the hot path.
pub fn capture_traceback() -> Vec<TracebackEntry> {
    unsafe {
        let depth = STACK_DEPTH;
        let mut entries = Vec::with_capacity(depth);
        for e in &CALL_STACK[..depth] {
            let func_name = if e.func_name.is_null() || e.func_name_len == 0 {
                "<unknown>".to_string()
            } else {
                let bytes = std::slice::from_raw_parts(e.func_name, e.func_name_len);
                String::from_utf8_lossy(bytes).into_owned()
            };
            let file_name = if e.file_name.is_null() || e.file_name_len == 0 {
                "<unknown>".to_string()
            } else {
                let bytes = std::slice::from_raw_parts(e.file_name, e.file_name_len);
                String::from_utf8_lossy(bytes).into_owned()
            };
            entries.push(TracebackEntry {
                func_name,
                file_name,
                line_number: e.line_number,
            });
        }
        entries
    }
}

/// Print a traceback to stderr in CPython format.
///
/// ```text
/// Traceback (most recent call last):
///   File "main.py", line 15, in main
///   File "main.py", line 8, in process
/// ```
pub fn format_traceback(entries: &[TracebackEntry]) {
    if entries.is_empty() {
        return;
    }
    eprintln!("Traceback (most recent call last):");
    for entry in entries {
        eprintln!(
            "  File \"{}\", line {}, in {}",
            entry.file_name, entry.line_number, entry.func_name
        );
    }
}
