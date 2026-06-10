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

use std::collections::HashMap;

use la_arena::{Arena, Idx};

use pyaot_types::SemTy;
use pyaot_utils::{ClassId, FuncId, InternedString, LocalId, Span, SymbolId};

// Re-exported so the resolution-vocabulary consumers (`semantics`) can name
// `Symbol::Builtin`'s payload without each taking a direct `core-defs` dep.
pub use pyaot_core_defs::BuiltinFunctionKind;
// Re-exported so `semantics`/`lowering` can name an exception class's builtin
// base (Phase 7C) without a direct `core-defs` dep.
pub use pyaot_core_defs::BuiltinExceptionKind;

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
    /// User-defined classes (Phase 5). Methods are ordinary [`HirFunction`]s in
    /// `functions`; an [`HirClass`] records their FuncIds plus its raw shape (base
    /// names + class-level annotations). The resolved [`ClassTable`] (MRO, slot
    /// layout, field/method tables) is computed by `semantics`, not stored here.
    pub classes: Vec<HirClass>,
    /// The synthetic module-body function.
    pub main: FuncId,
    /// Generator resume functions (Phase 6E), indexed by dense `gen_id`: the
    /// `g.<resume>(gen) -> Value` state machine the dispatcher tail-calls. A
    /// generator's wrapper carries the same `gen_id` in its `MakeGenerator`.
    pub generators: Vec<FuncId>,
    /// Module-level annotated promoted globals (Phase 8): `var_id → declared
    /// SemTy`. A module-level `name: T = …` declares the slot's type as a
    /// contract, so `typeck` keeps it even when a *function* writes the slot
    /// (which would otherwise demote it to `Dyn`). Globals are physically tagged
    /// storage; this only refines how a `GlobalGet` result is typed downstream.
    pub global_annotations: HashMap<u32, SemTy>,
}

impl HirModule {
    pub fn function(&self, id: FuncId) -> &HirFunction {
        &self.functions[id.index()]
    }

    pub fn function_mut(&mut self, id: FuncId) -> &mut HirFunction {
        &mut self.functions[id.index()]
    }
}

/// A whole multi-module compilation unit (Phase 8).
///
/// Every imported module is lowered into the SAME shared [`HirModule`] — one
/// global `FuncId` / `ClassId` / `gen_id` / promoted-var-slot space, no merge or
/// remap pass. `namespaces` records the per-module name-resolution scopes so two
/// modules may define the same `add`/`Animal` without colliding. A single-file
/// program is the degenerate case: one namespace, no imports.
#[derive(Debug)]
pub struct HirProgram {
    pub module: HirModule,
    pub namespaces: NamespaceTable,
}

/// Per-module name-resolution scopes (Phase 8). Resolution of a `Name` inside a
/// function uses the function's owning namespace (`func_ns[fid]`): its own
/// module's functions/classes plus that module's imported bindings.
#[derive(Debug, Default)]
pub struct NamespaceTable {
    /// Owning namespace id per `FuncId` (parallel to [`HirModule::functions`]).
    pub func_ns: Vec<u32>,
    /// Owning namespace id per user `ClassId`.
    pub class_ns: HashMap<ClassId, u32>,
    /// Imported name bindings, indexed by namespace id.
    pub imports: Vec<NamespaceImports>,
}

impl NamespaceTable {
    /// The single-file degenerate case: one namespace, every function/class in
    /// it, no imports.
    pub fn single(num_funcs: usize) -> Self {
        NamespaceTable {
            func_ns: vec![0; num_funcs],
            class_ns: HashMap::new(),
            imports: vec![NamespaceImports::default()],
        }
    }
}

/// One module's imported name bindings (Phase 8): a name bound by `from M import
/// f`/`Cls`, or a module-init call target, resolves through here in addition to
/// the module's own definitions.
#[derive(Debug, Default, Clone)]
pub struct NamespaceImports {
    pub funcs: HashMap<InternedString, FuncId>,
    pub classes: HashMap<InternedString, ClassId>,
}

/// A function parameter. The annotation drives the parameter's `Repr` (and hence
/// the ABI). Parameters are also mirrored as the first locals.
#[derive(Debug, Clone)]
pub struct HirParam {
    pub name: InternedString,
    pub ty: SemTy,
    /// Constant default value (Phase 6C; immutable literals only, the
    /// [`ClassAttrInit`] shape). Direct call sites fill missing trailing args
    /// from it; indirect calls require full declared arity.
    pub default: Option<ClassAttrInit>,
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
    /// This slot holds a cell whose contents another function may WRITE (a
    /// descendant's `nonlocal`, or this function's own `nonlocal` capture) —
    /// Phase 6B. `typeck` must type its `CellGet` as `Dyn` instead of the join
    /// of this function's writes, because cross-function writes are invisible
    /// to per-function inference (a precise join would be an unsound unbox
    /// hint, PITFALLS A2).
    pub cell_shared: bool,
}

/// A function: a flat `exprs` arena, a `locals` table, and a CFG of `blocks`.
///
/// There is deliberately NO `is_closure` flag: a nested function's environment
/// is just its explicit param 0 (`__env__: Dyn`), so the ABI stays a pure
/// function of parameter `Repr`s (Phase 6A / Invariant 3).
#[derive(Debug)]
pub struct HirFunction {
    pub name: InternedString,
    pub params: Vec<HirParam>,
    /// The trailing `*args` param (one `tuple[Dyn, ...]` slot) is present (6C).
    pub varargs: bool,
    /// The trailing `**kwargs` param (one `dict[str, Dyn]` slot) is present (6C).
    pub kwargs: bool,
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
    /// Slice read `base[start:end:step]` (Phase 8E). Each bound is optional; an
    /// absent bound takes the runtime's `i64::MIN`/`i64::MAX`/`1` default. The
    /// result has the same kind as `base` (list→list, str→str, …); a `Dyn` base
    /// routes to the runtime-dispatched `rt_obj_slice`.
    Slice {
        base: Idx<HirExpr>,
        start: Option<Idx<HirExpr>>,
        end: Option<Idx<HirExpr>>,
        step: Option<Idx<HirExpr>>,
    },
    /// An f-string field with a (static) format spec — `f"{x:.4f}"` (Phase 8E).
    /// Lowers to `rt_format(value, spec)`; the result is always a `str`. Any
    /// `!s`/`!r` conversion is already applied to `value` by the frontend.
    FormatValue { value: Idx<HirExpr>, spec: InternedString },
    /// A frontend-synthesized container operation (`x in y` → `Contains`; the
    /// for-loop iterator protocol → `Iter`/`IterNext`/`IterExhausted`). Container
    /// *builtins* called by name (`len`/`enumerate`/`zip`/…) instead flow through
    /// [`HirExprKind::Call`] → [`Symbol::Container`] so user shadowing is honored.
    ContainerExpr { op: ContainerOp, args: Vec<Idx<HirExpr>> },
    /// A method call `recv.method(args...)`. The frontend carries the interned
    /// method *name* (no early rejection of unknown names — Phase 5); lowering
    /// dispatches by the receiver's static type: a container receiver resolves the
    /// name to a [`ContainerMethod`] (the Phase-4D path), a class receiver resolves
    /// it to the method's `FuncId` via the [`ClassTable`] (a devirtualized direct
    /// call, or a `CallVirtual` when polymorphic — Phase 5B).
    MethodCall {
        recv: Idx<HirExpr>,
        method_name: InternedString,
        args: Vec<Idx<HirExpr>>,
    },
    /// Attribute read `value.name` (Phase 5). The slot is resolved at lowering
    /// from the receiver's class via the [`ClassTable`]; a `@property` getter
    /// becomes a method call (Phase 5D). Attribute *writes* are [`HirStmt::SetAttr`].
    Attribute { value: Idx<HirExpr>, name: InternedString },
    /// `super()` evaluated inside a method of the carried class (Phase 5B). Only
    /// ever the receiver of a [`Self::MethodCall`]; resolved at lowering against the
    /// enclosing class's MRO to a direct `Call` with the current `self`.
    Super(ClassId),
    /// `isinstance(value, Cls)` (Phase 5B) → `Bool`. The class is resolved by the
    /// frontend; lowering emits the inheritance-aware runtime check.
    IsInstance { value: Idx<HirExpr>, class_id: ClassId },
    /// `isinstance(value, str|int|float|bool)` (Phase 8B) → `Bool`. Folded
    /// statically at lowering from `value`'s inferred `SemTy` — a `Dyn` value is
    /// a loud compile error (a runtime tag check is out of scope).
    IsInstanceBuiltin { value: Idx<HirExpr>, target: SemTy },
    /// `value is None` (Phase 8D) → `Bool`, via `rt_is_none` — the identity test
    /// that recognizes both the immediate `None` tag and a heap `None` object
    /// (which `==` does not). `value is not None` is `Unary{Not, IsNone}`.
    IsNone { value: Idx<HirExpr> },
    /// A call to a frozen-runtime stdlib function/attr/field through its
    /// declarative descriptor (Phase 8B). `args` is positionally aligned with
    /// the descriptor's params: the frontend's call adaptation fills optional
    /// params that carry a `ConstValue` default with literal exprs; an optional
    /// param with NO default stays `None` and lowers to a null-pointer `Value`
    /// (the runtime's "absent object" sentinel). `provided` is the user-written
    /// arg count, appended as a trailing raw i64 when the descriptor's hints say
    /// `pass_arg_count`.
    CallRuntime {
        target: RuntimeCallTarget,
        args: Vec<Option<Idx<HirExpr>>>,
        provided: u32,
    },
    /// A subscripted generic construction `Stack[int](args)` (Phase 5E). Lowers
    /// identically to `Stack(args)` (type args are erased at repr — every
    /// instantiation shares one physical layout); the `type_args` only refine the
    /// static type to `SemTy::Generic{base, args}` for precise field/method
    /// substitution in `typeck`.
    GenericConstruct {
        class_id: ClassId,
        type_args: Vec<SemTy>,
        args: Vec<Idx<HirExpr>>,
    },

    // ── closures (Phase 6A) ──
    /// Build a closure value over `func` (Phase 6A): an env tuple of `1+N` slots
    /// — slot 0 the int-tagged code address, slots `1..=N` the `captures` (each a
    /// direct read of a cell-holding local; always tagged cell pointers, never
    /// raw values — the P6-2 cell rule). `func`'s compiled signature has the env
    /// as explicit param 0, so the ABI stays a pure function of param `Repr`s.
    MakeClosure {
        func: FuncId,
        captures: Vec<Idx<HirExpr>>,
    },
    /// Allocate a fresh cell (`rt_make_cell_ptr`) holding `init` (or `None`).
    /// One per celled variable per *function activation*, emitted in the owner's
    /// entry block — this is what gives CPython late-binding/cell-identity
    /// semantics (P6-2).
    MakeCell {
        init: Option<Idx<HirExpr>>,
    },
    /// Read the current value of the cell held in local `cell`.
    CellGet { cell: LocalId },
    /// Read promoted module-global slot `var_id` (Phase 6B) — GC-rooted uniform
    /// tagged storage (`rt_global_get_ptr`).
    GlobalGet { var_id: u32 },

    // ── generators (Phase 6E) ──
    /// Build a generator object (the wrapper's body) — `rt_make_generator`.
    MakeGenerator { gen_id: u32, num_locals: u32 },
    /// A generator state-machine query carrying its generator operand (P6-3:
    /// all values crossing a `GenOp` are `Tagged`, structurally enforced). The
    /// `slot`/`state` immediate rides alongside (`GetLocal`), and `value` is the
    /// sent value (`Send`); other ops ignore both.
    GenQuery { op: GenOp, gen: Idx<HirExpr>, imm: u32, value: Option<Idx<HirExpr>> },

    // ── exceptions (Phase 7) ──
    /// A query against the thread-local exception state (Phase 7A). Only ever
    /// emitted by the frontend's `try`/`with` desugar, inside handler blocks
    /// where an exception is pending.
    ExcQuery(ExcQuery),
    /// `str(e)` / `print(e)` of a caught exception instance (Phase 7B) —
    /// `rt_exc_instance_str(value)` → the message `StrObj`.
    ExcInstanceStr { value: Idx<HirExpr> },
}

// ============================================================================
// Stdlib runtime calls (Phase 8B)
// ============================================================================

/// What a [`HirExprKind::CallRuntime`] targets — a `&'static` descriptor from
/// the frozen `stdlib-defs` substrate. The descriptor is the single source of
/// truth across the pipeline: the frontend adapts the Python-level call against
/// its `params`, typeck types the result from its `TypeSpec`s, lowering derives
/// per-arg `Repr`s from `(TypeSpec, ParamType)`, and codegen builds the
/// Cranelift signature from its `codegen: RuntimeFuncDef`.
#[derive(Clone, Copy)]
pub enum RuntimeCallTarget {
    /// A module-level function (`math.sqrt`, `random.seed`).
    Func(&'static pyaot_stdlib_defs::StdlibFunctionDef),
    /// A module attribute read (`sys.argv`) — a zero/fixed-arg getter.
    Attr(&'static pyaot_stdlib_defs::StdlibAttrDef),
    /// A runtime-object field read (`t.tm_year`) — receiver is arg 0, plus the
    /// descriptor's constant `field_index` when present.
    Field(&'static pyaot_stdlib_defs::object_types::ObjectFieldDef),
}

impl RuntimeCallTarget {
    /// The codegen descriptor (symbol + Cranelift ABI) for this target.
    pub fn codegen(&self) -> &'static pyaot_core_defs::RuntimeFuncDef {
        match self {
            RuntimeCallTarget::Func(f) => &f.codegen,
            RuntimeCallTarget::Attr(a) => &a.codegen,
            RuntimeCallTarget::Field(f) => &f.codegen,
        }
    }

    /// The semantic result type, via [`semty_from_typespec`].
    pub fn result_semty(&self) -> SemTy {
        match self {
            RuntimeCallTarget::Func(f) => semty_from_typespec(&f.return_type),
            RuntimeCallTarget::Attr(a) => semty_from_typespec(&a.ty),
            RuntimeCallTarget::Field(f) => semty_from_typespec(&f.field_type),
        }
    }
}

impl std::fmt::Debug for RuntimeCallTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeCallTarget::Func(d) => write!(f, "Func({})", d.runtime_name),
            RuntimeCallTarget::Attr(d) => write!(f, "Attr({})", d.runtime_getter),
            RuntimeCallTarget::Field(d) => write!(f, "Field({})", d.runtime_getter),
        }
    }
}

/// Map a declarative stdlib [`TypeSpec`](pyaot_stdlib_defs::TypeSpec) to a
/// semantic type. Lives in `hir` so frontend / typeck / lowering share one
/// mapping while `types` stays stdlib-free. Object specs map to
/// `SemTy::RuntimeObject` with their `TypeTagKind`; `Any` and `Optional` are
/// gradual (`Dyn`) — always-correct `Tagged`.
pub fn semty_from_typespec(spec: &pyaot_stdlib_defs::TypeSpec) -> SemTy {
    use pyaot_core_defs::TypeTagKind;
    use pyaot_stdlib_defs::TypeSpec;
    match spec {
        TypeSpec::Int => SemTy::Int,
        TypeSpec::Float => SemTy::Float,
        TypeSpec::Bool => SemTy::Bool,
        TypeSpec::Str => SemTy::Str,
        TypeSpec::Bytes => SemTy::Bytes,
        TypeSpec::None => SemTy::NoneTy,
        TypeSpec::List(elem) => SemTy::list_of(semty_from_typespec(elem)),
        TypeSpec::Set(elem) => SemTy::set_of(semty_from_typespec(elem)),
        // A stdlib `Tuple(T)` is homogeneous but of gradual length (`os.path.
        // split` is a 2-tuple, `Match.span` a 2-tuple, `urlretrieve` a 2-tuple),
        // so it stays gradual (`Dyn`) — this lets a precise fixed-arity
        // annotation (`tuple[str, str]`) accept it through the gradual contract
        // exemption rather than tripping the var-vs-fixed tuple shape check.
        TypeSpec::Tuple(_) => SemTy::Dyn,
        TypeSpec::Dict(k, v) => SemTy::dict_of(semty_from_typespec(k), semty_from_typespec(v)),
        TypeSpec::Iterator(elem) => SemTy::Iterator(Box::new(semty_from_typespec(elem))),
        // `Optional[T]` narrows to `T` for static dispatch (Phase 8C): a stdlib
        // function declared `Optional[Match]` / `Optional[str]` (`re.search`,
        // `Match.group`) is used as the inner type so its methods resolve. The
        // None possibility is a gradual-typing simplification — the frozen
        // runtime accepts a null receiver (returns None / -1), matching
        // CPython's AttributeError-on-None failure mode rather than masking it.
        TypeSpec::Optional(inner) => semty_from_typespec(inner),
        TypeSpec::Any => SemTy::Dyn,
        TypeSpec::File => SemTy::File { binary: false },
        TypeSpec::Match => SemTy::RuntimeObject(TypeTagKind::Match),
        TypeSpec::StructTime => SemTy::RuntimeObject(TypeTagKind::StructTime),
        TypeSpec::CompletedProcess => SemTy::RuntimeObject(TypeTagKind::CompletedProcess),
        TypeSpec::ParseResult => SemTy::RuntimeObject(TypeTagKind::ParseResult),
        TypeSpec::HttpResponse => SemTy::RuntimeObject(TypeTagKind::HttpResponse),
        TypeSpec::Request => SemTy::RuntimeObject(TypeTagKind::Request),
        TypeSpec::Hash => SemTy::RuntimeObject(TypeTagKind::Hash),
        TypeSpec::StringIO => SemTy::RuntimeObject(TypeTagKind::StringIO),
        TypeSpec::BytesIO => SemTy::RuntimeObject(TypeTagKind::BytesIO),
        TypeSpec::Deque => SemTy::RuntimeObject(TypeTagKind::Deque),
        TypeSpec::Counter => SemTy::RuntimeObject(TypeTagKind::Counter),
    }
}

// ============================================================================
// Exceptions (Phase 7)
// ============================================================================

/// An exception-frame bookkeeping op (Phase 7A), emitted only by the frontend's
/// `try`/`with`/`finally` desugar. Each maps 1:1 to a runtime call:
/// `rt_exc_pop_frame` / `rt_exc_start_handling` / `rt_exc_end_handling`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExcOp {
    /// Normal exit from a protected region pops its frame exactly once.
    /// Handler entry never pops — `dispatch_to_handler` already did.
    PopFrame,
    /// Handler entry: move the current exception into "handling" state (so a
    /// nested raise chains it as `__context__`).
    StartHandling,
    /// Normal handler exit: clear the handled exception.
    EndHandling,
}

/// A query against the current exception (Phase 7A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExcQuery {
    /// The current exception as a heap instance — `rt_exc_get_current()` →
    /// a `Tagged` value (B5: rooted like any tagged slot).
    Current,
    /// Does the current exception match builtin tag? — `rt_exc_isinstance`.
    /// Knows BaseException-catches-all and the Exception/SystemExit split.
    MatchesBuiltin(u8),
    /// Does it match user exception class `cid` (walking the registered class
    /// hierarchy)? — `rt_exc_isinstance_class`.
    MatchesClass(ClassId),
}

/// A `raise` statement's resolved shape (Phase 7A/7C). Builtin-exception name
/// resolution is frontend-local (`class_map` first, then
/// `exception_name_to_tag`); custom-class construction details (own `__init__`
/// or not) are resolved at lowering via the `ClassTable`.
#[derive(Debug, Clone)]
pub enum HirRaise {
    /// `raise ValueError("msg")` / `raise ValueError`.
    Builtin { tag: u8, msg: Option<Idx<HirExpr>> },
    /// `raise X("m") from Y("c")` — both builtin.
    BuiltinFrom {
        tag: u8,
        msg: Option<Idx<HirExpr>>,
        cause_tag: u8,
        cause_msg: Option<Idx<HirExpr>>,
    },
    /// `raise X("m") from None`.
    BuiltinFromNone { tag: u8, msg: Option<Idx<HirExpr>> },
    /// `raise MyError(args…)` for a user exception class. Lowering constructs
    /// the instance (running `__init__` when the class has one; a single arg
    /// becomes the message operand otherwise so `str(e)` works).
    Custom { class_id: ClassId, args: Vec<Idx<HirExpr>> },
    /// `raise HTTPError(args…)` for a stdlib exception (Phase 8D). `exc_type_tag`
    /// is the builtin parent (`OSError`) so `except OSError` matches by tag;
    /// `class_id` is the reserved stdlib id so `except HTTPError` matches by id.
    /// `msg` is the first positional arg (its message); remaining args are
    /// ignored (the corpus never inspects the message).
    Stdlib { class_id: u8, exc_type_tag: u8, msg: Option<Idx<HirExpr>> },
    /// `raise e` — re-raise a caught exception instance value.
    Instance { value: Idx<HirExpr> },
    /// Bare `raise` — re-raise the exception being handled.
    Reraise,
}

/// A generator state-machine operation (Phase 6E) — the runtime-backed surface
/// of the desugared state machine. Each op has a fixed argument/result
/// representation the MIR verifier enforces (P6-3: tagged slot storage).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenOp {
    /// Read generator slot `imm` — `rt_generator_get_local_ptr` → tagged Value.
    GetLocal,
    /// Write generator slot `imm` — `rt_generator_set_local_ptr` (Tagged value).
    SetLocal,
    /// Current state — `rt_generator_get_state` → an `Int`.
    GetState,
    /// Set the state to `imm` — `rt_generator_set_state`.
    SetState,
    /// The value passed to `send()` — `rt_generator_get_sent_value` → Value.
    GetSentValue,
    /// Mark exhausted — `rt_generator_set_exhausted`.
    SetExhausted,
    /// In the `close()` unwind? — `rt_generator_is_closing` → `Bool`.
    IsClosing,
    /// `next(g)` — `rt_generator_next` → the yielded Value.
    Next,
    /// `g.send(v)` — `rt_generator_send` → the yielded Value.
    Send,
    /// `g.close()` — `rt_generator_close` (no result).
    Close,
}

impl GenOp {
    /// The result category (drives the `dst` representation), or `None` for a
    /// mutating op.
    pub fn result(self) -> GenResult {
        match self {
            GenOp::GetLocal | GenOp::GetSentValue | GenOp::Next | GenOp::Send => GenResult::Value,
            GenOp::GetState => GenResult::Int,
            GenOp::IsClosing => GenResult::Bool,
            GenOp::SetLocal | GenOp::SetState | GenOp::SetExhausted | GenOp::Close => {
                GenResult::None
            }
        }
    }

    /// True iff this op takes the `imm` immediate (slot index / state number).
    pub fn uses_imm(self) -> bool {
        matches!(self, GenOp::GetLocal | GenOp::SetLocal | GenOp::SetState)
    }

    /// True iff this op takes a stored value operand (`SetLocal` / `Send`).
    pub fn takes_value(self) -> bool {
        matches!(self, GenOp::SetLocal | GenOp::Send)
    }
}

/// The result category of a [`GenOp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenResult {
    /// A `Tagged` value (a slot read, the sent value).
    Value,
    /// A `Raw(I64)` integer (the state).
    Int,
    /// A `Raw(I8)` boolean (`is_closing`).
    Bool,
    /// No result (a mutating op).
    None,
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
    /// Attribute write `base.name = value` (Phase 5). The slot is resolved at
    /// lowering from `base`'s class; the value coerces to the uniform tagged field
    /// slot (the A5 storage rule). A `@property` setter becomes a method call (5D).
    SetAttr {
        base: Idx<HirExpr>,
        name: InternedString,
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
    /// Store `value` into the cell held in local `cell` (Phase 6A). Assignments
    /// to a celled variable rewrite to this; the cell local itself is written
    /// exactly once (the entry-block `MakeCell`).
    CellSet {
        cell: LocalId,
        value: Idx<HirExpr>,
    },
    /// Write promoted module-global slot `var_id` (Phase 6B) —
    /// `rt_global_set_ptr` (uniform tagged storage).
    GlobalSet {
        var_id: u32,
        value: Idx<HirExpr>,
    },

    // ── generators (Phase 6E) ──
    /// Write generator slot `slot` from `value` — `GenOp::SetLocal`.
    GenSetLocal { gen: Idx<HirExpr>, slot: u32, value: Idx<HirExpr> },
    /// Set the generator state — `GenOp::SetState`.
    GenSetState { gen: Idx<HirExpr>, state: u32 },
    /// Mark the generator exhausted — `GenOp::SetExhausted`.
    GenSetExhausted { gen: Idx<HirExpr> },

    // ── exceptions (Phase 7) ──
    /// Exception-frame bookkeeping (pop / start-handling / end-handling).
    ExcOp(ExcOp),
    /// `raise …` — must be the last statement of its block, followed by an
    /// [`HirTerminator::Unreachable`] (the AssertFail shape).
    Raise(HirRaise),
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
    /// Enter a protected region (Phase 7A): push an exception frame + `setjmp`.
    /// Falls through to `normal`; a raise inside the region longjmps to
    /// `handler` (with the frame already popped by `dispatch_to_handler`).
    TryEnter {
        normal: Idx<HirBlock>,
        handler: Idx<HirBlock>,
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
    /// A user-defined class name used as a value — almost always a constructor
    /// call `Cls(args)` (Phase 5). Carries the frontend-assigned [`ClassId`].
    Class(ClassId),
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

// ============================================================================
// Classes (Phase 5)
// ============================================================================

/// A user-defined class as the frontend produces it: its identity + raw shape.
/// Methods are ordinary [`HirFunction`]s in [`HirModule::functions`]; this records
/// their FuncIds. The *resolved* layout (MRO, slots, inherited members) is the
/// [`ClassTable`], computed once by `semantics`.
#[derive(Debug, Clone)]
pub struct HirClass {
    /// The bare class name (`Widget`).
    pub name: InternedString,
    /// The CPython qualified name (`__main__.Widget`) for the default repr,
    /// interned by the frontend (the only stage with a mutable interner).
    pub qualname: InternedString,
    /// The frontend-assigned class id (≥ `FIRST_USER_CLASS_ID`).
    pub class_id: ClassId,
    /// Base-class names in declaration order (`class Dog(Animal)` → `[Animal]`).
    pub base_names: Vec<InternedString>,
    /// `(method_name, func_id)` for ordinary instance methods defined directly on
    /// this class (`__init__`, `area`, dunders, …). These get vtable slots + virtual
    /// dispatch. `@staticmethod`/`@classmethod`/`@property` live separately below.
    pub methods: Vec<(InternedString, FuncId)>,
    /// `@staticmethod`s (no `self`) — called directly (Phase 5D).
    pub static_methods: Vec<(InternedString, FuncId)>,
    /// `@classmethod`s (`cls` is the enclosing class, statically resolved) — Phase 5D.
    pub class_methods: Vec<(InternedString, FuncId)>,
    /// `@property` getters + their `@x.setter`s (Phase 5D).
    pub properties: Vec<HirProperty>,
    /// Class-level value attributes (`count = 0`) — shared across instances (5D).
    pub class_attrs: Vec<HirClassAttr>,
    /// Class-level `name: T` annotations contributing field types (B10/D5).
    pub field_annotations: Vec<(InternedString, SemTy)>,
    /// Declared type parameters (`class Stack[T]` / `Generic[T]`), Phase 5E.
    pub type_params: Vec<InternedString>,
}

/// A `@property`: a getter and an optional `@x.setter` (Phase 5D).
#[derive(Debug, Clone)]
pub struct HirProperty {
    pub name: InternedString,
    pub getter: FuncId,
    pub setter: Option<FuncId>,
    /// The getter's declared return type (the property's value type).
    pub ty: SemTy,
}

/// A class-level value attribute with a constant initializer (Phase 5D).
#[derive(Debug, Clone)]
pub struct HirClassAttr {
    pub name: InternedString,
    pub ty: SemTy,
    pub init: ClassAttrInit,
}

/// A constant class-attribute initializer (`count = 0`, `scale = "c"`). Non-literal
/// initializers are out of scope for Phase 5D.
#[derive(Debug, Clone)]
pub enum ClassAttrInit {
    Int(i64),
    BigInt(InternedString),
    Float(f64),
    Bool(bool),
    Str(InternedString),
    Bytes(InternedString),
    None,
    /// The empty tuple `()` — only valid as a parameter default (Phase 8E),
    /// where it is materialized as a fresh empty `TupleLit` at each direct call
    /// site. Not supported as a class-level attribute (no empty-tuple `Const`).
    EmptyTuple,
}

/// One instance field's resolved layout entry: its name, best-effort static type
/// (D5), and 0-based slot index. The slot is stable across subclasses — a base
/// field keeps its offset in every subclass (parent-first layout, Phase 5B).
#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub name: InternedString,
    pub ty: SemTy,
}

/// One method's resolved entry: its name, the `FuncId` to call, and its vtable
/// slot (stable across the class and its subclasses; Phase 5B).
#[derive(Debug, Clone)]
pub struct MethodInfo {
    pub name: InternedString,
    pub func_id: FuncId,
    pub slot: usize,
}

/// A fully-resolved class: identity, inheritance (parent + C3 MRO), instance-field
/// slot layout, and the method table (own + inherited). Produced by `semantics`
/// after `resolve`; consumed by `typeck` (field/method/return types, the nominal
/// subtyping oracle), `lowering` (slot/FuncId resolution), and `codegen` (the
/// `__pyaot_classinit` registrations).
#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub class_id: ClassId,
    pub name: InternedString,
    /// `__main__.Widget` — the CPython qualified name for the default repr.
    pub qualname: InternedString,
    /// Direct parent (single inheritance fast path); `None` for a root class.
    /// Multiple inheritance still records the full MRO but the runtime parent
    /// chain follows the first base (Phase 5B).
    pub parent: Option<ClassId>,
    /// C3 linearization, `self` first (Phase 5B; `[self]` in 5A).
    pub mro: Vec<ClassId>,
    /// Instance fields ordered by slot index (parent fields first).
    pub fields: Vec<FieldInfo>,
    /// Methods incl. inherited, each with its resolved `FuncId` + vtable slot.
    pub methods: Vec<MethodInfo>,
    /// Methods defined *directly* on this class (own body only) — drives `super()`
    /// resolution and the "overridden below" polymorphism check (Phase 5B).
    pub own_methods: Vec<(InternedString, FuncId)>,
    /// `@staticmethod`s (own + inherited), called directly (Phase 5D).
    pub static_methods: Vec<MethodInfo>,
    /// `@classmethod`s (own + inherited), called directly (Phase 5D).
    pub class_methods: Vec<MethodInfo>,
    /// `@property` definitions (own + inherited), Phase 5D.
    pub properties: Vec<PropertyInfo>,
    /// Class-level attributes (own + inherited) with their assigned slot (Phase 5D).
    pub class_attrs: Vec<ClassAttrInfo>,
    /// Number of vtable slots (max slot + 1 across the class; Phase 5B).
    pub num_vtable_slots: usize,
    /// Declared type parameters (Phase 5E).
    pub type_params: Vec<InternedString>,
    /// The builtin exception this class (transitively) derives from (Phase 7C):
    /// `class MyError(ValueError)` → `Some(ValueError)`, inherited through user
    /// parents. `None` for ordinary (non-exception) classes.
    pub exception_base: Option<BuiltinExceptionKind>,
}

/// A resolved `@property` (Phase 5D).
#[derive(Debug, Clone)]
pub struct PropertyInfo {
    pub name: InternedString,
    pub getter: FuncId,
    pub setter: Option<FuncId>,
    pub ty: SemTy,
}

/// A resolved class-level attribute (Phase 5D): its name, best-effort type, the
/// runtime `attr_idx` slot, and constant initializer.
#[derive(Debug, Clone)]
pub struct ClassAttrInfo {
    pub name: InternedString,
    pub ty: SemTy,
    pub attr_idx: u32,
    pub init: ClassAttrInit,
}

impl ClassInfo {
    /// Slot index of `name` in this class's field layout.
    pub fn field_slot(&self, name: InternedString) -> Option<usize> {
        self.fields.iter().position(|f| f.name == name)
    }
    /// Best-effort static type of field `name`.
    pub fn field_ty(&self, name: InternedString) -> Option<&SemTy> {
        self.fields.iter().find(|f| f.name == name).map(|f| &f.ty)
    }
    /// Resolve method `name` (own or inherited).
    pub fn method(&self, name: InternedString) -> Option<&MethodInfo> {
        self.methods.iter().find(|m| m.name == name)
    }
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }
    /// Resolve a `@staticmethod` by name (Phase 5D).
    pub fn static_method(&self, name: InternedString) -> Option<&MethodInfo> {
        self.static_methods.iter().find(|m| m.name == name)
    }
    /// Resolve a `@classmethod` by name (Phase 5D).
    pub fn class_method(&self, name: InternedString) -> Option<&MethodInfo> {
        self.class_methods.iter().find(|m| m.name == name)
    }
    /// Resolve a `@property` by name (Phase 5D).
    pub fn property(&self, name: InternedString) -> Option<&PropertyInfo> {
        self.properties.iter().find(|p| p.name == name)
    }
    /// Resolve a class-level attribute by name (Phase 5D).
    pub fn class_attr(&self, name: InternedString) -> Option<&ClassAttrInfo> {
        self.class_attrs.iter().find(|a| a.name == name)
    }
    /// True iff this class is a user exception class (Phase 7C).
    pub fn is_exception_class(&self) -> bool {
        self.exception_base.is_some()
    }
}

/// The resolved class table — `ClassId → ClassInfo`. The *shape* lives here (like
/// [`ResolveResult`]); `semantics` fills it.
#[derive(Debug, Default, Clone)]
pub struct ClassTable {
    classes: HashMap<ClassId, ClassInfo>,
}

impl ClassTable {
    pub fn new() -> Self {
        Self { classes: HashMap::new() }
    }
    pub fn insert(&mut self, info: ClassInfo) {
        self.classes.insert(info.class_id, info);
    }
    pub fn get(&self, cid: ClassId) -> Option<&ClassInfo> {
        self.classes.get(&cid)
    }
    pub fn iter(&self) -> impl Iterator<Item = &ClassInfo> {
        self.classes.values()
    }
    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }
    /// True iff `cid` is a user exception class (Phase 7C).
    pub fn is_exception_class(&self, cid: ClassId) -> bool {
        self.get(cid).is_some_and(ClassInfo::is_exception_class)
    }

    /// Nominal subtyping (D8): `a <: b` iff `b` appears in `a`'s MRO. Consulted in
    /// `typeck`, never baked into the sealed `types` lattice.
    pub fn is_subclass(&self, a: ClassId, b: ClassId) -> bool {
        if a == b {
            return true;
        }
        self.classes.get(&a).is_some_and(|info| info.mro.contains(&b))
    }

    /// Resolve `super().name()` from class `cid` (Phase 5B): the first class in
    /// `cid`'s MRO *after* `cid` whose own body defines `name`.
    pub fn resolve_super_method(&self, cid: ClassId, name: InternedString) -> Option<FuncId> {
        let info = self.get(cid)?;
        for ancestor in info.mro.iter().skip(1) {
            if let Some(ac) = self.get(*ancestor) {
                if let Some((_, fid)) = ac.own_methods.iter().find(|(n, _)| *n == name) {
                    return Some(*fid);
                }
            }
        }
        None
    }

    /// Resolve `super().name`'s declared return type (Phase 5B), `None` if unknown.
    pub fn resolve_super_method_info(&self, cid: ClassId, name: InternedString) -> Option<FuncId> {
        self.resolve_super_method(cid, name)
    }

    /// True iff method `name` is overridden in a *proper subclass* of `cid` — i.e.
    /// a receiver statically typed `cid` may dynamically dispatch to a different
    /// body, so it must use virtual dispatch (D7). When false, a `cid`-typed
    /// receiver devirtualizes to `cid`'s resolved method.
    pub fn method_overridden_below(&self, cid: ClassId, name: InternedString) -> bool {
        self.classes.values().any(|d| {
            d.class_id != cid
                && d.mro.contains(&cid)
                && d.own_methods.iter().any(|(n, _)| *n == name)
        })
    }
}
