//! Cranelift code generation backend
//!
//! This crate generates native code from MIR using Cranelift.
//!
//! # Module Structure
//!
//! - `context` - Codegen context structures
//! - `utils` - Utility functions (type conversion, name mangling, operand loading)
//! - `gc` - GC prologue/epilogue and root management
//! - `runtime_calls` - Runtime function call generation
//! - `runtime_helpers` - Helper functions for runtime calls (reduces code duplication)
//! - `exceptions` - Exception handling instructions and terminators
//! - `instructions` - Core instruction compilation
//! - `terminators` - Terminator compilation
//! - `function` - Function declaration and definition

#![forbid(unsafe_code)]

mod context;
mod exceptions;
mod function;
mod gc;
mod instructions;
mod runtime_calls;
mod runtime_helpers;
mod terminators;
mod utils;

use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::FunctionBuilderContext;
use cranelift_module::FuncId as ClFuncId;
use cranelift_object::{ObjectBuilder, ObjectModule};
use indexmap::IndexMap;
use pyaot_diagnostics::Result;
use pyaot_mir as mir;
use pyaot_utils::StringInterner;
use target_lexicon::Triple;

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::{Linkage, Module};
use pyaot_utils::FuncId;

use function::{
    declare_function, declare_runtime_functions, define_function,
    generate_main_entry_point_with_module_inits, FunctionCompiler,
};
use gc::{declare_gc_pop, declare_gc_push};

/// Main code generator struct
pub struct Codegen {
    module: ObjectModule,
    ctx: Context,
    func_builder_ctx: FunctionBuilderContext,
    gc_push_id: Option<ClFuncId>,
    gc_pop_id: Option<ClFuncId>,
}

impl Codegen {
    /// Create a new code generator for the given target
    pub fn new(target: Triple, enable_debug: bool) -> Result<Self> {
        let mut flag_builder = settings::builder();

        // Set optimization level based on debug flag
        if enable_debug {
            // Debug mode: disable optimizations for easier debugging
            flag_builder
                .set("opt_level", "none")
                .expect("failed to set Cranelift opt_level flag");
            // Preserve frame pointers for better stack traces
            flag_builder
                .set("preserve_frame_pointers", "true")
                .expect("failed to set Cranelift preserve_frame_pointers flag");
            // Enable verifier for additional checks
            flag_builder
                .set("enable_verifier", "true")
                .expect("failed to set Cranelift enable_verifier flag");
        } else {
            // Release mode: optimize for speed
            flag_builder
                .set("opt_level", "speed")
                .expect("failed to set Cranelift opt_level flag");
        }

        flag_builder
            .set("is_pic", "true")
            .expect("failed to set Cranelift is_pic flag");

        let isa_builder =
            cranelift_codegen::isa::lookup(target).expect("failed to lookup target ISA");
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .expect("failed to finish ISA builder");

        let builder = ObjectBuilder::new(
            isa,
            "python_module",
            cranelift_module::default_libcall_names(),
        )
        .expect("failed to create ObjectBuilder");
        let module = ObjectModule::new(builder);

        Ok(Self {
            module,
            ctx: Context::new(),
            func_builder_ctx: FunctionBuilderContext::new(),
            gc_push_id: None,
            gc_pop_id: None,
        })
    }

    /// Compile a MIR module to object code
    pub fn compile_module(
        mut self,
        mir_module: &mir::Module,
        interner: &StringInterner,
    ) -> Result<Vec<u8>> {
        // Declare runtime functions
        let (rt_init_id, rt_shutdown_id) = declare_runtime_functions(&mut self.module);

        // Declare GC functions
        self.gc_push_id = Some(declare_gc_push(&mut self.module));
        self.gc_pop_id = Some(declare_gc_pop(&mut self.module));

        // Declare all functions first and collect parameter types
        let mut func_ids = IndexMap::new();
        let mut func_name_ids = IndexMap::new();
        let mut func_param_types = IndexMap::new();
        for (fid, func) in &mir_module.functions {
            let cl_func_id = declare_function(&mut self.module, func)?;
            func_ids.insert(*fid, cl_func_id);
            func_name_ids.insert(func.name.clone(), cl_func_id);
            // Collect parameter types for type coercion at call sites (e.g., Bool -> Int)
            let param_types: Vec<_> = func.params.iter().map(|p| p.ty.clone()).collect();
            func_param_types.insert(*fid, param_types);
        }

        // Create vtable data sections and collect their IDs
        let vtable_data_ids = self.create_vtable_data_sections(mir_module, &func_ids)?;

        // Define all functions
        for (fid, func) in &mir_module.functions {
            let cl_func_id = *func_ids
                .get(fid)
                .expect("internal error: function ID not in func_ids map - codegen bug");
            let mut compiler = FunctionCompiler {
                module: &mut self.module,
                ctx: &mut self.ctx,
                func_builder_ctx: &mut self.func_builder_ctx,
                func_ids: &func_ids,
                func_name_ids: &func_name_ids,
                func_param_types: &func_param_types,
                interner,
                gc_push_id: self.gc_push_id,
                gc_pop_id: self.gc_pop_id,
            };
            define_function(&mut compiler, func, cl_func_id)?;
        }

        // Collect generator resume functions for the dispatcher
        // Resume functions have names ending with "$resume" and their original func_id
        // is stored as func_id - 10000
        let resume_funcs: Vec<(FuncId, ClFuncId)> = mir_module
            .functions
            .iter()
            .filter(|(_, f)| f.name.ends_with("$resume"))
            .map(|(fid, _)| {
                // The original generator func_id is stored as resume_id - 10000
                (
                    *fid,
                    *func_ids.get(fid).expect(
                        "internal error: resume function ID not in func_ids map - codegen bug",
                    ),
                )
            })
            .collect();

        // Always generate the generator resume dispatcher (even if empty)
        // The runtime references __pyaot_generator_resume, so it must always exist
        self.generate_generator_dispatcher(&resume_funcs, mir_module, &func_ids)?;

        // Generate vtable registration function
        self.generate_vtable_registration(&vtable_data_ids)?;

        // Find main module init function (__pyaot_module_init__)
        let main_module_init_id = mir_module
            .functions
            .iter()
            .find(|(_, f)| f.name == "__pyaot_module_init__")
            .map(|(fid, _)| {
                *func_ids
                    .get(fid)
                    .expect("internal error: main module init function ID not in func_ids map - codegen bug")
            });

        // Collect imported module init functions in dependency order
        // These are the __module_<name>_init__ functions for imported modules
        let module_init_ids: Vec<ClFuncId> = mir_module
            .module_init_order
            .iter()
            .filter_map(|(_name, fid)| func_ids.get(fid).copied())
            .collect();

        // Generate C main entry point with multi-module support
        generate_main_entry_point_with_module_inits(
            &mut self.module,
            &mut self.ctx,
            &mut self.func_builder_ctx,
            main_module_init_id,
            rt_init_id,
            rt_shutdown_id,
            &module_init_ids,
        )?;

        // Finalize and get object file
        let product = self.module.finish();
        Ok(product
            .emit()
            .expect("failed to emit object file from Cranelift module"))
    }

    /// Create vtable data sections for each class
    /// Returns a map from class_id to the DataId of the vtable
    fn create_vtable_data_sections(
        &mut self,
        mir_module: &mir::Module,
        func_ids: &IndexMap<FuncId, ClFuncId>,
    ) -> Result<IndexMap<u32, cranelift_module::DataId>> {
        use cranelift_module::DataDescription;

        let mut vtable_data_ids = IndexMap::new();

        for vtable_info in &mir_module.vtables {
            let class_id = vtable_info.class_id.0;
            let num_slots = vtable_info.entries.len();

            // Vtable layout: [num_slots: u64, method_ptrs: [*const (); num_slots]]
            // Total size: 8 + 8 * num_slots bytes
            let vtable_size = 8 + 8 * num_slots;

            // Declare the data object
            let data_name = format!("__vtable_{}", class_id);
            let data_id = self
                .module
                .declare_data(&data_name, Linkage::Local, false, false)
                .expect("failed to declare vtable data section");

            // Create data description with initial data (num_slots at offset 0)
            let mut data_desc = DataDescription::new();

            // Create a buffer with num_slots followed by zeroes for function pointers
            let mut init_data = vec![0u8; vtable_size];
            // Write num_slots at offset 0 (little-endian)
            let num_slots_bytes = (num_slots as u64).to_le_bytes();
            init_data[..8].copy_from_slice(&num_slots_bytes);

            data_desc.define(init_data.into_boxed_slice());

            // Write function pointers starting at offset 8
            for entry in &vtable_info.entries {
                let offset = 8 + entry.slot * 8;
                if let Some(&cl_func_id) = func_ids.get(&entry.method_func_id) {
                    // Get the function reference for this data section
                    let func_ref = self.module.declare_func_in_data(cl_func_id, &mut data_desc);
                    data_desc.write_function_addr(offset as u32, func_ref);
                }
            }

            // Define the data object
            self.module
                .define_data(data_id, &data_desc)
                .expect("failed to define vtable data section");

            vtable_data_ids.insert(class_id, data_id);
        }

        Ok(vtable_data_ids)
    }

    /// Generate the __pyaot_init_vtables__ function that registers all vtables
    fn generate_vtable_registration(
        &mut self,
        vtable_data_ids: &IndexMap<u32, cranelift_module::DataId>,
    ) -> Result<()> {
        // Create signature: fn()
        let mut sig = self.module.make_signature();
        sig.call_conv = CallConv::SystemV;

        // Declare the function
        let func_id = self
            .module
            .declare_function("__pyaot_init_vtables__", Linkage::Export, &sig)
            .expect("failed to declare __pyaot_init_vtables__ function");

        // Declare rt_register_vtable
        let mut reg_sig = self.module.make_signature();
        reg_sig.call_conv = CallConv::SystemV;
        reg_sig.params.push(AbiParam::new(cltypes::I8)); // class_id
        reg_sig.params.push(AbiParam::new(cltypes::I64)); // vtable_ptr
        let rt_register_vtable_id = self
            .module
            .declare_function("rt_register_vtable", Linkage::Import, &reg_sig)
            .expect("failed to declare rt_register_vtable function");

        // Define the function
        self.ctx.clear();
        self.ctx.func.signature = sig;

        let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.func_builder_ctx);
        let entry_block = builder.create_block();
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        // Get function reference for rt_register_vtable
        let rt_register_vtable_ref = self
            .module
            .declare_func_in_func(rt_register_vtable_id, builder.func);

        // Register each vtable
        for (class_id, data_id) in vtable_data_ids {
            // Get the global value for the vtable data
            let gv = self.module.declare_data_in_func(*data_id, builder.func);
            let vtable_addr = builder.ins().global_value(cltypes::I64, gv);

            // Call rt_register_vtable(class_id, vtable_addr)
            let class_id_val = builder.ins().iconst(cltypes::I8, *class_id as i64);
            builder
                .ins()
                .call(rt_register_vtable_ref, &[class_id_val, vtable_addr]);
        }

        // Return
        builder.ins().return_(&[]);
        builder.finalize();

        // Define the function in the module
        self.module
            .define_function(func_id, &mut self.ctx)
            .expect("failed to define __pyaot_init_vtables__ function");
        self.ctx.clear();

        Ok(())
    }

    /// Generate the `__pyaot_generator_resume` dispatcher function
    ///
    /// This function dispatches to the appropriate resume function based on func_id
    /// stored in the generator object.
    fn generate_generator_dispatcher(
        &mut self,
        resume_funcs: &[(FuncId, ClFuncId)],
        _mir_module: &mir::Module,
        _func_ids: &IndexMap<FuncId, ClFuncId>,
    ) -> Result<()> {
        // Create signature: fn(gen: *mut Obj) -> *mut Obj
        let mut sig = self.module.make_signature();
        sig.call_conv = CallConv::SystemV;
        sig.params.push(AbiParam::new(cltypes::I64)); // generator pointer
        sig.returns.push(AbiParam::new(cltypes::I64)); // return value pointer

        // Declare the function
        let func_id = self
            .module
            .declare_function("__pyaot_generator_resume", Linkage::Export, &sig)
            .expect("failed to declare __pyaot_generator_resume function");

        // Define the function
        self.ctx.clear();
        self.ctx.func.signature = sig;

        let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.func_builder_ctx);
        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);

        // Get the generator pointer parameter
        let gen_ptr = builder.block_params(entry_block)[0];

        // Load func_id from generator object
        // GeneratorObj layout: header (ObjHeader: type_tag(1) + marked(1) + size(8) = 10 bytes, aligned to 16)
        // func_id is at offset 16 (after header)
        let func_id_offset = 16i32; // offset of func_id in GeneratorObj
        let func_id_val = builder.ins().load(
            cltypes::I32,
            cranelift_codegen::ir::MemFlags::new(),
            gen_ptr,
            func_id_offset,
        );

        // For simplicity, use if-else chain instead of switch
        // This is less efficient but avoids CFG complexity issues

        // Create blocks for each resume function
        let dispatch_info: Vec<(u32, ClFuncId)> = resume_funcs
            .iter()
            .map(|(mir_fid, cl_fid)| (mir_fid.0 - 10000, *cl_fid))
            .collect();

        if dispatch_info.is_empty() {
            // No generators - just return null from entry block
            builder.seal_block(entry_block);
            let null_val = builder.ins().iconst(cltypes::I64, 0);
            builder.ins().return_(&[null_val]);
        } else {
            // Default: return null (StopIteration will be raised by runtime)
            let default_block = builder.create_block();

            // Build if-else chain
            let mut current_block = entry_block;

            for (i, (original_func_id, cl_func_id)) in dispatch_info.iter().enumerate() {
                let is_last = i == dispatch_info.len() - 1;

                // Compare func_id
                let expected = builder.ins().iconst(cltypes::I32, *original_func_id as i64);
                let cmp = builder.ins().icmp(
                    cranelift_codegen::ir::condcodes::IntCC::Equal,
                    func_id_val,
                    expected,
                );

                // Create call block for this generator
                let call_block = builder.create_block();

                // Next comparison block or default
                let else_block = if is_last {
                    default_block
                } else {
                    builder.create_block()
                };

                // Branch based on comparison
                builder.ins().brif(cmp, call_block, &[], else_block, &[]);
                builder.seal_block(current_block);

                // Call block: call resume function and return result
                builder.switch_to_block(call_block);
                builder.seal_block(call_block);

                let func_ref = self.module.declare_func_in_func(*cl_func_id, builder.func);
                let call_inst = builder.ins().call(func_ref, &[gen_ptr]);
                let result = builder.inst_results(call_inst)[0];
                builder.ins().return_(&[result]);

                // Move to next comparison block
                if !is_last {
                    builder.switch_to_block(else_block);
                    current_block = else_block;
                }
            }

            // Default block: return null (only reached when generators exist but func_id doesn't match)
            builder.switch_to_block(default_block);
            builder.seal_block(default_block);
            let null_val = builder.ins().iconst(cltypes::I64, 0);
            builder.ins().return_(&[null_val]);
        }

        builder.finalize();

        // Define the function in the module
        self.module
            .define_function(func_id, &mut self.ctx)
            .expect("failed to define __pyaot_generator_resume function");
        self.ctx.clear();

        Ok(())
    }
}
