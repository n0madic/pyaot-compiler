//! Real tracebacks: PC-chain capture at raise time + lazy resolution.
//!
//! Codegen bakes a traceback table into the binary (`pyaot_tb_table`,
//! layout pinned in `pyaot_core_defs::layout`): one record per compiled
//! Python function — base address, code size, display name, file path and
//! the `LineMarker`-driven `[start, end) → line` ranges. `main` registers it
//! via [`rt_tb_register_table`] before user code runs.
//!
//! A raise walks the frame-pointer chain (the same walk the unwinder does)
//! and snapshots the return PCs of every frame that resolves to a table
//! record — generated Python frames only; runtime Rust frames and the
//! `Tail` trampolines are not in the table and drop out naturally. The raw
//! PCs are stored in the `ExceptionObject`; names/files/lines are resolved
//! only if the exception goes unhandled and the traceback is printed.
//!
//! Code the optimizer inlined resolves to an *inline chain* (contract
//! extension, `TB_LOC_INLINE_FLAG`): one PC prints as the frames of every
//! function inlined at that point, so inlining does not erase frames.
//!
//! Documented divergences from CPython's traceback output: the source-line
//! echo and `^^^` anchors are not printed (the binary does not embed source
//! text), and a bare `raise` re-raise keeps the traceback captured at the
//! original raise point instead of appending the re-raise site.

use std::cell::UnsafeCell;

use pyaot_core_defs::layout::{
    TB_INLINE_FRAME_SIZE, TB_LOC_ENTRY_SIZE, TB_LOC_INLINE_FLAG, TB_RECORD_CODE_SIZE_OFFSET,
    TB_RECORD_FILE_LEN_OFFSET, TB_RECORD_FILE_OFF_OFFSET, TB_RECORD_LOC_OFF_OFFSET,
    TB_RECORD_NAME_LEN_OFFSET, TB_RECORD_NAME_OFF_OFFSET, TB_RECORD_SIZE,
};

/// One resolved function record. The string/loc slices point into the
/// program's static data section — valid for the process lifetime.
struct FnRecord {
    start: usize,
    code_size: u32,
    name: &'static str,
    file: &'static str,
    /// `(start, end, line)` u32 triples, sorted by `start` (raw LE bytes).
    locs: &'static [u8],
    loc_count: u32,
    /// The table base (inline-chain offsets are relative to it).
    table: *const u8,
}

struct TbTable {
    records: Vec<FnRecord>,
    sorted: bool,
}

/// Single-threaded interior mutability (the established runtime pattern).
struct TableCell(UnsafeCell<TbTable>);
unsafe impl Sync for TableCell {}

static TB_TABLE: TableCell = TableCell(UnsafeCell::new(TbTable {
    records: Vec::new(),
    sorted: false,
}));

/// Register the program's traceback table (called from generated `main`
/// before any user code). `ptr` addresses `count` fixed records followed by
/// the auxiliary string/line area; all offsets are relative to `ptr`.
///
/// # Safety
/// `ptr` must point to a well-formed table baked by codegen (static data).
#[no_mangle]
pub unsafe extern "C" fn rt_tb_register_table(ptr: *const u8, count: usize) {
    let read_usize = |off: usize| -> usize {
        let mut b = [0u8; 8];
        std::ptr::copy_nonoverlapping(ptr.add(off), b.as_mut_ptr(), 8);
        usize::from_le_bytes(b)
    };
    let read_u32 = |off: usize| -> u32 {
        let mut b = [0u8; 4];
        std::ptr::copy_nonoverlapping(ptr.add(off), b.as_mut_ptr(), 4);
        u32::from_le_bytes(b)
    };
    let str_at = |off: u32, len: u32| -> &'static str {
        let bytes = std::slice::from_raw_parts(ptr.add(off as usize), len as usize);
        std::str::from_utf8(bytes).unwrap_or("<bad-utf8>")
    };
    let table = &mut *TB_TABLE.0.get();
    table.records.reserve(count);
    for i in 0..count {
        let base = i * TB_RECORD_SIZE as usize;
        let loc_off = read_u32(base + TB_RECORD_LOC_OFF_OFFSET as usize) as usize;
        let loc_count = read_u32(loc_off);
        table.records.push(FnRecord {
            start: read_usize(base),
            code_size: read_u32(base + TB_RECORD_CODE_SIZE_OFFSET as usize),
            name: str_at(
                read_u32(base + TB_RECORD_NAME_OFF_OFFSET as usize),
                read_u32(base + TB_RECORD_NAME_LEN_OFFSET as usize),
            ),
            file: str_at(
                read_u32(base + TB_RECORD_FILE_OFF_OFFSET as usize),
                read_u32(base + TB_RECORD_FILE_LEN_OFFSET as usize),
            ),
            locs: std::slice::from_raw_parts(
                ptr.add(loc_off + 4),
                loc_count as usize * TB_LOC_ENTRY_SIZE as usize,
            ),
            loc_count,
            table: ptr,
        });
    }
    table.sorted = false;
}

/// A resolved traceback frame.
struct Frame {
    name: &'static str,
    file: &'static str,
    line: u32,
}

/// Read a little-endian u32 at `off` of a static table.
///
/// # Safety
/// `base + off .. + 4` must lie inside the registered table.
unsafe fn read_u32_at(base: *const u8, off: usize) -> u32 {
    let mut b = [0u8; 4];
    std::ptr::copy_nonoverlapping(base.add(off), b.as_mut_ptr(), 4);
    u32::from_le_bytes(b)
}

/// Resolve a PC to the frames it stands for, OUTERMOST first: one frame for
/// ordinary code, the whole inline chain for code the optimizer inlined.
/// The caller passes a return address minus one, so a call at the very end
/// of a line range attributes to the call itself, not to whatever
/// instruction follows it.
fn resolve(pc: usize) -> Option<Vec<Frame>> {
    let table = unsafe { &mut *TB_TABLE.0.get() };
    if table.records.is_empty() {
        return None;
    }
    if !table.sorted {
        table.records.sort_unstable_by_key(|r| r.start);
        table.sorted = true;
    }
    let idx = match table.records.binary_search_by_key(&pc, |r| r.start) {
        Ok(i) => i,
        Err(0) => return None,
        Err(i) => i - 1,
    };
    let rec = &table.records[idx];
    if pc - rec.start >= rec.code_size as usize {
        return None;
    }
    let off = (pc - rec.start) as u32;
    let mut line = 0u32;
    for e in 0..rec.loc_count as usize {
        let at = e * TB_LOC_ENTRY_SIZE as usize;
        let f = |k: usize| {
            u32::from_le_bytes([
                rec.locs[at + k],
                rec.locs[at + k + 1],
                rec.locs[at + k + 2],
                rec.locs[at + k + 3],
            ])
        };
        let (start, end, l) = (f(0), f(4), f(8));
        if off < start {
            break; // sorted by start: no later range can contain `off`
        }
        if off < end {
            line = l;
            break;
        }
    }
    if line & TB_LOC_INLINE_FLAG == 0 {
        return Some(vec![Frame {
            name: rec.name,
            file: rec.file,
            line,
        }]);
    }
    // SAFETY: codegen wrote a well-formed chain at this table offset.
    unsafe {
        let base = rec.table;
        let off = (line & !TB_LOC_INLINE_FLAG) as usize;
        let count = read_u32_at(base, off) as usize;
        let str_at = |o: u32, l: u32| -> &'static str {
            let bytes = std::slice::from_raw_parts(base.add(o as usize), l as usize);
            std::str::from_utf8(bytes).unwrap_or("<bad-utf8>")
        };
        let frames = (0..count)
            .map(|k| {
                let at = off + 4 + k * TB_INLINE_FRAME_SIZE as usize;
                let f = |i: usize| read_u32_at(base, at + 4 * i);
                Frame {
                    name: str_at(f(0), f(1)),
                    file: str_at(f(2), f(3)),
                    line: f(4),
                }
            })
            .collect();
        Some(frames)
    }
}

/// Snapshot the current call stack as raw return PCs (innermost first),
/// keeping only frames that resolve to compiled Python functions. Called at
/// every raise; resolution to names/lines is deferred to printing.
pub fn capture_traceback() -> Vec<usize> {
    let mut pcs = Vec::new();
    crate::exceptions::walk_return_pcs(|pc| {
        if resolve(pc.wrapping_sub(1)).is_some() {
            pcs.push(pc);
        }
    });
    pcs
}

/// Print a traceback to stderr in CPython's frame format (outermost first):
///
/// ```text
/// Traceback (most recent call last):
///   File "main.py", line 15, in <module>
///   File "main.py", line 8, in process
/// ```
///
/// (No source-line echo — the binary does not embed source text.)
pub fn format_traceback(pcs: &[usize]) {
    if pcs.is_empty() {
        return;
    }
    eprintln!("Traceback (most recent call last):");
    for pc in pcs.iter().rev() {
        for fr in resolve(pc.wrapping_sub(1)).unwrap_or_default() {
            eprintln!("  File \"{}\", line {}, in {}", fr.file, fr.line, fr.name);
        }
    }
}
