//! Argument expansion and lowering helpers

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::call_resolution::ParamClassification;
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
                    let tuple_type = self.expr_type_hint(*expr_id, hir_module);

                    // Lower the tuple expression to get the operand
                    let tuple_operand = self.lower_expr(tuple_expr, hir_module, mir_func)?;

                    // Extract element types
                    if let Type::Tuple(elem_types) = tuple_type {
                        // Extract each element from the tuple
                        for (i, elem_type) in elem_types.iter().enumerate() {
                            let get_func = crate::type_dispatch::tuple_get_func(elem_type);

                            let elem_local = self.emit_runtime_call(
                                get_func,
                                vec![
                                    tuple_operand.clone(),
                                    mir::Operand::Constant(mir::Constant::Int(i as i64)),
                                ],
                                elem_type.clone(),
                                mir_func,
                            );

                            operands.push(mir::Operand::Local(elem_local));
                        }
                    } else {
                        // Should not happen - type checker should catch this
                        // But handle gracefully by passing the tuple as-is
                        operands.push(tuple_operand);
                    }
                }
                ExpandedArg::RuntimeUnpackList(expr_id) => {
                    // When parameter types are known, delegate to the full list-unpack
                    // machinery (same path used by resolve_call_args).
                    if let Some(params) = param_types {
                        let param_classification = ParamClassification::from_params(params);
                        let extracted = self.lower_runtime_list_unpack(
                            *expr_id,
                            &operands,
                            &param_classification,
                            hir_module,
                            mir_func,
                        )?;
                        positional_index += extracted.len();
                        operands.extend(extracted);
                    } else {
                        // No signature available (indirect/dynamic call) — cannot determine
                        // how many elements to extract at compile time.
                        return Err(pyaot_diagnostics::CompilerError::semantic_error(
                            "Star unpacking of non-literal lists is not supported for \
                             indirect calls with unknown signatures",
                            self.call_span(),
                        ));
                    }
                }
            }
        }

        Ok(operands)
    }
}
