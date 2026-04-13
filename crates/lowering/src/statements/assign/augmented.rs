//! Index delete lowering: del obj[key]

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a delete indexed item: del obj[key]
    /// Uses DictPop for dicts and ListPop for lists (discarding the result).
    pub(crate) fn lower_index_delete(
        &mut self,
        obj: hir::ExprId,
        index: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let obj_expr = &hir_module.exprs[obj];
        let obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
        let obj_type = self.get_type_of_expr_id(obj, hir_module);

        let index_expr = &hir_module.exprs[index];
        let index_operand = self.lower_expr(index_expr, hir_module, mir_func)?;
        let index_type = self.get_type_of_expr_id(index, hir_module);

        match obj_type {
            Type::Dict(_, _) | Type::DefaultDict(_, _) => {
                // del dict[key] → rt_dict_pop(dict, key) and discard result
                let boxed_key = self.box_primitive_if_needed(index_operand, &index_type, mir_func);
                self.emit_runtime_call_void(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_POP),
                    vec![obj_operand, boxed_key],
                    mir_func,
                );
            }
            Type::List(_) => {
                // del list[index] → rt_list_pop(list, index) and discard result
                self.emit_runtime_call_void(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_POP),
                    vec![obj_operand, index_operand],
                    mir_func,
                );
            }
            Type::Class { class_id, .. } => {
                // Class with __delitem__ dunder
                let delitem_func = self
                    .get_class_info(&class_id)
                    .and_then(|info| info.get_dunder_func("__delitem__"));

                if let Some(func_id) = delitem_func {
                    let dummy_local = self.alloc_and_add_local(Type::Any, mir_func);
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: dummy_local,
                        func: func_id,
                        args: vec![obj_operand, index_operand],
                    });
                }
            }
            _ => {
                // Unsupported type for indexed delete
            }
        }

        Ok(())
    }
}
