//! Traceback-related code generation: stack push/pop declarations

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::AbiParam;
use cranelift_codegen::isa::CallConv;
use cranelift_module::{FuncId as ClFuncId, Linkage, Module};
use cranelift_object::ObjectModule;

/// Declare rt_stack_push:
///   extern "C" fn(func_name: *const u8, func_name_len: usize,
///                 file_name: *const u8, file_name_len: usize,
///                 line_number: u32)
pub fn declare_stack_push(module: &mut ObjectModule) -> ClFuncId {
    let mut sig = module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // func_name ptr
    sig.params.push(AbiParam::new(cltypes::I64)); // func_name_len
    sig.params.push(AbiParam::new(cltypes::I64)); // file_name ptr
    sig.params.push(AbiParam::new(cltypes::I64)); // file_name_len
    sig.params.push(AbiParam::new(cltypes::I32)); // line_number

    module
        .declare_function("rt_stack_push", Linkage::Import, &sig)
        .expect("failed to declare rt_stack_push")
}

/// Declare rt_stack_pop: extern "C" fn()
pub fn declare_stack_pop(module: &mut ObjectModule) -> ClFuncId {
    let mut sig = module.make_signature();
    sig.call_conv = CallConv::SystemV;

    module
        .declare_function("rt_stack_pop", Linkage::Import, &sig)
        .expect("failed to declare rt_stack_pop")
}
