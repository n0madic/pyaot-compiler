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

    // ==================== Value tag boxing/unboxing instructions ====================
    /// Unified box: tag a primitive into a tagged `Value`. Codegen lowers
    /// based on `src_type`:
    /// - `Int`  → `(src << 3) | 1`
    /// - `Bool` → `(src << 3) | 3`  (with i8→i64 zext)
    /// - `Float` → `rt_box_float(src)`  (heap-allocated `FloatObj`)
    /// - `None` → `rt_box_none()`       (singleton `NoneObj`)
    ///
    /// Pass-through guard: if `src`'s operand type is already a tagged
    /// `Value` (`HeapAny` / `Union` / `Any`) **and** `src_type == Float`,
    /// codegen emits a no-op copy — eliminates the historical
    /// `emit_value_slot` Float-passthrough hack (lib.rs:108-113).
    BoxValue {
        dest: LocalId,
        src: Operand,
        src_type: Type,
    },

    /// Unified unbox: extract a primitive from a tagged `Value`. Codegen
    /// lowers based on `dest_type`:
    /// - `Int`  → arithmetic right shift `>> 3`
    /// - `Bool` → `((v >> 3) & 1) as i8`
    /// - `Float` → `rt_unbox_float(src)`  (tag-dispatching unbox)
    UnboxValue {
        dest: LocalId,
        src: Operand,
        dest_type: Type,
    },

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

/// True if the runtime function attached to a `RuntimeCall` instruction
/// produces no observable SSA value at codegen time. The `dest` field on
/// such calls is a placeholder MIR often reuses as a side-effectful
/// target (e.g. `rt_list_set` writes into the same list local); renaming
/// it as a fresh SSA def would shadow live state.
///
/// Visibility note: `pub` so other crates (optimizer, codegen) can share
/// the predicate. A mismatch would cause `ssa_check` to flag valid MIR
/// as invalid.
pub fn runtime_call_is_void(func: &RuntimeFunc) -> bool {
    match func {
        // Descriptor-based: returns field is authoritative.
        RuntimeFunc::Call(def) => def.returns.is_none(),
        // Legacy variants known to leave `dest` untouched at codegen.
        RuntimeFunc::AssertFail
        | RuntimeFunc::PrintValue(_)
        | RuntimeFunc::ExcRegisterClassName
        | RuntimeFunc::ExcRaise
        | RuntimeFunc::ExcReraise
        | RuntimeFunc::ExcClear => true,
        // These return values or are dispatched via terminators whose own
        // codegen still writes through `dest`, so keep them as SSA defs.
        RuntimeFunc::MakeStr
        | RuntimeFunc::MakeBytes
        | RuntimeFunc::ExcSetjmp
        | RuntimeFunc::ExcGetType
        | RuntimeFunc::ExcHasException
        | RuntimeFunc::ExcGetCurrent
        | RuntimeFunc::ExcIsinstanceClass
        | RuntimeFunc::ExcRaiseCustom
        | RuntimeFunc::ExcInstanceStr => false,
    }
}

impl InstructionKind {
    /// Returns the `LocalId` defined (written) by this instruction, if any.
    ///
    /// Special case: a `RuntimeCall` whose `func` is void-returning
    /// (`runtime_call_is_void`) does NOT produce a new SSA value — its
    /// `dest` slot is a placeholder the codegen leaves untouched. Such
    /// instructions return `None`.
    ///
    /// `GcPush.frame` and `ExcPushFrame.frame_local` are classified as
    /// defs even though the value is Cranelift-synthesized at codegen
    /// time — the SSA checker needs them treated as defining sites.
    ///
    /// Side-effect-only instructions (`GcPop`, `ExcPopFrame`, `ExcClear`,
    /// `ExcStartHandling`, `ExcEndHandling`) return `None`.
    pub fn def(&self) -> Option<LocalId> {
        use InstructionKind::*;
        match self {
            RuntimeCall { dest, func, .. } => {
                if runtime_call_is_void(func) {
                    None
                } else {
                    Some(*dest)
                }
            }
            Const { dest, .. }
            | BinOp { dest, .. }
            | UnOp { dest, .. }
            | Call { dest, .. }
            | CallDirect { dest, .. }
            | CallNamed { dest, .. }
            | CallVirtual { dest, .. }
            | CallVirtualNamed { dest, .. }
            | FuncAddr { dest, .. }
            | BuiltinAddr { dest, .. }
            | Copy { dest, .. }
            | GcAlloc { dest, .. }
            | FloatToInt { dest, .. }
            | BoolToInt { dest, .. }
            | IntToFloat { dest, .. }
            | FloatBits { dest, .. }
            | IntBitsToFloat { dest, .. }
            | BoxValue { dest, .. }
            | UnboxValue { dest, .. }
            | FloatAbs { dest, .. }
            | ExcGetType { dest }
            | ExcHasException { dest }
            | ExcGetCurrent { dest }
            | ExcCheckType { dest, .. }
            | ExcCheckClass { dest, .. }
            | Phi { dest, .. }
            | Refine { dest, .. } => Some(*dest),
            GcPush { frame } => Some(*frame),
            ExcPushFrame { frame_local } => Some(*frame_local),
            GcPop | ExcPopFrame | ExcClear | ExcStartHandling | ExcEndHandling => None,
        }
    }

    /// Apply `f` to each `LocalId` used (read) by this instruction.
    ///
    /// `Phi` instructions iterate their `sources: Vec<(BlockId, Operand)>`.
    /// `GcPush` / `ExcPushFrame` classify their frame field as a def (see
    /// [`Self::def`]) so they have NO uses here — emitting one would make
    /// the def "depend on itself" in reachability analysis.
    pub fn for_each_use<F: FnMut(LocalId)>(&self, mut f: F) {
        use InstructionKind::*;
        fn push<F: FnMut(LocalId)>(op: &Operand, f: &mut F) {
            if let Operand::Local(id) = op {
                f(*id);
            }
        }
        match self {
            Const { .. }
            | FuncAddr { .. }
            | BuiltinAddr { .. }
            | GcAlloc { .. }
            | GcPop
            | GcPush { .. }
            | ExcPopFrame
            | ExcPushFrame { .. }
            | ExcClear
            | ExcGetType { .. }
            | ExcHasException { .. }
            | ExcGetCurrent { .. }
            | ExcCheckType { .. }
            | ExcCheckClass { .. }
            | ExcStartHandling
            | ExcEndHandling => {}
            BinOp { left, right, .. } => {
                push(left, &mut f);
                push(right, &mut f);
            }
            UnOp { operand, .. } => push(operand, &mut f),
            Copy { src, .. }
            | FloatToInt { src, .. }
            | BoolToInt { src, .. }
            | IntToFloat { src, .. }
            | FloatBits { src, .. }
            | IntBitsToFloat { src, .. }
            | BoxValue { src, .. }
            | UnboxValue { src, .. }
            | FloatAbs { src, .. }
            | Refine { src, .. } => push(src, &mut f),
            Call { func, args, .. } => {
                push(func, &mut f);
                for a in args {
                    push(a, &mut f);
                }
            }
            CallDirect { args, .. } | CallNamed { args, .. } | RuntimeCall { args, .. } => {
                for a in args {
                    push(a, &mut f);
                }
            }
            CallVirtual { obj, args, .. } | CallVirtualNamed { obj, args, .. } => {
                push(obj, &mut f);
                for a in args {
                    push(a, &mut f);
                }
            }
            Phi { sources, .. } => {
                for (_, op) in sources {
                    push(op, &mut f);
                }
            }
        }
    }

    /// Apply `f` to a mutable reference to each `LocalId` used by this
    /// instruction. Used by SSA renaming to substitute uses with their
    /// current top-of-stack name.
    ///
    /// **Phi NOTE**: Phi `sources` are NOT visited here. SSA construction
    /// fills phi sources from each predecessor's `rename_block` via
    /// `fill_phi_sources` — a renaming pass that walks Phi here would
    /// rewrite the predecessor-tagged slot with the *current* block's
    /// stack top, which is wrong.
    pub fn for_each_use_mut<F: FnMut(&mut LocalId)>(&mut self, mut f: F) {
        use InstructionKind::*;
        fn push<F: FnMut(&mut LocalId)>(op: &mut Operand, f: &mut F) {
            if let Operand::Local(id) = op {
                f(id);
            }
        }
        match self {
            Const { .. }
            | FuncAddr { .. }
            | BuiltinAddr { .. }
            | GcAlloc { .. }
            | GcPop
            | GcPush { .. }
            | ExcPopFrame
            | ExcPushFrame { .. }
            | ExcClear
            | ExcGetType { .. }
            | ExcHasException { .. }
            | ExcGetCurrent { .. }
            | ExcCheckType { .. }
            | ExcCheckClass { .. }
            | ExcStartHandling
            | ExcEndHandling => {}
            BinOp { left, right, .. } => {
                push(left, &mut f);
                push(right, &mut f);
            }
            UnOp { operand, .. } => push(operand, &mut f),
            Copy { src, .. }
            | FloatToInt { src, .. }
            | BoolToInt { src, .. }
            | IntToFloat { src, .. }
            | FloatBits { src, .. }
            | IntBitsToFloat { src, .. }
            | BoxValue { src, .. }
            | UnboxValue { src, .. }
            | FloatAbs { src, .. }
            | Refine { src, .. } => push(src, &mut f),
            Call { func, args, .. } => {
                push(func, &mut f);
                for a in args {
                    push(a, &mut f);
                }
            }
            CallDirect { args, .. } | CallNamed { args, .. } | RuntimeCall { args, .. } => {
                for a in args {
                    push(a, &mut f);
                }
            }
            CallVirtual { obj, args, .. } | CallVirtualNamed { obj, args, .. } => {
                push(obj, &mut f);
                for a in args {
                    push(a, &mut f);
                }
            }
            Phi { .. } => {
                // φ sources are populated by the predecessor's
                // `rename_block` / `fill_phi_sources`, not here.
            }
        }
    }

    /// Rewrite the destination local of this instruction to `fresh`. Used
    /// by SSA renaming to assign each def a fresh LocalId.
    ///
    /// Panics in debug builds when called on a side-effect-only
    /// instruction (`GcPop`, `ExcPopFrame`, `ExcClear`, `ExcStartHandling`,
    /// `ExcEndHandling`) — such instructions have no def. Callers should
    /// gate on [`Self::def`] returning `Some` first.
    pub fn set_def(&mut self, fresh: LocalId) {
        use InstructionKind::*;
        match self {
            Const { dest, .. }
            | BinOp { dest, .. }
            | UnOp { dest, .. }
            | Call { dest, .. }
            | CallDirect { dest, .. }
            | CallNamed { dest, .. }
            | CallVirtual { dest, .. }
            | CallVirtualNamed { dest, .. }
            | FuncAddr { dest, .. }
            | BuiltinAddr { dest, .. }
            | RuntimeCall { dest, .. }
            | Copy { dest, .. }
            | GcAlloc { dest, .. }
            | FloatToInt { dest, .. }
            | BoolToInt { dest, .. }
            | IntToFloat { dest, .. }
            | FloatBits { dest, .. }
            | IntBitsToFloat { dest, .. }
            | BoxValue { dest, .. }
            | UnboxValue { dest, .. }
            | FloatAbs { dest, .. }
            | ExcGetType { dest }
            | ExcHasException { dest }
            | ExcGetCurrent { dest }
            | ExcCheckType { dest, .. }
            | ExcCheckClass { dest, .. }
            | Phi { dest, .. }
            | Refine { dest, .. } => {
                *dest = fresh;
            }
            GcPush { frame } => {
                *frame = fresh;
            }
            ExcPushFrame { frame_local } => {
                *frame_local = fresh;
            }
            GcPop | ExcPopFrame | ExcClear | ExcStartHandling | ExcEndHandling => {
                debug_assert!(false, "set_def called on a defless instruction");
            }
        }
    }

    /// Returns the primitive type being boxed if this instruction is a boxing
    /// instruction that wraps a raw primitive into a tagged `Value` slot, or
    /// `None` otherwise.
    ///
    /// Covers the three distinct boxing forms:
    /// - `BoxValue { src_type: Int | Bool | Float | None, .. }` → the `src_type`
    /// - `RuntimeCall(RT_BOX_FLOAT, ..)` → `Type::Float`
    /// - `RuntimeCall(RT_BOX_NONE, ..)` → `Type::None`
    ///
    /// This is the inverse of `abi_repair::box_primitive_inst`: given the
    /// instruction that boxed a value, recovers the logical type of the
    /// unboxed primitive. Callers that only handle raw-unboxable primitives
    /// (Int / Bool / Float — not None) should filter with
    /// `filter(|t| !matches!(t, Type::None))`.
    pub fn boxed_primitive_type(&self) -> Option<Type> {
        match self {
            InstructionKind::BoxValue { src_type, .. }
                if matches!(src_type, Type::Int | Type::Bool | Type::Float | Type::None) =>
            {
                Some(src_type.clone())
            }
            InstructionKind::RuntimeCall {
                func: RuntimeFunc::Call(def),
                ..
            } if std::ptr::eq(*def, &pyaot_core_defs::runtime_func_def::RT_BOX_FLOAT) => {
                Some(Type::Float)
            }
            InstructionKind::RuntimeCall {
                func: RuntimeFunc::Call(def),
                ..
            } if std::ptr::eq(*def, &pyaot_core_defs::runtime_func_def::RT_BOX_NONE) => {
                Some(Type::None)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod value_tag_kinds {
    use super::*;
    use pyaot_types::Type;
    use pyaot_utils::LocalId;

    fn lid(id: u32) -> LocalId {
        LocalId::from(id)
    }

    fn mk(kind: InstructionKind) -> Instruction {
        Instruction { kind, span: None }
    }

    #[test]
    fn box_value_constructs() {
        let dest = lid(1);
        let src = Operand::Constant(crate::Constant::Int(42));
        let inst = mk(InstructionKind::BoxValue {
            dest,
            src: src.clone(),
            src_type: Type::Int,
        });
        assert!(matches!(
            inst.kind,
            InstructionKind::BoxValue {
                src_type: Type::Int,
                ..
            }
        ));
    }

    #[test]
    fn unbox_value_constructs() {
        let dest = lid(2);
        let src_local = lid(1);
        let inst = mk(InstructionKind::UnboxValue {
            dest,
            src: Operand::Local(src_local),
            dest_type: Type::Int,
        });
        assert!(matches!(
            inst.kind,
            InstructionKind::UnboxValue {
                dest_type: Type::Int,
                ..
            }
        ));
    }

    /// Verify that BoxValue/UnboxValue dest fields are accessible.
    #[test]
    fn instruction_dest_returns_dest() {
        let d = lid(10);
        let const_src = Operand::Constant(crate::Constant::Int(0));
        let local_src = Operand::Local(lid(9));

        for kind in [
            InstructionKind::BoxValue {
                dest: d,
                src: const_src.clone(),
                src_type: Type::Int,
            },
            InstructionKind::UnboxValue {
                dest: d,
                src: local_src.clone(),
                dest_type: Type::Int,
            },
            InstructionKind::BoxValue {
                dest: d,
                src: const_src.clone(),
                src_type: Type::Bool,
            },
            InstructionKind::UnboxValue {
                dest: d,
                src: local_src.clone(),
                dest_type: Type::Bool,
            },
        ] {
            let dest_field = match &kind {
                InstructionKind::BoxValue { dest, .. }
                | InstructionKind::UnboxValue { dest, .. } => *dest,
                _ => unreachable!(),
            };
            assert_eq!(dest_field, d);
        }
    }

    /// Verify that a `Local` src is recognised as a use.
    #[test]
    fn local_src_is_a_use() {
        let src_local = lid(7);
        for kind in [
            InstructionKind::BoxValue {
                dest: lid(10),
                src: Operand::Local(src_local),
                src_type: Type::Int,
            },
            InstructionKind::UnboxValue {
                dest: lid(10),
                src: Operand::Local(src_local),
                dest_type: Type::Int,
            },
        ] {
            let src_operand = match &kind {
                InstructionKind::BoxValue { src, .. } | InstructionKind::UnboxValue { src, .. } => {
                    src.clone()
                }
                _ => unreachable!(),
            };
            assert_eq!(src_operand, Operand::Local(src_local));
        }
    }

    /// Verify that a `Const` src yields no local use.
    #[test]
    fn const_src_has_no_local_use() {
        let const_src = Operand::Constant(crate::Constant::Int(0));
        for kind in [
            InstructionKind::BoxValue {
                dest: lid(10),
                src: const_src.clone(),
                src_type: Type::Int,
            },
            InstructionKind::BoxValue {
                dest: lid(10),
                src: const_src.clone(),
                src_type: Type::Bool,
            },
        ] {
            let is_local = match &kind {
                InstructionKind::BoxValue { src, .. } => matches!(src, Operand::Local(_)),
                _ => unreachable!(),
            };
            assert!(!is_local);
        }
    }
}
