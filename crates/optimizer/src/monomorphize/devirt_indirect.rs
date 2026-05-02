//! S3.3b.2 Stage D: devirt indirect calls inside specialised wrapper bodies.
//!
//! After `specialize_wrapper` clones a decorator wrapper and retypes its
//! fn-pointer parameter to `Type::Function { … }`, the runtime trampoline
//! calls (`rt_call_with_tuple_args` / `rt_call_with_captures_and_args`) that
//! still carry the indirect dispatch can be rewritten to `CallDirect` —
//! the captured target is statically known.
//!
//! Rewrite criterion: a `RuntimeCall` whose first argument transitively
//! traces back (through `UnwrapValueInt` and `Copy { src: Local }`) to the
//! wrapper's fn-pointer parameter. Other indirect calls (dynamic
//! dispatch, captures from outer scope) are left untouched — the safe
//! fallback is the existing runtime trampoline.
//!
//! The rewrite preserves the call's `dest` local so downstream Phi/Copy
//! plumbing keeps working; WPA pass 2 then retypes the dest to the
//! captured function's return type and DCE/constfold prune any
//! unreachable closure-path branches.

use pyaot_mir::{Function, InstructionKind, Operand};
use pyaot_types::Type;
use pyaot_utils::{FuncId, LocalId};

/// Devirtify all `rt_call_with_tuple_args` / `rt_call_with_captures_and_args`
/// invocations inside a specialised wrapper, where the fn-pointer argument
/// traces back to the wrapper's fn-pointer parameter.
///
/// `fn_ptr_param_idx` is the index of the wrapper's fn-pointer parameter
/// (always 0 in the current ABI). `captured_func_id` is the static target.
/// `captured_param_types` are the parameter types of the captured function.
/// Devirt proceeds only when the wrapper's user-visible parameters are a
/// type-by-type match for the captured target's parameters — that means
/// the wrapper takes the same scalars/objects the target expects, with no
/// `*args` packing or other ABI mediation in between. A type mismatch
/// (e.g. wrapper takes `tuple[Any]` and captured wants `int`) means the
/// runtime trampoline is doing real work that we can't elide here.
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
    let user_params: Vec<&pyaot_mir::Local> = func
        .params
        .iter()
        .enumerate()
        .filter_map(|(i, p)| (i != fn_ptr_param_idx).then_some(p))
        .collect();

    // Skip devirt unless the wrapper's user-visible signature matches the
    // captured target slot-for-slot. `*args`-style wrappers fail this
    // because the wrapper has a single `tuple[...]` slot where the captured
    // target has N individual parameters; the runtime trampoline is doing
    // real splatting work and we can't elide it without emitting unpack.
    if user_params.len() != captured_param_types.len() {
        return false;
    }
    for (wp, cp) in user_params.iter().zip(captured_param_types.iter()) {
        if &wp.ty != cp {
            return false;
        }
    }
    let user_arg_locals: Vec<LocalId> = user_params.iter().map(|p| p.id).collect();

    let mut changed = false;
    // Function-wide alias set: lowering emits the prologue
    // `UnwrapValueInt(fn_ptr_param)` in the entry block, but `rt_call_with_*`
    // sites can live in any successor block. Walk all blocks once to gather
    // the closure of fn-pointer aliases, then rewrite call sites in all
    // blocks. (We make multiple passes because aliases can chain across
    // blocks via Copy, but in practice one fixpoint converges quickly since
    // chain length is short.)
    let mut fnptr_aliases: Vec<LocalId> = vec![fn_ptr_param_local];
    loop {
        let before = fnptr_aliases.len();
        for block in func.blocks.values() {
            for instr in &block.instructions {
                match &instr.kind {
                    InstructionKind::UnwrapValueInt {
                        dest,
                        src: Operand::Local(s),
                    } if fnptr_aliases.contains(s) && !fnptr_aliases.contains(dest) => {
                        fnptr_aliases.push(*dest);
                    }
                    InstructionKind::Copy {
                        dest,
                        src: Operand::Local(s),
                    } if fnptr_aliases.contains(s) && !fnptr_aliases.contains(dest) => {
                        fnptr_aliases.push(*dest);
                    }
                    _ => {}
                }
            }
        }
        if fnptr_aliases.len() == before {
            break;
        }
    }
    for block in func.blocks.values_mut() {
        // Second pass: rewrite RuntimeCall sites whose first arg is one of
        // the fn-pointer aliases.
        for instr in block.instructions.iter_mut() {
            let dest_local = match &instr.kind {
                InstructionKind::RuntimeCall { dest, func, args } => {
                    let symbol = match func {
                        pyaot_mir::RuntimeFunc::Call(def) => def.symbol,
                        _ => continue,
                    };
                    if symbol != "rt_call_with_tuple_args"
                        && symbol != "rt_call_with_captures_and_args"
                    {
                        continue;
                    }
                    let first_arg_local = match args.first() {
                        Some(Operand::Local(id)) => *id,
                        _ => continue,
                    };
                    if !fnptr_aliases.contains(&first_arg_local) {
                        continue;
                    }
                    *dest
                }
                _ => continue,
            };

            // Build the CallDirect args from wrapper's user-visible params.
            let new_args: Vec<Operand> = user_arg_locals
                .iter()
                .copied()
                .map(Operand::Local)
                .collect();
            instr.kind = InstructionKind::CallDirect {
                dest: dest_local,
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
            },
            Local {
                id: LocalId::from(1u32),
                name: None,
                ty: Type::Int,
                is_gc_root: false,
            },
        ];
        let raw_fn = Local {
            id: LocalId::from(2u32),
            name: None,
            ty: Type::Int,
            is_gc_root: false,
        };
        let args_tuple = Local {
            id: LocalId::from(3u32),
            name: None,
            ty: Type::Any,
            is_gc_root: true,
        };
        let result = Local {
            id: LocalId::from(4u32),
            name: None,
            ty: Type::Any,
            is_gc_root: false,
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
                    kind: InstructionKind::UnwrapValueInt {
                        dest: LocalId::from(2u32),
                        src: Operand::Local(LocalId::from(0u32)),
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
    fn skips_when_arity_mismatches() {
        // Wrapper takes 1 user arg, captured target wants 2 — devirt must
        // skip and leave the runtime trampoline in place (fallback for
        // `*args` wrappers whose user-visible param is a packed tuple).
        let mut func = make_wrapper_with_indirect_call();
        let changed = devirt_wrapper_indirect_calls(
            &mut func,
            0,
            FuncId::from(42u32),
            &[Type::Int, Type::Int],
        );
        assert!(!changed, "arity mismatch must not be rewritten");
        let block = func.blocks.values().next().unwrap();
        assert!(matches!(
            block.instructions[1].kind,
            InstructionKind::RuntimeCall { .. }
        ));
    }
}
