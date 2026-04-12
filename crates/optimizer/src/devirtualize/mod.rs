//! Devirtualization pass
//!
//! Replaces `CallVirtual` with `CallDirect` when the receiver's concrete type
//! is statically known. This eliminates vtable pointer loads and indirect calls,
//! and enables downstream inlining of the resolved method.

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use pyaot_mir::{InstructionKind, Module, Operand};
use pyaot_types::Type;
use pyaot_utils::{ClassId, FuncId, StringInterner};

use crate::pass::OptimizationPass;

/// Build a lookup table from (class_id, vtable_slot) → concrete FuncId.
fn build_vtable_map(module: &Module) -> HashMap<(ClassId, usize), FuncId> {
    let mut map = HashMap::new();
    for vtable_info in &module.vtables {
        for entry in &vtable_info.entries {
            map.insert((vtable_info.class_id, entry.slot), entry.method_func_id);
        }
    }
    map
}

/// Get the ClassId from an operand's type within a function, if statically known.
fn operand_class_id(operand: &Operand, func: &pyaot_mir::Function) -> Option<ClassId> {
    if let Operand::Local(id) = operand {
        let ty = func
            .locals
            .get(id)
            .map(|l| &l.ty)
            .or_else(|| func.params.iter().find(|p| p.id == *id).map(|p| &p.ty))?;
        if let Type::Class { class_id, .. } = ty {
            return Some(*class_id);
        }
    }
    None
}

/// Run devirtualization on the entire module.
///
/// For each `CallVirtual { dest, obj, slot, args }` where the receiver type is
/// a concrete `Type::Class`, resolves the vtable slot to a `FuncId` and replaces
/// the instruction with `CallDirect { dest, func, args: [obj] ++ args }`.
pub fn devirtualize(module: &mut Module) -> bool {
    let vtable_map = build_vtable_map(module);
    if vtable_map.is_empty() {
        return false;
    }

    let func_ids: Vec<FuncId> = module.functions.keys().copied().collect();
    let mut changed = false;

    for func_id in func_ids {
        let func = &module.functions[&func_id];

        // Phase 1: collect replacements (read-only pass to satisfy borrow checker)
        let mut replacements: Vec<(pyaot_utils::BlockId, usize, FuncId, Operand)> = Vec::new();

        for (block_id, block) in &func.blocks {
            for (idx, inst) in block.instructions.iter().enumerate() {
                if let InstructionKind::CallVirtual { obj, slot, .. } = &inst.kind {
                    if let Some(class_id) = operand_class_id(obj, func) {
                        if let Some(&method_func_id) = vtable_map.get(&(class_id, *slot)) {
                            replacements.push((*block_id, idx, method_func_id, obj.clone()));
                        }
                    }
                }
            }
        }

        if replacements.is_empty() {
            continue;
        }

        changed = true;

        // Phase 2: apply replacements
        let func = module
            .functions
            .get_mut(&func_id)
            .expect("internal error: func_id not found in module");
        for (block_id, idx, method_func_id, obj_operand) in replacements {
            let block = func
                .blocks
                .get_mut(&block_id)
                .expect("internal error: block_id not found in function");
            let inst = &mut block.instructions[idx];

            if let InstructionKind::CallVirtual { dest, args, .. } = &inst.kind {
                let dest = *dest;
                let mut new_args = Vec::with_capacity(1 + args.len());
                new_args.push(obj_operand);
                new_args.extend(args.iter().cloned());

                inst.kind = InstructionKind::CallDirect {
                    dest,
                    func: method_func_id,
                    args: new_args,
                };
            }
        }
    }

    changed
}

/// Pass wrapper for devirtualization.
pub struct DevirtualizePass;

impl OptimizationPass for DevirtualizePass {
    fn name(&self) -> &str {
        "devirtualize"
    }

    fn run_once(&mut self, module: &mut Module, _interner: &mut StringInterner) -> bool {
        devirtualize(module)
    }

    fn is_fixpoint(&self) -> bool {
        false
    }
}
