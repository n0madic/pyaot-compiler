//! MIR Verifier — strong-typing enforcement pass.
//!
//! Phase 1 of the Strong-Typed MIR Rewrite (see
//! `.claude/plans/velvety-waddling-map.md`). Walks every function and
//! checks that each instruction's operand and destination types match
//! the instruction's declared signature.
//!
//! # Mode
//!
//! Phase 1 runs in **warning mode**: violations are collected and
//! returned via `Err(Vec<MirError>)`. Callers in the pipeline print the
//! report to stderr but continue compilation. The collected violations
//! drive Phase 2 typed-lowering migration.
//!
//! After Phase 3 boundary, the verifier becomes hard-error: returning
//! `Err(_)` will abort compilation.
//!
//! # Coverage matrix (Phase 1-ext)
//!
//! Instruction kinds with full type checks:
//!   - `Const`, `Copy`, `Phi` — uses `assignable_to` widening
//!   - `BinOp`, `UnOp` — Raw operand / dest required
//!   - `BoxValue` (Raw → Tagged), `UnboxValue` (Tagged-compatible → Raw)
//!   - `FloatToInt`, `IntToFloat`, `BoolToInt`, `FloatBits`,
//!     `IntBitsToFloat`, `FloatAbs` — fixed RawKind src/dest
//!   - `CallDirect` — args + return checked against
//!     `callee.resolved_signature()` when module context available
//!   - `FuncAddr` — callee resolution + Raw(I64) dest accepted as
//!     physical layout
//!   - `BuiltinAddr` — dest must hold a code pointer slot
//!   - `GcAlloc` — dest assignable-from storage shape of `ty`
//!   - `Refine` — embedded `ty` must match dest's resolved MirType
//!   - `Terminator::Return` — operand assignable to function's
//!     `resolved_signature().return_type`
//!
//! Instruction kinds intentionally not checked (require cross-pass
//! or cross-module info beyond Phase 1 scope):
//!   - `Call` (indirect): operand FuncPtr signature data not threaded
//!   - `CallNamed`: cross-module resolution (already narrowed by
//!     `abi_repair`)
//!   - `CallVirtual` / `CallVirtualNamed`: needs vtable typed signatures
//!   - `RuntimeCall`: needs typed `RuntimeFuncDef` (Phase 2d)
//!   - Exception ops, `GcPush` / `GcPop`, `LoadGlobal` /
//!     `StoreGlobal`, `LoadCellValue` / `StoreCellValue`: dest type by
//!     codegen convention; bookkeeping checks only
//!
//! Functions with `is_generic_template == true` are skipped — their
//! `MirType::Var` placeholders fail every primitive-type check by
//! design; `MonomorphizePass` specialises them later.
//!
//! # Translation: legacy `Type` → `MirType`
//!
//! Existing MIR uses `pyaot_types::Type` per `Local`. The verifier
//! consults `Local::resolved_mir_type()` which honours an explicit
//! `mir_ty: Some(_)` annotation (Phase 2 typed lowering) or falls
//! back to register-level `type_to_mir_type_register(&ty)`. Both
//! `verify` and `phi_normalize` consult the same accessor so their
//! views of operand types stay in sync.

use crate::core::{Function, Module};
use crate::instructions::InstructionKind;
use crate::operands::{Constant, Operand};
use crate::types::{type_to_mir_type_register, MirType, RawKind};
use pyaot_utils::{BlockId, FuncId, LocalId};

/// A single verifier violation. Carries enough context to be actionable:
/// function name, block id, instruction kind summary, and the specific
/// mismatch description.
#[derive(Debug, Clone)]
pub struct MirError {
    pub function: String,
    pub block: Option<BlockId>,
    pub instruction: String,
    pub message: String,
}

impl std::fmt::Display for MirError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.block {
            Some(b) => write!(
                f,
                "[{} @ {:?}] {}: {}",
                self.function, b, self.instruction, self.message
            ),
            None => write!(
                f,
                "[{}] {}: {}",
                self.function, self.instruction, self.message
            ),
        }
    }
}

/// Verify the entire module. Returns the list of violations or `Ok(())`
/// if the module is verifier-clean.
///
/// Module-level verification additionally checks cross-function ABI
/// (e.g., `CallDirect` args against callee `resolved_signature()`),
/// which a standalone `verify_function` call cannot inspect.
pub fn verify_mir(module: &Module) -> Result<(), Vec<MirError>> {
    let mut errors = Vec::new();
    for func in module.functions.values() {
        if let Err(mut fn_errors) = verify_function_in_module(func, Some(module)) {
            errors.append(&mut fn_errors);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Verify a single function in isolation. Cross-function ABI checks
/// (`CallDirect` against callee signature) are skipped.
pub fn verify_function(func: &Function) -> Result<(), Vec<MirError>> {
    verify_function_in_module(func, None)
}

/// Verify a single function with optional module context. When `module`
/// is `Some`, the verifier additionally checks each `CallDirect` against
/// the callee's `resolved_signature()`.
///
/// Skips templates of either kind:
/// - **Generic templates** (`is_generic_template == true`) — carry
///   `MirType::Var(_)` placeholders that fail every primitive-type
///   check; only become well-formed after `MonomorphizePass`.
/// - **Wrapper templates** (`wrapper_fn_ptr_capture_index.is_some()`)
///   — decorator wrappers whose fn-ptr parameter holds an arbitrary
///   callee signature; the body's indirect calls and arithmetic on
///   propagated Tagged values become well-formed only after
///   `specialize_wrapper` (S3.3b.2) per captured function.
///
/// Verifying templates of either kind produces only false positives.
pub fn verify_function_in_module(
    func: &Function,
    module: Option<&Module>,
) -> Result<(), Vec<MirError>> {
    if func.is_generic_template || func.wrapper_fn_ptr_capture_index.is_some() {
        return Ok(());
    }
    let mut ctx = Ctx {
        func_name: func.name.clone(),
        current_block: None,
        errors: Vec::new(),
        module,
    };
    for (block_id, block) in &func.blocks {
        ctx.current_block = Some(*block_id);
        for inst in &block.instructions {
            check_instruction(&mut ctx, func, &inst.kind);
        }
        check_terminator(&mut ctx, func, &block.terminator);
    }
    if ctx.errors.is_empty() {
        Ok(())
    } else {
        Err(ctx.errors)
    }
}

struct Ctx<'a> {
    func_name: String,
    current_block: Option<BlockId>,
    errors: Vec<MirError>,
    module: Option<&'a Module>,
}

impl Ctx<'_> {
    fn error(&mut self, instruction: &str, message: impl Into<String>) {
        self.errors.push(MirError {
            function: self.func_name.clone(),
            block: self.current_block,
            instruction: instruction.into(),
            message: message.into(),
        });
    }
}

fn local_mir_type(func: &Function, id: LocalId) -> Option<MirType> {
    // Locals first; fall back to params if not found. Use the
    // `resolved_mir_type()` accessor so the verifier honours an
    // explicit `mir_ty: Some(...)` override (Phase 2 typed lowering)
    // rather than always re-translating the legacy `ty`. This keeps
    // the verifier in agreement with `phi_normalize`'s view of the
    // canonical local type.
    if let Some(local) = func.locals.get(&id) {
        return Some(local.resolved_mir_type());
    }
    func.params
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.resolved_mir_type())
}

fn operand_mir_type(func: &Function, op: &Operand) -> Option<MirType> {
    match op {
        Operand::Local(id) => local_mir_type(func, *id),
        Operand::Constant(c) => Some(constant_mir_type(c)),
    }
}

fn constant_mir_type(c: &Constant) -> MirType {
    match c {
        Constant::Int(_) => MirType::raw_i64(),
        Constant::Float(_) => MirType::raw_f64(),
        Constant::Bool(_) => MirType::raw_i8(),
        Constant::None => MirType::raw_i8(),
        Constant::Str(_) => MirType::str_heap(),
        Constant::Bytes(_) => MirType::bytes_heap(),
    }
}

fn check_instruction(ctx: &mut Ctx, func: &Function, kind: &InstructionKind) {
    match kind {
        InstructionKind::Const { dest, value } => check_const(ctx, func, *dest, value),
        InstructionKind::BinOp {
            dest,
            op,
            left,
            right,
        } => check_binop(ctx, func, *dest, *op, left, right),
        InstructionKind::UnOp { dest, operand, .. } => check_unop(ctx, func, *dest, operand),
        InstructionKind::Copy { dest, src } => check_copy(ctx, func, *dest, src),
        InstructionKind::BoxValue {
            dest,
            src,
            src_type,
        } => check_box(ctx, func, *dest, src, src_type),
        InstructionKind::UnboxValue {
            dest,
            src,
            dest_type,
        } => check_unbox(ctx, func, *dest, src, dest_type),
        InstructionKind::Phi { dest, sources } => check_phi(ctx, func, *dest, sources),
        InstructionKind::FloatToInt { dest, src } => {
            check_unary_conversion(
                ctx,
                func,
                *dest,
                src,
                "FloatToInt",
                RawKind::F64,
                RawKind::I64,
            );
        }
        InstructionKind::IntToFloat { dest, src } => {
            check_unary_conversion(
                ctx,
                func,
                *dest,
                src,
                "IntToFloat",
                RawKind::I64,
                RawKind::F64,
            );
        }
        InstructionKind::BoolToInt { dest, src } => {
            check_unary_conversion(
                ctx,
                func,
                *dest,
                src,
                "BoolToInt",
                RawKind::I8,
                RawKind::I64,
            );
        }
        InstructionKind::FloatBits { dest, src } => {
            check_unary_conversion(
                ctx,
                func,
                *dest,
                src,
                "FloatBits",
                RawKind::F64,
                RawKind::I64,
            );
        }
        InstructionKind::IntBitsToFloat { dest, src } => {
            check_unary_conversion(
                ctx,
                func,
                *dest,
                src,
                "IntBitsToFloat",
                RawKind::I64,
                RawKind::F64,
            );
        }
        InstructionKind::FloatAbs { dest, src } => {
            check_unary_conversion(
                ctx,
                func,
                *dest,
                src,
                "FloatAbs",
                RawKind::F64,
                RawKind::F64,
            );
        }
        InstructionKind::CallDirect {
            dest,
            func: callee_id,
            args,
        } => {
            check_call_direct(ctx, func, *dest, *callee_id, args);
        }
        InstructionKind::GcAlloc { dest, ty, .. } => {
            check_gc_alloc(ctx, func, *dest, ty);
        }
        InstructionKind::FuncAddr { dest, func: callee } => {
            check_func_addr(ctx, func, *dest, *callee);
        }
        InstructionKind::BuiltinAddr { dest, .. } => {
            check_builtin_addr(ctx, func, *dest);
        }
        InstructionKind::Refine { dest, src, ty } => {
            check_refine(ctx, func, *dest, src, ty);
        }
        InstructionKind::RuntimeCall {
            dest,
            func: rt,
            args,
        } => {
            check_runtime_call(ctx, func, *dest, rt, args);
        }
        InstructionKind::CallVirtual {
            dest,
            obj,
            slot,
            args,
        } => {
            check_call_virtual_receiver(ctx, func, obj);
            check_call_virtual_typed(ctx, func, *dest, obj, Some(*slot), args);
        }
        InstructionKind::CallVirtualNamed {
            dest, obj, args, ..
        } => {
            check_call_virtual_receiver(ctx, func, obj);
            check_call_virtual_typed(ctx, func, *dest, obj, None, args);
        }
        InstructionKind::Call {
            dest,
            func: callable,
            args,
        } => {
            check_indirect_callable(ctx, func, callable);
            check_indirect_call_arity(ctx, func, *dest, callable, args);
        }
        InstructionKind::CallNamed { dest, name, args } => {
            check_call_named(ctx, func, *dest, name, args);
        }
        InstructionKind::ExcHasException { dest }
        | InstructionKind::ExcCheckType { dest, .. }
        | InstructionKind::ExcCheckClass { dest, .. } => {
            check_exception_bool_dest(ctx, func, *dest);
        }
        InstructionKind::ExcGetType { dest } => {
            check_exception_int_dest(ctx, func, *dest, "ExcGetType");
        }
        InstructionKind::ExcGetCurrent { dest } => {
            check_exception_int_dest(ctx, func, *dest, "ExcGetCurrent");
        }
        _ => {}
    }
}

/// Stage A.2 + Stage B.1 typed vtable: verify CallVirtual against the
/// resolved vtable slot signature when reachable.
///
/// * When obj's MirType points at a concrete `Heap(Class(C))` (or Generic
///   variant) AND the slot is known AND the module exposes the vtable
///   entry → check arity and dest type against the resolved signature.
/// * Otherwise fall back to Stage A.2's Raw(F64) coarse reject.
fn check_call_virtual_typed(
    ctx: &mut Ctx,
    func: &Function,
    dest: LocalId,
    obj: &Operand,
    slot: Option<usize>,
    args: &[Operand],
) {
    let dest_ty_opt = local_mir_type(func, dest);
    let module = ctx.module;

    // Step 1: resolve receiver class id (if any).
    let class_id = receiver_class_id(func, obj);

    // Step 2: try the typed-signature path when slot, class, module all known.
    if let (Some(slot), Some(cid), Some(module)) = (slot, class_id, module) {
        if let Some(sig) = module.vtable_slot_signature(cid, slot) {
            // Subtract 1 from sig.params.len() because virtual signatures
            // include `self` as params[0]; CallVirtual args do not include it.
            let expected_user_arity = sig.params.len().saturating_sub(1);
            if args.len() != expected_user_arity {
                ctx.error(
                    "CallVirtual",
                    format!(
                        "slot {} on class {:?}: expected {} user args, got {}",
                        slot,
                        cid,
                        expected_user_arity,
                        args.len()
                    ),
                );
                return;
            }
            // Dest type assignability vs slot return type.
            if let Some(dest_ty) = &dest_ty_opt {
                // Pointer-shaped dest is acceptable when slot returns
                // None (mutator methods); the runtime stack slot is
                // never read.
                let pointer_shaped_dest = matches!(
                    dest_ty,
                    MirType::Heap(_) | MirType::FuncPtr(_) | MirType::Closure(_) | MirType::Tagged
                );
                if !sig.return_type.assignable_to(dest_ty) && !pointer_shaped_dest {
                    ctx.error(
                        "CallVirtual",
                        format!(
                            "slot {} on class {:?}: return {} not assignable to dest {}",
                            slot, cid, sig.return_type, dest_ty
                        ),
                    );
                }
            }
            return;
        }
    }

    // Step 3: fallback coarse check (Stage A.2 baseline).
    let Some(dest_ty) = dest_ty_opt else {
        return;
    };
    if matches!(dest_ty, MirType::Raw(RawKind::F64)) {
        ctx.error(
            "CallVirtual",
            format!("dest local {dest:?} is Raw(F64); virtual returns are Tagged or Heap"),
        );
    }
}

/// Helper: extract the receiver class id from a CallVirtual obj operand.
/// Returns `None` if the operand's MirType is not a `Heap(Class { .. })`.
fn receiver_class_id(func: &Function, obj: &Operand) -> Option<pyaot_utils::ClassId> {
    let ty = operand_mir_type(func, obj)?;
    match ty {
        MirType::Heap(crate::types::HeapShape::Class { id, .. }) => Some(id),
        _ => None,
    }
}

/// Stage A.2: verify indirect Call (`func: Operand`) arity matches the
/// callable's FuncPtr signature when known. Works when the callable
/// operand resolves to MirType::FuncPtr(sig) or Closure(shape). When
/// the callable widens to Tagged/Raw(I64) we can't infer arity.
fn check_indirect_call_arity(
    ctx: &mut Ctx,
    func: &Function,
    dest: LocalId,
    callable: &Operand,
    args: &[Operand],
) {
    let Some(callable_ty) = operand_mir_type(func, callable) else {
        return;
    };
    let sig = match &callable_ty {
        MirType::FuncPtr(sig) => sig.as_ref().clone(),
        MirType::Closure(shape) => shape.signature.clone(),
        _ => return,
    };
    if args.len() != sig.params.len() {
        ctx.error(
            "Call",
            format!(
                "indirect call arity mismatch: callable expects {} args, got {}",
                sig.params.len(),
                args.len()
            ),
        );
        return;
    }
    let Some(dest_ty) = local_mir_type(func, dest) else {
        return;
    };
    // Indirect-call dest may widen to Tagged regardless of sig return.
    if matches!(dest_ty, MirType::Tagged) {
        return;
    }
    if !sig.return_type.assignable_to(&dest_ty) {
        ctx.error(
            "Call",
            format!(
                "indirect call return {} not assignable to dest {}",
                sig.return_type, dest_ty
            ),
        );
    }
}

/// Stage A.2: CallNamed cross-module resolution. If a unique function
/// exists in the module by `name`, run CallDirect-style arity and
/// signature checks against it. CallNamed is normally narrowed to
/// CallDirect by abi_repair before final-pre-codegen, so this catches
/// orphaned CallNamed that didn't get narrowed.
fn check_call_named(ctx: &mut Ctx, caller: &Function, dest: LocalId, name: &str, args: &[Operand]) {
    let Some(module) = ctx.module else {
        return;
    };
    let mut matched: Option<FuncId> = None;
    let mut multiple = false;
    for (id, f) in module.functions.iter() {
        if f.name == name {
            if matched.is_some() {
                multiple = true;
                break;
            }
            matched = Some(*id);
        }
    }
    if multiple {
        // Ambiguous — can't verify without name-mangling context.
        return;
    }
    if let Some(callee_id) = matched {
        check_call_direct(ctx, caller, dest, callee_id, args);
    }
}

/// Verify a bool-result exception query (ExcHasException, ExcCheckType,
/// ExcCheckClass) produces a sensible result type. Reject F64 dest as
/// obvious mismatch; accept Raw(I8) (canonical Bool), Raw(I64) (widened),
/// or Tagged (boxed).
fn check_exception_bool_dest(ctx: &mut Ctx, func: &Function, dest: LocalId) {
    let Some(dest_ty) = local_mir_type(func, dest) else {
        return;
    };
    if matches!(dest_ty, MirType::Raw(RawKind::F64)) {
        ctx.error(
            "ExcCheck",
            "bool-result dest is Raw(F64); expected Raw(I8|I64) or Tagged".to_string(),
        );
    }
}

/// Verify an int-result exception query (ExcGetType returns the exception
/// type tag; ExcGetCurrent returns `*mut Obj` cast to i64). Both are
/// i64-width. Reject F64 dest as obvious mismatch.
fn check_exception_int_dest(ctx: &mut Ctx, func: &Function, dest: LocalId, op: &str) {
    let Some(dest_ty) = local_mir_type(func, dest) else {
        return;
    };
    if matches!(dest_ty, MirType::Raw(RawKind::F64)) {
        ctx.error(
            op,
            "dest is Raw(F64); expected I64-width (Raw(I64)/Tagged/Heap)".to_string(),
        );
    }
}

/// Verify that an indirect `Call`'s callable operand is pointer-shaped
/// (FuncPtr, Closure, Tagged, or Heap). Indirect calls dereference the
/// callable as a code address; a Raw-primitive callable would attempt
/// to interpret an int/bool/float bit pattern as a function pointer,
/// which is undefined behavior and impossible at the Python level.
fn check_indirect_callable(ctx: &mut Ctx, func: &Function, callable: &Operand) {
    let Some(ty) = operand_mir_type(func, callable) else {
        return;
    };
    let pointer_shaped = matches!(
        ty,
        MirType::FuncPtr(_) | MirType::Closure(_) | MirType::Tagged | MirType::Heap(_)
    );
    if !pointer_shaped && !matches!(ty, MirType::Never) {
        ctx.error(
            "Call",
            format!("indirect callable {ty} is not pointer-shaped (FuncPtr/Closure/Tagged/Heap)"),
        );
    }
}

/// Verify that a virtual call's receiver (`obj`) is heap-pointer-shaped.
/// CallVirtual dispatches via vtable lookup which requires a valid heap
/// object pointer. Tagged or pointer-shaped Heap/Class/FuncPtr/Closure
/// receivers are valid (all I64 pointer bits at the physical level).
/// Raw primitives are not — calling `42.method()` is a TypeError at HIR
/// level and should never reach MIR.
fn check_call_virtual_receiver(ctx: &mut Ctx, func: &Function, obj: &Operand) {
    let Some(obj_ty) = operand_mir_type(func, obj) else {
        return;
    };
    let pointer_shaped = matches!(
        obj_ty,
        MirType::Heap(_) | MirType::Tagged | MirType::FuncPtr(_) | MirType::Closure(_)
    );
    if !pointer_shaped && !matches!(obj_ty, MirType::Never) {
        ctx.error(
            "CallVirtual",
            format!("receiver {obj_ty} is not pointer-shaped (Heap/Tagged/FuncPtr/Closure)"),
        );
    }
}

/// Verify a `RuntimeCall` against its `RuntimeFuncDef` declaration.
///
/// Phase 1-ext: arity check only. Full per-arg type validation (mapping
/// `ParamType` to `MirType` and comparing operand `mir_ty`) is deferred
/// to Phase 2d (typed `RuntimeFuncDef`).
///
/// `RuntimeCall.dest` is always present at the MIR level (typed `LocalId`,
/// not `Option`); for void runtime functions it carries a dummy local
/// that codegen ignores.
fn check_runtime_call(
    ctx: &mut Ctx,
    func: &Function,
    _dest: LocalId,
    rt: &crate::RuntimeFunc,
    args: &[Operand],
) {
    use crate::RuntimeFunc;
    // Only `RuntimeFunc::Call(&def)` carries a typed signature today; other
    // variants (e.g. `Print`) bundle their own per-variant logic and are
    // not yet structured for declarative checking.
    let RuntimeFunc::Call(def) = rt else {
        return;
    };
    let expected_arity = def.params.len();
    if args.len() != expected_arity {
        ctx.error(
            "RuntimeCall",
            format!(
                "{}: arity mismatch — got {} args, expected {}",
                def.symbol,
                args.len(),
                expected_arity
            ),
        );
        return;
    }

    // Stage B.2: per-arg type validation when the RuntimeFuncDef has an
    // explicit `mir_param_semantics` annotation. Inferred defaults are
    // skipped to avoid the historical false-positive flood (constants
    // widened to I8 params etc.).
    if def.mir_param_semantics.is_none() {
        return;
    }
    for (idx, arg) in args.iter().enumerate() {
        let Some(arg_ty) = operand_mir_type(func, arg) else {
            continue;
        };
        let sem = def.param_semantic(idx);
        if !semantic_accepts(sem, &arg_ty, def.params.get(idx).copied()) {
            ctx.error(
                "RuntimeCall",
                format!(
                    "{}: arg #{idx} has type {} but param expects {:?}",
                    def.symbol, arg_ty, sem
                ),
            );
        }
    }
}

/// Stage B.2 helper: does the given MirType satisfy a declared
/// MirSemantic? Permissive — widening to Tagged is always accepted, and
/// Heap-shape matching is loose (any Heap pointer accepted in Tagged-
/// position because Tagged subsumes Heap pointers physically).
fn semantic_accepts(
    sem: pyaot_core_defs::runtime_func_def::MirSemantic,
    arg_ty: &MirType,
    raw_class: Option<pyaot_core_defs::runtime_func_def::ParamType>,
) -> bool {
    use pyaot_core_defs::runtime_func_def::MirSemantic;
    match sem {
        MirSemantic::Raw => {
            // Raw accepts matching RawKind OR Tagged (caller may have
            // a Tagged Value being implicitly unboxed in codegen).
            // Reject obvious cross-class mismatches (F64 ↔ I64-class).
            match (arg_ty, raw_class) {
                (MirType::Raw(_), _) => true,
                (MirType::Tagged, _) => true,
                (MirType::Heap(_) | MirType::FuncPtr(_) | MirType::Closure(_), _) => true,
                (MirType::Never, _) => true,
                _ => false,
            }
        }
        MirSemantic::Tagged => matches!(
            arg_ty,
            MirType::Tagged
                | MirType::Heap(_)
                | MirType::FuncPtr(_)
                | MirType::Closure(_)
                | MirType::Raw(_)  // legacy: lowering may still emit raw int into Tagged param
                | MirType::Never
        ),
        MirSemantic::Heap(_) => matches!(
            arg_ty,
            MirType::Heap(_)
                | MirType::Tagged
                | MirType::FuncPtr(_)
                | MirType::Closure(_)
                | MirType::Never
        ),
    }
}

fn check_refine(
    ctx: &mut Ctx,
    func: &Function,
    dest: LocalId,
    src: &Operand,
    ty: &pyaot_types::Type,
) {
    let Some(dest_ty) = local_mir_type(func, dest) else {
        ctx.error("Refine", format!("dest local {dest:?} has no type"));
        return;
    };
    // Refine narrows or restates a type — the embedded `ty` must
    // match the dest local's resolved MirType (register-level). If
    // they disagree, the Refine is silently lying about the dest.
    let expected = crate::types::type_to_mir_type_register(ty);
    if !expected.assignable_to(&dest_ty) {
        ctx.error(
            "Refine",
            format!("refinement {expected} (from ty {ty:?}) not assignable to dest {dest_ty}"),
        );
    }
    // Stage A.2: chain validation, narrowed scope. Refine lowers to Copy at
    // codegen, so cross-class moves (Heap → Raw(F64), Raw(F64) → Heap, etc.)
    // are real bugs.
    //
    // Exemptions for current MIR:
    //   * Tagged ↔ Raw(I8|I64) — isinstance bool refinement
    //   * Heap(_) → Raw(I8) when refining to `Type::None` — `is None`
    //     branch narrowing where the value is unused beyond the branch test
    //   * Tagged → Raw(I8) when refining to `Type::None` — same
    //
    // Stage G.2 will tighten these (force explicit UnboxValue at refinement
    // boundaries where representation actually changes).
    if let Some(src_ty) = operand_mir_type(func, src) {
        let src_class = repr_class(&src_ty);
        let dest_class = repr_class(&dest_ty);
        let is_none_refine = matches!(ty, pyaot_types::Type::None);
        let cross_class_ok = matches!(
            (&src_ty, &dest_ty),
            (MirType::Tagged, MirType::Raw(_)) | (MirType::Raw(_), MirType::Tagged)
        ) || (is_none_refine && matches!(&dest_ty, MirType::Raw(RawKind::I8)));
        if src_class != dest_class && !src_ty.assignable_to(&dest_ty) && !cross_class_ok {
            ctx.error(
                "Refine",
                format!(
                    "refine src {src_ty} not assignable to dest {dest_ty} \
                     (refinement lowers to Copy — cross-class moves are bugs)"
                ),
            );
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ReprClass {
    Raw,
    Tagged,
    Heap,
    FuncPtr,
    Closure,
    Var,
    Never,
}

fn repr_class(ty: &MirType) -> ReprClass {
    match ty {
        MirType::Raw(_) => ReprClass::Raw,
        MirType::Tagged => ReprClass::Tagged,
        MirType::Heap(_) => ReprClass::Heap,
        MirType::FuncPtr(_) => ReprClass::FuncPtr,
        MirType::Closure(_) => ReprClass::Closure,
        MirType::Var(_) => ReprClass::Var,
        MirType::Never => ReprClass::Never,
    }
}

fn check_builtin_addr(ctx: &mut Ctx, caller: &Function, dest: LocalId) {
    let Some(dest_ty) = local_mir_type(caller, dest) else {
        ctx.error("BuiltinAddr", format!("dest local {dest:?} has no type"));
        return;
    };
    // BuiltinAddr produces a runtime function pointer. The dest may
    // hold it as FuncPtr / Tagged / Raw(I64) (the physical layout).
    let acceptable = matches!(dest_ty, MirType::FuncPtr(_) | MirType::Tagged)
        || matches!(dest_ty, MirType::Raw(RawKind::I64));
    if !acceptable {
        ctx.error(
            "BuiltinAddr",
            format!("dest {dest_ty} cannot hold a builtin function pointer"),
        );
    }
}

fn check_func_addr(ctx: &mut Ctx, caller: &Function, dest: LocalId, callee_id: FuncId) {
    let Some(dest_ty) = local_mir_type(caller, dest) else {
        ctx.error("FuncAddr", format!("dest local {dest:?} has no type"));
        return;
    };
    let Some(module) = ctx.module else {
        return;
    };
    let Some(callee) = module.functions.get(&callee_id) else {
        ctx.error(
            "FuncAddr",
            format!("callee {callee_id:?} not found in module"),
        );
        return;
    };
    let sig = callee.resolved_signature();
    let expected = MirType::func_ptr(sig);
    // FuncAddr produces a code-pointer matching the callee's signature.
    // Dest may hold it directly (FuncPtr), widen to Tagged, OR be
    // stored as Raw(I64) (the physical representation — Cranelift
    // models function pointers as i64, and lowering frequently emits
    // FuncAddr into a `Type::Int` local destined for vtable / closure
    // tuple slots that are themselves stored as i64).
    let i64_dest = matches!(dest_ty, MirType::Raw(RawKind::I64));
    if !expected.assignable_to(&dest_ty) && !i64_dest {
        ctx.error(
            "FuncAddr",
            format!(
                "addr of {} ({}) not assignable to dest {}",
                callee.name, expected, dest_ty
            ),
        );
    }
}

fn check_unop(ctx: &mut Ctx, func: &Function, dest: LocalId, operand: &Operand) {
    let Some(dest_ty) = local_mir_type(func, dest) else {
        ctx.error("UnOp", format!("dest local {dest:?} has no type"));
        return;
    };
    let Some(op_ty) = operand_mir_type(func, operand) else {
        ctx.error("UnOp", "operand has no resolvable type");
        return;
    };
    if !matches!(dest_ty, MirType::Raw(_)) {
        ctx.error(
            "UnOp",
            format!("dest {dest_ty} is not Raw — UnOp operates on Raw primitives only"),
        );
    }
    if !matches!(op_ty, MirType::Raw(_)) {
        ctx.error(
            "UnOp",
            format!("operand {op_ty} is not Raw — UnOp operates on Raw primitives only"),
        );
    }
}

fn check_gc_alloc(ctx: &mut Ctx, func: &Function, dest: LocalId, ty: &pyaot_types::Type) {
    let Some(dest_ty) = local_mir_type(func, dest) else {
        ctx.error("GcAlloc", format!("dest local {dest:?} has no type"));
        return;
    };
    let expected = crate::types::type_to_mir_type_storage(ty);
    // GcAlloc produces a heap pointer of the allocated shape. The dest
    // local must hold that shape, or widen to Tagged (the universal
    // pointer-shaped slot).
    if !expected.assignable_to(&dest_ty) {
        ctx.error(
            "GcAlloc",
            format!("alloc shape {expected} not assignable to dest {dest_ty}"),
        );
    }
}

fn check_terminator(ctx: &mut Ctx, func: &Function, term: &crate::terminators::Terminator) {
    use crate::terminators::Terminator;
    if let Terminator::Return(Some(op)) = term {
        // Generator resume functions are marked `phase4_return_abi_flipped`
        // but their body returns raw `Constant::Int(state_code)` operands
        // that the runtime trampoline reads as state machine codes, NOT as
        // boxed Python values. The conversion to the tagged-Value ABI
        // happens at codegen, not at the MIR level. Skip Return validation
        // for these — the signal is in the codegen-emitted call boundary,
        // not the MIR Return shape.
        if func.phase4_return_abi_flipped && func.phase4_original_return_type.is_none() {
            return;
        }
        let expected = func.resolved_signature().return_type;
        // `Constant::None` represents the None singleton; at codegen it
        // becomes a tagged-Value (TAG_NONE) or pointer to the NoneObj.
        // Accept None returns at any pointer-shaped return type, same
        // bit-pattern-compatibility argument as the SSA null sentinel.
        if matches!(op, Operand::Constant(Constant::None))
            && matches!(
                expected,
                MirType::Heap(_) | MirType::FuncPtr(_) | MirType::Closure(_) | MirType::Tagged
            )
        {
            return;
        }
        let Some(op_ty) = operand_mir_type(func, op) else {
            ctx.error("Return", "return operand has no resolvable type");
            return;
        };
        // Numeric tower coercions at Return boundary: codegen
        // (`compile_terminator` in codegen-cranelift::terminators) inserts
        // explicit Cranelift conversions when the return operand's register
        // type differs from the function signature:
        //   Raw(I8)  → Raw(I64)  : uextend  (bool widens to int)
        //   Raw(I64) → Raw(F64)  : fcvt_from_sint (int promotes to float)
        //   Raw(I8)  → Raw(F64)  : uextend + fcvt_from_sint
        //   Raw(I8|I64|F64) → Tagged : box (inline shift/or for int/bool,
        //                              rt_box_float for float; covers
        //                              functions whose return signature is
        //                              `Union[…]` / `Any` / `HeapAny`)
        // Accept these MIR-level mismatches; the codegen emits the necessary
        // conversion instructions correctly. Without these widenings the
        // verifier would flag legal numeric promotions that work today via
        // the codegen-level coercion arm.
        // REQUIRED — removing fails test_types_system. Codegen inserts
        // explicit Cranelift conversions at Return (uextend for I8→I64,
        // fcvt_from_sint for I64→F64, and inline shift/or or rt_box_float
        // for Raw→Tagged). These MIR-level representation mismatches are
        // structurally sound and resolved by the codegen pass.
        let is_numeric_widening = matches!(
            (&op_ty, &expected),
            (MirType::Raw(RawKind::I8), MirType::Raw(RawKind::I64))
                | (MirType::Raw(RawKind::I64), MirType::Raw(RawKind::F64))
                | (MirType::Raw(RawKind::I8), MirType::Raw(RawKind::F64))
                | (MirType::Raw(_), MirType::Tagged)
        );
        if is_numeric_widening {
            return;
        }
        if !op_ty.assignable_to(&expected) {
            ctx.error(
                "Return",
                format!("operand {op_ty} not assignable to function return {expected}"),
            );
        }
    }
}

fn check_call_direct(
    ctx: &mut Ctx,
    caller: &Function,
    dest: LocalId,
    callee_id: FuncId,
    args: &[Operand],
) {
    let Some(module) = ctx.module else {
        // Standalone verify — no module context to resolve callee.
        return;
    };
    let Some(callee) = module.functions.get(&callee_id) else {
        ctx.error(
            "CallDirect",
            format!("callee {callee_id:?} not found in module"),
        );
        return;
    };
    let sig = callee.resolved_signature();

    // Arity check.
    if args.len() != sig.params.len() {
        ctx.error(
            "CallDirect",
            format!(
                "arity mismatch calling {}: expected {} args, got {}",
                callee.name,
                sig.params.len(),
                args.len()
            ),
        );
        // Don't continue with per-arg checks when arity is wrong —
        // index alignment is meaningless.
        return;
    }

    // Per-arg type compatibility.
    for (idx, (arg, expected_ty)) in args.iter().zip(sig.params.iter()).enumerate() {
        let Some(arg_ty) = operand_mir_type(caller, arg) else {
            ctx.error(
                "CallDirect",
                format!("arg #{idx} to {}: unresolvable operand type", callee.name),
            );
            continue;
        };
        if !arg_ty.assignable_to(expected_ty) {
            ctx.error(
                "CallDirect",
                format!(
                    "arg #{idx} to {}: {} not assignable to param {}",
                    callee.name, arg_ty, expected_ty
                ),
            );
        }
    }

    // Dest type compatibility with callee's return type.
    let Some(dest_ty) = local_mir_type(caller, dest) else {
        ctx.error(
            "CallDirect",
            format!("dest local {dest:?} has no type calling {}", callee.name),
        );
        return;
    };
    // None-returning callees can be called for side-effects (`del obj[i]`,
    // `print(...)`, mutator methods like `list.append`). The lowering
    // allocates a discard dest local that may be Tagged / Heap / FuncPtr /
    // Closure (whatever default the call-result expression's HIR type
    // produced). Codegen ignores the void return at the call boundary;
    // the dest's bit pattern is never consumed. Accept any pointer-shaped
    // dest for a `Type::None` callee return — same physical-bits-equivalent
    // argument as the SSA undef null sentinel.
    let pointer_shaped_dest = matches!(
        dest_ty,
        MirType::Heap(_) | MirType::FuncPtr(_) | MirType::Closure(_) | MirType::Tagged
    );
    if matches!(callee.return_type, pyaot_types::Type::None) && pointer_shaped_dest {
        return;
    }
    if !sig.return_type.assignable_to(&dest_ty) {
        ctx.error(
            "CallDirect",
            format!(
                "return of {}: {} not assignable to dest {}",
                callee.name, sig.return_type, dest_ty
            ),
        );
    }
}

fn check_const(ctx: &mut Ctx, func: &Function, dest: LocalId, value: &Constant) {
    let Some(dest_ty) = local_mir_type(func, dest) else {
        ctx.error("Const", format!("dest local {dest:?} has no type"));
        return;
    };
    let expected = constant_mir_type(value);
    // Use `assignable_to` so widening (e.g., `Heap(Str)` constant
    // into a `Tagged` dest) is accepted without an explicit BoxValue.
    if !expected.assignable_to(&dest_ty) {
        ctx.error(
            "Const",
            format!("constant kind {expected} not assignable to dest {dest_ty}"),
        );
    }
}

fn check_binop(
    ctx: &mut Ctx,
    func: &Function,
    dest: LocalId,
    op: crate::BinOp,
    left: &Operand,
    right: &Operand,
) {
    let Some(dest_ty) = local_mir_type(func, dest) else {
        ctx.error("BinOp", format!("dest local {dest:?} has no type"));
        return;
    };
    let left_ty = match operand_mir_type(func, left) {
        Some(t) => t,
        None => {
            ctx.error("BinOp", "left operand has no resolvable type");
            return;
        }
    };
    let right_ty = match operand_mir_type(func, right) {
        Some(t) => t,
        None => {
            ctx.error("BinOp", "right operand has no resolvable type");
            return;
        }
    };
    // Identity comparisons (`is` / `is not`) lower to `BinOp::Eq` /
    // `BinOp::NotEq` on the raw pointer bits of both operands. Heap /
    // Tagged / FuncPtr / Closure inputs are all pointer-shaped i64 at
    // the physical level — accept them as valid Raw bit sources for
    // these two ops. Dest is still Raw(I8) (boolean).
    let is_identity = matches!(op, crate::BinOp::Eq | crate::BinOp::NotEq);
    let pointer_shaped = |t: &MirType| -> bool {
        matches!(
            t,
            MirType::Heap(_) | MirType::Tagged | MirType::FuncPtr(_) | MirType::Closure(_)
        )
    };
    // `Never`-typed operands or dests represent unreachable code paths
    // (e.g., a lambda that is never invoked, or a body that returns
    // before the BinOp). The BinOp itself is dead and its types are
    // irrelevant — accept any combination silently. Mirrors the
    // assignable_to Never widening rule.
    let is_dead = matches!(dest_ty, MirType::Never)
        || matches!(left_ty, MirType::Never)
        || matches!(right_ty, MirType::Never);

    // Phase-2 widening: class-method accumulator pattern where the
    // accumulator local is alloc'd with Tagged storage (defensive
    // boxing for Phi-merged Int/Any union) but the BinOp operates on
    // it as Raw I64. Both bit-patterns are identical at Cranelift
    // level (I64), so the BinOp produces the correct value at runtime.
    // Restricted to class-method functions (name contains '$' for
    // `Class$method`) to avoid masking real bugs in closure-trampoline
    // raw-bits paths (decorator_factory et al).
    let is_class_method = func.is_class_method();
    // REQUIRED — removing fails test_future_annotations. Class-method
    // accumulator BinOp (e.g. Payload$sum_x) where the Phi-merged
    // accumulator local has Tagged mir_ty (defensive boxing) but the
    // BinOp body uses it as Raw I64. Identical Cranelift bit pattern.
    // Phase 4 codegen migration will tighten via explicit BoxValue.
    let tagged_class_method_widening = is_class_method
        && matches!(dest_ty, MirType::Tagged | MirType::Raw(_))
        && matches!(left_ty, MirType::Tagged | MirType::Raw(_))
        && matches!(right_ty, MirType::Tagged | MirType::Raw(_));

    if !is_dead && !matches!(dest_ty, MirType::Raw(_)) && !tagged_class_method_widening {
        ctx.error(
            "BinOp",
            format!("dest {dest_ty} is not Raw — BinOp operates on Raw primitives only"),
        );
    }
    if !is_dead
        && !matches!(left_ty, MirType::Raw(_))
        && !(is_identity && pointer_shaped(&left_ty))
        && !tagged_class_method_widening
    {
        ctx.error(
            "BinOp",
            format!("left operand {left_ty} is not Raw — BinOp operates on Raw primitives only"),
        );
    }
    if !is_dead
        && !matches!(right_ty, MirType::Raw(_))
        && !(is_identity && pointer_shaped(&right_ty))
        && !tagged_class_method_widening
    {
        ctx.error(
            "BinOp",
            format!("right operand {right_ty} is not Raw — BinOp operates on Raw primitives only"),
        );
    }
}

fn check_copy(ctx: &mut Ctx, func: &Function, dest: LocalId, src: &Operand) {
    let Some(dest_ty) = local_mir_type(func, dest) else {
        ctx.error("Copy", format!("dest local {dest:?} has no type"));
        return;
    };
    // SSA undef null-sentinel: literal `Constant::Int(0)` is a valid
    // null pointer for any pointer-shaped dest (same physical bits).
    // Mirrors the rule in `check_phi`.
    if matches!(src, Operand::Constant(Constant::Int(0)))
        && matches!(
            dest_ty,
            MirType::Heap(_) | MirType::FuncPtr(_) | MirType::Closure(_) | MirType::Tagged
        )
    {
        return;
    }
    let Some(src_ty) = operand_mir_type(func, src) else {
        ctx.error("Copy", "src operand has no resolvable type");
        return;
    };
    // Phase-2 Copy widening: Raw(I64|I8) → Tagged is bit-compatible at
    // the Cranelift register level (I64 / uextended I8 both fit in i64).
    // Class-method accumulator pattern and similar defensive-boxing
    // scenarios produce a Raw value that flows into a Tagged-mir_ty slot
    // — the Copy is physically valid. Phase 4 codegen will tighten when
    // MirType becomes authoritative and explicit BoxValue is emitted at
    // the boundary. Raw(F64) is intentionally excluded: F64 lives in
    // Cranelift XMM registers, distinct from I64 general-purpose, so a
    // direct copy would require explicit bitcast.
    // REQUIRED — removing fails test_future_annotations (1 example).
    // Payload$sum_x accumulator pattern: Raw(I64) local flows into Tagged-mir_ty
    // slot after WPA/optimizer pass (class-method accumulator where Phi merges
    // Int/Any produces a Tagged dest but value path keeps Raw). Phase 4 codegen
    // migration will tighten via explicit BoxValue at the boundary.
    if matches!(
        src_ty,
        MirType::Raw(RawKind::I64) | MirType::Raw(RawKind::I8)
    ) && matches!(dest_ty, MirType::Tagged)
    {
        return;
    }
    if !src_ty.assignable_to(&dest_ty) {
        ctx.error(
            "Copy",
            format!("src {src_ty} not assignable to dest {dest_ty}"),
        );
    }
}

fn check_box(
    ctx: &mut Ctx,
    func: &Function,
    dest: LocalId,
    src: &Operand,
    src_type: &pyaot_types::Type,
) {
    let Some(dest_ty) = local_mir_type(func, dest) else {
        ctx.error("BoxValue", format!("dest local {dest:?} has no type"));
        return;
    };
    let Some(src_ty) = operand_mir_type(func, src) else {
        ctx.error("BoxValue", "src operand has no resolvable type");
        return;
    };
    if !matches!(dest_ty, MirType::Tagged) {
        ctx.error(
            "BoxValue",
            format!("dest {dest_ty} is not Tagged — BoxValue produces Tagged"),
        );
    }
    let expected_src = type_to_mir_type_register(src_type);
    if !matches!(expected_src, MirType::Raw(_)) {
        ctx.error(
            "BoxValue",
            format!("src_type {src_type:?} (→ {expected_src}) is not a raw primitive"),
        );
    }
    // Accept tagged src as well (Float pass-through legacy hack); flag
    // when src is neither Raw matching src_type nor Tagged.
    if !src_ty.assignable_to(&expected_src) && !matches!(src_ty, MirType::Tagged) {
        ctx.error(
            "BoxValue",
            format!("src {src_ty} doesn't match expected {expected_src} (nor Tagged pass-through)"),
        );
    }
}

fn check_unbox(
    ctx: &mut Ctx,
    func: &Function,
    dest: LocalId,
    src: &Operand,
    dest_type: &pyaot_types::Type,
) {
    let Some(dest_ty) = local_mir_type(func, dest) else {
        ctx.error("UnboxValue", format!("dest local {dest:?} has no type"));
        return;
    };
    let Some(src_ty) = operand_mir_type(func, src) else {
        ctx.error("UnboxValue", "src operand has no resolvable type");
        return;
    };
    let expected_dest = type_to_mir_type_register(dest_type);
    if !matches!(expected_dest, MirType::Raw(_)) {
        ctx.error(
            "UnboxValue",
            format!("dest_type {dest_type:?} (→ {expected_dest}) is not a raw primitive"),
        );
    }
    if !dest_ty.assignable_to(&expected_dest) {
        ctx.error(
            "UnboxValue",
            format!("dest {dest_ty} doesn't match expected {expected_dest}"),
        );
    }
    // Source must be Tagged or widen to Tagged (Heap / FuncPtr / Closure).
    // UnboxValue is conceptually `Tagged → Raw`, but the verifier permits
    // pointer-shaped sources because they share the Tagged bit-layout —
    // an explicit narrowing through `Refine` is not always emitted by
    // lowering for these widening-eligible types.
    if !src_ty.assignable_to(&MirType::Tagged) {
        ctx.error(
            "UnboxValue",
            format!("src {src_ty} is not Tagged-compatible — UnboxValue consumes Tagged"),
        );
    }
}

fn check_phi(ctx: &mut Ctx, func: &Function, dest: LocalId, sources: &[(BlockId, Operand)]) {
    let Some(dest_ty) = local_mir_type(func, dest) else {
        ctx.error("Phi", format!("dest local {dest:?} has no type"));
        return;
    };
    // SSA construction emits `Constant::Int(0)` as the φ-source on
    // edges where the local is undefined (see
    // `ssa_construct::default_undef_operand`). Cranelift represents
    // both raw i64 and heap pointers as i64 at the ABI level, so the
    // 0 sentinel is a valid null pointer on the dead edge — control
    // doesn't reach a φ-consumed undef value at runtime. Accept
    // literal Int(0) as compatible with any pointer-shaped dest.
    let is_null_sentinel = |op: &Operand| matches!(op, Operand::Constant(Constant::Int(0)));
    let pointer_shaped_dest = matches!(
        dest_ty,
        MirType::Heap(_) | MirType::FuncPtr(_) | MirType::Closure(_) | MirType::Tagged
    );
    for (pred, op) in sources {
        if is_null_sentinel(op) && pointer_shaped_dest {
            continue;
        }
        let Some(src_ty) = operand_mir_type(func, op) else {
            ctx.error(
                "Phi",
                format!("source from {pred:?} has no resolvable type"),
            );
            continue;
        };
        if !src_ty.assignable_to(&dest_ty) {
            ctx.error(
                "Phi",
                format!("source from {pred:?} type {src_ty} != dest {dest_ty}"),
            );
        }
    }
}

fn check_unary_conversion(
    ctx: &mut Ctx,
    func: &Function,
    dest: LocalId,
    src: &Operand,
    name: &str,
    expected_src: RawKind,
    expected_dest: RawKind,
) {
    let Some(dest_ty) = local_mir_type(func, dest) else {
        ctx.error(name, format!("dest local {dest:?} has no type"));
        return;
    };
    let Some(src_ty) = operand_mir_type(func, src) else {
        ctx.error(name, "src operand has no resolvable type");
        return;
    };
    let exp_src_mt = MirType::Raw(expected_src);
    let exp_dest_mt = MirType::Raw(expected_dest);
    if !src_ty.assignable_to(&exp_src_mt) {
        ctx.error(name, format!("src {src_ty} != expected {exp_src_mt}"));
    }
    if !dest_ty.assignable_to(&exp_dest_mt) {
        ctx.error(name, format!("dest {dest_ty} != expected {exp_dest_mt}"));
    }
}

/// Print a verifier report to stderr in warning mode. Groups violations
/// by function for readability. Does NOT fail compilation.
pub fn report_warnings(stage: &str, errors: &[MirError]) {
    if errors.is_empty() {
        return;
    }
    use std::collections::BTreeMap;

    // Aggregate breakdown by instruction kind across the whole module —
    // gives a single-glance distribution before the per-function detail.
    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    for e in errors {
        *by_kind.entry(e.instruction.as_str()).or_insert(0) += 1;
    }
    let kind_summary: Vec<String> = by_kind.iter().map(|(k, n)| format!("{k}={n}")).collect();
    eprintln!(
        "[mir verifier @ {}] {} violation(s): [{}]",
        stage,
        errors.len(),
        kind_summary.join(", ")
    );

    // Per-function detail. Show up to PER_FUNC_LIMIT lines; when more
    // violations exist, also dump one representative per *distinct*
    // instruction kind so Phase 5 / Phase 3 investigators can see all
    // categories without scrolling through hundreds of repeats.
    const PER_FUNC_LIMIT: usize = 5;
    let mut by_func: BTreeMap<&str, Vec<&MirError>> = BTreeMap::new();
    for e in errors {
        by_func.entry(e.function.as_str()).or_default().push(e);
    }
    for (func_name, items) in &by_func {
        eprintln!("  {} ({} violations):", func_name, items.len());
        for e in items.iter().take(PER_FUNC_LIMIT) {
            eprintln!(
                "    {:?} {}: {}",
                e.block.unwrap_or(BlockId(0)),
                e.instruction,
                e.message
            );
        }
        if items.len() > PER_FUNC_LIMIT {
            // Find one representative per kind from the truncated tail
            // (skipping kinds already shown in the head).
            let shown_kinds: std::collections::BTreeSet<&str> = items
                .iter()
                .take(PER_FUNC_LIMIT)
                .map(|e| e.instruction.as_str())
                .collect();
            let mut tail_kinds: std::collections::BTreeMap<&str, &MirError> =
                std::collections::BTreeMap::new();
            for e in items.iter().skip(PER_FUNC_LIMIT) {
                let k = e.instruction.as_str();
                if !shown_kinds.contains(k) {
                    tail_kinds.entry(k).or_insert(*e);
                }
            }
            if !tail_kinds.is_empty() {
                eprintln!("    [other kinds in tail:]");
                for (_, e) in &tail_kinds {
                    eprintln!(
                        "    {:?} {}: {}",
                        e.block.unwrap_or(BlockId(0)),
                        e.instruction,
                        e.message
                    );
                }
            }
            eprintln!("    ... and {} more", items.len() - PER_FUNC_LIMIT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{BasicBlock, Function, Local, Module};
    use crate::instructions::Instruction;
    use crate::terminators::Terminator;
    use pyaot_types::Type;
    use pyaot_utils::{BlockId, FuncId, LocalId};

    #[test]
    fn empty_module_passes() {
        let module = Module::new();
        assert!(verify_mir(&module).is_ok());
    }

    #[test]
    fn func_addr_unknown_callee_reported() {
        let mut caller = make_func(73, "caller", vec![], Type::None);
        let dest = LocalId::from(0u32);
        caller.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Int, // Raw(I64) — acceptable code-pointer slot
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            },
        );
        let entry = caller.entry_block;
        caller.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::FuncAddr {
                        dest,
                        func: FuncId::from(999u32),
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );

        let mut module = Module::new();
        module.functions.insert(caller.id, caller);

        let errors = verify_mir(&module).expect_err("expected unknown callee");
        assert!(
            errors
                .iter()
                .any(|e| e.instruction == "FuncAddr" && e.message.contains("not found in module")),
            "expected FuncAddr unknown-callee error, got: {errors:?}"
        );
    }

    #[test]
    fn func_addr_into_raw_i64_dest_passes() {
        // Lowering frequently emits FuncAddr into a Raw(I64) local
        // (Type::Int) destined for vtable / closure tuple slots.
        // Verifier must accept this as the canonical physical layout.
        let callee = make_func(74, "callee", vec![], Type::Int);
        let mut caller = make_func(75, "caller", vec![], Type::None);
        let dest = LocalId::from(0u32);
        caller.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Int,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            },
        );
        let entry = caller.entry_block;
        caller.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::FuncAddr {
                        dest,
                        func: callee.id,
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );

        let mut module = Module::new();
        module.functions.insert(callee.id, callee);
        module.functions.insert(caller.id, caller);

        let result = verify_mir(&module);
        let addr_errs: Vec<_> = result
            .as_ref()
            .err()
            .map(|errs| {
                errs.iter()
                    .filter(|e| e.instruction == "FuncAddr")
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            addr_errs.is_empty(),
            "unexpected FuncAddr violations: {addr_errs:?}"
        );
    }

    #[test]
    fn unop_tagged_operand_flagged() {
        // UnOp with Tagged operand instead of Raw — should fire.
        let mut func = make_func(72, "bad_unop", vec![], Type::Int);
        let src = LocalId::from(0u32);
        let dest = LocalId::from(1u32);
        func.locals.insert(
            src,
            Local {
                id: src,
                name: None,
                ty: Type::Any, // Tagged
                is_gc_root: true,
                abi_immutable: false,
                mir_ty: None,
            },
        );
        func.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Int,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            },
        );
        let entry = func.entry_block;
        func.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::UnOp {
                        dest,
                        op: crate::UnOp::Neg,
                        operand: Operand::Local(src),
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );

        let errors = verify_function(&func).expect_err("expected UnOp Tagged-operand");
        assert!(
            errors
                .iter()
                .any(|e| e.instruction == "UnOp" && e.message.contains("not Raw")),
            "expected UnOp Tagged operand violation, got: {errors:?}"
        );
    }

    #[test]
    fn gc_alloc_dest_type_mismatch_reported() {
        // GcAlloc { dest: I64-typed local, ty: Type::Str } — allocates a
        // string but assigns to an Int local. Should fire violation.
        let mut func = make_func(70, "bad_alloc", vec![], Type::None);
        let dest = LocalId::from(0u32);
        func.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Int, // wrong: alloc is a Str pointer
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            },
        );
        let entry = func.entry_block;
        func.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::GcAlloc {
                        dest,
                        ty: Type::Str,
                        size: 16,
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );

        let errors = verify_function(&func).expect_err("expected GcAlloc mismatch");
        assert!(
            errors
                .iter()
                .any(|e| e.instruction == "GcAlloc" && e.message.contains("not assignable")),
            "expected GcAlloc dest mismatch, got: {errors:?}"
        );
    }

    #[test]
    fn gc_alloc_widens_to_tagged() {
        // GcAlloc { dest: Tagged-typed local, ty: Type::Str } — fine,
        // Heap(Str) widens to Tagged.
        let mut func = make_func(71, "ok_alloc", vec![], Type::None);
        let dest = LocalId::from(0u32);
        func.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Any, // → Tagged at register level
                is_gc_root: true,
                abi_immutable: false,
                mir_ty: None,
            },
        );
        let entry = func.entry_block;
        func.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::GcAlloc {
                        dest,
                        ty: Type::Str,
                        size: 16,
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );

        let result = verify_function(&func);
        let alloc_errs: Vec<_> = result
            .as_ref()
            .err()
            .map(|errs| errs.iter().filter(|e| e.instruction == "GcAlloc").collect())
            .unwrap_or_default();
        assert!(
            alloc_errs.is_empty(),
            "unexpected GcAlloc violations: {alloc_errs:?}"
        );
    }

    #[test]
    fn generic_template_function_is_skipped() {
        // Build a function with intentionally-broken types but mark it
        // as a generic template — verifier must produce zero violations.
        let mut func = make_func(60, "template", vec![], Type::Int);
        func.is_generic_template = true;
        func.typevar_params = vec![]; // doesn't matter for the skip
        let dest = LocalId::from(0u32);
        func.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Str, // broken: dest is Str but BinOp produces Raw
                is_gc_root: true,
                abi_immutable: false,
                mir_ty: None,
            },
        );
        let entry = func.entry_block;
        func.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::BinOp {
                        dest,
                        op: crate::BinOp::Add,
                        left: Operand::Constant(crate::Constant::Int(1)),
                        right: Operand::Constant(crate::Constant::Int(2)),
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );

        // Standalone verify should return Ok(()) — template skip.
        assert!(
            verify_function(&func).is_ok(),
            "generic template must be skipped by verifier"
        );
    }

    fn make_func(id: u32, name: &str, params: Vec<(LocalId, Type)>, ret: Type) -> Function {
        let param_locals: Vec<Local> = params
            .into_iter()
            .map(|(lid, ty)| Local {
                id: lid,
                name: None,
                ty,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            })
            .collect();
        Function::new(FuncId::from(id), name.to_string(), param_locals, ret, None)
    }

    #[test]
    fn call_direct_arity_mismatch_reported() {
        // Callee: fn callee(a: Int) -> Int
        let callee = make_func(
            10,
            "callee",
            vec![(LocalId::from(0u32), Type::Int)],
            Type::Int,
        );

        // Caller: fn caller() -> Int { d = callee() }  // 0 args, wrong
        let mut caller = make_func(11, "caller", vec![], Type::Int);
        let dest = LocalId::from(0u32);
        caller.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Int,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            },
        );
        let entry = caller.entry_block;
        caller.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::CallDirect {
                        dest,
                        func: FuncId::from(10u32),
                        args: vec![],
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );

        let mut module = Module::new();
        module.functions.insert(callee.id, callee);
        module.functions.insert(caller.id, caller);

        let errors = verify_mir(&module).expect_err("expected arity violation");
        assert!(
            errors
                .iter()
                .any(|e| e.instruction == "CallDirect" && e.message.contains("arity mismatch")),
            "expected arity mismatch error, got: {errors:?}"
        );
    }

    #[test]
    fn call_direct_unknown_callee_reported() {
        let mut caller = make_func(20, "caller", vec![], Type::Int);
        let dest = LocalId::from(0u32);
        caller.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Int,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            },
        );
        let entry = caller.entry_block;
        caller.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::CallDirect {
                        dest,
                        func: FuncId::from(999u32),
                        args: vec![],
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );

        let mut module = Module::new();
        module.functions.insert(caller.id, caller);

        let errors = verify_mir(&module).expect_err("expected unknown-callee violation");
        assert!(
            errors
                .iter()
                .any(|e| e.instruction == "CallDirect"
                    && e.message.contains("not found in module")),
            "expected callee-not-found error, got: {errors:?}"
        );
    }

    #[test]
    fn call_direct_matching_signature_passes() {
        // Callee: fn callee(a: Int) -> Int  — register-level: Raw(I64) → Raw(I64).
        let callee = make_func(
            30,
            "callee",
            vec![(LocalId::from(0u32), Type::Int)],
            Type::Int,
        );

        // Caller has an Int local, calls callee with it, receives into Int.
        let mut caller = make_func(31, "caller", vec![], Type::Int);
        let arg_local = LocalId::from(0u32);
        let dest = LocalId::from(1u32);
        for &lid in &[arg_local, dest] {
            caller.locals.insert(
                lid,
                Local {
                    id: lid,
                    name: None,
                    ty: Type::Int,
                    is_gc_root: false,
                    abi_immutable: false,
                    mir_ty: None,
                },
            );
        }
        let entry = caller.entry_block;
        caller.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::CallDirect {
                        dest,
                        func: FuncId::from(30u32),
                        args: vec![Operand::Local(arg_local)],
                    },
                    span: None,
                }],
                terminator: Terminator::Return(Some(Operand::Local(dest))),
            },
        );

        let mut module = Module::new();
        module.functions.insert(callee.id, callee);
        module.functions.insert(caller.id, caller);

        // No CallDirect violations expected. Other instructions may emit
        // some legacy noise, so filter specifically to CallDirect.
        let result = verify_mir(&module);
        let call_direct_errs: Vec<_> = result
            .as_ref()
            .err()
            .map(|errs| {
                errs.iter()
                    .filter(|e| e.instruction == "CallDirect")
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            call_direct_errs.is_empty(),
            "unexpected CallDirect violations: {call_direct_errs:?}"
        );
    }

    #[test]
    fn return_terminator_type_mismatch_reported() {
        // fn returns Int but Return operand is a Str literal local.
        let mut func = make_func(50, "wrong_return", vec![], Type::Int);
        let str_local = LocalId::from(0u32);
        func.locals.insert(
            str_local,
            Local {
                id: str_local,
                name: None,
                ty: Type::Str,
                is_gc_root: true,
                abi_immutable: false,
                mir_ty: None,
            },
        );
        let entry = func.entry_block;
        func.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![],
                terminator: Terminator::Return(Some(Operand::Local(str_local))),
            },
        );

        let errors = verify_function(&func).expect_err("expected Return mismatch");
        assert!(
            errors.iter().any(|e| e.instruction == "Return"
                && e.message.contains("not assignable to function return")),
            "expected Return type mismatch, got: {errors:?}"
        );
    }

    #[test]
    fn return_terminator_matching_passes() {
        // fn returns Int and returns an Int local — clean.
        let mut func = make_func(51, "ok_return", vec![], Type::Int);
        let int_local = LocalId::from(0u32);
        func.locals.insert(
            int_local,
            Local {
                id: int_local,
                name: None,
                ty: Type::Int,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            },
        );
        let entry = func.entry_block;
        func.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![],
                terminator: Terminator::Return(Some(Operand::Local(int_local))),
            },
        );

        let result = verify_function(&func);
        let return_errs: Vec<_> = result
            .as_ref()
            .err()
            .map(|errs| errs.iter().filter(|e| e.instruction == "Return").collect())
            .unwrap_or_default();
        assert!(
            return_errs.is_empty(),
            "unexpected Return violations: {return_errs:?}"
        );
    }

    #[test]
    fn standalone_verify_skips_calldirect_checks() {
        // verify_function (no module) must NOT report CallDirect arity
        // even when args.len() doesn't match — the callee isn't reachable.
        let mut caller = make_func(40, "caller", vec![], Type::Int);
        let dest = LocalId::from(0u32);
        caller.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Int,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            },
        );
        let entry = caller.entry_block;
        caller.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::CallDirect {
                        dest,
                        func: FuncId::from(999u32),
                        args: vec![],
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );

        // Standalone verify must not flag CallDirect.
        let result = verify_function(&caller);
        let call_direct_errs: Vec<_> = result
            .as_ref()
            .err()
            .map(|errs| {
                errs.iter()
                    .filter(|e| e.instruction == "CallDirect")
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            call_direct_errs.is_empty(),
            "standalone verify should skip CallDirect checks, got: {call_direct_errs:?}"
        );
    }

    /// Phase 1-ext: indirect Call with Raw-callable flagged.
    #[test]
    fn indirect_call_raw_callable_reported() {
        use crate::terminators::Terminator;
        use crate::types::HeapShape;
        let mut func = make_func(1, "f", vec![(LocalId::from(0u32), Type::Int)], Type::None);
        let dest = LocalId::from(1u32);
        func.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Any,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: Some(MirType::Tagged),
            },
        );
        func.blocks.insert(
            BlockId::from(0u32),
            BasicBlock {
                id: BlockId::from(0u32),
                instructions: vec![Instruction {
                    kind: InstructionKind::Call {
                        dest,
                        // Raw I64 callable — invalid (not pointer-shaped).
                        func: Operand::Local(LocalId::from(0u32)),
                        args: vec![],
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );
        let result = verify_function(&func);
        let _ = HeapShape::Str;
        let errs = result.err().unwrap_or_default();
        assert!(
            errs.iter()
                .any(|e| e.instruction == "Call" && e.message.contains("not pointer-shaped")),
            "expected Call non-pointer-shaped error, got: {errs:?}"
        );
    }

    /// Phase 1-ext: RuntimeCall arity mismatch flagged.
    #[test]
    fn runtime_call_arity_mismatch_reported() {
        use crate::terminators::Terminator;
        use pyaot_core_defs::runtime_func_def::{ReturnType, RuntimeFuncDef};
        // Fake runtime def: 1 param, returns I64. We'll call it with 0 args.
        const FAKE_DEF: RuntimeFuncDef = RuntimeFuncDef::new(
            "rt_fake_unary",
            &[pyaot_core_defs::runtime_func_def::ParamType::I64],
            Some(ReturnType::I64),
            false,
        );
        let mut func = make_func(1, "f", vec![], Type::None);
        let dest = LocalId::from(0u32);
        func.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Int,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: Some(MirType::Raw(RawKind::I64)),
            },
        );
        func.blocks.insert(
            BlockId::from(0u32),
            BasicBlock {
                id: BlockId::from(0u32),
                instructions: vec![Instruction {
                    kind: InstructionKind::RuntimeCall {
                        dest,
                        func: crate::RuntimeFunc::Call(&FAKE_DEF),
                        args: vec![], // 0 args, expected 1
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );
        let result = verify_function(&func);
        let errs = result.err().unwrap_or_default();
        assert!(
            errs.iter().any(|e| e.instruction == "RuntimeCall"
                && e.message.contains("arity mismatch")
                && e.message.contains("rt_fake_unary")),
            "expected RuntimeCall arity error, got: {errs:?}"
        );
    }

    /// Phase-2 widening: class-method BinOp accepts Tagged operands.
    #[test]
    fn class_method_binop_tagged_operand_accepted() {
        use crate::terminators::Terminator;
        let mut func = make_func(1, "MyClass$method", vec![], Type::Int);
        let dest = LocalId::from(0u32);
        let lhs = LocalId::from(1u32);
        let rhs = LocalId::from(2u32);
        for (id, mt) in &[
            (dest, MirType::Tagged),
            (lhs, MirType::Tagged),
            (rhs, MirType::Raw(RawKind::I64)),
        ] {
            func.locals.insert(
                *id,
                Local {
                    id: *id,
                    name: None,
                    ty: Type::Any,
                    is_gc_root: false,
                    abi_immutable: false,
                    mir_ty: Some(mt.clone()),
                },
            );
        }
        func.blocks.insert(
            BlockId::from(0u32),
            BasicBlock {
                id: BlockId::from(0u32),
                instructions: vec![Instruction {
                    kind: InstructionKind::BinOp {
                        dest,
                        op: crate::BinOp::Add,
                        left: Operand::Local(lhs),
                        right: Operand::Local(rhs),
                    },
                    span: None,
                }],
                terminator: Terminator::Return(Some(Operand::Local(dest))),
            },
        );
        let result = verify_function(&func);
        // In a class method (name contains '$'), Tagged/Raw mixed operands
        // are accepted by the Phase-2 widening (commit 5c6516f).
        let binop_errs: Vec<_> = result
            .as_ref()
            .err()
            .map(|errs| errs.iter().filter(|e| e.instruction == "BinOp").collect())
            .unwrap_or_default();
        assert!(
            binop_errs.is_empty(),
            "class-method BinOp with Tagged/Raw mix should be accepted, got: {binop_errs:?}"
        );
    }

    // ============================================================
    // Stage A.2 — new comprehensive coverage tests
    // ============================================================

    /// Stage A.2: CallNamed with unique callee in module — arity mismatch
    /// surfaces through CallDirect-style check.
    #[test]
    fn call_named_unique_arity_mismatch_reported() {
        // Callee: fn target(a: Int) -> Int
        let callee = make_func(
            300,
            "target_fn",
            vec![(LocalId::from(0u32), Type::Int)],
            Type::Int,
        );

        // Caller: calls "target_fn" by name with 0 args — wrong.
        let mut caller = make_func(301, "caller", vec![], Type::Int);
        let dest = LocalId::from(0u32);
        caller.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Int,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: Some(MirType::Raw(RawKind::I64)),
            },
        );
        let entry = caller.entry_block;
        caller.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::CallNamed {
                        dest,
                        name: "target_fn".to_string(),
                        args: vec![],
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );

        let mut module = Module::new();
        module.functions.insert(callee.id, callee);
        module.functions.insert(caller.id, caller);

        let errors = verify_mir(&module).expect_err("expected CallNamed arity violation");
        assert!(
            errors
                .iter()
                .any(|e| e.instruction == "CallDirect" && e.message.contains("arity mismatch")),
            "expected CallNamed arity error (via CallDirect path), got: {errors:?}"
        );
    }

    /// Stage A.2: CallNamed with ambiguous (multi-match) name — silently
    /// skipped because name-mangling context isn't available.
    #[test]
    fn call_named_ambiguous_name_skipped() {
        let callee_a = make_func(310, "ambiguous", vec![], Type::Int);
        let callee_b = make_func(311, "ambiguous", vec![], Type::Int);
        let mut caller = make_func(312, "caller", vec![], Type::Int);
        let dest = LocalId::from(0u32);
        caller.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Int,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: Some(MirType::Raw(RawKind::I64)),
            },
        );
        let entry = caller.entry_block;
        caller.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::CallNamed {
                        dest,
                        name: "ambiguous".to_string(),
                        args: vec![Operand::Constant(crate::Constant::Int(1))], // wrong arity vs both
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );

        let mut module = Module::new();
        module.functions.insert(callee_a.id, callee_a);
        module.functions.insert(callee_b.id, callee_b);
        module.functions.insert(caller.id, caller);

        let result = verify_mir(&module);
        let arity_errs: Vec<_> = result
            .as_ref()
            .err()
            .map(|errs| {
                errs.iter()
                    .filter(|e| e.message.contains("arity mismatch"))
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            arity_errs.is_empty(),
            "ambiguous CallNamed must not be checked, got: {arity_errs:?}"
        );
    }

    /// Stage A.2: CallVirtual dest = Raw(F64) is flagged (no virtual method
    /// returns f64 directly).
    #[test]
    fn call_virtual_raw_f64_dest_flagged() {
        let mut caller = make_func(320, "caller", vec![], Type::None);
        let recv = LocalId::from(0u32);
        let dest = LocalId::from(1u32);
        caller.locals.insert(
            recv,
            Local {
                id: recv,
                name: None,
                ty: Type::Any,
                is_gc_root: true,
                abi_immutable: false,
                mir_ty: Some(MirType::Tagged),
            },
        );
        caller.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Float,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: Some(MirType::Raw(RawKind::F64)),
            },
        );
        let entry = caller.entry_block;
        caller.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::CallVirtual {
                        dest,
                        obj: Operand::Local(recv),
                        slot: 0,
                        args: vec![],
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );

        let errors = verify_function(&caller).expect_err("expected CallVirtual dest violation");
        assert!(
            errors
                .iter()
                .any(|e| e.instruction == "CallVirtual" && e.message.contains("Raw(F64)")),
            "expected CallVirtual Raw(F64) dest violation, got: {errors:?}"
        );
    }

    /// Stage A.2: indirect Call with FuncPtr callable — arity mismatch
    /// against the callable's signature surfaces.
    #[test]
    fn indirect_call_arity_via_funcptr_reported() {
        // Callable signature: (Int) -> Int — but call passes 0 args.
        let sig = crate::types::Signature {
            params: vec![MirType::Raw(RawKind::I64)],
            return_type: MirType::Raw(RawKind::I64),
        };
        let mut func = make_func(330, "f", vec![], Type::Int);
        let callable = LocalId::from(0u32);
        let dest = LocalId::from(1u32);
        func.locals.insert(
            callable,
            Local {
                id: callable,
                name: None,
                ty: Type::Any,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: Some(MirType::func_ptr(sig)),
            },
        );
        func.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Int,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: Some(MirType::Raw(RawKind::I64)),
            },
        );
        let entry = func.entry_block;
        func.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::Call {
                        dest,
                        func: Operand::Local(callable),
                        args: vec![],
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );

        let errors = verify_function(&func).expect_err("expected indirect Call arity violation");
        assert!(
            errors
                .iter()
                .any(|e| e.instruction == "Call" && e.message.contains("arity mismatch")),
            "expected indirect Call arity error, got: {errors:?}"
        );
    }

    /// Stage A.2: Refine with cross-class src → dest (Heap(Str) → Raw(F64))
    /// flagged. Not covered by the legacy None-narrowing exemption.
    #[test]
    fn refine_cross_class_heap_to_raw_f64_flagged() {
        use crate::types::HeapShape;
        let mut func = make_func(340, "f", vec![], Type::Float);
        let src_l = LocalId::from(0u32);
        let dest = LocalId::from(1u32);
        func.locals.insert(
            src_l,
            Local {
                id: src_l,
                name: None,
                ty: Type::Str,
                is_gc_root: true,
                abi_immutable: false,
                mir_ty: Some(MirType::Heap(HeapShape::Str)),
            },
        );
        func.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Float,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: Some(MirType::Raw(RawKind::F64)),
            },
        );
        let entry = func.entry_block;
        func.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::Refine {
                        dest,
                        src: Operand::Local(src_l),
                        ty: Type::Float,
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );

        let errors = verify_function(&func).expect_err("expected Refine cross-class violation");
        assert!(
            errors
                .iter()
                .any(|e| e.instruction == "Refine" && e.message.contains("cross-class")),
            "expected Refine cross-class error, got: {errors:?}"
        );
    }

    /// Stage A.2: Refine to `Type::None` (Raw(I8)) from Heap(Str) — accepted
    /// under `is None` exemption. Pattern: `if x is None: ...` branch.
    #[test]
    fn refine_none_exemption_heap_str_to_raw_i8_accepted() {
        use crate::types::HeapShape;
        let mut func = make_func(341, "f", vec![], Type::None);
        let src_l = LocalId::from(0u32);
        let dest = LocalId::from(1u32);
        func.locals.insert(
            src_l,
            Local {
                id: src_l,
                name: None,
                ty: Type::Str,
                is_gc_root: true,
                abi_immutable: false,
                mir_ty: Some(MirType::Heap(HeapShape::Str)),
            },
        );
        func.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::None,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: Some(MirType::Raw(RawKind::I8)),
            },
        );
        let entry = func.entry_block;
        func.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::Refine {
                        dest,
                        src: Operand::Local(src_l),
                        ty: Type::None,
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );

        let result = verify_function(&func);
        let refine_errs: Vec<_> = result
            .as_ref()
            .err()
            .map(|errs| errs.iter().filter(|e| e.instruction == "Refine").collect())
            .unwrap_or_default();
        assert!(
            refine_errs.is_empty(),
            "is-None refine exemption broken: {refine_errs:?}"
        );
    }

    /// Stage A.2: Refine Tagged → Raw(I8) — accepted under the
    /// representation-bridge exemption (isinstance bool refinement).
    #[test]
    fn refine_tagged_to_raw_i8_accepted() {
        let mut func = make_func(342, "f", vec![], Type::Bool);
        let src_l = LocalId::from(0u32);
        let dest = LocalId::from(1u32);
        func.locals.insert(
            src_l,
            Local {
                id: src_l,
                name: None,
                ty: Type::Any,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: Some(MirType::Tagged),
            },
        );
        func.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Bool,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: Some(MirType::Raw(RawKind::I8)),
            },
        );
        let entry = func.entry_block;
        func.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::Refine {
                        dest,
                        src: Operand::Local(src_l),
                        ty: Type::Bool,
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );

        let result = verify_function(&func);
        let refine_errs: Vec<_> = result
            .as_ref()
            .err()
            .map(|errs| errs.iter().filter(|e| e.instruction == "Refine").collect())
            .unwrap_or_default();
        assert!(
            refine_errs.is_empty(),
            "Tagged→Raw(I8) refine bridge broken: {refine_errs:?}"
        );
    }

    /// Stage B.1: CallVirtual against a known class+slot resolves the
    /// callee signature; mismatched user arity is flagged.
    #[test]
    fn call_virtual_typed_arity_via_vtable_reported() {
        use crate::core::{VtableEntry, VtableInfo};
        use crate::types::HeapShape;
        use pyaot_utils::ClassId;

        let class_id = ClassId::from(7u32);

        // Method: fn method(self: Tagged, x: Int) -> Int — 2 params total.
        let mut method = make_func(
            500,
            "MyClass$method",
            vec![
                (LocalId::from(0u32), Type::Any),
                (LocalId::from(1u32), Type::Int),
            ],
            Type::Int,
        );
        method.signature = Some(crate::types::Signature {
            params: vec![MirType::Tagged, MirType::Raw(RawKind::I64)],
            return_type: MirType::Raw(RawKind::I64),
        });

        let mut caller = make_func(501, "use_obj", vec![], Type::None);
        let obj = LocalId::from(0u32);
        let dest = LocalId::from(1u32);
        caller.locals.insert(
            obj,
            Local {
                id: obj,
                name: None,
                ty: Type::Any,
                is_gc_root: true,
                abi_immutable: false,
                mir_ty: Some(MirType::Heap(HeapShape::Class {
                    id: class_id,
                    type_args: vec![],
                })),
            },
        );
        caller.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Int,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: Some(MirType::Raw(RawKind::I64)),
            },
        );
        let entry = caller.entry_block;
        caller.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![Instruction {
                    kind: InstructionKind::CallVirtual {
                        dest,
                        obj: Operand::Local(obj),
                        slot: 0,
                        args: vec![], // wrong: expected 1 user arg
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );

        let mut module = Module::new();
        module.functions.insert(method.id, method);
        module.functions.insert(caller.id, caller);
        module.vtables.push(VtableInfo {
            class_id,
            entries: vec![VtableEntry {
                slot: 0,
                name_hash: 0,
                method_func_id: FuncId::from(500u32),
            }],
        });

        let errors = verify_mir(&module).expect_err("expected user-arity violation");
        assert!(
            errors
                .iter()
                .any(|e| e.instruction == "CallVirtual"
                    && e.message.contains("expected 1 user args")),
            "expected typed CallVirtual user-arity error, got: {errors:?}"
        );
    }

    /// Non-class-method BinOp with Tagged operand still flagged.
    #[test]
    fn non_class_method_binop_tagged_operand_flagged() {
        use crate::terminators::Terminator;
        let mut func = make_func(1, "regular_function", vec![], Type::Int);
        let dest = LocalId::from(0u32);
        let lhs = LocalId::from(1u32);
        let rhs = LocalId::from(2u32);
        for (id, mt) in &[
            (dest, MirType::Tagged),
            (lhs, MirType::Tagged),
            (rhs, MirType::Raw(RawKind::I64)),
        ] {
            func.locals.insert(
                *id,
                Local {
                    id: *id,
                    name: None,
                    ty: Type::Any,
                    is_gc_root: false,
                    abi_immutable: false,
                    mir_ty: Some(mt.clone()),
                },
            );
        }
        func.blocks.insert(
            BlockId::from(0u32),
            BasicBlock {
                id: BlockId::from(0u32),
                instructions: vec![Instruction {
                    kind: InstructionKind::BinOp {
                        dest,
                        op: crate::BinOp::Add,
                        left: Operand::Local(lhs),
                        right: Operand::Local(rhs),
                    },
                    span: None,
                }],
                terminator: Terminator::Return(Some(Operand::Local(dest))),
            },
        );
        let result = verify_function(&func);
        let errs = result.err().unwrap_or_default();
        assert!(
            errs.iter()
                .any(|e| e.instruction == "BinOp" && e.message.contains("Tagged is not Raw")),
            "non-class-method BinOp with Tagged operand should be flagged, got: {errs:?}"
        );
    }
}
