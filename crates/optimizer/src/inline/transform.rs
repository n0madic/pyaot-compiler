//! Core inlining transformation
//!
//! Transforms CallDirect instructions into inlined code by:
//! 1. Copying arguments to parameter locals
//! 2. Inserting remapped callee blocks
//! 3. Replacing return points with jumps to continuation

use indexmap::IndexMap;
use pyaot_mir::{BasicBlock, Function, Instruction, InstructionKind, Local, Module, Terminator};
use pyaot_utils::{BlockId, FuncId};

use super::analysis::{FunctionCost, InlineDecision};
use super::remap::InlineRemapper;
use super::InlineConfig;
use crate::call_graph::CallGraph;

/// Perform inlining pass on the module. Returns `true` if any inlining was performed.
pub fn inline_pass(module: &mut Module, config: &InlineConfig) -> bool {
    let mut ever_inlined = false;

    // Perform multiple iterations to handle transitive inlining
    // Recompute call graph and costs each iteration since inlining changes function sizes
    for _iteration in 0..config.max_iterations {
        let call_graph = CallGraph::build(module);
        let costs: IndexMap<FuncId, FunctionCost> = module
            .functions
            .keys()
            .map(|&id| {
                let cost = FunctionCost::compute(&module.functions[&id], &call_graph);
                (id, cost)
            })
            .collect();

        let inlinable: IndexMap<FuncId, InlineDecision> = costs
            .iter()
            .map(|(&id, cost)| (id, cost.should_inline(config)))
            .collect();

        let mut any_inlined = false;

        // Collect function IDs to process (avoid borrowing issues)
        let func_ids: Vec<FuncId> = module.functions.keys().copied().collect();

        for func_id in func_ids {
            if inline_calls_in_function(module, func_id, &inlinable, config) {
                any_inlined = true;
            }
        }

        if !any_inlined {
            break;
        }
        ever_inlined = true;
    }

    ever_inlined
}

/// Maximum total instruction count for a caller before we stop inlining into it.
/// Prevents unbounded growth from many small inlined functions.
const MAX_CALLER_INSTRUCTIONS: usize = 500;

/// Inline eligible calls within a single function
/// Returns true if any inlining was performed
fn inline_calls_in_function(
    module: &mut Module,
    caller_id: FuncId,
    inlinable: &IndexMap<FuncId, InlineDecision>,
    config: &InlineConfig,
) -> bool {
    let mut any_inlined = false;

    // We need to iterate over blocks, but inlining modifies the block structure
    // So we process one call at a time and restart
    loop {
        // Stop inlining into this function if it has grown too large
        if let Some(caller) = module.functions.get(&caller_id) {
            let total_instrs: usize = caller.blocks.values().map(|b| b.instructions.len()).sum();
            if total_instrs > MAX_CALLER_INSTRUCTIONS.max(config.max_inline_size * 10) {
                break;
            }
        }

        // Find next inline candidate
        let candidate = find_next_inline_candidate(module, caller_id, inlinable);

        match candidate {
            Some((block_id, instr_idx, callee_id)) => {
                // Perform the inline
                if perform_inline(module, caller_id, block_id, instr_idx, callee_id) {
                    any_inlined = true;
                }
            }
            None => break,
        }
    }

    any_inlined
}

/// Find the next CallDirect that can be inlined
fn find_next_inline_candidate(
    module: &Module,
    caller_id: FuncId,
    inlinable: &IndexMap<FuncId, InlineDecision>,
) -> Option<(BlockId, usize, FuncId)> {
    let caller = &module.functions[&caller_id];

    for (block_id, block) in &caller.blocks {
        for (idx, instr) in block.instructions.iter().enumerate() {
            if let InstructionKind::CallDirect {
                func: callee_id, ..
            } = &instr.kind
            {
                // Don't inline self-recursion
                if *callee_id == caller_id {
                    continue;
                }

                // Check if callee is inlinable
                if let Some(decision) = inlinable.get(callee_id) {
                    if matches!(decision, InlineDecision::Always | InlineDecision::Consider) {
                        return Some((*block_id, idx, *callee_id));
                    }
                }
            }
        }
    }

    None
}

/// Perform inlining of a single call site
fn perform_inline(
    module: &mut Module,
    caller_id: FuncId,
    call_block_id: BlockId,
    call_instr_idx: usize,
    callee_id: FuncId,
) -> bool {
    // Clone callee function to avoid borrow issues
    let callee: Function = match module.functions.get(&callee_id).cloned() {
        Some(f) => f,
        None => return false,
    };

    // Now we can safely get mutable access to caller
    let caller = match module.functions.get_mut(&caller_id) {
        Some(f) => f,
        None => return false,
    };

    // Extract call instruction info
    let call_block = match caller.blocks.get(&call_block_id) {
        Some(b) => b,
        None => return false,
    };

    let (dest, args, call_span) = match &call_block.instructions.get(call_instr_idx) {
        Some(Instruction {
            kind: InstructionKind::CallDirect { dest, args, .. },
            span,
        }) => (*dest, args.clone(), *span),
        _ => return false,
    };

    // Compute next available IDs
    let next_local = caller
        .locals
        .keys()
        .map(|id| id.0 + 1)
        .max()
        .unwrap_or(0)
        .max(caller.params.iter().map(|p| p.id.0 + 1).max().unwrap_or(0));
    let next_block = caller.blocks.keys().map(|id| id.0 + 1).max().unwrap_or(0);

    // Create remapper
    let mut remapper = InlineRemapper::new(next_local, next_block);

    // Allocate a fresh block ID for the continuation block (after inlined code)
    let continuation_block_id = remapper.allocate_block_id();

    // Remap callee's parameter locals to new locals in caller
    // and create Copy instructions to pass arguments
    assert_eq!(
        callee.params.len(),
        args.len(),
        "inline: parameter/argument count mismatch for function"
    );
    let mut param_copies: Vec<Instruction> = Vec::new();
    for (param, arg) in callee.params.iter().zip(args.iter()) {
        let new_param_id = remapper.remap_local(param.id);

        // Add local to caller
        let new_local = Local {
            id: new_param_id,
            name: param.name,
            ty: param.ty.clone(),
            abi_immutable: false,
            is_var_local: false,
            // Phase 3e: preserve mir_ty from the inlined callee param so
            // body locals keep their precise MirType signature instead of
            // falling back to register-level translation of `ty`.
            mir_ty: param.mir_ty.clone(),
        };
        caller.locals.insert(new_param_id, new_local);

        // Create copy instruction: new_param = arg (attributed to call site)
        param_copies.push(Instruction {
            kind: InstructionKind::Copy {
                dest: new_param_id,
                src: arg.clone(),
            },
            span: call_span,
        });
    }

    // Add callee's non-parameter locals to caller (remapped)
    for local in callee.locals.values() {
        let new_id = remapper.remap_local(local.id);
        let new_local = Local {
            id: new_id,
            name: local.name,
            ty: local.ty.clone(),
            abi_immutable: false,
            is_var_local: false,
            // Phase 3e: preserve mir_ty from the inlined callee local so
            // body operations keep their precise MirType signature.
            mir_ty: local.mir_ty.clone(),
        };
        caller.locals.insert(new_id, new_local);
    }

    // Remap callee blocks. Replace every `Return` terminator with
    // `Goto(continuation)` and collect the return-value operand (or
    // record a void return) per block — instead of emitting a
    // multi-def `Copy(dest, val)` in each returning block, which would
    // break SSA now that the pipeline runs `construct_ssa` before the
    // optimizer (§1.14b-prep).
    let inlined_entry = remapper.remap_block(callee.entry_block);
    let mut inlined_blocks: IndexMap<BlockId, BasicBlock> = IndexMap::new();
    let mut return_sources: Vec<(BlockId, pyaot_mir::Operand)> = Vec::new();
    let mut void_return_blocks: Vec<BlockId> = Vec::new();

    for block in callee.blocks.values() {
        let mut remapped_block = remapper.remap_block_contents(block);

        if let Terminator::Return(ret_val) = &remapped_block.terminator {
            match ret_val {
                Some(val) => return_sources.push((remapped_block.id, val.clone())),
                None => void_return_blocks.push(remapped_block.id),
            }
            remapped_block.terminator = Terminator::Goto(continuation_block_id);
        }

        inlined_blocks.insert(remapped_block.id, remapped_block);
    }

    // Split the call block:
    // - Before call: instructions[0..call_instr_idx] + param_copies + Goto(inlined_entry)
    // - After call (continuation): instructions[call_instr_idx+1..] + original terminator

    let original_block = caller.blocks.get(&call_block_id).expect("block exists");
    let instructions_before: Vec<Instruction> =
        original_block.instructions[..call_instr_idx].to_vec();
    let instructions_after: Vec<Instruction> =
        original_block.instructions[call_instr_idx + 1..].to_vec();
    let original_terminator = original_block.terminator.clone();

    // Update call block: instructions before + param copies + goto inlined entry
    let call_block = caller.blocks.get_mut(&call_block_id).expect("block exists");
    call_block.instructions = instructions_before;
    call_block.instructions.extend(param_copies);
    call_block.terminator = Terminator::Goto(inlined_entry);

    // Build the continuation block. If the callee has at least one
    // value-returning path, emit a Phi at the head merging the return
    // values from each predecessor. A void/bare `return` semantically yields
    // None, so for a mixed value/void callee whose merge dest is a tagged slot
    // (Optional/Union/Any) the void paths contribute `Constant::None` — the
    // old `Constant::Int(0)` placeholder made the None-returning path produce
    // 0, which a caller checking `is None` would observe. When dest is a
    // concrete raw type (genuinely-void mix where the value is never read), a
    // None operand would mismatch the verifier, so keep the Int(0) placeholder.
    let mut continuation_instructions: Vec<Instruction> = Vec::new();
    if !return_sources.is_empty() {
        let dest_is_tagged = caller.locals.get(&dest).is_some_and(|l| {
            matches!(
                l.ty,
                pyaot_types::Type::Any | pyaot_types::Type::None | pyaot_types::Type::Union(_)
            ) || matches!(l.resolved_mir_type(), pyaot_mir::MirType::Tagged)
        });
        // A bare `return` lowers to `Return(Some(Constant::None))`, so it lands
        // in `return_sources` (not `void_return_blocks`). The merge therefore
        // mixes a value (e.g. raw `Int`) with `None`. The result must be a
        // tagged Value whenever it can be None: a raw slot stores `None` as
        // `i8 0` (reads back as `False`/`0`), and an earlier WPA pass may have
        // narrowed the dest to a raw primitive. Also force-tagged when the
        // dest is already a tagged slot or there are genuinely-void paths.
        let any_none_source = return_sources
            .iter()
            .any(|(_, op)| matches!(op, pyaot_mir::Operand::Constant(pyaot_mir::Constant::None)));
        let result_is_tagged = dest_is_tagged || !void_return_blocks.is_empty() || any_none_source;

        // Combined Phi sources: value returns plus void returns (→ None).
        let mut phi_sources: Vec<(BlockId, pyaot_mir::Operand)> = return_sources.clone();
        let void_placeholder = if result_is_tagged {
            pyaot_mir::Operand::Constant(pyaot_mir::Constant::None)
        } else {
            pyaot_mir::Operand::Constant(pyaot_mir::Constant::Int(0))
        };
        for &vb in &void_return_blocks {
            phi_sources.push((vb, void_placeholder.clone()));
        }

        if result_is_tagged {
            // Retype the merge dest to a tagged slot so it can hold None.
            // GC-rooting is computed from `mir_ty`, so `Some(Tagged)` suffices.
            if !dest_is_tagged {
                if let Some(l) = caller.locals.get_mut(&dest) {
                    l.ty = pyaot_types::Type::Any;
                    l.mir_ty = Some(pyaot_mir::MirType::Tagged);
                }
            }
            // Box every raw-primitive Phi source into a tagged Value in its
            // source block — a `Raw → Tagged` Phi edge is rejected by the
            // verifier, and `None`/`Int(0)`/`False` raw bits must become a
            // real tagged `Value` so downstream `is None` / print observe it.
            let mut next_local = caller.locals.keys().map(|k| k.0).max().unwrap_or(0) + 1;
            for (block_id, op) in phi_sources.iter_mut() {
                let Some(src_type) = inline_raw_src_type(op, caller) else {
                    continue;
                };
                let fresh = pyaot_utils::LocalId(next_local);
                next_local += 1;
                caller.locals.insert(
                    fresh,
                    Local {
                        id: fresh,
                        name: None,
                        ty: pyaot_types::Type::Any,
                        abi_immutable: false,
                        is_var_local: false,
                        mir_ty: Some(pyaot_mir::MirType::Tagged),
                    },
                );
                if let Some(src_block) = inlined_blocks.get_mut(block_id) {
                    src_block.instructions.push(Instruction {
                        kind: InstructionKind::BoxValue {
                            dest: fresh,
                            src: op.clone(),
                            src_type,
                        },
                        span: call_span,
                    });
                }
                *op = pyaot_mir::Operand::Local(fresh);
            }
        }

        continuation_instructions.push(Instruction {
            kind: InstructionKind::Phi {
                dest,
                sources: phi_sources,
            },
            span: call_span,
        });
    }
    continuation_instructions.extend(instructions_after);

    let continuation_block = BasicBlock {
        id: continuation_block_id,
        instructions: continuation_instructions,
        terminator: original_terminator,
    };

    // Add all new blocks to caller
    caller
        .blocks
        .insert(continuation_block_id, continuation_block);
    for (id, block) in inlined_blocks {
        caller.blocks.insert(id, block);
    }

    // Rewrite Phi sources in any block reachable from `continuation_block`'s
    // (former call_block's) terminator: their predecessor was `call_block_id`
    // before the split, but after inlining the same edge originates from
    // `continuation_block_id`. Without this fix, codegen later panics with
    // "phi has no source for predecessor block — arity violation".
    let successor_targets =
        terminator_successors(&caller.blocks[&continuation_block_id].terminator);
    for target_id in successor_targets {
        if let Some(target_block) = caller.blocks.get_mut(&target_id) {
            for inst in &mut target_block.instructions {
                let InstructionKind::Phi { sources, .. } = &mut inst.kind else {
                    break;
                };
                for (pred, _) in sources.iter_mut() {
                    if *pred == call_block_id {
                        *pred = continuation_block_id;
                    }
                }
            }
        }
    }

    caller.invalidate_dom_tree();
    true
}

/// If `op` is a raw-primitive Phi source that must be boxed before feeding a
/// tagged merge dest, return the `src_type` to use for the `BoxValue`. Returns
/// `None` for operands that are already tagged Values / heap pointers (which
/// widen to `Tagged` implicitly).
fn inline_raw_src_type(op: &pyaot_mir::Operand, func: &Function) -> Option<pyaot_types::Type> {
    use pyaot_mir::{Constant, MirType, Operand, RawKind};
    use pyaot_types::Type;
    match op {
        Operand::Constant(Constant::Int(_)) => Some(Type::Int),
        Operand::Constant(Constant::Bool(_)) => Some(Type::Bool),
        Operand::Constant(Constant::Float(_)) => Some(Type::Float),
        Operand::Constant(Constant::None) => Some(Type::None),
        // Str/Bytes constants are already heap-shaped Values.
        Operand::Constant(_) => None,
        Operand::Local(id) => {
            let l = func.locals.get(id)?;
            match l.resolved_mir_type() {
                MirType::Raw(RawKind::F64) => Some(Type::Float),
                MirType::Raw(_) => match &l.ty {
                    Type::Int | Type::Bool | Type::Float | Type::None => Some(l.ty.clone()),
                    // Raw mir_ty with a non-primitive declared type (an `Any`
                    // local narrowed to raw bits): map by raw kind (I64/I32/I8
                    // → Int is the only safe integer reading).
                    _ => Some(Type::Int),
                },
                // Tagged / Heap / FuncPtr / Closure — already a tagged Value.
                _ => None,
            }
        }
    }
}

/// Collect every successor block id a terminator can jump to.
fn terminator_successors(term: &Terminator) -> Vec<BlockId> {
    match term {
        Terminator::Goto(b) => vec![*b],
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        Terminator::TrySetjmp {
            try_body,
            handler_entry,
            ..
        } => vec![*try_body, *handler_entry],
        // Diverging / no-successor terminators
        Terminator::Return(_)
        | Terminator::Unreachable
        | Terminator::Raise { .. }
        | Terminator::RaiseCustom { .. }
        | Terminator::Reraise => Vec::new(),
        // Anything else (try-end, etc.) — be conservative and walk it later if
        // we hit a panic; for now those don't appear in inlinable callers.
        _ => Vec::new(),
    }
}
