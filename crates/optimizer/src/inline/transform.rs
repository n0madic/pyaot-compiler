//! Core inlining transformation
//!
//! Transforms CallDirect instructions into inlined code by:
//! 1. Copying arguments to parameter locals
//! 2. Inserting remapped callee blocks
//! 3. Replacing return points with jumps to continuation

use indexmap::IndexMap;
use pyaot_mir::{BasicBlock, Function, Instruction, InstructionKind, Local, Module, Terminator};
use pyaot_utils::{BlockId, FuncId};

use super::analysis::{CallGraph, FunctionCost, InlineDecision};
use super::remap::InlineRemapper;
use super::InlineConfig;

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

    let (dest, args) = match &call_block.instructions.get(call_instr_idx) {
        Some(Instruction {
            kind: InstructionKind::CallDirect { dest, args, .. },
            ..
        }) => (*dest, args.clone()),
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
            is_gc_root: param.is_gc_root,
        };
        caller.locals.insert(new_param_id, new_local);

        // Create copy instruction: new_param = arg (synthetic, no span)
        param_copies.push(Instruction {
            kind: InstructionKind::Copy {
                dest: new_param_id,
                src: arg.clone(),
            },
            span: None,
        });
    }

    // Add callee's non-parameter locals to caller (remapped)
    for local in callee.locals.values() {
        let new_id = remapper.remap_local(local.id);
        let new_local = Local {
            id: new_id,
            name: local.name,
            ty: local.ty.clone(),
            is_gc_root: local.is_gc_root,
        };
        caller.locals.insert(new_id, new_local);
    }

    // Remap callee blocks
    let inlined_entry = remapper.remap_block(callee.entry_block);
    let mut inlined_blocks: IndexMap<BlockId, BasicBlock> = IndexMap::new();

    for block in callee.blocks.values() {
        let mut remapped_block = remapper.remap_block_contents(block);

        // Replace Return terminators with Copy + Goto to continuation
        if let Terminator::Return(ret_val) = &remapped_block.terminator {
            if let Some(val) = ret_val {
                // Copy return value to dest (synthetic, no span)
                remapped_block.instructions.push(Instruction {
                    kind: InstructionKind::Copy {
                        dest,
                        src: val.clone(),
                    },
                    span: None,
                });
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

    // Create continuation block with instructions after + original terminator
    let continuation_block = BasicBlock {
        id: continuation_block_id,
        instructions: instructions_after,
        terminator: original_terminator,
    };

    // Add all new blocks to caller
    caller
        .blocks
        .insert(continuation_block_id, continuation_block);
    for (id, block) in inlined_blocks {
        caller.blocks.insert(id, block);
    }

    true
}
