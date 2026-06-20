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
    BUILTIN_DEFAULTDICT_CLASS_ID, BUILTIN_DEQUE_CLASS_ID, BUILTIN_DICT_CLASS_ID,
    BUILTIN_LIST_CLASS_ID, BUILTIN_SET_CLASS_ID, BUILTIN_TUPLE_CLASS_ID,
    BUILTIN_TUPLE_VAR_CLASS_ID,
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

/// The conservative magnitude bound for the proof-gated `int → Raw(I64)`
/// narrowing (Phase 3c). A value provably in `[-BOUND, BOUND]` cannot promote to
/// a heap `BigInt` and leaves ample headroom so that a raw `Add`/`Sub` of two
/// such values never overflows i64 *and* its result is still a valid tagged
/// fixnum (so re-tagging into a tagged slot round-trips). `2^48` covers any
/// realistic literal-bounded loop while staying far below the `i64::MAX >> 3`
/// fixnum ceiling (~`2^60`). Soundness over completeness: when in doubt, the
/// slot stays `Tagged` (PITFALLS A6).
pub const RAW_I64_NARROW_BOUND: i64 = 1 << 48;

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

/// The runtime shape-guard a gradual `Tagged → Heap(shape)` coercion needs to
/// stay safe: when a genuinely-`Dyn` value flows into a typed heap
/// slot, lowering emits a CHECKED coercion that calls one of these guards
/// (`rt_check_heap_kind` / `rt_check_instance`) to raise `TypeError` at the
/// boundary instead of crashing later at the first container op. This is the
/// `Heap` analogue of the `Raw` checked-unbox family (`rt_unbox_float`/…).
/// See PITFALLS B18: a checked `Heap` coercion is admissible ONLY for the
/// shapes with a matching raising guard — exactly the set
/// [`HeapShape::dyn_check`] returns `Some` for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeapCheck {
    /// A builtin-container tag check — `code` is a
    /// [`pyaot_core_defs::isinstance_kind`] code (`str`/`bytes`/`list`/`dict`/
    /// `set`/`tuple`). The `dict` code is family-aware at the runtime
    /// (`Dict`/`DefaultDict`/`Counter` share one layout); the rest are
    /// subtype-free singletons (user classes cannot subclass builtins), so an
    /// exact-tag check is regression-safe.
    Kind(u8),
    /// A user-class instance check — subclass-aware (`rt_class_inherits_from`),
    /// so a `Dog` value passes an `Animal` param.
    Class(ClassId),
}

impl HeapShape {
    /// The runtime guard a gradual `Tagged → Heap(self)` coercion must call, or
    /// `None` for the rare shapes that keep the unchecked reinterpret
    /// (`BigInt`/`RuntimeObj`/`Iterator` — no isinstance code and no nominal
    /// class to check against).
    ///
    /// This single mapping is the source of truth consumed by `mir`'s checked
    /// admission ([`crate`]-external `CoerceInst::new_checked`), `lowering`'s
    /// needs-check gate (`coerce_value`), and `codegen`'s guard dispatch
    /// (`lower_coerce`) — no logic is duplicated across the three crates.
    pub fn dyn_check(&self) -> Option<HeapCheck> {
        use pyaot_core_defs::isinstance_kind as k;
        match self {
            HeapShape::Str => Some(HeapCheck::Kind(k::STR as u8)),
            HeapShape::Bytes => Some(HeapCheck::Kind(k::BYTES as u8)),
            HeapShape::List(_) => Some(HeapCheck::Kind(k::LIST as u8)),
            HeapShape::Dict(..) => Some(HeapCheck::Kind(k::DICT as u8)),
            HeapShape::Set(_) => Some(HeapCheck::Kind(k::SET as u8)),
            HeapShape::Tuple(_) | HeapShape::TupleVar(_) => Some(HeapCheck::Kind(k::TUPLE as u8)),
            HeapShape::Class(cid) => Some(HeapCheck::Class(*cid)),
            HeapShape::BigInt | HeapShape::RuntimeObj(_) | HeapShape::Iterator(_) => None,
        }
    }
}

/// A function-pointer signature at the representation level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigRepr {
    pub params: Vec<Repr>,
    pub ret: Box<Repr>,
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
    /// Bare code address with a typed signature (the dunder-pointer shape;
    /// `repr_of` never produces it for user values).
    FuncPtr(Box<SigRepr>),
    /// A closure value (Phase 6): physically ONE tagged pointer to an ordinary
    /// runtime tuple of `1+N` slots — slot 0 the int-tagged target code address,
    /// slots `1..=N` the captured cells (each a tagged `Value`). The signature is
    /// the *visible* one (env excluded); the capture count is not part of the
    /// representation (it lives only on `MakeClosure`), so every closure of the
    /// same signature shares one `Repr` and one indirect-call ABI (PITFALLS A4:
    /// no marker bits, no per-function ABI flags).
    Closure(Box<SigRepr>),
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

        // A callable VALUE is always the uniform env-tuple closure (Phase 6) —
        // never a bare code address, so one indirect-call ABI covers captureless
        // functions, closures, and thunked top-level functions alike. Every
        // closure shares ONE repr, [`GENERIC_SIG`]: slot 0 is an arity-generic
        // `(args_tuple, kwargs_dict) → Value` uniform thunk, so even a
        // genuinely-`Dyn` callee is callable through the single indirect ABI.
        // The precise `Sig` survives in `SemTy::Callable` only as a (later)
        // devirtualization hint — it is no longer the call signature.
        SemTy::Callable(_) => Repr::Closure(Box::new(generic_sig())),

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
    } else if base == BUILTIN_DICT_CLASS_ID || base == BUILTIN_DEFAULTDICT_CLASS_ID {
        // A defaultdict IS a `DictObj` — its repr is honestly `Heap(Dict(K, V))`,
        // so every dict-keyed op (store, del, view) is repr-identical (PITFALLS
        // A1/A2: the Tagged baseline stays correct; this is just the dict shape).
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

/// The single uniform closure ABI signature: every callable value is the
/// arity-generic `(args_tuple: Tagged, kwargs_dict: Tagged) → Value` shape, with
/// the env tuple added as a leading arg by codegen. All closures share this
/// `Repr::Closure(GENERIC_SIG)`, so every `CallIndirect` carries it and the MIR
/// verifier's strict `Closure(s) == sig` check holds with no relaxation. The
/// per-function specialized native ABI survives only for **direct** by-name
/// calls; value-position calls route through this uniform entry (PITFALLS A4: one
/// ABI, no per-function flags).
pub fn generic_sig() -> SigRepr {
    SigRepr {
        params: vec![Repr::Tagged, Repr::Tagged],
        ret: Box::new(Repr::Tagged),
    }
}

/// The representation-level signature of a semantic [`Sig`] (the visible
/// signature — the closure env param is an ABI detail added at lowering). Used
/// for the dunder `FuncPtr` shape and as a devirtualization hint; a closure
/// VALUE's repr is always [`generic_sig`], never this.
pub fn sig_repr(sig: &Sig) -> SigRepr {
    SigRepr {
        params: sig.params.iter().map(repr_of).collect(),
        ret: Box::new(repr_of(&sig.ret)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::{NoClasses, TypeLattice};

    fn callable(params: Vec<SemTy>, ret: SemTy) -> SemTy {
        SemTy::Callable(Box::new(Sig::fixed(params, ret)))
    }

    #[test]
    fn repr_of_callable_is_closure() {
        // A callable VALUE always maps to the uniform env-tuple closure repr
        // (the single `GENERIC_SIG` ABI), regardless of its precise signature —
        // so a genuinely-`Dyn` callee is callable through the same indirect ABI.
        let c = callable(vec![SemTy::Int], SemTy::Int);
        match repr_of(&c) {
            Repr::Closure(sig) => assert_eq!(*sig, generic_sig()),
            other => panic!("expected Closure, got {other:?}"),
        }
        // A different precise signature still yields the very same repr.
        let c2 = callable(vec![SemTy::Str, SemTy::Float], SemTy::NoneTy);
        assert_eq!(repr_of(&c), repr_of(&c2));
        assert!(repr_of(&c).is_gc_root());
    }

    #[test]
    fn join_distinct_callables_is_dyn() {
        // Two different signatures never merge into one sig nor a union — the
        // join is the gradual top, so the slot stays Tagged (Phase 6A).
        let a = callable(vec![SemTy::Int], SemTy::Int);
        let b = callable(vec![SemTy::Str, SemTy::Str], SemTy::Str);
        assert_eq!(a.join(&b, &NoClasses), SemTy::Dyn);
        // The same signature joins to itself.
        assert_eq!(a.join(&a, &NoClasses), a);
    }
}
