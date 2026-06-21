//! HIR construction: module + per-function CFG building.
//!
//! [`FnLowerer`] is a block builder. Statements append to the *current* block;
//! emitting a terminator seals it and switches to a successor. Block-producing
//! expressions (short-circuit `and`/`or`, ternary, chained compares) split the
//! current block and route through a single-eval result local.
//!
//! The implemented subset grows per milestone; anything outside it returns a
//! [`CompilerError::parse_error`].

use std::collections::{HashMap, HashSet};

use la_arena::{Arena, Idx};
use rustpython_parser::ast::{
    BoolOp as PyBoolOp, CmpOp as PyCmpOp, Comprehension, Constant, Expr, ExprBinOp, ExprBoolOp,
    ExprAttribute, ExprCall, ExprCompare, ExprDict, ExprDictComp, ExprGeneratorExp, ExprIfExp,
    ExprLambda, ExprListComp,
    ExprNamedExpr, ExprSetComp, ExprSubscript, ExprUnaryOp, Keyword, Operator as PyOperator,
    Ranged, Stmt, StmtClassDef, StmtDelete, StmtFunctionDef, StmtImport, StmtImportFrom,
    UnaryOp as PyUnaryOp,
};
use rustpython_parser::text_size::TextRange;

use pyaot_core_defs::FIRST_USER_CLASS_ID;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{
    BinOp, ClassAttrInit, CmpOp, ContainerOp, ExcOp, ExcQuery, GenOp, HirBlock, HirClass,
    HirClassAttr, HirExpr, HirExprKind, HirFunction, HirLocal, HirModule, HirParam, HirProgram,
    HirProperty, HirRaise, HirStmt, HirTerminator, NamespaceImports, NamespaceTable, ParamDefault,
    PrintTarget, SymbolRef, UnaryOp,
};
use pyaot_types::{SemTy, Sig};
use pyaot_utils::{ClassId, FuncId, InternedString, LineMap, LocalId, Span, StringInterner};

use crate::freevars::{self, ScopeFacts};

mod annotations;
mod builder;
mod builtins;
mod calls;
mod classes;
mod closures;
mod comprehensions;
mod exceptions;
mod expressions;
mod generators;
mod patterns;
mod program;
mod statements;
mod stdlib;
mod synth_class;
#[cfg(test)]
mod tests;

use annotations::*;
use calls::*;
use classes::*;
use expressions::*;
use generators::*;
use program::*;
use statements::*;
use stdlib::*;
use synth_class::*;

/// Maps a user class *name* (as written in an annotation) to its assigned
/// `ClassId` and interned name, so `def f() -> Widget` / `x: Widget` resolve to
/// `SemTy::Class`. Built up front from all top-level `class` statements, so
/// forward references resolve regardless of declaration order.
type ClassNameMap = HashMap<String, (ClassId, InternedString)>;

/// The in-scope type variables (`T = TypeVar("T")` module-level vars plus the
/// enclosing generic class's type params), mapping each name to its interned id so
/// an annotation `T` resolves to `SemTy::Var("T")` instead of `Dyn` (Phase 5E).
type TypeVarSet = HashMap<String, InternedString>;

/// One parameter's call-facing shape (Phase 6C): name, annotated type, optional
/// default (a literal `Const` or a `Slot` for a mutable/computed top-level
/// default).
#[derive(Debug, Clone)]
pub(crate) struct ParamInfo {
    pub name: InternedString,
    pub ty: SemTy,
    pub default: Option<ParamDefault>,
}

/// `(top-level def name, parameter name) → synthetic promoted-global slot`, for
/// the mutable/computed parameter defaults of top-level `def`s. Allocated once
/// per module in `lower_module_into`; a non-literal default reads its slot at
/// every defaulted call and the slot is set once at the def's module-init
/// position (CPython def-time once-evaluation + shared-object aliasing).
type DefaultSlotMap = HashMap<(InternedString, InternedString), u32>;

/// A top-level `def`'s call-facing shape, collected up front so any function
/// can synthesize a value-position thunk for it (Phase 6A) and reorder / fill
/// keyword and default arguments at known-callee call sites (Phase 6C).
///
/// The MIR parameter order is **fixed positional → keyword-only → `*args`
/// tuple → `**kwargs` dict** (variadic slots trailing); call-site adaptation
/// produces exactly that positional vector.
#[derive(Debug, Clone)]
pub(crate) struct TopDefInfo {
    pub fixed: Vec<ParamInfo>,
    pub kwonly: Vec<ParamInfo>,
    /// `*args` name (a `tuple[Dyn, ...]`), if present.
    pub varargs: Option<InternedString>,
    /// `**kwargs` name (a `dict[str, Dyn]`), if present.
    pub kwargs: Option<InternedString>,
    pub ret: SemTy,
}

/// Top-level `def` name → its call-facing shape.
type TopDefMap = HashMap<String, TopDefInfo>;

/// Annotation-resolution context: the class-name map + the in-scope type vars +
/// the top-level def table + the promoted module-globals table (Phase 6B).
///
/// `Copy` so a callee that must restrict the context (e.g. a nested def must not
/// see the enclosing top-level def's `default_slots`) can cheaply clone-and-edit.
#[derive(Clone, Copy)]
pub(crate) struct AnnCtx<'a> {
    class_map: &'a ClassNameMap,
    type_vars: &'a TypeVarSet,
    top_defs: &'a TopDefMap,
    /// Promoted module-global name → dense `var_id` (Phase 6B).
    promoted: &'a HashMap<String, u32>,
    /// Names of decorated module-level functions (Phase 6D). Their public name
    /// is a promoted global slot of a `(*args, **kwargs)` wrapper, so a call by
    /// that name packs its positional/keyword args into the variadic slots.
    decorated: &'a HashSet<String>,
    /// `import M` alias names (Phase 8). `M.f(...)` / `M.Cls(...)` / `M.VAR`
    /// fold to qualified accesses: imported funcs/classes live in `top_defs` /
    /// `class_map` under the `"M.name"` key, module vars in `alias_vars`.
    aliases: &'a HashSet<String>,
    /// `"M.VAR"` → the exporter module's promoted global slot, for live reads of
    /// an aliased module's module-level variable (Phase 8).
    alias_vars: &'a HashMap<String, u32>,
    /// Stdlib bindings (Phase 8B): names bound to frozen-runtime descriptors by
    /// `import math` / `from math import sqrt` when no user module shadows them.
    stdlib: &'a StdlibBindings,
    /// Mutable/computed parameter-default slots, set `Some` **only** while
    /// lowering top-level `def`s (so a non-literal default resolves to its
    /// global slot). `None` everywhere else — nested defs, methods, decorated
    /// defs, generators — where such a default is a clean parse error. Threaded
    /// as `Option` (not relied-on key absence) so a method/nested function that
    /// shares a top-level function's name cannot wrongly pick up its slot.
    default_slots: Option<&'a DefaultSlotMap>,
    /// Module type aliases: `type X = T` (PEP 695) and `X:
    /// TypeAlias = T` (PEP 613) → the alias name resolves to its body `SemTy` in
    /// annotation position (consulted by `named_annotation`).
    type_aliases: &'a HashMap<String, SemTy>,
    /// Class ids of `Protocol` classes: a protocol-typed slot/param
    /// erases to `Dyn` (Tagged baseline) so method dispatch rides the gradual
    /// `rt_obj_method` path. Consulted by `named_annotation` /
    /// `annotation_subscript` after a class id is resolved.
    proto_ids: &'a HashSet<ClassId>,
}

/// Per-module stdlib bindings (Phase 8B), collected during the import scan.
/// A user module on the search path always wins over a same-named stdlib
/// module (CPython script-dir-first), so these are only populated when the
/// loader had no match. Lookup keys: the bound name for `from M import f
/// [as g]`, the qualified `"M.f"` for `import M [as A]` accesses.
#[derive(Default)]
pub(crate) struct StdlibBindings {
    /// `import M` alias names whose target is a stdlib module.
    aliases: HashSet<String>,
    /// Callable name → its function descriptor.
    funcs: HashMap<String, &'static pyaot_stdlib_defs::StdlibFunctionDef>,
    /// Constant name (`math.pi` / from-imported `pi`) → its descriptor; folds
    /// to a literal at every use site.
    consts: HashMap<String, &'static pyaot_stdlib_defs::StdlibConstDef>,
    /// Module attribute (`sys.argv`) → its getter descriptor.
    attrs: HashMap<String, &'static pyaot_stdlib_defs::StdlibAttrDef>,
    /// Stdlib class name in annotation position (`time.struct_time`) → its
    /// semantic type (`RuntimeObject(tag)`).
    classes: HashMap<String, SemTy>,
    /// Stdlib exception name (`HTTPError`) → `(reserved class_id, builtin parent
    /// tag)`. `except HTTPError:` matches by class id; the parent tag makes
    /// `except OSError:` / `except Exception:` match too (Phase 8D).
    exceptions: HashMap<String, (u8, u8)>,
}

/// A decorated module-level function's runtime facts (Phase 6D): the promoted
/// global slot holding its wrapper, and the generic `(*args, **kwargs)` adapter
/// thunk over its renamed `<orig>` body.
struct DecoratedDef {
    slot: u32,
    thunk_fid: FuncId,
}

/// Module-wide mutable lowering state shared by every (possibly nested)
/// function lowering: the function table with *reserved* slots (a nested `def`
/// reserves its `FuncId` before its body is lowered, so ids stay dense and
/// stable regardless of nesting), plus the per-module thunk memo.
pub(crate) struct Shared {
    funcs: Vec<Option<HirFunction>>,
    /// Owning namespace id per `FuncId` (parallel to `funcs`), set at reserve
    /// time from `current_ns` (Phase 8). Every function — including synthetics
    /// (thunks / lambdas / generator resumes) — belongs to the module being
    /// lowered, so `semantics` resolves its `Unresolved` names in that scope.
    func_ns: Vec<u32>,
    /// Namespace new reservations are tagged with (the module being lowered).
    current_ns: u32,
    /// Memoized value-position thunks for top-level functions: `(namespace,
    /// name) → FuncId`. Keyed by namespace so a same-named function in two
    /// modules gets distinct thunks (Phase 8).
    thunks: HashMap<(u32, String), FuncId>,
    /// Generator resume functions indexed by dense `gen_id` (Phase 6E) — a
    /// single program-global, dense space across all modules.
    generators: Vec<FuncId>,
    /// Display path of the module being lowered (real tracebacks): the entry
    /// script's command-line path or the loader's resolved path. Saved and
    /// restored around nested module lowering (imports lower recursively).
    cur_file: Option<InternedString>,
    /// Byte-offset → line map of the module being lowered (same discipline).
    line_map: LineMap,
    /// Module globals a `del name` unbinds (`var_id → name`), accumulated across
    /// every function/module lowered. Flows into [`HirModule::deletable_globals`]
    /// to drive the `GlobalGet` read-guards. Program-global like `generators`.
    deletable_globals: HashMap<u32, InternedString>,
    /// Instance-field names a `del obj.attr` unbinds, accumulated across the
    /// whole program. Flows into [`HirModule::deletable_fields`].
    deletable_fields: HashSet<InternedString>,
    /// Method names invoked as a method call (`x.NAME(...)`) anywhere in a
    /// module body, accumulated before that module's classes are lowered
    /// (gradual-completeness method dispatch, Phase B). An instance method whose
    /// name is in this set gets a uniform thunk — the over-approximate gate
    /// (the frontend runs pre-typeck, so "called on a `Dyn` receiver" is not yet
    /// knowable; method-call syntax is the soundest available proxy).
    dyn_method_names: HashSet<InternedString>,
    /// `method_FuncId → uniform_thunk_FuncId` built during class lowering;
    /// flows into [`HirModule::method_uniform_thunks`] for the codegen
    /// `rt_register_method_uniform` registrations (Phase B).
    method_uniform_thunks: HashMap<FuncId, FuncId>,
    /// `next_method_FuncId → iternext_thunk_FuncId` built during class lowering
    /// for each class with an own `__next__`; flows into
    /// [`HirModule::iternext_thunks`] for the codegen `rt_register_iternext`
    /// registrations (lazy user-class iterator protocol).
    iternext_thunks: HashMap<FuncId, FuncId>,
    /// `dunder_method_FuncId → copy_thunk_FuncId` built during class lowering for
    /// each class that defines `__copy__` and/or `__deepcopy__`; flows into
    /// [`HirModule::copy_thunks`] for the codegen `rt_register_copy_func` /
    /// `rt_register_deepcopy_func` registrations. One map holds both dunders
    /// (keyed by each method's own FuncId).
    copy_thunks: HashMap<FuncId, FuncId>,
    /// `@staticmethod`/`@classmethod` shapes for the spread-call desugar:
    /// synthetic name `"Cls.method"` → (`is_classmethod`, raw AST args). Recorded
    /// in a pre-pass BEFORE bodies are lowered (classes are lowered last), so a
    /// `Cls.smeth(*args)` call site can build a receiver-less uniform thunk on
    /// demand (parsed lazily where `ctx` exists) and route through `CallValue` —
    /// the free-function spread path. `is_classmethod` selects [`FirstParam`]
    /// (`SkipCls` drops the leading `cls`, mirroring how the method itself was
    /// lowered). Accumulated across modules; class names are program-unique.
    static_method_shapes: HashMap<String, (bool, rustpython_parser::ast::Arguments)>,
}

impl Shared {
    fn new() -> Self {
        Self {
            funcs: Vec::new(),
            func_ns: Vec::new(),
            current_ns: 0,
            thunks: HashMap::new(),
            generators: Vec::new(),
            cur_file: None,
            line_map: LineMap::new(""),
            deletable_globals: HashMap::new(),
            deletable_fields: HashSet::new(),
            dyn_method_names: HashSet::new(),
            method_uniform_thunks: HashMap::new(),
            iternext_thunks: HashMap::new(),
            copy_thunks: HashMap::new(),
            static_method_shapes: HashMap::new(),
        }
    }

    /// Reserve the next dense `FuncId`; the caller must `fill` it.
    fn reserve(&mut self) -> FuncId {
        let id = FuncId::new(self.funcs.len() as u32);
        self.funcs.push(None);
        self.func_ns.push(self.current_ns);
        id
    }

    fn fill(&mut self, id: FuncId, f: HirFunction) {
        debug_assert!(self.funcs[id.index()].is_none(), "double fill of {id:?}");
        self.funcs[id.index()] = Some(f);
    }

    fn finish(self) -> Vec<HirFunction> {
        self.funcs
            .into_iter()
            .map(|f| f.expect("every reserved FuncId is filled"))
            .collect()
    }
}

/// What a fully-lowered module exports (frontend-internal, Phase 8). Drives
/// `from M import x` static binding and `M.x` attribute folding in importers.
#[derive(Clone)]
pub(crate) struct ModuleExports {
    init_fid: FuncId,
    init_name: InternedString,
    /// Top-level def name → (FuncId, call-facing shape).
    funcs: HashMap<String, (FuncId, TopDefInfo)>,
    /// Top-level class name → (ClassId, interned class name).
    classes: HashMap<String, (ClassId, InternedString)>,
    /// Module-level variable name → promoted global var_id.
    var_slots: HashMap<String, u32>,
}

/// One import statement's runtime effect (Phase 8), precomputed during the load
/// DFS (so first-import order matches CPython) and replayed positionally during
/// the importing module's body lowering.
#[derive(Default, Clone)]
struct ImportAction {
    /// Module-`<init>` callees to invoke in order (parent packages first, each
    /// only at its program-wide first-import site).
    init_calls: Vec<InternedString>,
    /// `from M import VAR` snapshots: (importer slot, exporter slot).
    snapshots: Vec<(u32, u32)>,
}

/// Per-module binding state collected during the import scan (Phase A) and used
/// when lowering the module body (Phase B).
struct ImportCollect {
    class_map: ClassNameMap,
    promoted: HashMap<String, u32>,
    /// `from M import f` / qualified `M.f` → call-facing shape, by lookup key.
    imported_funcs: Vec<(String, TopDefInfo)>,
    /// `import M` alias names — `M.x` folds to a qualified access.
    aliases: HashSet<String>,
    /// `"M.VAR"` → the exporter module's global slot (for live `M.VAR` reads).
    alias_vars: HashMap<String, u32>,
    /// Names this module re-exports: `from .x import Y` makes `Y` part of THIS
    /// module's public surface, so a downstream `from thismod import Y` (and
    /// `import thismod; thismod.Y`) resolves — the canonical package-`__init__`
    /// API. Funcs/classes only; re-exported variables already ride `promoted`.
    reexport_funcs: Vec<(String, FuncId, TopDefInfo)>,
    reexport_classes: Vec<(String, ClassId, InternedString)>,
    /// Stdlib bindings (Phase 8B), populated when the loader has no user
    /// module for an imported name.
    stdlib: StdlibBindings,
    /// Import-statement START OFFSET → its precomputed runtime effect. The key is
    /// the statement's source-text start (`range().start().to_u32()`), not a flat
    /// body index, so a conditionally-nested import (inside an `if`/`try` at
    /// module level) can be located from `lower_stmt` and replayed in position.
    actions: HashMap<u32, ImportAction>,
    /// Start offsets of EVERY import the scan visited (including stdlib/typing
    /// no-ops). Lets `lower_stmt` tell a module-level conditional import (the scan
    /// loaded + bound it, so it is allowed) from an import in a function body /
    /// `while`/`for`/`with` (never scanned, still rejected).
    scanned_imports: HashSet<u32>,
}

/// The shared program-lowering context (Phase 8). Owns the global allocators and
/// drives the import-graph DFS; every module lowers into the one shared
/// function / class / generator / global-slot space — no merge or remap pass.
pub(crate) struct ProgramLowerer<'a> {
    interner: &'a mut StringInterner,
    loader: &'a mut dyn crate::ModuleSource,
    shared: Shared,
    classes: Vec<HirClass>,
    global_annotations: HashMap<u32, SemTy>,
    class_ns: HashMap<ClassId, u32>,
    namespace_imports: Vec<NamespaceImports>,
    next_class_id: u32,
    next_global: u32,
    loaded: HashMap<String, ModuleExports>,
    loading: Vec<String>,
    init_emitted: HashSet<String>,
}




/// One active control scope, pushed while lowering its body (Phase 7
/// generalization of the loop stack). Early exits (`return` / `break` /
/// `continue`) walk this stack emitting each scope's cleanup. Protected
/// scopes carry the handler context OUTSIDE them (`outer`): leaving one on an
/// early exit switches `cur_handler` back to `outer` in a fresh block, so the
/// cleanup code itself (a finalbody, an `__exit__` call) is not protected by
/// the region it is leaving — the static equivalent of the old frame pop.
#[derive(Clone)]
enum ScopeCtx {
    /// A loop body: `break`/`continue` jump targets; no cleanup of its own.
    Loop {
        continue_to: Idx<HirBlock>,
        break_to: Idx<HirBlock>,
    },
    /// A `try` body protected by an except handler. Cleanup: exit the region.
    TryFrame { outer: Option<Idx<HirBlock>> },
    /// An `except` handler body. Cleanup: `EndHandling`.
    Handler,
    /// A `try` body protected by a finally handler. Cleanup: exit the region +
    /// the re-lowered (cloned) finalbody.
    Finally {
        outer: Option<Idx<HirBlock>>,
        stmts: Vec<Stmt>,
    },
    /// A `with` body. Cleanup: exit the region + `mgr.__exit__(None, None, None)`.
    WithCleanup {
        outer: Option<Idx<HirBlock>>,
        mgr: LocalId,
    },
}

/// The element action of a comprehension: append to a list/set, or insert into a
/// dict. Carries the result container local plus the borrowed element expressions.
enum CompKind<'a> {
    List {
        result: LocalId,
        elt: &'a Expr,
    },
    Set {
        result: LocalId,
        elt: &'a Expr,
    },
    Dict {
        result: LocalId,
        key: &'a Expr,
        val: &'a Expr,
    },
}

/// A simple iterator loop opened by `begin_iter_loop`: the header to jump back to,
/// the exit block to continue at, and the per-iteration element local.
struct IterLoop {
    header: Idx<HirBlock>,
    exit: Idx<HirBlock>,
    elem: LocalId,
}

/// How a `min`/`max` `key=` callable is invoked (Phase 7): staged once into a
/// local and called indirectly (lambdas, locals), or called directly by name
/// per element (builtins / top-level functions, which have no value-position
/// staging requirement).
enum KeyMode<'e> {
    Staged(LocalId),
    ByName(&'e Expr),
}

/// A pending call-argument value during keyword adaptation: the raw AST
/// expression (no-keyword calls — slot order coincides with written order, so
/// lowering at slot-fill time is correct), or a local STAGED in written order
/// (keyword calls — slot filling reorders arguments, and CPython evaluates
/// them left-to-right as written, so side effects must run before matching).
#[derive(Clone, Copy)]
enum ArgSrc<'e> {
    Plain(&'e Expr),
    Staged(LocalId),
}

/// One positional argument of a call, after `*` classification. A `*list` /
/// `*tuple` LITERAL spread is flattened into its [`PosItem::Plain`] elements
/// (compile-time-known arity); a runtime `*seq` spread (variable / call result
/// / comprehension) of unknown length stays a [`PosItem::Spread`].
enum PosItem<'e> {
    Plain(&'e Expr),
    Spread(&'e Expr),
}


/// How a source name in scope maps to storage (Phase 6A): directly to a local
/// slot, or through a cell held in a local slot (a captured / capturable
/// variable — the P6-2 rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Binding {
    Direct(LocalId),
    Cell(LocalId),
}

/// Where a name access lands: a scope binding, or a promoted module-global
/// slot (Phase 6B).
#[derive(Debug, Clone, Copy)]
enum Place {
    Bind(Binding),
    Global(u32),
}

pub(crate) struct FnLowerer<'a> {
    interner: &'a mut StringInterner,
    ctx: &'a AnnCtx<'a>,
    shared: &'a mut Shared,
    name: InternedString,
    /// Raw (un-interned) base name, for `{base}.<locals>.{child}#k` synthetics.
    base_name: String,
    /// The class this is a method of (`Some` for methods), for `super()` resolution.
    enclosing_class: Option<ClassId>,
    /// For a `@classmethod` body: `(class_id, interned class name)` so a bare
    /// `cls` resolves as a compile-time alias of the enclosing class — `cls.attr`
    /// / `cls.method(...)` take the static class-reference paths and `cls(...)`
    /// constructs the class. `None` everywhere else. (Static model, like
    /// `super()` / `object.__new__(cls)`: `cls` is the statically enclosing
    /// class, never a runtime subclass.)
    cls_ref: Option<(ClassId, InternedString)>,
    params: Vec<HirParam>,
    ret_ty: SemTy,
    exprs: Arena<HirExpr>,
    blocks: Arena<HirBlock>,
    locals: Vec<HirLocal>,
    scope: HashMap<InternedString, Binding>,
    /// Own names that must live in cells (from `freevars`), interned.
    celled: HashSet<InternedString>,
    /// Cell names some descendant writes via `nonlocal` (typing demotion, 6B).
    shared_writes: HashSet<InternedString>,
    /// `global`-declared names in this scope (Phase 6B), interned.
    global_decls: HashSet<InternedString>,
    /// Names this scope binds (from `freevars`) — a promoted global READ routes
    /// to its slot only when the name is not locally bound (CPython scoping).
    bound_names: HashSet<InternedString>,
    /// True for `__main__`: every promoted name lives in its global slot (the
    /// single storage), so main's own accesses rewrite to GlobalGet/GlobalSet.
    is_main: bool,
    /// Import-statement start offset → its runtime effect (`<init>` calls +
    /// `from M import VAR` snapshots). Empty except on the module-init lowerer
    /// (`main`), where it carries `ImportCollect::actions` so a module-level
    /// CONDITIONAL import (inside an `if`/`try`) can replay its action in position
    /// from `lower_stmt`. Top-level imports replay from the Phase B loop instead.
    import_actions: HashMap<u32, ImportAction>,
    /// Start offsets of every import the module scan visited. Empty except on
    /// `main`; lets `lower_stmt` accept a module-level conditional import (scanned
    /// → loaded + bound) and still reject one in a function body / loop / `with`.
    scanned_import_offsets: HashSet<u32>,
    entry: Idx<HirBlock>,
    cur: Idx<HirBlock>,
    /// Blocks already sealed with a real terminator. Open-ness is tracked here,
    /// not by inspecting the placeholder `Unreachable` terminator — an explicit
    /// `Unreachable` seal (the `raise` shape) must stay sealed (Phase 7).
    sealed: HashSet<Idx<HirBlock>>,
    /// The handler block protecting code emitted RIGHT NOW (table-based
    /// unwinding): set while lowering a `try`/`with` body, restored on exit.
    /// Stamped onto each block the first time it receives a statement or
    /// terminator ([`Self::stamp_handler`]) — fill-time, not creation-time,
    /// because join blocks are created inside a region but filled after it.
    cur_handler: Option<Idx<HirBlock>>,
    /// Blocks whose `handler` field has been stamped (distinguishes a stamped
    /// `None` from "not filled yet", and backs the one-context-per-block
    /// debug assertion).
    stamped: HashSet<Idx<HirBlock>>,
    /// Last `HirStmt::Line` marker emitted into the CURRENT block (real
    /// tracebacks). Reset on every `switch` — codegen's srcloc state follows
    /// emission order, so each block must re-establish its line.
    cur_line: Option<u32>,
    scope_stack: Vec<ScopeCtx>,
    /// Uniquifier for sibling synthetic functions (lambdas / nested defs).
    synth_counter: u32,
    /// Set when this nested function captures its OWN name (recursion): the
    /// self-capture cell's local plus this function's synthetic name. A call
    /// through that cell compiles to a direct self-call passing the env through
    /// — same shared cells, precise return type. (Documented divergence:
    /// rebinding the name in the enclosing scope after creation would not be
    /// observed by the recursion; the corpus never does that.)
    self_capture: Option<(LocalId, InternedString)>,
    /// Generator state-machine context (Phase 6E): set while lowering a resume
    /// function — yields become suspend points and `return` exhausts.
    gen: Option<GenCtx>,
}

/// Generator-lowering state (Phase 6E).
struct GenCtx {
    /// The generator object — the resume function's param 0.
    gen_local: LocalId,
    /// Next state number for a yield (state 0 = start).
    next_state: u32,
    /// `(state, resume_block)` per yield, for the entry dispatch.
    resume_targets: Vec<(u32, Idx<HirBlock>)>,
}


/// A `range()` bound argument: the literal `0` start of `range(stop)`, or an
/// arbitrary expression.
enum RangeArg<'a> {
    Zero,
    Expr(&'a Expr),
}

/// One bindable parameter of a [`UniformTarget`]: its name (for `**kwargs`
/// by-name binding), annotated type (for the checked float/bool seam), and
/// optional default.
#[derive(Clone)]
struct ThunkParam {
    name: InternedString,
    ty: SemTy,
    default: Option<ParamDefault>,
}

impl ThunkParam {
    fn from_param_info(p: &ParamInfo) -> Self {
        Self {
            name: p.name,
            ty: p.ty.clone(),
            default: p.default.clone(),
        }
    }
    fn from_hir_param(p: &HirParam) -> Self {
        Self {
            name: p.name,
            ty: p.ty.clone(),
            default: p.default.clone(),
        }
    }
}

/// The call-facing shape a single uniform thunk needs to bind `__args__` /
/// `__kwargs__` to a target function `F` and call it (the one mechanism that
/// replaces the old decorator generic thunk *and* the top-level-fn-value typed
/// thunk).
struct UniformTarget {
    /// The name resolving to `Symbol::Function(F)` — a top-level def, a renamed
    /// decorated `<orig>`, or a synthetic nested-def / lambda name.
    name: InternedString,
    /// `F`'s declared return type (the body call's type; the thunk's own return
    /// is `Dyn`, so a non-`Tagged` `F` result is boxed by the return terminator).
    ret: SemTy,
    /// Pass the closure env tuple as `F`'s leading positional — true for nested
    /// defs / lambdas (env param 0), false for top-level / decorated targets.
    pass_env: bool,
    fixed: Vec<ThunkParam>,
    kwonly: Vec<ThunkParam>,
    varargs: bool,
    kwargs: bool,
    /// Bind *fixed* (positional-or-keyword) params from `__kwargs__` too, not
    /// just positionally — true for **method** thunks (gradual-completeness
    /// dispatch, Phase B), where a `Dyn`-receiver call may pass a positional-or-
    /// keyword param by keyword (`obj.m(a, scale=2)`). False for value-call
    /// thunks (closures), whose call sites pass the null `__kwargs__` sentinel
    /// (keywords out of scope), keeping that hot path allocation-free.
    kw_bindable: bool,
}

impl UniformTarget {
    /// Derive the target shape from a top-level `def`'s [`TopDefInfo`].
    fn from_top_def(name: InternedString, info: &TopDefInfo) -> Self {
        Self {
            name,
            ret: info.ret.clone(),
            pass_env: false,
            fixed: info.fixed.iter().map(ThunkParam::from_param_info).collect(),
            kwonly: info
                .kwonly
                .iter()
                .map(ThunkParam::from_param_info)
                .collect(),
            varargs: info.varargs.is_some(),
            kwargs: info.kwargs.is_some(),
            kw_bindable: false,
        }
    }
}

/// How a callable treats its first parameter (Phase 5D).
enum FirstParam {
    /// An instance method / property accessor: param 0 is `self`, typed as the
    /// class (carried `SemTy::Class`).
    Method(SemTy),
    /// A `@classmethod`: the first param (`cls`) is dropped — it is resolved
    /// statically to the enclosing class. Skip-only: used where the body is NOT
    /// lowered (the spread-call thunk in `closures.rs`), so no alias is bound.
    SkipCls,
    /// A `@classmethod` whose body IS lowered: drop `cls` from the signature AND
    /// bind it as a compile-time alias of the enclosing class (`cls.attr` /
    /// `cls.method(...)` / `cls(...)`). Carries the class id and interned name.
    ClsMethod {
        class_id: ClassId,
        name: InternedString,
    },
    /// A free function / `@staticmethod`: no special first-param handling.
    Plain,
}

/// The parsed parameter shape of a callable (Phase 6C).
struct ParsedParams {
    fixed: Vec<ParamInfo>,
    kwonly: Vec<ParamInfo>,
    varargs: Option<InternedString>,
    kwargs: Option<InternedString>,
}

/// A `class` defined below module top level (inside a function body, inside
/// another class, or inside module-level control flow) — collected by the
/// module pre-scan so it can be registered + lowered alongside the top-level
/// classes (FIX 2). `enclosing_locals` is the union of every enclosing
/// *function* scope's bound names (empty when the class is only nested in
/// classes / module-level control flow); a method free name found in this set
/// is an enclosing-local capture and is rejected up front (A2: never silently
/// lift a capturing class to module scope).
struct NestedClass<'a> {
    def: &'a StmtClassDef,
    enclosing_locals: HashSet<String>,
}

/// A method's decorator classification (Phase 5D).
enum MethodDecor {
    Instance,
    Static,
    Class,
    Property,
    Setter(String),
}

/// Auto- vs manual-numbering mode of a `str.format` template (CPython forbids
/// mixing empty `{}` auto-indexing with explicit `{0}` indices).
#[derive(PartialEq)]
enum FmtNumbering {
    Unset,
    Auto,
    Manual,
}

/// Which argument a `str.format` replacement field binds to.
enum FmtFieldRef {
    /// `{}` — the next auto-index positional.
    Auto,
    /// `{0}` — an explicit positional index.
    Index(usize),
    /// `{name}` — a keyword argument.
    Keyword(String),
}

/// One parsed segment of a `str.format` template.
enum FmtSeg {
    /// Literal text (with `{{`/`}}` already unescaped).
    Lit(String),
    /// A replacement field `{[name][!conv][:spec]}`.
    Field {
        field: FmtFieldRef,
        conv: rustpython_parser::ast::ConversionFlag,
        spec: String,
    },
}

