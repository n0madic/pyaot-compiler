//! Utility methods for type conversion, isinstance checks, and helper functions

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::FuncId;

use super::Lowering;

/// Result of extracting a function reference from an expression.
/// Can be either a user-defined function (with optional captures) or a builtin function.
#[derive(Debug, Clone)]
pub enum FuncOrBuiltin {
    /// User-defined function with optional closure captures
    UserFunc(FuncId, Vec<hir::ExprId>),
    /// Built-in function (len, str, int, etc.) - no captures needed
    Builtin(mir::BuiltinFunctionKind),
}

impl<'a> Lowering<'a> {
    /// Get the effective (offset-adjusted) VarId for a global variable
    pub(crate) fn get_effective_var_id(&self, var_id: pyaot_utils::VarId) -> i64 {
        (var_id.0 + self.var_id_offset) as i64
    }

    /// Get the effective (offset-adjusted) ClassId for a class
    pub(crate) fn get_effective_class_id(&self, class_id: pyaot_utils::ClassId) -> i64 {
        (class_id.0 + self.class_id_offset) as i64
    }

    /// Check if an expression is a variable that was narrowed from a Union type.
    /// This is used for `is`/`is not` comparisons where the variable still holds
    /// a boxed pointer even though the type has been narrowed.
    pub(crate) fn is_narrowed_union_var(&self, expr: &hir::Expr) -> bool {
        if let hir::ExprKind::Var(var_id) = &expr.kind {
            // Check if this variable is tracked in narrowed_union_vars
            // This tracks variables narrowed from Union to Int/Float/Bool/Str/None
            self.narrowed_union_vars.contains_key(var_id)
        } else {
            false
        }
    }

    /// Require exact argument count for a builtin function
    pub(crate) fn require_exact_args(
        &self,
        args: &[hir::ExprId],
        count: usize,
        func_name: &str,
    ) -> pyaot_diagnostics::Result<()> {
        if args.len() != count {
            return Err(pyaot_diagnostics::CompilerError::type_error(
                format!(
                    "{func_name}() requires exactly {count} argument(s), got {}",
                    args.len()
                ),
                pyaot_utils::Span::dummy(),
            ));
        }
        Ok(())
    }

    /// Require minimum argument count for a builtin function
    pub(crate) fn require_min_args(
        &self,
        args: &[hir::ExprId],
        min: usize,
        func_name: &str,
    ) -> pyaot_diagnostics::Result<()> {
        if args.len() < min {
            return Err(pyaot_diagnostics::CompilerError::type_error(
                format!(
                    "{func_name}() requires at least {min} argument(s), got {}",
                    args.len()
                ),
                pyaot_utils::Span::dummy(),
            ));
        }
        Ok(())
    }

    /// Emit boolean truthiness check for a collection via its length.
    /// Returns true if len(collection) != 0.
    pub(crate) fn emit_collection_bool_via_len(
        &mut self,
        len_func: mir::RuntimeFunc,
        operand: mir::Operand,
        result_local: pyaot_utils::LocalId,
        mir_func: &mut mir::Function,
    ) {
        let len_local = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: len_local,
            func: len_func,
            args: vec![operand],
        });
        let zero = mir::Operand::Constant(mir::Constant::Int(0));
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: result_local,
            op: mir::BinOp::NotEq,
            left: mir::Operand::Local(len_local),
            right: zero,
        });
    }

    /// Convert an operand to a boolean for use in branch conditions.
    /// This handles truthiness testing for all types (int != 0, str len > 0, etc.)
    pub(crate) fn convert_to_bool(
        &mut self,
        operand: mir::Operand,
        operand_type: &Type,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        match operand_type {
            Type::Bool => {
                // Already a bool, return as-is
                operand
            }
            Type::Int => {
                // bool(int) -> False if 0, True otherwise
                let result_local = self.alloc_local_id();
                mir_func.add_local(mir::Local {
                    id: result_local,
                    name: None,
                    ty: Type::Bool,
                    is_gc_root: false,
                });
                let zero = mir::Operand::Constant(mir::Constant::Int(0));
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: result_local,
                    op: mir::BinOp::NotEq,
                    left: operand,
                    right: zero,
                });
                mir::Operand::Local(result_local)
            }
            Type::Float => {
                // bool(float) -> False if 0.0, True otherwise
                let result_local = self.alloc_local_id();
                mir_func.add_local(mir::Local {
                    id: result_local,
                    name: None,
                    ty: Type::Bool,
                    is_gc_root: false,
                });
                let zero = mir::Operand::Constant(mir::Constant::Float(0.0));
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: result_local,
                    op: mir::BinOp::NotEq,
                    left: operand,
                    right: zero,
                });
                mir::Operand::Local(result_local)
            }
            Type::Str => {
                let result_local = self.alloc_and_add_local(Type::Bool, mir_func);
                self.emit_collection_bool_via_len(
                    mir::RuntimeFunc::StrLenInt,
                    operand,
                    result_local,
                    mir_func,
                );
                mir::Operand::Local(result_local)
            }
            Type::None => {
                // None is always falsy
                mir::Operand::Constant(mir::Constant::Bool(false))
            }
            Type::Bytes => {
                let result_local = self.alloc_and_add_local(Type::Bool, mir_func);
                self.emit_collection_bool_via_len(
                    mir::RuntimeFunc::BytesLen,
                    operand,
                    result_local,
                    mir_func,
                );
                mir::Operand::Local(result_local)
            }
            Type::List(_) | Type::Dict(_, _) | Type::Tuple(_) | Type::Set(_) => {
                let result_local = self.alloc_and_add_local(Type::Bool, mir_func);
                let runtime_func = match operand_type {
                    Type::List(_) => mir::RuntimeFunc::ListLen,
                    Type::Tuple(_) => mir::RuntimeFunc::TupleLen,
                    Type::Dict(_, _) => mir::RuntimeFunc::DictLen,
                    Type::Set(_) => mir::RuntimeFunc::SetLen,
                    _ => unreachable!(),
                };
                self.emit_collection_bool_via_len(runtime_func, operand, result_local, mir_func);
                mir::Operand::Local(result_local)
            }
            Type::Union(_) | Type::Any => {
                // For Union and Any types, use runtime truthiness dispatch
                let result_local = self.alloc_and_add_local(Type::Bool, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::IsTruthy,
                    args: vec![operand],
                });
                mir::Operand::Local(result_local)
            }
            _ => {
                // For other types (Iterator, Class instances, etc.), assume truthy
                // Class instances and iterators are always truthy in Python
                mir::Operand::Constant(mir::Constant::Bool(true))
            }
        }
    }

    /// Check if object type matches the isinstance check type (compile-time)
    pub(crate) fn types_match_isinstance(&self, obj_type: &Type, check_type: &Type) -> bool {
        match (obj_type, check_type) {
            (Type::Int, Type::Int) => true,
            (Type::Float, Type::Float) => true,
            (Type::Bool, Type::Bool) => true,
            (Type::Bool, Type::Int) => true, // bool is a subclass of int in Python
            (Type::Str, Type::Str) => true,
            (Type::None, Type::None) => true,
            (Type::List(_), Type::List(_)) => true,
            (Type::Tuple(_), Type::Tuple(_)) => true,
            (Type::Dict(_, _), Type::Dict(_, _)) => true,
            (Type::Class { class_id: id1, .. }, Type::Class { class_id: id2, .. }) => id1 == id2,
            _ => false,
        }
    }

    /// Get the TypeTag value for a type (from core-defs single source of truth)
    pub(crate) fn get_type_tag_for_isinstance_check(&self, ty: &Type) -> i64 {
        use pyaot_core_defs::TypeTagKind;
        match ty {
            Type::Int => TypeTagKind::Int.tag() as i64,
            Type::Float => TypeTagKind::Float.tag() as i64,
            Type::Bool => TypeTagKind::Bool.tag() as i64,
            Type::Str => TypeTagKind::Str.tag() as i64,
            Type::None => TypeTagKind::None.tag() as i64,
            Type::List(_) => TypeTagKind::List.tag() as i64,
            Type::Tuple(_) => TypeTagKind::Tuple.tag() as i64,
            Type::Dict(_, _) => TypeTagKind::Dict.tag() as i64,
            Type::Class { .. } => TypeTagKind::Instance.tag() as i64,
            Type::Iterator(_) => TypeTagKind::Iterator.tag() as i64,
            Type::Set(_) => TypeTagKind::Set.tag() as i64,
            Type::Bytes => TypeTagKind::Bytes.tag() as i64,
            Type::File => TypeTagKind::File.tag() as i64,
            _ => -1, // Unknown type
        }
    }

    /// Extract FuncId and captures from an expression (for map/filter with closures).
    /// Returns (func_id, captures) where captures is a list of ExprIds to be lowered.
    /// For non-capturing functions, returns empty captures vector.
    pub(crate) fn extract_func_with_captures(
        &self,
        expr: &hir::Expr,
        _hir_module: &hir::Module,
    ) -> Option<(FuncId, Vec<hir::ExprId>)> {
        match &expr.kind {
            hir::ExprKind::FuncRef(func_id) => Some((*func_id, Vec::new())),
            hir::ExprKind::Closure { func, captures } => Some((*func, captures.clone())),
            hir::ExprKind::Var(var_id) => {
                // Check if this variable holds a function reference (no captures)
                if let Some(func_id) = self.var_to_func.get(var_id) {
                    return Some((*func_id, Vec::new()));
                }
                // Check if it's a closure (with captures)
                if let Some((func_id, captures)) = self.var_to_closure.get(var_id) {
                    return Some((*func_id, captures.clone()));
                }
                None
            }
            _ => None,
        }
    }

    /// Extract a function or builtin from an expression (for map/filter/sorted with builtins).
    /// This extends extract_func_with_captures to also handle BuiltinRef expressions.
    /// Returns FuncOrBuiltin which can be a user function (with captures) or a builtin.
    pub(crate) fn extract_func_or_builtin(
        &self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
    ) -> Option<FuncOrBuiltin> {
        match &expr.kind {
            // Built-in function reference (len, str, int, etc.)
            hir::ExprKind::BuiltinRef(builtin_kind) => Some(FuncOrBuiltin::Builtin(*builtin_kind)),
            // User-defined function or closure - delegate to existing method
            _ => self
                .extract_func_with_captures(expr, hir_module)
                .map(|(func_id, captures)| FuncOrBuiltin::UserFunc(func_id, captures)),
        }
    }

    /// Extract key= and reverse= kwargs for sort/sorted operations.
    /// Returns a SortKwargs struct with the parsed values.
    /// Only processes "key" and "reverse" kwargs; caller should validate for unknown kwargs.
    /// Supports both user-defined functions and first-class builtins for key=.
    pub(crate) fn extract_sort_kwargs(
        &mut self,
        kwargs: &[hir::KeywordArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<super::SortKwargs> {
        use super::KeyFuncSource;
        use pyaot_diagnostics::CompilerError;

        let mut key_func: Option<KeyFuncSource> = None;
        let mut reverse_operand = mir::Operand::Constant(mir::Constant::Bool(false));

        for kw in kwargs {
            let kw_name = self.interner.resolve(kw.name);
            match kw_name {
                "key" => {
                    let key_expr = &hir_module.exprs[kw.value];
                    // key=None is a no-op
                    if !matches!(key_expr.kind, hir::ExprKind::None) {
                        // Try to extract as builtin or user function
                        if let Some(func_or_builtin) =
                            self.extract_func_or_builtin(key_expr, hir_module)
                        {
                            key_func = Some(match func_or_builtin {
                                FuncOrBuiltin::UserFunc(func_id, captures) => {
                                    KeyFuncSource::UserFunc(func_id, captures)
                                }
                                FuncOrBuiltin::Builtin(builtin_kind) => {
                                    KeyFuncSource::Builtin(builtin_kind)
                                }
                            });
                        } else {
                            return Err(CompilerError::type_error(
                                "'key' must be a function or None",
                                kw.span,
                            ));
                        }
                    }
                }
                "reverse" => {
                    reverse_operand =
                        self.lower_expr(&hir_module.exprs[kw.value], hir_module, mir_func)?;
                }
                _ => {
                    // Caller handles unknown kwargs
                }
            }
        }

        Ok(super::SortKwargs {
            reverse: reverse_operand,
            key_func,
        })
    }

    /// Emit FuncAddr or BuiltinAddr instruction for key function if present.
    /// Returns the operand to pass to runtime (key_fn_local) or None if no key function.
    /// Supports both user-defined functions and first-class builtins.
    pub(crate) fn emit_key_func_addr(
        &mut self,
        key_func: Option<&super::KeyFuncSource>,
        mir_func: &mut mir::Function,
    ) -> Option<mir::Operand> {
        use super::KeyFuncSource;
        key_func.map(|source| {
            let key_fn_local = self.alloc_and_add_local(Type::Int, mir_func);
            match source {
                KeyFuncSource::UserFunc(func_id, _captures) => {
                    // TODO: pass captures to key function for closures
                    self.emit_instruction(mir::InstructionKind::FuncAddr {
                        dest: key_fn_local,
                        func: *func_id,
                    });
                }
                KeyFuncSource::Builtin(builtin_kind) => {
                    self.emit_instruction(mir::InstructionKind::BuiltinAddr {
                        dest: key_fn_local,
                        builtin: *builtin_kind,
                    });
                }
            }
            mir::Operand::Local(key_fn_local)
        })
    }

    /// Determine the elem_tag for a given element type.
    /// Returns the constant value that corresponds to how the runtime stores elements:
    /// - 0 (ELEM_HEAP_OBJ): Elements are *mut Obj with valid headers
    /// - 1 (ELEM_RAW_INT): Elements are raw i64 values
    /// - 2 (ELEM_RAW_BOOL): Elements are raw i8 cast to pointer (currently not used in lists)
    ///
    /// This is used when passing elem_tag to runtime functions that need to box
    /// raw elements before calling key functions (sorted, min, max with key=).
    pub(crate) fn elem_tag_for_type(elem_type: &Type) -> i64 {
        match elem_type {
            Type::Int => 1,  // ELEM_RAW_INT
            Type::Bool => 0, // Bool in lists is boxed (ELEM_HEAP_OBJ)
            _ => 0,          // ELEM_HEAP_OBJ (Float, Str, etc.)
        }
    }

    /// Returns true if values of this type are heap-allocated and need GC tracing.
    /// Raw primitives (int, bool, float, None) don't need tracing because they're
    /// stored directly as i64 bit patterns, not as heap object pointers.
    pub(crate) fn type_needs_gc_trace(ty: &Type) -> bool {
        !matches!(ty, Type::Int | Type::Bool | Type::Float | Type::None)
    }

    /// Determine the elem_tag to pass to runtime for key functions.
    ///
    /// Returns the elem_tag value based on:
    /// - Whether the key function is a builtin wrapper (needs boxing) or user function
    /// - The element type of the container
    ///
    /// Builtin wrappers (rt_builtin_abs, rt_builtin_str, etc.) expect boxed *mut Obj
    /// arguments, so raw elements (like list[int]) need to be boxed before calling them.
    ///
    /// User-defined key functions are compiled with the appropriate parameter types
    /// matching the container element type, so they don't need boxing.
    pub(crate) fn elem_tag_for_key_func(key_func: &super::KeyFuncSource, elem_type: &Type) -> i64 {
        use super::KeyFuncSource;
        match key_func {
            KeyFuncSource::Builtin(_) => {
                // Builtin wrappers need boxing for raw element types
                Self::elem_tag_for_type(elem_type)
            }
            KeyFuncSource::UserFunc(..) => {
                // User functions work with raw values - no boxing needed
                0 // ELEM_HEAP_OBJ (no boxing)
            }
        }
    }

    /// Determine the elem_tag to pass to runtime for key functions (FuncOrBuiltin variant).
    ///
    /// Same logic as `elem_tag_for_key_func` but accepts `FuncOrBuiltin` type
    /// which is used in min/max lowering.
    pub(crate) fn elem_tag_for_func_or_builtin(
        func_or_builtin: &FuncOrBuiltin,
        elem_type: &Type,
    ) -> i64 {
        match func_or_builtin {
            FuncOrBuiltin::Builtin(_) => {
                // Builtin wrappers need boxing for raw element types
                Self::elem_tag_for_type(elem_type)
            }
            FuncOrBuiltin::UserFunc(_, _) => {
                // User functions work with raw values - no boxing needed
                0 // ELEM_HEAP_OBJ (no boxing)
            }
        }
    }
}
