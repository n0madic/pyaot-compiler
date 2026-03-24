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
use pyaot_utils::{BlockId, ClassId, FuncId, InternedString, LocalId, StringInterner, VarId};
use std::collections::HashMap;

/// Key function source for sort/sorted operations
pub enum KeyFuncSource {
    /// User-defined function (by FuncId, with optional captures)
    UserFunc(FuncId, Vec<hir::ExprId>),
    /// First-class builtin function (len, str, etc.)
    Builtin(mir::BuiltinFunctionKind),
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
    /// Dunder method tracking
    pub str_func: Option<FuncId>, // __str__ method
    pub repr_func: Option<FuncId>, // __repr__ method
    pub eq_func: Option<FuncId>,   // __eq__ method
    pub ne_func: Option<FuncId>,   // __ne__ method
    pub lt_func: Option<FuncId>,   // __lt__ method
    pub le_func: Option<FuncId>,   // __le__ method
    pub gt_func: Option<FuncId>,   // __gt__ method
    pub ge_func: Option<FuncId>,   // __ge__ method
    pub hash_func: Option<FuncId>, // __hash__ method
    pub len_func: Option<FuncId>,  // __len__ method
    /// Arithmetic dunders
    pub add_func: Option<FuncId>, // __add__ method
    pub sub_func: Option<FuncId>,  // __sub__ method
    pub mul_func: Option<FuncId>,  // __mul__ method
    pub truediv_func: Option<FuncId>, // __truediv__ method
    pub floordiv_func: Option<FuncId>, // __floordiv__ method
    pub mod_func: Option<FuncId>,  // __mod__ method
    pub pow_func: Option<FuncId>,  // __pow__ method
    /// Unary dunders
    pub neg_func: Option<FuncId>, // __neg__ method
    pub bool_func: Option<FuncId>, // __bool__ method
    /// Container dunders
    pub getitem_func: Option<FuncId>, // __getitem__ method
    pub setitem_func: Option<FuncId>, // __setitem__ method
    pub delitem_func: Option<FuncId>, // __delitem__ method
    pub contains_func: Option<FuncId>, // __contains__ method
    /// Iterator protocol dunders
    pub iter_func: Option<FuncId>, // __iter__ method
    pub next_func: Option<FuncId>, // __next__ method
    /// Callable dunder
    pub call_func: Option<FuncId>, // __call__ method
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
}

/// Main lowering context that transforms HIR to MIR
pub struct Lowering<'a> {
    pub(crate) interner: &'a mut StringInterner,
    pub(crate) mir_module: mir::Module,
    pub(crate) next_local_id: u32,
    pub(crate) next_block_id: u32,
    pub(crate) var_to_local: IndexMap<VarId, LocalId>,
    /// Track variable types for proper type inference
    pub(crate) var_types: IndexMap<VarId, Type>,
    pub(crate) current_blocks: Vec<mir::BasicBlock>,
    pub(crate) current_block_idx: usize,
    /// Map from function name to FuncId for resolving calls
    pub(crate) func_name_map: IndexMap<String, FuncId>,
    /// Stack of loop contexts: (continue_target, break_target)
    pub(crate) loop_stack: Vec<(BlockId, BlockId)>,
    /// Class information for field access and method calls
    pub(crate) class_info: IndexMap<ClassId, LoweredClassInfo>,
    /// Map from class name to ClassId for instantiation
    pub(crate) class_name_map: IndexMap<String, ClassId>,
    /// Track variables that hold function references (for lambda calls)
    pub(crate) var_to_func: IndexMap<VarId, FuncId>,
    /// Track variables that hold closures (func_id, captures)
    pub(crate) var_to_closure: IndexMap<VarId, (FuncId, Vec<hir::ExprId>)>,
    /// Track variables that hold decorator wrapper closures
    /// Maps: variable -> (wrapper_func_id, original_func_id)
    /// Used when a decorator returns a closure that wraps the original function
    pub(crate) var_to_wrapper: IndexMap<VarId, (FuncId, FuncId)>,
    /// Track variables that are function pointer parameters (for indirect calls)
    /// Used in wrapper functions where the captured `func` parameter is called indirectly
    pub(crate) func_ptr_params: IndexSet<VarId>,
    /// Track wrapper function IDs (closures returned by decorators)
    /// These functions have a function pointer as their first capture parameter
    pub(crate) wrapper_func_ids: IndexSet<FuncId>,
    /// Return type of the current function being lowered
    /// Used to infer the result type of indirect calls through func_ptr_params
    pub(crate) current_func_return_type: Option<Type>,
    /// Track inferred return types for functions (especially lambdas)
    pub(crate) func_return_types: IndexMap<FuncId, Type>,
    /// Track captured variable types for closures (used during lambda lowering)
    pub(crate) closure_capture_types: IndexMap<FuncId, Vec<Type>>,
    /// Caller-provided parameter type hints for lambdas (e.g., reduce passes element type for both params)
    pub(crate) lambda_param_type_hints: IndexMap<FuncId, Vec<Type>>,
    /// Track global variables (shared across all functions via runtime storage)
    pub(crate) globals: IndexSet<VarId>,
    /// Track types of global variables (preserved across function boundaries)
    pub(crate) global_var_types: IndexMap<VarId, Type>,
    /// Variables that need to be wrapped in cells (used by inner functions via nonlocal)
    pub(crate) cell_vars: IndexSet<VarId>,
    /// Map nonlocal variables to their cell local (for reading/writing through cells)
    pub(crate) nonlocal_cells: IndexMap<VarId, LocalId>,
    /// Expected type for the current expression being lowered (set by assignment context).
    /// Used by empty list/dict/set literals to infer the correct elem_tag.
    pub(crate) expected_type: Option<Type>,
    /// Track original types of narrowed Union variables (for unboxing during reads)
    /// Key: VarId, Value: Original Union type before narrowing
    pub(crate) narrowed_union_vars: IndexMap<VarId, Type>,
    /// Mapping from (module_name, var_name) to (VarId, Type) for cross-module variable access
    pub(crate) module_var_exports: HashMap<(String, String), (VarId, Type)>,
    /// Mapping from (module_name, func_name) to return Type for cross-module function calls
    pub(crate) module_func_exports: HashMap<(String, String), Type>,
    /// Mapping from (module_name, class_name) to (ClassId, class_name_string) for cross-module class instantiation
    pub(crate) module_class_exports: HashMap<(String, String), (ClassId, String)>,
    /// Cross-module class information (field_offsets, field_types, method_return_types) for field/method access
    /// Uses String keys for field/method names to work across different interners
    pub(crate) cross_module_class_info: HashMap<ClassId, CrossModuleClassInfo>,
    /// VarId offset for this module (to avoid collisions with other modules)
    pub(crate) var_id_offset: u32,
    /// ClassId offset for this module (to avoid collisions with other modules)
    pub(crate) class_id_offset: u32,
    /// Pre-built varargs tuple from list unpacking (used during resolve_call_args)
    pub(crate) pending_varargs_from_unpack: Option<LocalId>,
    /// Runtime kwargs dict from **kwargs unpacking (used during resolve_call_args)
    /// Contains (dict_operand, value_type) for extracting kwargs at runtime
    pub(crate) pending_kwargs_from_unpack: Option<(LocalId, Type)>,
    /// Refined types for variables from empty container analysis.
    /// Persists across function lowerings (unlike var_types which is cleared per function).
    /// Used to give empty lists/sets the correct elem_tag based on subsequent usage.
    pub(crate) refined_var_types: IndexMap<VarId, Type>,
    /// Memoized expression types — persists across functions (ExprIds are unique per-module).
    /// Replaces the former RefCell<HashMap> expr_type_cache.
    pub(crate) expr_types: HashMap<hir::ExprId, Type>,
    /// Storage for mutable default parameter values.
    /// Maps (FuncId, param_index) to global slot ID.
    /// In Python, mutable defaults (list, dict, set, class instances) are evaluated once
    /// at function definition time and shared across all calls.
    pub(crate) default_value_slots: IndexMap<(FuncId, usize), u32>,
    /// Counter for allocating default value global slots.
    /// Uses a separate namespace (starting at high value) to avoid collision with regular globals.
    pub(crate) next_default_slot: u32,
    /// Warnings collected during lowering (dead code, etc.)
    pub(crate) warnings: CompilerWarnings,
    /// Track module-level variables that hold decorator wrapper closures (persists across function lowering)
    /// Maps: variable -> (wrapper_func_id, original_func_id)
    /// Used when a module-level decorated function is called from other functions
    pub(crate) module_var_wrappers: IndexMap<VarId, (FuncId, FuncId)>,
    /// Track module-level variables that hold function references (persists across function lowering)
    /// Used when a module-level function reference is called from other functions
    pub(crate) module_var_funcs: IndexMap<VarId, FuncId>,
}
