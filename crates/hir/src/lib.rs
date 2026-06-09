//! # HIR — High-level IR (CFG-only)
//!
//! Control-flow-graph IR: functions own `blocks`, an `entry` block, and a flat
//! `exprs` arena; structured control flow lives in an [`HirTerminator`], not in
//! nested statement variants. Generators are desugared into regular functions at
//! this level (Phase 6).
//!
//! Every typed slot carries a [`pyaot_types::SemTy`] **only** — physical
//! representation ([`pyaot_types::Repr`]) is assigned later at the lowering
//! boundary, never stored here. There is no representation-ambiguous `Any` here.
//!
//! ## Arena strategy
//!
//! * **Intra-function** `blocks` / `exprs` use [`la_arena`] (`Idx<_>` handles).
//! * **Cross-function** references use the [`pyaot_utils::FuncId`] newtype: a
//!   module's `functions` is a `Vec<HirFunction>` indexed by `FuncId`, so a
//!   function's identity survives unchanged across the lowering boundary into
//!   MIR (HIR `FuncId` == MIR `FuncId`).
//!
//! ## Locals
//!
//! Each function owns a flat [`HirLocal`] table; a [`pyaot_utils::LocalId`] is an
//! index into it. Parameters occupy the first `params.len()` slots, so HIR
//! `LocalId` maps 1:1 onto the MIR `LocalId` Repr table. The frontend allocates
//! every named local up front and refers to them by `Symbol::Local`; `typeck`
//! refines each [`HirLocal::ty`] (so `repr_of` can pick `Raw` for float/bool
//! locals) but the always-correct tagged baseline holds even if it does not.
//!
//! ## Name-resolution vocabulary
//!
//! [`SymbolRef`], [`Symbol`], and [`ResolveResult`] are the *shapes* of name
//! resolution and live here because [`SymbolRef`] is embedded directly in
//! [`HirExprKind::Name`]. The `pyaot-semantics` crate owns the *algorithm* that
//! fills them (`resolve`), while `pyaot-typeck` and `pyaot-lowering` consume the
//! result.

#![forbid(unsafe_code)]

use la_arena::{Arena, Idx};

use pyaot_types::SemTy;
use pyaot_utils::{FuncId, InternedString, LocalId, Span, SymbolId};

// Re-exported so the resolution-vocabulary consumers (`semantics`) can name
// `Symbol::Builtin`'s payload without each taking a direct `core-defs` dep.
pub use pyaot_core_defs::BuiltinFunctionKind;

// ============================================================================
// Module / function structure
// ============================================================================

/// A whole compilation unit. Module-level code is lowered into a synthetic
/// `__main__` function (named by [`HirModule::main`]) — the one function codegen
/// wraps in the C `main`, and exactly what Phase 8 module-body execution reuses.
#[derive(Debug)]
pub struct HirModule {
    /// All functions, indexed by [`FuncId`]. `__main__` is one of these.
    pub functions: Vec<HirFunction>,
    /// The synthetic module-body function.
    pub main: FuncId,
}

impl HirModule {
    pub fn function(&self, id: FuncId) -> &HirFunction {
        &self.functions[id.index()]
    }

    pub fn function_mut(&mut self, id: FuncId) -> &mut HirFunction {
        &mut self.functions[id.index()]
    }
}

/// A function parameter. The annotation drives the parameter's `Repr` (and hence
/// the ABI). Parameters are also mirrored as the first locals.
#[derive(Debug, Clone)]
pub struct HirParam {
    pub name: InternedString,
    pub ty: SemTy,
}

/// A local slot. Index into [`HirFunction::locals`] is the [`LocalId`].
#[derive(Debug, Clone)]
pub struct HirLocal {
    pub name: InternedString,
    pub ty: SemTy,
    /// Proof-gated representation override (Phase 3c): when `true` **and** the
    /// inferred [`Self::ty`] is `int`, lowering stores this slot as an unboxed
    /// `Raw(I64)` instead of the tagged default. Set only where a range proof
    /// guarantees the value cannot overflow i64 *or* promote to a heap `BigInt`
    /// (a literal-bounded `range()` cursor) — the soundness obligation of
    /// PITFALLS A6/B16. It is **not** a `SemTy` change: the slot stays
    /// semantically `int`. Default `false` (the always-correct tagged baseline).
    pub raw_int_ok: bool,
    /// Pin this slot to the `Tagged` representation regardless of inference. Set
    /// for the local that directly receives an `iter_next` result: that result is
    /// a tagged `Value` that is **null on exhaustion**, so the slot must stay
    /// `Tagged` — inferring it to a typed `Raw(F64)`/`Raw(I8)` (a `float`/`bool`
    /// element iterable) would make the on-exhaustion store an `UnboxFloat` /
    /// `UntagBool` of null (a SIGSEGV). The typed loop variable is a *separate*
    /// local, bound from this one only inside the loop body where it is non-null.
    pub pin_tagged: bool,
}

/// A function: a flat `exprs` arena, a `locals` table, and a CFG of `blocks`.
#[derive(Debug)]
pub struct HirFunction {
    pub name: InternedString,
    pub params: Vec<HirParam>,
    pub ret_ty: SemTy,
    pub locals: Vec<HirLocal>,
    pub blocks: Arena<HirBlock>,
    pub entry: Idx<HirBlock>,
    pub exprs: Arena<HirExpr>,
}

impl HirFunction {
    pub fn local_ty(&self, id: LocalId) -> &SemTy {
        &self.locals[id.index()].ty
    }
}

/// A basic block: a straight-line list of statements ending in exactly one
/// terminator.
#[derive(Debug)]
pub struct HirBlock {
    pub stmts: Vec<HirStmt>,
    pub term: HirTerminator,
}

// ============================================================================
// Expressions
// ============================================================================

/// An expression node. Carries its [`SemTy`] (semantic type **only**) and source
/// [`Span`]. Literal types are set at parse time; everything else starts
/// `SemTy::Dyn` and is refined by `pyaot-typeck`.
#[derive(Debug, Clone)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub ty: SemTy,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum HirExprKind {
    /// A string literal; bytes are interned.
    StrLit(InternedString),
    /// An integer literal that fits a tagged fixnum (`i64`, the codegen tags it).
    IntLit(i64),
    /// An integer literal too large for `i64`; carries the decimal text for
    /// `rt_bigint_from_str`.
    BigIntLit(InternedString),
    FloatLit(f64),
    BoolLit(bool),
    NoneLit,
    /// A name reference, resolved by `pyaot-semantics`.
    Name(SymbolRef),
    /// A direct reference to a local slot. The frontend emits this for reads it
    /// can resolve from its own scope (named locals it has allocated, and the
    /// synthetic result locals produced by desugaring short-circuit `and`/`or`,
    /// ternaries, and chained comparisons). Already resolved, so `semantics`
    /// leaves it untouched.
    Local(LocalId),
    /// A binary arithmetic / bitwise / shift operator (never short-circuiting).
    BinOp {
        op: BinOp,
        l: Idx<HirExpr>,
        r: Idx<HirExpr>,
    },
    /// A unary operator.
    Unary {
        op: UnaryOp,
        operand: Idx<HirExpr>,
    },
    /// A single comparison `l <op> r`. Chained comparisons are desugared by the
    /// frontend into short-circuit branch CFG with single-eval operands.
    Compare {
        op: CmpOp,
        l: Idx<HirExpr>,
        r: Idx<HirExpr>,
    },
    /// A call `callee(args...)`. Callee and args index this function's `exprs`.
    Call {
        callee: Idx<HirExpr>,
        args: Vec<Idx<HirExpr>>,
    },

    // ── containers (Phase 4) ──
    /// A list literal `[e0, e1, …]` (possibly empty).
    ListLit { elems: Vec<Idx<HirExpr>> },
    /// A fixed-arity tuple literal `(e0, e1, …)` (possibly empty).
    TupleLit { elems: Vec<Idx<HirExpr>> },
    /// A set literal `{e0, e1, …}` (never empty — `{}` is a dict).
    SetLit { elems: Vec<Idx<HirExpr>> },
    /// A dict literal `{k0: v0, …}` (possibly empty).
    DictLit { pairs: Vec<(Idx<HirExpr>, Idx<HirExpr>)> },
    /// A bytes literal `b"…"`; the raw bytes are interned like a string literal.
    BytesLit(InternedString),
    /// Subscript read `base[index]`. The runtime dispatch (`rt_list_get` /
    /// `rt_dict_get` / generic `rt_any_getitem`) is selected at lowering from the
    /// `base` representation. Subscript *writes* are [`HirStmt::SetItem`].
    Subscript { base: Idx<HirExpr>, index: Idx<HirExpr> },
    /// A frontend-synthesized container operation (`x in y` → `Contains`; the
    /// for-loop iterator protocol → `Iter`/`IterNext`/`IterExhausted`). Container
    /// *builtins* called by name (`len`/`enumerate`/`zip`/…) instead flow through
    /// [`HirExprKind::Call`] → [`Symbol::Container`] so user shadowing is honored.
    ContainerExpr { op: ContainerOp, args: Vec<Idx<HirExpr>> },
    /// A method call `recv.method(args...)` on a statically-known container
    /// receiver (Phase 4D). Bounded built-in-method dispatch, not Phase-5 vtables.
    /// The method name is resolved to a [`ContainerMethod`] in the frontend; the
    /// concrete runtime op is chosen at lowering from the receiver's type.
    MethodCall {
        recv: Idx<HirExpr>,
        method: ContainerMethod,
        args: Vec<Idx<HirExpr>>,
    },
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

/// Short-circuit boolean operators. Used by the frontend's CFG desugaring; not a
/// standalone expression node (the desugaring produces a result local instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolOp {
    And,
    Or,
}

// ============================================================================
// Container operations (Phase 4)
// ============================================================================

/// A container / iterator operation, carried by [`HirExprKind::ContainerExpr`],
/// [`HirStmt::SetItem`]/[`HirStmt::ContainerPush`], and ultimately the single MIR
/// instruction `CallContainer`. Living here (not `core-defs`) keeps the frozen
/// `BuiltinFunctionKind` untouched while adding the whole container surface.
///
/// Each op has a fixed **argument-representation signature** ([`Self::arg_kinds`])
/// and a **result category** ([`Self::result`]); the MIR verifier enforces both,
/// which is what structurally guarantees uniform tagged element storage (PITFALLS
/// A5: every element/key/value arg is `Tagged`, only index/count/size args are
/// `Raw(I64)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerOp {
    // ── construction (Heap producer; arg is the capacity/size hint) ──
    ListNew,
    DictNew,
    SetNew,
    TupleNew,
    // ── population (mutating; no result) ──
    ListPush,
    ListSet,
    DictSet,
    SetAdd,
    TupleSet,
    // ── indexed read ──
    ListGet,
    DictGet,
    TupleGet,
    BytesGet,
    StrGet,
    AnyGetItem,
    // ── length / membership ──
    Len,
    Contains,
    // ── operators (`+` / `*`) producing a fresh container ──
    // (No `TupleRepeat`: the frozen runtime ships no `rt_tuple_repeat`, so
    // `tuple * int` falls through to the tagged baseline.)
    ListConcat,
    ListRepeat,
    TupleConcat,
    BytesConcat,
    BytesRepeat,
    // ── ordering comparison (`<` / `<=` / `>` / `>=`) on list / tuple ──
    // `==` / `!=` on every container goes through the tagged `rt_obj_eq` baseline;
    // only list / tuple *ordering* needs a typed runtime call (`rt_obj_cmp` raises
    // `TypeError` on them). bytes / str ordering also rides the tagged baseline.
    ListCmp(CmpOp),
    TupleCmp(CmpOp),
    // ── iterator protocol (Phase 4B) ──
    Iter,
    IterNext,
    IterExhausted,
    // ── iteration builtins (Phase 4C) ──
    /// `enumerate(iter, start)` → an iterator of `(index, elem)` pairs. Arg 0 is an
    /// already-`iter()`-wrapped iterator; arg 1 is the `Raw(I64)` start.
    Enumerate,
    /// `zip(iter1, iter2)` → an iterator of pairs (both args pre-wrapped).
    Zip,
    /// `list(iter)` → a fresh list materialized from a pre-wrapped iterator.
    ListFromIter,
    /// `tuple(iter)` → a fresh tuple from a pre-wrapped iterator.
    TupleFromIter,
    /// `dict(pairs)` → a fresh dict from a list of key/value pairs.
    DictFromPairs,
    /// `bytes(list_of_ints)` → a fresh bytes object from a list of ints.
    BytesFromList,
    /// `sorted(list)` → a new sorted list (codegen supplies `reverse=0`, the
    /// list container tag); the input is pre-materialized to a list.
    Sorted,
    /// `reversed(list)` → a reverse iterator over a pre-materialized list.
    Reversed,
    /// `range(start, stop, step)` used as a *value* (not the for-loop fast path) →
    /// a range iterator. All three args are `Raw(I64)` (start/stop/step).
    RangeIter,
    // ── container methods (Phase 4D) ──
    // list
    ListPop,
    ListInsert,
    ListExtend,
    ListIndexOf,
    ListCount,
    ListClear,
    ListCopy,
    ListReverse,
    ListSortMut,
    // dict
    DictGetDefault,
    DictKeys,
    DictValues,
    DictItems,
    DictPopM,
    DictSetdefault,
    DictUpdate,
    DictClear,
    DictCopy,
    // set
    SetRemove,
    SetDiscard,
    SetUpdate,
    SetUnion,
    SetIntersection,
    SetDifference,
    SetCopy,
    SetClear,
}

/// A container method name as resolved by the frontend (which has the interner).
/// The concrete runtime op is selected at lowering from the receiver's type, so
/// shared names (`pop`, `clear`, `copy`, `update`) disambiguate there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerMethod {
    Append,
    Pop,
    Insert,
    Extend,
    Index,
    Count,
    Clear,
    Copy,
    Reverse,
    Sort,
    Get,
    Keys,
    Values,
    Items,
    Setdefault,
    Update,
    Add,
    Remove,
    Discard,
    Union,
    Intersection,
    Difference,
}

impl ContainerMethod {
    /// Resolve a method name to its kind, or `None` if it is not a supported
    /// container method.
    pub fn from_name(name: &str) -> Option<ContainerMethod> {
        Some(match name {
            "append" => ContainerMethod::Append,
            "pop" => ContainerMethod::Pop,
            "insert" => ContainerMethod::Insert,
            "extend" => ContainerMethod::Extend,
            "index" => ContainerMethod::Index,
            "count" => ContainerMethod::Count,
            "clear" => ContainerMethod::Clear,
            "copy" => ContainerMethod::Copy,
            "reverse" => ContainerMethod::Reverse,
            "sort" => ContainerMethod::Sort,
            "get" => ContainerMethod::Get,
            "keys" => ContainerMethod::Keys,
            "values" => ContainerMethod::Values,
            "items" => ContainerMethod::Items,
            "setdefault" => ContainerMethod::Setdefault,
            "update" => ContainerMethod::Update,
            "add" => ContainerMethod::Add,
            "remove" => ContainerMethod::Remove,
            "discard" => ContainerMethod::Discard,
            "union" => ContainerMethod::Union,
            "intersection" => ContainerMethod::Intersection,
            "difference" => ContainerMethod::Difference,
            _ => return None,
        })
    }
}

/// The representation a [`ContainerOp`] argument must have. `Val` is a `Tagged`
/// value (containers, elements, keys, values — uniform tagged storage, A5); `Idx`
/// is an unboxed `Raw(I64)` (an index, count, size, or capacity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerArg {
    Val,
    Idx,
}

/// The result category of a [`ContainerOp`] — drives the `dst` representation the
/// verifier requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerResult {
    /// A `Tagged` value (a fetched element, a reduced value).
    Value,
    /// A `Raw(I64)` integer (`len`, a byte value, an index/count query).
    Int,
    /// A `Raw(I8)` boolean (`in`, a comparison, iterator-exhausted).
    Bool,
    /// A heap object — `dst` must be some `Heap(_)` (container or iterator).
    Heap,
    /// No result; `dst` must be `None` (a mutating op).
    None,
}

impl ContainerOp {
    /// The fixed argument-representation signature (positional).
    pub fn arg_kinds(self) -> &'static [ContainerArg] {
        use ContainerArg::{Idx, Val};
        match self {
            ContainerOp::ListNew
            | ContainerOp::DictNew
            | ContainerOp::SetNew
            | ContainerOp::TupleNew => &[Idx],
            ContainerOp::ListPush | ContainerOp::SetAdd => &[Val, Val],
            ContainerOp::ListSet | ContainerOp::TupleSet => &[Val, Idx, Val],
            ContainerOp::DictSet => &[Val, Val, Val],
            ContainerOp::ListGet
            | ContainerOp::TupleGet
            | ContainerOp::BytesGet
            | ContainerOp::StrGet
            | ContainerOp::AnyGetItem
            | ContainerOp::ListRepeat
            | ContainerOp::BytesRepeat
            | ContainerOp::ListPop => &[Val, Idx],
            // ── method ops (Phase 4D) ──
            ContainerOp::ListExtend
            | ContainerOp::ListIndexOf
            | ContainerOp::ListCount
            | ContainerOp::DictPopM
            | ContainerOp::DictUpdate
            | ContainerOp::SetRemove
            | ContainerOp::SetDiscard
            | ContainerOp::SetUpdate
            | ContainerOp::SetUnion
            | ContainerOp::SetIntersection
            | ContainerOp::SetDifference => &[Val, Val],
            // `list.insert(index, value)` — the index is an unboxed `Raw(I64)`.
            ContainerOp::ListInsert => &[Val, Idx, Val],
            // `dict.get(k[, default])` / `dict.setdefault(k[, default])` — all tagged.
            ContainerOp::DictSetdefault | ContainerOp::DictGetDefault => &[Val, Val, Val],
            ContainerOp::ListClear
            | ContainerOp::ListCopy
            | ContainerOp::ListReverse
            | ContainerOp::ListSortMut
            | ContainerOp::DictKeys
            | ContainerOp::DictValues
            | ContainerOp::DictItems
            | ContainerOp::DictClear
            | ContainerOp::DictCopy
            | ContainerOp::SetCopy
            | ContainerOp::SetClear => &[Val],
            ContainerOp::DictGet
            | ContainerOp::Contains
            | ContainerOp::ListConcat
            | ContainerOp::TupleConcat
            | ContainerOp::BytesConcat
            | ContainerOp::ListCmp(_)
            | ContainerOp::TupleCmp(_)
            | ContainerOp::Zip => &[Val, Val],
            ContainerOp::Enumerate => &[Val, Idx],
            ContainerOp::RangeIter => &[Idx, Idx, Idx],
            ContainerOp::Len
            | ContainerOp::Iter
            | ContainerOp::IterNext
            | ContainerOp::IterExhausted
            | ContainerOp::ListFromIter
            | ContainerOp::TupleFromIter
            | ContainerOp::DictFromPairs
            | ContainerOp::BytesFromList
            | ContainerOp::Sorted
            | ContainerOp::Reversed => &[Val],
        }
    }

    /// The result category (drives the `dst` representation).
    pub fn result(self) -> ContainerResult {
        match self {
            ContainerOp::ListNew
            | ContainerOp::DictNew
            | ContainerOp::SetNew
            | ContainerOp::TupleNew
            | ContainerOp::ListConcat
            | ContainerOp::ListRepeat
            | ContainerOp::TupleConcat
            | ContainerOp::BytesConcat
            | ContainerOp::BytesRepeat
            | ContainerOp::Iter
            | ContainerOp::Enumerate
            | ContainerOp::Zip
            | ContainerOp::ListFromIter
            | ContainerOp::TupleFromIter
            | ContainerOp::DictFromPairs
            | ContainerOp::BytesFromList
            | ContainerOp::Sorted
            | ContainerOp::Reversed
            | ContainerOp::RangeIter
            | ContainerOp::ListCopy
            | ContainerOp::DictKeys
            | ContainerOp::DictValues
            | ContainerOp::DictItems
            | ContainerOp::DictCopy
            | ContainerOp::SetUnion
            | ContainerOp::SetIntersection
            | ContainerOp::SetDifference
            | ContainerOp::SetCopy => ContainerResult::Heap,
            ContainerOp::ListPush
            | ContainerOp::ListSet
            | ContainerOp::DictSet
            | ContainerOp::SetAdd
            | ContainerOp::TupleSet
            | ContainerOp::ListInsert
            | ContainerOp::ListExtend
            | ContainerOp::ListClear
            | ContainerOp::ListReverse
            | ContainerOp::ListSortMut
            | ContainerOp::DictUpdate
            | ContainerOp::DictClear
            | ContainerOp::SetRemove
            | ContainerOp::SetDiscard
            | ContainerOp::SetUpdate
            | ContainerOp::SetClear => ContainerResult::None,
            ContainerOp::ListGet
            | ContainerOp::DictGet
            | ContainerOp::TupleGet
            | ContainerOp::StrGet
            | ContainerOp::AnyGetItem
            | ContainerOp::IterNext
            | ContainerOp::ListPop
            | ContainerOp::DictGetDefault
            | ContainerOp::DictPopM
            | ContainerOp::DictSetdefault => ContainerResult::Value,
            ContainerOp::BytesGet
            | ContainerOp::Len
            | ContainerOp::ListIndexOf
            | ContainerOp::ListCount => ContainerResult::Int,
            ContainerOp::Contains
            | ContainerOp::IterExhausted
            | ContainerOp::ListCmp(_)
            | ContainerOp::TupleCmp(_) => ContainerResult::Bool,
        }
    }

    /// Resolve a built-in *name* to the container op it denotes, for the
    /// `Symbol::Container` resolution path (`len`, the iteration builtins, and the
    /// container constructors). Checked *before* [`BuiltinFunctionKind::from_name`]
    /// so `len` routes through the shared container read path, yet *after* local /
    /// function scope so user shadowing still wins.
    pub fn from_name(name: &str) -> Option<ContainerOp> {
        match name {
            "len" => Some(ContainerOp::Len),
            "enumerate" => Some(ContainerOp::Enumerate),
            "zip" => Some(ContainerOp::Zip),
            "sorted" => Some(ContainerOp::Sorted),
            "reversed" => Some(ContainerOp::Reversed),
            // Constructors over an iterable. Lowering branches on the argument
            // count (`list()` → empty, `list(it)` → materialize). `set` / `sum` /
            // `min` / `max` instead desugar to loops in the frontend, so they are
            // intentionally absent here.
            "list" => Some(ContainerOp::ListFromIter),
            "tuple" => Some(ContainerOp::TupleFromIter),
            "dict" => Some(ContainerOp::DictFromPairs),
            "bytes" => Some(ContainerOp::BytesFromList),
            _ => None,
        }
    }
}

// ============================================================================
// Statements / terminators
// ============================================================================

#[derive(Debug, Clone)]
pub enum HirStmt {
    /// An expression evaluated for its side effects.
    Expr(Idx<HirExpr>),
    /// Assign `value` into a local. Augmented and multiple assignment desugar to
    /// a sequence of these in the frontend.
    Assign {
        target: LocalId,
        value: Idx<HirExpr>,
    },
    /// `assert cond` — the message expression (Phase 7) is dropped here.
    Assert { cond: Idx<HirExpr> },
    /// `print(args, sep=…, end=…)`. `print` is *the* special builtin: `sep`/`end`
    /// are string-literal options that a generic `Call` (no keywords field)
    /// cannot carry, so it gets a dedicated statement. `sep`/`end` are `None` for
    /// the defaults (`' '` between args, `'\n'` trailing); `Some` carries an
    /// interned literal (possibly empty). `typeck` infers each arg's type, and
    /// `lowering` expands this into the `MirInst::Print` sequence with per-arg
    /// `PrintKind` dispatch.
    Print {
        args: Vec<Idx<HirExpr>>,
        sep: Option<InternedString>,
        end: Option<InternedString>,
    },
    /// Subscript write `base[index] = value` (Phase 4A). The runtime dispatch
    /// (`rt_list_set` / `rt_dict_set`) is selected at lowering from the `base`
    /// representation; assigning to a tuple element is a compile error.
    SetItem {
        base: Idx<HirExpr>,
        index: Idx<HirExpr>,
        value: Idx<HirExpr>,
    },
    /// Append `value` to the container local `container` (Phase 4C comprehension
    /// element-push). Lowers to the same `CallContainer{ListPush/SetAdd}` path as a
    /// literal build, so a desugared comprehension never needs user methods.
    ContainerPush { container: LocalId, value: Idx<HirExpr> },
    /// Insert `key: value` into the dict local `container` (Phase 4C dict
    /// comprehension). Lowers to `CallContainer{DictSet}`.
    ContainerInsert {
        container: LocalId,
        key: Idx<HirExpr>,
        value: Idx<HirExpr>,
    },
}

/// How a block ends.
#[derive(Debug, Clone)]
pub enum HirTerminator {
    Return(Option<Idx<HirExpr>>),
    Jump(Idx<HirBlock>),
    Branch {
        cond: Idx<HirExpr>,
        then: Idx<HirBlock>,
        else_: Idx<HirBlock>,
    },
    /// Provably unreachable (e.g. the fall-through of an `assert` fail block).
    Unreachable,
}

// ============================================================================
// Name-resolution vocabulary (shapes; algorithm lives in pyaot-semantics)
// ============================================================================

/// A name occurrence. The parser emits [`SymbolRef::Unresolved`]; `semantics`
/// rewrites it in place to [`SymbolRef::Resolved`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolRef {
    Unresolved(InternedString),
    Resolved(SymbolId),
}

/// A resolved symbol.
///
/// `print` / `range` are **not** first-class builtins
/// (`BuiltinFunctionKind::from_name` returns `None`), so they get their own
/// variants — the honest home for their special-casing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symbol {
    BuiltinPrint,
    BuiltinRange,
    Builtin(BuiltinFunctionKind),
    Local(LocalId),
    Function(FuncId),
    /// A container / iteration builtin (`len`, `enumerate`, `sorted`, the
    /// `list`/`dict`/… constructors). Resolved here instead of as a frozen
    /// `BuiltinFunctionKind` so `core-defs` stays sealed (Phase 4).
    Container(ContainerOp),
}

/// The output of name resolution: a table of [`Symbol`]s indexed by
/// [`SymbolId`]. `semantics` produces it; `typeck` and `lowering` consume it.
#[derive(Debug, Default)]
pub struct ResolveResult {
    symbols: Vec<Symbol>,
}

impl ResolveResult {
    pub fn new() -> Self {
        Self { symbols: Vec::new() }
    }

    /// Intern a resolved symbol, returning its [`SymbolId`].
    pub fn intern(&mut self, sym: Symbol) -> SymbolId {
        let id = SymbolId::new(self.symbols.len() as u32);
        self.symbols.push(sym);
        id
    }

    pub fn symbol(&self, id: SymbolId) -> Symbol {
        self.symbols[id.index()]
    }
}
