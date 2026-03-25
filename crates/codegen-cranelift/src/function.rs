//! Function declaration and definition
//!
//! This module handles code generation for MIR functions including
//! function declaration, definition, GC prologue generation, and
//! the main entry point generation.

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, StackSlotData, StackSlotKind};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId as ClFuncId, Linkage, Module};
use cranelift_object::ObjectModule;
use indexmap::IndexMap;
use pyaot_diagnostics::Result;
use pyaot_mir as mir;
use pyaot_utils::{FuncId, LineMap, LocalId, StringInterner};

use crate::context::{CodegenContext, GcFrameData};
use crate::instructions::compile_instruction;
use crate::terminators::compile_terminator;
use crate::utils::{mangle_function_name, type_to_cranelift};

/// Context for function compilation, grouping related parameters
pub struct FunctionCompiler<'a> {
    pub module: &'a mut ObjectModule,
    pub ctx: &'a mut Context,
    pub func_builder_ctx: &'a mut FunctionBuilderContext,
    pub func_ids: &'a IndexMap<FuncId, ClFuncId>,
    pub func_name_ids: &'a IndexMap<String, ClFuncId>,
    pub func_param_types: &'a IndexMap<FuncId, Vec<pyaot_types::Type>>,
    pub interner: &'a StringInterner,
    pub gc_push_id: Option<ClFuncId>,
    pub gc_pop_id: Option<ClFuncId>,
    pub line_map: Option<&'a LineMap>,
}

/// Declare a MIR function in the Cranelift module
pub fn declare_function(module: &mut ObjectModule, func: &mir::Function) -> Result<ClFuncId> {
    let mut sig = module.make_signature();
    sig.call_conv = CallConv::SystemV;

    // Add parameters
    for param in &func.params {
        let cl_type = type_to_cranelift(&param.ty);
        sig.params.push(AbiParam::new(cl_type));
    }

    // Add return type (skip for None/void, but include Bool which is also I8)
    let ret_type = type_to_cranelift(&func.return_type);
    if !matches!(func.return_type, pyaot_types::Type::None) {
        sig.returns.push(AbiParam::new(ret_type));
    }

    // Mangle function name to avoid conflicts with C reserved names
    let func_name = mangle_function_name(&func.name);

    // Declare function - use mangled function name
    let func_id = module
        .declare_function(&func_name, Linkage::Export, &sig)
        .expect("failed to declare runtime function");

    Ok(func_id)
}

/// Define a MIR function in the Cranelift module
pub fn define_function(
    compiler: &mut FunctionCompiler,
    func: &mir::Function,
    cl_func_id: ClFuncId,
) -> Result<()> {
    compiler.ctx.clear();
    compiler.ctx.func.signature = compiler.module.make_signature();
    compiler.ctx.func.signature.call_conv = CallConv::SystemV;

    // Set up signature (same as declare)
    for param in &func.params {
        let cl_type = type_to_cranelift(&param.ty);
        compiler
            .ctx
            .func
            .signature
            .params
            .push(AbiParam::new(cl_type));
    }
    // Add return type (skip for None/void, but include Bool which is also I8)
    let ret_type = type_to_cranelift(&func.return_type);
    if !matches!(func.return_type, pyaot_types::Type::None) {
        compiler
            .ctx
            .func
            .signature
            .returns
            .push(AbiParam::new(ret_type));
    }

    // Count GC roots - locals that need GC tracking
    let gc_roots: Vec<(LocalId, usize)> = func
        .locals
        .iter()
        .filter(|(_, l)| l.is_gc_root)
        .enumerate()
        .map(|(idx, (local_id, _))| (*local_id, idx))
        .collect();
    let nroots = gc_roots.len();

    let mut builder = FunctionBuilder::new(&mut compiler.ctx.func, compiler.func_builder_ctx);

    // Create entry block
    let entry_block = builder.create_block();
    builder.append_block_params_for_function_params(entry_block);
    builder.switch_to_block(entry_block);

    // Map MIR locals to Cranelift variables
    let mut var_map = IndexMap::new();
    for (local_id, local) in func.locals.iter() {
        let var = builder.declare_var(type_to_cranelift(&local.ty));
        var_map.insert(*local_id, var);
    }

    // Map parameters to variables
    let entry_params = builder.block_params(entry_block).to_vec();
    for (idx, param) in func.params.iter().enumerate() {
        if let Some(&var) = var_map.get(&param.id) {
            builder.def_var(var, entry_params[idx]);
        }
    }

    // Generate GC frame prologue if we have roots
    let gc_frame_data = if nroots > 0 {
        Some(generate_gc_prologue(
            &mut builder,
            compiler.module,
            &gc_roots,
            nroots,
            compiler.gc_push_id,
            func,
            &entry_params,
        ))
    } else {
        None
    };

    // Generate code for each block
    let mut block_map = IndexMap::new();
    for (block_id, _) in &func.blocks {
        if *block_id != func.entry_block {
            let cl_block = builder.create_block();
            block_map.insert(*block_id, cl_block);
        } else {
            block_map.insert(*block_id, entry_block);
        }
    }

    // Codegen blocks
    {
        let mut codegen_ctx = CodegenContext {
            var_map: &var_map,
            locals: &func.locals,
            module: compiler.module,
            func_ids: compiler.func_ids,
            interner: compiler.interner,
            gc_frame_data: &gc_frame_data,
            block_map: &block_map,
            gc_pop_id: compiler.gc_pop_id,
            func_name_ids: compiler.func_name_ids,
            func_param_types: compiler.func_param_types,
            return_type: &func.return_type,
            line_map: compiler.line_map,
        };

        for (block_id, block) in &func.blocks {
            let cl_block = *codegen_ctx
                .block_map
                .get(block_id)
                .expect("internal error: block not in block_map - codegen bug");
            if cl_block != entry_block {
                builder.switch_to_block(cl_block);
            }

            // Instructions
            for inst in &block.instructions {
                // Set source location for debug info
                if let (Some(span), Some(lm)) = (inst.span, codegen_ctx.line_map) {
                    let line = lm.line_number(span.start);
                    builder.set_srcloc(cranelift_codegen::ir::SourceLoc::new(line));
                } else {
                    builder.set_srcloc(cranelift_codegen::ir::SourceLoc::default());
                }
                compile_instruction(&mut builder, inst, &mut codegen_ctx)?;
            }

            // Terminator
            compile_terminator(&mut builder, &block.terminator, &mut codegen_ctx)?;
        }
    }

    builder.seal_all_blocks();
    builder.finalize();

    // Define the function in the module
    compiler
        .module
        .define_function(cl_func_id, compiler.ctx)
        .expect("failed to declare runtime function");

    Ok(())
}

/// Generate the GC frame prologue
fn generate_gc_prologue(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    gc_roots: &[(LocalId, usize)],
    nroots: usize,
    gc_push_id: Option<ClFuncId>,
    func: &mir::Function,
    entry_params: &[cranelift_codegen::ir::Value],
) -> GcFrameData {
    // Create stack slot for ShadowFrame struct
    // ShadowFrame: prev (8 bytes) + nroots (8 bytes) + roots ptr (8 bytes) = 24 bytes
    let frame_slot = builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        24,
        3, // 8-byte alignment (2^3 = 8)
    ));

    // Create stack slot for roots array (8 bytes per root pointer)
    let roots_slot = builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        (8 * nroots) as u32,
        3, // 8-byte alignment (2^3 = 8)
    ));

    // Get addresses of frame and roots array
    let frame_addr = builder.ins().stack_addr(cltypes::I64, frame_slot, 0);
    let roots_addr = builder.ins().stack_addr(cltypes::I64, roots_slot, 0);

    // Initialize ShadowFrame fields:
    // frame.prev = null (will be set by gc_push)
    let zero = builder.ins().iconst(cltypes::I64, 0);
    builder.ins().store(MemFlags::new(), zero, frame_addr, 0);

    // frame.nroots = nroots
    let nroots_val = builder.ins().iconst(cltypes::I64, nroots as i64);
    builder
        .ins()
        .store(MemFlags::new(), nroots_val, frame_addr, 8);

    // frame.roots = roots_addr
    builder
        .ins()
        .store(MemFlags::new(), roots_addr, frame_addr, 16);

    // Initialize all roots to null
    for i in 0..nroots {
        let offset = (8 * i) as i32;
        builder
            .ins()
            .store(MemFlags::new(), zero, roots_addr, offset);
    }

    // Call gc_push(frame_addr)
    if let Some(gc_push_id) = gc_push_id {
        let gc_push_ref = module.declare_func_in_func(gc_push_id, builder.func);
        builder.ins().call(gc_push_ref, &[frame_addr]);
    }

    // Store parameters that are GC roots in the roots array
    for (idx, param) in func.params.iter().enumerate() {
        if let Some(&(_, root_idx)) = gc_roots.iter().find(|(id, _)| *id == param.id) {
            let roots_addr = builder.ins().stack_addr(cltypes::I64, roots_slot, 0);
            let offset = (8 * root_idx) as i32;
            builder
                .ins()
                .store(MemFlags::new(), entry_params[idx], roots_addr, offset);
        }
    }

    GcFrameData {
        roots_slot,
        gc_roots: gc_roots.to_vec(),
    }
}

/// Declare runtime functions rt_init and rt_shutdown
pub fn declare_runtime_functions(module: &mut ObjectModule) -> (ClFuncId, ClFuncId) {
    // rt_init(argc: i32, argv: *const *const i8)
    let mut init_sig = module.make_signature();
    init_sig.call_conv = CallConv::SystemV;
    init_sig.params.push(AbiParam::new(cltypes::I32)); // argc
    init_sig.params.push(AbiParam::new(cltypes::I64)); // argv pointer

    let rt_init_id = module
        .declare_function("rt_init", Linkage::Import, &init_sig)
        .expect("failed to declare runtime function");

    // rt_shutdown()
    let mut shutdown_sig = module.make_signature();
    shutdown_sig.call_conv = CallConv::SystemV;

    let rt_shutdown_id = module
        .declare_function("rt_shutdown", Linkage::Import, &shutdown_sig)
        .expect("failed to declare runtime function");

    (rt_init_id, rt_shutdown_id)
}

/// Generate C main entry point with support for multiple module initializations.
/// Calls module inits in dependency order before the main module init.
pub fn generate_main_entry_point_with_module_inits(
    module: &mut ObjectModule,
    ctx: &mut Context,
    func_builder_ctx: &mut FunctionBuilderContext,
    main_module_init_id: Option<ClFuncId>,
    rt_init_id: ClFuncId,
    rt_shutdown_id: ClFuncId,
    module_init_ids: &[ClFuncId],
) -> Result<()> {
    // Signature: int main(int argc, char** argv)
    let mut sig = module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I32)); // argc
    sig.params.push(AbiParam::new(cltypes::I64)); // argv pointer
    sig.returns.push(AbiParam::new(cltypes::I32));

    let main_id = module
        .declare_function("main", Linkage::Export, &sig)
        .expect("failed to declare runtime function");

    // Declare __pyaot_init_vtables__ (always exists, generated by codegen)
    let mut vtables_sig = module.make_signature();
    vtables_sig.call_conv = CallConv::SystemV;
    let init_vtables_id = module
        .declare_function("__pyaot_init_vtables__", Linkage::Import, &vtables_sig)
        .expect("failed to declare runtime function");

    ctx.clear();
    ctx.func.signature = sig.clone();

    let mut builder = FunctionBuilder::new(&mut ctx.func, func_builder_ctx);
    let entry_block = builder.create_block();
    builder.append_block_params_for_function_params(entry_block);
    builder.switch_to_block(entry_block);
    builder.seal_block(entry_block);

    // Get argc and argv from main parameters
    let params = builder.block_params(entry_block).to_vec();
    let argc = params[0];
    let argv = params[1];

    // Call rt_init(argc, argv)
    let rt_init_ref = module.declare_func_in_func(rt_init_id, builder.func);
    builder.ins().call(rt_init_ref, &[argc, argv]);

    // Call __pyaot_init_vtables__() - must be before module_init to set up vtables
    let init_vtables_ref = module.declare_func_in_func(init_vtables_id, builder.func);
    builder.ins().call(init_vtables_ref, &[]);

    // Call module inits in dependency order (imported modules first)
    for &init_id in module_init_ids {
        let init_ref = module.declare_func_in_func(init_id, builder.func);
        builder.ins().call(init_ref, &[]);
    }

    // Call main module's __pyaot_module_init__() if exists
    if let Some(init_id) = main_module_init_id {
        let init_ref = module.declare_func_in_func(init_id, builder.func);
        builder.ins().call(init_ref, &[]);
    }

    // Call rt_shutdown()
    let rt_shutdown_ref = module.declare_func_in_func(rt_shutdown_id, builder.func);
    builder.ins().call(rt_shutdown_ref, &[]);

    // Return 0
    let zero = builder.ins().iconst(cltypes::I32, 0);
    builder.ins().return_(&[zero]);

    builder.finalize();
    module
        .define_function(main_id, ctx)
        .expect("failed to define main function");

    Ok(())
}
