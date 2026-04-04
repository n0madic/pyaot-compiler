//! Core lowering context and module/function entry points
//!
//! This module is decomposed into submodules for maintainability:
//! - `constructors`: Context creation and initialization
//! - `function_lowering`: Module and function lowering entry points
//! - `locals`: Local variable and basic block allocation
//! - `helpers`: Utility methods for type conversion, isinstance, etc.
//! - `accessors`: Getter/setter methods for internal state

mod accessors;
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
use pyaot_types::Type;
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
/// Uses String keys for field/method names to work across different interners.
#[derive(Debug, Clone, Default)]
pub struct CrossModuleClassInfo {
    /// Map field name to its offset in the instance
    pub field_offsets: HashMap<String, usize>,
    /// Map field name to its type
    pub field_types: HashMap<String, Type>,
    /// Map method name to its return type
    pub method_return_types: HashMap<String, Type>,
    /// Total field count including inherited fields (for instance allocation)
    pub total_field_count: usize,
}

/// All recognized dunder method names (excluding __init__ which is handled separately).
/// Used by `set_dunder_func` to validate whether a method name is a tracked dunder.
const KNOWN_DUNDERS: &[&str] = &[
    "__str__",
    "__repr__",
    "__eq__",
    "__ne__",
    "__lt__",
    "__le__",
    "__gt__",
    "__ge__",
    "__hash__",
    "__len__",
    "__add__",
    "__sub__",
    "__mul__",
    "__truediv__",
    "__floordiv__",
    "__mod__",
    "__pow__",
    "__radd__",
    "__rsub__",
    "__rmul__",
    "__rtruediv__",
    "__rfloordiv__",
    "__rmod__",
    "__rpow__",
    "__and__",
    "__or__",
    "__xor__",
    "__lshift__",
    "__rshift__",
    "__rand__",
    "__ror__",
    "__rxor__",
    "__rlshift__",
    "__rrshift__",
    "__matmul__",
    "__rmatmul__",
    "__neg__",
    "__pos__",
    "__abs__",
    "__invert__",
    "__bool__",
    "__int__",
    "__float__",
    "__getitem__",
    "__setitem__",
    "__delitem__",
    "__contains__",
    "__iter__",
    "__next__",
    "__call__",
    "__index__",
    "__format__",
    "__del__",
    "__new__",
    "__copy__",
    "__deepcopy__",
];

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
    /// Note: `__init__` is intentionally excluded — it is handled separately via
    /// `class_def.init_method` and stored in `init_func` by the caller.
    pub fn set_dunder_func(&mut self, name: &str, func_id: FuncId) -> bool {
        if let Some(&static_name) = KNOWN_DUNDERS.iter().find(|&&n| n == name) {
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
}

/// Cross-module imports, exports, and offsets
pub struct ModuleState {
    /// (module_name, var_name) → (VarId, Type) for cross-module variable access
    pub module_var_exports: HashMap<(String, String), (VarId, Type)>,
    /// (module_name, func_name) → return Type for cross-module function calls
    pub module_func_exports: HashMap<(String, String), Type>,
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
}

/// Variable names → local IDs, function references, global tracking
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

/// Type tracking: variable types, expression cache, narrowing
pub struct TypeEnvironment {
    /// Track variable types for proper type inference (cleared per function)
    pub var_types: IndexMap<VarId, Type>,
    /// Memoized expression types — persists across functions (ExprIds are unique per-module)
    pub expr_types: HashMap<hir::ExprId, Type>,
    /// Refined types for variables from empty container analysis (persists across functions)
    pub refined_var_types: IndexMap<VarId, Type>,
    /// Track original types of narrowed Union variables (for unboxing during reads)
    pub narrowed_union_vars: IndexMap<VarId, Type>,
    /// Track inferred return types for functions (especially lambdas)
    pub func_return_types: IndexMap<FuncId, Type>,
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
/// - `types`: Type inference and expression type cache
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
    /// Type inference and expression type cache
    pub(crate) types: TypeEnvironment,
    /// Warnings collected during lowering
    pub(crate) warnings: CompilerWarnings,
}
