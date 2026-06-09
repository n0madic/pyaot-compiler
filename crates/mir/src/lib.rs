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
//! SSA, if ever needed, stays an optimizer-internal detail.
//!
//! ## What lives here
//!
//! * the IR shapes ([`MirProgram`] / [`MirFunction`] / [`MirInst`] / …) with
//!   parameterized "kind" enums ([`PrintKind`]) so runtime ops don't explode
//!   into one variant each;
//! * the **coercion legality table** ([`classify_coercion`] / [`Coercion`]).
//!   It lives here, not in `lowering`, because the verifier must enforce it and
//!   `mir` cannot depend on `lowering`. `lowering::legalize` is still the *only*
//!   place that *emits* a [`MirInst::Coerce`]; this is merely the shared
//!   predicate that makes "coercions only via legalize" structurally checkable.
//! * [`verify`] — mandatory from commit #1, run in debug at *every* pass
//!   boundary; rejects any instruction whose operand/result `Repr`s violate its
//!   typed signature.
//!
//! ## Phase 1 scope
//!
//! Only the instructions needed for `print("hello")` are interpreted by
//! [`verify`]. [`PrintKind`] and [`Coercion`] already carry their full variant
//! set so later phases grow without reshaping the enums.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use pyaot_types::{HeapShape, Repr};
use pyaot_utils::{BlockId, FuncId, InternedString, LocalId};

pub mod verify;
pub use verify::{verify, VerifyError};

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
    /// A parameterized print op — one variant covers every print form rather
    /// than one runtime-call variant per symbol.
    Print { kind: PrintKind, arg: Option<Operand> },
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
    /// Generic tagged-value print (`repr()`-ish dispatch).
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
    // Reserved: Int(..), Float(f64), Bool(bool), None, ...
}

#[derive(Debug, Clone)]
pub enum MirTerminator {
    Return(Option<Operand>),
    // Reserved: Branch { cond, then, else_ }, Jump(BlockId), Unreachable, ...
}

#[derive(Debug, Clone)]
pub enum Operand {
    Local(LocalId),
    // Reserved: immediate constants once they bypass a local slot.
}

// ============================================================================
// String pool
// ============================================================================

/// Maps each string literal's [`InternedString`] to its raw bytes. Lowering
/// fills it (resolving through the frontend's interner); codegen reads it to
/// emit one data object per distinct literal.
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

/// The kind of bridging a [`MirInst::Coerce`] performs. Phase 1 only ever
/// produces [`Coercion::Noop`]; the rest are reserved for Phase 2/3 numeric and
/// boxing coercions so the enum need not be reshaped later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coercion {
    /// Zero machine instructions: the bits are already valid at `to`.
    Noop,
    /// A typed heap pointer reinterpreted as the universal tagged value. Free at
    /// runtime in this runtime's ABI (heap pointers *are* tagged `Value`s), so
    /// Phase 1 classifies that case as [`Coercion::Noop`]; this variant is
    /// reserved for any future representation where the widen is not free.
    HeapToTagged,
    BoxFloat,
    UnboxFloat,
    TagInt,
    UntagInt,
}

/// **The single coercion legality table.** Returns `Some(kind)` if a value at
/// representation `from` may be legally bridged to `to`, else `None`.
///
/// `lowering::legalize::coerce` is a thin wrapper over this; the verifier calls
/// it directly. Keeping the predicate here (not in `lowering`) is what lets the
/// verifier enforce "every `Coerce` is a legal coercion" without `mir` depending
/// on `lowering`.
pub fn classify_coercion(from: &Repr, to: &Repr) -> Option<Coercion> {
    if from == to {
        return Some(Coercion::Noop);
    }
    match (from, to) {
        // A typed heap pointer is bit-identical to a tagged `Value` (rt_* return
        // `Value`), so widening it to the universal tagged repr costs nothing.
        (Repr::Heap(_), Repr::Tagged) => Some(Coercion::Noop),
        // Phase 2/3: numeric box/unbox/tag/untag land here.
        _ => None,
    }
}

/// True iff `to` is the universal heap-string representation. Small helper so
/// the verifier reads declaratively.
pub(crate) fn is_heap_str(repr: &Repr) -> bool {
    matches!(repr, Repr::Heap(HeapShape::Str))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyaot_types::RawKind;

    #[test]
    fn coercion_table_phase1() {
        // Identity is a no-op.
        assert_eq!(
            classify_coercion(&Repr::Tagged, &Repr::Tagged),
            Some(Coercion::Noop)
        );
        // A typed heap pointer widens to Tagged for free.
        assert_eq!(
            classify_coercion(&Repr::Heap(HeapShape::Str), &Repr::Tagged),
            Some(Coercion::Noop)
        );
        // Numeric box/unbox is not yet in the table (Phase 2/3).
        assert_eq!(
            classify_coercion(&Repr::Raw(RawKind::F64), &Repr::Tagged),
            None
        );
        // Narrowing Tagged back to a typed pointer is not a Phase-1 coercion.
        assert_eq!(
            classify_coercion(&Repr::Tagged, &Repr::Heap(HeapShape::Str)),
            None
        );
    }
}
