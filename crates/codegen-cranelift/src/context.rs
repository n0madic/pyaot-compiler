//! Codegen context structures for function compilation

use cranelift_frontend::Variable;
use cranelift_module::FuncId as ClFuncId;
use cranelift_object::ObjectModule;
use indexmap::IndexMap;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{BlockId, FuncId, LineMap, LocalId, StringInterner};

/// GC frame data passed through function compilation
/// Contains information about the shadow frame and root tracking
pub struct GcFrameData {
    /// Stack slot for roots array (8 bytes per root)
    pub roots_slot: cranelift_codegen::ir::StackSlot,
    /// Mapping from LocalId to root index
    pub gc_roots: Vec<(LocalId, usize)>,
}

/// Context for code generation, grouping commonly used parameters
pub struct CodegenContext<'a> {
    pub var_map: &'a IndexMap<LocalId, Variable>,
    pub locals: &'a IndexMap<LocalId, mir::Local>,
    pub module: &'a mut ObjectModule,
    pub func_ids: &'a IndexMap<FuncId, ClFuncId>,
    pub interner: &'a StringInterner,
    pub gc_frame_data: &'a Option<GcFrameData>,
    pub block_map: &'a IndexMap<BlockId, cranelift_codegen::ir::Block>,
    pub gc_pop_id: Option<ClFuncId>,
    /// Map from function name to Cranelift FuncId (for CallNamed instruction)
    pub func_name_ids: &'a IndexMap<String, ClFuncId>,
    /// Map from FuncId to parameter types (for type coercion at call sites)
    pub func_param_types: &'a IndexMap<FuncId, Vec<Type>>,
    /// The function's declared return type (needed for return terminator codegen)
    pub return_type: &'a Type,
    /// Line map for source-level debug info (None when --debug is not set)
    pub line_map: Option<&'a LineMap>,
}
