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
    /// Get the current ambient source span, falling back to a dummy span.
    /// Safe to call in any expression-lowering context because `lower_expr()`
    /// always sets `self.codegen.current_span = Some(expr.span)` before dispatching.
    pub(crate) fn call_span(&self) -> pyaot_utils::Span {
        self.codegen
            .current_span
            .unwrap_or_else(pyaot_utils::Span::dummy)
    }

    /// Get the effective (offset-adjusted) VarId for a global variable
    pub(crate) fn get_effective_var_id(&self, var_id: pyaot_utils::VarId) -> i64 {
        (var_id.0 + self.modules.var_id_offset) as i64
    }

    /// Get the effective (offset-adjusted) ClassId for a class
    pub(crate) fn get_effective_class_id(&self, class_id: pyaot_utils::ClassId) -> i64 {
        (class_id.0 + self.modules.class_id_offset) as i64
    }

    /// Check if an expression is a variable that was narrowed from a Union type.
    /// This is used for `is`/`is not` comparisons where the variable still holds
    /// a boxed pointer even though the type has been narrowed.
    pub(crate) fn is_narrowed_union_var(&self, expr: &hir::Expr) -> bool {
        if let hir::ExprKind::Var(var_id) = &expr.kind {
            // Check if this variable is tracked in narrowed_union_vars
            // This tracks variables narrowed from Union to Int/Float/Bool/Str/None
            self.hir_types.narrowed_union_vars.contains_key(var_id)
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
        call_span: pyaot_utils::Span,
    ) -> pyaot_diagnostics::Result<()> {
        if args.len() != count {
            return Err(pyaot_diagnostics::CompilerError::type_error(
                format!(
                    "{func_name}() requires exactly {count} argument(s), got {}",
                    args.len()
                ),
                call_span,
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
        call_span: pyaot_utils::Span,
    ) -> pyaot_diagnostics::Result<()> {
        if args.len() < min {
            return Err(pyaot_diagnostics::CompilerError::type_error(
                format!(
                    "{func_name}() requires at least {min} argument(s), got {}",
                    args.len()
                ),
                call_span,
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
        let len_local = self.emit_runtime_call(len_func, vec![operand], Type::Int, mir_func);
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
        use crate::type_dispatch::{select_truthiness, TruthinessStrategy};

        match select_truthiness(operand_type) {
            TruthinessStrategy::AlreadyBool => operand,
            TruthinessStrategy::IntNotZero => {
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
            TruthinessStrategy::FloatNotZero => {
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
            TruthinessStrategy::LenBased(len_func) => {
                let result_local = self.alloc_and_add_local(Type::Bool, mir_func);
                self.emit_collection_bool_via_len(
                    mir::RuntimeFunc::Call(len_func),
                    operand,
                    result_local,
                    mir_func,
                );
                mir::Operand::Local(result_local)
            }
            TruthinessStrategy::AlwaysFalse => mir::Operand::Constant(mir::Constant::Bool(false)),
            TruthinessStrategy::RuntimeIsTruthy => {
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_IS_TRUTHY),
                    vec![operand],
                    Type::Bool,
                    mir_func,
                );
                mir::Operand::Local(result_local)
            }
            TruthinessStrategy::ClassInstance => {
                // For class instances, check for __bool__ and __len__ dunders.
                // Python truthiness rules: __bool__ takes priority over __len__.
                // If neither is defined, instances are always truthy (Python default).
                if let Type::Class { class_id, .. } = operand_type {
                    if let Some(class_info) = self.get_class_info(class_id).cloned() {
                        if let Some(bool_func_id) = class_info.get_dunder_func("__bool__") {
                            let result_local = self.alloc_and_add_local(Type::Bool, mir_func);
                            self.emit_instruction(mir::InstructionKind::CallDirect {
                                dest: result_local,
                                func: bool_func_id,
                                args: vec![operand],
                            });
                            return mir::Operand::Local(result_local);
                        } else if let Some(len_func_id) = class_info.get_dunder_func("__len__") {
                            let len_local = self.alloc_and_add_local(Type::Int, mir_func);
                            self.emit_instruction(mir::InstructionKind::CallDirect {
                                dest: len_local,
                                func: len_func_id,
                                args: vec![operand],
                            });
                            let result_local = self.alloc_and_add_local(Type::Bool, mir_func);
                            let zero = mir::Operand::Constant(mir::Constant::Int(0));
                            self.emit_instruction(mir::InstructionKind::BinOp {
                                dest: result_local,
                                op: mir::BinOp::NotEq,
                                left: mir::Operand::Local(len_local),
                                right: zero,
                            });
                            return mir::Operand::Local(result_local);
                        }
                    }
                }
                // No __bool__ or __len__: class instances are always truthy
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
            // int is NOT a subclass of bool: isinstance(int_val, bool) must return false
            (Type::Int, Type::Bool) => false,
            (Type::Str, Type::Str) => true,
            (Type::None, Type::None) => true,
            (Type::List(_), Type::List(_)) => true,
            (Type::Tuple(_), Type::Tuple(_)) => true,
            (Type::Dict(_, _), Type::Dict(_, _)) => true,
            (Type::Class { class_id: id1, .. }, Type::Class { class_id: id2, .. }) => id1 == id2,
            _ => false,
        }
    }

    /// Check whether an inferred type is compatible with an annotated type.
    ///
    /// This mirrors `Type::is_subtype_of()` but has access to lowering-time
    /// class hierarchy metadata, so annotation checks can accept subclasses
    /// inside unions and container element types.
    pub(crate) fn types_compatible_for_annotation(
        &self,
        actual: &Type,
        expected: &Type,
        hir_module: &hir::Module,
    ) -> bool {
        if let Type::Class { class_id, .. } = expected {
            if hir_module
                .class_defs
                .get(class_id)
                .is_some_and(|class_def| class_def.is_protocol)
            {
                return true;
            }
        }

        match (actual, expected) {
            (a, b) if a == b => true,
            (Type::Never, _) => true,
            (_, Type::Any | Type::HeapAny) => true,
            (Type::Bool, Type::Int) | (Type::Int, Type::Float) | (Type::Bool, Type::Float) => true,
            (Type::None, Type::Union(set)) if set.contains(&Type::None) => true,
            (Type::Union(left), right) => left
                .iter()
                .all(|member| self.types_compatible_for_annotation(member, right, hir_module)),
            (left, Type::Union(right)) => right
                .iter()
                .any(|member| self.types_compatible_for_annotation(left, member, hir_module)),
            (Type::List(a), Type::List(b)) | (Type::Set(a), Type::Set(b)) => {
                **a == Type::Any
                    || **b == Type::Any
                    || self.types_compatible_for_annotation(a, b, hir_module)
            }
            (Type::Dict(k1, v1), Type::Dict(k2, v2))
            | (Type::DefaultDict(k1, v1), Type::DefaultDict(k2, v2))
            | (Type::DefaultDict(k1, v1), Type::Dict(k2, v2)) => {
                (**k1 == Type::Any
                    || **k2 == Type::Any
                    || self.types_compatible_for_annotation(k1, k2, hir_module))
                    && (**v1 == Type::Any
                        || **v2 == Type::Any
                        || self.types_compatible_for_annotation(v1, v2, hir_module))
            }
            (Type::Tuple(ts1), Type::Tuple(ts2)) => {
                ts1.len() == ts2.len()
                    && ts1.iter().zip(ts2.iter()).all(|(t1, t2)| {
                        *t1 == Type::Any || self.types_compatible_for_annotation(t1, t2, hir_module)
                    })
            }
            (Type::Tuple(ts), Type::TupleVar(elem)) => ts.iter().all(|t| {
                *t == Type::Any || self.types_compatible_for_annotation(t, elem, hir_module)
            }),
            (Type::TupleVar(a), Type::TupleVar(b)) | (Type::Iterator(a), Type::Iterator(b)) => {
                **a == Type::Any || self.types_compatible_for_annotation(a, b, hir_module)
            }
            (
                Type::Function {
                    params: p1,
                    ret: r1,
                },
                Type::Function {
                    params: p2,
                    ret: r2,
                },
            ) => {
                p1.len() == p2.len()
                    && p2
                        .iter()
                        .zip(p1.iter())
                        .all(|(expected_param, actual_param)| {
                            self.types_compatible_for_annotation(
                                expected_param,
                                actual_param,
                                hir_module,
                            )
                        })
                    && self.types_compatible_for_annotation(r1, r2, hir_module)
            }
            (
                Type::Class {
                    class_id: actual_id,
                    ..
                },
                Type::Class {
                    class_id: expected_id,
                    ..
                },
            ) => actual_id == expected_id || self.is_proper_subclass(*actual_id, *expected_id),
            _ => actual.is_subtype_of(expected),
        }
    }

    /// Get the TypeTag value for a type (from core-defs single source of truth).
    /// Returns `None` for types that have no corresponding runtime type tag.
    pub(crate) fn get_type_tag_for_isinstance_check(&self, ty: &Type) -> Option<i64> {
        use pyaot_core_defs::TypeTagKind;
        match ty {
            Type::Int => Some(TypeTagKind::Int.tag() as i64),
            Type::Float => Some(TypeTagKind::Float.tag() as i64),
            Type::Bool => Some(TypeTagKind::Bool.tag() as i64),
            Type::Str => Some(TypeTagKind::Str.tag() as i64),
            Type::None => Some(TypeTagKind::None.tag() as i64),
            Type::List(_) => Some(TypeTagKind::List.tag() as i64),
            Type::Tuple(_) => Some(TypeTagKind::Tuple.tag() as i64),
            Type::Dict(_, _) => Some(TypeTagKind::Dict.tag() as i64),
            Type::Class { .. } => Some(TypeTagKind::Instance.tag() as i64),
            Type::Iterator(_) => Some(TypeTagKind::Iterator.tag() as i64),
            Type::Set(_) => Some(TypeTagKind::Set.tag() as i64),
            Type::Bytes => Some(TypeTagKind::Bytes.tag() as i64),
            Type::File(_) => Some(TypeTagKind::File.tag() as i64),
            _ => None, // Unknown type
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
                if let Some(func_id) = self.symbols.var_to_func.get(var_id) {
                    return Some((*func_id, Vec::new()));
                }
                // Check if it's a closure (with captures)
                if let Some((func_id, captures)) = self.closures.var_to_closure.get(var_id) {
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
    /// Returns ResolvedKeyFunc with function address + captures info, or None if no key function.
    /// Supports both user-defined functions and first-class builtins.
    pub(crate) fn emit_key_func_with_captures(
        &mut self,
        key_func: Option<&super::KeyFuncSource>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<Option<super::ResolvedKeyFunc>> {
        use super::KeyFuncSource;
        let Some(source) = key_func else {
            return Ok(None);
        };
        let key_fn_local = self.alloc_and_add_local(Type::Int, mir_func);
        let (captures_op, count_op) = match source {
            KeyFuncSource::UserFunc(func_id, captures) => {
                self.emit_instruction(mir::InstructionKind::FuncAddr {
                    dest: key_fn_local,
                    func: *func_id,
                });
                if captures.is_empty() {
                    (
                        mir::Operand::Constant(mir::Constant::Int(0)),
                        mir::Operand::Constant(mir::Constant::Int(0)),
                    )
                } else {
                    // Record capture types for the closure
                    if !self.has_closure_capture_types(func_id) {
                        let mut capture_types = Vec::new();
                        for capture_id in captures {
                            let capture_type = self.get_type_of_expr_id(*capture_id, hir_module);
                            capture_types.push(capture_type);
                        }
                        self.insert_closure_capture_types(*func_id, capture_types);
                    }
                    let captures_tuple =
                        self.lower_captures_to_tuple(captures, hir_module, mir_func)?;
                    let count = captures.len() as i64;
                    (
                        captures_tuple,
                        mir::Operand::Constant(mir::Constant::Int(count)),
                    )
                }
            }
            KeyFuncSource::Builtin(builtin_kind) => {
                self.emit_instruction(mir::InstructionKind::BuiltinAddr {
                    dest: key_fn_local,
                    builtin: *builtin_kind,
                });
                (
                    mir::Operand::Constant(mir::Constant::Int(0)),
                    mir::Operand::Constant(mir::Constant::Int(0)),
                )
            }
        };
        Ok(Some(super::ResolvedKeyFunc {
            func_addr: mir::Operand::Local(key_fn_local),
            captures: captures_op,
            capture_count: count_op,
        }))
    }

    /// Shared helper for split/rsplit methods (used by both str and bytes).
    pub(crate) fn lower_split_variant_impl(
        &mut self,
        obj_operand: mir::Operand,
        arg_operands: Vec<mir::Operand>,
        runtime_func: mir::RuntimeFunc,
        elem_type: Type,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // sep argument - use null pointer (0) to signal split on whitespace
        let sep_operand = arg_operands
            .first()
            .cloned()
            .unwrap_or(mir::Operand::Constant(mir::Constant::Int(0)));

        // maxsplit argument (-1 = no limit)
        let maxsplit_operand = arg_operands
            .get(1)
            .cloned()
            .unwrap_or(mir::Operand::Constant(mir::Constant::Int(-1)));

        let result_local = self.emit_runtime_call(
            runtime_func,
            vec![obj_operand, sep_operand, maxsplit_operand],
            Type::List(Box::new(elem_type)),
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Shared helper for join methods (used by both str and bytes).
    pub(crate) fn lower_join_impl(
        &mut self,
        obj_operand: mir::Operand,
        arg_operands: Vec<mir::Operand>,
        runtime_func: mir::RuntimeFunc,
        result_type: Type,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if arg_operands.is_empty() {
            return Ok(mir::Operand::Constant(mir::Constant::None));
        }

        let result_local = self.emit_runtime_call(
            runtime_func,
            vec![obj_operand, arg_operands[0].clone()],
            result_type,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
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
                crate::type_dispatch::elem_tag_for_type(elem_type)
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
                crate::type_dispatch::elem_tag_for_type(elem_type)
            }
            FuncOrBuiltin::UserFunc(_, _) => {
                // User functions work with raw values - no boxing needed
                0 // ELEM_HEAP_OBJ (no boxing)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::LoweredClassInfo;
    use indexmap::IndexMap;
    use pyaot_utils::{ClassId, StringInterner};

    fn stub_class_info(base_class: Option<ClassId>) -> LoweredClassInfo {
        LoweredClassInfo {
            field_offsets: IndexMap::new(),
            field_types: IndexMap::new(),
            method_funcs: IndexMap::new(),
            init_func: None,
            dunder_methods: IndexMap::new(),
            base_class,
            total_field_count: 0,
            own_field_offset: 0,
            vtable_slots: IndexMap::new(),
            class_attr_offsets: IndexMap::new(),
            class_attr_types: IndexMap::new(),
            static_methods: IndexMap::new(),
            class_methods: IndexMap::new(),
            properties: IndexMap::new(),
            property_types: IndexMap::new(),
            is_exception_class: false,
        }
    }

    #[test]
    fn annotation_compatibility_accepts_union_of_subclasses_for_base() {
        let mut interner = StringInterner::default();
        let mut lowering = Lowering::new(&mut interner);
        let module_name = lowering.interner.intern("compat_test");
        let module = hir::Module::new(module_name);

        let shape_id = ClassId::new(0);
        let circle_id = ClassId::new(1);
        let square_id = ClassId::new(2);
        let shape_name = lowering.interner.intern("Shape");
        let circle_name = lowering.interner.intern("Circle");
        let square_name = lowering.interner.intern("Square");

        lowering
            .classes
            .class_info
            .insert(shape_id, stub_class_info(None));
        lowering
            .classes
            .class_info
            .insert(circle_id, stub_class_info(Some(shape_id)));
        lowering
            .classes
            .class_info
            .insert(square_id, stub_class_info(Some(shape_id)));

        let actual = Type::Union(vec![
            Type::Class {
                class_id: circle_id,
                name: circle_name,
            },
            Type::Class {
                class_id: square_id,
                name: square_name,
            },
        ]);
        let expected = Type::Class {
            class_id: shape_id,
            name: shape_name,
        };

        assert!(lowering.types_compatible_for_annotation(&actual, &expected, &module));
    }

    #[test]
    fn annotation_compatibility_accepts_container_of_subclass_for_base() {
        let mut interner = StringInterner::default();
        let mut lowering = Lowering::new(&mut interner);
        let module_name = lowering.interner.intern("compat_test");
        let module = hir::Module::new(module_name);

        let shape_id = ClassId::new(0);
        let circle_id = ClassId::new(1);
        let shape_name = lowering.interner.intern("Shape");
        let circle_name = lowering.interner.intern("Circle");

        lowering
            .classes
            .class_info
            .insert(shape_id, stub_class_info(None));
        lowering
            .classes
            .class_info
            .insert(circle_id, stub_class_info(Some(shape_id)));

        let actual = Type::List(Box::new(Type::Class {
            class_id: circle_id,
            name: circle_name,
        }));
        let expected = Type::List(Box::new(Type::Class {
            class_id: shape_id,
            name: shape_name,
        }));

        assert!(lowering.types_compatible_for_annotation(&actual, &expected, &module));
    }
}
