//! Type narrowing for isinstance() checks and identity comparisons
//!
//! This module provides type narrowing capabilities after isinstance() checks
//! and `is None`/`is not None` comparisons, allowing the compiler to generate
//! more efficient code by leveraging runtime type information.
//!
//! Example:
//! ```python
//! x: int | str = get_value()
//! if isinstance(x, int):
//!     print(x + 1)      # Use int addition, not generic dispatch
//! else:
//!     print(x.upper())  # Use str method directly
//!
//! y: int | None = maybe_get_int()
//! if y is not None:
//!     print(y + 1)      # y is narrowed to int
//! ```

use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{Span, VarId};

use crate::context::Lowering;

/// Indicates which branch of an if statement is dead (unreachable)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadBranch {
    /// The then-branch is dead (isinstance always returns False)
    ThenBranch,
    /// The else-branch is dead (isinstance always returns True)
    ElseBranch,
}

/// Information extracted from an isinstance() call for narrowing and dead code detection
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct IsinstanceInfo {
    /// The variable being checked
    pub var_id: VarId,
    /// The original type of the variable before narrowing
    pub original_type: Type,
    /// The type being checked against
    pub checked_type: Type,
    /// If set, indicates which branch is dead (unreachable)
    pub dead_branch: Option<DeadBranch>,
    /// The span of the isinstance expression (for warning location)
    pub span: Span,
    /// Variable name (for better error messages), if available
    pub var_name: Option<String>,
}

/// Information about a single type narrowing from an isinstance() check
#[derive(Debug, Clone)]
pub struct TypeNarrowingInfo {
    /// The variable being narrowed
    pub var_id: VarId,
    /// The narrowed type (after applying the isinstance check)
    pub narrowed_type: Type,
    /// The original type (before narrowing)
    pub original_type: Type,
}

/// Result of analyzing an if-condition for type narrowing opportunities
#[derive(Debug, Default)]
pub struct NarrowingAnalysis {
    /// Narrowings to apply in the then-branch (isinstance is true)
    pub then_narrowings: Vec<TypeNarrowingInfo>,
    /// Narrowings to apply in the else-branch (isinstance is false)
    pub else_narrowings: Vec<TypeNarrowingInfo>,
}

impl<'a> Lowering<'a> {
    fn narrowing_input_type(&self, var_id: &VarId) -> Option<Type> {
        self.get_var_type(var_id)
            .cloned()
            .and_then(|ty| {
                if matches!(ty, Type::Any | Type::HeapAny) {
                    self.get_base_var_type(var_id).cloned().or(Some(ty))
                } else {
                    Some(ty)
                }
            })
            .or_else(|| self.get_base_var_type(var_id).cloned())
    }

    /// Apply a block's incoming narrowings by materializing explicit MIR defs
    /// at block entry and updating the lowering-time type view for HIR queries.
    pub(crate) fn enter_cfg_block_narrowings(
        &mut self,
        narrowings: &[TypeNarrowingInfo],
        mir_func: &mut mir::Function,
    ) {
        self.clear_block_narrowed_locals();

        for info in narrowings {
            if matches!(info.narrowed_type, Type::Never) {
                continue;
            }
            let narrowed_local = self.materialize_block_narrowing_local(info, mir_func);
            self.insert_block_narrowed_local(
                info.var_id,
                narrowed_local,
                info.original_type.clone(),
                info.narrowed_type.clone(),
            );
        }
    }

    /// Restore the pre-branch type view and drop any block-local narrowing defs.
    pub(crate) fn leave_cfg_block_narrowings(&mut self) {
        self.clear_block_narrowed_locals();
    }

    fn materialize_block_narrowing_local(
        &mut self,
        info: &TypeNarrowingInfo,
        mir_func: &mut mir::Function,
    ) -> pyaot_utils::LocalId {
        let src = self.narrowing_source_operand(info, mir_func);
        self.materialize_narrowed_local_from_operand(
            src,
            &info.original_type,
            &info.narrowed_type,
            mir_func,
        )
    }

    pub(crate) fn materialize_narrowed_local_from_operand(
        &mut self,
        src: mir::Operand,
        original_type: &Type,
        narrowed_type: &Type,
        mir_func: &mut mir::Function,
    ) -> pyaot_utils::LocalId {
        if Self::narrowing_requires_unbox(original_type, narrowed_type) {
            if let Some(unbox_func) = Self::unbox_func_for_type(narrowed_type) {
                let unboxed =
                    self.emit_runtime_call(unbox_func, vec![src], narrowed_type.clone(), mir_func);
                let dest = self.alloc_and_add_local(narrowed_type.clone(), mir_func);
                self.emit_instruction(mir::InstructionKind::Refine {
                    dest,
                    src: mir::Operand::Local(unboxed),
                    ty: narrowed_type.clone(),
                });
                return dest;
            }
        }

        let dest = self.alloc_and_add_local(narrowed_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::Refine {
            dest,
            src,
            ty: narrowed_type.clone(),
        });
        dest
    }

    fn narrowing_requires_unbox(original_type: &Type, narrowed_type: &Type) -> bool {
        matches!(original_type, Type::Union(_) | Type::Any | Type::HeapAny)
            && matches!(narrowed_type, Type::Int | Type::Float | Type::Bool)
    }

    fn narrowing_source_operand(
        &mut self,
        info: &TypeNarrowingInfo,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        if self.is_global(&info.var_id) {
            let effective_var_id = self.get_effective_var_id(info.var_id);
            let get_func = self.get_global_get_func(&info.original_type);
            let local = self.emit_runtime_call(
                get_func,
                vec![mir::Operand::Constant(mir::Constant::Int(effective_var_id))],
                info.original_type.clone(),
                mir_func,
            );
            return mir::Operand::Local(local);
        }

        if let Some(cell_local) = self.get_nonlocal_cell(&info.var_id) {
            let get_func = self.get_cell_get_func(&info.original_type);
            let local = self.emit_runtime_call(
                get_func,
                vec![mir::Operand::Local(cell_local)],
                info.original_type.clone(),
                mir_func,
            );
            return mir::Operand::Local(local);
        }

        let local = self.get_or_create_local(info.var_id, info.original_type.clone(), mir_func);
        mir::Operand::Local(local)
    }

    /// Analyze a condition expression for type narrowing opportunities.
    /// Returns narrowings for both then-branch and else-branch.
    pub(crate) fn analyze_condition_for_narrowing(
        &self,
        cond_expr: &hir::Expr,
        hir_module: &hir::Module,
    ) -> NarrowingAnalysis {
        let mut analysis = NarrowingAnalysis::default();
        self.extract_narrowing_info(cond_expr, hir_module, false, &mut analysis);
        analysis
    }

    /// Recursively extract narrowing information from a condition expression.
    /// `negated` indicates if we're inside a `not` expression.
    fn extract_narrowing_info(
        &self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        negated: bool,
        analysis: &mut NarrowingAnalysis,
    ) {
        match &expr.kind {
            // isinstance(x, T) - direct isinstance call
            hir::ExprKind::BuiltinCall {
                builtin: hir::Builtin::Isinstance,
                args,
                ..
            } => {
                if let Some(info) = self.extract_isinstance_info(args, hir_module) {
                    if negated {
                        // not isinstance(x, T): then-branch gets excluding, else-branch gets narrowed
                        analysis.then_narrowings.push(TypeNarrowingInfo {
                            var_id: info.var_id,
                            narrowed_type: info.original_type.narrow_excluding(&info.checked_type),
                            original_type: info.original_type.clone(),
                        });
                        analysis.else_narrowings.push(TypeNarrowingInfo {
                            var_id: info.var_id,
                            narrowed_type: info.original_type.narrow_to(&info.checked_type),
                            original_type: info.original_type,
                        });
                    } else {
                        // isinstance(x, T): then-branch gets narrowed, else-branch gets excluding
                        analysis.then_narrowings.push(TypeNarrowingInfo {
                            var_id: info.var_id,
                            narrowed_type: info.original_type.narrow_to(&info.checked_type),
                            original_type: info.original_type.clone(),
                        });
                        analysis.else_narrowings.push(TypeNarrowingInfo {
                            var_id: info.var_id,
                            narrowed_type: info.original_type.narrow_excluding(&info.checked_type),
                            original_type: info.original_type,
                        });
                    }
                }
            }

            // x is None / x is not None - identity comparison with None
            hir::ExprKind::Compare {
                left,
                op: op @ (hir::CmpOp::Is | hir::CmpOp::IsNot),
                right,
            } => {
                let left_expr = &hir_module.exprs[*left];
                let right_expr = &hir_module.exprs[*right];

                // Determine which side is None and which is a variable
                let (var_id, is_none_on_right) = match (&left_expr.kind, &right_expr.kind) {
                    (hir::ExprKind::Var(var_id), hir::ExprKind::None) => (Some(*var_id), true),
                    (hir::ExprKind::None, hir::ExprKind::Var(var_id)) => (Some(*var_id), false),
                    _ => (None, false),
                };

                // Only narrow if we have a variable and can get its type
                if let Some(var_id) = var_id {
                    if let Some(original_type) = self.narrowing_input_type(&var_id) {
                        // Only narrow if the original type contains None (i.e., is Optional)
                        if self.type_contains_none(&original_type) {
                            let _ = is_none_on_right; // Both orderings are equivalent

                            // Determine if this is effectively `is None` or `is not None`
                            // taking into account potential outer negation
                            let is_none_check = matches!(op, hir::CmpOp::Is);
                            let effective_is_none = is_none_check != negated;

                            if effective_is_none {
                                // `x is None` (or `not (x is not None)`)
                                // then-branch: x is None
                                // else-branch: x is not None (exclude None)
                                analysis.then_narrowings.push(TypeNarrowingInfo {
                                    var_id,
                                    narrowed_type: Type::None,
                                    original_type: original_type.clone(),
                                });
                                analysis.else_narrowings.push(TypeNarrowingInfo {
                                    var_id,
                                    narrowed_type: original_type.narrow_excluding(&Type::None),
                                    original_type,
                                });
                            } else {
                                // `x is not None` (or `not (x is None)`)
                                // then-branch: x is not None (exclude None)
                                // else-branch: x is None
                                analysis.then_narrowings.push(TypeNarrowingInfo {
                                    var_id,
                                    narrowed_type: original_type.narrow_excluding(&Type::None),
                                    original_type: original_type.clone(),
                                });
                                analysis.else_narrowings.push(TypeNarrowingInfo {
                                    var_id,
                                    narrowed_type: Type::None,
                                    original_type,
                                });
                            }
                        }
                    }
                }
            }

            // not <expr> - negate the condition
            hir::ExprKind::UnOp {
                op: hir::UnOp::Not,
                operand,
            } => {
                let inner_expr = &hir_module.exprs[*operand];
                self.extract_narrowing_info(inner_expr, hir_module, !negated, analysis);
            }

            // <expr> and <expr> - conjunction (both must be true in then-branch)
            hir::ExprKind::LogicalOp {
                op: hir::LogicalOp::And,
                left,
                right,
            } if !negated => {
                // For `a and b` in then-branch: both a and b are true
                // For else-branch: at least one is false (we can't safely narrow)
                let left_expr = &hir_module.exprs[*left];
                let right_expr = &hir_module.exprs[*right];

                // Collect narrowings from both sides for then-branch
                let mut left_analysis = NarrowingAnalysis::default();
                self.extract_narrowing_info(left_expr, hir_module, false, &mut left_analysis);

                let mut right_analysis = NarrowingAnalysis::default();
                self.extract_narrowing_info(right_expr, hir_module, false, &mut right_analysis);

                // Merge then-narrowings (both are applied)
                analysis
                    .then_narrowings
                    .extend(left_analysis.then_narrowings);
                analysis
                    .then_narrowings
                    .extend(right_analysis.then_narrowings);

                // For else-branch of `a and b`: we can't safely narrow because
                // either a or b (or both) could be false. Skip else narrowings.
            }

            // <expr> or <expr> - disjunction (at least one must be true in then-branch)
            hir::ExprKind::LogicalOp {
                op: hir::LogicalOp::Or,
                left,
                right,
            } if !negated => {
                // For `a or b` in then-branch: at least one is true (can't safely narrow)
                // For else-branch: both are false (both exclusionary narrowings apply)
                let left_expr = &hir_module.exprs[*left];
                let right_expr = &hir_module.exprs[*right];

                let mut left_analysis = NarrowingAnalysis::default();
                self.extract_narrowing_info(left_expr, hir_module, false, &mut left_analysis);

                let mut right_analysis = NarrowingAnalysis::default();
                self.extract_narrowing_info(right_expr, hir_module, false, &mut right_analysis);

                // Merge else-narrowings (both exclusions apply when 'or' is false)
                analysis
                    .else_narrowings
                    .extend(left_analysis.else_narrowings);
                analysis
                    .else_narrowings
                    .extend(right_analysis.else_narrowings);
            }

            // Negated 'and' (not (a and b)) - De Morgan: becomes (not a) or (not b)
            hir::ExprKind::LogicalOp {
                op: hir::LogicalOp::And,
                left,
                right,
            } if negated => {
                // In then-branch: at least one is false (can't safely narrow)
                // In else-branch: `not (a and b)` is false, so `a and b` is true
                let left_expr = &hir_module.exprs[*left];
                let right_expr = &hir_module.exprs[*right];

                let mut left_analysis = NarrowingAnalysis::default();
                self.extract_narrowing_info(left_expr, hir_module, false, &mut left_analysis);

                let mut right_analysis = NarrowingAnalysis::default();
                self.extract_narrowing_info(right_expr, hir_module, false, &mut right_analysis);

                // Merge then-narrowings for the else-branch
                analysis
                    .else_narrowings
                    .extend(left_analysis.then_narrowings);
                analysis
                    .else_narrowings
                    .extend(right_analysis.then_narrowings);
            }

            // Negated 'or' (not (a or b)) - De Morgan: becomes (not a) and (not b)
            hir::ExprKind::LogicalOp {
                op: hir::LogicalOp::Or,
                left,
                right,
            } if negated => {
                // In then-branch: `not (a or b)` is true, so `a or b` is false (both are false)
                // In else-branch: at least one is true (can't safely narrow)
                let left_expr = &hir_module.exprs[*left];
                let right_expr = &hir_module.exprs[*right];

                let mut left_analysis = NarrowingAnalysis::default();
                self.extract_narrowing_info(left_expr, hir_module, false, &mut left_analysis);

                let mut right_analysis = NarrowingAnalysis::default();
                self.extract_narrowing_info(right_expr, hir_module, false, &mut right_analysis);

                // Merge else-narrowings (exclusions) for the then-branch
                analysis
                    .then_narrowings
                    .extend(left_analysis.else_narrowings);
                analysis
                    .then_narrowings
                    .extend(right_analysis.else_narrowings);
            }

            // Bare variable condition: if x: where x is Optional[T]
            // For truthiness narrowing, we can exclude None in the then-branch
            // because None is always falsy
            hir::ExprKind::Var(var_id) => {
                if let Some(original_type) = self.narrowing_input_type(var_id) {
                    // Only narrow if the original type contains None (i.e., is Optional)
                    if self.type_contains_none(&original_type) {
                        if negated {
                            // `not x` where x: T | None
                            // then-branch: x is falsy (could be None or falsy T value)
                            // We can safely narrow to None in then-branch only for types
                            // where None is the only falsy value. For safety, we only
                            // narrow the else-branch (where x is truthy, so not None).
                            analysis.else_narrowings.push(TypeNarrowingInfo {
                                var_id: *var_id,
                                narrowed_type: original_type.narrow_excluding(&Type::None),
                                original_type,
                            });
                        } else {
                            // `if x:` where x: T | None
                            // then-branch: x is truthy, so x is not None
                            // else-branch: x is falsy (could be None or falsy T value)
                            // We only narrow the then-branch because we know x is not None there
                            analysis.then_narrowings.push(TypeNarrowingInfo {
                                var_id: *var_id,
                                narrowed_type: original_type.narrow_excluding(&Type::None),
                                original_type,
                            });
                            // Note: We don't narrow else-branch because x could be 0, "", [], etc.
                        }
                    }
                }
            }

            _ => {
                // Unknown pattern - no narrowing
            }
        }
    }

    /// Extract isinstance information from the arguments of an isinstance call.
    /// Returns the var_id, original type, the type being checked, and dead branch info.
    fn extract_isinstance_info(
        &self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
    ) -> Option<IsinstanceInfo> {
        if args.len() < 2 {
            return None;
        }

        let obj_expr = &hir_module.exprs[args[0]];
        let type_expr = &hir_module.exprs[args[1]];

        // First argument must be a variable
        let var_id = match &obj_expr.kind {
            hir::ExprKind::Var(var_id) => *var_id,
            _ => return None, // Can't narrow non-variables
        };

        // Get the original type of the variable
        let original_type = self.narrowing_input_type(&var_id)?;

        // Second argument must be a type reference
        let checked_type = match &type_expr.kind {
            hir::ExprKind::TypeRef(ty) => ty.clone(),
            hir::ExprKind::ClassRef(class_id) => {
                // Get class name for the type
                if let Some(class_def) = hir_module.class_defs.get(class_id) {
                    Type::Class {
                        class_id: *class_id,
                        name: class_def.name,
                    }
                } else {
                    return None;
                }
            }
            _ => return None, // Unknown type expression
        };

        // Perform narrowing to detect contradictions (dead code)
        let then_type = original_type.narrow_to(&checked_type);
        let else_type = original_type.narrow_excluding(&checked_type);

        // Class inheritance blindness: `Type::narrow_to` uses a structural
        // match and cannot see that a Union member may be a subclass of the
        // checked class. Before flagging the then-branch as dead, ask the
        // class registry. `isinstance(other, Base)` where `other: Derived|int`
        // must NOT be marked unreachable.
        let class_inheritance_might_match = if let Type::Class {
            class_id: target_id,
            ..
        } = &checked_type
        {
            self.union_contains_subclass_of(&original_type, *target_id)
        } else {
            false
        };

        // Detect dead branches based on narrowing results
        let dead_branch = if matches!(original_type, Type::Union(_)) {
            // For Union types, check if narrowing produces Type::Never
            if matches!(then_type, Type::Never) && !class_inheritance_might_match {
                Some(DeadBranch::ThenBranch)
            } else if matches!(else_type, Type::Never) {
                Some(DeadBranch::ElseBranch)
            } else {
                None
            }
        } else if matches!(original_type, Type::Any | Type::HeapAny) {
            // Any/HeapAny can hold any runtime type — both branches are live
            None
        } else {
            // For concrete non-Union types, check direct type compatibility
            // isinstance(x: int, str) is always False (then-branch dead)
            // isinstance(x: str, str) is always True (else-branch dead)
            if self.types_compatible_for_isinstance_full(&original_type, &checked_type) {
                Some(DeadBranch::ElseBranch)
            } else {
                Some(DeadBranch::ThenBranch)
            }
        };

        // For narrowing purposes, only return info for Union/Any types
        // Concrete non-Union types don't benefit from narrowing but we still detect dead code
        if !matches!(original_type, Type::Union(_) | Type::Any | Type::HeapAny)
            && dead_branch.is_none()
        {
            return None;
        }

        Some(IsinstanceInfo {
            var_id,
            original_type,
            checked_type,
            dead_branch,
            span: obj_expr.span, // Use the variable expression's span
            var_name: None,      // Will be filled in by caller if needed
        })
    }

    /// Check if a type contains None (i.e., is Optional or Union including None)
    fn type_contains_none(&self, ty: &Type) -> bool {
        match ty {
            Type::None => true,
            Type::Union(types) => types.iter().any(|t| matches!(t, Type::None)),
            _ => false,
        }
    }

    /// Check if two types are compatible for isinstance purposes (primitive/container types).
    /// Returns true if isinstance(value_of_type_a, type_b) would always return True.
    /// This static method handles all non-class types; for class inheritance, use the
    /// instance method `types_compatible_for_isinstance_full`.
    fn types_compatible_for_isinstance(actual: &Type, target: &Type) -> bool {
        match (actual, target) {
            // Exact matches
            (Type::Int, Type::Int) => true,
            (Type::Float, Type::Float) => true,
            (Type::Bool, Type::Bool) => true,
            (Type::Bool, Type::Int) => true, // bool is a subclass of int in Python
            (Type::Str, Type::Str) => true,
            (Type::Bytes, Type::Bytes) => true,
            (Type::None, Type::None) => true,
            // Container types match by kind
            (Type::List(_), Type::List(_)) => true,
            (Type::Dict(_, _), Type::Dict(_, _)) => true,
            (Type::Set(_), Type::Set(_)) => true,
            (Type::Tuple(_), Type::Tuple(_)) => true,
            // Class types: exact match only (inheritance checked separately)
            (Type::Class { class_id: id1, .. }, Type::Class { class_id: id2, .. }) => id1 == id2,
            // Everything else is incompatible
            _ => false,
        }
    }

    /// Returns `true` iff `ty` is a Union that contains at least one Class
    /// member that is a subclass of `target_id` (or equal to it). Used to
    /// suppress false-positive "unreachable code" warnings from
    /// `isinstance(x, Base)` when `x: Derived | ...`.
    fn union_contains_subclass_of(&self, ty: &Type, target_id: pyaot_utils::ClassId) -> bool {
        let Type::Union(members) = ty else {
            return false;
        };
        for m in members {
            if let Type::Class { class_id, .. } = m {
                if *class_id == target_id || self.is_proper_subclass(*class_id, target_id) {
                    return true;
                }
            }
        }
        false
    }

    /// Full isinstance compatibility check including class inheritance.
    /// Walks the class hierarchy to determine if `actual` is a subclass of `target`.
    fn types_compatible_for_isinstance_full(&self, actual: &Type, target: &Type) -> bool {
        if Self::types_compatible_for_isinstance(actual, target) {
            return true;
        }
        // Class inheritance: delegate to the shared walk.
        if let (Type::Class { class_id: id1, .. }, Type::Class { class_id: id2, .. }) =
            (actual, target)
        {
            return self.is_proper_subclass(*id1, *id2);
        }
        false
    }

    /// Extract isinstance information from an isinstance expression for dead code detection.
    /// This is a public wrapper for use by control_flow.rs.
    #[allow(dead_code)]
    pub(crate) fn extract_isinstance_info_from_expr(
        &self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
    ) -> Option<IsinstanceInfo> {
        match &expr.kind {
            hir::ExprKind::BuiltinCall {
                builtin: hir::Builtin::Isinstance,
                args,
                ..
            } => {
                let mut info = self.extract_isinstance_info(args, hir_module)?;
                // Use the isinstance expression's span for better error location
                info.span = expr.span;
                Some(info)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyaot_mir::InstructionKind;
    use pyaot_utils::{ClassId, LocalId, StringInterner};

    #[test]
    fn test_narrowing_analysis_default() {
        let analysis = NarrowingAnalysis::default();
        assert!(analysis.then_narrowings.is_empty());
        assert!(analysis.else_narrowings.is_empty());
    }

    #[test]
    fn test_dead_branch_enum() {
        // Test that DeadBranch enum variants exist and compare correctly
        let then_dead = DeadBranch::ThenBranch;
        let else_dead = DeadBranch::ElseBranch;
        assert_ne!(then_dead, else_dead);
        assert_eq!(then_dead, DeadBranch::ThenBranch);
        assert_eq!(else_dead, DeadBranch::ElseBranch);
    }

    #[test]
    fn test_types_compatible_for_isinstance() {
        // Test exact type matches
        assert!(Lowering::types_compatible_for_isinstance(
            &Type::Int,
            &Type::Int
        ));
        assert!(Lowering::types_compatible_for_isinstance(
            &Type::Str,
            &Type::Str
        ));
        assert!(Lowering::types_compatible_for_isinstance(
            &Type::Float,
            &Type::Float
        ));
        assert!(Lowering::types_compatible_for_isinstance(
            &Type::Bool,
            &Type::Bool
        ));
        assert!(Lowering::types_compatible_for_isinstance(
            &Type::None,
            &Type::None
        ));

        // Test bool is subtype of int (Python semantics)
        assert!(Lowering::types_compatible_for_isinstance(
            &Type::Bool,
            &Type::Int
        ));

        // Test incompatible types
        assert!(!Lowering::types_compatible_for_isinstance(
            &Type::Int,
            &Type::Str
        ));
        assert!(!Lowering::types_compatible_for_isinstance(
            &Type::Str,
            &Type::Int
        ));
        assert!(!Lowering::types_compatible_for_isinstance(
            &Type::Float,
            &Type::Str
        ));
        assert!(!Lowering::types_compatible_for_isinstance(
            &Type::Int,
            &Type::Float
        ));
        assert!(!Lowering::types_compatible_for_isinstance(
            &Type::Int,
            &Type::None
        ));

        // Test container types match by kind
        assert!(Lowering::types_compatible_for_isinstance(
            &Type::List(Box::new(Type::Int)),
            &Type::List(Box::new(Type::Any))
        ));
        assert!(Lowering::types_compatible_for_isinstance(
            &Type::Dict(Box::new(Type::Str), Box::new(Type::Int)),
            &Type::Dict(Box::new(Type::Any), Box::new(Type::Any))
        ));
    }

    #[test]
    fn test_isinstance_info_struct() {
        // Test that IsinstanceInfo can be created with all fields
        let info = IsinstanceInfo {
            var_id: VarId::new(0),
            original_type: Type::Int,
            checked_type: Type::Str,
            dead_branch: Some(DeadBranch::ThenBranch),
            span: Span::dummy(),
            var_name: Some("test_var".to_string()),
        };

        assert_eq!(info.var_id, VarId::new(0));
        assert_eq!(info.original_type, Type::Int);
        assert_eq!(info.checked_type, Type::Str);
        assert_eq!(info.dead_branch, Some(DeadBranch::ThenBranch));
        assert_eq!(info.var_name, Some("test_var".to_string()));
    }

    #[test]
    fn test_type_narrow_excluding_none() {
        // Test narrow_excluding for None types
        // Union[int, None] excluding None -> int
        let optional_int = Type::Union(vec![Type::Int, Type::None]);
        let narrowed = optional_int.narrow_excluding(&Type::None);
        assert_eq!(narrowed, Type::Int);

        // Union[int, str, None] excluding None -> Union[int, str]
        let triple_union = Type::Union(vec![Type::Int, Type::Str, Type::None]);
        let narrowed = triple_union.narrow_excluding(&Type::None);
        assert_eq!(narrowed, Type::Union(vec![Type::Int, Type::Str]));

        // None excluding None -> Never (bottom type)
        let narrowed = Type::None.narrow_excluding(&Type::None);
        assert_eq!(narrowed, Type::Never);
    }

    #[test]
    fn test_type_narrow_to_none() {
        // Test narrow_to for None types
        // Union[int, None] narrowed to None -> None
        let optional_int = Type::Union(vec![Type::Int, Type::None]);
        let narrowed = optional_int.narrow_to(&Type::None);
        assert_eq!(narrowed, Type::None);

        // int narrowed to None -> Never (no match, unreachable code)
        // This is correct because isinstance(int_value, type(None)) is always False
        let narrowed = Type::Int.narrow_to(&Type::None);
        assert_eq!(narrowed, Type::Never);
    }

    #[test]
    fn test_type_contains_none() {
        // None type contains None
        let none_type = Type::None;
        assert!(matches!(none_type, Type::None));

        // Union with None contains None
        let optional_int = Type::Union(vec![Type::Int, Type::None]);
        let has_none = optional_int
            .clone()
            .is_subtype_of(&Type::Union(vec![Type::Int, Type::None]));
        assert!(has_none);

        // We can verify None is in a Union by checking narrow_to
        let narrowed = optional_int.narrow_to(&Type::None);
        assert_eq!(narrowed, Type::None);

        // Union without None - narrowing to None returns Never (unreachable)
        // This is correct because isinstance(x, type(None)) is always False for int|str
        let int_or_str = Type::Union(vec![Type::Int, Type::Str]);
        let narrowed = int_or_str.narrow_to(&Type::None);
        assert_eq!(narrowed, Type::Never); // No None to narrow to, so unreachable
    }

    #[test]
    fn test_only_falsy_is_none_for_primitives() {
        // Test that primitives with falsy values are correctly identified
        // These types have falsy values other than None:
        // - int | None: 0 is falsy
        // - str | None: "" is falsy
        // - list | None: [] is falsy
        // - bool | None: False is falsy

        // We can't directly test only_falsy_is_none without a Lowering context,
        // but we can verify the type behavior through narrow_excluding

        // Union[int, None] - int has 0 as falsy
        let optional_int = Type::Union(vec![Type::Int, Type::None]);
        let narrowed = optional_int.narrow_excluding(&Type::None);
        assert_eq!(narrowed, Type::Int);

        // Union[str, None] - str has "" as falsy
        let optional_str = Type::Union(vec![Type::Str, Type::None]);
        let narrowed = optional_str.narrow_excluding(&Type::None);
        assert_eq!(narrowed, Type::Str);

        // Union[list[int], None] - list has [] as falsy
        let optional_list = Type::Union(vec![Type::List(Box::new(Type::Int)), Type::None]);
        let narrowed = optional_list.narrow_excluding(&Type::None);
        assert_eq!(narrowed, Type::List(Box::new(Type::Int)));
    }

    #[test]
    fn cfg_block_narrowing_materializes_unbox_and_refine_for_primitive_union_payloads() {
        let mut interner = StringInterner::default();
        let mut lowering = Lowering::new(&mut interner);
        let mut mir_func = mir::Function::new(
            pyaot_utils::FuncId::from(0u32),
            "f".to_string(),
            Vec::new(),
            Type::None,
            None,
        );
        let block = lowering.new_block();
        lowering.push_block(block);

        let var_id = VarId::new(0);
        let union_ty = Type::Union(vec![Type::Int, Type::Str]);
        let base_local = LocalId::from(0u32);
        mir_func.add_local(mir::Local {
            id: base_local,
            name: None,
            ty: union_ty.clone(),
            is_gc_root: true,
        });
        lowering.insert_var_local(var_id, base_local);
        lowering.insert_var_type(var_id, union_ty.clone());

        lowering.enter_cfg_block_narrowings(
            &[TypeNarrowingInfo {
                var_id,
                narrowed_type: Type::Int,
                original_type: union_ty.clone(),
            }],
            &mut mir_func,
        );

        let instructions = lowering.current_block_mut().instructions.clone();
        assert_eq!(instructions.len(), 2);
        match &instructions[0].kind {
            InstructionKind::RuntimeCall { func, .. } => match func {
                mir::RuntimeFunc::Call(def) => {
                    assert!(std::ptr::eq(
                        *def,
                        &pyaot_core_defs::runtime_func_def::RT_UNBOX_INT
                    ));
                }
                other => panic!("expected runtime unbox call, got {other:?}"),
            },
            other => panic!("expected RuntimeCall, got {other:?}"),
        }
        match &instructions[1].kind {
            InstructionKind::Refine { ty, .. } => assert_eq!(ty, &Type::Int),
            other => panic!("expected Refine, got {other:?}"),
        }
        let narrowed_local = lowering
            .get_block_narrowed_local(&var_id)
            .expect("narrowed local recorded");
        assert_eq!(mir_func.locals[&narrowed_local].ty, Type::Int);

        lowering.leave_cfg_block_narrowings();
        assert_eq!(lowering.get_var_type(&var_id), Some(&union_ty));
        assert!(lowering.get_block_narrowed_local(&var_id).is_none());
    }

    #[test]
    fn cfg_block_narrowing_materializes_unbox_and_refine_for_any_payloads() {
        let mut interner = StringInterner::default();
        let mut lowering = Lowering::new(&mut interner);
        let mut mir_func = mir::Function::new(
            pyaot_utils::FuncId::from(1u32),
            "f_any".to_string(),
            Vec::new(),
            Type::None,
            None,
        );
        let block = lowering.new_block();
        lowering.push_block(block);

        let var_id = VarId::new(0);
        let base_local = LocalId::from(0u32);
        mir_func.add_local(mir::Local {
            id: base_local,
            name: None,
            ty: Type::Any,
            is_gc_root: true,
        });
        lowering.insert_var_local(var_id, base_local);
        lowering.insert_var_type(var_id, Type::Any);

        lowering.enter_cfg_block_narrowings(
            &[TypeNarrowingInfo {
                var_id,
                narrowed_type: Type::Int,
                original_type: Type::Any,
            }],
            &mut mir_func,
        );

        let instructions = lowering.current_block_mut().instructions.clone();
        assert_eq!(instructions.len(), 2);
        match &instructions[0].kind {
            InstructionKind::RuntimeCall { func, .. } => match func {
                mir::RuntimeFunc::Call(def) => {
                    assert!(std::ptr::eq(
                        *def,
                        &pyaot_core_defs::runtime_func_def::RT_UNBOX_INT
                    ));
                }
                other => panic!("expected runtime unbox call, got {other:?}"),
            },
            other => panic!("expected RuntimeCall, got {other:?}"),
        }
        match &instructions[1].kind {
            InstructionKind::Refine { ty, .. } => assert_eq!(ty, &Type::Int),
            other => panic!("expected Refine, got {other:?}"),
        }
    }

    #[test]
    fn cfg_block_narrowing_materializes_refine_only_for_heap_compatible_types() {
        let mut interner = StringInterner::default();
        let mut lowering = Lowering::new(&mut interner);
        let mut mir_func = mir::Function::new(
            pyaot_utils::FuncId::from(1u32),
            "g".to_string(),
            Vec::new(),
            Type::None,
            None,
        );
        let block = lowering.new_block();
        lowering.push_block(block);

        let class_ty = Type::Class {
            class_id: ClassId::from(1u32),
            name: lowering.interner.intern("Box"),
        };
        let var_id = VarId::new(1);
        let base_local = LocalId::from(0u32);
        mir_func.add_local(mir::Local {
            id: base_local,
            name: None,
            ty: Type::Any,
            is_gc_root: true,
        });
        lowering.insert_var_local(var_id, base_local);
        lowering.insert_var_type(var_id, Type::Any);

        lowering.enter_cfg_block_narrowings(
            &[TypeNarrowingInfo {
                var_id,
                narrowed_type: class_ty.clone(),
                original_type: Type::Any,
            }],
            &mut mir_func,
        );

        let instructions = lowering.current_block_mut().instructions.clone();
        assert_eq!(instructions.len(), 1);
        match &instructions[0].kind {
            InstructionKind::Refine { ty, .. } => assert_eq!(ty, &class_ty),
            other => panic!("expected Refine, got {other:?}"),
        }
        let narrowed_local = lowering
            .get_block_narrowed_local(&var_id)
            .expect("narrowed local recorded");
        assert_eq!(mir_func.locals[&narrowed_local].ty, class_ty);
    }
}
