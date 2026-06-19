//! [`SemTy`] — the semantic (Python-level) type.
//!
//! This is what type inference, dispatch, and diagnostics reason about. It says
//! *nothing* about machine representation — that is [`crate::Repr`]'s job. The
//! gradual-typing "dynamic" type is the explicit [`SemTy::Dyn`]; there is no
//! representation-ambiguous "could be raw or pointer" type (see PITFALLS A1).

use pyaot_core_defs::{BuiltinExceptionKind, TypeTagKind};
use pyaot_utils::{ClassId, InternedString};

use crate::builtin_classes::{
    BUILTIN_DEFAULTDICT_CLASS_ID, BUILTIN_DEQUE_CLASS_ID, BUILTIN_DICT_CLASS_ID,
    BUILTIN_LIST_CLASS_ID, BUILTIN_SET_CLASS_ID, BUILTIN_TUPLE_CLASS_ID, BUILTIN_TUPLE_VAR_CLASS_ID,
};

/// A callable signature at the semantic level.
///
/// `params` lists every parameter type IN ABI ORDER, including (when the flags
/// are set) the trailing `*args` tuple and `**kwargs` dict — each is ONE
/// parameter (`tuple[Dyn, ...]` / `dict[str, Dyn]`), so the indirect-call
/// signature stays fixed regardless of how many values a call site spreads
/// (Phase 6C; this is what makes `def wrapper(*args, **kwargs)` decoratable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sig {
    pub params: Vec<SemTy>,
    pub ret: SemTy,
    /// The last (or second-to-last, before `kwargs`) param is a `*args` tuple.
    pub varargs: bool,
    /// The last param is a `**kwargs` dict.
    pub kwargs: bool,
}

impl Sig {
    /// A plain fixed-arity signature (no `*args` / `**kwargs`).
    pub fn fixed(params: Vec<SemTy>, ret: SemTy) -> Sig {
        Sig {
            params,
            ret,
            varargs: false,
            kwargs: false,
        }
    }

    /// The number of leading fixed (non-`*args`/`**kwargs`) parameters.
    pub fn fixed_arity(&self) -> usize {
        self.params.len() - usize::from(self.varargs) - usize::from(self.kwargs)
    }
}

/// The semantic (Python-level) type.
///
/// Note: `Int` is **arbitrary precision** by design (a fixed-width tagged int
/// that silently wraps is intentionally avoided — PITFALLS A6). Whether an `int`
/// lives as an unboxed `i64` or as a heap bignum is a *representation* decision
/// made later, never a semantic one — see [`crate::repr::repr_of`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemTy {
    // ── primitives ──
    Int,
    Float,
    Bool,
    Str,
    Bytes,
    NoneTy,

    /// Unified generic: built-in containers (`base ∈ builtin_classes::*`) and
    /// user-defined generic classes (`Stack[T]`) share one shape.
    Generic {
        base: ClassId,
        args: Vec<SemTy>,
    },

    /// Nominal user-defined class instance.
    Class {
        class_id: ClassId,
        name: InternedString,
    },

    /// Callable value (function / lambda / bound method).
    Callable(Box<Sig>),

    /// stdlib runtime-backed object (StructTime, CompletedProcess, ...).
    RuntimeObject(TypeTagKind),
    /// Built-in exception type (for `try/except` checking).
    BuiltinException(BuiltinExceptionKind),
    /// `open()` result; `binary` distinguishes text (`str`) from binary (`bytes`).
    File {
        binary: bool,
    },
    /// Iterator yielding the inner element type.
    Iterator(Box<SemTy>),

    // ── lattice / gradual ──
    /// Normalized union with unique members (also encodes `Optional[T]`).
    Union(Vec<SemTy>),
    /// Type variable / inference variable. Erased before codegen by monomorphization.
    Var(InternedString),
    /// Gradual "dynamic". Consistent with every type; its `Repr` is always
    /// [`crate::Repr::Tagged`]. There is no representation-ambiguous `Any`.
    Dyn,
    /// Bottom type (empty union, unreachable). Subtype of everything.
    Never,
    /// `NotImplemented` sentinel — a control-flow signal for dunder fallback,
    /// not a real value type.
    NotImplementedT,
}

impl SemTy {
    // ── container constructors ──

    pub fn list_of(elem: SemTy) -> SemTy {
        SemTy::Generic {
            base: BUILTIN_LIST_CLASS_ID,
            args: vec![elem],
        }
    }
    pub fn dict_of(k: SemTy, v: SemTy) -> SemTy {
        SemTy::Generic {
            base: BUILTIN_DICT_CLASS_ID,
            args: vec![k, v],
        }
    }
    pub fn set_of(elem: SemTy) -> SemTy {
        SemTy::Generic {
            base: BUILTIN_SET_CLASS_ID,
            args: vec![elem],
        }
    }
    pub fn tuple_of(elems: Vec<SemTy>) -> SemTy {
        SemTy::Generic {
            base: BUILTIN_TUPLE_CLASS_ID,
            args: elems,
        }
    }
    pub fn tuple_var_of(elem: SemTy) -> SemTy {
        SemTy::Generic {
            base: BUILTIN_TUPLE_VAR_CLASS_ID,
            args: vec![elem],
        }
    }
    pub fn deque_of(elem: SemTy) -> SemTy {
        SemTy::Generic {
            base: BUILTIN_DEQUE_CLASS_ID,
            args: vec![elem],
        }
    }
    /// `collections.defaultdict[K, V]`. Physically a `DictObj`
    /// (`repr_of` → `Heap(Dict(K, V))`); the only behavioral divergence from a
    /// plain dict is the subscript-read (auto-insert the factory default on a
    /// miss). `dict_kv()` matches this base so every dict-keyed site treats it
    /// as a dict; the read divergence is keyed on [`SemTy::is_defaultdict`].
    pub fn defaultdict_of(k: SemTy, v: SemTy) -> SemTy {
        SemTy::Generic {
            base: BUILTIN_DEFAULTDICT_CLASS_ID,
            args: vec![k, v],
        }
    }
    /// The value `SemTy` a `defaultdict(...)` factory tag denotes (the runtime
    /// `FACTORY_*` constants in `runtime/src/defaultdict.rs`). This is the single
    /// source of truth shared by the frontend (which maps a factory Name → tag)
    /// and typeck (which recovers `V` from the tag literal the construction call
    /// carries, since the descriptor's return `TypeSpec` cannot encode a per-call
    /// value type). An unknown / absent (`-1`) tag is gradual `Dyn`.
    pub fn defaultdict_value_ty(tag: i64) -> SemTy {
        match tag {
            0 => SemTy::Int,
            1 => SemTy::Float,
            2 => SemTy::Str,
            3 => SemTy::Bool,
            4 => SemTy::list_of(SemTy::Dyn),
            5 => SemTy::dict_of(SemTy::Dyn, SemTy::Dyn),
            6 => SemTy::set_of(SemTy::Dyn),
            _ => SemTy::Dyn,
        }
    }

    /// `Optional[T]` sugar (`Union[T, None]`).
    pub fn optional(t: SemTy) -> SemTy {
        if t == SemTy::NoneTy {
            SemTy::NoneTy
        } else {
            SemTy::Union(vec![t, SemTy::NoneTy])
        }
    }

    // ── container accessors ──

    pub fn list_elem(&self) -> Option<&SemTy> {
        match self {
            SemTy::Generic { base, args } if *base == BUILTIN_LIST_CLASS_ID => args.first(),
            _ => None,
        }
    }
    /// The `(K, V)` of a `dict[K, V]` OR a `defaultdict[K, V]` — both share the
    /// `DictObj` layout, so every dict-keyed site (store, del, `.get`/`.keys`/
    /// `.values`, subscript typing, iter) transparently treats a defaultdict as
    /// a dict. The defaultdict-only subscript-read divergence is gated separately
    /// by [`SemTy::is_defaultdict`], checked *before* the generic dict-read path.
    pub fn dict_kv(&self) -> Option<(&SemTy, &SemTy)> {
        match self {
            SemTy::Generic { base, args }
                if (*base == BUILTIN_DICT_CLASS_ID || *base == BUILTIN_DEFAULTDICT_CLASS_ID)
                    && args.len() == 2 =>
            {
                Some((&args[0], &args[1]))
            }
            _ => None,
        }
    }
    /// True iff this is a `defaultdict[K, V]` base — the one site where a
    /// defaultdict diverges from a plain dict (the auto-inserting subscript-read)
    /// keys off this, never off a method name (the §10 trap).
    pub fn is_defaultdict(&self) -> bool {
        matches!(self, SemTy::Generic { base, .. } if *base == BUILTIN_DEFAULTDICT_CLASS_ID)
    }
    pub fn set_elem(&self) -> Option<&SemTy> {
        match self {
            SemTy::Generic { base, args } if *base == BUILTIN_SET_CLASS_ID => args.first(),
            _ => None,
        }
    }
    pub fn tuple_elems(&self) -> Option<&[SemTy]> {
        match self {
            SemTy::Generic { base, args } if *base == BUILTIN_TUPLE_CLASS_ID => Some(args),
            _ => None,
        }
    }
    pub fn tuple_var_elem(&self) -> Option<&SemTy> {
        match self {
            SemTy::Generic { base, args } if *base == BUILTIN_TUPLE_VAR_CLASS_ID => args.first(),
            _ => None,
        }
    }
    pub fn deque_elem(&self) -> Option<&SemTy> {
        match self {
            SemTy::Generic { base, args } if *base == BUILTIN_DEQUE_CLASS_ID => args.first(),
            _ => None,
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, SemTy::NoneTy)
    }

    // ── TypeVar support (used by monomorphization) ──

    /// True iff any `Var` leaf appears anywhere in the type tree.
    pub fn contains_var(&self) -> bool {
        match self {
            SemTy::Var(_) => true,
            SemTy::Generic { args, .. } => args.iter().any(SemTy::contains_var),
            SemTy::Union(ts) => ts.iter().any(SemTy::contains_var),
            SemTy::Iterator(t) => t.contains_var(),
            SemTy::Callable(sig) => {
                sig.params.iter().any(SemTy::contains_var) || sig.ret.contains_var()
            }
            _ => false,
        }
    }

    /// Recursively replace every `Var(name)` using `subst`; unmapped vars stay.
    pub fn substitute(&self, subst: &std::collections::HashMap<InternedString, SemTy>) -> SemTy {
        match self {
            SemTy::Var(name) => subst.get(name).cloned().unwrap_or_else(|| self.clone()),
            SemTy::Generic { base, args } => SemTy::Generic {
                base: *base,
                args: args.iter().map(|a| a.substitute(subst)).collect(),
            },
            SemTy::Union(ts) => SemTy::Union(ts.iter().map(|t| t.substitute(subst)).collect()),
            SemTy::Iterator(t) => SemTy::Iterator(Box::new(t.substitute(subst))),
            SemTy::Callable(sig) => SemTy::Callable(Box::new(Sig {
                params: sig.params.iter().map(|p| p.substitute(subst)).collect(),
                ret: sig.ret.substitute(subst),
                varargs: sig.varargs,
                kwargs: sig.kwargs,
            })),
            other => other.clone(),
        }
    }
}
