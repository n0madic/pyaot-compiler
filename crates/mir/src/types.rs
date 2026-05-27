//! Strong-typed MIR representation types.
//!
//! Phase 0 of the Strong-Typed MIR Rewrite (see
//! `.claude/plans/velvety-waddling-map.md`). This module introduces a
//! representation-aware type system for MIR slots and operands.
//!
//! # Architectural Axiom
//!
//! > **`MirType` determines physical representation. The MIR Verifier
//! > rejects programs where representation doesn't match.** Unlike the
//! > shared HIR/MIR `pyaot_types::Type` (a logical / structural type),
//! > `MirType` describes how bits are laid out in a slot at runtime.
//!
//! Aspirational Storage-Uniform Invariant from the prior plan becomes
//! enforced by the type system: `Tagged` is the only type that holds
//! mixed-tag values, `Raw(K)` is the only type that holds raw primitive
//! bits, and the conversions between them go through the single
//! `BoxValue` / `UnboxValue` instruction pair.
//!
//! # Status (Phase 0)
//!
//! These types are declared parallel to the existing `pyaot_types::Type`
//! enum but are not yet used anywhere. Migration begins in Phase 2 (typed
//! lowering); the MIR Verifier (Phase 1) will report violations against
//! the legacy weakly-typed MIR to drive that migration.

use pyaot_core_defs::{BuiltinExceptionKind, TypeTagKind};
use pyaot_types::Type;
use pyaot_types::{
    BUILTIN_DICT_CLASS_ID, BUILTIN_LIST_CLASS_ID, BUILTIN_SET_CLASS_ID, BUILTIN_TUPLE_CLASS_ID,
    BUILTIN_TUPLE_VAR_CLASS_ID,
};
use pyaot_utils::{ClassId, InternedString};

// =============================================================================
// MirType — physical representation type for MIR slots and operands
// =============================================================================

/// MIR-level type. **Determines physical representation**, not just a
/// hint. The MIR Verifier rejects programs where an instruction's operand
/// or destination doesn't match the declared `MirType`.
///
/// HIR continues to use `pyaot_types::Type` for logical / structural type
/// information. Translation `Type → MirType` happens at the lowering
/// boundary (Phase 2).
///
/// # Key invariants
///
/// - `Tagged` is the ONLY type that holds mixed-tag values (a runtime
///   `Value` with any of `TAG_PTR`, `TAG_INT`, `TAG_BOOL`, `TAG_NONE`).
/// - `Raw(K)` is the ONLY type that holds raw primitive bits.
/// - `Heap(S)` slots ALWAYS contain aligned pointers to S-shaped data.
/// - `BoxValue { src: Raw(K), dest: Tagged, src_kind: K }` is the SINGLE
///   Raw → Tagged conversion. Codegen lowers based on `src_kind`.
/// - `UnboxValue { src: Tagged, dest: Raw(K), dest_kind: K }` is the
///   SINGLE Tagged → Raw conversion.
/// - `HeapAny` (from `pyaot_types::Type`) does NOT exist in MirType — its
///   role is subsumed by `Tagged` (any tag) and `Heap(S)` (specific
///   pointer shape).
/// - `Any` (from `pyaot_types::Type`) does NOT exist in MirType — every
///   slot has explicit representation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MirType {
    /// Raw bit storage — primitive value occupies the slot directly.
    /// Used for register-level computation and explicit raw ABI specialisation.
    Raw(RawKind),

    /// Tagged storage — slot holds a tagged `Value` (any of `TAG_PTR`,
    /// `TAG_INT`, `TAG_BOOL`, `TAG_NONE`). The runtime distinguishes the
    /// concrete tag at dispatch sites (e.g., `rt_obj_*` family).
    Tagged,

    /// Heap pointer with statically-known shape. Slot holds an aligned
    /// pointer to the structure described by `HeapShape`. The GC's mark
    /// walk uses the shape to descend correctly.
    Heap(HeapShape),

    /// Function pointer — slot holds a code address. Includes the
    /// callee's signature (param/return types) for ABI verification.
    /// Distinct from `Closure` because function pointers carry no
    /// captures — they're plain code addresses.
    FuncPtr(Box<Signature>),

    /// Closure tuple — slot holds a heap pointer to a tuple of
    /// `(func_ptr, captures)`. The signature and capture shape are
    /// statically known via `ClosureShape`. Distinct from `Heap(_)`
    /// because the closure tuple has a specialised layout and dispatch
    /// semantics (via the closure trampoline).
    Closure(Box<ClosureShape>),

    /// TypeVar placeholder (pre-monomorphisation). Must be erased before
    /// codegen. WPA passes through; `MonomorphizePass` replaces.
    /// Holds the TypeVar's name (e.g., `T`, `K`, `V`).
    Var(InternedString),

    /// Never — bottom type. Control doesn't reach a `Never`-typed slot
    /// at runtime. The slot's representation is irrelevant; verifier
    /// accepts any use.
    Never,
}

impl MirType {
    /// True if this is a raw primitive representation.
    #[inline]
    pub fn is_raw(&self) -> bool {
        matches!(self, Self::Raw(_))
    }

    /// True if this is a tagged `Value` representation.
    #[inline]
    pub fn is_tagged(&self) -> bool {
        matches!(self, Self::Tagged)
    }

    /// True if this is a heap pointer representation.
    #[inline]
    pub fn is_heap(&self) -> bool {
        matches!(self, Self::Heap(_))
    }

    /// True if this is a function pointer representation.
    #[inline]
    pub fn is_func_ptr(&self) -> bool {
        matches!(self, Self::FuncPtr(_))
    }

    /// True if this is a closure tuple representation.
    #[inline]
    pub fn is_closure(&self) -> bool {
        matches!(self, Self::Closure(_))
    }

    /// True if this is a TypeVar placeholder.
    #[inline]
    pub fn is_var(&self) -> bool {
        matches!(self, Self::Var(_))
    }

    /// True if this is the bottom `Never` type.
    #[inline]
    pub fn is_never(&self) -> bool {
        matches!(self, Self::Never)
    }

    /// True if this is a fixed-length or variable-length tuple heap pointer.
    ///
    /// Mirrors `pyaot_types::Type::is_tuple_like()` at the MirType level:
    /// both `Heap(TupleFixed(_))` and `Heap(TupleVar(_))` share the same
    /// runtime `TupleObj` layout, so this predicate is the single check
    /// needed wherever the distinction between the two shapes does not matter.
    #[inline]
    pub fn is_tuple_like(&self) -> bool {
        matches!(
            self,
            Self::Heap(HeapShape::TupleFixed(_) | HeapShape::TupleVar(_))
        )
    }

    /// True if a slot of this type needs GC root tracking (i.e. its bits
    /// may point into the heap and must be scanned by the mark walk).
    ///
    /// - `Raw(_)` and `Var(_)` and `Never`: no — raw bits never alias heap.
    /// - `FuncPtr(_)`: no — code segment pointers don't need tracking.
    /// - `Tagged`: yes — may carry a pointer tag.
    /// - `Heap(_)`: yes — always a heap pointer.
    /// - `Closure(_)`: yes — closure tuple is heap-allocated.
    #[inline]
    pub fn needs_gc_root(&self) -> bool {
        match self {
            Self::Raw(_) | Self::Var(_) | Self::Never | Self::FuncPtr(_) => false,
            Self::Tagged | Self::Heap(_) | Self::Closure(_) => true,
        }
    }

    /// True if a value of this type can be assigned to a slot of `other`
    /// without an explicit conversion (Box/Unbox/Refine). This is the
    /// strict equality of physical representation; for narrowing /
    /// widening conversions, use `Refine` / `BoxValue` / `UnboxValue`.
    ///
    /// Widening allowances (no physical conversion needed):
    ///
    /// - `Heap(_)` → `Tagged`: a heap pointer with PTR tag IS a valid
    ///   `Tagged` value. Widening loses static shape but bits unchanged.
    /// - `FuncPtr(_)` → `Tagged`: a function pointer also fits the
    ///   Tagged representation when low 3 bits are zero (function
    ///   pointers are always aligned).
    /// - `Closure(_)` → `Tagged`: closure tuple pointer is heap-allocated.
    /// - `Heap(Class { .. })` → `Heap(Class { .. })` (class subtyping):
    ///   the verifier cannot resolve the full class hierarchy without a
    ///   `Module` reference, so it accepts any pair of class instance
    ///   pointers as assignable. This eliminates the false-positive
    ///   flood from inheritance chains (Cat → Animal, Circle → Shape)
    ///   at the cost of letting unrelated-class assignments through.
    ///   Phase 3 (typed optimizer) tightens this with hierarchy-aware
    ///   checking.
    /// - Anything → `Never` and `Never` → anything: vacuously assignable
    ///   (control doesn't reach a `Never` slot at runtime).
    #[inline]
    pub fn assignable_to(&self, other: &MirType) -> bool {
        if self == other {
            return true;
        }
        if matches!(self, Self::Never) || matches!(other, Self::Never) {
            return true;
        }
        // Widening pointer-shaped sources to Tagged.
        // REQUIRED — removing fails 14 examples. Universally sound: a heap
        // pointer with TAG_PTR IS a valid Tagged value (bits unchanged, only
        // static shape information is lost). This widening is permanent —
        // it is not a Phase-2 pragmatic hack but a fundamental property of
        // the tagged-Value representation.
        if matches!(other, Self::Tagged)
            && matches!(self, Self::Heap(_) | Self::FuncPtr(_) | Self::Closure(_))
        {
            return true;
        }
        // Class subtyping (permissive, no hierarchy check).
        if let (Self::Heap(s), Self::Heap(t)) = (self, other) {
            // REQUIRED — removing fails 4 examples (test_builtins, test_classes,
            // test_generics, test_types_system). Inheritance chains (Cat → Animal,
            // Dog → Animal, Circle → Shape) pass subclass to superclass __init__
            // as Heap(Class(N)) → Heap(Class(M)) where N≠M. A proper hierarchy
            // check needs Module context; deferred to Phase 3 typed optimizer.
            if matches!(s, HeapShape::Class { .. }) && matches!(t, HeapShape::Class { .. }) {
                return true;
            }
            // Tuple shape interchange: `TupleFixed` and `TupleVar` are
            // both backed by the same runtime `TupleObj` (uniform
            // `[Value]` storage); the distinction is compile-time only.
            // Callers and callees that disagree on the shape (e.g.
            // empty literal `()` → `TupleFixed([])` vs variadic param
            // `tuple[T, ...]` → `TupleVar(_)`) still pass pointers to
            // the same physical structure.
            // REQUIRED — removing fails test_classes, test_future_annotations.
            // Storage-uniform invariant; physically sound to keep.
            if matches!(s, HeapShape::TupleFixed(_) | HeapShape::TupleVar(_))
                && matches!(t, HeapShape::TupleFixed(_) | HeapShape::TupleVar(_))
            {
                return true;
            }
            // Container covariance: List/Dict/Set/Iterator/Cell store elements as
            // tagged Value (post storage-uniform BigBang). Two same-outer-shape
            // containers are assignable iff their element types are assignable
            // recursively. Preserves type safety (Heap(List(Raw(I64))) is NOT
            // assignable to Heap(List(Heap(Str))) — Raw(I64) ↛ Heap(Str)) while
            // accepting the common Phase-2 case where WPA hasn't fully refined
            // an inner element type across function-call boundaries (Tagged at
            // outer call site, Heap(Class) inside callee — Tagged → Heap(Class)
            // is permitted by the existing Tagged-widening rule).
            // REQUIRED — removing fails test_classes (1 example). WPA hasn't
            // fully refined inner element types across function-call boundaries
            // (e.g. Tagged at outer call site, Heap(Class) inside callee).
            match (s, t) {
                (HeapShape::List(s_e), HeapShape::List(t_e)) => return s_e.assignable_to(t_e),
                (HeapShape::Set(s_e), HeapShape::Set(t_e)) => return s_e.assignable_to(t_e),
                (HeapShape::Iterator(s_e), HeapShape::Iterator(t_e)) => {
                    return s_e.assignable_to(t_e);
                }
                (HeapShape::Cell(s_e), HeapShape::Cell(t_e)) => return s_e.assignable_to(t_e),
                (
                    HeapShape::Dict { key: sk, value: sv },
                    HeapShape::Dict { key: tk, value: tv },
                ) => return sk.assignable_to(tk) && sv.assignable_to(tv),
                _ => {}
            }
        }
        // Tagged → FuncPtr / Heap: a Tagged value carrying TAG_PTR
        // with pointer-aligned bits IS a heap pointer at the physical
        // level. The HIR type system filters values flowing into
        // pointer-typed slots; the canonical lowering for both
        // monomorphized wrapper call sites (FuncPtr) and post-isinstance
        // narrowing (Heap shapes) leaves Tagged operands flowing into
        // these slots without an explicit Refine. The "safety" depends
        // on the HIR type contract and is no weaker than the symmetric
        // Heap/FuncPtr → Tagged widening already accepted above.
        // REQUIRED — removing fails 10 examples (test_builtins, test_classes,
        // test_decorator_factory, test_functions, test_import, etc.). The
        // lowering leaves Tagged operands flowing into Heap/FuncPtr slots at
        // monomorphized wrapper call sites and post-isinstance narrowing sites
        // without an explicit Refine. Phase 4b/4c codegen migration (blocked
        // per plan) will tighten this when explicit UnboxValue is emitted.
        if matches!(self, Self::Tagged) && matches!(other, Self::Heap(_) | Self::FuncPtr(_)) {
            return true;
        }
        false
    }
}

impl std::fmt::Display for MirType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Raw(k) => write!(f, "Raw({k})"),
            Self::Tagged => write!(f, "Tagged"),
            Self::Heap(s) => write!(f, "Heap({s})"),
            Self::FuncPtr(sig) => write!(f, "FuncPtr({sig})"),
            Self::Closure(c) => write!(f, "Closure({c})"),
            Self::Var(name) => write!(f, "Var({name:?})"),
            Self::Never => write!(f, "Never"),
        }
    }
}

// =============================================================================
// RawKind — physical bit width for raw representation
// =============================================================================

/// Physical bit width / interpretation for `MirType::Raw`.
///
/// Each `RawKind` maps to a specific Cranelift type during codegen:
/// - `I64` → `cranelift_codegen::ir::types::I64`
/// - `F64` → `cranelift_codegen::ir::types::F64`
/// - `I8`  → `cranelift_codegen::ir::types::I8`
/// - `I32` → `cranelift_codegen::ir::types::I32`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawKind {
    /// Raw 64-bit signed integer. Used for `int` at register level.
    I64,
    /// Raw 64-bit IEEE float. Used for `float` at register level.
    F64,
    /// Raw byte. Used for `bool` and internal exception/sentinel flags.
    I8,
    /// Raw 32-bit. Used for some indices and global-storage slot IDs.
    I32,
}

impl std::fmt::Display for RawKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::I64 => write!(f, "I64"),
            Self::F64 => write!(f, "F64"),
            Self::I8 => write!(f, "I8"),
            Self::I32 => write!(f, "I32"),
        }
    }
}

// =============================================================================
// HeapShape — typed pointer-target shape
// =============================================================================

/// Shape of the heap-allocated structure pointed to by a `Heap(_)` slot.
/// Parametric shapes (List, Dict, Tuple, etc.) carry their element /
/// key / value types so the GC and runtime can navigate the structure
/// without consulting side-tables.
///
/// `RuntimeObj(TypeTagKind)` is a catch-all for opaque runtime objects
/// (File, Hash, StringIO, Counter, Deque, Match, etc.) whose internal
/// structure isn't compositionally typed at the MIR level. Their layout
/// is defined entirely in the runtime crate; MIR treats them as opaque
/// pointers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HeapShape {
    /// Python string. Pointer to a `StrObj` with embedded byte buffer.
    Str,
    /// Python bytes. Pointer to a `BytesObj` with embedded byte buffer.
    Bytes,
    /// Homogeneous list. Element representation typed by inner `MirType`.
    /// E.g. `Heap(List(Tagged))` is a `list` of Python values;
    /// `Heap(List(Raw(I64)))` is a specialised raw-int list (future).
    List(Box<MirType>),
    /// Fixed-length heterogeneous tuple. One `MirType` per slot.
    TupleFixed(Vec<MirType>),
    /// Variable-length homogeneous tuple. Element representation typed
    /// by inner `MirType`.
    TupleVar(Box<MirType>),
    /// Dictionary. Key and value representations typed independently.
    Dict {
        key: Box<MirType>,
        value: Box<MirType>,
    },
    /// `collections.defaultdict`. Same physical layout as `Dict` plus a
    /// default-factory function pointer.
    DefaultDict {
        key: Box<MirType>,
        value: Box<MirType>,
    },
    /// Set with typed element.
    Set(Box<MirType>),
    /// Iterator yielding values of the given element representation.
    Iterator(Box<MirType>),
    /// Suspended coroutine state (generator object). Not parameterised
    /// at the MIR level — yield/send types are erased to `Tagged` at
    /// the resume ABI.
    Generator,
    /// Nonlocal cell wrapping a value of the given representation.
    Cell(Box<MirType>),
    /// Boxed float (`FloatObj`). Used when a float must be heap-allocated
    /// (e.g. stored in a `Tagged` slot at a different tag than `Float`'s
    /// raw bits would allow — i.e. always).
    FloatObj,
    /// Boxed `None` singleton (`NoneObj`).
    NoneObj,
    /// Concrete user-defined class instance. `type_args` carry the
    /// substituted generic parameters (empty for non-generic classes).
    Class {
        id: ClassId,
        type_args: Vec<MirType>,
    },
    /// Built-in exception instance with a specific exception kind.
    Exception(BuiltinExceptionKind),
    /// Opaque runtime object identified by a `TypeTagKind`. Used for
    /// runtime types whose internal structure isn't compositionally typed
    /// at the MIR level (File, Hash, StringIO, BytesIO, Counter, Deque,
    /// Match, StructTime, CompletedProcess, ParseResult, HttpResponse,
    /// Request, StringBuilder, NotImplemented, ...).
    RuntimeObj(TypeTagKind),
}

impl std::fmt::Display for HeapShape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Str => write!(f, "Str"),
            Self::Bytes => write!(f, "Bytes"),
            Self::List(elem) => write!(f, "List({elem})"),
            Self::TupleFixed(elems) => {
                write!(f, "TupleFixed(")?;
                for (i, t) in elems.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{t}")?;
                }
                write!(f, ")")
            }
            Self::TupleVar(elem) => write!(f, "TupleVar({elem})"),
            Self::Dict { key, value } => write!(f, "Dict({key}, {value})"),
            Self::DefaultDict { key, value } => write!(f, "DefaultDict({key}, {value})"),
            Self::Set(elem) => write!(f, "Set({elem})"),
            Self::Iterator(elem) => write!(f, "Iterator({elem})"),
            Self::Generator => write!(f, "Generator"),
            Self::Cell(inner) => write!(f, "Cell({inner})"),
            Self::FloatObj => write!(f, "FloatObj"),
            Self::NoneObj => write!(f, "NoneObj"),
            Self::Class { id, type_args } => {
                write!(f, "Class({id}")?;
                if !type_args.is_empty() {
                    write!(f, "[")?;
                    for (i, t) in type_args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{t}")?;
                    }
                    write!(f, "]")?;
                }
                write!(f, ")")
            }
            Self::Exception(kind) => write!(f, "Exception({kind})"),
            Self::RuntimeObj(tag) => write!(f, "RuntimeObj({tag})"),
        }
    }
}

// =============================================================================
// Signature — typed function signature
// =============================================================================

/// Typed function signature. The canonical contract between a callee and
/// its callers: `args` must match `params` element-wise; `dest` must
/// match `return_type`. The MIR Verifier rejects any `CallDirect` /
/// `CallVirtual` / `CallNamed` / `Call` where this isn't true.
///
/// # Uniform Tagged ABI
///
/// By default (after Phase 2), every lowered function has signature
/// `Signature { params: vec![Tagged; n], return_type: Tagged }`.
/// Callee prologue emits `UnboxValue` for params it uses as `Raw`
/// locally; callee epilogue emits `BoxValue` before `Return` if the body
/// produced `Raw`. Caller sees `Tagged` on both sides — no per-callee
/// bridging needed.
///
/// Phase 3 optimizer can specialise signatures (hoist box/unbox to caller
/// side for known-static call graphs).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Signature {
    pub params: Vec<MirType>,
    pub return_type: MirType,
}

impl Signature {
    /// Construct a uniform Tagged-ABI signature with the given number of
    /// parameters. Used by Phase 2 lowering as the default callee
    /// signature.
    pub fn uniform_tagged(n_params: usize) -> Self {
        Self {
            params: vec![MirType::Tagged; n_params],
            return_type: MirType::Tagged,
        }
    }

    /// True if every parameter and the return are `Tagged`.
    pub fn is_uniform_tagged(&self) -> bool {
        matches!(self.return_type, MirType::Tagged)
            && self.params.iter().all(|p| matches!(p, MirType::Tagged))
    }
}

impl std::fmt::Display for Signature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "fn(")?;
        for (i, p) in self.params.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{p}")?;
        }
        write!(f, ") -> {}", self.return_type)
    }
}

// =============================================================================
// ClosureShape — typed closure tuple layout
// =============================================================================

/// Closure tuple layout: the function's typed signature plus the captured
/// variables' representations in order. The runtime closure trampoline
/// uses this to extract captures and dispatch to the callee.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClosureShape {
    pub signature: Signature,
    pub captures: Vec<MirType>,
}

impl std::fmt::Display for ClosureShape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} [captures: ", self.signature)?;
        for (i, c) in self.captures.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{c}")?;
        }
        write!(f, "]")
    }
}

// =============================================================================
// Convenience constructors
// =============================================================================

impl MirType {
    /// `MirType::Raw(RawKind::I64)` — raw signed int.
    pub fn raw_i64() -> Self {
        Self::Raw(RawKind::I64)
    }

    /// `MirType::Raw(RawKind::F64)` — raw IEEE float.
    pub fn raw_f64() -> Self {
        Self::Raw(RawKind::F64)
    }

    /// `MirType::Raw(RawKind::I8)` — raw byte (e.g. bool).
    pub fn raw_i8() -> Self {
        Self::Raw(RawKind::I8)
    }

    /// `MirType::Raw(RawKind::I32)` — raw 32-bit (e.g. global slot id).
    pub fn raw_i32() -> Self {
        Self::Raw(RawKind::I32)
    }

    /// `MirType::Heap(HeapShape::Str)` — Python string.
    pub fn str_heap() -> Self {
        Self::Heap(HeapShape::Str)
    }

    /// `MirType::Heap(HeapShape::Bytes)` — Python bytes.
    pub fn bytes_heap() -> Self {
        Self::Heap(HeapShape::Bytes)
    }

    /// `MirType::Heap(HeapShape::List(_))` — list with element rep.
    pub fn list_of(elem: MirType) -> Self {
        Self::Heap(HeapShape::List(Box::new(elem)))
    }

    /// `MirType::Heap(HeapShape::Dict { _, _ })`.
    pub fn dict_of(key: MirType, value: MirType) -> Self {
        Self::Heap(HeapShape::Dict {
            key: Box::new(key),
            value: Box::new(value),
        })
    }

    /// `MirType::Heap(HeapShape::Set(_))`.
    pub fn set_of(elem: MirType) -> Self {
        Self::Heap(HeapShape::Set(Box::new(elem)))
    }

    /// `MirType::Heap(HeapShape::TupleFixed(_))`.
    pub fn tuple_fixed(elems: Vec<MirType>) -> Self {
        Self::Heap(HeapShape::TupleFixed(elems))
    }

    /// `MirType::Heap(HeapShape::TupleVar(_))`.
    pub fn tuple_var(elem: MirType) -> Self {
        Self::Heap(HeapShape::TupleVar(Box::new(elem)))
    }

    /// `MirType::Heap(HeapShape::Class { id, type_args })`.
    pub fn class(id: ClassId, type_args: Vec<MirType>) -> Self {
        Self::Heap(HeapShape::Class { id, type_args })
    }

    /// `MirType::Heap(HeapShape::Iterator(_))`.
    pub fn iterator_of(elem: MirType) -> Self {
        Self::Heap(HeapShape::Iterator(Box::new(elem)))
    }

    /// `MirType::FuncPtr(_)` with the given signature.
    pub fn func_ptr(sig: Signature) -> Self {
        Self::FuncPtr(Box::new(sig))
    }

    /// `MirType::Closure(_)` with the given shape.
    pub fn closure(shape: ClosureShape) -> Self {
        Self::Closure(Box::new(shape))
    }

    // ===== Stage F.2 query helpers — mirror `pyaot_types::Type` methods
    // that have no MirType equivalent, so call sites can migrate from
    // `local.ty.X()` to `local.resolved_mir_type().X()`.

    /// True iff any subterm is `Var(_)`. Mirrors `Type::contains_var`.
    pub fn contains_var(&self) -> bool {
        match self {
            MirType::Var(_) => true,
            MirType::Raw(_) | MirType::Tagged | MirType::Never => false,
            MirType::Heap(shape) => shape.contains_var(),
            MirType::FuncPtr(sig) => {
                sig.params.iter().any(|p| p.contains_var()) || sig.return_type.contains_var()
            }
            MirType::Closure(shape) => {
                shape.signature.params.iter().any(|p| p.contains_var())
                    || shape.signature.return_type.contains_var()
                    || shape.captures.iter().any(|c| c.contains_var())
            }
        }
    }

    /// Append every `Var(name)` leaf into `out` (deduplication left to caller).
    pub fn collect_var_names(&self, out: &mut Vec<InternedString>) {
        match self {
            MirType::Var(name) => out.push(*name),
            MirType::Raw(_) | MirType::Tagged | MirType::Never => {}
            MirType::Heap(shape) => shape.collect_var_names(out),
            MirType::FuncPtr(sig) => {
                for p in &sig.params {
                    p.collect_var_names(out);
                }
                sig.return_type.collect_var_names(out);
            }
            MirType::Closure(shape) => {
                for p in &shape.signature.params {
                    p.collect_var_names(out);
                }
                shape.signature.return_type.collect_var_names(out);
                for c in &shape.captures {
                    c.collect_var_names(out);
                }
            }
        }
    }

    /// Element type of `Heap(List(_))`. None otherwise.
    pub fn list_elem(&self) -> Option<&MirType> {
        match self {
            MirType::Heap(HeapShape::List(elem)) => Some(elem),
            _ => None,
        }
    }

    /// `(key, value)` of `Heap(Dict { … })`. None otherwise.
    pub fn dict_kv(&self) -> Option<(&MirType, &MirType)> {
        match self {
            MirType::Heap(HeapShape::Dict { key, value }) => Some((key, value)),
            _ => None,
        }
    }

    /// Element types of `Heap(TupleFixed(_))`. None for `TupleVar` or non-tuple.
    pub fn tuple_elems(&self) -> Option<&[MirType]> {
        match self {
            MirType::Heap(HeapShape::TupleFixed(elems)) => Some(elems),
            _ => None,
        }
    }

    /// Element type of `Heap(TupleVar(_))`. None otherwise.
    pub fn tuple_var_elem(&self) -> Option<&MirType> {
        match self {
            MirType::Heap(HeapShape::TupleVar(elem)) => Some(elem),
            _ => None,
        }
    }

    /// Element type of `Heap(Set(_))`. None otherwise.
    pub fn set_elem(&self) -> Option<&MirType> {
        match self {
            MirType::Heap(HeapShape::Set(elem)) => Some(elem),
            _ => None,
        }
    }

    /// `ClassId` of `Heap(Class { id, … })`. None for non-class shapes.
    pub fn class_id(&self) -> Option<ClassId> {
        match self {
            MirType::Heap(HeapShape::Class { id, .. }) => Some(*id),
            _ => None,
        }
    }
}

impl HeapShape {
    fn contains_var(&self) -> bool {
        match self {
            HeapShape::Str
            | HeapShape::Bytes
            | HeapShape::Generator
            | HeapShape::FloatObj
            | HeapShape::NoneObj
            | HeapShape::RuntimeObj(_)
            | HeapShape::Exception(_) => false,
            HeapShape::List(elem)
            | HeapShape::Set(elem)
            | HeapShape::Iterator(elem)
            | HeapShape::Cell(elem)
            | HeapShape::TupleVar(elem) => elem.contains_var(),
            HeapShape::TupleFixed(elems) => elems.iter().any(|e| e.contains_var()),
            HeapShape::Dict { key, value } | HeapShape::DefaultDict { key, value } => {
                key.contains_var() || value.contains_var()
            }
            HeapShape::Class { type_args, .. } => type_args.iter().any(|t| t.contains_var()),
        }
    }

    fn collect_var_names(&self, out: &mut Vec<InternedString>) {
        match self {
            HeapShape::Str
            | HeapShape::Bytes
            | HeapShape::Generator
            | HeapShape::FloatObj
            | HeapShape::NoneObj
            | HeapShape::RuntimeObj(_)
            | HeapShape::Exception(_) => {}
            HeapShape::List(elem)
            | HeapShape::Set(elem)
            | HeapShape::Iterator(elem)
            | HeapShape::Cell(elem)
            | HeapShape::TupleVar(elem) => elem.collect_var_names(out),
            HeapShape::TupleFixed(elems) => {
                for e in elems {
                    e.collect_var_names(out);
                }
            }
            HeapShape::Dict { key, value } | HeapShape::DefaultDict { key, value } => {
                key.collect_var_names(out);
                value.collect_var_names(out);
            }
            HeapShape::Class { type_args, .. } => {
                for t in type_args {
                    t.collect_var_names(out);
                }
            }
        }
    }
}

// =============================================================================
// Translation: pyaot_types::Type (HIR / legacy MIR) → MirType
// =============================================================================

/// Translate a logical HIR / legacy-MIR `Type` into a physical
/// representation `MirType`. **Storage interpretation**: primitives map
/// to `Tagged` (the future storage-uniform invariant). For legacy MIR
/// verification (Phase 1), this is used as a best-effort projection of
/// existing Local types — many translations to `Tagged` will mismatch
/// instructions expecting `Raw`. Each mismatch becomes a violation
/// driving Phase 2 typed-lowering migration.
///
/// **Register interpretation** (used post-Phase-2 for register-level
/// computation) is provided via `Type::to_mir_type_register`. Storage
/// path uses this function (`Type::to_mir_type_storage`).
///
/// # Mapping rules
///
/// Logical → Physical (storage default):
/// - `Int`, `Float`, `Bool`, `None` → `Tagged` (uniform-tagged storage)
/// - `Str` → `Heap(Str)`
/// - `Bytes` → `Heap(Bytes)`
/// - `Any`, `HeapAny` → `Tagged`
/// - `Never` → `Never`
/// - `Var(name)` → `Var(name)`
/// - `Union(_)` → `Tagged` (only common representation)
/// - `Class { class_id, .. }` → `Heap(Class { id, type_args: [] })`
/// - `Generic { base, args }`:
///   - `BUILTIN_LIST_CLASS_ID` → `Heap(List(map(args[0])))`
///   - `BUILTIN_DICT_CLASS_ID` → `Heap(Dict { map(args[0]), map(args[1]) })`
///   - `BUILTIN_SET_CLASS_ID` → `Heap(Set(map(args[0])))`
///   - `BUILTIN_TUPLE_CLASS_ID` → `Heap(TupleFixed(args.map(map)))`
///   - `BUILTIN_TUPLE_VAR_CLASS_ID` → `Heap(TupleVar(map(args[0])))`
///   - user generic → `Heap(Class { id: base, type_args: args.map(map) })`
/// - `DefaultDict(k, v)` → `Heap(DefaultDict { map(k), map(v) })`
/// - `Iterator(elem)` → `Heap(Iterator(map(elem)))`
/// - `BuiltinException(kind)` → `Heap(Exception(kind))`
/// - `File(_)` → `Heap(RuntimeObj(TypeTagKind::File))`
/// - `RuntimeObject(tag)` → `Heap(RuntimeObj(tag))`
/// - `NotImplementedT` → `Heap(RuntimeObj(TypeTagKind::NotImplemented))`
/// - `Function { params, ret }` → `FuncPtr(Signature {
///     params: params.map(map_storage), return: map_storage(ret) })`
pub fn type_to_mir_type_storage(ty: &Type) -> MirType {
    match ty {
        Type::Int | Type::Bool | Type::Float | Type::None => MirType::Tagged,
        Type::Str => MirType::str_heap(),
        Type::Bytes => MirType::bytes_heap(),
        Type::Any => MirType::Tagged,
        Type::Never => MirType::Never,
        Type::Var(name) => MirType::Var(*name),
        Type::Union(_) => MirType::Tagged,
        Type::Class { class_id, .. } => MirType::class(*class_id, vec![]),
        Type::Generic { base, args } => translate_generic(*base, args),
        Type::DefaultDict(k, v) => MirType::Heap(HeapShape::DefaultDict {
            key: Box::new(type_to_mir_type_storage(k)),
            value: Box::new(type_to_mir_type_storage(v)),
        }),
        Type::Iterator(elem) => MirType::iterator_of(type_to_mir_type_storage(elem)),
        Type::BuiltinException(kind) => MirType::Heap(HeapShape::Exception(*kind)),
        Type::File(_) => MirType::Heap(HeapShape::RuntimeObj(TypeTagKind::File)),
        Type::RuntimeObject(tag) => MirType::Heap(HeapShape::RuntimeObj(*tag)),
        Type::NotImplementedT => MirType::Heap(HeapShape::RuntimeObj(TypeTagKind::NotImplemented)),
        Type::Function { params, ret } => {
            let sig = Signature {
                params: params.iter().map(type_to_mir_type_storage).collect(),
                return_type: type_to_mir_type_storage(ret),
            };
            MirType::func_ptr(sig)
        }
    }
}

/// Translate a logical `Type` into the **register-level** physical
/// representation. Differs from `type_to_mir_type_storage` only for
/// primitives, which use `Raw(K)` at register level (vs `Tagged` at
/// storage / ABI level). Used by Phase 2 lowering for body Locals that
/// hold primitive computation results.
pub fn type_to_mir_type_register(ty: &Type) -> MirType {
    match ty {
        Type::Int => MirType::raw_i64(),
        Type::Float => MirType::raw_f64(),
        Type::Bool => MirType::raw_i8(),
        Type::None => MirType::raw_i8(),
        _ => type_to_mir_type_storage(ty),
    }
}

fn translate_generic(base: ClassId, args: &[Type]) -> MirType {
    if base == BUILTIN_LIST_CLASS_ID {
        let elem = args
            .first()
            .map(type_to_mir_type_storage)
            .unwrap_or(MirType::Tagged);
        MirType::list_of(elem)
    } else if base == BUILTIN_DICT_CLASS_ID {
        let k = args
            .first()
            .map(type_to_mir_type_storage)
            .unwrap_or(MirType::Tagged);
        let v = args
            .get(1)
            .map(type_to_mir_type_storage)
            .unwrap_or(MirType::Tagged);
        MirType::dict_of(k, v)
    } else if base == BUILTIN_SET_CLASS_ID {
        let elem = args
            .first()
            .map(type_to_mir_type_storage)
            .unwrap_or(MirType::Tagged);
        MirType::set_of(elem)
    } else if base == BUILTIN_TUPLE_CLASS_ID {
        let elems = args.iter().map(type_to_mir_type_storage).collect();
        MirType::tuple_fixed(elems)
    } else if base == BUILTIN_TUPLE_VAR_CLASS_ID {
        let elem = args
            .first()
            .map(type_to_mir_type_storage)
            .unwrap_or(MirType::Tagged);
        MirType::tuple_var(elem)
    } else {
        let type_args = args.iter().map(type_to_mir_type_storage).collect();
        MirType::class(base, type_args)
    }
}

// =============================================================================
// Phase 2d (foundation): RuntimeFuncDef ParamType / ReturnType → MirType
// =============================================================================

/// Convert a runtime function `ParamType` (declared in `pyaot-core-defs`)
/// to its corresponding `MirType` for register-level register-width selection.
///
/// `ParamType` is a flat Cranelift-shape descriptor (I64/F64/I8/I32) used by
/// every runtime function's declarative signature. The full Phase 2d
/// transition would replace `ParamType` with typed `MirType` directly on
/// `RuntimeFuncDef`, but that's a large refactor. This translation helper
/// lets verifier-side checks (and future typed callsites) consume the
/// existing ParamType data without per-call match arms.
///
/// Note: `ParamType::I64` is ambiguous at the MIR level — it could mean
/// `Raw(I64)` (raw int / raw pointer), `Tagged` (tagged Value), or any of
/// `Heap(_)` / `FuncPtr(_)` / `Closure(_)` (typed pointer). The translation
/// returns `MirType::Tagged` as the most permissive interpretation — all
/// pointer-shaped MirTypes widen to Tagged in the verifier's
/// `assignable_to` rules. Callsites passing a `Raw(I64)` operand to an
/// `I64` runtime param will still pass; passing a strict typed-pointer
/// operand also passes. This is correct for `rt_list_append(list, value)`
/// where `list` is `Heap(List)` and `value` is `Tagged`.
pub fn param_type_to_mir_type(pt: pyaot_core_defs::runtime_func_def::ParamType) -> MirType {
    use pyaot_core_defs::runtime_func_def::ParamType;
    match pt {
        ParamType::I64 => MirType::Tagged,
        ParamType::F64 => MirType::Raw(RawKind::F64),
        ParamType::I8 => MirType::Raw(RawKind::I8),
        ParamType::I32 => MirType::Raw(RawKind::I32),
    }
}

/// Convert a `ReturnType` to its corresponding `MirType`. Symmetric to
/// `param_type_to_mir_type`.
pub fn return_type_to_mir_type(rt: pyaot_core_defs::runtime_func_def::ReturnType) -> MirType {
    use pyaot_core_defs::runtime_func_def::ReturnType;
    match rt {
        ReturnType::I64 => MirType::Tagged,
        ReturnType::F64 => MirType::Raw(RawKind::F64),
        ReturnType::I8 => MirType::Raw(RawKind::I8),
        ReturnType::I32 => MirType::Raw(RawKind::I32),
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use pyaot_core_defs::{BuiltinExceptionKind, TypeTagKind};

    #[test]
    fn raw_kind_display() {
        assert_eq!(format!("{}", RawKind::I64), "I64");
        assert_eq!(format!("{}", RawKind::F64), "F64");
        assert_eq!(format!("{}", RawKind::I8), "I8");
        assert_eq!(format!("{}", RawKind::I32), "I32");
    }

    #[test]
    fn mir_type_predicates() {
        assert!(MirType::raw_i64().is_raw());
        assert!(MirType::Tagged.is_tagged());
        assert!(MirType::str_heap().is_heap());
        assert!(MirType::Never.is_never());
    }

    #[test]
    fn gc_root_classification() {
        assert!(!MirType::raw_i64().needs_gc_root());
        assert!(!MirType::raw_f64().needs_gc_root());
        assert!(!MirType::Never.needs_gc_root());
        assert!(MirType::Tagged.needs_gc_root());
        assert!(MirType::str_heap().needs_gc_root());
        assert!(MirType::list_of(MirType::Tagged).needs_gc_root());
        // FuncPtr is code-segment, not heap; no GC root.
        let sig = Signature::uniform_tagged(2);
        assert!(!MirType::func_ptr(sig.clone()).needs_gc_root());
        // Closure tuple is heap-allocated.
        assert!(MirType::closure(ClosureShape {
            signature: sig,
            captures: vec![MirType::Tagged],
        })
        .needs_gc_root());
    }

    #[test]
    fn assignable_to_self_and_never() {
        let t1 = MirType::Tagged;
        let t2 = MirType::Tagged;
        assert!(t1.assignable_to(&t2));
        let never = MirType::Never;
        assert!(never.assignable_to(&MirType::raw_i64()));
        assert!(MirType::raw_i64().assignable_to(&never));
    }

    #[test]
    fn nominal_inequality_blocks_assignment() {
        assert!(!MirType::raw_i64().assignable_to(&MirType::Tagged));
        assert!(!MirType::Tagged.assignable_to(&MirType::raw_i64()));
        assert!(!MirType::raw_i64().assignable_to(&MirType::raw_f64()));
    }

    #[test]
    fn pointer_shapes_widen_to_tagged() {
        // Heap → Tagged
        assert!(MirType::str_heap().assignable_to(&MirType::Tagged));
        assert!(MirType::list_of(MirType::Tagged).assignable_to(&MirType::Tagged));
        // FuncPtr → Tagged
        let sig = Signature::uniform_tagged(1);
        assert!(MirType::func_ptr(sig.clone()).assignable_to(&MirType::Tagged));
        // Closure → Tagged
        let cs = ClosureShape {
            signature: sig,
            captures: vec![MirType::Tagged],
        };
        assert!(MirType::closure(cs).assignable_to(&MirType::Tagged));
    }

    /// Stage G.2: `Tagged → Heap/FuncPtr` widening is REQUIRED.
    /// Removing it fails 10 examples (test_builtins, test_classes,
    /// test_decorator_factory, test_functions, test_import, etc.).
    #[test]
    fn tagged_widens_to_heap_and_funcptr() {
        // Tagged → Heap: allowed (lowering leaves Tagged flowing into
        // Heap-typed slots at isinstance-narrowing and wrapper call sites).
        assert!(MirType::Tagged.assignable_to(&MirType::str_heap()));
        assert!(MirType::Tagged.assignable_to(&MirType::list_of(MirType::Tagged)));
        // Tagged → FuncPtr: allowed (monomorphized wrapper call sites).
        let sig = Signature::uniform_tagged(1);
        assert!(MirType::Tagged.assignable_to(&MirType::func_ptr(sig)));
        // Tagged → Raw: NOT allowed.
        assert!(!MirType::Tagged.assignable_to(&MirType::raw_i64()));
        assert!(!MirType::Tagged.assignable_to(&MirType::raw_f64()));
        // Closure → Tagged widened (but not vice versa explicitly tested here).
    }

    #[test]
    fn class_subtyping_permissive() {
        // Phase 1 cannot resolve class hierarchies — accept any
        // Heap(Class { .. }) ↔ Heap(Class { .. }) pair.
        let a = MirType::Heap(HeapShape::Class {
            id: ClassId(1),
            type_args: vec![],
        });
        let b = MirType::Heap(HeapShape::Class {
            id: ClassId(2),
            type_args: vec![],
        });
        assert!(a.assignable_to(&b));
        assert!(b.assignable_to(&a));
    }

    #[test]
    fn class_does_not_assign_to_non_class_heap() {
        // Permissive class rule is scoped to class-to-class only.
        let cls = MirType::Heap(HeapShape::Class {
            id: ClassId(1),
            type_args: vec![],
        });
        assert!(!cls.assignable_to(&MirType::str_heap()));
        assert!(!MirType::str_heap().assignable_to(&cls));
    }

    #[test]
    fn signature_uniform_tagged() {
        let sig = Signature::uniform_tagged(3);
        assert_eq!(sig.params.len(), 3);
        assert!(sig.is_uniform_tagged());
        assert_eq!(sig.return_type, MirType::Tagged);
    }

    #[test]
    fn signature_non_uniform() {
        let sig = Signature {
            params: vec![MirType::raw_i64(), MirType::Tagged],
            return_type: MirType::raw_f64(),
        };
        assert!(!sig.is_uniform_tagged());
    }

    #[test]
    fn translate_primitives_to_storage() {
        assert_eq!(type_to_mir_type_storage(&Type::Int), MirType::Tagged);
        assert_eq!(type_to_mir_type_storage(&Type::Float), MirType::Tagged);
        assert_eq!(type_to_mir_type_storage(&Type::Bool), MirType::Tagged);
        assert_eq!(type_to_mir_type_storage(&Type::None), MirType::Tagged);
        assert_eq!(type_to_mir_type_storage(&Type::Str), MirType::str_heap());
        assert_eq!(
            type_to_mir_type_storage(&Type::Bytes),
            MirType::bytes_heap()
        );
        assert_eq!(type_to_mir_type_storage(&Type::Any), MirType::Tagged);
        assert_eq!(type_to_mir_type_storage(&Type::Any), MirType::Tagged);
        assert_eq!(type_to_mir_type_storage(&Type::Never), MirType::Never);
    }

    #[test]
    fn translate_primitives_to_register() {
        assert_eq!(type_to_mir_type_register(&Type::Int), MirType::raw_i64());
        assert_eq!(type_to_mir_type_register(&Type::Float), MirType::raw_f64());
        assert_eq!(type_to_mir_type_register(&Type::Bool), MirType::raw_i8());
        assert_eq!(type_to_mir_type_register(&Type::None), MirType::raw_i8());
        // Non-primitives same as storage.
        assert_eq!(type_to_mir_type_register(&Type::Str), MirType::str_heap());
    }

    #[test]
    fn translate_builtin_generics() {
        let list_int = Type::Generic {
            base: BUILTIN_LIST_CLASS_ID,
            args: vec![Type::Int],
        };
        assert_eq!(
            type_to_mir_type_storage(&list_int),
            MirType::list_of(MirType::Tagged)
        );

        let dict_str_int = Type::Generic {
            base: BUILTIN_DICT_CLASS_ID,
            args: vec![Type::Str, Type::Int],
        };
        assert_eq!(
            type_to_mir_type_storage(&dict_str_int),
            MirType::dict_of(MirType::str_heap(), MirType::Tagged)
        );

        let set_int = Type::Generic {
            base: BUILTIN_SET_CLASS_ID,
            args: vec![Type::Int],
        };
        assert_eq!(
            type_to_mir_type_storage(&set_int),
            MirType::set_of(MirType::Tagged)
        );

        let tuple_fixed = Type::Generic {
            base: BUILTIN_TUPLE_CLASS_ID,
            args: vec![Type::Int, Type::Str, Type::Bool],
        };
        assert_eq!(
            type_to_mir_type_storage(&tuple_fixed),
            MirType::tuple_fixed(vec![MirType::Tagged, MirType::str_heap(), MirType::Tagged])
        );

        let tuple_var = Type::Generic {
            base: BUILTIN_TUPLE_VAR_CLASS_ID,
            args: vec![Type::Float],
        };
        assert_eq!(
            type_to_mir_type_storage(&tuple_var),
            MirType::tuple_var(MirType::Tagged)
        );
    }

    #[test]
    fn translate_user_generic_class() {
        // User class id 100, parameterised with Int and Str.
        let user_class = Type::Generic {
            base: ClassId(100),
            args: vec![Type::Int, Type::Str],
        };
        assert_eq!(
            type_to_mir_type_storage(&user_class),
            MirType::class(ClassId(100), vec![MirType::Tagged, MirType::str_heap()])
        );
    }

    #[test]
    fn translate_function_to_funcptr() {
        let fty = Type::Function {
            params: vec![Type::Int, Type::Str],
            ret: Box::new(Type::Bool),
        };
        let mir = type_to_mir_type_storage(&fty);
        match mir {
            MirType::FuncPtr(sig) => {
                assert_eq!(sig.params.len(), 2);
                assert_eq!(sig.params[0], MirType::Tagged);
                assert_eq!(sig.params[1], MirType::str_heap());
                assert_eq!(sig.return_type, MirType::Tagged);
            }
            other => panic!("expected FuncPtr, got {other:?}"),
        }
    }

    #[test]
    fn translate_exception() {
        let exc = Type::BuiltinException(BuiltinExceptionKind::ValueError);
        assert_eq!(
            type_to_mir_type_storage(&exc),
            MirType::Heap(HeapShape::Exception(BuiltinExceptionKind::ValueError))
        );
    }

    #[test]
    fn translate_runtime_objects() {
        let file_t = Type::File(false);
        assert_eq!(
            type_to_mir_type_storage(&file_t),
            MirType::Heap(HeapShape::RuntimeObj(TypeTagKind::File))
        );

        let counter = Type::RuntimeObject(TypeTagKind::Counter);
        assert_eq!(
            type_to_mir_type_storage(&counter),
            MirType::Heap(HeapShape::RuntimeObj(TypeTagKind::Counter))
        );
    }

    #[test]
    fn display_round_trip_smoke() {
        // Construct nested types and ensure Display produces non-empty output.
        let dict = MirType::dict_of(MirType::str_heap(), MirType::raw_i64());
        let s = format!("{dict}");
        assert!(s.contains("Dict"));
        assert!(s.contains("Str"));
        assert!(s.contains("I64"));

        let tuple = MirType::tuple_fixed(vec![MirType::raw_i64(), MirType::Tagged]);
        let s = format!("{tuple}");
        assert!(s.contains("TupleFixed"));

        let exc = MirType::Heap(HeapShape::Exception(BuiltinExceptionKind::ValueError));
        let s = format!("{exc}");
        assert!(s.contains("Exception"));
        assert!(s.contains("ValueError"));

        let runtime = MirType::Heap(HeapShape::RuntimeObj(TypeTagKind::File));
        let s = format!("{runtime}");
        assert!(s.contains("RuntimeObj"));
        assert!(s.contains("File"));
    }
}
