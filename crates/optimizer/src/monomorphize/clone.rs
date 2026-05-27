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
                // Preserve ABI immutability from template: monomorphization
                // must not relax the prologue-unbox contract.
                abi_immutable: p.abi_immutable,
                // Phase 3e: derive mir_ty from substituted type at register
                // level so monomorphized params have precise MirType.
                is_var_local: false,
                mir_ty: Some(pyaot_mir::type_to_mir_type_register(&new_ty)),
            }
        })
        .collect();

    // Clone and remap locals, substituting types.
    let mut locals: IndexMap<LocalId, Local> = IndexMap::new();
    for (_, local) in &template.locals {
        let new_id = remapper.remap_local(local.id);
        let new_ty = local.ty.substitute(subst);
        // Phase 3e: derive mir_ty from substituted ty at register level.
        // Was `mir_ty: None` which forced fallback translation.
        let new_mir_ty = pyaot_mir::type_to_mir_type_register(&new_ty);
        locals.insert(
            new_id,
            Local {
                id: new_id,
                name: local.name,
                ty: new_ty.clone(),
                abi_immutable: local.abi_immutable,
                is_var_local: false,
                mir_ty: Some(new_mir_ty),
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
        // Inherit kind from template: a specialised generic still belongs to
        // the same conceptual category (lambda / class method / etc.).
        kind: template.kind,
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
        // Specialised generic templates lose their wrapper status too;
        // wrapper-mode specialisations override this in `specialize_wrapper`.
        wrapper_fn_ptr_capture_index: None,
        phase4_return_abi_flipped: false,
        phase4_original_return_type: None,
        dom_tree_cache: std::cell::OnceCell::new(),
        signature: None,
    }
}

/// Clone a decorator-wrapper template into a per-captured-fn specialisation
/// (S3.3b.2). Unlike `specialize_function` (Var-substitution), this mode:
/// - retypes the fn-pointer parameter at `fn_ptr_param_idx` from `HeapAny` /
///   `Int` (whatever the wrapper used) to `Type::Function { params, ret }` of
///   the captured function;
/// - clears `is_gc_root` on that param (a code pointer is not a heap object);
/// - keeps the body unchanged at this stage — the indirect-call devirt pass
///   (Stage D) rewrites the runtime trampoline to `CallDirect{captured_id}`
///   in a separate step;
/// - clears `is_generic_template` and `wrapper_fn_ptr_capture_index` (the
///   specialisation is no longer a template).
///
/// Caller is responsible for picking `fresh_id`, `fresh_name`, and ensuring
/// the spec cache is updated.
pub fn specialize_wrapper(
    template: &Function,
    fn_ptr_param_idx: usize,
    _captured_func_id: FuncId,
    captured_signature: Type,
    fresh_id: FuncId,
    fresh_name: String,
) -> Function {
    // Wrapper mode uses an empty Var-substitution; types are not Var-bearing
    // in the wrapper signature. We still flow the existing remapper so that
    // ID space is freshened consistently.
    let empty_subst: HashMap<InternedString, Type> = HashMap::new();
    let mut remapper = InlineRemapper::new(0, 0);

    // Params: clone, then retype the fn-ptr slot.
    let params: Vec<Local> = template
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let new_id = remapper.remap_local(p.id);
            if i == fn_ptr_param_idx {
                Local {
                    id: new_id,
                    name: p.name,
                    ty: captured_signature.clone(),
                    abi_immutable: false,
                    // Wrapper-specialization mir_ty deliberately None: the
                    // captured_signature is a Function type whose precise
                    // FuncPtr MirType is computed by Phase 3e's verifier
                    // separately. Setting it explicitly here broke
                    // decorator_factory_optimized.
                    is_var_local: false,
                    mir_ty: None,
                }
            } else {
                Local {
                    id: new_id,
                    name: p.name,
                    ty: p.ty.clone(),
                    abi_immutable: false,
                    is_var_local: false,
                    mir_ty: None,
                }
            }
        })
        .collect();

    // Locals: clone unchanged. The fn-ptr param's local also needs retyping
    // because params are added back as locals after Function::new in the
    // builder; here we mirror that into the locals map.
    let mut locals: IndexMap<LocalId, Local> = IndexMap::new();
    for (_, local) in &template.locals {
        let new_id = remapper.remap_local(local.id);
        // If this local corresponds to the fn-ptr param, retype it to the
        // captured signature; GC-root status is derived from the resulting
        // mir_ty via computed_is_gc_root().
        let is_fn_ptr_param_local = template
            .params
            .get(fn_ptr_param_idx)
            .is_some_and(|p| p.id == local.id);
        let ty = if is_fn_ptr_param_local {
            captured_signature.clone()
        } else {
            local.ty.clone()
        };
        locals.insert(
            new_id,
            Local {
                id: new_id,
                name: local.name,
                ty,
                abi_immutable: local.abi_immutable,
                // Wrapper-specialization: keep mir_ty None to preserve
                // decorator_factory_optimized behavior.
                is_var_local: false,
                mir_ty: None,
            },
        );
    }

    // Blocks: clone with ID remap (no type subst needed here).
    for (block_id, _) in &template.blocks {
        remapper.remap_block(*block_id);
    }
    let mut blocks: IndexMap<BlockId, BasicBlock> = IndexMap::new();
    for (_, block) in &template.blocks {
        let new_block = remap_block_with_type_subst(&mut remapper, block, &empty_subst);
        blocks.insert(new_block.id, new_block);
    }

    let entry_block = remapper.remap_block(template.entry_block);
    // Return type stays as-is from the template at this stage; WPA pass 2
    // (post-mono) refines it once the indirect-call devirt has narrowed
    // dependent locals.
    let return_type = template.return_type.clone();

    Function {
        id: fresh_id,
        // Inherit kind from template: a wrapper specialisation is still a
        // lambda / regular / whatever the template was.
        kind: template.kind,
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
        wrapper_fn_ptr_capture_index: None,
        phase4_return_abi_flipped: false,
        phase4_original_return_type: None,
        dom_tree_cache: std::cell::OnceCell::new(),
        signature: None,
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
