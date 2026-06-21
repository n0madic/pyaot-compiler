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

use std::collections::{HashMap, HashSet};

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
    /// Module globals a `del name` unbinds (`var_id → name`). The name is kept
    /// for the `NameError` message. Lowering guards every `GlobalGet` of these
    /// slots with `rt_check_bound` (kind=Global). Globals are physically tagged,
    /// so no representation change is needed — only the read-guard.
    pub deletable_globals: HashMap<u32, InternedString>,
    /// Instance-field names a `del obj.attr` unbinds (keyed by name only — the
    /// frontend cannot resolve the receiver's class pre-typeck). Lowering guards
    /// every `obj.<name>` read whose name is in this set with `rt_check_bound`
    /// (kind=Attr). Fields are stored as uniform tagged slots, so the guard runs
    /// on the tagged value before any unbox — no representation change needed.
    pub deletable_fields: HashSet<InternedString>,
    /// Per-instance-method **uniform thunk** FuncIds: `method_FuncId →
    /// thunk_FuncId` (gradual-completeness method dispatch, Phase B). The
    /// frontend builds a thunk `M.m.<uniform>(self, __args__, __kwargs__) →
    /// Value` for each instance method whose name is invoked as a method call
    /// somewhere, so `rt_obj_method` can dispatch an arbitrary user method on a
    /// `Dyn` receiver. Keyed by the method's OWN `FuncId`, so an inherited
    /// method (whose `ClassInfo.methods` entry reuses the base's FuncId)
    /// resolves the base's thunk — lowering registers it under the subclass id.
    pub method_uniform_thunks: HashMap<FuncId, FuncId>,
    /// Per-class **iternext thunk** FuncIds: `next_method_FuncId →
    /// iternext_thunk_FuncId` (lazy user-class iterator protocol). The frontend
    /// builds a thunk `Cls.<iternext>(self) → Value` ≡ `try: return
    /// self.__next__() except StopIteration: return UNBOUND` for each class with
    /// an own `__next__`, so the runtime's `iter_next_instance` can drive
    /// `for x in instance` / `iter()` / `next()`. Keyed by the `__next__`
    /// method's OWN `FuncId`, so an inherited `__next__` (whose `ClassInfo`
    /// entry reuses the base's FuncId) resolves the base's thunk — lowering
    /// registers it under the subclass id.
    pub iternext_thunks: HashMap<FuncId, FuncId>,
    /// Per-class **copy thunk** FuncIds for `__copy__` / `__deepcopy__`:
    /// `dunder_method_FuncId → thunk_FuncId`. The frontend builds a thunk
    /// `Cls.<__copy__>(self) → Value` ≡ `return self.__copy__()` (and the
    /// `__deepcopy__` analogue, passing a fresh memo dict) for each class that
    /// defines either, so `copy.copy` / `copy.deepcopy` dispatch to the user
    /// method. One map holds both dunders — keyed by each method's OWN `FuncId`,
    /// so an inherited dunder resolves the base's thunk (lowering registers it
    /// under the subclass id, like `iternext_thunks`).
    pub copy_thunks: HashMap<FuncId, FuncId>,
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
    /// Default value (Phase 6C). An immutable literal is materialized fresh per
    /// call ([`ParamDefault::Const`]); a mutable/computed default of a top-level
    /// function reads a synthetic GC-rooted global slot evaluated once at
    /// def-time ([`ParamDefault::Slot`], CPython's shared-default semantics).
    /// Direct call sites fill missing trailing args from it; indirect calls
    /// require full declared arity.
    pub default: Option<ParamDefault>,
}

/// A local slot. Index into [`HirFunction::locals`] is the [`LocalId`].
#[derive(Debug, Clone)]
pub struct HirLocal {
    pub name: InternedString,
    pub ty: SemTy,
    /// Proof-gated representation override (Phase 3c): when `true` **and** the
    /// inferred [`Self::ty`] is `int`, lowering stores this slot as an unboxed
    /// `Raw(I64)` instead of the tagged default. Set by typeck's interval pass
    /// ([`crate`]-external) where a range proof guarantees every value written
    /// to the slot cannot overflow i64 *or* promote to a heap `BigInt` (a
    /// literal-bounded `range()` cursor, its induction variable, or any local
    /// whose writers are all provably within `±RAW_I64_NARROW_BOUND`) — the
    /// soundness obligation of PITFALLS A6/B16. Never set for a parameter slot
    /// (its entry value comes from an unbounded caller). It is **not** a `SemTy`
    /// change: the slot stays semantically `int`. The per-expression analogue is
    /// [`HirExpr::raw_int_ok`]. Default `false` (the always-correct tagged
    /// baseline).
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
    /// A `del name` somewhere unbinds this slot. The frontend sets it (together
    /// with [`Self::pin_tagged`], so the slot can hold the `Value::UNBOUND`
    /// immediate regardless of the inferred type), and lowering guards every
    /// read of the slot with `rt_check_bound` → `UnboundLocalError` if it
    /// observes the sentinel. Default `false`.
    pub deletable: bool,
}

/// A function: a flat `exprs` arena, a `locals` table, and a CFG of `blocks`.
///
/// There is deliberately NO `is_closure` flag: a nested function's environment
/// is just its explicit param 0 (`__env__: Dyn`), so the ABI stays a pure
/// function of parameter `Repr`s (Phase 6A / Invariant 3).
#[derive(Debug)]
pub struct HirFunction {
    pub name: InternedString,
    /// Display path of the source file this function was lowered from (real
    /// tracebacks): the entry script's path as given on the command line, or
    /// the loader-resolved path for imported modules.
    pub file: InternedString,
    pub params: Vec<HirParam>,
    /// The trailing `*args` param (one `tuple[Dyn, ...]` slot) is present (6C).
    pub varargs: bool,
    /// The trailing `**kwargs` param (one `dict[str, Dyn]` slot) is present (6C).
    pub kwargs: bool,
    pub ret_ty: SemTy,
    /// Proof-gated representation override for the RETURN value (Phase 3c,
    /// interprocedural): when `true` **and** [`Self::ret_ty`] is `int`, lowering
    /// makes this function's signature return — and every `Return` terminator —
    /// an unboxed `Raw(I64)` instead of the tagged default. Set by typeck's
    /// whole-program interval pass only for a **specializable** function (address
    /// never taken — no `MakeClosure`, generator, or `ClassTable` slot holds its
    /// `FuncId`, so every call site is a direct, resolvable `Call`) whose every
    /// `return` expression provably stays within `±RAW_I64_NARROW_BOUND`. Like
    /// [`HirLocal::raw_int_ok`] it is a representation proof, **not** an
    /// ABI/convention flag: the ABI still derives deterministically from the
    /// return `Repr` (Invariant 3/6), and the caller's `Call.dst` repr follows
    /// the same proof so the verifier sees a consistent `Repr`. Default `false`
    /// (the always-correct tagged baseline).
    pub ret_raw_int: bool,
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
    /// The exception-handler block protecting this block's code, if it is
    /// lexically inside a `try` body / `with` body (table-based unwinding —
    /// the static replacement for the Phase-7 frame stack). A raise anywhere
    /// in this block — directly or from any call — lands at `handler`.
    /// Handler blocks themselves carry the *outer* handler (or `None`), which
    /// is what makes a raise inside an `except` body propagate outward.
    pub handler: Option<Idx<HirBlock>>,
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
    /// Proof-gated (typeck's interval pass): when `true` **and** [`Self::ty`] is
    /// `int`, this expression's value provably stays within
    /// `±RAW_I64_NARROW_BOUND` with no i64 overflow, so lowering MAY produce it
    /// directly as `Raw(I64)` (a raw `Mul`/`Mod`/`FloorDiv`) instead of the
    /// tagged baseline. The interval pass guarantees a **bottom-up closure
    /// invariant**: a flagged `BinOp` has every operand itself flagged-raw, a
    /// fixnum `IntLit` within bound, or a `raw_int_ok` local — so lowering can
    /// request each operand as `Raw(I64)` with no untag of a possibly-bignum
    /// value (closing PITFALLS B16 for the derived-expr path). Mirrors
    /// [`HirLocal::raw_int_ok`]. Default `false` (the always-correct tagged
    /// baseline).
    pub raw_int_ok: bool,
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
    /// The `NotImplemented` singleton (§4a) — the dunder-fallback control-flow
    /// signal. Mirrors [`Self::NoneLit`]: typed [`SemTy::NotImplementedT`] (→
    /// always `Repr::Tagged`), lowered to `rt_not_implemented_singleton()`. The
    /// runtime's forward→reflected→NotImplemented→TypeError protocol consumes
    /// it dynamically (a dunder body `return NotImplemented` produces the real
    /// singleton).
    NotImplementedLit,
    /// `object.__new__(cls)` (§3) — allocate a bare instance of the class whose
    /// id is `cls` (a `cls`-as-int value). Lowers to `rt_object_new(cls as i8)`;
    /// the result is a heap instance ptr typed `Dyn` (`Tagged`). Only ever
    /// appears inside a user `__new__` body.
    ObjectNew {
        cls: Idx<HirExpr>,
    },
    /// The `Value::UNBOUND` sentinel — the value a `del name`/`del obj.attr`
    /// stores into the unbound slot. Lowers to `Const::Unbound`. Typed as
    /// `SemTy::Never` (it is not a real value), so it contributes nothing to any
    /// value-type join. Never read directly; only stored, then caught by the
    /// `rt_check_bound` read-guard on the deletable slot.
    Unbound,
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
    /// A **pre-packed** indirect call through a callable VALUE: `callee` is a
    /// closure value and `args` is an already-built `tuple[Dyn, ...]` of the
    /// positional arguments, with `kwargs` an optional `dict[str, Dyn]` (the null
    /// sentinel when absent). Unlike [`HirExprKind::Call`], the args are NOT
    /// individually slot-matched / packed by lowering — they are handed straight
    /// to the uniform closure ABI (`CallIndirect` over `GENERIC_SIG`). The
    /// frontend emits this only where the positional sequence cannot be expressed
    /// as flat args — a runtime `*seq` spread / `**dict` forward into a value
    /// callee (e.g. a decorator wrapper's `func(*args, **kwargs)`).
    CallValue {
        callee: Idx<HirExpr>,
        args: Idx<HirExpr>,
        kwargs: Option<Idx<HirExpr>>,
    },
    /// A method call with a `*args` / `**kwargs` spread (`recv.m(*xs, **d)`). Like
    /// [`Self::CallValue`] for methods: the frontend has already packed the
    /// positional `args` into a tuple and the keyword `kwargs` into a dict (or
    /// `None`), so lowering routes straight to the DYNAMIC dispatcher
    /// (`rt_obj_method`), which needs no static arity. (A plain / keyword-only
    /// method call keeps the static [`Self::MethodCall`] path and its
    /// devirtualization.) Always yields the tagged baseline (`Dyn`).
    MethodCallValue {
        recv: Idx<HirExpr>,
        method_name: InternedString,
        args: Idx<HirExpr>,
        kwargs: Option<Idx<HirExpr>>,
    },

    // ── containers (Phase 4) ──
    /// A list literal `[e0, e1, …]` (possibly empty).
    ListLit {
        elems: Vec<Idx<HirExpr>>,
    },
    /// A fixed-arity tuple literal `(e0, e1, …)` (possibly empty).
    TupleLit {
        elems: Vec<Idx<HirExpr>>,
    },
    /// A set literal `{e0, e1, …}` (never empty — `{}` is a dict).
    SetLit {
        elems: Vec<Idx<HirExpr>>,
    },
    /// A dict literal `{k0: v0, …}` (possibly empty).
    DictLit {
        pairs: Vec<(Idx<HirExpr>, Idx<HirExpr>)>,
    },
    /// A bytes literal `b"…"`; the raw bytes are interned like a string literal.
    BytesLit(InternedString),
    /// Subscript read `base[index]`. The runtime dispatch (`rt_list_get` /
    /// `rt_dict_get` / generic `rt_any_getitem`) is selected at lowering from the
    /// `base` representation. Subscript *writes* are [`HirStmt::SetItem`].
    Subscript {
        base: Idx<HirExpr>,
        index: Idx<HirExpr>,
    },
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
    /// An f-string field or `format()`/`str.format()` replacement — `f"{x:.4f}"`,
    /// `format(x, spec)`, `"{:.4f}".format(x)` (§5/§9/§13). Lowers to
    /// `rt_format(value, spec)`; the result is always a `str`. The `spec` is a
    /// string-valued expr (a `StrLit` for a static spec, an f-string concat for a
    /// dynamic one like `f"{x:.{n}f}"`). Any `!s`/`!r`/`!a` conversion is already
    /// applied to `value` by the frontend (so `value` may be a `str(...)`/
    /// `repr(...)`/`ascii(...)` call). An empty `spec` routes a class instance to
    /// its `__format__` (CPython `f"{p}"` ≡ `format(p, "")`).
    FormatValue {
        value: Idx<HirExpr>,
        spec: Idx<HirExpr>,
    },
    /// `sum(iterable[, start])` (Phase 8H, D2). Typed by `typeck` (numeric
    /// promotion / inferred `__add__`/`__radd__` dunder returns ride the
    /// fixpoint); EXPANDED by `lowering` into a Tagged-accumulator iterator
    /// loop — never reaches MIR. A generator-expression argument was already
    /// materialized into a list comprehension by the frontend.
    Sum {
        iterable: Idx<HirExpr>,
        start: Option<Idx<HirExpr>>,
    },
    /// A frontend-synthesized container operation (`x in y` → `Contains`; the
    /// for-loop iterator protocol → `Iter`/`IterNext`/`IterExhausted`). Container
    /// *builtins* called by name (`len`/`enumerate`/`zip`/…) instead flow through
    /// [`HirExprKind::Call`] → [`Symbol::Container`] so user shadowing is honored.
    ContainerExpr {
        op: ContainerOp,
        args: Vec<Idx<HirExpr>>,
    },
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
        /// Keyword arguments in WRITTEN order (Phase 10). The frontend has
        /// already STAGED the receiver / argument values into locals when this
        /// is non-empty, so consumers may map names to parameter slots freely
        /// (via [`match_keywords`]) without reordering side effects.
        kwargs: Vec<(InternedString, Idx<HirExpr>)>,
    },
    /// Attribute read `value.name` (Phase 5). The slot is resolved at lowering
    /// from the receiver's class via the [`ClassTable`]; a `@property` getter
    /// becomes a method call (Phase 5D). Attribute *writes* are [`HirStmt::SetAttr`].
    Attribute {
        value: Idx<HirExpr>,
        name: InternedString,
    },
    /// `super()` evaluated inside a method of the carried class (Phase 5B). Only
    /// ever the receiver of a [`Self::MethodCall`]; resolved at lowering against the
    /// enclosing class's MRO to a direct `Call` with the current `self`.
    Super(ClassId),
    /// `isinstance(value, Cls)` (Phase 5B) → `Bool`. The class is resolved by the
    /// frontend; lowering emits the inheritance-aware runtime check.
    IsInstance {
        value: Idx<HirExpr>,
        class_id: ClassId,
    },
    /// `isinstance(value, str|int|float|bool)` (Phase 8B) → `Bool`. Folded
    /// statically at lowering from `value`'s inferred `SemTy` — a `Dyn` value is
    /// a loud compile error (a runtime tag check is out of scope).
    IsInstanceBuiltin {
        value: Idx<HirExpr>,
        target: SemTy,
    },
    /// `hasattr(value, "name")` (§5) → `Bool`. Folded statically at lowering from
    /// `value`'s `ClassInfo` (field / method / property / static- or class-method);
    /// a `Dyn` / non-class receiver is a loud compile error (a runtime attribute
    /// probe is out of scope), mirroring [`Self::IsInstanceBuiltin`].
    HasAttr {
        value: Idx<HirExpr>,
        name: InternedString,
    },
    /// `issubclass(sub, sup)` (§5) → `Bool`. Both classes are resolved by the
    /// frontend to user [`ClassId`]s; lowering folds via [`ClassTable::is_subclass`]
    /// (the C3-MRO check). The builtin-type / tuple second-arg forms are rejected
    /// by the frontend (out of scope).
    IsSubclass {
        sub: ClassId,
        sup: ClassId,
    },
    /// `callable(value)` (§5) → `Bool`. Folded statically at lowering from
    /// `value`'s inferred `SemTy`: a [`SemTy::Callable`], or a class instance whose
    /// class (via MRO) defines `__call__`, is callable; other concrete types are
    /// not; a `Dyn` / `Union` value is a loud compile error (a runtime callability
    /// probe is out of scope), mirroring [`Self::HasAttr`]. A bare name resolving
    /// to a user class or top-level function folds to `True` in the frontend.
    IsCallable {
        value: Idx<HirExpr>,
    },
    /// `getattr(value, "name"[, default])` (§5) → `Dyn`. Distinct from the
    /// 2-arg [`Self::Attribute`] desugar: it carries the explicit `getattr`
    /// fallback semantics so lowering can route a provably-absent field on a
    /// concrete receiver to a runtime probe instead of a compile error.
    /// Lowering keeps the static fast path when the attr is provably present
    /// (field/property/class-attr); otherwise it routes to
    /// `rt_getattr_name`/`rt_getattr_name_or_default` (`default = None`/`Some`).
    /// The `name` is a string literal (dynamic names stay out of scope).
    GetAttrByName {
        value: Idx<HirExpr>,
        name: InternedString,
        default: Option<Idx<HirExpr>>,
    },
    /// `value is None` (Phase 8D) → `Bool`, via `rt_is_none` — the identity test
    /// that recognizes both the immediate `None` tag and a heap `None` object
    /// (which `==` does not). `value is not None` is `Unary{Not, IsNone}`.
    IsNone {
        value: Idx<HirExpr>,
    },
    /// `l is r` for non-`None` operands (general identity) → `Bool`, via
    /// `rt_is` — bit-identity: immediates (`int`/`bool`/`None`) compare by their
    /// tagged `Value` bits, heap objects by pointer (`None`'s several ABI
    /// encodings are normalized). `l is not r` is `Unary{Not, Is}`. The
    /// `is None` form stays the dedicated [`Self::IsNone`] (single-operand,
    /// null-aware); `is` never dispatches through `__eq__` (that is `Compare`).
    Is {
        l: Idx<HirExpr>,
        r: Idx<HirExpr>,
    },
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
    CellGet {
        cell: LocalId,
    },
    /// Read promoted module-global slot `var_id` (Phase 6B) — GC-rooted uniform
    /// tagged storage (`rt_global_get_ptr`).
    GlobalGet {
        var_id: u32,
    },

    // ── generators (Phase 6E) ──
    /// Build a generator object (the wrapper's body) — `rt_make_generator`.
    MakeGenerator {
        gen_id: u32,
        num_locals: u32,
    },
    /// A generator state-machine query carrying its generator operand (P6-3:
    /// all values crossing a `GenOp` are `Tagged`, structurally enforced). The
    /// `slot`/`state` immediate rides alongside (`GetLocal`), and `value` is the
    /// sent value (`Send`); other ops ignore both.
    GenQuery {
        op: GenOp,
        gen: Idx<HirExpr>,
        imm: u32,
        value: Option<Idx<HirExpr>>,
    },

    // ── exceptions (Phase 7) ──
    /// A query against the thread-local exception state (Phase 7A). Only ever
    /// emitted by the frontend's `try`/`with` desugar, inside handler blocks
    /// where an exception is pending.
    ExcQuery(ExcQuery),
    /// `str(e)` / `print(e)` of a caught exception instance (Phase 7B) —
    /// `rt_exc_instance_str(value)` → the message `StrObj`.
    ExcInstanceStr {
        value: Idx<HirExpr>,
    },
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
        TypeSpec::FrozenSet => SemTy::RuntimeObject(TypeTagKind::FrozenSet),
        TypeSpec::ByteArray => SemTy::RuntimeObject(TypeTagKind::ByteArray),
    }
}

// ============================================================================
// Exceptions (Phase 7)
// ============================================================================

/// An exception-state bookkeeping op (Phase 7A), emitted only by the frontend's
/// `try`/`with`/`finally` desugar. Each maps 1:1 to a runtime call:
/// `rt_exc_start_handling` / `rt_exc_end_handling`. (The frame push/pop ops
/// are gone — protected regions are static [`HirBlock::handler`] annotations
/// consumed by table-based unwinding, not runtime frames.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExcOp {
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
    /// `raise MyError(args…)` for a user exception class. Lowering constructs
    /// the instance (running `__init__` when the class has one; a single arg
    /// becomes the message operand otherwise so `str(e)` works).
    Custom {
        class_id: ClassId,
        args: Vec<Idx<HirExpr>>,
    },
    /// `raise HTTPError(args…)` for a stdlib exception (Phase 8D). `exc_type_tag`
    /// is the builtin parent (`OSError`) so `except OSError` matches by tag;
    /// `class_id` is the reserved stdlib id so `except HTTPError` matches by id.
    /// `msg` is the first positional arg (its message); remaining args are
    /// ignored (the corpus never inspects the message).
    Stdlib {
        class_id: u8,
        exc_type_tag: u8,
        msg: Option<Idx<HirExpr>>,
    },
    /// `raise e` — re-raise a caught exception instance value.
    Instance { value: Idx<HirExpr> },
    /// Bare `raise` — re-raise the exception being handled.
    Reraise,
}

/// The explicit cause of a `raise TARGET from CAUSE` (PEP 3134), decoupled from
/// the raise target. Lowered to an [`HirStmt::ArmCause`] emitted immediately
/// before the target's [`HirStmt::Raise`]: it stashes the pending cause into
/// runtime exception state, which the next raise builder consumes when it
/// constructs its `ExceptionObject`. This avoids a per-target cause-variant
/// explosion — every target shape gets `from` support for free.
#[derive(Debug, Clone)]
pub enum ArmCause {
    /// `from None` — suppress the implicit `__context__` chain (cause stays
    /// `None`, `suppress_context = true`).
    Suppress,
    /// `from <builtin name / Builtin(msg)>` — a scalar builtin-exception cause
    /// encoded as `(tag, message)`, with no value→cause path needed.
    Builtin {
        cause_tag: u8,
        cause_msg: Option<Idx<HirExpr>>,
    },
    /// `from <value expr>` — a caught variable or a constructed custom/stdlib
    /// exception. The runtime extracts `(class_id, message)` from the Tagged
    /// instance value (raising `TypeError` for a non-exception).
    Value(Idx<HirExpr>),
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
    MatMul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    BitAnd,
    BitOr,
    /// In-place bitwise-or `|=`. The in-place sibling of [`BinOp::BitOr`]: the
    /// frontend emits this (not `BitOr`) for `x |= y`, so the runtime can mutate
    /// `dict`/`set` operands in place and return the same object — making the
    /// `x = x |= y` rebind alias-preserving. Numeric `|=` still delegates to the
    /// `BitOr` path inside the runtime. Always Tagged (no raw fast path).
    IOr,
    BitXor,
    /// In-place bitwise-and `&=`, in-place subtract `-=`, in-place xor `^=`. The
    /// in-place siblings of [`BinOp::BitAnd`]/[`BinOp::Sub`]/[`BinOp::BitXor`]
    /// (the `IOr` pattern): the frontend emits these for `s &= y` / `s -= y` /
    /// `s ^= y` so the runtime can mutate a `set` operand in place
    /// (`*_update`) and return the same object, making the augmented-assign
    /// rebind alias-preserving. Every non-set operand delegates to the
    /// new-object numeric path (`BitAnd`/`Sub`/`BitXor`). Always Tagged.
    IAnd,
    ISub,
    IXor,
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
    /// `zip(iter1, …, iterN)` for N≥3 → an iterator of N-tuples. Arg 0 is a
    /// runtime list holding the N pre-`iter()`-wrapped iterators; arg 1 is the
    /// `Raw(I64)` count. (The 2-iterable form stays the dedicated [`Self::Zip`].)
    /// Lowering builds the list; `rt_zipn_new` consumes it.
    ZipN,
    /// `list(iter)` → a fresh list materialized from a pre-wrapped iterator.
    ListFromIter,
    /// `tuple(iter)` → a fresh tuple from a pre-wrapped iterator.
    TupleFromIter,
    /// `dict(pairs)` → a fresh dict from a list of key/value pairs.
    DictFromPairs,
    /// `bytes(list_of_ints)` → a fresh bytes object from a list of ints.
    BytesFromList,
    /// `bytes(n)` → a fresh zero-filled bytes object of length `n` (the count is
    /// a `Raw(I64)`). Selected by lowering when the `bytes(...)` argument is an
    /// int/bool, distinct from `BytesFromList` (an iterable) / `BytesFromStr`.
    BytesZero,
    /// `bytes(s[, encoding])` → a fresh bytes object encoding the str `s` (UTF-8;
    /// the encoding argument is accepted but only UTF-8 is supported). Selected
    /// by lowering when the `bytes(...)` argument is a str.
    BytesFromStr,
    /// `sorted(list, reverse)` → a new sorted list; the input is
    /// pre-materialized to a list, `reverse` is a `Raw(I8)` truthiness flag.
    Sorted,
    /// `rt_list_sort_by_keys(list, keys, reverse)` — stable tandem sort of
    /// `list` by the parallel `keys` list (the compiled `key=` callback runs
    /// in a frontend-desugared loop BEFORE this op; no runtime callbacks).
    ListSortByKeys,
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
    /// `list.remove(x)` → `rt_list_remove`; removes the first value-equal element
    /// (ValueError if absent), a `None`-returning mutation.
    ListRemove,
    // tuple (§9 — `tuple.index(x)` / `tuple.count(x)`, value-comparing, B13)
    TupleIndexOf,
    TupleCount,
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
    /// `a | b` over two dicts (PEP 584) → a fresh merged dict (`rt_dict_merge`,
    /// right operand wins on key collisions). The operator-only sibling of
    /// `dict.update` / the `|=` in-place merge; routed from `try_container_binop`.
    DictMerge,
    /// `dict.popitem()` → a fresh `(key, value)` 2-tuple (LIFO, KeyError if
    /// empty); the tuple is a `Value`/`Tagged` GC-rootable result (B5).
    DictPopitem,
    // set
    SetRemove,
    SetDiscard,
    /// `set.pop()` → remove and return an arbitrary element (KeyError if empty);
    /// the element is a `Value`/`Tagged` GC-rootable result (B5).
    SetPop,
    SetUpdate,
    SetUnion,
    SetIntersection,
    SetDifference,
    /// `set.symmetric_difference(other)` → a fresh set of elements in exactly one
    /// of the two (the new-set algebra, distinct from `*_update`).
    SetSymmetricDifference,
    SetCopy,
    SetClear,
    // set comparison (§9 — value-comparing `rt_set_*`, → bool, B13)
    SetIsSubset,
    SetIsSuperset,
    SetIsDisjoint,
    // set in-place update (§9 — mutate in place, no result)
    SetIntersectionUpdate,
    SetDifferenceUpdate,
    SetSymmetricDifferenceUpdate,
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
    SymmetricDifference,
    Popitem,
    /// `OrderedDict.move_to_end(key, last=True)` — move an existing key to either
    /// end. Lowered directly via `rt_dict_move_to_end` (a `void` runtime call, not
    /// a recv-first `ContainerOp`), like [`ContainerMethod::Fromkeys`].
    MoveToEnd,
    IsSubset,
    IsSuperset,
    IsDisjoint,
    IntersectionUpdate,
    DifferenceUpdate,
    SymmetricDifferenceUpdate,
    /// `dict.fromkeys(keys[, value])` — build a fresh dict mapping every key in
    /// the iterable to `value` (default `None`). Lowered specially (the receiver
    /// is discarded; `rt_dict_fromkeys` takes only the keys list and the value),
    /// so it does not fit the recv-first `ContainerOp` signature.
    Fromkeys,
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
            "symmetric_difference" => ContainerMethod::SymmetricDifference,
            "popitem" => ContainerMethod::Popitem,
            "move_to_end" => ContainerMethod::MoveToEnd,
            "issubset" => ContainerMethod::IsSubset,
            "issuperset" => ContainerMethod::IsSuperset,
            "isdisjoint" => ContainerMethod::IsDisjoint,
            "intersection_update" => ContainerMethod::IntersectionUpdate,
            "difference_update" => ContainerMethod::DifferenceUpdate,
            "symmetric_difference_update" => ContainerMethod::SymmetricDifferenceUpdate,
            "fromkeys" => ContainerMethod::Fromkeys,
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
    /// An unboxed `Raw(I8)` boolean flag (`reverse=` — CPython truthiness,
    /// computed by lowering's `truthy_i8`).
    Bool,
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
        use ContainerArg::{Bool, Idx, Val};
        match self {
            ContainerOp::ListNew
            | ContainerOp::DictNew
            | ContainerOp::SetNew
            | ContainerOp::TupleNew
            // `bytes(n)` zero-fill — the count is an unboxed `Raw(I64)`.
            | ContainerOp::BytesZero => &[Idx],
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
            | ContainerOp::ListRemove
            | ContainerOp::TupleIndexOf
            | ContainerOp::TupleCount
            | ContainerOp::DictPopM
            | ContainerOp::DictUpdate
            | ContainerOp::DictMerge
            | ContainerOp::SetRemove
            | ContainerOp::SetDiscard
            | ContainerOp::SetUpdate
            | ContainerOp::SetUnion
            | ContainerOp::SetIntersection
            | ContainerOp::SetDifference
            | ContainerOp::SetSymmetricDifference
            | ContainerOp::SetIsSubset
            | ContainerOp::SetIsSuperset
            | ContainerOp::SetIsDisjoint
            | ContainerOp::SetIntersectionUpdate
            | ContainerOp::SetDifferenceUpdate
            | ContainerOp::SetSymmetricDifferenceUpdate => &[Val, Val],
            // `list.insert(index, value)` — the index is an unboxed `Raw(I64)`.
            ContainerOp::ListInsert => &[Val, Idx, Val],
            // `dict.get(k[, default])` / `dict.setdefault(k[, default])` — all tagged.
            ContainerOp::DictSetdefault | ContainerOp::DictGetDefault => &[Val, Val, Val],
            ContainerOp::ListSortMut => &[Val, Bool],
            ContainerOp::ListSortByKeys => &[Val, Val, Bool],
            ContainerOp::ListClear
            | ContainerOp::ListCopy
            | ContainerOp::ListReverse
            | ContainerOp::DictKeys
            | ContainerOp::DictValues
            | ContainerOp::DictItems
            | ContainerOp::DictClear
            | ContainerOp::DictCopy
            | ContainerOp::DictPopitem
            | ContainerOp::SetCopy
            | ContainerOp::SetPop
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
            // `rt_zipn_new(iters_list, count)` — the iterators ride a tagged list
            // pointer, the count an unboxed `Raw(I64)`.
            ContainerOp::ZipN => &[Val, Idx],
            ContainerOp::RangeIter => &[Idx, Idx, Idx],
            ContainerOp::Len
            | ContainerOp::Iter
            | ContainerOp::IterNext
            | ContainerOp::IterExhausted
            | ContainerOp::ListFromIter
            | ContainerOp::TupleFromIter
            | ContainerOp::DictFromPairs
            | ContainerOp::BytesFromList
            | ContainerOp::BytesFromStr
            | ContainerOp::Reversed => &[Val],
            ContainerOp::Sorted => &[Val, Bool],
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
            | ContainerOp::ZipN
            | ContainerOp::ListFromIter
            | ContainerOp::TupleFromIter
            | ContainerOp::DictFromPairs
            | ContainerOp::BytesFromList
            | ContainerOp::BytesZero
            | ContainerOp::BytesFromStr
            | ContainerOp::Sorted
            | ContainerOp::Reversed
            | ContainerOp::RangeIter
            | ContainerOp::ListCopy
            | ContainerOp::DictKeys
            | ContainerOp::DictValues
            | ContainerOp::DictItems
            | ContainerOp::DictCopy
            | ContainerOp::DictMerge
            | ContainerOp::SetUnion
            | ContainerOp::SetIntersection
            | ContainerOp::SetDifference
            | ContainerOp::SetSymmetricDifference
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
            | ContainerOp::ListSortByKeys
            | ContainerOp::DictUpdate
            | ContainerOp::DictClear
            | ContainerOp::ListRemove
            | ContainerOp::SetRemove
            | ContainerOp::SetDiscard
            | ContainerOp::SetUpdate
            | ContainerOp::SetClear
            | ContainerOp::SetIntersectionUpdate
            | ContainerOp::SetDifferenceUpdate
            | ContainerOp::SetSymmetricDifferenceUpdate => ContainerResult::None,
            ContainerOp::ListGet
            | ContainerOp::DictGet
            | ContainerOp::TupleGet
            | ContainerOp::StrGet
            | ContainerOp::AnyGetItem
            | ContainerOp::IterNext
            | ContainerOp::ListPop
            | ContainerOp::DictGetDefault
            | ContainerOp::DictPopM
            | ContainerOp::DictPopitem
            | ContainerOp::SetPop
            | ContainerOp::DictSetdefault => ContainerResult::Value,
            ContainerOp::BytesGet
            | ContainerOp::Len
            | ContainerOp::ListIndexOf
            | ContainerOp::ListCount
            | ContainerOp::TupleIndexOf
            | ContainerOp::TupleCount => ContainerResult::Int,
            ContainerOp::Contains
            | ContainerOp::IterExhausted
            | ContainerOp::ListCmp(_)
            | ContainerOp::TupleCmp(_)
            | ContainerOp::SetIsSubset
            | ContainerOp::SetIsSuperset
            | ContainerOp::SetIsDisjoint => ContainerResult::Bool,
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

/// The output stream a `print` writes to. `Stdout` is the default;
/// `print(..., file=sys.stderr)` selects `Stderr`. Realized in `lowering` by
/// toggling the runtime's global print target (`rt_print_set_stderr` /
/// `rt_print_set_stdout`) around the line's writes — the runtime print surface
/// itself stays one set of `rt_print_*` calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrintTarget {
    #[default]
    Stdout,
    Stderr,
}

#[derive(Debug, Clone)]
pub enum HirStmt {
    /// Source-line marker (real tracebacks): the statements that follow — up
    /// to the next marker — originate from this 1-based source line. Emitted
    /// by the frontend on every line change AND as the first statement of
    /// every block (codegen's `srcloc` state follows emission order, not
    /// control flow, so each block must re-establish its line). No runtime
    /// effect; codegen turns it into Cranelift `set_srcloc` metadata.
    Line(u32),
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
    /// `PrintKind` dispatch. `file` selects the output stream
    /// (`print(..., file=sys.stderr)`); lowering toggles the runtime's global
    /// print target around the line's writes (after evaluating the args, so a
    /// side-effecting argument's own output still goes to the current stream).
    /// `flush` (`print(..., flush=True)`) emits a flush of the selected stream
    /// after the line is written.
    Print {
        args: Vec<Idx<HirExpr>>,
        sep: Option<InternedString>,
        end: Option<InternedString>,
        file: PrintTarget,
        flush: bool,
    },
    /// Subscript write `base[index] = value` (Phase 4A). The runtime dispatch
    /// (`rt_list_set` / `rt_dict_set`) is selected at lowering from the `base`
    /// representation; assigning to a tuple element is a compile error.
    SetItem {
        base: Idx<HirExpr>,
        index: Idx<HirExpr>,
        value: Idx<HirExpr>,
    },
    /// Subscript delete `del base[index]` (the container `del` form). Mirrors
    /// [`Self::SetItem`] minus the value: lowering dispatches the runtime
    /// deleter (`rt_list_delete` / `rt_dict_delete` / `rt_any_delitem`, or a
    /// class `__delitem__`) from the `base` representation, raising
    /// IndexError/KeyError like CPython.
    DelItem {
        base: Idx<HirExpr>,
        index: Idx<HirExpr>,
    },
    /// Slice write `base[start:end:step] = value`. Each bound is optional (absent
    /// → the runtime's `i64::MIN`/`i64::MAX`/`1` default, like [`HirExprKind::Slice`]);
    /// `value` is a fresh list the frontend materialized from the RHS iterable
    /// (any iterable → list, also breaking `a[1:3] = a` aliasing). Lowering emits
    /// `rt_list_setslice`; a non-list base raises `TypeError` at runtime.
    SetSlice {
        base: Idx<HirExpr>,
        start: Option<Idx<HirExpr>>,
        end: Option<Idx<HirExpr>>,
        step: Option<Idx<HirExpr>>,
        value: Idx<HirExpr>,
    },
    /// Slice delete `del base[start:end:step]`. Mirrors [`Self::SetSlice`] minus
    /// the value; lowering emits `rt_list_delslice` (a non-list base raises
    /// `TypeError` at runtime).
    DelSlice {
        base: Idx<HirExpr>,
        start: Option<Idx<HirExpr>>,
        end: Option<Idx<HirExpr>>,
        step: Option<Idx<HirExpr>>,
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
    ContainerPush {
        container: LocalId,
        value: Idx<HirExpr>,
    },
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
    CellSet { cell: LocalId, value: Idx<HirExpr> },
    /// Write promoted module-global slot `var_id` (Phase 6B) —
    /// `rt_global_set_ptr` (uniform tagged storage).
    GlobalSet { var_id: u32, value: Idx<HirExpr> },

    // ── generators (Phase 6E) ──
    /// Write generator slot `slot` from `value` — `GenOp::SetLocal`.
    GenSetLocal {
        gen: Idx<HirExpr>,
        slot: u32,
        value: Idx<HirExpr>,
    },
    /// Set the generator state — `GenOp::SetState`.
    GenSetState { gen: Idx<HirExpr>, state: u32 },
    /// Mark the generator exhausted — `GenOp::SetExhausted`.
    GenSetExhausted { gen: Idx<HirExpr> },

    // ── exceptions (Phase 7) ──
    /// Exception-frame bookkeeping (pop / start-handling / end-handling).
    ExcOp(ExcOp),
    /// Stash a pending `from CAUSE` for the immediately-following [`Raise`]
    /// (PEP 3134). Emitted right before the raise in the same block so the
    /// runtime sets and consumes the pending slot synchronously.
    ArmCause(ArmCause),
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
        Self {
            symbols: Vec::new(),
        }
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
    /// `class C(Protocol)` / `class C(Protocol[T])` — a structural-typing marker.
    /// A protocol contributes no runtime base; its instances are never
    /// constructed. Protocol-typed slots erase to `Dyn` (Tagged baseline) and
    /// `isinstance(obj, P)` is a structural method-presence check.
    pub is_protocol: bool,
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
#[derive(Debug, Clone, PartialEq)]
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

/// A function parameter's default value. A constant literal is materialized
/// fresh at each call site (the today's-baseline [`Self::Const`] path); a
/// mutable/computed default of a **top-level** function is evaluated exactly
/// once at the `def`'s module-init position into a synthetic GC-rooted global
/// slot, then read (shared) at every defaulted call — CPython's "mutable
/// default is one object reused across calls" semantics ([`Self::Slot`]).
#[derive(Debug, Clone, PartialEq)]
pub enum ParamDefault {
    /// Immutable literal — materialized fresh per call (the [`ClassAttrInit`]
    /// shape, shared with class attributes).
    Const(ClassAttrInit),
    /// Non-literal (mutable/computed) — read the promoted global slot `var_id`,
    /// which holds the one object evaluated once at def-time.
    Slot(u32),
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
    /// `class C(Protocol)` — a structural-typing marker. Drives the
    /// structural `isinstance(obj, P)` lowering (method-presence probe) instead
    /// of the nominal MRO check.
    pub is_protocol: bool,
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
        Self {
            classes: HashMap::new(),
        }
    }
    pub fn insert(&mut self, info: ClassInfo) {
        self.classes.insert(info.class_id, info);
    }
    pub fn get(&self, cid: ClassId) -> Option<&ClassInfo> {
        self.classes.get(&cid)
    }
    /// Mutable access for `typeck`'s B10 field-type write-back — the one
    /// consumer that updates the table after construction.
    pub fn get_mut(&mut self, cid: ClassId) -> Option<&mut ClassInfo> {
        self.classes.get_mut(&cid)
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

    /// Nominal subtyping (D8): `a <: b` iff `b` appears in `a`'s MRO. The lattice
    /// consults this through the [`pyaot_types::ClassHierarchy`] env — the MRO
    /// data lives only here, never duplicated into `types`.
    pub fn is_subclass(&self, a: ClassId, b: ClassId) -> bool {
        if a == b {
            return true;
        }
        self.classes
            .get(&a)
            .is_some_and(|info| info.mro.contains(&b))
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

/// The lattice's view of the class hierarchy: the C3 MRO computed by
/// `semantics`. Unknown ids (e.g. builtin container `ClassId`s, which never
/// enter the table) get an empty MRO, so they stay nominally unrelated.
impl pyaot_types::ClassHierarchy for ClassTable {
    fn mro(&self, c: ClassId) -> &[ClassId] {
        self.get(c).map(|info| info.mro.as_slice()).unwrap_or(&[])
    }
    fn class_name(&self, c: ClassId) -> Option<InternedString> {
        self.get(c).map(|info| info.name)
    }
}

// ── keyword → parameter-slot matching (Phase 10) ──────────────────────────────

/// Where one callee parameter slot's value comes from after keyword matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotSource {
    /// The i-th call-site positional argument.
    Pos(usize),
    /// The i-th call-site keyword argument.
    Kw(usize),
    /// The parameter's declared default.
    Default,
}

/// Why a keyword call cannot be matched to the callee's parameters. Carries
/// interned names so the reporter can render CPython-flavored messages
/// (`Duplicate` ⇒ "got multiple values for argument …").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KwMatchError {
    Unexpected(InternedString),
    Duplicate(InternedString),
    Missing(InternedString),
    TooManyPositional { expected: usize, got: usize },
}

/// The result of [`match_keywords`]: one [`SlotSource`] per callee parameter,
/// plus the keyword indices left for a `**kwargs` callee's dict slot
/// (in written order; empty unless `allow_extra`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KwMatch {
    pub slots: Vec<SlotSource>,
    pub leftover: Vec<usize>,
}

/// Map a call site's positional count + keyword names onto a callee's fixed
/// parameter list (`param_names` EXCLUDES `self` and the `*args`/`**kwargs`
/// slots; `has_default` is parallel to it). Every named parameter is treated
/// as positional-or-keyword — the HIR subset does not model kw-only markers.
/// `allow_extra` (a `**kwargs` callee) routes unknown names to `leftover`
/// instead of erroring. Pure slot algebra — no side tables, shared by typeck
/// and lowering.
pub fn match_keywords(
    param_names: &[InternedString],
    has_default: &[bool],
    n_pos: usize,
    kw_names: &[InternedString],
    allow_extra: bool,
) -> Result<KwMatch, KwMatchError> {
    debug_assert_eq!(param_names.len(), has_default.len());
    if n_pos > param_names.len() {
        return Err(KwMatchError::TooManyPositional {
            expected: param_names.len(),
            got: n_pos,
        });
    }
    let mut used = vec![false; kw_names.len()];
    let mut slots = Vec::with_capacity(param_names.len());
    for (i, &p) in param_names.iter().enumerate() {
        let kw_idx = kw_names.iter().position(|&k| k == p);
        if i < n_pos {
            if kw_idx.is_some() {
                return Err(KwMatchError::Duplicate(p));
            }
            slots.push(SlotSource::Pos(i));
        } else if let Some(k) = kw_idx {
            used[k] = true;
            slots.push(SlotSource::Kw(k));
        } else if has_default[i] {
            slots.push(SlotSource::Default);
        } else {
            return Err(KwMatchError::Missing(p));
        }
    }
    let leftover: Vec<usize> = (0..kw_names.len()).filter(|&k| !used[k]).collect();
    if !allow_extra {
        if let Some(&k) = leftover.first() {
            return Err(KwMatchError::Unexpected(kw_names[k]));
        }
    }
    Ok(KwMatch {
        slots,
        leftover: if allow_extra { leftover } else { Vec::new() },
    })
}
