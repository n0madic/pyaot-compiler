//! MIR function specialization for monomorphization.
//!
//! `specialize_function` clones a generic template `Function`, substitutes all
//! `Type::Var` leaves with concrete types from `subst`, and returns a fresh
//! function with a new `FuncId` and `is_generic_template = false`.

use std::collections::HashMap;

use indexmap::IndexMap;
use pyaot_mir::{BasicBlock, Function, Instruction, InstructionKind, Local};
use pyaot_types::Type;
use pyaot_utils::{BlockId, FuncId, InternedString, LocalId};

use crate::inline::remap::InlineRemapper;

/// Clone `template`, substituting every `Type::Var` occurrence with the
/// mapping in `subst`, and assign the clone `fresh_id` / `fresh_name`.
///
/// The clone is a fully independent function: it has its own `LocalId`/`BlockId`
/// space remapped from zero, `is_generic_template = false`, and empty
/// `typevar_params`.
pub fn specialize_function(
    template: &Function,
    subst: &HashMap<InternedString, Type>,
    fresh_id: FuncId,
    fresh_name: String,
) -> Function {
    // Determine starting IDs — remapper allocates from 0 for fresh standalone fn.
    let mut remapper = InlineRemapper::new(0, 0);

    // Clone and remap params, substituting types.
    let params: Vec<Local> = template
        .params
        .iter()
        .map(|p| {
            let new_id = remapper.remap_local(p.id);
            let new_ty = p.ty.substitute(subst);
            Local {
                id: new_id,
                name: p.name,
                ty: new_ty.clone(),
                is_gc_root: new_ty.is_heap(),
            }
        })
        .collect();

    // Clone and remap locals, substituting types.
    let mut locals: IndexMap<LocalId, Local> = IndexMap::new();
    for (_, local) in &template.locals {
        let new_id = remapper.remap_local(local.id);
        let new_ty = local.ty.substitute(subst);
        locals.insert(
            new_id,
            Local {
                id: new_id,
                name: local.name,
                ty: new_ty.clone(),
                is_gc_root: new_ty.is_heap(),
            },
        );
    }

    // Clone and remap all blocks.
    // First pass: register all block IDs so phi sources resolve correctly.
    for (block_id, _) in &template.blocks {
        remapper.remap_block(*block_id);
    }
    let mut blocks: IndexMap<BlockId, BasicBlock> = IndexMap::new();
    for (_, block) in &template.blocks {
        let new_block = remap_block_with_type_subst(&mut remapper, block, subst);
        blocks.insert(new_block.id, new_block);
    }

    let entry_block = remapper.remap_block(template.entry_block);
    let return_type = template.return_type.substitute(subst);

    Function {
        id: fresh_id,
        name: fresh_name,
        params,
        return_type,
        locals,
        blocks,
        entry_block,
        span: template.span,
        is_ssa: template.is_ssa,
        is_generic_template: false,
        typevar_params: Vec::new(),
        dom_tree_cache: std::cell::OnceCell::new(),
    }
}

/// Remap a block with both ID remapping and type substitution.
fn remap_block_with_type_subst(
    remapper: &mut InlineRemapper,
    block: &BasicBlock,
    subst: &HashMap<InternedString, Type>,
) -> BasicBlock {
    let new_id = remapper.remap_block(block.id);
    let instructions = block
        .instructions
        .iter()
        .map(|i| remap_instr_with_type_subst(remapper, i, subst))
        .collect();
    let terminator = remapper.remap_terminator(&block.terminator);
    BasicBlock {
        id: new_id,
        instructions,
        terminator,
    }
}

/// Remap an instruction, additionally substituting types on `Refine` and `GcAlloc`.
fn remap_instr_with_type_subst(
    remapper: &mut InlineRemapper,
    instr: &Instruction,
    subst: &HashMap<InternedString, Type>,
) -> Instruction {
    let kind = match &instr.kind {
        // These two carry embedded Type values that must be substituted.
        InstructionKind::Refine { dest, src, ty } => InstructionKind::Refine {
            dest: remapper.remap_local(*dest),
            src: remapper.remap_operand(src),
            ty: ty.substitute(subst),
        },
        InstructionKind::GcAlloc { dest, ty, size } => InstructionKind::GcAlloc {
            dest: remapper.remap_local(*dest),
            ty: ty.substitute(subst),
            size: *size,
        },
        // All other instructions: delegate to the existing remapper (type-blind).
        other => {
            return remapper.remap_instruction(&Instruction {
                kind: other.clone(),
                span: instr.span,
            })
        }
    };
    Instruction {
        kind,
        span: instr.span,
    }
}
