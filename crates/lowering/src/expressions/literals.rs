//! Literal expression lowering: Int, Float, Bool, Str, None, Var

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{InternedString, VarId};

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a string literal to a heap-allocated string object.
    pub(super) fn lower_str_literal(
        &mut self,
        s: InternedString,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // For string literals, we need to allocate them on the heap
        // so they can be used with string operations
        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        // Emit runtime call to make heap-allocated string
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::MakeStr,
            args: vec![mir::Operand::Constant(mir::Constant::Str(s))],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower a bytes literal to a heap-allocated bytes object.
    pub(super) fn lower_bytes_literal(
        &mut self,
        data: &[u8],
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // For bytes literals, we need to allocate them on the heap
        let result_local = self.alloc_and_add_local(Type::Bytes, mir_func);

        // Emit runtime call to make heap-allocated bytes
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::MakeBytes,
            args: vec![mir::Operand::Constant(mir::Constant::Bytes(data.to_vec()))],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower a variable reference.
    pub(super) fn lower_var(
        &mut self,
        var_id: VarId,
        expr: &hir::Expr,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Check if this is a global variable
        if self.is_global(&var_id) {
            // Global variable: emit runtime call to get the value
            let var_type = self
                .get_var_type(&var_id)
                .cloned()
                .or_else(|| expr.ty.clone())
                .unwrap_or(Type::Int); // Globals default to Int for backward compatibility

            let result_local = self.alloc_local_id();
            mir_func.add_local(mir::Local {
                id: result_local,
                name: None,
                ty: var_type.clone(),
                is_gc_root: var_type.is_heap(),
            });

            // Determine the type-specific runtime function for global get
            let runtime_func = self.get_global_get_func(&var_type);

            // Emit type-specific GlobalGet runtime call with offset-adjusted VarId
            let effective_var_id = self.get_effective_var_id(var_id);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: runtime_func,
                args: vec![mir::Operand::Constant(mir::Constant::Int(effective_var_id))],
            });

            Ok(mir::Operand::Local(result_local))
        } else if let Some(cell_local) = self.get_nonlocal_cell(&var_id) {
            // Cell-wrapped variable (either cell_var or nonlocal_var): read through cell
            let var_type = self
                .get_var_type(&var_id)
                .cloned()
                .or_else(|| expr.ty.clone())
                .unwrap_or(Type::Int);

            let result_local = self.alloc_local_id();
            mir_func.add_local(mir::Local {
                id: result_local,
                name: None,
                ty: var_type.clone(),
                is_gc_root: var_type.is_heap(),
            });

            // Emit cell get operation
            let get_func = self.get_cell_get_func(&var_type);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: get_func,
                args: vec![mir::Operand::Local(cell_local)],
            });

            Ok(mir::Operand::Local(result_local))
        } else {
            // Local variable: use the standard local mapping
            let narrowed_type = self
                .get_var_type(&var_id)
                .cloned()
                .or_else(|| expr.ty.clone())
                .unwrap_or(Type::Any);

            // Check if this is a Union variable that has been narrowed to a primitive type
            // In that case, we need to unbox the value since Union variables are stored as boxed pointers
            let original_union_type = self.get_narrowed_union_type(&var_id);
            let needs_unbox = original_union_type.is_some()
                && matches!(narrowed_type, Type::Int | Type::Float | Type::Bool);

            if needs_unbox {
                // Get the local holding the boxed value (using original Union type)
                let original_ty = original_union_type.unwrap_or(Type::Any);
                let boxed_local = self.get_or_create_local(var_id, original_ty, mir_func);

                // Create a new local for the unboxed primitive value
                let unboxed_local = self.alloc_and_add_local(narrowed_type.clone(), mir_func);

                // Emit unbox operation based on narrowed type
                let unbox_func = match narrowed_type {
                    Type::Int => mir::RuntimeFunc::UnboxInt,
                    Type::Float => mir::RuntimeFunc::UnboxFloat,
                    Type::Bool => mir::RuntimeFunc::UnboxBool,
                    _ => unreachable!(), // Already checked in needs_unbox condition
                };

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: unboxed_local,
                    func: unbox_func,
                    args: vec![mir::Operand::Local(boxed_local)],
                });

                Ok(mir::Operand::Local(unboxed_local))
            } else {
                let local_id = self.get_or_create_local(var_id, narrowed_type, mir_func);
                Ok(mir::Operand::Local(local_id))
            }
        }
    }
}
