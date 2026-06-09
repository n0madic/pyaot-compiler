//! # MIR — Mid-level IR (CFG), representation-typed
//!
//! Every MIR value carries a [`pyaot_types::Repr`] **by value, not `Option`**:
//! there is exactly one representation field and it is total. (A dual
//! logical/physical type field with an optional, dual-meaning sentinel is the
//! anti-pattern this design exists to prevent — see PITFALLS A1.)
//!
//! The model is **locals-with-a-Repr-table, not SSA**: the runtime's GC roots
//! are frame slots, so a locals model maps 1:1 to rootable slots. (SSA would
//! need a separate spill pass — exactly the side-table the invariants forbid.)
//!
//! ## What lives here
//!
//! * the IR shapes ([`MirProgram`] / [`MirFunction`] / [`MirInst`] / …);
//! * the **coercion legality table** ([`classify_coercion`] / [`Coercion`]).
//!   It lives here, not in `lowering`, because the verifier must enforce it and
//!   `mir` cannot depend on `lowering`. `lowering::legalize` is still the *only*
//!   place that *emits* a [`MirInst::Coerce`]; this is merely the shared
//!   predicate that makes "coercions only via legalize" structurally checkable.
//! * [`verify`] — mandatory from commit #1, run in debug at *every* pass
//!   boundary; rejects any instruction whose operand/result `Repr`s violate its
//!   typed signature.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use pyaot_types::{HeapShape, RawKind, Repr};
use pyaot_utils::{BlockId, FuncId, InternedString, LocalId};

pub mod verify;
pub use verify::{verify, VerifyError};

// Re-exported so consumers (`lowering`, `codegen`) can name builtin kinds.
pub use pyaot_core_defs::BuiltinFunctionKind;
// Re-exported so consumers can name container ops without a direct `hir` dep.
// `ContainerCmpOp` is the HIR comparison operator carried by `ContainerOp`'s
// ordering variants (aliased to avoid clashing with this crate's own `CmpOp`).
pub use pyaot_hir::{ContainerArg, ContainerOp, ContainerResult, CmpOp as ContainerCmpOp};

// ============================================================================
// Program / function structure
// ============================================================================

/// A whole compiled program: functions (indexed by [`FuncId`]) plus the pool of
/// string-literal bytes the codegen backend materializes into data objects.
#[derive(Debug)]
pub struct MirProgram {
    pub funcs: Vec<MirFunction>,
    /// The synthetic `__main__` function codegen wraps in C `main`.
    pub entry: FuncId,
    pub str_pool: StrPool,
}

/// A function. `locals` is the Repr table; every [`LocalId`] indexes it.
/// `params.len()` leading locals are the parameters (ABI = f(param Repr)).
#[derive(Debug)]
pub struct MirFunction {
    pub name: InternedString,
    pub params: Vec<Repr>,
    pub ret: Repr,
    pub locals: Vec<LocalDecl>,
    pub blocks: Vec<MirBlock>,
    pub entry: BlockId,
}

impl MirFunction {
    /// The representation of a local.
    pub fn local_repr(&self, id: LocalId) -> &Repr {
        &self.locals[id.index()].repr
    }

    /// The representation an operand evaluates to.
    pub fn operand_repr(&self, op: &Operand) -> &Repr {
        match op {
            Operand::Local(id) => self.local_repr(*id),
        }
    }
}

/// A local slot's declaration. `Repr` is mandatory and by value (never
/// `Option`); GC-rootness is derived from it via [`Repr::is_gc_root`], never
/// stored here.
#[derive(Debug, Clone)]
pub struct LocalDecl {
    pub repr: Repr,
}

/// A basic block: straight-line instructions ending in exactly one terminator.
#[derive(Debug)]
pub struct MirBlock {
    pub insts: Vec<MirInst>,
    pub term: MirTerminator,
}

// ============================================================================
// Instructions / operands / terminators
// ============================================================================

#[derive(Debug, Clone)]
pub enum MirInst {
    /// Materialize a constant into `dst`.
    Const { dst: LocalId, val: Const },
    /// Bridge a value's representation from `from` to `to`. **Only**
    /// `lowering::legalize` emits this, and only when [`classify_coercion`]
    /// accepts `(from, to)`; the verifier re-checks both facts.
    Coerce {
        dst: LocalId,
        src: Operand,
        from: Repr,
        to: Repr,
    },
    /// A binary op on the tagged baseline. ALL ops (arithmetic *and* bitwise /
    /// shift) take and produce `Tagged` and dispatch on the tag in the runtime
    /// (`rt_obj_*`), so they are bignum-safe: an `int` operand may dynamically be
    /// a heap `BigInt`, and unboxing it to a raw `i64` would be a silent
    /// miscompile (Invariant 2). A range-proven raw fast path for bitwise/shift
    /// is a Phase-3 optimization, not the correct default.
    BinOp {
        dst: LocalId,
        op: BinOp,
        l: Operand,
        r: Operand,
    },
    /// Unary `Neg`/`Pos`/`Invert` on the tagged baseline; `Not` is truthiness
    /// negation (tagged operand → `Raw(I8)` result).
    Unary {
        dst: LocalId,
        op: UnaryOp,
        operand: Operand,
    },
    /// A single comparison (tagged operands → `Raw(I8)` result).
    Compare {
        dst: LocalId,
        op: CmpOp,
        l: Operand,
        r: Operand,
    },
    /// Truthiness test (tagged operand → `Raw(I8)` result).
    Truthy { dst: LocalId, operand: Operand },
    /// Call a compiled function. Args coerced to the callee's param `Repr`s.
    Call {
        dst: Option<LocalId>,
        func: FuncId,
        args: Vec<Operand>,
    },
    /// Call a runtime builtin (`abs`/`len`/`int`/`float`/`str`/`bool`/…). The
    /// runtime shims take and return tagged `Value`s.
    CallBuiltin {
        dst: Option<LocalId>,
        kind: BuiltinFunctionKind,
        args: Vec<Operand>,
    },
    /// Call a container / iterator runtime op (Phase 4). Parallels `CallBuiltin`
    /// but with a per-op argument/result representation signature
    /// ([`ContainerOp::arg_kinds`] / [`ContainerOp::result`]) the verifier
    /// enforces. Element/key/value args are `Tagged` (uniform tagged storage,
    /// PITFALLS A5); index/count/size args are `Raw(I64)`; the `dst` repr is the
    /// op's result category. The concrete `rt_*` to call is selected at codegen
    /// from `op` plus the receiver representation, exactly as the verifier sees it.
    CallContainer {
        dst: Option<LocalId>,
        op: ContainerOp,
        args: Vec<Operand>,
    },
    /// Raise `AssertionError` (no message in Phase 2). Followed by `Unreachable`.
    AssertFail,
    /// A parameterized print op — one variant covers every print form rather
    /// than one runtime-call variant per symbol.
    Print { kind: PrintKind, arg: Option<Operand> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Pos,
    Invert,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    NotEq,
    Lt,
    LtE,
    Gt,
    GtE,
}

/// The flavor of a print operation. Parameterized so the runtime print surface
/// does not explode into per-symbol instruction variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintKind {
    /// `str()`-semantics: print a string object's raw bytes, no quotes.
    StrObj,
    Int,
    Float,
    Bool,
    None_,
    /// Generic tagged-value print (tag-dispatched; bignum-safe for ints).
    Obj,
    /// The default `' '` separator between arguments.
    Sep,
    /// The trailing newline.
    Newline,
}

#[derive(Debug, Clone)]
pub enum Const {
    /// A string literal; the bytes live in [`MirProgram::str_pool`].
    Str(InternedString),
    /// A `bytes` literal `b"…"`; the raw bytes live in [`MirProgram::str_pool`].
    /// Codegen materializes it via `rt_make_bytes` into a `Heap(Bytes)`.
    Bytes(InternedString),
    /// A fixnum integer literal (tagged at codegen).
    Int(i64),
    /// A big integer literal; decimal text lives in [`MirProgram::str_pool`].
    BigIntStr(InternedString),
    Float(f64),
    Bool(bool),
    None,
}

#[derive(Debug, Clone)]
pub enum MirTerminator {
    Return(Option<Operand>),
    Jump(BlockId),
    Branch {
        cond: Operand,
        then: BlockId,
        else_: BlockId,
    },
    Unreachable,
}

#[derive(Debug, Clone)]
pub enum Operand {
    Local(LocalId),
}

// ============================================================================
// String pool
// ============================================================================

/// Maps each interned string (literal bytes or big-int decimal text) to its raw
/// bytes. Lowering fills it; codegen reads it to emit one data object per id.
#[derive(Debug, Default)]
pub struct StrPool {
    bytes: HashMap<InternedString, Vec<u8>>,
}

impl StrPool {
    pub fn new() -> Self {
        Self { bytes: HashMap::new() }
    }

    /// Record the bytes of a string literal (idempotent for a given id).
    pub fn insert(&mut self, id: InternedString, bytes: Vec<u8>) {
        self.bytes.entry(id).or_insert(bytes);
    }

    /// The bytes of a previously-recorded literal.
    pub fn bytes(&self, id: InternedString) -> Option<&[u8]> {
        self.bytes.get(&id).map(Vec::as_slice)
    }

    /// Iterate every (id, bytes) pair — codegen declares one data object each.
    pub fn iter(&self) -> impl Iterator<Item = (InternedString, &[u8])> {
        self.bytes.iter().map(|(id, b)| (*id, b.as_slice()))
    }
}

// ============================================================================
// Coercion legality (the shared predicate the verifier enforces)
// ============================================================================

/// The kind of bridging a [`MirInst::Coerce`] performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coercion {
    /// Zero machine instructions: the bits are already valid at `to`.
    Noop,
    /// A typed heap pointer reinterpreted as the universal tagged value — free
    /// in this runtime's ABI (heap pointers *are* tagged `Value`s).
    HeapToTagged,
    /// The sound reverse: a tagged value re-typed as a heap pointer. Bit-identical
    /// (a heap pointer *is* a tagged value), so it is a zero-instruction Noop in
    /// codegen — unlike `UnboxFloat`/`UntagBool`, it does **not** reinterpret bits
    /// by an assumed primitive type, so a value of the wrong dynamic type is not
    /// immediately mis-read. It is emitted only when `typeck` has typed the slot as
    /// that container/iterator (e.g. a uniform-tagged `rt_*_get` result feeding a
    /// `list[list[int]]` element local, or `iter_next` into a typed loop variable),
    /// so the narrowing is proven sound.
    TaggedToHeap,
    BoxFloat,
    UnboxFloat,
    TagInt,
    UntagInt,
    TagBool,
    UntagBool,
}

/// **The single coercion legality table.** Returns `Some(kind)` if a value at
/// representation `from` may be legally bridged to `to`, else `None`.
pub fn classify_coercion(from: &Repr, to: &Repr) -> Option<Coercion> {
    if from == to {
        return Some(Coercion::Noop);
    }
    match (from, to) {
        // Two heap shapes of the same container *family* (same constructor; same
        // arity for fixed tuples) are physically identical — element/key/value
        // representation is compile-time metadata only, since every slot is stored
        // as a tagged `Value`. So re-typing one as the other (a `list[Never]`
        // comprehension result into an annotated `list[int]`, a `list[int]` into a
        // `list[Dyn]`, …) is a zero-instruction Noop. Different families
        // (`list` → `dict`) stay illegal: that would mis-dispatch the runtime.
        (Repr::Heap(a), Repr::Heap(b)) if same_container_family(a, b) => Some(Coercion::Noop),
        // A typed heap pointer is bit-identical to a tagged `Value` (both ways).
        (Repr::Heap(_), Repr::Tagged) => Some(Coercion::Noop),
        (Repr::Tagged, Repr::Heap(_)) => Some(Coercion::TaggedToHeap),
        (Repr::Raw(RawKind::F64), Repr::Tagged) => Some(Coercion::BoxFloat),
        (Repr::Tagged, Repr::Raw(RawKind::F64)) => Some(Coercion::UnboxFloat),
        (Repr::Raw(RawKind::I8), Repr::Tagged) => Some(Coercion::TagBool),
        (Repr::Tagged, Repr::Raw(RawKind::I8)) => Some(Coercion::UntagBool),
        (Repr::Raw(RawKind::I64), Repr::Tagged) => Some(Coercion::TagInt),
        (Repr::Tagged, Repr::Raw(RawKind::I64)) => Some(Coercion::UntagInt),
        _ => None,
    }
}

/// True iff `to` is the universal heap-string representation.
pub(crate) fn is_heap_str(repr: &Repr) -> bool {
    matches!(repr, Repr::Heap(HeapShape::Str))
}

/// True iff two heap shapes are the same container *family* — the same physical
/// object kind, differing only in compile-time element/key/value metadata (which
/// is irrelevant because every container slot is a tagged `Value`). Fixed tuples
/// must additionally share arity. Non-container heap shapes match only themselves
/// (handled by the `from == to` fast path, so they are not listed here).
fn same_container_family(a: &HeapShape, b: &HeapShape) -> bool {
    use HeapShape::{Dict, Iterator, List, Set, Tuple, TupleVar};
    match (a, b) {
        (List(_), List(_)) => true,
        (Dict(..), Dict(..)) => true,
        (Set(_), Set(_)) => true,
        (TupleVar(_), TupleVar(_)) => true,
        (Tuple(x), Tuple(y)) => x.len() == y.len(),
        (Iterator(_), Iterator(_)) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coercion_table_phase2() {
        assert_eq!(
            classify_coercion(&Repr::Tagged, &Repr::Tagged),
            Some(Coercion::Noop)
        );
        assert_eq!(
            classify_coercion(&Repr::Heap(HeapShape::Str), &Repr::Tagged),
            Some(Coercion::Noop)
        );
        assert_eq!(
            classify_coercion(&Repr::Raw(RawKind::F64), &Repr::Tagged),
            Some(Coercion::BoxFloat)
        );
        assert_eq!(
            classify_coercion(&Repr::Tagged, &Repr::Raw(RawKind::F64)),
            Some(Coercion::UnboxFloat)
        );
        assert_eq!(
            classify_coercion(&Repr::Raw(RawKind::I8), &Repr::Tagged),
            Some(Coercion::TagBool)
        );
        assert_eq!(
            classify_coercion(&Repr::Tagged, &Repr::Raw(RawKind::I8)),
            Some(Coercion::UntagBool)
        );
        // Tagged → a typed heap pointer is the sound, bit-identical reverse Noop
        // (emitted only where typeck has proven the slot's container type).
        assert_eq!(
            classify_coercion(&Repr::Tagged, &Repr::Heap(HeapShape::Str)),
            Some(Coercion::TaggedToHeap)
        );
    }
}
