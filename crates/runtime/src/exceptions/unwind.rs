//! Table-based stack unwinding (the zero-cost replacement for the Phase-7
//! setjmp frames).
//!
//! Codegen emits every potentially-raising call inside a `try` region as a
//! Cranelift `try_call` and bakes a table of records — one per protected
//! machine call site — into the binary (`pyaot_exc_table`, layout pinned in
//! `pyaot_core_defs::layout`). `main` hands the table to
//! [`rt_exc_register_table`] before user code runs.
//!
//! A raise walks the frame-pointer chain from inside the runtime: every
//! frame's return address is looked up (exact match) in the sorted table;
//! the first hit identifies the protected call site and its handler. The
//! runtime restores SP/FP to that frame and jumps to the handler entry —
//! [`resume`]. Frames in between (generated and runtime Rust frames alike)
//! are simply abandoned: Cranelift's `try_call` contract is that ALL
//! registers are clobbered at a protected call site, so the handler frame
//! keeps nothing in registers, and `Drop`s of skipped Rust frames do not run
//! (the owned-message discipline, PITFALLS B2, is unchanged from longjmp).
//!
//! Requirements: frame pointers everywhere — Cranelift compiles with
//! `preserve_frame_pointers=true`; the runtime is built for targets where
//! Rust keeps frame pointers (mandatory on macOS arm64; enable
//! `force-frame-pointers` for Linux builds).

use std::cell::UnsafeCell;

use pyaot_core_defs::layout::{
    EXC_RECORD_FRAME_OFF_OFFSET, EXC_RECORD_HANDLER_OFF_OFFSET, EXC_RECORD_SITE_OFF_OFFSET,
    EXC_TABLE_RECORD_SIZE,
};

/// One resolved record: the absolute return-PC of a protected call site, the
/// absolute handler entry PC, and the FP-to-SP distance at the site.
#[derive(Clone, Copy)]
struct SiteRecord {
    ret_pc: usize,
    handler_pc: usize,
    frame_off: u32,
}

struct UnwindTable {
    records: Vec<SiteRecord>,
    sorted: bool,
}

/// Single-threaded interior mutability (the established runtime pattern).
struct TableCell(UnsafeCell<UnwindTable>);
unsafe impl Sync for TableCell {}

static UNWIND_TABLE: TableCell = TableCell(UnsafeCell::new(UnwindTable {
    records: Vec::new(),
    sorted: false,
}));

/// Register a module's unwind table (called from generated `main` before any
/// user code). `ptr` addresses `count` records of `EXC_TABLE_RECORD_SIZE`
/// bytes each; function addresses inside are already relocated, so each
/// record resolves to absolute PCs here.
///
/// # Safety
/// `ptr` must point to `count` well-formed records baked by codegen.
#[no_mangle]
pub unsafe extern "C" fn rt_exc_register_table(ptr: *const u8, count: usize) {
    let table = &mut *UNWIND_TABLE.0.get();
    table.records.reserve(count);
    for i in 0..count {
        let rec = ptr.add(i * EXC_TABLE_RECORD_SIZE as usize);
        let read_usize = |off: usize| -> usize {
            let mut b = [0u8; 8];
            std::ptr::copy_nonoverlapping(rec.add(off), b.as_mut_ptr(), 8);
            usize::from_le_bytes(b)
        };
        let read_u32 = |off: u32| -> u32 {
            let mut b = [0u8; 4];
            std::ptr::copy_nonoverlapping(rec.add(off as usize), b.as_mut_ptr(), 4);
            u32::from_le_bytes(b)
        };
        let func_addr = read_usize(0);
        table.records.push(SiteRecord {
            ret_pc: func_addr + read_u32(EXC_RECORD_SITE_OFF_OFFSET) as usize,
            handler_pc: func_addr + read_u32(EXC_RECORD_HANDLER_OFF_OFFSET) as usize,
            frame_off: read_u32(EXC_RECORD_FRAME_OFF_OFFSET),
        });
    }
    table.sorted = false;
}

/// A located handler: where to jump and the SP/FP to restore.
pub(super) struct Handler {
    pub pc: usize,
    pub sp: usize,
    pub fp: usize,
}

/// Hard cap on the frame walk — a corrupted FP chain must not loop forever.
const MAX_FRAMES: usize = 1 << 20;

/// Walk the frame-pointer chain from the current (runtime) frame outward,
/// looking each return address up in the unwind table. Returns the innermost
/// handler, or `None` (unhandled — caller prints and exits).
///
/// Frame layout on both supported architectures: `[fp]` holds the caller's
/// frame pointer, `[fp + 8]` the return address. The chain terminates at a
/// null or non-monotonic FP (the process entry frame).
pub(super) fn find_handler() -> Option<Handler> {
    let table = unsafe { &mut *UNWIND_TABLE.0.get() };
    if table.records.is_empty() {
        return None;
    }
    if !table.sorted {
        table.records.sort_unstable_by_key(|r| r.ret_pc);
        table.sorted = true;
    }

    let mut fp = current_fp();
    for _ in 0..MAX_FRAMES {
        if fp == 0 || fp & 0x7 != 0 {
            return None;
        }
        let (caller_fp, ret_pc) = unsafe {
            (
                *(fp as *const usize),
                *((fp + 8) as *const usize),
            )
        };
        if let Ok(i) = table
            .records
            .binary_search_by_key(&ret_pc, |r| r.ret_pc)
        {
            let rec = table.records[i];
            // `ret_pc` lies inside the protected caller, whose frame pointer
            // is the one we just loaded from this frame's back-link.
            return Some(Handler {
                pc: rec.handler_pc,
                sp: caller_fp - rec.frame_off as usize,
                fp: caller_fp,
            });
        }
        // The stack grows down; a walk that does not move strictly upward
        // has left the well-formed chain.
        if caller_fp <= fp {
            return None;
        }
        fp = caller_fp;
    }
    None
}

/// Walk the frame-pointer chain from the current frame outward, visiting
/// each return PC (innermost first) — the traceback capture's view of the
/// same walk [`find_handler`] performs. Stops on the same sanity conditions.
pub(crate) fn walk_return_pcs(mut visit: impl FnMut(usize)) {
    let mut fp = current_fp();
    for _ in 0..MAX_FRAMES {
        if fp == 0 || fp & 0x7 != 0 {
            return;
        }
        let (caller_fp, ret_pc) = unsafe { (*(fp as *const usize), *((fp + 8) as *const usize)) };
        visit(ret_pc);
        if caller_fp <= fp {
            return;
        }
        fp = caller_fp;
    }
}

/// Read the current frame pointer.
#[inline(always)]
fn current_fp() -> usize {
    let fp: usize;
    #[cfg(target_arch = "aarch64")]
    unsafe {
        std::arch::asm!("mov {}, x29", out(reg) fp, options(nomem, nostack, preserves_flags));
    }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        std::arch::asm!("mov {}, rbp", out(reg) fp, options(nomem, nostack, preserves_flags));
    }
    fp
}

// ── resume stub ─────────────────────────────────────────────────────────────
//
// Restores SP and FP to the handler's frame and jumps to the handler entry.
// Nothing else needs restoring: the `try_call` ABI contract clobbers every
// register at a protected call site, so the handler reloads all live values
// from its own frame. A global_asm stub (not inline asm) because Rust inline
// asm cannot legally rewrite sp/fp in an ordinary function body.

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
std::arch::global_asm!(
    ".p2align 2",
    ".globl _pyaot_resume_to_handler",
    "_pyaot_resume_to_handler:",
    "mov sp, x1",
    "mov x29, x2",
    "br x0",
);

#[cfg(all(target_arch = "aarch64", not(target_os = "macos")))]
std::arch::global_asm!(
    ".p2align 2",
    ".globl pyaot_resume_to_handler",
    "pyaot_resume_to_handler:",
    "mov sp, x1",
    "mov x29, x2",
    "br x0",
);

#[cfg(all(target_arch = "x86_64", target_os = "macos"))]
std::arch::global_asm!(
    ".p2align 4",
    ".globl _pyaot_resume_to_handler",
    "_pyaot_resume_to_handler:",
    "mov rsp, rsi",
    "mov rbp, rdx",
    "jmp rdi",
);

#[cfg(all(target_arch = "x86_64", not(target_os = "macos")))]
std::arch::global_asm!(
    ".p2align 4",
    ".globl pyaot_resume_to_handler",
    "pyaot_resume_to_handler:",
    "mov rsp, rsi",
    "mov rbp, rdx",
    "jmp rdi",
);

extern "C" {
    fn pyaot_resume_to_handler(pc: usize, sp: usize, fp: usize) -> !;
}

/// Jump to a located handler, restoring its frame's SP/FP.
///
/// # Safety
/// `h` must come from [`find_handler`] on the CURRENT stack — the target
/// frame must still be live below the caller.
pub(super) unsafe fn resume(h: Handler) -> ! {
    pyaot_resume_to_handler(h.pc, h.sp, h.fp)
}
