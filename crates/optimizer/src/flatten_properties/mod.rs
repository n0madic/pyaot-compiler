//! Property flattening pass
//!
//! Detects trivial property getters — functions whose body is a single
//! `InstanceGetField(self, offset)` followed by a return — and replaces
//! `CallDirect` calls to them with an inline `InstanceGetField` instruction.
//! This eliminates function call overhead for simple `@property` accessors.

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use pyaot_core_defs::runtime_func_def::RT_INSTANCE_GET_FIELD;
use pyaot_mir::{Constant, InstructionKind, Module, Operand, RuntimeFunc, Terminator};
use pyaot_utils::{FuncId, StringInterner};

use crate::pass::OptimizationPass;

/// Analyze a function to determine if it is a trivial getter.
///
/// A trivial getter has:
/// - Exactly 1 parameter (self)
/// - Exactly 1 basic block
/// - Exactly 1 instruction: `RuntimeCall { func: InstanceGetField, args: [self, Const(offset)] }`
/// - Terminator: `Return(Some(Local(dest)))` where dest matches the instruction's dest
///
/// Returns `Some(offset)` if the function is a trivial getter.
fn analyze_trivial_getter(func: &pyaot_mir::Function) -> Option<i64> {
    if func.params.len() != 1 {
        return None;
    }
    if func.blocks.len() != 1 {
        return None;
    }

    let self_param_id = func.params[0].id;
    let block = func.blocks.values().next()?;

    if block.instructions.len() != 1 {
        return None;
    }

    let inst = &block.instructions[0];

    if let InstructionKind::RuntimeCall {
        dest,
        func: RuntimeFunc::Call(def),
        args,
    } = &inst.kind
    {
        if def.symbol != RT_INSTANCE_GET_FIELD.symbol {
            return None;
        }
        if args.len() != 2 {
            return None;
        }
        // First arg must be the self parameter
        if args[0] != Operand::Local(self_param_id) {
            return None;
        }
        // Second arg must be a constant offset
        if let Operand::Constant(Constant::Int(offset)) = &args[1] {
            // Terminator must return the instruction's dest
            if let Terminator::Return(Some(Operand::Local(ret_id))) = &block.terminator {
                if *ret_id == *dest {
                    return Some(*offset);
                }
            }
        }
    }

    None
}

/// Run property flattening on the entire module.
///
/// Phase 1: Scan all functions to identify trivial getters.
/// Phase 2: Replace `CallDirect` to trivial getters with inline `InstanceGetField`.
pub fn flatten_property_getters(module: &mut Module) -> bool {
    // Phase 1: identify trivial getters
    let mut trivial_getters: HashMap<FuncId, i64> = HashMap::new();
    for (func_id, func) in &module.functions {
        if let Some(offset) = analyze_trivial_getter(func) {
            trivial_getters.insert(*func_id, offset);
        }
    }

    if trivial_getters.is_empty() {
        return false;
    }

    // Phase 2: replace CallDirect to trivial getters with InstanceGetField
    let func_ids: Vec<FuncId> = module.functions.keys().copied().collect();
    let mut changed = false;

    for func_id in func_ids {
        let func = module
            .functions
            .get_mut(&func_id)
            .expect("internal error: func_id not found in module");
        for block in func.blocks.values_mut() {
            for inst in &mut block.instructions {
                if let InstructionKind::CallDirect {
                    dest,
                    func: callee_id,
                    args,
                } = &inst.kind
                {
                    if args.len() == 1 {
                        if let Some(&offset) = trivial_getters.get(callee_id) {
                            let dest = *dest;
                            let obj_operand = args[0].clone();

                            inst.kind = InstructionKind::RuntimeCall {
                                dest,
                                func: RuntimeFunc::Call(&RT_INSTANCE_GET_FIELD),
                                args: vec![obj_operand, Operand::Constant(Constant::Int(offset))],
                            };
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    changed
}

/// Pass wrapper for property flattening.
pub struct FlattenPropertiesPass;

impl OptimizationPass for FlattenPropertiesPass {
    fn name(&self) -> &str {
        "flatten-properties"
    }

    fn run_once(&mut self, module: &mut Module, _interner: &mut StringInterner) -> bool {
        flatten_property_getters(module)
    }

    fn is_fixpoint(&self) -> bool {
        false
    }
}
