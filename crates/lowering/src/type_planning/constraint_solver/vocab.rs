//! Constraint vocabulary — the closed set of relations the solver supports.
//!
//! Every constraint either (a) flows information from one key into another
//! via a monotone JOIN, or (b) defers to a pure reducer function that is
//! re-evaluated whenever any of its input keys change. The solver itself is
//! oblivious to the semantics of each variant; the reducer impls live in
//! `solve.rs` and reuse the pure type-inference helpers from `infer.rs`.

use pyaot_hir::{BinOp, Builtin, UnOp};
use pyaot_utils::{ClassId, FuncId, InternedString};

use super::key::TypeKey;

/// Builtin function identifier — a thin wrapper around the HIR
/// [`Builtin`] enum so the production `ReducerCtx` impl can dispatch to
/// the existing `resolve_builtin_with_overrides` resolver. The wrapper
/// (instead of a bare `Builtin`) exists for future-proofing — if we
/// later need to attach per-call metadata (overload selection, version
/// info, …) it goes here without changing every collector emit site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinId(pub Builtin);

/// Container literal kind for the `ContainerLiteral` constraint. Discriminates
/// between the shapes the solver needs to handle directly; tuple_var is
/// emitted via `tuple_var_of` from a single repeated element constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainerKind {
    List,
    Set,
    Dict,
    Tuple,
}

/// Callee discriminator for `Constraint::Call`.
///
/// `Func` and `ClassCtor` are statically resolved at constraint-collection
/// time. `Builtin` defers to the builtin table. `Dynamic` carries a key
/// whose env value is the callee's `Type` (function, callable instance,
/// or `Any`) — re-evaluated whenever that key's type changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalleeRef {
    Func(FuncId),
    Builtin(BuiltinId),
    Dynamic(TypeKey),
    ClassCtor(ClassId),
}

/// The closed set of constraints the solver supports.
///
/// `Concrete` / `FlowsInto` / `Equal` / `Return` / `Yield` / `FieldWrite` /
/// `Capture` / `LambdaParamHint` are pure JOIN-into-dst: their evaluation
/// reads zero env state and writes a single key. The remaining variants are
/// **deferred reducers** that re-read their input keys from env when
/// re-evaluated, and whose result is JOIN'd into a single destination key.
#[derive(Debug, Clone)]
pub enum Constraint {
    /// `env[key] := env[key].join(ty)`. Used for literals, annotations, and
    /// anywhere the type is statically known at collection time.
    Concrete(TypeKey, pyaot_types::Type),

    /// `env[dst] := env[dst].join(env[src])`. The fundamental dataflow edge;
    /// monotone by construction (`Type::join` is monotone in both arguments).
    FlowsInto { src: TypeKey, dst: TypeKey },

    /// Two-way `FlowsInto`. The collector expands equality into two
    /// `FlowsInto` edges directly, so this variant is never constructed in
    /// production — but `evaluate`/`inputs_of` still handle it defensively
    /// (and `Solver::add_equal` constructs it in unit tests), so it remains
    /// part of the vocabulary.
    #[allow(dead_code)] // reserved vocab; only constructed via test-only `add_equal`
    Equal(TypeKey, TypeKey),

    /// Binary operator. Re-reduced via `infer::binop_result_type` whenever
    /// either operand's type changes.
    BinOp {
        result: TypeKey,
        op: BinOp,
        lhs: TypeKey,
        rhs: TypeKey,
    },

    /// Unary operator. Re-reduced via `infer::unop_result_type`.
    UnaryOp {
        result: TypeKey,
        op: UnOp,
        operand: TypeKey,
    },

    /// Generic call site. The callee discriminator picks the reducer
    /// (function-return lookup, builtin table, class constructor, or
    /// dynamic dispatch). Keyword args carry their parameter name so the
    /// reducer can match them against the callee's signature.
    Call {
        result: TypeKey,
        callee: CalleeRef,
        args: Vec<TypeKey>,
        kwargs: Vec<(InternedString, TypeKey)>,
    },

    /// `recv.name(args...)` — dispatched through `infer::method_call_result_type`.
    MethodCall {
        result: TypeKey,
        recv: TypeKey,
        name: InternedString,
        args: Vec<TypeKey>,
    },

    /// `recv.name` attribute read — dispatched through
    /// `infer::attribute_result_type`. Handles class fields, methods,
    /// properties, and module-level attributes uniformly.
    Attribute {
        result: TypeKey,
        recv: TypeKey,
        name: InternedString,
    },

    /// `recv[index]` — dispatched through `infer::index_result_type`.
    Subscript {
        result: TypeKey,
        recv: TypeKey,
        index: TypeKey,
    },

    /// Container literal `[a, b, c]` / `{a, b}` / `{k: v, ...}` / `(a, b, c)`.
    /// `elems` is used for List/Set/Tuple; `kv` is used for Dict.
    /// (The collector emits one or the other, never both populated.)
    ContainerLiteral {
        result: TypeKey,
        kind: ContainerKind,
        elems: Vec<TypeKey>,
        kv: Vec<(TypeKey, TypeKey)>,
    },

    /// Iterator-element extraction: `for x in iter` → element type of iter.
    /// Re-reduced via `closure_scan::extract_iterable_element_type`.
    IterElem { result: TypeKey, iter: TypeKey },

    /// Project position `index` out of a tuple-typed key. Used to
    /// destructure a loop element: `for a, b in pairs` binds `a` to
    /// position 0 and `b` to position 1 of `pairs`'s element tuple.
    /// Fixed tuples (`tuple[A, B]`) return the per-position element;
    /// variable tuples (`tuple[T, ...]`) return the homogeneous `T` for
    /// any index. Defers (`Never`) until the tuple type resolves.
    TupleProject {
        result: TypeKey,
        tuple: TypeKey,
        index: usize,
    },

    /// A store into a class field — joins the value's type into
    /// `ClassField(class, name)`. Drives cross-instance field refinement
    /// without a separate side-table.
    FieldWrite {
        class: ClassId,
        name: InternedString,
        value: TypeKey,
    },

    /// A call-site type hint for a lambda parameter — joins into
    /// `LambdaParam(func, ix)`. The hint key is the argument's type at the
    /// call site.
    LambdaParamHint {
        func: FuncId,
        param_ix: usize,
        hint: TypeKey,
    },

    /// A captured upvalue inside a closure — joins the captured local's
    /// type into `Capture(func, slot)`.
    Capture {
        func: FuncId,
        slot: usize,
        src: TypeKey,
    },

    /// A return statement inside `func` — joins `value` into
    /// `FuncReturn(func)`.
    Return { func: FuncId, value: TypeKey },

    /// A yield statement inside generator `func` — joins `value` into
    /// `FuncYield(func)`.
    Yield { func: FuncId, value: TypeKey },

    /// `result := Iterator(env[elem])`. Wraps an element-type key in an
    /// `Iterator`. Used to give `map(f, xs)` the precise result type
    /// `Iterator[FuncReturn(f)]` — the generic builtin reducer can only
    /// see `f`'s value-type (`Any`), so without this the map result is
    /// `Iterator[Any]` and a consumer (`filter(..., map(f, xs))`, or a
    /// `for` loop) infers `Any` elements, forcing tagged ops where the
    /// callback body emitted raw ones.
    WrapIterator { result: TypeKey, elem: TypeKey },

    /// Derives an unannotated generator's return type during solving:
    /// `FuncReturn(func) := Iterator(FuncYield(func))`. Emitted once per
    /// unannotated generator. Without it the `Iterator(yield)` wrapping
    /// happens only at materialize/apply time, so a CALLER of the
    /// generator (e.g. a genexp iterating `range_gen(1, 5)`) reads
    /// `FuncReturn = Never` during solving and infers `Iterator[Never]`
    /// for itself. Re-reduced via the worklist whenever `FuncYield(func)`
    /// sharpens.
    GeneratorReturn { func: FuncId },
}

/// Stable identifier for a constraint inside the solver's storage. Used as
/// the value type of the dependents map (`TypeKey → IndexSet<ConstraintId>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConstraintId(pub u32);
