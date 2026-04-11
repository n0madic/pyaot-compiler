//! Argument expansion and lowering helpers

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

use super::ExpandedArg;

impl<'a> Lowering<'a> {
    /// Lower expanded call arguments to MIR operands, handling runtime tuple unpacking.
    /// If `param_types` is provided, parameter types are propagated into argument
    /// expressions via `expected_type` (bidirectional type inference).
    pub(crate) fn lower_expanded_args(
        &mut self,
        expanded_args: &[ExpandedArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<Vec<mir::Operand>> {
        self.lower_expanded_args_with_params(expanded_args, None, hir_module, mir_func)
    }

    /// Lower expanded call arguments with optional parameter type propagation.
    pub(super) fn lower_expanded_args_with_params(
        &mut self,
        expanded_args: &[ExpandedArg],
        param_types: Option<&[hir::Param]>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<Vec<mir::Operand>> {
        let mut operands = Vec::new();
        let mut positional_index = 0usize;

        for arg in expanded_args {
            match arg {
                ExpandedArg::Regular(expr_id) => {
                    let arg_expr = &hir_module.exprs[*expr_id];

                    // Bidirectional: propagate parameter type into argument expression
                    let expected = param_types
                        .and_then(|p| p.get(positional_index))
                        .and_then(|p| p.ty.clone());
                    let operand = self.lower_expr_expecting(
                        arg_expr,
                        expected.clone(),
                        hir_module,
                        mir_func,
                    )?;

                    // Box primitives when passing to Union-typed parameters
                    let operand = if matches!(&expected, Some(Type::Union(_))) {
                        let arg_type = self.operand_type(&operand, mir_func);
                        self.box_primitive_if_needed(operand, &arg_type, mir_func)
                    } else {
                        operand
                    };

                    operands.push(operand);
                    positional_index += 1;
                }
                ExpandedArg::RuntimeUnpackTuple(expr_id) => {
                    // Runtime tuple unpacking - extract each element
                    let tuple_expr = &hir_module.exprs[*expr_id];
                    let tuple_type = self.get_type_of_expr_id(*expr_id, hir_module);

                    // Lower the tuple expression to get the operand
                    let tuple_operand = self.lower_expr(tuple_expr, hir_module, mir_func)?;

                    // Extract element types
                    if let Type::Tuple(elem_types) = tuple_type {
                        // Extract each element from the tuple
                        for (i, elem_type) in elem_types.iter().enumerate() {
                            let elem_local = self.alloc_and_add_local(elem_type.clone(), mir_func);

                            let get_func = crate::type_dispatch::tuple_get_func(elem_type);

                            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                                dest: elem_local,
                                func: get_func,
                                args: vec![
                                    tuple_operand.clone(),
                                    mir::Operand::Constant(mir::Constant::Int(i as i64)),
                                ],
                            });

                            operands.push(mir::Operand::Local(elem_local));
                        }
                    } else {
                        // Should not happen - type checker should catch this
                        // But handle gracefully by passing the tuple as-is
                        operands.push(tuple_operand);
                    }
                }
                ExpandedArg::RuntimeUnpackList(_expr_id) => {
                    // TODO: Implement full list unpacking for all call paths
                    // Runtime list unpacking is handled in resolve_call_args
                    // where we have access to the function signature.
                    // This case should not be reached when using lower_expanded_args
                    // directly (without resolve_call_args).
                    return Err(pyaot_diagnostics::CompilerError::semantic_error(
                        "Star unpacking of non-literal lists is not yet supported in this call context",
                        self.call_span(),
                    ));
                }
            }
        }

        Ok(operands)
    }
}
