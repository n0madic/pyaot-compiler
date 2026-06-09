//! [`Repr`] — physical representation, and [`repr_of`], the single boundary
//! that maps [`SemTy`] → [`Repr`].
//!
//! This is the layer the MIR verifier and codegen consume. It is **mandatory**:
//! every MIR value has exactly one `Repr`, never `Option<Repr>`. A dual-meaning
//! optional representation sentinel (the trap in PITFALLS A1) cannot recur here
//! by construction.

use pyaot_core_defs::TypeTagKind;
use pyaot_utils::ClassId;

use crate::builtin_classes::{
    BUILTIN_DEQUE_CLASS_ID, BUILTIN_DICT_CLASS_ID, BUILTIN_LIST_CLASS_ID, BUILTIN_SET_CLASS_ID,
    BUILTIN_TUPLE_CLASS_ID, BUILTIN_TUPLE_VAR_CLASS_ID,
};
use crate::sem::{SemTy, Sig};

/// Width/kind of an unboxed primitive held directly in a slot/register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawKind {
    I64,
    F64,
    I8,
    I32,
}

/// Shape of a typed heap pointer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeapShape {
    Str,
    Bytes,
    /// Arbitrary-precision integer object. `int` promotes here on overflow; this
    /// is the representation that makes `int` genuinely unbounded (PITFALLS A6).
    BigInt,
    List(Box<Repr>),
    Dict(Box<Repr>, Box<Repr>),
    Set(Box<Repr>),
    /// Fixed-arity tuple — per-slot representations.
    Tuple(Vec<Repr>),
    /// Variable-length homogeneous tuple — one element representation.
    TupleVar(Box<Repr>),
    /// User-class instance.
    Class(ClassId),
    /// stdlib runtime-backed object (deque, StructTime, File, ...).
    RuntimeObj(TypeTagKind),
    Iterator(Box<Repr>),
}

/// A function-pointer signature at the representation level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigRepr {
    pub params: Vec<Repr>,
    pub ret: Box<Repr>,
}

/// A closure tuple: a function pointer plus captured-environment representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosureShape {
    pub sig: SigRepr,
    pub captures: Vec<Repr>,
}

/// Physical representation of a value. Mandatory and total.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repr {
    /// Unboxed primitive bits in the slot. Chosen only when proven safe.
    Raw(RawKind),
    /// Universal tagged `Value` — the always-correct default substrate. May
    /// carry a fixnum, a bool, `None`, or any heap pointer.
    Tagged,
    /// Typed heap pointer.
    Heap(HeapShape),
    /// Bare code address with a typed signature.
    FuncPtr(Box<SigRepr>),
    /// Closure tuple (code address + captures).
    Closure(Box<ClosureShape>),
    /// Bottom — produced by unreachable code.
    Never,
}

impl Repr {
    /// GC-rootness is *derived*, never a separate flag. A slot must be traced by
    /// the collector iff it can hold a heap pointer: tagged values (which may
    /// carry one), typed heap pointers, and closures.
    pub fn is_gc_root(&self) -> bool {
        matches!(self, Repr::Tagged | Repr::Heap(_) | Repr::Closure(_))
    }
}

/// **The single `SemTy` → `Repr` boundary.** Deterministic and correctness-first.
///
/// Defaulting rule (invariant #2 of the crate): anything whose physical shape is
/// not *unconditionally* safe to unbox maps to [`Repr::Tagged`]. Unboxing
/// (`Raw`/typed `Heap`) is an optimization typeck applies on top of this correct
/// baseline — never a default that can corrupt memory if inference is wrong.
pub fn repr_of(ty: &SemTy) -> Repr {
    match ty {
        // Unconditionally-unboxable primitives.
        SemTy::Bool => Repr::Raw(RawKind::I8),
        SemTy::Float => Repr::Raw(RawKind::F64),

        // `int` is bignum-capable, so the safe default is a tagged value that may
        // hold a fixnum OR a heap BigInt. typeck narrows to Raw(I64) only with a
        // proven non-overflowing range.
        SemTy::Int => Repr::Tagged,

        SemTy::Str => Repr::Heap(HeapShape::Str),
        SemTy::Bytes => Repr::Heap(HeapShape::Bytes),

        // `None` is a tagged singleton.
        SemTy::NoneTy => Repr::Tagged,

        SemTy::Generic { base, args } => repr_of_generic(*base, args),

        SemTy::Class { class_id, .. } => Repr::Heap(HeapShape::Class(*class_id)),
        SemTy::RuntimeObject(tag) => Repr::Heap(HeapShape::RuntimeObj(*tag)),
        SemTy::File { .. } => Repr::Heap(HeapShape::RuntimeObj(TypeTagKind::File)),
        SemTy::Iterator(elem) => Repr::Heap(HeapShape::Iterator(Box::new(repr_of(elem)))),

        SemTy::Callable(sig) => Repr::FuncPtr(Box::new(sig_repr(sig))),

        // Exception instances are heap objects but flow through tagged slots in
        // handler/raise paths; tagged is the correctness-first choice.
        SemTy::BuiltinException(_) => Repr::Tagged,

        // Gradual / unknown / sentinel — always tagged, never ambiguous.
        SemTy::Union(_) | SemTy::Var(_) | SemTy::Dyn | SemTy::NotImplementedT => Repr::Tagged,

        SemTy::Never => Repr::Never,
    }
}

fn repr_of_generic(base: ClassId, args: &[SemTy]) -> Repr {
    let elem = |i: usize| Box::new(args.get(i).map(repr_of).unwrap_or(Repr::Tagged));
    if base == BUILTIN_LIST_CLASS_ID {
        Repr::Heap(HeapShape::List(elem(0)))
    } else if base == BUILTIN_DICT_CLASS_ID {
        Repr::Heap(HeapShape::Dict(elem(0), elem(1)))
    } else if base == BUILTIN_SET_CLASS_ID {
        Repr::Heap(HeapShape::Set(elem(0)))
    } else if base == BUILTIN_TUPLE_CLASS_ID {
        Repr::Heap(HeapShape::Tuple(args.iter().map(repr_of).collect()))
    } else if base == BUILTIN_TUPLE_VAR_CLASS_ID {
        Repr::Heap(HeapShape::TupleVar(elem(0)))
    } else if base == BUILTIN_DEQUE_CLASS_ID {
        // deque is runtime-backed; element type is compile-time only.
        Repr::Heap(HeapShape::RuntimeObj(TypeTagKind::Deque))
    } else {
        // User-defined generic class → nominal instance pointer.
        Repr::Heap(HeapShape::Class(base))
    }
}

fn sig_repr(sig: &Sig) -> SigRepr {
    SigRepr {
        params: sig.params.iter().map(repr_of).collect(),
        ret: Box::new(repr_of(&sig.ret)),
    }
}
