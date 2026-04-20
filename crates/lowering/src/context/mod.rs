//! Core lowering context and module/function entry points
//!
//! This module is decomposed into submodules for maintainability:
//! - `constructors`: Context creation and initialization
//! - `function_lowering`: Module and function lowering entry points
//! - `locals`: Local variable and basic block allocation
//! - `helpers`: Utility methods for type conversion, isinstance, etc.
//! - `accessors`: Getter/setter methods for internal state

mod accessors;
mod cfg_walker;
mod constructors;
mod function_lowering;
mod helpers;
mod locals;

// Re-export FuncOrBuiltin for use in iteration.rs
pub use helpers::FuncOrBuiltin;

use indexmap::IndexMap;
use indexmap::IndexSet;
use pyaot_diagnostics::CompilerWarnings;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::{dunders::canonical_dunder_name, Type};
use pyaot_utils::{BlockId, ClassId, FuncId, InternedString, LocalId, Span, StringInterner, VarId};
use std::collections::HashMap;

/// Key function source for sort/sorted operations
pub enum KeyFuncSource {
    /// User-defined function (by FuncId, with optional captures)
    UserFunc(FuncId, Vec<hir::ExprId>),
    /// First-class builtin function (len, str, etc.)
    Builtin(mir::BuiltinFunctionKind),
}

/// Resolved key function info: function pointer + captures for runtime calls
pub struct ResolvedKeyFunc {
    /// The key function address operand
    pub func_addr: mir::Operand,
    /// Captures tuple operand (0 / null if no captures)
    pub captures: mir::Operand,
    /// Capture count operand
    pub capture_count: mir::Operand,
}

/// Parsed sort kwargs (key= and reverse=) for list.sort() and sorted()
pub struct SortKwargs {
    /// The reverse operand (lowered from reverse= kwarg or default false)
    pub reverse: mir::Operand,
    /// The key function source if key= kwarg was provided
    pub key_func: Option<KeyFuncSource>,
}

/// Cross-module class information for field and method access across module boundaries.
///
/// Uses InternedString keys, re-interned into each module's interner during setup.
#[derive(Debug, Clone, Default)]
pub struct CrossModuleClassInfo {
    /// Map field name to its offset in the instance
    pub field_offsets: IndexMap<InternedString, usize>,
    /// Map field name to its type
    pub field_types: IndexMap<InternedString, Type>,
    /// Map method name to its return type
    pub method_return_types: IndexMap<InternedString, Type>,
    /// Total field count including inherited fields (for instance allocation)
    pub total_field_count: usize,
}

/// Class information for lowering (compiled from HIR ClassDef)
///
/// Contains field layout, method mapping, and vtable information for virtual dispatch.
/// Fields support single inheritance with inherited fields placed before own fields.
/// Virtual dispatch is implemented via vtable_slots mapping method names to slot indices.
#[derive(Debug, Clone)]
pub struct LoweredClassInfo {
    /// Map field name to offset (0-based, includes inherited fields)
    pub field_offsets: IndexMap<InternedString, usize>,
    /// Map field name to type
    pub field_types: IndexMap<InternedString, Type>,
    /// Map instance method name to FuncId (only regular instance methods, not static/class)
    pub method_funcs: IndexMap<InternedString, FuncId>,
    /// The __init__ method FuncId if present
    pub init_func: Option<FuncId>,
    /// Dunder methods — unified storage keyed by dunder name (e.g., "__str__", "__eq__")
    pub dunder_methods: IndexMap<&'static str, FuncId>,
    /// Base class ID for single inheritance (None if no parent)
    pub base_class: Option<ClassId>,
    /// Total field count including inherited fields
    pub total_field_count: usize,
    /// Offset where this class's own fields start (after inherited fields)
    /// Used in class_metadata.rs for computing inherited field layout
    pub own_field_offset: usize,
    /// Map method name to vtable slot index for virtual dispatch
    /// Used in class_metadata.rs for vtable building and expressions/access for CallVirtual
    pub vtable_slots: IndexMap<InternedString, usize>,
    /// Map class attribute name to (owning_class_id, offset index) for class attribute storage
    /// The owning_class_id is the class where the attribute is actually defined (for inheritance)
    pub class_attr_offsets: IndexMap<InternedString, (ClassId, usize)>,
    /// Map class attribute name to type
    pub class_attr_types: IndexMap<InternedString, Type>,
    /// Map static method name to FuncId (@staticmethod - no self/cls)
    pub static_methods: IndexMap<InternedString, FuncId>,
    /// Map class method name to FuncId (@classmethod - receives cls)
    pub class_methods: IndexMap<InternedString, FuncId>,
    /// Map property name to (getter FuncId, optional setter FuncId)
    pub properties: IndexMap<InternedString, (FuncId, Option<FuncId>)>,
    /// Map property name to property type (return type of getter)
    pub property_types: IndexMap<InternedString, Type>,
    /// Whether this class is an exception class (inherits from Exception)
    pub is_exception_class: bool,
}

impl LoweredClassInfo {
    /// Look up a dunder method by name.
    /// Returns `None` for non-dunder names or dunders not defined on this class.
    pub fn get_dunder_func(&self, name: &str) -> Option<FuncId> {
        self.dunder_methods.get(name).copied()
    }

    /// Set a dunder method by name. Returns `true` if the name was recognized as a
    /// tracked dunder, `false` if the caller should treat it as a regular method.
    ///
    /// Uses `pyaot_types::dunders::canonical_dunder_name` as the single source of truth
    /// for recognized dunder names.
    ///
    /// Note: `__init__` is intentionally excluded — it is handled separately via
    /// `class_def.init_method` and stored in `init_func` by the caller.
    pub fn set_dunder_func(&mut self, name: &str, func_id: FuncId) -> bool {
        // __init__ is routed to init_func by the caller; do not store it here.
        if name == "__init__" {
            return false;
        }
        if let Some(static_name) = canonical_dunder_name(name) {
            self.dunder_methods.insert(static_name, func_id);
            true
        } else {
            false
        }
    }
}

// =============================================================================
// Sub-structs: decompose Lowering god-object into focused contexts (Phase 3.2)
// =============================================================================

/// Closure and decorator tracking: captures, wrappers, dynamic vars
pub struct ClosureState {
    /// Track variables that hold closures (func_id, captures)
    pub var_to_closure: IndexMap<VarId, (FuncId, Vec<hir::ExprId>)>,
    /// Track variables that hold decorator wrapper closures (wrapper_func_id, original_func_id)
    pub var_to_wrapper: IndexMap<VarId, (FuncId, FuncId)>,
    /// Variables holding dynamically returned closures (need emit_closure_call dispatch)
    pub dynamic_closure_vars: IndexSet<VarId>,
    /// Captured variable types for closures (used during lambda lowering)
    pub closure_capture_types: IndexMap<FuncId, Vec<Type>>,
    /// Wrapper function IDs (closures returned by decorators)
    pub wrapper_func_ids: IndexSet<FuncId>,
    /// Variables that are function pointer parameters (for indirect calls)
    pub func_ptr_params: IndexSet<VarId>,
    /// VarPositional (*args) parameter VarIds for the current function
    pub varargs_params: IndexSet<VarId>,
    /// Caller-provided parameter type hints for lambdas
    pub lambda_param_type_hints: IndexMap<FuncId, Vec<Type>>,
    /// Maps original (decorated) function ID to its wrapper function ID.
    /// Populated during pre-scan; used to look up the original function for a wrapper.
    pub decorated_to_wrapper: IndexMap<FuncId, FuncId>,
    /// Maps wrapper function ID to the decorator's function-parameter name.
    /// Used by function_lowering.rs to detect the func-ptr param by name regardless
    /// of whether the user named it "func", "f", "fn", etc.
    pub wrapper_func_param_name: IndexMap<FuncId, InternedString>,
}

/// A simple-constant default value for a user-function parameter that can be
/// materialised across module boundaries. Complex defaults (expressions,
/// collections, references) are represented as `None` — callers must pass
/// those args explicitly.
#[derive(Debug, Clone)]
pub enum SimpleDefault {
    None,
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
}

/// Cross-module-visible parameter info for a user-defined function.
#[derive(Debug, Clone)]
pub struct ExportedParam {
    pub name: String,
    /// Materialised default, if the parameter has one we can encode as a
    /// simple constant. Parameters without a default (or with a complex
    /// default expression) have `None` — callers must supply the arg
    /// explicitly and will get a `None` fill otherwise.
    pub default: Option<SimpleDefault>,
}

/// Cross-module imports, exports, and offsets
pub struct ModuleState {
    /// (module_name, var_name) → (VarId, Type) for cross-module variable access
    pub module_var_exports: HashMap<(String, String), (VarId, Type)>,
    /// (module_name, func_name) → return Type for cross-module function calls
    pub module_func_exports: HashMap<(String, String), Type>,
    /// (module_name, func_name) → ordered parameter list for cross-module
    /// user-function calls. Used by `lower_imported_call` to map keyword
    /// arguments to positional slots and fill unset slots with simple
    /// defaults. Absent entries fall back to pass-through positional calls.
    pub module_func_params: HashMap<(String, String), Vec<ExportedParam>>,
    /// (module_name, class_name) → (ClassId, class_name_string) for cross-module class instantiation
    pub module_class_exports: HashMap<(String, String), (ClassId, String)>,
    /// Cross-module class information for field/method access
    pub cross_module_class_info: HashMap<ClassId, CrossModuleClassInfo>,
    /// VarId offset for this module (to avoid collisions with other modules)
    pub var_id_offset: u32,
    /// ClassId offset for this module (to avoid collisions with other modules)
    pub class_id_offset: u32,
    /// Module-level variables that hold decorator wrapper closures (persists across function lowering)
    pub module_var_wrappers: IndexMap<VarId, (FuncId, FuncId)>,
    /// Module-level variables that hold function references (persists across function lowering)
    pub module_var_funcs: IndexMap<VarId, FuncId>,
}

/// Class metadata: lowered class info, vtables, name mapping
pub struct ClassRegistry {
    /// Class information for field access and method calls
    pub class_info: IndexMap<ClassId, LoweredClassInfo>,
    /// Map from class name to ClassId for instantiation
    pub class_name_map: IndexMap<String, ClassId>,
}

/// §1.17b-c — per-iter-expr state stored in `CodeGenState::iter_cache`.
/// `IterSetup` picks the variant based on the iterable kind and stashes
/// the relevant locals; `IterHasNext` / `IterAdvance` dispatch on the
/// variant to emit either the iterator protocol or indexed iteration.
#[derive(Debug, Clone)]
pub enum IterState {
    /// Iterator-protocol dispatch. Used for Generator/Iterator kinds
    /// (after Dict/Set/Str/Bytes moved to Indexed).
    /// `iter_local` holds the heap IteratorObj. `value_local` stores the
    /// most-recent next() result — populated by IterHasNext (which
    /// calls next FIRST, then checks exhausted) and consumed by
    /// IterAdvance (which just binds value_local to target). Matches
    /// tree walker's `lower_for_iterator` semantics exactly.
    Protocol {
        iter_local: LocalId,
        value_local: LocalId,
        elem_type: Type,
    },
    /// Class iterator dispatch. `iter_local` holds the result of
    /// `__iter__()`, `value_local` caches the most-recent `__next__()`
    /// result, and `next_func_id` is called behind a small
    /// `TrySetjmp`/`StopIteration` shim in `IterHasNext`.
    Class {
        iter_local: LocalId,
        value_local: LocalId,
        elem_type: Type,
        next_func_id: FuncId,
    },
    /// Indexed dispatch. Used for List and Tuple — mirrors the
    /// `lower_for_iterable` fast path which sidesteps the iterator
    /// object and reads directly via `rt_list_get_typed` /
    /// `rt_tuple_get_X`. Matches the tree walker's optimized path
    /// so CFG walker output is semantically equivalent.
    ///
    /// `IterHasNext` emits `BinOp::Lt(idx_local, len_local)`.
    /// `IterAdvance` emits typed get + bind target, then increments
    /// `idx_local` (the increment must happen AFTER the bind so the
    /// bound value corresponds to the pre-increment index).
    Indexed {
        container_local: LocalId,
        idx_local: LocalId,
        len_local: LocalId,
        elem_type: Type,
        kind: IterableKindCached,
    },
    /// Range-specific dispatch. Used for `for i in range(...)`.
    /// Mirrors `lower_for_range`'s direct counter loop — no iterator
    /// object allocation, no boxing/unboxing. `idx_local` starts at
    /// `start` and increments by `step`; `stop_local` is the bound.
    ///
    /// For positive step: `idx < stop` is has-next.
    /// For negative step: `idx > stop` is has-next.
    /// Direction is tracked via `step_is_negative_local` (a bool) —
    /// used by `IterHasNext` to pick the comparison operator.
    ///
    /// Avoids the range-iterator-protocol path's heap alloc + box +
    /// unbox triple that otherwise segfaults at module-init level.
    Range {
        idx_local: LocalId,
        stop_local: LocalId,
        step_local: LocalId,
        step_is_negative: StepDirection,
    },
}

/// Compile-time direction of `range()`'s step argument. If we can
/// determine the step's sign at lowering time, we skip the runtime
/// direction check — `range(0, 10)` and `range(0, 10, -1)` are both
/// directly specializable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepDirection {
    Positive,
    Negative,
    /// Step is a runtime variable; pick direction via runtime check.
    Unknown,
}

/// Subset of `crate::utils::IterableKind` cached in `IterState::Indexed`
/// to pick the right `rt_X_get` / `rt_X_len` at `IterAdvance` /
/// `IterHasNext` time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IterableKindCached {
    List,
    Tuple,
    Str,
    Bytes,
}

/// MIR construction: blocks, instructions, current position
pub struct CodeGenState {
    pub next_local_id: u32,
    pub next_block_id: u32,
    pub current_blocks: Vec<mir::BasicBlock>,
    pub current_block_idx: usize,
    /// Stack of loop contexts: (continue_target, break_target)
    pub loop_stack: Vec<(BlockId, BlockId)>,
    /// Current source span (set by lower_stmt/lower_expr, used by emit_instruction for debug info)
    pub current_span: Option<Span>,
    /// Expected type for the current expression (set by assignment context)
    pub expected_type: Option<Type>,
    /// Pre-built varargs tuple from list unpacking (used during resolve_call_args)
    pub pending_varargs_from_unpack: Option<LocalId>,
    /// Runtime kwargs dict from **kwargs unpacking
    pub pending_kwargs_from_unpack: Option<(LocalId, Type)>,
    /// §1.17b-c — per-function iterator cache for `ExprKind::IterHasNext`
    /// / `StmtKind::IterAdvance` lowering. `IterSetup` populates the
    /// appropriate variant; subsequent IterHasNext / IterAdvance for the
    /// same ExprId read from the cache.
    pub iter_cache: IndexMap<pyaot_hir::ExprId, IterState>,
}

/// Variable names → local IDs, function references, global tracking, per-function type state
pub struct SymbolTable {
    /// Map variable to local ID within current function
    pub var_to_local: IndexMap<VarId, LocalId>,
    /// Map variable to function reference
    pub var_to_func: IndexMap<VarId, FuncId>,
    /// Map from function name to FuncId for resolving calls
    pub func_name_map: IndexMap<String, FuncId>,
    /// Global variable VarIds
    pub globals: IndexSet<VarId>,
    /// Types of global variables (preserved across function boundaries)
    pub global_var_types: IndexMap<VarId, Type>,
    /// Track variable types for type inference (cleared per function).
    /// Populated during lowering as assignments are processed.
    /// Distinct from `HirTypeInference::refined_var_types` (set during type planning).
    pub var_types: IndexMap<VarId, Type>,
    // The three legacy pre-scan / narrowing maps (`prescan_var_types`,
    // `per_function_prescan_var_types`, `narrowed_union_vars`) moved to
    // `HirTypeInference` in S1.9c (Phase 1 §1.4) — see `Lowering::hir_types`.
    /// Variables that need to be wrapped in cells (used by inner functions via nonlocal)
    pub cell_vars: IndexSet<VarId>,
    /// Map nonlocal variables to their cell local
    pub nonlocal_cells: IndexMap<VarId, LocalId>,
    /// Storage for mutable default parameter values: (FuncId, param_index) → global slot ID
    pub default_value_slots: IndexMap<(FuncId, usize), u32>,
    /// Counter for allocating default value global slots
    pub next_default_slot: u32,
    /// Return type of the current function being lowered
    pub current_func_return_type: Option<Type>,
}

/// Type planning results: populated during type planning, **immutable during lowering**.
///
/// After `run_type_planning()` completes, this struct is only READ by lowering:
/// - `expr_types`: memoized expression types
///
/// `refined_var_types` moved to `HirTypeInference::refined_var_types` in
/// Unified HIR-level type-inference state — Phase 1 §1.4.
///
/// Collects the four legacy `SymbolTable` / `TypeEnvironment` maps into one
/// owned struct on `Lowering` plus the narrowing stack that replaces the
/// legacy `apply_narrowings` / `restore_types` pair. §1.4u step 2 folded
/// the standalone `TypeEnvironment::expr_types` memoization cache into
/// this struct so there is a single HIR-type-inference owner; access it
/// via the `lookup(expr_id)` accessor, which is the forward-compatible
/// API for §1.4u-b (lowering reads exclusively from
/// `HirTypeInference::lookup`).
pub struct HirTypeInference {
    /// Pre-scanned unified types for locals (Area E §E.6). Populated by
    /// `precompute_var_types` before each function's body is lowered.
    /// When present, `get_or_create_local` uses the pre-scan type to size
    /// the MIR local so later rebinds with wider numeric / incompatible
    /// types can still be stored. Cleared per function from
    /// `per_function_prescan_var_types[func_id]`.
    pub prescan_var_types: IndexMap<VarId, Type>,
    /// Per-function pre-scan results (Area E §E.6). Computed during
    /// `run_type_planning` so that `infer_all_return_types` can see
    /// unified local types when inferring `return x`. Survives across
    /// functions; `lower_function` copies the relevant entry into
    /// `prescan_var_types` for the current function.
    pub per_function_prescan_var_types: IndexMap<FuncId, IndexMap<VarId, Type>>,
    /// Track original types of narrowed Union variables (for unboxing
    /// during reads). Cleared per function.
    pub narrowed_union_vars: IndexMap<VarId, Type>,
    /// Refined types for variables from empty container analysis
    /// (persists across functions).
    pub refined_var_types: IndexMap<VarId, Type>,
    /// Stack of active narrowing frames. Pushed by
    /// `Lowering::push_narrowing_frame` when entering an
    /// `isinstance`-narrowed branch, popped by
    /// `Lowering::pop_narrowing_frame` on exit. Replaces the legacy
    /// `apply_narrowings` / `restore_types` pair that returned and
    /// consumed an `IndexMap<VarId, Type>` explicitly — S1.9d moves
    /// that data to this internal stack so callers don't have to thread
    /// the saved state through their own scope.
    pub narrowing_stack: Vec<NarrowingFrame>,
    /// Memoized expression types — persists across functions (ExprIds
    /// are unique per-module). Moved here from the deleted
    /// `TypeEnvironment` in §1.4u step 2. Access via `lookup()` /
    /// `insert_type()` where possible; direct field access is still
    /// available for the memoization fast path in `get_type_of_expr_id`.
    pub expr_types: HashMap<hir::ExprId, Type>,
    /// §1.4u-b: persistent per-module map of every variable's **base**
    /// type. Populated once at the end of `run_type_planning` by the
    /// eager HIR-type-cache walk, from: every function's annotated
    /// parameters (`hir::Param::ty`), every function's prescan-inferred
    /// local types (`per_function_prescan_var_types`), and module-level
    /// globals. Never mutated during lowering — independent of
    /// narrowing. Consulted by `get_base_var_type` so `compute_expr_type`
    /// can be a pure function of HIR + stable state and its results
    /// can be cached at the module level.
    pub base_var_types: IndexMap<VarId, Type>,
}

/// One narrowing scope's undo information. Produced by
/// `push_narrowing_frame` and consumed by `pop_narrowing_frame`. Never
/// inspected by callers — opaque stack entry.
pub struct NarrowingFrame {
    /// Original var_types values overwritten by the narrowing — restored
    /// on pop so post-branch code sees the pre-narrowing type.
    pub saved_var_types: IndexMap<VarId, Type>,
    /// Variables whose `narrowed_union_vars` tracking was added by this
    /// push — removed on pop so later branches see a clean slate.
    pub added_union_tracking: Vec<VarId>,
}

impl HirTypeInference {
    pub fn new() -> Self {
        Self {
            prescan_var_types: IndexMap::new(),
            per_function_prescan_var_types: IndexMap::new(),
            narrowed_union_vars: IndexMap::new(),
            refined_var_types: IndexMap::new(),
            narrowing_stack: Vec::new(),
            expr_types: HashMap::new(),
            base_var_types: IndexMap::new(),
        }
    }

    /// Forward-compatible HIR type-query API — §1.4u-b target. Returns
    /// the memoized type for `expr_id` if already computed; `None`
    /// otherwise. Callers that need compute-on-miss semantics must
    /// continue to use `Lowering::get_type_of_expr_id`. §1.4u-b will
    /// migrate all ~124 post-planning `get_type_of_expr_id` call sites
    /// to read exclusively through this accessor once the memoization
    /// is guaranteed populated by a preceding HIR type pass.
    pub fn lookup(&self, expr_id: hir::ExprId) -> Option<&Type> {
        self.expr_types.get(&expr_id)
    }

    /// Insert a computed type into the memoization cache. Used by the
    /// lowering-path `compute_expr_type` wrapper to seed the cache.
    pub fn insert_type(&mut self, expr_id: hir::ExprId, ty: Type) {
        self.expr_types.insert(expr_id, ty);
    }
}

impl Default for HirTypeInference {
    fn default() -> Self {
        Self::new()
    }
}

/// Mutable type state that evolves during both type planning and lowering.
///
/// Separated from `TypeEnvironment` to keep it immutable after planning.
pub struct FuncReturnTypes {
    /// Inferred return types for functions (especially lambdas).
    /// Populated during type planning, extended during lowering when
    /// actual return types are discovered.
    pub inner: IndexMap<FuncId, Type>,
}

/// State for the "may return NotImplemented" analysis.
///
/// Filled lazily on first query by `func_may_return_not_implemented`
/// in `type_planning::ni_analysis`. Shared between binary-op dispatch and
/// builtin-reduction dispatch so both emit the §3.3.8 fallback on the
/// same set of dunders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NiState {
    Yes,
    No,
    /// Visiting the function body — treated as `No` on recursive re-entry
    /// to break cycles. Upon return we commit the final state.
    Computing,
}

#[derive(Debug, Default)]
pub struct NiAnalysis {
    pub cache: IndexMap<FuncId, NiState>,
}

// =============================================================================
// Main lowering context
// =============================================================================

/// Main lowering context that transforms HIR to MIR.
///
/// Decomposed into focused sub-structs for maintainability:
/// - `closures`: Closure and decorator tracking
/// - `modules`: Cross-module imports/exports
/// - `classes`: Class metadata and vtables
/// - `codegen`: MIR construction state
/// - `symbols`: Variable/function name resolution
/// - `hir_types`: HIR type inference state (legacy maps + expr_types cache + narrowing stack)
pub struct Lowering<'a> {
    pub(crate) interner: &'a mut StringInterner,
    pub(crate) mir_module: mir::Module,
    /// Closure and decorator tracking
    pub(crate) closures: ClosureState,
    /// Cross-module imports/exports
    pub(crate) modules: ModuleState,
    /// Class metadata and vtables
    pub(crate) classes: ClassRegistry,
    /// MIR construction state (blocks, locals, loops)
    pub(crate) codegen: CodeGenState,
    /// Variable/function name resolution
    pub(crate) symbols: SymbolTable,
    /// Unified HIR-level type-inference state (Phase 1 §1.4 / S1.9c /
    /// §1.4u step 2). Single owner of all HIR-type state: the four
    /// legacy prescan/narrowing/refinement maps plus the memoized
    /// `expr_types` cache that previously lived in the separate
    /// `TypeEnvironment` struct (deleted).
    pub(crate) hir_types: HirTypeInference,
    /// Inferred function return types (mutable during lowering)
    pub(crate) func_return_types: FuncReturnTypes,
    /// Inter-procedural `NotImplemented` analysis (filled lazily)
    pub(crate) ni_analysis: NiAnalysis,
    /// Warnings collected during lowering
    pub(crate) warnings: CompilerWarnings,
}
