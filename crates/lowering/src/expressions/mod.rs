//! Expression lowering from HIR to MIR
//!
//! This module handles lowering of all expression types from HIR to MIR.
//! It is organized into submodules by expression category:
//! - `literals`: Int, Float, Bool, Str, None, Var
//! - `operators`: BinOp, Compare, UnOp, LogicalOp
//! - `calls`: Call, ClassRef (instantiation)
//! - `builtins`: BuiltinCall (all built-in functions)
//! - `collections`: List, Tuple, Dict, Set
//! - `access`: Index, Slice, Attribute, MethodCall

mod access;
mod builtins; // Directory module with submodules: print, conversions, math, predicates, introspection, iteration, collections
mod calls;
mod collections;
mod literals;
mod operators;
mod stdlib;

// Re-export ExpandedArg for use in resolve_call_args
pub(crate) use calls::ExpandedArg;

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::LocalId;

use crate::context::Lowering;
use crate::utils::is_heap_type;

impl<'a> Lowering<'a> {
    /// Main entry point for lowering an expression.
    /// Dispatches to appropriate submodule based on expression kind.
    pub(crate) fn lower_expr(
        &mut self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        match &expr.kind {
            // Literals (literals.rs)
            hir::ExprKind::Int(val) => Ok(mir::Operand::Constant(mir::Constant::Int(*val))),
            hir::ExprKind::Float(val) => Ok(mir::Operand::Constant(mir::Constant::Float(*val))),
            hir::ExprKind::Bool(val) => Ok(mir::Operand::Constant(mir::Constant::Bool(*val))),
            hir::ExprKind::Str(s) => self.lower_str_literal(*s, mir_func),
            hir::ExprKind::Bytes(b) => self.lower_bytes_literal(b, mir_func),
            hir::ExprKind::None => Ok(mir::Operand::Constant(mir::Constant::None)),
            hir::ExprKind::Var(var_id) => self.lower_var(*var_id, expr, mir_func),

            // Operators (operators.rs)
            hir::ExprKind::BinOp { op, left, right } => {
                self.lower_binop(*op, *left, *right, expr, hir_module, mir_func)
            }
            hir::ExprKind::Compare { left, op, right } => {
                self.lower_compare(*left, *op, *right, hir_module, mir_func)
            }
            hir::ExprKind::UnOp { op, operand } => {
                self.lower_unop(*op, *operand, hir_module, mir_func)
            }
            hir::ExprKind::LogicalOp { op, left, right } => {
                self.lower_logical_op(*op, *left, *right, hir_module, mir_func)
            }

            // Calls (calls.rs)
            hir::ExprKind::Call {
                func,
                args,
                kwargs,
                kwargs_unpack,
            } => self.lower_call(
                *func,
                args,
                kwargs,
                kwargs_unpack,
                expr,
                hir_module,
                mir_func,
            ),
            hir::ExprKind::ClassRef(class_id) => {
                // This expression is used when a class is called as constructor
                // The actual instantiation is handled in Call expression
                // Here we just store the class ID constant with offset adjustment
                let effective_class_id = self.get_effective_class_id(*class_id);
                Ok(mir::Operand::Constant(mir::Constant::Int(
                    effective_class_id,
                )))
            }

            // Class attribute reference: ClassName.attr
            hir::ExprKind::ClassAttrRef { class_id, attr } => {
                self.lower_class_attr_ref(*class_id, *attr, mir_func)
            }

            // Ternary expression (operators.rs)
            hir::ExprKind::IfExpr {
                cond,
                then_val,
                else_val,
            } => self.lower_if_expr(*cond, *then_val, *else_val, hir_module, mir_func),

            // Builtins (builtins.rs)
            hir::ExprKind::BuiltinCall {
                builtin,
                args,
                kwargs,
            } => self.lower_builtin_call(builtin, args, kwargs, hir_module, mir_func),

            // Collections (collections.rs)
            hir::ExprKind::List(elements) => self.lower_list(elements, expr, hir_module, mir_func),
            hir::ExprKind::Tuple(elements) => {
                self.lower_tuple(elements, expr, hir_module, mir_func)
            }
            hir::ExprKind::Dict(pairs) => self.lower_dict(pairs, expr, hir_module, mir_func),
            hir::ExprKind::Set(elements) => self.lower_set(elements, expr, hir_module, mir_func),

            // Access (access.rs)
            hir::ExprKind::Slice {
                obj,
                start,
                end,
                step,
            } => self.lower_slice(*obj, start, end, step, hir_module, mir_func),
            hir::ExprKind::Index { obj, index } => {
                self.lower_index(*obj, *index, hir_module, mir_func)
            }
            hir::ExprKind::MethodCall {
                obj,
                method,
                args,
                kwargs,
            } => self.lower_method_call(*obj, *method, args, kwargs, hir_module, mir_func),
            hir::ExprKind::Attribute { obj, attr } => {
                self.lower_attribute(*obj, *attr, hir_module, mir_func)
            }

            // Function reference: emit FuncAddr instruction to get function pointer
            // This is used when passing a function as an argument (e.g., to decorators)
            // Direct calls to FuncRef are handled in calls.rs
            hir::ExprKind::FuncRef(func_id) => {
                let result_local = self.alloc_and_add_local(Type::Any, mir_func);
                self.emit_instruction(mir::InstructionKind::FuncAddr {
                    dest: result_local,
                    func: *func_id,
                });
                Ok(mir::Operand::Local(result_local))
            }

            // Built-in function reference (len, str, int, etc.) - get function pointer from runtime table
            hir::ExprKind::BuiltinRef(builtin_kind) => {
                let result_local = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::BuiltinAddr {
                    dest: result_local,
                    builtin: *builtin_kind,
                });
                Ok(mir::Operand::Local(result_local))
            }

            // Closure expression: store as a function reference with captured values
            // The actual call handling prepends captures to arguments (in calls.rs)
            hir::ExprKind::Closure { func, captures } => {
                self.lower_closure(*func, captures, hir_module, mir_func)
            }

            // Yield expression (inside generator functions)
            // This is handled specially during generator lowering
            hir::ExprKind::Yield(value) => {
                self.lower_yield_expr(value.as_ref().copied(), hir_module, mir_func)
            }

            // Type reference (for isinstance)
            hir::ExprKind::TypeRef(_) => Ok(mir::Operand::Constant(mir::Constant::None)),

            // Super call for inheritance: super().method(args)
            hir::ExprKind::SuperCall { method, args } => {
                self.lower_super_call(*method, args, hir_module, mir_func)
            }

            // Imported reference: resolved during multi-module merging
            // By the time we get here, these should be resolved to FuncRef/ClassRef/Var
            hir::ExprKind::ImportedRef { module, name } => {
                self.lower_imported_ref(module, name, hir_module, mir_func)
            }

            // Module attribute access: resolved during multi-module merging
            hir::ExprKind::ModuleAttr { module, attr } => {
                self.lower_module_attr(module, *attr, hir_module, mir_func)
            }

            // Standard library attribute access (sys.argv, os.environ)
            hir::ExprKind::StdlibAttr(attr) => self.lower_stdlib_attr(attr, mir_func),

            // Standard library function call (sys.exit, os.path.join, re.*)
            hir::ExprKind::StdlibCall { func, args } => {
                self.lower_stdlib_call(func, args, hir_module, mir_func)
            }

            // Standard library compile-time constant (math.pi, math.e)
            hir::ExprKind::StdlibConst(const_def) => self.lower_stdlib_const(const_def, mir_func),
        }
    }

    /// Lower an imported reference expression.
    /// Handles imported variables (from module import VAR) and imported functions used as values.
    fn lower_imported_ref(
        &mut self,
        module: &str,
        name: &str,
        _hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Look up the variable in module exports
        let key = (module.to_string(), name.to_string());
        if let Some((var_id, var_type)) = self.get_module_var_export(&key).cloned() {
            // This is an imported variable - emit global read
            let result_local = self.alloc_local_id();
            let is_ptr_type = self.is_global_ptr_type(&var_type);
            mir_func.add_local(mir::Local {
                id: result_local,
                name: None,
                ty: var_type.clone(),
                is_gc_root: is_ptr_type,
            });

            let get_func = self.get_global_get_func(&var_type);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: get_func,
                args: vec![mir::Operand::Constant(mir::Constant::Int(var_id.0 as i64))],
            });

            return Ok(mir::Operand::Local(result_local));
        }

        // Not a variable - might be a function reference (not yet supported as value)
        // Return None as placeholder
        Ok(mir::Operand::Constant(mir::Constant::None))
    }

    /// Lower a module attribute access expression.
    /// Handles access to module-level variables (globals) from imported modules.
    /// For function calls, this is bypassed by lower_call() which uses lower_imported_call().
    fn lower_module_attr(
        &mut self,
        module: &str,
        attr: pyaot_utils::InternedString,
        _hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let attr_name = self.resolve(attr).to_string();

        // Look up the variable in module exports
        let key = (module.to_string(), attr_name.clone());
        if let Some((var_id, var_type)) = self.get_module_var_export(&key).cloned() {
            // Emit global read for the module variable
            let result_local = self.alloc_local_id();
            let is_ptr_type = self.is_global_ptr_type(&var_type);
            mir_func.add_local(mir::Local {
                id: result_local,
                name: None,
                ty: var_type.clone(),
                is_gc_root: is_ptr_type,
            });

            let get_func = self.get_global_get_func(&var_type);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: get_func,
                args: vec![mir::Operand::Constant(mir::Constant::Int(var_id.0 as i64))],
            });

            return Ok(mir::Operand::Local(result_local));
        }

        // Not a variable - could be a function reference (not yet supported)
        // Return None for now
        Ok(mir::Operand::Constant(mir::Constant::None))
    }

    /// Lower a class attribute reference expression: ClassName.attr
    fn lower_class_attr_ref(
        &mut self,
        class_id: pyaot_utils::ClassId,
        attr: pyaot_utils::InternedString,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Look up the class info
        if let Some(class_info) = self.get_class_info(&class_id) {
            // Get attribute (owning_class_id, offset) and type
            // The owning_class_id is the class where the attribute was actually defined,
            // which matters for inherited attributes
            if let (Some(&(owning_class_id, attr_offset)), Some(attr_type)) = (
                class_info.class_attr_offsets.get(&attr),
                class_info.class_attr_types.get(&attr).cloned(),
            ) {
                // Allocate result local
                let result_local = self.alloc_typed_local(mir_func, attr_type.clone());

                // Get the appropriate runtime function based on type
                let get_func = self.get_class_attr_get_func(&attr_type);

                // Emit runtime call: rt_class_attr_get_*(owning_class_id, attr_idx)
                // Use the owning_class_id, not the accessed class_id, to handle inheritance
                let effective_class_id = self.get_effective_class_id(owning_class_id);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: get_func,
                    args: vec![
                        mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                        mir::Operand::Constant(mir::Constant::Int(attr_offset as i64)),
                    ],
                });

                return Ok(mir::Operand::Local(result_local));
            }
        }

        // Attribute not found - return 0 as default
        Ok(mir::Operand::Constant(mir::Constant::Int(0)))
    }

    /// Lower a closure expression.
    /// For non-captured closures (lambdas without free variables), this is just a FuncRef.
    /// For closures with captures, we create a closure tuple (func_ptr, cap0, cap1, ...).
    pub(super) fn lower_closure(
        &mut self,
        func: pyaot_utils::FuncId,
        captures: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if captures.is_empty() {
            // No captures - just return the function ID as a constant
            Ok(mir::Operand::Constant(mir::Constant::Int(func.0 as i64)))
        } else {
            // With captures: create a nested closure tuple (func_ptr, (cap0, cap1, ...))
            // The outer tuple has exactly 2 elements: func_ptr and captures_tuple
            // This uniform format simplifies dispatch in indirect calls.

            // Get function address
            let func_ptr_local = self.alloc_and_add_local(Type::Any, mir_func);
            self.emit_instruction(mir::InstructionKind::FuncAddr {
                dest: func_ptr_local,
                func,
            });

            // Lower all captured expressions
            let mut capture_operands = Vec::with_capacity(captures.len());
            for capture_id in captures {
                let capture_expr = &hir_module.exprs[*capture_id];
                // Check if this capture is a cell variable - if so, pass the cell pointer directly
                let capture_op = if let hir::ExprKind::Var(var_id) = &capture_expr.kind {
                    if let Some(cell_local) = self.get_nonlocal_cell(var_id) {
                        // This is a cell variable - pass the cell pointer, not the value
                        mir::Operand::Local(cell_local)
                    } else {
                        self.lower_expr(capture_expr, hir_module, mir_func)?
                    }
                } else {
                    self.lower_expr(capture_expr, hir_module, mir_func)?
                };
                capture_operands.push(capture_op);
            }

            let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

            // Determine capture tuple elem_tag based on actual capture types.
            // If no capture needs GC tracing (all ints/bools/floats), use ELEM_RAW_INT
            // so the GC doesn't try to trace raw values as heap pointers.
            // Cell variables are heap pointers and DO need GC tracing.
            let capture_elem_tag: i64 = {
                let any_needs_gc = captures.iter().enumerate().any(|(i, capture_id)| {
                    let capture_expr = &hir_module.exprs[*capture_id];
                    // Cell variables are always heap pointers
                    if let hir::ExprKind::Var(var_id) = &capture_expr.kind {
                        if self.get_nonlocal_cell(var_id).is_some() {
                            return true;
                        }
                    }
                    // Check the operand type (more reliable than expression type)
                    let op_type = self.operand_type(&capture_operands[i], mir_func);
                    Self::type_needs_gc_trace(&op_type)
                });
                if any_needs_gc {
                    0
                } else {
                    1
                }
            };

            // Collect capture operand types for heap_field_mask
            let capture_op_types: Vec<Type> = capture_operands
                .iter()
                .map(|op| self.operand_type(op, mir_func))
                .collect();

            let captures_tuple = self.alloc_and_add_local(Type::Any, mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: captures_tuple,
                func: mir::RuntimeFunc::MakeTuple,
                args: vec![
                    mir::Operand::Constant(mir::Constant::Int(captures.len() as i64)),
                    mir::Operand::Constant(mir::Constant::Int(capture_elem_tag)),
                ],
            });

            // Set per-field heap_field_mask for mixed-type captures
            if capture_elem_tag == 0 {
                self.emit_heap_field_mask(captures_tuple, &capture_op_types, mir_func);
            }

            // Set each capture in the inner tuple
            for (i, capture_op) in capture_operands.into_iter().enumerate() {
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::TupleSet,
                    args: vec![
                        mir::Operand::Local(captures_tuple),
                        mir::Operand::Constant(mir::Constant::Int(i as i64)),
                        capture_op,
                    ],
                });
            }

            // Create outer tuple (func_ptr, captures_tuple) - always size 2
            // heap_field_mask: bit 0 = 0 (func_ptr is raw), bit 1 = 1 (captures_tuple is heap)
            let result_local = self.alloc_and_add_local(Type::Any, mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::MakeTuple,
                args: vec![
                    mir::Operand::Constant(mir::Constant::Int(2)),
                    mir::Operand::Constant(mir::Constant::Int(0)), // ELEM_HEAP_OBJ
                ],
            });
            self.emit_heap_field_mask(
                result_local,
                &[Type::Int, Type::Any], // func_ptr=raw, captures=heap
                mir_func,
            );

            // Set func_ptr at index 0
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::TupleSet,
                args: vec![
                    mir::Operand::Local(result_local),
                    mir::Operand::Constant(mir::Constant::Int(0)),
                    mir::Operand::Local(func_ptr_local),
                ],
            });

            // Set captures_tuple at index 1
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::TupleSet,
                args: vec![
                    mir::Operand::Local(result_local),
                    mir::Operand::Constant(mir::Constant::Int(1)),
                    mir::Operand::Local(captures_tuple),
                ],
            });

            Ok(mir::Operand::Local(result_local))
        }
    }

    // =====================================================================
    // Shared Helper Functions
    // =====================================================================

    /// Allocate a local variable with standard configuration.
    /// Automatically sets is_gc_root based on whether the type is heap-allocated.
    pub(super) fn alloc_typed_local(&mut self, mir_func: &mut mir::Function, ty: Type) -> LocalId {
        let local_id = self.alloc_local_id();
        mir_func.add_local(mir::Local {
            id: local_id,
            name: None,
            ty: ty.clone(),
            is_gc_root: is_heap_type(&ty),
        });
        local_id
    }

    /// Promote an operand to float if needed.
    /// Returns the operand unchanged if already float, otherwise emits IntToFloat conversion.
    pub(super) fn promote_to_float_if_needed(
        &mut self,
        mir_func: &mut mir::Function,
        operand: mir::Operand,
        current_type: &Type,
    ) -> mir::Operand {
        if *current_type != Type::Float {
            let temp_local = self.alloc_typed_local(mir_func, Type::Float);
            self.emit_instruction(mir::InstructionKind::IntToFloat {
                dest: temp_local,
                src: operand,
            });
            mir::Operand::Local(temp_local)
        } else {
            operand
        }
    }
}
