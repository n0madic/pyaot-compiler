//! S3.3b.2 Stage D: devirt indirect calls inside specialised wrapper bodies.
//!
//! After `specialize_wrapper` clones a decorator wrapper and retypes its
//! fn-pointer parameter to `Type::Function { … }`, the runtime trampoline
//! calls (`rt_call_with_tuple_args` / `rt_call_with_captures_and_args`) that
//! still carry the indirect dispatch can be rewritten to `CallDirect` —
//! the captured target is statically known.
//!
//! Two ABI shapes are handled:
//!
//! - **Slot-for-slot**: wrapper user-visible parameters match the captured
//!   target's parameters one-to-one. The replacement `CallDirect` reuses
//!   wrapper params verbatim — no unpacking, no extra instructions.
//! - **`*args` unpack**: wrapper takes a single tuple-like user parameter
//!   (the packed `*args`), captured target wants N individual scalars/objects.
//!   For each captured slot we emit `rt_tuple_get(args_tuple, i)` and
//!   per-type unbox (`UnwrapValueInt` / `UnwrapValueBool` / `rt_unbox_float`
//!   / identity for heap objects), feeding the unboxed locals into the
//!   replacement `CallDirect`.
//!
//! Other shapes (dynamic fn-pointer, mismatched non-tuple types) are left
//! untouched — runtime trampoline keeps them correct, devirt is a strict
//! pure win where it applies.
//!
//! The rewrite preserves the call's `dest` local so downstream Phi/Copy
//! plumbing keeps working; WPA pass 2 then retypes the dest to the
//! captured function's return type and DCE/constfold prune any
//! unreachable closure-path branches.

use pyaot_core_defs::runtime_func_def::{RT_TUPLE_GET, RT_UNBOX_FLOAT};
use pyaot_mir::{
    BinOp as MirBinOp, Constant, Function, Instruction, InstructionKind, Local, Operand,
    RuntimeFunc, Terminator,
};
use pyaot_types::Type;
use pyaot_utils::{FuncId, LocalId};

/// Devirtify all `rt_call_with_tuple_args` / `rt_call_with_captures_and_args`
/// invocations inside a specialised wrapper, where the fn-pointer argument
/// traces back to the wrapper's fn-pointer parameter.
///
/// `fn_ptr_param_idx` is the index of the wrapper's fn-pointer parameter
/// (always 0 in the current ABI). `captured_func_id` is the static target.
/// `captured_param_types` are the parameter types of the captured function.
///
/// Returns `true` if any rewrite occurred.
pub fn devirt_wrapper_indirect_calls(
    func: &mut Function,
    fn_ptr_param_idx: usize,
    captured_func_id: FuncId,
    captured_param_types: &[Type],
) -> bool {
    let fn_ptr_param_local = match func.params.get(fn_ptr_param_idx) {
        Some(p) => p.id,
        None => return false,
    };

    // User parameters become the direct-call arguments. Skip the fn-ptr
    // slot — it's the captured-function identity, not a runtime arg.
    let user_params: Vec<&Local> = func
        .params
        .iter()
        .enumerate()
        .filter_map(|(i, p)| (i != fn_ptr_param_idx).then_some(p))
        .collect();

    let strategy = pick_strategy(&user_params, captured_param_types);
    let strategy = match strategy {
        Some(s) => s,
        None => return false,
    };

    // Function-wide alias closure for the fn-pointer parameter — covers
    // raw locals produced by `UnwrapValueInt(fn_ptr_param)` and any
    // `Copy { src: Local }` chains. The runtime trampoline always takes
    // the fn-pointer through one of these aliases.
    let fnptr_aliases = collect_aliases(func, fn_ptr_param_local);

    // For unpack-mode we also need aliases for the `*args` tuple param —
    // a Copy through the same block re-routes it before the call.
    let tuple_aliases = match &strategy {
        Strategy::Unpack {
            args_tuple_local, ..
        } => collect_aliases(func, *args_tuple_local),
        Strategy::Direct { .. } => Vec::new(),
    };

    let rewritten = match strategy {
        Strategy::Direct { user_arg_locals } => {
            rewrite_direct(func, &fnptr_aliases, &user_arg_locals, captured_func_id)
        }
        Strategy::Unpack {
            args_tuple_local: _,
            ..
        } => rewrite_with_unpack(
            func,
            &fnptr_aliases,
            &tuple_aliases,
            captured_func_id,
            captured_param_types,
        ),
    };

    // After rewriting, also collapse the closure-vs-raw type-tag branch in
    // the entry block. The captured target is a static `FuncAddr`, so the
    // closure path (`rt_call_with_captures_and_args` after re-extracting
    // an embedded fn-pointer) is unreachable in any real run, but
    // `rt_get_type_tag` reads the raw fn-pointer's first byte and may
    // return any value at runtime — so the branch isn't statically false.
    // Fold the entry-block `Branch{cond: rt_get_type_tag(fn_ptr) == 6}` to
    // `Goto(else_block)` (the raw-fn path), then immediately prune
    // unreachable blocks + their stale phi sources so subsequent type
    // analysis sees a single-source phi (raw path) and propagates the
    // narrowed return type. We can't rely on DCE running afterward
    // because optimisations are opt-in (e.g. without `-O dce`).
    if rewritten && close_dead_closure_branch(func, &fnptr_aliases) {
        crate::dce::reachability::eliminate_unreachable_blocks(func);
    }

    rewritten
}

/// Pattern-match the entry block's
/// `cond = (rt_get_type_tag(fn_ptr) == 6); Branch{cond, then=closure, else=raw}`
/// shape and replace the terminator with `Goto(else_block)`. No-op when
/// the shape doesn't match — safe fallback.
///
/// Constant `6` corresponds to `TypeTagKind::Tuple` — the runtime check
/// `lower_indirect_call_with_varargs` emits to distinguish a closure-tuple
/// fn-pointer from a raw `FuncAddr`. The check is meaningful for
/// non-specialised wrappers (where the fn-pointer might genuinely be a
/// closure tuple) but always false for specialised wrappers.
fn close_dead_closure_branch(func: &mut Function, fnptr_aliases: &[LocalId]) -> bool {
    let entry_id = func.entry_block;
    let entry = match func.blocks.get(&entry_id) {
        Some(b) => b,
        None => return false,
    };

    let (cond_local, _then_block, else_block) = match &entry.terminator {
        Terminator::Branch {
            cond: Operand::Local(c),
            then_block,
            else_block,
        } => (*c, *then_block, *else_block),
        _ => return false,
    };

    // Find: cond_local = BinOp { op: Eq, left: ttag_local, right: Int(6) }
    let mut ttag_local: Option<LocalId> = None;
    for instr in &entry.instructions {
        if let InstructionKind::BinOp {
            dest,
            op: MirBinOp::Eq,
            left,
            right,
        } = &instr.kind
        {
            if *dest != cond_local {
                continue;
            }
            let left_local = match left {
                Operand::Local(l) => *l,
                _ => continue,
            };
            let is_six = matches!(right, Operand::Constant(Constant::Int(6)));
            if !is_six {
                continue;
            }
            ttag_local = Some(left_local);
            break;
        }
    }
    let ttag_local = match ttag_local {
        Some(l) => l,
        None => return false,
    };

    // Find: ttag_local = RuntimeCall(rt_get_type_tag, [fnptr_alias])
    let typetag_of_fnptr = entry.instructions.iter().any(|inst| {
        let InstructionKind::RuntimeCall { dest, func, args } = &inst.kind else {
            return false;
        };
        if *dest != ttag_local {
            return false;
        }
        let symbol = match func {
            RuntimeFunc::Call(d) => d.symbol,
            _ => return false,
        };
        if symbol != "rt_get_type_tag" {
            return false;
        }
        match args.first() {
            Some(Operand::Local(l)) => fnptr_aliases.contains(l),
            _ => false,
        }
    });

    if !typetag_of_fnptr {
        return false;
    }

    // Replace terminator. The phi-source pruning needed by surviving merge
    // blocks is handled by DCE's `eliminate_unreachable_blocks` later in
    // the pipeline, which has the existing fixup for this case.
    let entry_mut = func
        .blocks
        .get_mut(&entry_id)
        .expect("entry block existed during read");
    entry_mut.terminator = Terminator::Goto(else_block);
    func.invalidate_dom_tree();
    true
}

/// Strategy for the rewrite, computed once per wrapper based on the
/// signature shapes.
enum Strategy {
    /// Wrapper user params match captured params slot-for-slot (no
    /// packing). The replacement `CallDirect` uses wrapper params as-is.
    Direct { user_arg_locals: Vec<LocalId> },
    /// Wrapper has a single tuple-like user param (the packed `*args`).
    /// Replacement emits `rt_tuple_get` + per-type unbox for each
    /// captured slot.
    Unpack { args_tuple_local: LocalId },
}

fn pick_strategy(user_params: &[&Local], captured_param_types: &[Type]) -> Option<Strategy> {
    // Slot-for-slot: same arity, exact type match per slot.
    if user_params.len() == captured_param_types.len()
        && user_params
            .iter()
            .zip(captured_param_types.iter())
            .all(|(p, t)| &p.ty == t)
    {
        return Some(Strategy::Direct {
            user_arg_locals: user_params.iter().map(|p| p.id).collect(),
        });
    }

    // Unpack: single tuple-like user param vs N captured params.
    if user_params.len() == 1 && user_params[0].resolved_mir_type().is_tuple_like() {
        return Some(Strategy::Unpack {
            args_tuple_local: user_params[0].id,
        });
    }

    None
}

/// Collect all locals reachable from `root` through `UnwrapValueInt` and
/// `Copy { src: Local }` chains, function-wide. Iterates to fixpoint
/// because chains can span blocks (Copy in entry, Unwrap later).
fn collect_aliases(func: &Function, root: LocalId) -> Vec<LocalId> {
    let mut aliases = vec![root];
    loop {
        let before = aliases.len();
        for block in func.blocks.values() {
            for instr in &block.instructions {
                match &instr.kind {
                    InstructionKind::UnboxValue {
                        dest,
                        src: Operand::Local(s),
                        dest_type: Type::Int,
                    } if aliases.contains(s) && !aliases.contains(dest) => {
                        aliases.push(*dest);
                    }
                    InstructionKind::Copy {
                        dest,
                        src: Operand::Local(s),
                    } if aliases.contains(s) && !aliases.contains(dest) => {
                        aliases.push(*dest);
                    }
                    _ => {}
                }
            }
        }
        if aliases.len() == before {
            break;
        }
    }
    aliases
}

/// Match the runtime trampoline shape, returning `(dest, args_tuple_idx)`.
///
/// `args_tuple_idx` is the index in the `args` vector that holds the
/// user-args tuple — `1` for `rt_call_with_tuple_args(fn_ptr, args)` and
/// `2` for `rt_call_with_captures_and_args(fn_ptr, captures, args)`.
///
/// The match also requires `args[0]` to be one of the fn-pointer aliases.
fn match_trampoline_call(
    instr: &Instruction,
    fnptr_aliases: &[LocalId],
) -> Option<(LocalId, usize, Vec<Operand>)> {
    let (dest, args, args_tuple_idx) = match &instr.kind {
        InstructionKind::RuntimeCall { dest, func, args } => {
            let symbol = match func {
                RuntimeFunc::Call(def) => def.symbol,
                _ => return None,
            };
            let idx = match symbol {
                "rt_call_with_tuple_args" => 1usize,
                "rt_call_with_captures_and_args" => 2usize,
                _ => return None,
            };
            (dest, args, idx)
        }
        _ => return None,
    };
    let first_arg_local = match args.first() {
        Some(Operand::Local(id)) => *id,
        _ => return None,
    };
    if !fnptr_aliases.contains(&first_arg_local) {
        return None;
    }
    Some((*dest, args_tuple_idx, args.clone()))
}

/// Slot-for-slot rewrite: trampoline → `CallDirect{captured, [user_params...]}`.
///
/// Both trampoline shapes are rewritten: `rt_call_with_tuple_args` (raw
/// fn-ptr path) and `rt_call_with_captures_and_args` (closure-tuple path).
/// In a specialised wrapper the closure path is unreachable in practice
/// (the captured target is a static `FuncAddr`, not a closure tuple), but
/// `rt_get_type_tag` reads the raw fn-pointer's first byte and can return
/// any value — the runtime branch isn't statically false. Rewriting both
/// arms keeps the merge type consistent so the Phi at the join still sees
/// matching scalars instead of a tagged-Value vs raw-int mix.
fn rewrite_direct(
    func: &mut Function,
    fnptr_aliases: &[LocalId],
    user_arg_locals: &[LocalId],
    captured_func_id: FuncId,
) -> bool {
    let mut changed = false;
    for block in func.blocks.values_mut() {
        for instr in block.instructions.iter_mut() {
            let dest = match match_trampoline_call(instr, fnptr_aliases) {
                Some((d, _, _)) => d,
                None => continue,
            };
            let new_args: Vec<Operand> = user_arg_locals
                .iter()
                .copied()
                .map(Operand::Local)
                .collect();
            instr.kind = InstructionKind::CallDirect {
                dest,
                func: captured_func_id,
                args: new_args,
            };
            changed = true;
        }
    }
    if changed {
        func.invalidate_dom_tree();
    }
    changed
}

/// `*args` unpack rewrite: trampoline → `rt_tuple_get` + per-type unbox +
/// `CallDirect{captured, [unboxed...]}`.
///
/// For each captured parameter slot we emit:
/// 1. `tagged = rt_tuple_get(args_tuple, i)` — Value-bits of the i-th arg.
/// 2. unbox according to the captured slot's declared type:
///    - `Int`     → `UnwrapValueInt(tagged)`
///    - `Bool`    → `UnwrapValueBool(tagged)`
///    - `Float`   → `rt_unbox_float(tagged)`
///    - heap/Any  → use `tagged` directly (Value bits == raw ptr for heap
///      objects; the captured target's ABI consumes raw i64).
fn rewrite_with_unpack(
    func: &mut Function,
    fnptr_aliases: &[LocalId],
    tuple_aliases: &[LocalId],
    captured_func_id: FuncId,
    captured_param_types: &[Type],
) -> bool {
    // Allocate fresh local IDs above the current max.
    let mut next_local: u32 = func.locals.keys().map(|id| id.0 + 1).max().unwrap_or(0);
    let mut alloc_local = |func: &mut Function, ty: Type, is_gc_root: bool| -> LocalId {
        let id = LocalId::from(next_local);
        next_local += 1;
        func.locals.insert(
            id,
            Local {
                id,
                name: None,
                ty: ty.clone(),
                is_gc_root,
                abi_immutable: false,
                // Phase 3e: derive mir_ty from `ty` at register level so
                // newly-allocated temps have a well-defined MirType
                // signature for downstream verifier / optimiser passes.
                mir_ty: Some(pyaot_mir::type_to_mir_type_register(&ty)),
            },
        );
        id
    };

    let block_ids: Vec<_> = func.blocks.keys().copied().collect();
    let mut changed = false;
    for block_id in block_ids {
        // Drain the block's instructions into a fresh Vec, inserting unpack
        // sequences before each rewritten trampoline call.
        let old_instrs = std::mem::take(&mut func.blocks[&block_id].instructions);
        let mut new_instrs: Vec<Instruction> = Vec::with_capacity(old_instrs.len());
        for instr in old_instrs {
            let dest = match match_trampoline_call(&instr, fnptr_aliases) {
                Some((d, args_tuple_idx, args)) => {
                    // The args-tuple slot index depends on the trampoline
                    // shape: 1 for `rt_call_with_tuple_args`, 2 for
                    // `rt_call_with_captures_and_args`. The closure-path
                    // trampoline (which extracts args[2]) re-uses the
                    // wrapper's user-visible tuple param verbatim.
                    let tuple_arg_local = match args.get(args_tuple_idx) {
                        Some(Operand::Local(id)) => *id,
                        _ => {
                            new_instrs.push(instr);
                            continue;
                        }
                    };
                    if !tuple_aliases.contains(&tuple_arg_local) {
                        new_instrs.push(instr);
                        continue;
                    }
                    (d, tuple_arg_local)
                }
                None => {
                    new_instrs.push(instr);
                    continue;
                }
            };
            let (dest_local, args_tuple_local) = dest;
            let span = instr.span;

            // Emit unpack + per-type unbox for each captured slot.
            let mut call_args: Vec<Operand> = Vec::with_capacity(captured_param_types.len());
            for (i, captured_ty) in captured_param_types.iter().enumerate() {
                // Step 1: rt_tuple_get(args_tuple, i) -> tagged Value (HeapAny:
                // rt_tuple_get has mir_return_semantic=Tagged; mirrors lowering's
                // closure.rs which already uses Type::Any for the same call).
                let tagged = alloc_local(func, Type::Any, true);
                new_instrs.push(Instruction {
                    kind: InstructionKind::RuntimeCall {
                        dest: tagged,
                        func: RuntimeFunc::Call(&RT_TUPLE_GET),
                        args: vec![
                            Operand::Local(args_tuple_local),
                            Operand::Constant(pyaot_mir::Constant::Int(i as i64)),
                        ],
                    },
                    span,
                });

                // Step 2: per-type unbox.
                let arg_local = match captured_ty {
                    Type::Int => {
                        let unboxed = alloc_local(func, Type::Int, false);
                        new_instrs.push(Instruction {
                            kind: InstructionKind::UnboxValue {
                                dest: unboxed,
                                src: Operand::Local(tagged),
                                dest_type: Type::Int,
                            },
                            span,
                        });
                        unboxed
                    }
                    Type::Bool => {
                        let unboxed = alloc_local(func, Type::Bool, false);
                        new_instrs.push(Instruction {
                            kind: InstructionKind::UnboxValue {
                                dest: unboxed,
                                src: Operand::Local(tagged),
                                dest_type: Type::Bool,
                            },
                            span,
                        });
                        unboxed
                    }
                    Type::Float => {
                        let unboxed = alloc_local(func, Type::Float, false);
                        new_instrs.push(Instruction {
                            kind: InstructionKind::RuntimeCall {
                                dest: unboxed,
                                func: RuntimeFunc::Call(&RT_UNBOX_FLOAT),
                                args: vec![Operand::Local(tagged)],
                            },
                            span,
                        });
                        unboxed
                    }
                    // Heap types (Str, List, Dict, Set, Tuple, Bytes,
                    // Class, Generic{heap}, Any, HeapAny, …) carry raw
                    // pointer bits in `Value` directly: pass the tagged
                    // local through. Codegen consumes i64.
                    _ => tagged,
                };
                call_args.push(Operand::Local(arg_local));
            }

            new_instrs.push(Instruction {
                kind: InstructionKind::CallDirect {
                    dest: dest_local,
                    func: captured_func_id,
                    args: call_args,
                },
                span,
            });
            changed = true;
        }
        func.blocks[&block_id].instructions = new_instrs;
    }

    if changed {
        func.invalidate_dom_tree();
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use pyaot_core_defs::runtime_func_def::RT_CALL_WITH_TUPLE_ARGS;
    use pyaot_mir::{
        BasicBlock, Function as MirFunction, Instruction as MirInstruction, Local, RuntimeFunc,
        Terminator,
    };
    use pyaot_types::Type;

    fn make_wrapper_with_indirect_call() -> MirFunction {
        // wrapper(fn_ptr: Function, x: Int) -> Any
        // body: raw_fn = UnwrapValueInt(fn_ptr); rt_call_with_tuple_args(raw_fn, args_tuple)
        let mut params = vec![
            Local {
                id: LocalId::from(0u32),
                name: None,
                ty: Type::Function {
                    params: vec![Type::Int],
                    ret: Box::new(Type::Int),
                },
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            },
            Local {
                id: LocalId::from(1u32),
                name: None,
                ty: Type::Int,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            },
        ];
        let raw_fn = Local {
            id: LocalId::from(2u32),
            name: None,
            ty: Type::Int,
            is_gc_root: false,
            abi_immutable: false,
            mir_ty: None,
        };
        let args_tuple = Local {
            id: LocalId::from(3u32),
            name: None,
            ty: Type::Any,
            is_gc_root: true,
            abi_immutable: false,
            mir_ty: None,
        };
        let result = Local {
            id: LocalId::from(4u32),
            name: None,
            ty: Type::Any,
            is_gc_root: false,
            abi_immutable: false,
            mir_ty: None,
        };

        let mut func = MirFunction::new(
            FuncId::from(99u32),
            "wrapper@<captured>".to_string(),
            std::mem::take(&mut params),
            Type::Any,
            None,
        );
        func.locals.insert(raw_fn.id, raw_fn);
        func.locals.insert(args_tuple.id, args_tuple);
        func.locals.insert(result.id, result);
        for p in &func.params.clone() {
            func.locals.insert(p.id, p.clone());
        }

        let block = BasicBlock {
            id: func.entry_block,
            instructions: vec![
                MirInstruction {
                    kind: InstructionKind::UnboxValue {
                        dest: LocalId::from(2u32),
                        src: Operand::Local(LocalId::from(0u32)),
                        dest_type: Type::Int,
                    },
                    span: None,
                },
                MirInstruction {
                    kind: InstructionKind::RuntimeCall {
                        dest: LocalId::from(4u32),
                        func: RuntimeFunc::Call(&RT_CALL_WITH_TUPLE_ARGS),
                        args: vec![
                            Operand::Local(LocalId::from(2u32)),
                            Operand::Local(LocalId::from(3u32)),
                        ],
                    },
                    span: None,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Local(LocalId::from(4u32)))),
        };
        func.blocks.clear();
        func.blocks.insert(block.id, block);
        let _: &mut IndexMap<_, _> = &mut func.locals; // silence unused-import warning
        func
    }

    /// Build a `*args` wrapper: params [fn_ptr: Function, args: tuple[Any]].
    /// Body is `UnboxValue(fn_ptr, Int) → raw_fn; rt_call_with_tuple_args(raw_fn, args)`.
    fn make_varargs_wrapper() -> MirFunction {
        let params = vec![
            Local {
                id: LocalId::from(0u32),
                name: None,
                ty: Type::Function {
                    params: vec![Type::Int, Type::Int],
                    ret: Box::new(Type::Int),
                },
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            },
            Local {
                id: LocalId::from(1u32),
                name: None,
                ty: Type::tuple_var_of(Type::Any),
                is_gc_root: true,
                abi_immutable: false,
                mir_ty: None,
            },
        ];
        let mut func = MirFunction::new(
            FuncId::from(99u32),
            "varargs_wrapper@<add>".to_string(),
            params,
            Type::Any,
            None,
        );
        func.locals.insert(
            LocalId::from(2u32),
            Local {
                id: LocalId::from(2u32),
                name: None,
                ty: Type::Int,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            },
        );
        func.locals.insert(
            LocalId::from(3u32),
            Local {
                id: LocalId::from(3u32),
                name: None,
                ty: Type::Any,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            },
        );
        for p in &func.params.clone() {
            func.locals.insert(p.id, p.clone());
        }

        let block = BasicBlock {
            id: func.entry_block,
            instructions: vec![
                MirInstruction {
                    kind: InstructionKind::UnboxValue {
                        dest: LocalId::from(2u32),
                        src: Operand::Local(LocalId::from(0u32)),
                        dest_type: Type::Int,
                    },
                    span: None,
                },
                MirInstruction {
                    kind: InstructionKind::RuntimeCall {
                        dest: LocalId::from(3u32),
                        func: RuntimeFunc::Call(&RT_CALL_WITH_TUPLE_ARGS),
                        args: vec![
                            Operand::Local(LocalId::from(2u32)),
                            Operand::Local(LocalId::from(1u32)),
                        ],
                    },
                    span: None,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Local(LocalId::from(3u32)))),
        };
        func.blocks.clear();
        func.blocks.insert(block.id, block);
        func
    }

    #[test]
    fn devirts_rt_call_with_tuple_args_to_calldirect() {
        let mut func = make_wrapper_with_indirect_call();
        let captured = FuncId::from(42u32);
        let changed = devirt_wrapper_indirect_calls(&mut func, 0, captured, &[Type::Int]);
        assert!(changed, "devirt should rewrite the call");

        let block = func.blocks.values().next().unwrap();
        let call = &block.instructions[1];
        match &call.kind {
            InstructionKind::CallDirect { func: f, args, .. } => {
                assert_eq!(*f, captured);
                // Wrapper's user param is Local(1); args should have one entry.
                assert_eq!(args.len(), 1);
                match &args[0] {
                    Operand::Local(l) => assert_eq!(*l, LocalId::from(1u32)),
                    _ => panic!("expected local arg"),
                }
            }
            other => panic!("expected CallDirect, got {other:?}"),
        }
    }

    #[test]
    fn skips_unrelated_runtime_calls() {
        let mut func = make_wrapper_with_indirect_call();
        // Replace the RuntimeCall args[0] with an unrelated local so the
        // backward-trace fails — the call must be left alone.
        let block = func.blocks.values_mut().next().unwrap();
        if let InstructionKind::RuntimeCall { args, .. } = &mut block.instructions[1].kind {
            args[0] = Operand::Local(LocalId::from(99u32));
        }

        let changed =
            devirt_wrapper_indirect_calls(&mut func, 0, FuncId::from(42u32), &[Type::Int]);
        assert!(!changed, "unrelated call must not be rewritten");
    }

    #[test]
    fn skips_when_arity_mismatches_and_no_tuple_param() {
        // Wrapper takes 1 user arg (Int, not tuple), captured target wants
        // 2 — devirt must skip: not slot-for-slot, not unpack-eligible.
        let mut func = make_wrapper_with_indirect_call();
        let changed = devirt_wrapper_indirect_calls(
            &mut func,
            0,
            FuncId::from(42u32),
            &[Type::Int, Type::Int],
        );
        assert!(!changed, "non-tuple arity mismatch must not be rewritten");
        let block = func.blocks.values().next().unwrap();
        assert!(matches!(
            block.instructions[1].kind,
            InstructionKind::RuntimeCall { .. }
        ));
    }

    #[test]
    fn devirts_varargs_wrapper_with_unpack_two_int_args() {
        let mut func = make_varargs_wrapper();
        let captured = FuncId::from(42u32);
        let changed =
            devirt_wrapper_indirect_calls(&mut func, 0, captured, &[Type::Int, Type::Int]);
        assert!(changed, "*args devirt should rewrite the call");

        let block = func.blocks.values().next().unwrap();
        // Layout after rewrite:
        //   [0] UnwrapValueInt fn_ptr (untouched)
        //   [1] rt_tuple_get(args, 0) -> tagged0
        //   [2] UnwrapValueInt(tagged0) -> arg0
        //   [3] rt_tuple_get(args, 1) -> tagged1
        //   [4] UnwrapValueInt(tagged1) -> arg1
        //   [5] CallDirect(captured, [arg0, arg1])
        assert_eq!(block.instructions.len(), 6, "expected 6 instructions");

        // tuple_get / unwrap pairs
        match &block.instructions[1].kind {
            InstructionKind::RuntimeCall { func: rf, args, .. } => {
                let symbol = match rf {
                    RuntimeFunc::Call(def) => def.symbol,
                    _ => panic!("not a Call"),
                };
                assert_eq!(symbol, "rt_tuple_get");
                assert!(matches!(
                    args[0],
                    Operand::Local(l) if l == LocalId::from(1u32)
                ));
                assert!(matches!(
                    args[1],
                    Operand::Constant(pyaot_mir::Constant::Int(0))
                ));
            }
            other => panic!("expected rt_tuple_get, got {other:?}"),
        }
        match &block.instructions[2].kind {
            InstructionKind::UnboxValue {
                dest_type: Type::Int,
                ..
            } => {}
            other => panic!("expected UnboxValue(Int), got {other:?}"),
        }
        match &block.instructions[5].kind {
            InstructionKind::CallDirect { func: f, args, .. } => {
                assert_eq!(*f, captured);
                assert_eq!(args.len(), 2);
            }
            other => panic!("expected CallDirect, got {other:?}"),
        }
    }

    #[test]
    fn devirts_varargs_wrapper_zero_args() {
        // Captured target has no params — unpack emits zero rt_tuple_get
        // calls, just an empty CallDirect.
        let mut func = make_varargs_wrapper();
        let captured = FuncId::from(42u32);
        let changed = devirt_wrapper_indirect_calls(&mut func, 0, captured, &[]);
        assert!(changed, "zero-arg *args devirt should still rewrite");

        let block = func.blocks.values().next().unwrap();
        // [0] UnwrapValueInt fn_ptr (untouched), [1] CallDirect(captured, [])
        assert_eq!(block.instructions.len(), 2);
        match &block.instructions[1].kind {
            InstructionKind::CallDirect { args, .. } => assert!(args.is_empty()),
            other => panic!("expected empty-args CallDirect, got {other:?}"),
        }
    }

    #[test]
    fn devirts_varargs_wrapper_with_heap_arg_no_unbox() {
        // Captured wants Str (heap) — devirt emits rt_tuple_get but no
        // unbox; tagged local feeds CallDirect directly.
        let mut func = make_varargs_wrapper();
        // Re-type the Function param's ret to match Str captured below.
        let captured = FuncId::from(42u32);
        let changed = devirt_wrapper_indirect_calls(&mut func, 0, captured, &[Type::Str]);
        assert!(changed, "heap-arg devirt should rewrite");

        let block = func.blocks.values().next().unwrap();
        // [0] UnwrapValueInt fn_ptr, [1] rt_tuple_get, [2] CallDirect
        assert_eq!(block.instructions.len(), 3);
        match &block.instructions[2].kind {
            InstructionKind::CallDirect { args, .. } => assert_eq!(args.len(), 1),
            other => panic!("expected CallDirect, got {other:?}"),
        }
    }
}
