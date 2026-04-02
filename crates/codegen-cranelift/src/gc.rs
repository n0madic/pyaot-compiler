//! GC-related code generation: prologue, epilogue, and root management

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::{FuncId as ClFuncId, Linkage, Module};
use cranelift_object::ObjectModule;
use pyaot_utils::LocalId;

use pyaot_core_defs::layout;

use crate::context::GcFrameData;

/// Declare gc_push function: extern "C" fn(*mut ShadowFrame)
pub fn declare_gc_push(module: &mut ObjectModule) -> ClFuncId {
    let mut sig = module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // *mut ShadowFrame

    module
        .declare_function("gc_push", Linkage::Import, &sig)
        .expect("failed to declare gc runtime function")
}

/// Declare gc_pop function: extern "C" fn()
pub fn declare_gc_pop(module: &mut ObjectModule) -> ClFuncId {
    let mut sig = module.make_signature();
    sig.call_conv = CallConv::SystemV;

    module
        .declare_function("gc_pop", Linkage::Import, &sig)
        .expect("failed to declare gc runtime function")
}

/// Update the GC roots array if the destination local is a GC root.
/// This must be called after any instruction that writes to a local that could be a GC root.
pub fn update_gc_root_if_needed(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    value: cranelift_codegen::ir::Value,
    gc_frame_data: &Option<GcFrameData>,
) {
    if let Some(gc_data) = gc_frame_data {
        // Check if dest is a GC root
        if let Some(&(_, root_idx)) = gc_data.gc_roots.iter().find(|(id, _)| id == dest) {
            let roots_addr = builder
                .ins()
                .stack_addr(cltypes::I64, gc_data.roots_slot, 0);
            builder.ins().store(
                MemFlags::new(),
                value,
                roots_addr,
                layout::gc_root_offset(root_idx),
            );
        }
    }
}
