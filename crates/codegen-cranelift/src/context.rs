//! Codegen context structures for function compilation

use cranelift_codegen::ir::Value;
use cranelift_frontend::{FunctionBuilder, Variable};
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

/// Symbol and mapping tables for code generation.
/// Groups variable maps, function IDs, and block maps.
pub struct CodegenSymbols<'a> {
    /// Mapping from MIR LocalId to Cranelift Variable
    pub var_map: &'a IndexMap<LocalId, Variable>,
    /// MIR local metadata (types, names, GC flags)
    pub locals: &'a IndexMap<LocalId, mir::Local>,
    /// MIR FuncId → Cranelift FuncId (for direct calls)
    pub func_ids: &'a IndexMap<FuncId, ClFuncId>,
    /// Function name → Cranelift FuncId (for cross-module calls)
    pub func_name_ids: &'a IndexMap<String, ClFuncId>,
    /// FuncId → parameter types (for type coercion at call sites)
    pub func_param_types: &'a IndexMap<FuncId, Vec<Type>>,
    /// MIR BlockId → Cranelift Block (for branches)
    pub block_map: &'a IndexMap<BlockId, cranelift_codegen::ir::Block>,
    /// Full MIR block map of the function being compiled. Consulted by
    /// terminator codegen to forward SSA φ-node sources as block-call args
    /// on each outgoing edge. Empty Phi-defs mean no augmentation — the
    /// non-SSA path is unchanged.
    pub mir_blocks: &'a IndexMap<BlockId, mir::BasicBlock>,
    /// The MIR BlockId currently being emitted. Terminators use this to
    /// know which predecessor edge they represent when gathering φ args.
    pub current_block: BlockId,
}

/// GC and stack unwinding state for function compilation.
pub struct GcState<'a> {
    /// GC frame data (shadow frame slot and root mapping), None if no GC roots
    pub frame_data: &'a Option<GcFrameData>,
    /// Cranelift FuncId for gc_pop (GC frame cleanup on return)
    pub gc_pop_id: Option<ClFuncId>,
    /// Cranelift FuncId for stack_pop (traceback cleanup on return)
    pub stack_pop_id: Option<ClFuncId>,
}

/// Debug and type metadata for the function being compiled.
pub struct DebugContext<'a> {
    /// MIR function name for diagnostics.
    pub function_name: &'a str,
    /// The function's declared return type (needed for return terminator codegen)
    pub return_type: &'a Type,
    /// Line map for source-level debug info (None when --debug is not set)
    pub line_map: Option<&'a LineMap>,
}

/// Context for code generation, grouping commonly used parameters
pub struct CodegenContext<'a> {
    pub symbols: CodegenSymbols<'a>,
    pub gc: GcState<'a>,
    pub debug: DebugContext<'a>,
    pub module: &'a mut ObjectModule,
    pub interner: &'a StringInterner,
}

impl CodegenContext<'_> {
    /// Store a computed value into a destination local and update GC roots.
    ///
    /// Combines three operations that must always happen together:
    /// 1. Look up the Cranelift `Variable` for the destination local
    /// 2. Define the variable with the computed value (`def_var`)
    /// 3. Update the GC root slot if the destination is a GC-tracked local
    pub fn store_result(&self, builder: &mut FunctionBuilder, dest: &LocalId, value: Value) {
        let var = *self
            .symbols
            .var_map
            .get(dest)
            .expect("internal error: local not in var_map - codegen bug");
        let expected_ty = self
            .symbols
            .locals
            .get(dest)
            .map(|local| crate::utils::type_to_cranelift(&local.ty))
            .expect("internal error: local metadata missing for codegen result");
        let actual_ty = builder.func.dfg.value_type(value);
        assert!(
            expected_ty == actual_ty,
            "codegen type mismatch in {} for local {:?}: declared {:?} => {:?}, value is {:?}",
            self.debug.function_name,
            dest,
            self.symbols.locals.get(dest).map(|local| &local.ty),
            expected_ty,
            actual_ty
        );
        builder.def_var(var, value);
        crate::gc::update_gc_root_if_needed(builder, dest, value, self.gc.frame_data);
    }
}
