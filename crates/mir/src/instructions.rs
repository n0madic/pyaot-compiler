//! MIR instructions

use pyaot_core_defs::BuiltinFunctionKind;
use pyaot_types::Type;
use pyaot_utils::{FuncId, LocalId};

use pyaot_utils::Span;

use pyaot_utils::BlockId;

use crate::{BinOp, Constant, Operand, RuntimeFunc, UnOp};

/// MIR Instruction
#[derive(Debug, Clone)]
pub struct Instruction {
    pub kind: InstructionKind,
    /// Source location from HIR (None for synthetic instructions)
    pub span: Option<Span>,
}

#[derive(Debug, Clone)]
pub enum InstructionKind {
    /// Assign a constant to a local
    Const { dest: LocalId, value: Constant },

    /// Binary operation: dest = left op right
    BinOp {
        dest: LocalId,
        op: BinOp,
        left: Operand,
        right: Operand,
    },

    /// Unary operation: dest = op operand
    UnOp {
        dest: LocalId,
        op: UnOp,
        operand: Operand,
    },

    /// Call function: dest = func(args)
    Call {
        dest: LocalId,
        func: Operand,
        args: Vec<Operand>,
    },

    /// Call function by FuncId directly (static dispatch)
    CallDirect {
        dest: LocalId,
        func: FuncId,
        args: Vec<Operand>,
    },

    /// Call function by name (for cross-module calls, resolved at codegen time)
    CallNamed {
        dest: LocalId,
        name: String,
        args: Vec<Operand>,
    },

    /// Virtual method call via vtable lookup (dynamic dispatch)
    /// dest = obj->vtable.methods[slot](obj, args...)
    CallVirtual {
        dest: LocalId,
        obj: Operand,       // The receiver (self)
        slot: usize,        // Vtable slot index
        args: Vec<Operand>, // Additional arguments (self is prepended at codegen)
    },

    /// Name-based virtual method call (Protocol dispatch)
    /// Looks up the vtable slot by method name hash on the actual runtime object.
    /// Used when the compile-time type is a Protocol and vtable slots may differ.
    CallVirtualNamed {
        dest: LocalId,
        obj: Operand,       // The receiver (self)
        name_hash: u64,     // FNV-1a hash of the method name
        args: Vec<Operand>, // Additional arguments (self is prepended at codegen)
    },

    /// Get function address (pointer) for passing to runtime functions (e.g., sorted key)
    FuncAddr { dest: LocalId, func: FuncId },

    /// Get builtin function address from runtime table (for first-class builtin functions)
    /// Used when builtins like len, str, int are passed as values to map/filter/sorted
    BuiltinAddr {
        dest: LocalId,
        builtin: BuiltinFunctionKind,
    },

    /// Call runtime function
    RuntimeCall {
        dest: LocalId,
        func: RuntimeFunc,
        args: Vec<Operand>,
    },

    /// Copy value: dest = src
    Copy { dest: LocalId, src: Operand },

    /// Register GC root (for shadow stack)
    GcPush { frame: LocalId },

    /// Unregister GC root
    GcPop,

    /// Allocate object
    GcAlloc {
        dest: LocalId,
        ty: Type,
        size: usize,
    },

    // ==================== Type conversion instructions ====================
    /// Convert float to int (truncate towards zero)
    FloatToInt { dest: LocalId, src: Operand },
    /// Convert bool (i8) to int (i64) - zero-extend
    BoolToInt { dest: LocalId, src: Operand },
    /// Convert int to float
    IntToFloat { dest: LocalId, src: Operand },
    /// Get float bits as int (bitcast f64 to i64)
    FloatBits { dest: LocalId, src: Operand },
    /// Reinterpret raw int bits as float (bitcast i64 to f64)
    /// Used when an iterator yields float values encoded as raw i64 bits.
    IntBitsToFloat { dest: LocalId, src: Operand },

    // ==================== Math instructions ====================
    /// Absolute value of float: abs(x)
    FloatAbs { dest: LocalId, src: Operand },

    // ==================== Exception handling instructions ====================
    /// Push exception frame for try block
    /// frame_local holds pointer to stack-allocated ExceptionFrame
    ExcPushFrame { frame_local: LocalId },

    /// Pop exception frame (normal try exit)
    ExcPopFrame,

    /// Get current exception type tag (result in dest, -1 if no exception)
    ExcGetType { dest: LocalId },

    /// Clear current exception (after handling)
    ExcClear,

    /// Check if exception is pending (result in dest: 0 or 1)
    ExcHasException { dest: LocalId },

    /// Get current exception as string object (for `except E as e:` binding)
    /// Result is a StrObj pointer with the exception message
    ExcGetCurrent { dest: LocalId },

    /// Check if current exception matches a type tag (for typed except handlers)
    /// Result is 1 if matches, 0 otherwise
    ExcCheckType { dest: LocalId, type_tag: u8 },

    /// Check if current exception is instance of a class (with inheritance support)
    /// For custom exception classes (class_id 27+) and built-in exceptions (class_id 0-26)
    /// Uses rt_exc_isinstance_class which walks the inheritance chain
    /// Result is 1 if matches, 0 otherwise
    ExcCheckClass { dest: LocalId, class_id: u8 },

    /// Mark start of exception handling (preserves exception for __context__)
    /// Called when entering an except handler to save the current exception.
    /// If a new exception is raised during handling, this saved exception
    /// becomes its __context__ (implicit exception chaining, PEP 3134).
    ExcStartHandling,

    /// Mark end of exception handling (clears handled exception)
    /// Called when exiting an except handler normally (not via raise/reraise).
    /// Clears the saved handling exception since we're done handling.
    ExcEndHandling,

    /// SSA type refinement: `dest = refine(src as ty)`. Runtime-free —
    /// lowers to a `Copy`, propagating the same bit pattern. The purpose is
    /// purely compile-time: `dest` carries a narrower static `Type` than
    /// `src` so downstream passes (flow-sensitive type inference, S1.8) can
    /// specialise dispatch on dominated uses of `dest`.
    ///
    /// Inserted at the entry of a block dominated by an `isinstance`
    /// success, or anywhere else the type lattice proves a narrower type
    /// holds along a control-flow edge. See `ARCHITECTURE_REFACTOR.md`
    /// §1.4. S1.7 introduces the variant; S1.8 starts emitting it.
    Refine {
        dest: LocalId,
        src: Operand,
        ty: Type,
    },

    /// SSA φ-node: `dest = φ((src_1 from pred_1), (src_2 from pred_2), …)`.
    ///
    /// Phi instructions must appear at the **head** of their basic block —
    /// that is, before any non-Phi instruction. `sources.len()` must equal
    /// the number of CFG predecessors of the containing block, and each
    /// `BlockId` in `sources` must be exactly one of those predecessors
    /// (no duplicates, no extras). The `crate::ssa_check` module enforces
    /// both invariants when `Function::is_ssa` is true.
    ///
    /// Added in Phase 1 S1.5 (`ARCHITECTURE_REFACTOR.md` §1.3). Before
    /// S1.6 lands Cytron-style renaming, no function actually emits Phi
    /// instructions; the codegen path is present but dormant.
    Phi {
        dest: LocalId,
        sources: Vec<(BlockId, Operand)>,
    },
}
