//! # codegen-cranelift — typed MIR → native code
//!
//! Lowers typed MIR to Cranelift IR and emits an object file. The runtime
//! provides **no** C `main`, so this backend emits `main(argc, argv)` that calls
//! `rt_init` → the module body → `rt_shutdown` → `return 0`.
//!
//! ## The ABI is one function
//!
//! [`clif_ty`] maps [`pyaot_types::Repr`] → a Cranelift `Type`. It *is* the ABI:
//! there is no second logical-type mapper and no per-function ABI flags. Every
//! place that needs a value's machine type asks `clif_ty`.
//!
//! ## Phase 1 scope
//!
//! `print(<str>)`: declare the `rt_*` imports, materialize each string literal
//! as a local data object, and inline `__main__`'s single block into C `main`.
//!
//! **GC shadow-stack:** Phase 1 emits the `nroots == 0` leaf path — no shadow
//! frame. *Safety:* a collection only fires inside `gc_alloc` past a 1 MiB
//! threshold and never re-entrantly; the single small `rt_make_str` cannot
//! collect itself, and nothing allocates between create and use
//! (`rt_print_str_obj` / `rt_print_newline` allocate nothing, `rt_shutdown` runs
//! after). So the StrObj is alive for its whole window with no root registration.
//!
//! `// PHASE-2-TODO:` emit a `ShadowFrame` + `gc_push`/`gc_pop` and store
//! `is_gc_root()` locals into `frame.roots` the moment a GC-root local is live
//! across an allocating call. The root set derives from
//! `locals[i].repr.is_gc_root()`, never a stored flag.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::Path;

use cranelift_codegen::ir::{types, AbiParam, InstBuilder, Signature, Type, Value};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{default_libcall_names, DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_mir::{
    classify_coercion, Coercion, Const, MirFunction, MirInst, MirProgram, MirTerminator, Operand,
    PrintKind,
};
use pyaot_types::{RawKind, Repr};
use pyaot_utils::InternedString;

/// **The single `Repr` → Cranelift `Type` mapping — this is the ABI.**
///
/// Unboxed `Raw` maps to its matching machine type; everything that flows as a
/// 64-bit word (`Tagged`, any typed `Heap` pointer, function pointers, closures)
/// maps to `I64`. `Never` has no machine representation.
fn clif_ty(repr: &Repr) -> Type {
    match repr {
        Repr::Raw(RawKind::I64) => types::I64,
        Repr::Raw(RawKind::F64) => types::F64,
        Repr::Raw(RawKind::I8) => types::I8,
        Repr::Raw(RawKind::I32) => types::I32,
        Repr::Tagged | Repr::Heap(_) | Repr::FuncPtr(_) | Repr::Closure(_) => types::I64,
        Repr::Never => unreachable!("Never has no machine representation"),
    }
}

/// The imported runtime functions Phase 1 needs.
struct RuntimeFns {
    init: FuncId,
    shutdown: FuncId,
    make_str: FuncId,
    print_str_obj: FuncId,
    print_newline: FuncId,
    #[allow(dead_code)] // declared per the seam; Phase 1 single-arg print never emits Sep.
    print_sep: FuncId,
}

impl RuntimeFns {
    fn declare(module: &mut ObjectModule, cc: CallConv, ptr_ty: Type) -> Result<Self> {
        Ok(Self {
            // rt_init(argc: i32, argv: *const *const i8)
            init: declare_import(module, cc, "rt_init", &[types::I32, ptr_ty], &[])?,
            // rt_shutdown()
            shutdown: declare_import(module, cc, "rt_shutdown", &[], &[])?,
            // rt_make_str(data: *const u8, len: usize) -> Value
            make_str: declare_import(module, cc, "rt_make_str", &[ptr_ty, types::I64], &[types::I64])?,
            // rt_print_str_obj(Value)
            print_str_obj: declare_import(module, cc, "rt_print_str_obj", &[types::I64], &[])?,
            // rt_print_newline()
            print_newline: declare_import(module, cc, "rt_print_newline", &[], &[])?,
            // rt_print_sep()
            print_sep: declare_import(module, cc, "rt_print_sep", &[], &[])?,
        })
    }
}

fn declare_import(
    module: &mut ObjectModule,
    cc: CallConv,
    name: &str,
    params: &[Type],
    returns: &[Type],
) -> Result<FuncId> {
    let mut sig = Signature::new(cc);
    sig.params.extend(params.iter().copied().map(AbiParam::new));
    sig.returns.extend(returns.iter().copied().map(AbiParam::new));
    module
        .declare_function(name, Linkage::Import, &sig)
        .map_err(|e| cg_error(format!("declare import `{name}`: {e}")))
}

/// Compile a [`MirProgram`] to a native object file at `out_obj`.
pub fn compile(program: &MirProgram, out_obj: &Path) -> Result<()> {
    // ── Host ISA. PIC + non-colocated libcalls so the string data relocation
    // and the imported `rt_*` symbols link in a macOS arm64 PIE. These flags are
    // validated on macOS arm64 only; Linux/x86-64 linking is unverified and may
    // need different settings (revisit with the linker work / CI). ──
    let mut flag_builder = settings::builder();
    flag_builder
        .set("is_pic", "true")
        .map_err(|e| cg_error(format!("set is_pic: {e}")))?;
    flag_builder
        .set("use_colocated_libcalls", "false")
        .map_err(|e| cg_error(format!("set use_colocated_libcalls: {e}")))?;
    let flags = settings::Flags::new(flag_builder);

    let isa_builder =
        cranelift_native::builder().map_err(|e| cg_error(format!("host ISA detection: {e}")))?;
    let isa = isa_builder
        .finish(flags)
        .map_err(|e| cg_error(format!("ISA finish: {e}")))?;

    let builder = ObjectBuilder::new(isa, "pyaot_module", default_libcall_names())
        .map_err(|e| cg_error(format!("object builder: {e}")))?;
    let mut module = ObjectModule::new(builder);

    let ptr_ty = module.target_config().pointer_type();
    let call_conv = CallConv::triple_default(module.isa().triple());

    let rt = RuntimeFns::declare(&mut module, call_conv, ptr_ty)?;

    // ── Declare + define one local data object per distinct string literal. ──
    let mut data_ids: HashMap<InternedString, DataId> = HashMap::new();
    for (interned, bytes) in program.str_pool.iter() {
        let name = format!("pyaot_str_{}", interned.index());
        let data_id = module
            .declare_data(&name, Linkage::Local, false, false)
            .map_err(|e| cg_error(format!("declare data `{name}`: {e}")))?;
        let mut desc = DataDescription::new();
        desc.define(bytes.to_vec().into_boxed_slice());
        module
            .define_data(data_id, &desc)
            .map_err(|e| cg_error(format!("define data `{name}`: {e}")))?;
        data_ids.insert(interned, data_id);
    }

    emit_main(&mut module, program, &rt, &data_ids, ptr_ty, call_conv)?;

    let product = module.finish();
    let bytes = product
        .emit()
        .map_err(|e| cg_error(format!("object emit: {e}")))?;
    std::fs::write(out_obj, bytes)
        .map_err(|e| cg_error(format!("write {}: {e}", out_obj.display())))?;
    Ok(())
}

fn emit_main(
    module: &mut ObjectModule,
    program: &MirProgram,
    rt: &RuntimeFns,
    data_ids: &HashMap<InternedString, DataId>,
    ptr_ty: Type,
    cc: CallConv,
) -> Result<()> {
    // main(argc: i32, argv: i64) -> i32 — matches `int main(int, char**)`.
    let mut sig = Signature::new(cc);
    sig.params.push(AbiParam::new(types::I32)); // argc
    sig.params.push(AbiParam::new(ptr_ty)); // argv
    sig.returns.push(AbiParam::new(types::I32)); // exit code
    let main_id = module
        .declare_function("main", Linkage::Export, &sig)
        .map_err(|e| cg_error(format!("declare main: {e}")))?;

    let mut ctx = module.make_context();
    ctx.func.signature = sig;
    let mut fctx = FunctionBuilderContext::new();

    {
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);

        let argc = builder.block_params(entry)[0];
        let argv = builder.block_params(entry)[1];

        // rt_init(argc, argv)
        let init_ref = module.declare_func_in_func(rt.init, builder.func);
        builder.ins().call(init_ref, &[argc, argv]);

        // Inline the synthetic __main__ body.
        let entry_func = &program.funcs[program.entry.index()];
        emit_function_body(module, &mut builder, entry_func, program, rt, data_ids, ptr_ty)?;

        // rt_shutdown()
        let shutdown_ref = module.declare_func_in_func(rt.shutdown, builder.func);
        builder.ins().call(shutdown_ref, &[]);

        // return 0
        let zero = builder.ins().iconst(types::I32, 0);
        builder.ins().return_(&[zero]);

        builder.finalize();
    }

    module
        .define_function(main_id, &mut ctx)
        .map_err(|e| cg_error(format!("define main: {e}")))?;
    module.clear_context(&mut ctx);
    Ok(())
}

fn emit_function_body(
    module: &mut ObjectModule,
    builder: &mut FunctionBuilder,
    func: &MirFunction,
    program: &MirProgram,
    rt: &RuntimeFns,
    data_ids: &HashMap<InternedString, DataId>,
    ptr_ty: Type,
) -> Result<()> {
    // Locals map 1:1 to SSA values for Phase 1 (single block, single assignment).
    let mut locals: Vec<Option<Value>> = vec![None; func.locals.len()];

    let block = &func.blocks[func.entry.index()];
    for inst in &block.insts {
        match inst {
            MirInst::Const {
                dst,
                val: Const::Str(interned),
            } => {
                let data_id = *data_ids
                    .get(interned)
                    .ok_or_else(|| cg_error("missing data object for string literal"))?;
                let gv = module.declare_data_in_func(data_id, builder.func);
                let ptr = builder.ins().global_value(ptr_ty, gv);
                let len = program.str_pool.bytes(*interned).map_or(0, <[u8]>::len) as i64;
                let len_val = builder.ins().iconst(types::I64, len);
                let make_str_ref = module.declare_func_in_func(rt.make_str, builder.func);
                let call = builder.ins().call(make_str_ref, &[ptr, len_val]);
                let result = builder.inst_results(call)[0];

                // The ABI contract, enforced: the materialized value's machine
                // type must equal `clif_ty` of the destination local's `Repr`.
                #[cfg(debug_assertions)]
                {
                    let actual = builder.func.dfg.value_type(result);
                    let expected = clif_ty(&func.locals[dst.index()].repr);
                    debug_assert_eq!(
                        actual, expected,
                        "Const::Str local {} machine type {actual:?} != clif_ty {expected:?}",
                        dst.index()
                    );
                }
                locals[dst.index()] = Some(result);
            }
            MirInst::Coerce { dst, src, from, to } => match classify_coercion(from, to) {
                Some(Coercion::Noop) => {
                    // Zero machine instructions: alias the source SSA value.
                    locals[dst.index()] = Some(operand_value(&locals, src)?);
                }
                other => {
                    return Err(cg_error(format!(
                        "coercion {from:?} -> {to:?} ({other:?}) not implemented in Phase 1"
                    )))
                }
            },
            MirInst::Print { kind, arg } => match kind {
                PrintKind::StrObj => {
                    let op = arg
                        .as_ref()
                        .ok_or_else(|| cg_error("Print(StrObj) missing argument"))?;
                    let v = operand_value(&locals, op)?;
                    let r = module.declare_func_in_func(rt.print_str_obj, builder.func);
                    builder.ins().call(r, &[v]);
                }
                PrintKind::Newline => {
                    let r = module.declare_func_in_func(rt.print_newline, builder.func);
                    builder.ins().call(r, &[]);
                }
                PrintKind::Sep => {
                    let r = module.declare_func_in_func(rt.print_sep, builder.func);
                    builder.ins().call(r, &[]);
                }
                other => {
                    return Err(cg_error(format!(
                        "Print kind {other:?} not implemented in Phase 1"
                    )))
                }
            },
        }
    }

    match &block.term {
        // __main__'s implicit `return None` is realized by `emit_main` as
        // rt_shutdown + `return 0`; nothing to emit for the operand itself.
        MirTerminator::Return(None) => Ok(()),
        MirTerminator::Return(Some(_)) => {
            Err(cg_error("non-None return is not supported in Phase 1"))
        }
    }
}

fn operand_value(locals: &[Option<Value>], op: &Operand) -> Result<Value> {
    match op {
        Operand::Local(id) => locals[id.index()].ok_or_else(|| cg_error("use of undefined local")),
    }
}

fn cg_error(msg: impl Into<String>) -> CompilerError {
    CompilerError::codegen_error(msg.into(), None)
}
