//! Lowering from HIR to MIR

#![forbid(unsafe_code)]

mod call_resolution;
mod class_metadata;
mod context;
mod exceptions;
mod expressions;
mod generators;
mod narrowing;
mod runtime_selector;
mod statements;
mod type_dispatch;
mod type_planning;
mod utils;

pub use context::{
    CrossModuleClassInfo, ExportedParam, FuncOrBuiltin, LoweredClassInfo, Lowering, SimpleDefault,
};

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{LocalId, VarId};

/// Phase 4 (Storage-Uniform): bit 60 of the closure-stored function
/// pointer marks the user-arg ABI of the callee. When set, the runtime
/// trampoline `rt_call_with_captures_and_args` extracts user args with
/// `extract_tuple_keeping_values` (tagged delivery) instead of
/// `extract_tuple_unwrapping_values` (legacy raw delivery). The callee's
/// prologue unboxes primitive-annotated user params via `UnboxValue` MIR
/// ops. Function pointers on x86_64 / ARM64 fit in 48 bits, so bit 60 is
/// always 0 in canonical user-space addresses.
///
/// **Why bit 60, not bit 63**: the closure tuple stores the func pointer
/// as a `BoxValue { src_type: Int }` tagged Value, encoded as
/// `(p << 3) | 1`. A marker on bit 63 would shift to bit 66 and be lost
/// to truncation; bit 60 shifts to bit 63 (still in i64 range), and the
/// subsequent arithmetic right-shift on unbox sign-extends but the
/// marker-mask check still works (and the trampoline clears bits 60-63
/// before invoking). Mirrors the runtime constant in
/// `crates/runtime/src/tuple/core.rs`.
pub const PHASE4_TAGGED_USER_ARGS_MARKER: i64 = 1i64 << 60;

/// Phase 4 Commit 4 — post-lowering / post-merge rewriter. For every
/// `CallDirect` to a callee whose return ABI was flipped
/// (`mir_func.phase4_return_abi_flipped == true`), insert an `Any`-typed
/// temp + an `UnboxValue` between the call and the original raw
/// primitive dest local. The dest local's declared type stays intact, so
/// consumers (`print`, binops, field stores) see a primitive operand
/// with raw bits while the callee uniformly delivers a tagged Value at
/// the ABI boundary.
///
/// Only fires when the dest local's declared type is one of `Int`,
/// `Bool`, `Float`. For dest types like `Any` / `HeapAny` / `Union`, the
/// unboxing is unnecessary (consumers already handle tagged Values) and
/// skipping the rewrite leaves the existing dataflow rules to
/// materialize the right type at `materialize_function_types`.
///
/// Must run **after** all modules are merged (multi-module compilation
/// produces a single `mir::Module` via `mir_merger`), so cross-module
/// `CallDirect` to a flipped callee is also rewritten. Must run
/// **before** the first `type_inference` / `abi_repair` pass, so the
/// re-typed temp/dest pair establishes the invariants downstream
/// passes expect.
pub fn rewrite_phase4_callee_returns(module: &mut mir::Module) {
    // Collect flipped callees with their *original* primitive return type
    // up front: we need to read other functions while mutating the caller.
    let flipped: indexmap::IndexMap<pyaot_utils::FuncId, Type> = module
        .functions
        .iter()
        .filter_map(|(id, f)| {
            if f.phase4_return_abi_flipped {
                f.phase4_original_return_type
                    .clone()
                    .map(|orig| (*id, orig))
            } else {
                None
            }
        })
        .collect();
    if flipped.is_empty() {
        return;
    }
    // Build slot-lookup tables for `CallVirtual` rewriting. For each
    // class, map `slot_index → method_func_id` so the rewriter can
    // resolve a `CallVirtual { obj: Class{cid}, slot }` to the
    // concrete callee at that slot in `cid`'s vtable, then check
    // whether the callee is flipped. Devirt converts singleton-target
    // `CallVirtual` to `CallDirect` (handled by the existing arm); the
    // remaining `CallVirtual` cases at rewriter time are genuine
    // dynamic dispatch where the receiver class is statically known
    // (from the obj operand's type) but the method binding requires
    // a vtable lookup.
    let vtable_lookup: indexmap::IndexMap<
        pyaot_utils::ClassId,
        indexmap::IndexMap<usize, pyaot_utils::FuncId>,
    > = module
        .vtables
        .iter()
        .map(|v| {
            let slot_map: indexmap::IndexMap<usize, pyaot_utils::FuncId> = v
                .entries
                .iter()
                .map(|e| (e.slot, e.method_func_id))
                .collect();
            (v.class_id, slot_map)
        })
        .collect();
    for func in module.functions.values_mut() {
        let mut next_local_id: u32 = func.locals.keys().map(|id| id.0).max().unwrap_or(0) + 1;
        for p in &func.params {
            if p.id.0 >= next_local_id {
                next_local_id = p.id.0 + 1;
            }
        }
        for block in func.blocks.values_mut() {
            let mut new_instructions: Vec<mir::Instruction> =
                Vec::with_capacity(block.instructions.len());
            for inst in block.instructions.drain(..) {
                // CallDirect → flipped callee: retype dest + insert UnboxValue.
                if let mir::InstructionKind::CallDirect {
                    dest,
                    func: callee,
                    ref args,
                } = inst.kind
                {
                    if let Some(orig_ret) = flipped.get(&callee) {
                        let dest_ty = func.locals.get(&dest).map(|l| l.ty.clone());
                        let should_rewrite = match &dest_ty {
                            // Caller's lowering saw the callee's func_def
                            // and allocated a raw primitive dest.
                            Some(Type::Int) | Some(Type::Bool) | Some(Type::Float) => true,
                            // Multi-module path: caller's lowering had no
                            // callee `func_def` to consult and defaulted
                            // to `Any`. Retype the dest to the original
                            // primitive shape and emit `UnboxValue` so
                            // downstream `rt_*_int` / `rt_*_bool` /
                            // `rt_*_float` consumers (selected from the
                            // HIR-level int/bool/float annotation) see
                            // matching raw bits.
                            Some(Type::Any) => true,
                            _ => false,
                        };
                        if should_rewrite {
                            // Retype dest to the original primitive shape.
                            // Explicitly set mir_ty to the matching Raw(K) so
                            // `computed_is_gc_root()` returns false; without
                            // it the register translation of `Type::Any`
                            // (formerly held) would have falsely tracked the
                            // raw primitive bits on the shadow stack.
                            if let Some(local) = func.locals.get_mut(&dest) {
                                local.ty = orig_ret.clone();
                                local.mir_ty = Some(match orig_ret {
                                    Type::Int => mir::MirType::raw_i64(),
                                    Type::Bool => mir::MirType::raw_i8(),
                                    Type::Float => mir::MirType::raw_f64(),
                                    _ => unreachable!("phase4 flip only over primitive returns"),
                                });
                            }
                            let temp_any_id = pyaot_utils::LocalId::from(next_local_id);
                            next_local_id += 1;
                            func.locals.insert(
                                temp_any_id,
                                mir::Local {
                                    id: temp_any_id,
                                    name: None,
                                    // Phase4-flipped callees return tagged Value bits.
                                    ty: Type::Any,
                                    abi_immutable: false,
                                    is_var_local: false,
                                    mir_ty: Some(mir::MirType::Tagged),
                                },
                            );
                            new_instructions.push(mir::Instruction {
                                kind: mir::InstructionKind::CallDirect {
                                    dest: temp_any_id,
                                    func: callee,
                                    args: args.clone(),
                                },
                                span: inst.span,
                            });
                            new_instructions.push(mir::Instruction {
                                kind: mir::InstructionKind::UnboxValue {
                                    dest,
                                    src: mir::Operand::Local(temp_any_id),
                                    dest_type: orig_ret.clone(),
                                },
                                span: inst.span,
                            });
                            continue;
                        }
                    }
                }
                // CallVirtual → flipped callee at receiver-class slot:
                // same rewrite as CallDirect. Receiver class extracted
                // from the obj operand's type (Type::Class { class_id }
                // / Type::Generic { class_id, .. }).
                if let mir::InstructionKind::CallVirtual {
                    dest,
                    ref obj,
                    slot,
                    ref args,
                } = inst.kind
                {
                    let receiver_class = obj_receiver_class(obj, &func.locals, &func.params);
                    let callee = receiver_class
                        .and_then(|cid| vtable_lookup.get(&cid))
                        .and_then(|slot_map| slot_map.get(&slot).copied());
                    if let Some(callee_id) = callee {
                        if let Some(orig_ret) = flipped.get(&callee_id) {
                            let dest_ty = func.locals.get(&dest).map(|l| l.ty.clone());
                            let should_rewrite = matches!(
                                dest_ty,
                                Some(Type::Int)
                                    | Some(Type::Bool)
                                    | Some(Type::Float)
                                    | Some(Type::Any)
                            );
                            if should_rewrite {
                                // Retype dest to the original primitive shape
                                // with matching Raw(K) mir_ty (same rationale
                                // as the CallDirect arm above).
                                if let Some(local) = func.locals.get_mut(&dest) {
                                    local.ty = orig_ret.clone();
                                    local.mir_ty = Some(match orig_ret {
                                        Type::Int => mir::MirType::raw_i64(),
                                        Type::Bool => mir::MirType::raw_i8(),
                                        Type::Float => mir::MirType::raw_f64(),
                                        _ => {
                                            unreachable!("phase4 flip only over primitive returns")
                                        }
                                    });
                                }
                                let temp_any_id = pyaot_utils::LocalId::from(next_local_id);
                                next_local_id += 1;
                                func.locals.insert(
                                    temp_any_id,
                                    mir::Local {
                                        id: temp_any_id,
                                        name: None,
                                        // Phase4-flipped callees return tagged Value bits.
                                        ty: Type::Any,
                                        abi_immutable: false,
                                        is_var_local: false,
                                        mir_ty: Some(mir::MirType::Tagged),
                                    },
                                );
                                new_instructions.push(mir::Instruction {
                                    kind: mir::InstructionKind::CallVirtual {
                                        dest: temp_any_id,
                                        obj: obj.clone(),
                                        slot,
                                        args: args.clone(),
                                    },
                                    span: inst.span,
                                });
                                new_instructions.push(mir::Instruction {
                                    kind: mir::InstructionKind::UnboxValue {
                                        dest,
                                        src: mir::Operand::Local(temp_any_id),
                                        dest_type: orig_ret.clone(),
                                    },
                                    span: inst.span,
                                });
                                continue;
                            }
                        }
                    }
                }
                new_instructions.push(inst);
            }
            block.instructions = new_instructions;
        }
    }
}

// Stage B.4 of Strong-Typed MIR Rewrite plan v2: `rebox_tagged_any_copies`
// was deleted. The Phase 3a-* monomorphization mir_ty syncs and the
// `box_fusion` pass now ensure post-rewrite Copy instructions never see a
// primitive-source / Any-dest mismatch. 42/42 examples remain
// verifier-clean and runtime-passing without this defensive sweep.

/// Helper for the `CallVirtual` arm of `rewrite_phase4_callee_returns`:
/// given a `CallVirtual.obj` operand, extract the receiver `ClassId` if
/// statically known. Returns `None` when the obj's type is something
/// other than `Type::Class` / `Type::Generic` — those cases don't have
/// a single resolvable vtable.
fn obj_receiver_class(
    obj: &mir::Operand,
    locals: &indexmap::IndexMap<pyaot_utils::LocalId, mir::Local>,
    params: &[mir::Local],
) -> Option<pyaot_utils::ClassId> {
    let local_id = match obj {
        mir::Operand::Local(id) => *id,
        _ => return None,
    };
    let ty = locals
        .get(&local_id)
        .map(|l| &l.ty)
        .or_else(|| params.iter().find(|p| p.id == local_id).map(|p| &p.ty))?;
    match ty {
        Type::Class { class_id, .. } => Some(*class_id),
        Type::Generic { base, .. } => Some(*base),
        _ => None,
    }
}

/// Extract the first argument from an operands vec, defaulting to None.
fn first_arg_or_none(args: Vec<mir::Operand>) -> mir::Operand {
    args.into_iter()
        .next()
        .unwrap_or(mir::Operand::Constant(mir::Constant::None))
}

impl<'a> Lowering<'a> {
    /// Box a primitive value to a tagged `Value` when needed.
    ///
    /// Primitives (Int, Bool, Float, None) must be box-tagged for storage in
    /// dict keys/values, union-typed variables, and any other context
    /// requiring heap-shaped slots. After §F.2:
    /// - `Int`/`Bool` emit inline `ValueFromInt` / `ValueFromBool` MIR
    ///   instructions (`(x << 3) | TAG`) — no runtime call.
    /// - `Float` boxes via `rt_box_float` (heap-allocated `FloatObj`).
    /// - `None` boxes via `rt_box_none` (singleton `NoneObj`).
    /// - Heap types (Str, List, Dict, Tuple, Set, class instances, etc.)
    ///   are already pointers and pass through unchanged.
    ///
    /// Uses `Type::Any` for the boxed result so callers see a uniform
    /// pointer-shaped local.
    pub(crate) fn emit_value_slot(
        &mut self,
        operand: mir::Operand,
        ty: &Type,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        // Boxing-invariant ABI guard for `Type::Float` only: if the caller
        // declared the slot as `Float` but the operand's actual MIR type
        // is `HeapAny` or `Union(_)`, the operand is already a tagged
        // `Value` (FloatObj pointer from `rt_obj_*`, INT / BOOL tag,
        // or NONE pointer). `rt_box_float` is codegen-wired via
        // `load_operand_as(F64)` which bitcasts i64 → f64 — the tagged
        // bits become a bogus denormal, the resulting FloatObj has
        // garbage payload, and any later `rt_unbox_float` round-trip
        // recovers garbage. Pass through instead — the downstream
        // consumer (list[Float] read via `emit_list_get`, tuple[Float]
        // read via `emit_tuple_get`, dict[_, Float], etc.) unboxes via
        // `rt_unbox_float` which dispatches on tag.
        //
        // Repro: microgpt's `m[i] = beta1 * m[i] + (1 - beta1) * p.grad`
        // — Union arithmetic in `(1 - beta1) * p.grad` routes through
        // `rt_obj_mul` returning tagged Value; the list-set lowering
        // calls `emit_value_slot(_, Float)` on the tagged operand →
        // denormal payload (~e-313) in `m[0]`, gradient never
        // accumulates, microgpt loss locks at the uniform baseline.
        //
        // `Type::Any` is intentionally excluded: per the API note,
        // `Any` is *ambiguous* (raw i64 OR pointer) — for class-method
        // params declared without annotation, lowering emits the slot
        // as `Any` while the function's true ABI is set later by the
        // optimizer's WPA narrowing. Passing through an `Any`-typed
        // raw-f64 operand to `rt_unbox_float` would deref the f64 bits
        // as a pointer and SEGV. `HeapAny` is the conservative
        // tag-aware shape (post-§F.7c BigBang) where the value is
        // guaranteed to be in tagged-Value form.
        //
        // Restricted to `Float` because `rt_unbox_float` is the only
        // unbox path that dispatches on the tag at runtime; `Int` /
        // `Bool` use `UnwrapValueInt` / `UnwrapValueBool` (raw
        // arithmetic shifts) that require a specific tag — pass-through
        // there would corrupt list[Int] / list[Bool] reads.
        // Phase 3.5b: emit unified `BoxValue { src_type }` for all
        // primitive types. Codegen lowers to existing primitives (inline
        // shift/or for Int/Bool, rt_box_float for Float, rt_box_none for
        // None). Float pass-through guard for already-tagged operands is
        // built into codegen (see `compile_box_value` in
        // `codegen-cranelift/src/instructions/tag.rs`).
        match ty {
            Type::Int | Type::Bool | Type::Float | Type::None => {
                let dest =
                    self.alloc_and_add_local_with_mir_ty(Type::Any, mir::MirType::Tagged, mir_func);
                self.emit_instruction(mir::InstructionKind::BoxValue {
                    dest,
                    src: operand,
                    src_type: ty.clone(),
                });
                mir::Operand::Local(dest)
            }
            // For `Any`-typed locals whose physical representation is `Raw(_)`,
            // box using the concrete raw type. This arises when lowering emits
            // a CallDirect with `ty: Any` (seed return type before WPA narrows
            // it) but the local later gets `mir_ty: Some(Raw(K))` set — e.g.
            // a function whose return is inferred as `Int` by WPA.  Without
            // this branch the raw i64 would be passed directly to `rt_obj_eq`
            // which expects a tagged Value, causing a SIGSEGV (address=6).
            //
            // We map Raw kind → concrete Type:
            //   Raw(I64) → Int,  Raw(F64) → Float,  Raw(I8) → Bool,
            //   Raw(I32) → pass-through (global slot ID, not a value type).
            Type::Any => {
                if let mir::Operand::Local(id) = &operand {
                    let src_type = match mir_func.locals.get(id).and_then(|l| l.mir_ty.as_ref()) {
                        Some(mir::MirType::Raw(mir::RawKind::I64)) => Some(Type::Int),
                        Some(mir::MirType::Raw(mir::RawKind::F64)) => Some(Type::Float),
                        Some(mir::MirType::Raw(mir::RawKind::I8)) => Some(Type::Bool),
                        _ => None,
                    };
                    if let Some(src_type) = src_type {
                        let dest = self.alloc_and_add_local_with_mir_ty(
                            Type::Any,
                            mir::MirType::Tagged,
                            mir_func,
                        );
                        self.emit_instruction(mir::InstructionKind::BoxValue {
                            dest,
                            src: operand,
                            src_type,
                        });
                        return mir::Operand::Local(dest);
                    }
                }
                // Already tagged (Heap, Tagged mir_ty, or no mir_ty) — pass through.
                operand
            }
            // All heap types are already object pointers — no boxing needed
            _ => operand,
        }
    }

    /// Unbox a tagged-value slot to a raw primitive if needed.
    ///
    /// - `Int`/`Bool`/`Float` with a `Tagged` (or Heap-widened) source:
    ///   emit `UnboxValue { dest_type }`. Codegen lowers this to the
    ///   appropriate tag-strip / value-extract sequence.
    /// - `Int`/`Bool`/`Float` whose source local already has `Raw(K)`
    ///   `mir_ty` matching the target kind: the bits are already in the
    ///   right representation — no unbox needed, return the operand as-is.
    ///   (`UnboxValue` requires a Tagged source; applying it to a Raw local
    ///   violates the MIR verifier invariant.)
    /// - Other types pass through unchanged.
    pub(crate) fn unbox_if_needed(
        &mut self,
        operand: mir::Operand,
        target_type: &Type,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        match target_type {
            Type::Int | Type::Bool | Type::Float => {
                // If the source local is already Raw with the expected kind,
                // skip UnboxValue — it would be invalid (UnboxValue requires
                // a Tagged source). The bits are already the right primitive.
                if let mir::Operand::Local(id) = &operand {
                    let expected = mir::type_to_mir_type_register(target_type);
                    if mir_func.locals.get(id).map(|l| l.resolved_mir_type()) == Some(expected) {
                        return operand;
                    }
                }
                // Source is Tagged (or no mir_ty) — emit UnboxValue to extract
                // the raw primitive. Default register translation gives Raw(K),
                // exactly what codegen expects.
                let dest = self.alloc_and_add_local(target_type.clone(), mir_func);
                self.emit_instruction(mir::InstructionKind::UnboxValue {
                    dest,
                    src: operand,
                    dest_type: target_type.clone(),
                });
                mir::Operand::Local(dest)
            }
            _ => operand,
        }
    }

    /// Emit a runtime call whose result follows the boxing-invariant ABI
    /// (always a tagged `Value`: INT/BOOL tag, NONE pointer, FloatObj pointer,
    /// or heap-object pointer).
    ///
    /// **Narrow-or-box invariant**: a tagged-Value runtime result must never
    /// land in a slot whose static type is a raw primitive (`Int`/`Bool`/
    /// `Float`) without an intervening unbox. Such a slot has
    /// `is_gc_root=false` (because the static type isn't heap-shaped), so
    /// the GC walker would skip it — and any heap pointer (FloatObj from
    /// `rt_obj_pow`, instance from `__add__` dunder, etc.) stored there
    /// becomes a use-after-free as soon as a collection runs.
    ///
    /// Use for `rt_obj_*` family, generator resume returns, and any other
    /// runtime helper that returns a tagged Value. For typed runtime calls
    /// (e.g. `rt_str_concat -> Str`, `rt_list_get -> elem_ty`) keep using
    /// `emit_runtime_call` directly — those return a known shape and the
    /// generic `is_heap()` check is sufficient.
    ///
    /// Behaviour:
    /// - `declared_type ∈ {Int, Bool, Float}` — allocates a `HeapAny`
    ///   intermediate (`is_gc_root=true`), runs the call, then unboxes to
    ///   the primitive via `unbox_if_needed`. The returned local is the
    ///   primitive.
    /// - Otherwise — `declared_type` is heap-shaped (`Union`, `HeapAny`,
    ///   `Class`, `Any`, etc.); allocates directly with the declared type
    ///   and returns the raw call's dest. Asserts `is_gc_root=true` in
    ///   debug builds to catch invariant violations early.
    pub(crate) fn emit_tagged_runtime_call(
        &mut self,
        func: mir::RuntimeFunc,
        args: Vec<mir::Operand>,
        declared_type: Type,
        mir_func: &mut mir::Function,
    ) -> pyaot_utils::LocalId {
        match declared_type {
            Type::Int | Type::Bool | Type::Float => {
                let raw_dest = self.alloc_and_add_local(Type::Any, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: raw_dest,
                    func,
                    args,
                });
                match self.unbox_if_needed(mir::Operand::Local(raw_dest), &declared_type, mir_func)
                {
                    mir::Operand::Local(id) => id,
                    _ => raw_dest,
                }
            }
            _ => {
                let dest = self.alloc_and_add_local(declared_type, mir_func);
                debug_assert!(
                    mir_func
                        .locals
                        .get(&dest)
                        .is_some_and(|l| l.computed_is_gc_root()),
                    "emit_tagged_runtime_call: declared_type must be heap-shaped \
                     (slot must be GC-tracked) — pass a primitive type to take \
                     the unbox path instead"
                );
                self.emit_instruction(mir::InstructionKind::RuntimeCall { dest, func, args });
                dest
            }
        }
    }

    /// Emit a `CallDirect` instruction, handling Phase 4 return ABI unboxing.
    ///
    /// After Phase 4, functions with primitive (Int/Bool/Float) logical return types
    /// actually return a tagged Value (Type::Any in the MIR signature). This helper:
    /// 1. Allocates a `Type::Any` intermediate when `result_ty` is primitive.
    /// 2. Emits the `CallDirect` to that intermediate.
    /// 3. Emits `UnboxValue` to extract the raw primitive into a correctly-typed local.
    ///
    /// For non-primitive return types the behaviour is identical to a bare `CallDirect`.
    ///
    /// Returns the `LocalId` of the final result (raw primitive or original type).
    ///
    /// NOTE: currently unused; will be activated when Phase 4 return ABI (sub-step 4) lands.
    #[allow(dead_code)]
    pub(crate) fn emit_call_direct(
        &mut self,
        func: pyaot_utils::FuncId,
        args: Vec<mir::Operand>,
        result_ty: Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        match result_ty {
            Type::Int | Type::Bool | Type::Float => {
                // Callee returns tagged Value (Any ABI); receive into HeapAny, then unbox.
                let heap_local = self.alloc_and_add_local(Type::Any, mir_func);
                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest: heap_local,
                    func,
                    args,
                });
                match self.unbox_if_needed(mir::Operand::Local(heap_local), &result_ty, mir_func) {
                    mir::Operand::Local(id) => id,
                    _ => heap_local,
                }
            }
            other => {
                let result_local = self.alloc_and_add_local(other, mir_func);
                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest: result_local,
                    func,
                    args,
                });
                result_local
            }
        }
    }

    /// Emit `rt_list_get(list, index)` with correct typed unwrapping.
    ///
    /// After F.7c BigBang Step 2, `rt_list_get` returns the slot's tagged
    /// `Value` bit-pattern. Int/Bool callers must unwrap; Float callers must
    /// unbox. This helper centralises the dispatch so every list-element
    /// read site stays correct after Step 2.
    pub(crate) fn emit_list_get(
        &mut self,
        list_operand: mir::Operand,
        index_operand: mir::Operand,
        elem_ty: &Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        match elem_ty {
            Type::Int | Type::Bool | Type::Float => {
                let heap_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET),
                    vec![list_operand, index_operand],
                    Type::Any,
                    mir_func,
                );
                match self.unbox_if_needed(mir::Operand::Local(heap_local), elem_ty, mir_func) {
                    mir::Operand::Local(id) => id,
                    _ => heap_local,
                }
            }
            // RT_LIST_GET returns a tagged Value (storage-uniform invariant §F.7c).
            // For `Any` elements use HeapAny to signal that the result is guaranteed
            // tagged — this lets binary-ops dispatch correctly route through rt_obj_*.
            Type::Any => self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET),
                vec![list_operand, index_operand],
                Type::Any,
                mir_func,
            ),
            _ => self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET),
                vec![list_operand, index_operand],
                elem_ty.clone(),
                mir_func,
            ),
        }
    }

    /// Get the type of a MIR operand.
    pub(crate) fn operand_type(&self, operand: &mir::Operand, mir_func: &mir::Function) -> Type {
        match operand {
            mir::Operand::Local(id) => mir_func.locals[id].ty.clone(),
            mir::Operand::Constant(c) => match c {
                mir::Constant::Int(_) => Type::Int,
                mir::Constant::Float(_) => Type::Float,
                mir::Constant::Bool(_) => Type::Bool,
                mir::Constant::Str(_) => Type::Str,
                mir::Constant::Bytes(_) => Type::Bytes,
                mir::Constant::None => Type::None,
            },
        }
    }

    /// Prefer the already-lowered MIR operand type when it is concrete; fall
    /// back to the seed/HIR hint only for dynamic `Any`/`HeapAny` cases.
    /// When the lowered operand is `HeapAny` (guaranteed pointer) and the
    /// seed gives `Any`, keep `HeapAny` — `HeapAny` retains the
    /// invariant that the value is in tagged-Value form (post-§F.7c
    /// BigBang), which `binary_ops` and other dispatchers use to route
    /// through `rt_obj_*` instead of treating the bits as a raw `i64`.
    /// `Any` does not carry that guarantee (narrowed return slots from
    /// devirt/inline may still hold raw bits), so demoting the hint
    /// would change downstream dispatch semantics.
    pub(crate) fn resolved_value_type_hint(
        &self,
        expr_id: hir::ExprId,
        operand: &mir::Operand,
        hir_module: &hir::Module,
        mir_func: &mir::Function,
    ) -> Type {
        let lowered = self.operand_type(operand, mir_func);
        if !matches!(lowered, Type::Any) {
            return lowered;
        }
        let hint = self.seed_expr_type(expr_id, hir_module);
        if matches!(lowered, Type::Any) && matches!(hint, Type::Any) {
            return lowered;
        }
        hint
    }

    /// Returns true when an `Any`-typed operand is guaranteed to carry a
    /// tagged Value at runtime — i.e., it was allocated as a compiler
    /// temporary (via `alloc_and_add_local` family) rather than as a HIR
    /// program-variable local (via `get_or_create_local` or as a function
    /// parameter).
    ///
    /// Stage F.2 pre-flight: the predicate previously relied on
    /// `mir_ty: None` as a sentinel for program-variable locals. After
    /// backfill (A.1), all locals carry `mir_ty: Some(...)`, so the
    /// temp-vs-var distinction is encoded by the dedicated `is_var_local`
    /// flag. Variable locals (program-variable slots, function params)
    /// may still carry raw primitive bits from legacy trampolines and
    /// must fall through to raw `BinOp` rather than `rt_obj_*`.
    pub(crate) fn operand_is_guaranteed_tagged(
        &self,
        operand: &mir::Operand,
        ty: &Type,
        mir_func: &mir::Function,
    ) -> bool {
        if !matches!(ty, Type::Any) {
            return false;
        }
        match operand {
            mir::Operand::Local(id) => mir_func.locals.get(id).is_some_and(|l| {
                !l.is_var_local && matches!(l.resolved_mir_type(), mir::MirType::Tagged)
            }),
            mir::Operand::Constant(_) => false,
        }
    }

    fn get_or_create_local(
        &mut self,
        var_id: VarId,
        var_type: Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        if let Some(local_id) = self.get_var_local(&var_id) {
            local_id
        } else {
            // Priority: refined container types > prescan unified type
            // (Area E §E.6) > per-site var_type. Refined types win so
            // `dict[Any, Any]` tightened to `dict[Str, Int]` by the
            // empty-container pass is preserved. Prescan now narrows
            // correctly through `Never`-seeded empty literals (lattice
            // bottom is identity in `join`), so no fallback filter is
            // needed against `*[Any]` shapes.
            let prescan = self
                .lowering_seed_info
                .current_local_seed_types
                .get(&var_id)
                .cloned();
            let raw_ty = self
                .lowering_seed_info
                .refined_container_types
                .get(&var_id)
                .cloned()
                .or(prescan)
                .unwrap_or(var_type);
            // Boundary coercion: an empty literal that was never refined
            // by usage lands here as `list[Never]` etc. The MIR / codegen
            // layer expects a runtime-safe shape — demote both top-level
            // `Never` (would route through the Int sentinel in storage
            // dispatch) and `Never` container parameters (would panic in
            // `type_to_cranelift`) to `Any`.
            let ty = match raw_ty {
                Type::Never => Type::Any,
                other => other.demote_never_params_to_any(),
            };
            let local_id = self.alloc_local_id();
            self.insert_var_local(var_id, local_id);
            mir_func.add_local(mir::Local {
                id: local_id,
                name: None,
                ty: ty.clone(),
                abi_immutable: false,
                is_var_local: true,
                mir_ty: None,
            });
            local_id
        }
    }

    /// Resolve positional and keyword arguments against function parameters.
    /// Returns operands in the order matching function parameters.
    ///
    /// This is the main entry point for call argument resolution. It delegates to
    /// helper functions in the `call_resolution` module for specific tasks.
    ///
    /// If `target_func_id` is provided, mutable defaults (list, dict, set, class instances)
    /// are loaded from global storage instead of being re-evaluated, implementing Python's
    /// semantics where mutable defaults are evaluated once at function definition time.
    ///
    /// The `param_index_offset` adjusts the lookup index for mutable defaults when `params`
    /// doesn't include all original function parameters. For example, when calling `__init__`,
    /// the `self` parameter is skipped (offset=1) because user arguments don't include `self`,
    /// but `default_value_slots` uses indices relative to the original function parameters.
    #[allow(clippy::too_many_arguments)]
    fn resolve_call_args(
        &mut self,
        positional: &[crate::expressions::ExpandedArg],
        kwargs: &[hir::KeywordArg],
        params: &[hir::Param],
        target_func_id: Option<pyaot_utils::FuncId>,
        param_index_offset: usize,
        call_span: pyaot_utils::Span,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<Vec<mir::Operand>> {
        use crate::call_resolution::ParamClassification;
        use pyaot_diagnostics::CompilerError;

        // Step 1: Classify parameters by kind
        let param_class = ParamClassification::from_params(params);

        // Step 2: Lower all positional arguments (handling runtime unpacking)
        let all_positional =
            self.lower_positional_args(positional, &param_class, hir_module, mir_func)?;

        // Step 3: Match positional arguments to regular parameters
        let (mut resolved, extra_positional) =
            self.match_positional_to_params(all_positional, param_class.regular.len());

        // Step 4: Match keyword arguments to parameters
        let (mut kwonly_resolved, extra_keywords) = self.match_kwargs_to_params(
            kwargs,
            &param_class.regular,
            &param_class.kwonly,
            &mut resolved,
            hir_module,
            mir_func,
        )?;

        // Step 5: Process runtime **kwargs dict if present
        if let Some((dict_local, value_type)) = self.take_pending_kwargs() {
            self.process_runtime_kwargs_dict(
                dict_local,
                value_type,
                &param_class.regular,
                &param_class.kwonly,
                &mut resolved,
                &mut kwonly_resolved,
                param_class.kwarg.is_some(),
                hir_module,
                mir_func,
            )?;
        }

        // Step 6: Fill defaults for missing regular params
        // Pass target_func_id so mutable defaults can be loaded from storage
        self.fill_param_defaults(
            &mut resolved,
            &param_class.regular,
            target_func_id,
            param_index_offset,
            call_span,
            hir_module,
            mir_func,
        )?;

        // Step 7: Fill defaults for missing keyword-only params
        // For keyword-only params, compute their offset relative to the original function params:
        // [skipped params] + [regular params] + [*args if present] + [kwonly params]
        let kwonly_offset = param_index_offset
            + param_class.regular.len()
            + if param_class.vararg.is_some() { 1 } else { 0 };
        self.fill_param_defaults(
            &mut kwonly_resolved,
            &param_class.kwonly,
            target_func_id,
            kwonly_offset,
            call_span,
            hir_module,
            mir_func,
        )?;

        // Step 7.5: box arguments for Any/Union-typed params (tagged Value consumer).
        // When a callee param is typed `Any` or `Union(...)`, it expects tagged Value bits.
        // Box raw primitives (Int/Bool/Float) before passing.
        for (i, operand_opt) in resolved.iter_mut().enumerate() {
            if let Some(operand) = operand_opt {
                if i < param_class.regular.len() {
                    let param = &param_class.regular[i];
                    if matches!(&param.ty, Some(Type::Any) | Some(Type::Union(_))) {
                        let arg_type = self.operand_type(operand, mir_func);
                        *operand = self.emit_value_slot(operand.clone(), &arg_type, mir_func);
                    }
                }
            }
        }
        for (i, operand_opt) in kwonly_resolved.iter_mut().enumerate() {
            if let Some(operand) = operand_opt {
                if i < param_class.kwonly.len() {
                    let param = &param_class.kwonly[i];
                    if matches!(&param.ty, Some(Type::Any) | Some(Type::Union(_))) {
                        let arg_type = self.operand_type(operand, mir_func);
                        *operand = self.emit_value_slot(operand.clone(), &arg_type, mir_func);
                    }
                }
            }
        }

        // Step 8: Build result starting with regular params
        let mut result: Vec<mir::Operand> = resolved.into_iter().flatten().collect();

        // Step 9: Build *args tuple from extra positional
        if let Some(vararg_param) = param_class.vararg {
            let tuple_local = self.build_varargs_tuple(extra_positional, vararg_param, mir_func);
            result.push(mir::Operand::Local(tuple_local));
        } else if !extra_positional.is_empty() {
            return Err(CompilerError::too_many_positional_arguments(
                param_class.regular.len(),
                positional.len(),
                call_span,
            ));
        }

        // Step 10: Add keyword-only parameters to result
        result.extend(kwonly_resolved.into_iter().flatten());

        // Step 11: Build **kwargs dict from extra keywords
        if param_class.kwarg.is_some() {
            let kwargs_dict = self.build_kwargs_dict(extra_keywords, mir_func);
            result.push(kwargs_dict);
        } else if !extra_keywords.is_empty() {
            let first_extra_name = extra_keywords
                .keys()
                .next()
                .expect("extra keywords must have at least one element");
            let kwarg_name = self.resolve(*first_extra_name).to_string();
            let kwarg_span = kwargs
                .iter()
                .find(|kw| kw.name == *first_extra_name)
                .map(|kw| kw.span)
                .unwrap_or_else(pyaot_utils::Span::dummy);
            return Err(CompilerError::unexpected_keyword_argument(
                kwarg_name, kwarg_span,
            ));
        } else {
            self.clear_pending_kwargs();
        }

        Ok(result)
    }

    /// Create a tuple from a vector of operands with proper element tag handling.
    /// `operand_types`: optional per-operand types for correct boxing when elem_tag is HEAP_OBJ.
    fn create_tuple_from_operands(
        &mut self,
        operands: &[mir::Operand],
        elem_type: &Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        self.create_tuple_from_operands_typed(operands, elem_type, None, mir_func)
    }

    /// Create a tuple with per-operand type information for correct boxing.
    fn create_tuple_from_operands_typed(
        &mut self,
        operands: &[mir::Operand],
        elem_type: &Type,
        operand_types: Option<&[Type]>,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        // After §F.7c: tuples store uniform tagged Values; box every primitive.
        let tuple_local = self.emit_runtime_call_gc(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_TUPLE),
            vec![mir::Operand::Constant(mir::Constant::Int(
                operands.len() as i64
            ))],
            Type::tuple_of(vec![elem_type.clone()]),
            mir_func,
        );

        for (i, op) in operands.iter().enumerate() {
            let op_type = operand_types
                .and_then(|types| types.get(i))
                .unwrap_or(elem_type);
            let final_operand = self.emit_value_slot(op.clone(), op_type, mir_func);

            self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_SET),
                vec![
                    mir::Operand::Local(tuple_local),
                    mir::Operand::Constant(mir::Constant::Int(i as i64)),
                    final_operand,
                ],
                Type::tuple_of(vec![elem_type.clone()]),
                mir_func,
            );
        }

        tuple_local
    }

    /// Create a combined varargs tuple from extra positional operands + pre-built list tail tuple
    /// Used when calling f(1, 2, *list) where f has *args
    fn create_combined_varargs_tuple(
        &mut self,
        extra_positional: &[mir::Operand],
        list_tail_tuple: LocalId,
        elem_type: &Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        // First, create a tuple from the extra positional operands
        let prefix_tuple = self.create_tuple_from_operands(extra_positional, elem_type, mir_func);

        // Then, concatenate prefix_tuple + list_tail_tuple

        self.emit_runtime_call_gc(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_CONCAT),
            vec![
                mir::Operand::Local(prefix_tuple),
                mir::Operand::Local(list_tail_tuple),
            ],
            Type::tuple_of(vec![Type::Any]),
            mir_func,
        )
    }

    /// Create a dict from keyword arguments
    fn create_dict_from_keywords(
        &mut self,
        keywords: &indexmap::IndexMap<pyaot_utils::InternedString, mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        // Emit: MakeDict(capacity)
        let dict_local = self.emit_runtime_call_gc(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_DICT),
            vec![mir::Operand::Constant(mir::Constant::Int(0))],
            Type::dict_of(Type::Str, Type::Any),
            mir_func,
        );

        // Emit: DictSet for each key-value pair
        for (key_name, value_op) in keywords {
            // key_name is already an InternedString, so we can use it directly
            let key_local = self.emit_runtime_call(
                mir::RuntimeFunc::MakeStr,
                vec![mir::Operand::Constant(mir::Constant::Str(*key_name))],
                Type::Str,
                mir_func,
            );
            self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_SET),
                vec![
                    mir::Operand::Local(dict_local),
                    mir::Operand::Local(key_local),
                    value_op.clone(),
                ],
                Type::dict_of(Type::Str, Type::Any),
                mir_func,
            );
        }

        dict_local
    }

    /// Convert a value from a dict for a specific parameter type.
    /// Dict values are stored as boxed pointers for GC safety.
    /// Primitive types (int, float, bool) need to be unboxed when retrieved.
    fn convert_dict_value_for_param(
        &mut self,
        dict_value_operand: mir::Operand,
        param_type: &Type,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        // Dict values are stored as boxed pointers for GC safety.
        // Primitive types need to be unboxed when retrieved.
        // Heap types (str, list, etc.) are stored as pointers and can be used directly.
        self.unbox_if_needed(dict_value_operand, param_type, mir_func)
    }
}
