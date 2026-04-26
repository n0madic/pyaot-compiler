//! Default parameter handling and positional/keyword argument matching.

use super::KwargsMatchResult;
use crate::context::Lowering;
use indexmap::IndexMap;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

impl<'a> Lowering<'a> {
    /// Match positional arguments to regular parameters.
    ///
    /// Returns (resolved params, extra positional args).
    pub(crate) fn match_positional_to_params(
        &self,
        all_positional: Vec<mir::Operand>,
        num_regular_params: usize,
    ) -> (Vec<Option<mir::Operand>>, Vec<mir::Operand>) {
        let mut resolved: Vec<Option<mir::Operand>> = vec![None; num_regular_params];
        let mut extra_positional = Vec::new();

        for (i, operand) in all_positional.into_iter().enumerate() {
            if i < num_regular_params {
                resolved[i] = Some(operand);
            } else {
                extra_positional.push(operand);
            }
        }

        (resolved, extra_positional)
    }

    /// Match keyword arguments to parameters.
    ///
    /// Returns (updated resolved, kwonly_resolved, extra_keywords).
    pub(crate) fn match_kwargs_to_params(
        &mut self,
        kwargs: &[hir::KeywordArg],
        regular_params: &[&hir::Param],
        kwonly_params: &[&hir::Param],
        resolved: &mut [Option<mir::Operand>],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<KwargsMatchResult> {
        let mut kwonly_resolved: Vec<Option<mir::Operand>> = vec![None; kwonly_params.len()];
        let mut extra_keywords = IndexMap::new();

        for kwarg in kwargs {
            let kwarg_name = self.resolve(kwarg.name);

            // Try regular params first
            let param_idx = regular_params
                .iter()
                .position(|p| self.resolve(p.name) == kwarg_name);

            if let Some(idx) = param_idx {
                if resolved[idx].is_some() {
                    return Err(CompilerError::duplicate_keyword_argument(
                        kwarg_name.to_string(),
                        kwarg.span,
                    ));
                }
                let arg_expr = &hir_module.exprs[kwarg.value];
                resolved[idx] = Some(self.lower_expr(arg_expr, hir_module, mir_func)?);
            } else {
                // Try kwonly params
                let kwonly_idx = kwonly_params
                    .iter()
                    .position(|p| self.resolve(p.name) == kwarg_name);

                if let Some(idx) = kwonly_idx {
                    if kwonly_resolved[idx].is_some() {
                        return Err(CompilerError::duplicate_keyword_argument(
                            kwarg_name.to_string(),
                            kwarg.span,
                        ));
                    }
                    let arg_expr = &hir_module.exprs[kwarg.value];
                    kwonly_resolved[idx] = Some(self.lower_expr(arg_expr, hir_module, mir_func)?);
                } else {
                    // Extra kwarg
                    let arg_expr = &hir_module.exprs[kwarg.value];
                    extra_keywords
                        .insert(kwarg.name, self.lower_expr(arg_expr, hir_module, mir_func)?);
                }
            }
        }

        Ok((kwonly_resolved, extra_keywords))
    }

    /// Fill defaults for missing parameters.
    ///
    /// If `target_func_id` is provided, mutable defaults (list, dict, set, class instances)
    /// are loaded from global storage instead of being re-evaluated. This implements Python's
    /// semantics where mutable defaults are evaluated once at function definition time.
    ///
    /// The `param_index_offset` adjusts the lookup index when the params slice doesn't start
    /// at index 0 in the original function. For example, when calling `__init__`, the `self`
    /// parameter is skipped when resolving user arguments, but `default_value_slots` uses
    /// indices relative to the original function parameters (including `self`).
    pub(crate) fn fill_param_defaults(
        &mut self,
        resolved: &mut [Option<mir::Operand>],
        params: &[&hir::Param],
        target_func_id: Option<pyaot_utils::FuncId>,
        param_index_offset: usize,
        call_span: pyaot_utils::Span,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        for (i, param) in params.iter().enumerate() {
            if resolved[i].is_none() {
                if let Some(default_id) = param.default {
                    // Check if this parameter has a stored mutable default
                    // Adjust index by offset when looking up stored mutable defaults
                    let stored_slot = target_func_id
                        .and_then(|fid| self.get_default_slot(&(fid, i + param_index_offset)));

                    if let Some(slot) = stored_slot {
                        // Load the pre-evaluated default from global storage —
                        // pre-evaluated defaults are heap pointers (boxed),
                        // so use the Ptr variant of the typed extern.
                        let default_local = self.emit_runtime_call_gc(
                            mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_GLOBAL_GET_PTR,
                            ),
                            vec![mir::Operand::Constant(mir::Constant::Int(slot as i64))],
                            Type::Any,
                            mir_func,
                        );
                        resolved[i] = Some(mir::Operand::Local(default_local));
                    } else {
                        // Evaluate the default expression (immutable types or unknown func)
                        let default_expr = &hir_module.exprs[default_id];
                        resolved[i] = Some(self.lower_expr(default_expr, hir_module, mir_func)?);
                    }
                } else {
                    let param_name = self.resolve(param.name).to_string();
                    return Err(CompilerError::missing_required_argument(
                        param_name, call_span,
                    ));
                }
            }
        }
        Ok(())
    }
}
