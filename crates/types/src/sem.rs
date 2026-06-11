//! [`SemTy`] — the semantic (Python-level) type.
//!
//! This is what type inference, dispatch, and diagnostics reason about. It says
//! *nothing* about machine representation — that is [`crate::Repr`]'s job. The
//! gradual-typing "dynamic" type is the explicit [`SemTy::Dyn`]; there is no
//! representation-ambiguous "could be raw or pointer" type (see PITFALLS A1).

use pyaot_core_defs::{BuiltinExceptionKind, TypeTagKind};
use pyaot_utils::{ClassId, InternedString};

use crate::builtin_classes::{
    BUILTIN_DEQUE_CLASS_ID, BUILTIN_DICT_CLASS_ID, BUILTIN_LIST_CLASS_ID, BUILTIN_SET_CLASS_ID,
    BUILTIN_TUPLE_CLASS_ID, BUILTIN_TUPLE_VAR_CLASS_ID,
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
    pub fn dict_kv(&self) -> Option<(&SemTy, &SemTy)> {
        match self {
            SemTy::Generic { base, args } if *base == BUILTIN_DICT_CLASS_ID && args.len() == 2 => {
                Some((&args[0], &args[1]))
            }
            _ => None,
        }
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
