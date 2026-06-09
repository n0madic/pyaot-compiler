//! # codegen-cranelift — typed MIR → native code
//!
//! Lowers typed MIR to Cranelift IR and emits an object file. Each
//! [`MirFunction`] becomes a Cranelift function; the runtime provides **no** C
//! `main`, so this backend emits `main(argc, argv)` that calls `rt_init` → the
//! module-body function (`__main__`) → `rt_shutdown` → `return 0`.
//!
//! ## The ABI is one function
//!
//! [`clif_ty`] maps [`pyaot_types::Repr`] → a Cranelift `Type`. It *is* the ABI:
//! there is no second logical-type mapper and no per-function ABI flags.
//!
//! ## Locals are Cranelift `Variable`s
//!
//! Each MIR local is a Cranelift `Variable` (typed by `clif_ty`), so values flow
//! naturally across blocks (loop counters, branch joins) via Cranelift's SSA
//! construction. GC shadow frames (milestone 2c) store rooted locals into a
//! frame roots array on definition; the root set derives from
//! `Repr::is_gc_root()`, never a stored flag.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::Path;

use cranelift_codegen::ir::{
    condcodes::IntCC, types, AbiParam, InstBuilder, MemFlags, Signature, StackSlot, StackSlotData,
    StackSlotKind, TrapCode, Type, Value,
};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{default_libcall_names, DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use pyaot_core_defs::tag;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_mir::{
    classify_coercion, BinOp, CmpOp, Coercion, Const, LocalDecl, MirFunction, MirInst, MirProgram,
    MirTerminator, Operand, PrintKind, UnaryOp,
};
use pyaot_types::{RawKind, Repr};
use pyaot_utils::{InternedString, LocalId};

const FLOAT_VALUE_OFFSET: i32 = pyaot_core_defs::layout::FLOAT_OBJ_VALUE_OFFSET;

/// **The single `Repr` → Cranelift `Type` mapping — this is the ABI.**
fn clif_ty(repr: &Repr) -> Type {
    match repr {
        Repr::Raw(RawKind::I64) => types::I64,
        Repr::Raw(RawKind::F64) => types::F64,
        Repr::Raw(RawKind::I8) => types::I8,
        Repr::Raw(RawKind::I32) => types::I32,
        Repr::Tagged | Repr::Heap(_) | Repr::FuncPtr(_) | Repr::Closure(_) => types::I64,
        Repr::Never => types::I64,
    }
}

/// The imported runtime functions. Declaring an import that is never *used*
/// emits no relocation, so this can cover the whole Phase-2 surface up front.
struct RuntimeFns {
    init: FuncId,
    shutdown: FuncId,
    make_str: FuncId,
    bigint_from_str: FuncId,
    box_float: FuncId,
    add_int: FuncId,
    sub_int: FuncId,
    mul_int: FuncId,
    obj_add: FuncId,
    obj_sub: FuncId,
    obj_mul: FuncId,
    obj_div: FuncId,
    obj_floordiv: FuncId,
    obj_mod: FuncId,
    obj_pow: FuncId,
    obj_neg: FuncId,
    obj_pos: FuncId,
    obj_invert: FuncId,
    obj_eq: FuncId,
    obj_cmp: FuncId,
    is_truthy: FuncId,
    obj_bitand: FuncId,
    obj_bitor: FuncId,
    obj_bitxor: FuncId,
    obj_lshift: FuncId,
    obj_rshift: FuncId,
    builtin_abs: FuncId,
    builtin_int: FuncId,
    builtin_float: FuncId,
    builtin_str: FuncId,
    builtin_bool: FuncId,
    builtin_len: FuncId,
    assert_fail: FuncId,
    print_int: FuncId,
    print_float: FuncId,
    print_bool: FuncId,
    print_none: FuncId,
    print_str_obj: FuncId,
    print_obj: FuncId,
    print_sep: FuncId,
    print_newline: FuncId,
    gc_push: FuncId,
    gc_pop: FuncId,
}

impl RuntimeFns {
    fn declare(m: &mut ObjectModule, cc: CallConv, ptr: Type) -> Result<Self> {
        let ti = types::I64;
        let t8 = types::I8;
        let t32 = types::I32;
        let tf = types::F64;
        let mut d = |name: &str, p: &[Type], r: &[Type]| declare_import(m, cc, name, p, r);
        Ok(Self {
            init: d("rt_init", &[t32, ptr], &[])?,
            shutdown: d("rt_shutdown", &[], &[])?,
            make_str: d("rt_make_str", &[ptr, ti], &[ti])?,
            bigint_from_str: d("rt_bigint_from_str", &[ptr, ti], &[ti])?,
            box_float: d("rt_box_float", &[tf], &[ti])?,
            // Raw i64 arithmetic (Phase 3c): used only on range-proven cursors.
            // These RAISE OverflowError on i64 overflow (unlike CPython's bignum
            // promotion), so they are correct only where overflow provably cannot
            // occur — lowering emits them solely for literal-bounded cursors.
            add_int: d("rt_add_int", &[ti, ti], &[ti])?,
            sub_int: d("rt_sub_int", &[ti, ti], &[ti])?,
            mul_int: d("rt_mul_int", &[ti, ti], &[ti])?,
            obj_add: d("rt_obj_add", &[ti, ti], &[ti])?,
            obj_sub: d("rt_obj_sub", &[ti, ti], &[ti])?,
            obj_mul: d("rt_obj_mul", &[ti, ti], &[ti])?,
            obj_div: d("rt_obj_div", &[ti, ti], &[ti])?,
            obj_floordiv: d("rt_obj_floordiv", &[ti, ti], &[ti])?,
            obj_mod: d("rt_obj_mod", &[ti, ti], &[ti])?,
            obj_pow: d("rt_obj_pow", &[ti, ti], &[ti])?,
            obj_neg: d("rt_obj_neg", &[ti], &[ti])?,
            obj_pos: d("rt_obj_pos", &[ti], &[ti])?,
            obj_invert: d("rt_obj_invert", &[ti], &[ti])?,
            obj_eq: d("rt_obj_eq", &[ti, ti], &[t8])?,
            obj_cmp: d("rt_obj_cmp", &[ti, ti, t8], &[t8])?,
            is_truthy: d("rt_is_truthy", &[ti], &[t8])?,
            obj_bitand: d("rt_obj_bitand", &[ti, ti], &[ti])?,
            obj_bitor: d("rt_obj_bitor", &[ti, ti], &[ti])?,
            obj_bitxor: d("rt_obj_bitxor", &[ti, ti], &[ti])?,
            obj_lshift: d("rt_obj_lshift", &[ti, ti], &[ti])?,
            obj_rshift: d("rt_obj_rshift", &[ti, ti], &[ti])?,
            builtin_abs: d("rt_builtin_abs", &[ti], &[ti])?,
            builtin_int: d("rt_builtin_int", &[ti], &[ti])?,
            builtin_float: d("rt_builtin_float", &[ti], &[ti])?,
            builtin_str: d("rt_builtin_str", &[ti], &[ti])?,
            builtin_bool: d("rt_builtin_bool", &[ti], &[ti])?,
            builtin_len: d("rt_builtin_len", &[ti], &[ti])?,
            assert_fail: d("rt_assert_fail", &[ptr], &[])?,
            print_int: d("rt_print_int_value", &[ti], &[])?,
            print_float: d("rt_print_float_value", &[tf], &[])?,
            print_bool: d("rt_print_bool_value", &[t8], &[])?,
            print_none: d("rt_print_none_value", &[], &[])?,
            print_str_obj: d("rt_print_str_obj", &[ti], &[])?,
            print_obj: d("rt_print_obj", &[ti], &[])?,
            print_sep: d("rt_print_sep", &[], &[])?,
            print_newline: d("rt_print_newline", &[], &[])?,
            gc_push: d("gc_push", &[ptr], &[])?,
            gc_pop: d("gc_pop", &[], &[])?,
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

    // One data object per interned string (literal bytes or big-int decimals).
    // Store the byte length alongside the id (Cranelift does not expose it back).
    let mut data_ids: HashMap<InternedString, (DataId, u32)> = HashMap::new();
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
        data_ids.insert(interned, (data_id, bytes.len() as u32));
    }

    // Declare every MIR function (so calls can reference forward / recursively).
    let mut func_ids: Vec<FuncId> = Vec::with_capacity(program.funcs.len());
    for (i, mf) in program.funcs.iter().enumerate() {
        let mut sig = Signature::new(call_conv);
        for p in &mf.params {
            sig.params.push(AbiParam::new(clif_ty(p)));
        }
        sig.returns.push(AbiParam::new(clif_ty(&mf.ret)));
        let name = format!("pyaot_fn_{i}");
        let id = module
            .declare_function(&name, Linkage::Local, &sig)
            .map_err(|e| cg_error(format!("declare `{name}`: {e}")))?;
        func_ids.push(id);
    }

    // Define each function body.
    for (i, mf) in program.funcs.iter().enumerate() {
        define_function(&mut module, mf, func_ids[i], &func_ids, &rt, &data_ids, ptr_ty, call_conv)?;
    }

    emit_main(&mut module, func_ids[program.entry.index()], &rt, ptr_ty, call_conv)?;

    let product = module.finish();
    let bytes = product
        .emit()
        .map_err(|e| cg_error(format!("object emit: {e}")))?;
    std::fs::write(out_obj, bytes)
        .map_err(|e| cg_error(format!("write {}: {e}", out_obj.display())))?;
    Ok(())
}

/// `main(argc, argv)` → rt_init → call `__main__` → rt_shutdown → return 0.
fn emit_main(
    module: &mut ObjectModule,
    entry_fn: FuncId,
    rt: &RuntimeFns,
    ptr_ty: Type,
    cc: CallConv,
) -> Result<()> {
    let mut sig = Signature::new(cc);
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(ptr_ty));
    sig.returns.push(AbiParam::new(types::I32));
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

        let init = module.declare_func_in_func(rt.init, builder.func);
        builder.ins().call(init, &[argc, argv]);

        let entry_ref = module.declare_func_in_func(entry_fn, builder.func);
        builder.ins().call(entry_ref, &[]);

        let shutdown = module.declare_func_in_func(rt.shutdown, builder.func);
        builder.ins().call(shutdown, &[]);

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

#[allow(clippy::too_many_arguments)]
fn define_function(
    module: &mut ObjectModule,
    mf: &MirFunction,
    cl_func_id: FuncId,
    func_ids: &[FuncId],
    rt: &RuntimeFns,
    data_ids: &HashMap<InternedString, (DataId, u32)>,
    ptr_ty: Type,
    cc: CallConv,
) -> Result<()> {
    let mut sig = Signature::new(cc);
    for p in &mf.params {
        sig.params.push(AbiParam::new(clif_ty(p)));
    }
    sig.returns.push(AbiParam::new(clif_ty(&mf.ret)));

    let mut ctx = module.make_context();
    ctx.func.signature = sig;
    let mut fctx = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fctx);

        // One Cranelift block per MIR block.
        let cl_blocks: Vec<_> = mf.blocks.iter().map(|_| builder.create_block()).collect();

        // Declare a Variable per MIR local. Cranelift assigns indices 0..n in
        // declaration order, so Variable index == LocalId.
        for local in &mf.locals {
            builder.declare_var(clif_ty(&local.repr));
        }

        // GC root set (PITFALLS B15): every local whose `Repr::is_gc_root()` gets
        // a slot in a frame roots array. Over-approximate — root each such local
        // for the whole function (store-on-def). The GC is non-moving, so the
        // Variable copy stays valid; the roots array only keeps the value marked.
        let mut root_slot_of = vec![None; mf.locals.len()];
        let mut nroots: u32 = 0;
        for (i, local) in mf.locals.iter().enumerate() {
            if local.repr.is_gc_root() {
                root_slot_of[i] = Some(nroots);
                nroots += 1;
            }
        }
        let (roots_slot, frame_slot) = if nroots > 0 {
            let roots = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                nroots * 8,
                3,
            ));
            let frame = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                pyaot_core_defs::layout::SHADOW_FRAME_SIZE,
                3,
            ));
            (Some(roots), Some(frame))
        } else {
            (None, None)
        };

        let entry_idx = mf.entry.index();
        builder.append_block_params_for_function_params(cl_blocks[entry_idx]);

        let mut fb = FnGen {
            module,
            builder: &mut builder,
            cl_blocks: &cl_blocks,
            func_ids,
            rt,
            data_ids,
            locals: &mf.locals,
            program_ret: clif_ty(&mf.ret),
            ptr_ty,
            root_slot_of,
            nroots,
            roots_slot,
            frame_slot,
        };

        for (bi, mblock) in mf.blocks.iter().enumerate() {
            fb.builder.switch_to_block(cl_blocks[bi]);
            if bi == entry_idx {
                // GC frame setup must precede any rooted store (incl. params).
                fb.emit_gc_prologue();
                // Prologue: define parameter variables from block params.
                let params: Vec<Value> = fb.builder.block_params(cl_blocks[bi]).to_vec();
                for (i, pv) in params.iter().enumerate() {
                    fb.def_local(LocalId::from(i), *pv);
                }
            }
            for inst in &mblock.insts {
                fb.lower_inst(inst)?;
            }
            fb.lower_terminator(&mblock.term)?;
        }

        builder.seal_all_blocks();
        builder.finalize();
    }
    module
        .define_function(cl_func_id, &mut ctx)
        .map_err(|e| cg_error(format!("define function: {e}")))?;
    module.clear_context(&mut ctx);
    Ok(())
}

/// Per-function codegen context.
struct FnGen<'a, 'b> {
    module: &'a mut ObjectModule,
    builder: &'a mut FunctionBuilder<'b>,
    cl_blocks: &'a [cranelift_codegen::ir::Block],
    func_ids: &'a [FuncId],
    rt: &'a RuntimeFns,
    data_ids: &'a HashMap<InternedString, (DataId, u32)>,
    /// The MIR function's local Repr table — drives per-operand dispatch (a
    /// `Raw(F64)`/`Raw(I64)` arithmetic operand inlines, a `Tagged` one calls
    /// `rt_obj_*`). This is the same `Repr` the verifier checked; codegen never
    /// re-derives it (Principle 6).
    locals: &'a [LocalDecl],
    program_ret: Type,
    ptr_ty: Type,
    /// Per-local GC roots-array index (`Some` iff the local is a GC root).
    root_slot_of: Vec<Option<u32>>,
    nroots: u32,
    roots_slot: Option<StackSlot>,
    frame_slot: Option<StackSlot>,
}

impl FnGen<'_, '_> {
    fn use_local(&mut self, id: LocalId) -> Value {
        self.builder.use_var(Variable::from_u32(id.index() as u32))
    }

    fn use_operand(&mut self, op: &Operand) -> Value {
        match op {
            Operand::Local(id) => self.use_local(*id),
        }
    }

    /// The declared representation of an operand (drives arithmetic dispatch).
    fn operand_repr(&self, op: &Operand) -> &Repr {
        match op {
            Operand::Local(id) => &self.locals[id.index()].repr,
        }
    }

    /// Define a local. If it is a GC root, mirror the value into the frame roots
    /// array (store-on-def) so the collector can find it (PITFALLS B15).
    fn def_local(&mut self, id: LocalId, val: Value) {
        self.builder.def_var(Variable::from_u32(id.index() as u32), val);
        if let Some(slot_idx) = self.root_slot_of[id.index()] {
            let rs = self.roots_slot.expect("rooted local needs a roots slot");
            self.builder.ins().stack_store(val, rs, (slot_idx * 8) as i32);
        }
    }

    /// Emit the GC frame prologue: zero the roots array, fill the `ShadowFrame`,
    /// and `gc_push` it. No-op for leaf functions (`nroots == 0`).
    fn emit_gc_prologue(&mut self) {
        if self.nroots == 0 {
            return;
        }
        use pyaot_core_defs::layout::{SHADOW_FRAME_NROOTS_OFFSET, SHADOW_FRAME_ROOTS_OFFSET};
        let roots = self.roots_slot.unwrap();
        let frame = self.frame_slot.unwrap();
        let zero = self.builder.ins().iconst(types::I64, 0);
        for i in 0..self.nroots {
            self.builder.ins().stack_store(zero, roots, (i * 8) as i32);
        }
        let nroots_v = self.builder.ins().iconst(types::I64, self.nroots as i64);
        self.builder.ins().stack_store(nroots_v, frame, SHADOW_FRAME_NROOTS_OFFSET);
        let roots_addr = self.builder.ins().stack_addr(self.ptr_ty, roots, 0);
        self.builder.ins().stack_store(roots_addr, frame, SHADOW_FRAME_ROOTS_OFFSET);
        let frame_addr = self.builder.ins().stack_addr(self.ptr_ty, frame, 0);
        self.call(self.rt.gc_push, &[frame_addr]);
    }

    /// `gc_pop` before a return (paired with the prologue's `gc_push`).
    fn emit_gc_epilogue(&mut self) {
        if self.nroots > 0 {
            self.call(self.rt.gc_pop, &[]);
        }
    }

    /// Call a runtime/user function, returning its single result (if any).
    fn call(&mut self, fid: FuncId, args: &[Value]) -> Option<Value> {
        let fref = self.module.declare_func_in_func(fid, self.builder.func);
        let inst = self.builder.ins().call(fref, args);
        let results = self.builder.inst_results(inst);
        results.first().copied()
    }

    fn lower_inst(&mut self, inst: &MirInst) -> Result<()> {
        match inst {
            MirInst::Const { dst, val } => self.lower_const(*dst, val),
            MirInst::Coerce { dst, src, from, to } => self.lower_coerce(*dst, src, from, to),
            MirInst::BinOp { dst, op, l, r } => self.lower_binop(*dst, *op, l, r),
            MirInst::Unary { dst, op, operand } => self.lower_unary(*dst, *op, operand),
            MirInst::Compare { dst, op, l, r } => self.lower_compare(*dst, *op, l, r),
            MirInst::Truthy { dst, operand } => {
                let v = self.use_operand(operand);
                let r = self.call(self.rt.is_truthy, &[v]).unwrap();
                self.def_local(*dst, r);
                Ok(())
            }
            MirInst::Call { dst, func, args } => {
                let vals: Vec<Value> = args.iter().map(|a| self.use_operand(a)).collect();
                let fid = self.func_ids[func.index()];
                let res = self.call(fid, &vals);
                if let (Some(d), Some(v)) = (dst, res) {
                    self.def_local(*d, v);
                }
                Ok(())
            }
            MirInst::CallBuiltin { dst, kind, args } => {
                let vals: Vec<Value> = args.iter().map(|a| self.use_operand(a)).collect();
                let fid = self.builtin_fn(*kind)?;
                let res = self.call(fid, &vals);
                if let (Some(d), Some(v)) = (dst, res) {
                    self.def_local(*d, v);
                }
                Ok(())
            }
            MirInst::AssertFail => {
                let null = self.builder.ins().iconst(self.ptr_ty, 0);
                self.call(self.rt.assert_fail, &[null]);
                Ok(())
            }
            MirInst::Print { kind, arg } => self.lower_print(*kind, arg),
        }
    }

    fn lower_const(&mut self, dst: LocalId, val: &Const) -> Result<()> {
        let v = match val {
            Const::Int(i) => {
                let tagged = ((*i) << tag::INT_SHIFT) | (tag::INT_TAG as i64);
                self.builder.ins().iconst(types::I64, tagged)
            }
            Const::Bool(b) => {
                let tagged = if *b {
                    ((1i64) << tag::BOOL_SHIFT) | (tag::BOOL_TAG as i64)
                } else {
                    tag::BOOL_TAG as i64
                };
                self.builder.ins().iconst(types::I64, tagged)
            }
            Const::None => self.builder.ins().iconst(types::I64, tag::NONE_TAG as i64),
            Const::Float(f) => self.builder.ins().f64const(*f),
            Const::Str(id) => {
                let (ptr, len) = self.str_data(*id)?;
                self.call(self.rt.make_str, &[ptr, len]).unwrap()
            }
            Const::BigIntStr(id) => {
                let (ptr, len) = self.str_data(*id)?;
                self.call(self.rt.bigint_from_str, &[ptr, len]).unwrap()
            }
        };
        self.def_local(dst, v);
        Ok(())
    }

    /// Materialize a string-pool data object's pointer + byte length.
    fn str_data(&mut self, id: InternedString) -> Result<(Value, Value)> {
        let (data_id, len) = *self
            .data_ids
            .get(&id)
            .ok_or_else(|| cg_error("missing data object for interned string"))?;
        let gv = self.module.declare_data_in_func(data_id, self.builder.func);
        let ptr = self.builder.ins().global_value(self.ptr_ty, gv);
        let len_val = self.builder.ins().iconst(types::I64, len as i64);
        Ok((ptr, len_val))
    }

    fn lower_coerce(&mut self, dst: LocalId, src: &Operand, from: &Repr, to: &Repr) -> Result<()> {
        let kind = classify_coercion(from, to)
            .ok_or_else(|| cg_error(format!("illegal coercion {from:?} -> {to:?}")))?;
        let s = self.use_operand(src);
        let v = match kind {
            Coercion::Noop | Coercion::HeapToTagged => s,
            Coercion::BoxFloat => self.call(self.rt.box_float, &[s]).unwrap(),
            Coercion::UnboxFloat => {
                self.builder
                    .ins()
                    .load(types::F64, MemFlags::trusted(), s, FLOAT_VALUE_OFFSET)
            }
            Coercion::TagInt => {
                let shifted = self.builder.ins().ishl_imm(s, tag::INT_SHIFT as i64);
                self.builder.ins().bor_imm(shifted, tag::INT_TAG as i64)
            }
            Coercion::UntagInt => self.builder.ins().sshr_imm(s, tag::INT_SHIFT as i64),
            Coercion::TagBool => {
                let wide = self.builder.ins().uextend(types::I64, s);
                let shifted = self.builder.ins().ishl_imm(wide, tag::BOOL_SHIFT as i64);
                self.builder.ins().bor_imm(shifted, tag::BOOL_TAG as i64)
            }
            Coercion::UntagBool => {
                let shifted = self.builder.ins().ushr_imm(s, tag::BOOL_SHIFT as i64);
                let bit = self.builder.ins().band_imm(shifted, 1);
                self.builder.ins().ireduce(types::I8, bit)
            }
        };
        self.def_local(dst, v);
        Ok(())
    }

    fn lower_binop(&mut self, dst: LocalId, op: BinOp, l: &Operand, r: &Operand) -> Result<()> {
        let lrepr = self.operand_repr(l).clone();
        let a = self.use_operand(l);
        let b = self.use_operand(r);
        // The verifier guarantees both operands and `dst` share `lrepr`, and that
        // a `Raw` operand only ever carries `Add`/`Sub`/`Mul`. Dispatch on it:
        // `Raw(F64)` inlines IEEE float arithmetic (no box, no call); `Tagged`
        // calls the tag-dispatched, bignum-safe `rt_obj_*` shims.
        let v = match (&lrepr, op) {
            (Repr::Raw(RawKind::F64), BinOp::Add) => self.builder.ins().fadd(a, b),
            (Repr::Raw(RawKind::F64), BinOp::Sub) => self.builder.ins().fsub(a, b),
            (Repr::Raw(RawKind::F64), BinOp::Mul) => self.builder.ins().fmul(a, b),
            // Raw i64 (range-proven cursors): checked machine arithmetic that
            // raises on i64 overflow — sound only because lowering proved range.
            (Repr::Raw(RawKind::I64), BinOp::Add) => self.call(self.rt.add_int, &[a, b]).unwrap(),
            (Repr::Raw(RawKind::I64), BinOp::Sub) => self.call(self.rt.sub_int, &[a, b]).unwrap(),
            (Repr::Raw(RawKind::I64), BinOp::Mul) => self.call(self.rt.mul_int, &[a, b]).unwrap(),
            (_, BinOp::Add) => self.call(self.rt.obj_add, &[a, b]).unwrap(),
            (_, BinOp::Sub) => self.call(self.rt.obj_sub, &[a, b]).unwrap(),
            (_, BinOp::Mul) => self.call(self.rt.obj_mul, &[a, b]).unwrap(),
            (_, BinOp::Div) => self.call(self.rt.obj_div, &[a, b]).unwrap(),
            (_, BinOp::FloorDiv) => self.call(self.rt.obj_floordiv, &[a, b]).unwrap(),
            (_, BinOp::Mod) => self.call(self.rt.obj_mod, &[a, b]).unwrap(),
            (_, BinOp::Pow) => self.call(self.rt.obj_pow, &[a, b]).unwrap(),
            // Bitwise/shift dispatch on the tag in the runtime (bignum-safe);
            // operands are Tagged, never raw-unboxed (Invariant 2).
            (_, BinOp::BitAnd) => self.call(self.rt.obj_bitand, &[a, b]).unwrap(),
            (_, BinOp::BitOr) => self.call(self.rt.obj_bitor, &[a, b]).unwrap(),
            (_, BinOp::BitXor) => self.call(self.rt.obj_bitxor, &[a, b]).unwrap(),
            (_, BinOp::Shl) => self.call(self.rt.obj_lshift, &[a, b]).unwrap(),
            (_, BinOp::Shr) => self.call(self.rt.obj_rshift, &[a, b]).unwrap(),
        };
        self.def_local(dst, v);
        Ok(())
    }

    fn lower_unary(&mut self, dst: LocalId, op: UnaryOp, operand: &Operand) -> Result<()> {
        let a = self.use_operand(operand);
        let v = match op {
            UnaryOp::Neg => self.call(self.rt.obj_neg, &[a]).unwrap(),
            UnaryOp::Pos => self.call(self.rt.obj_pos, &[a]).unwrap(),
            UnaryOp::Invert => self.call(self.rt.obj_invert, &[a]).unwrap(),
            UnaryOp::Not => {
                // `not x` = logical-negate truthiness → Raw(I8).
                let t = self.call(self.rt.is_truthy, &[a]).unwrap();
                self.builder.ins().bxor_imm(t, 1)
            }
        };
        self.def_local(dst, v);
        Ok(())
    }

    fn lower_compare(&mut self, dst: LocalId, op: CmpOp, l: &Operand, r: &Operand) -> Result<()> {
        let lrepr = self.operand_repr(l).clone();
        let a = self.use_operand(l);
        let b = self.use_operand(r);
        // Raw i64 (range-proven cursors): a signed machine `icmp` yielding the
        // `I8` boolean directly — no boxing, no `rt_obj_*` call. Bounded fixnums
        // compare identically to Python ints.
        if lrepr == Repr::Raw(RawKind::I64) {
            let cc = match op {
                CmpOp::Eq => IntCC::Equal,
                CmpOp::NotEq => IntCC::NotEqual,
                CmpOp::Lt => IntCC::SignedLessThan,
                CmpOp::LtE => IntCC::SignedLessThanOrEqual,
                CmpOp::Gt => IntCC::SignedGreaterThan,
                CmpOp::GtE => IntCC::SignedGreaterThanOrEqual,
            };
            let v = self.builder.ins().icmp(cc, a, b);
            self.def_local(dst, v);
            return Ok(());
        }
        let v = match op {
            CmpOp::Eq => self.call(self.rt.obj_eq, &[a, b]).unwrap(),
            CmpOp::NotEq => {
                let eq = self.call(self.rt.obj_eq, &[a, b]).unwrap();
                self.builder.ins().bxor_imm(eq, 1)
            }
            CmpOp::Lt | CmpOp::LtE | CmpOp::Gt | CmpOp::GtE => {
                let op_tag = match op {
                    CmpOp::Lt => 0i64,
                    CmpOp::LtE => 1,
                    CmpOp::Gt => 2,
                    CmpOp::GtE => 3,
                    _ => unreachable!(),
                };
                let tag_v = self.builder.ins().iconst(types::I8, op_tag);
                self.call(self.rt.obj_cmp, &[a, b, tag_v]).unwrap()
            }
        };
        self.def_local(dst, v);
        Ok(())
    }

    fn builtin_fn(&self, kind: pyaot_mir::BuiltinFunctionKind) -> Result<FuncId> {
        use pyaot_mir::BuiltinFunctionKind as K;
        Ok(match kind {
            K::Abs => self.rt.builtin_abs,
            K::Int => self.rt.builtin_int,
            K::Float => self.rt.builtin_float,
            K::Str => self.rt.builtin_str,
            K::Bool => self.rt.builtin_bool,
            K::Len => self.rt.builtin_len,
            other => return Err(cg_error(format!("builtin {other:?} not supported in Phase 2"))),
        })
    }

    fn lower_print(&mut self, kind: PrintKind, arg: &Option<Operand>) -> Result<()> {
        match kind {
            PrintKind::Sep => {
                self.call(self.rt.print_sep, &[]);
            }
            PrintKind::Newline => {
                self.call(self.rt.print_newline, &[]);
            }
            PrintKind::None_ => {
                self.call(self.rt.print_none, &[]);
            }
            PrintKind::StrObj => {
                let v = self.use_operand(arg.as_ref().unwrap());
                self.call(self.rt.print_str_obj, &[v]);
            }
            PrintKind::Obj => {
                let v = self.use_operand(arg.as_ref().unwrap());
                self.call(self.rt.print_obj, &[v]);
            }
            PrintKind::Float => {
                let v = self.use_operand(arg.as_ref().unwrap());
                self.call(self.rt.print_float, &[v]);
            }
            PrintKind::Bool => {
                let v = self.use_operand(arg.as_ref().unwrap());
                self.call(self.rt.print_bool, &[v]);
            }
            PrintKind::Int => {
                let v = self.use_operand(arg.as_ref().unwrap());
                self.call(self.rt.print_int, &[v]);
            }
        }
        Ok(())
    }

    fn lower_terminator(&mut self, term: &MirTerminator) -> Result<()> {
        match term {
            MirTerminator::Return(None) => {
                let v = self.default_ret();
                self.emit_gc_epilogue();
                self.builder.ins().return_(&[v]);
            }
            MirTerminator::Return(Some(op)) => {
                let v = self.use_operand(op);
                self.emit_gc_epilogue();
                self.builder.ins().return_(&[v]);
            }
            MirTerminator::Jump(target) => {
                let blk = self.cl_blocks[target.index()];
                self.builder.ins().jump(blk, &[]);
            }
            MirTerminator::Branch { cond, then, else_ } => {
                let c = self.use_operand(cond);
                let t = self.cl_blocks[then.index()];
                let e = self.cl_blocks[else_.index()];
                self.builder.ins().brif(c, t, &[], e, &[]);
            }
            MirTerminator::Unreachable => {
                self.builder.ins().trap(TrapCode::unwrap_user(1));
            }
        }
        Ok(())
    }

    /// A value of the function's return type for `Return(None)` (None-returning
    /// functions have a `Tagged` return → the tagged `None` singleton).
    fn default_ret(&mut self) -> Value {
        if self.program_ret == types::F64 {
            self.builder.ins().f64const(0.0)
        } else if self.program_ret == types::I8 {
            self.builder.ins().iconst(types::I8, 0)
        } else if self.program_ret == types::I32 {
            self.builder.ins().iconst(types::I32, 0)
        } else {
            self.builder.ins().iconst(types::I64, tag::NONE_TAG as i64)
        }
    }
}

fn cg_error(msg: impl Into<String>) -> CompilerError {
    CompilerError::codegen_error(msg.into(), None)
}
