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

use indexmap::IndexMap;
use pyaot_hir as hir;
use pyaot_types::Type;
use pyaot_utils::{Span, VarId};

use crate::context::{Lowering, NarrowingFrame};

/// Indicates which branch of an if statement is dead (unreachable)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadBranch {
    /// The then-branch is dead (isinstance always returns False)
    ThenBranch,
    /// The else-branch is dead (isinstance always returns True)
    ElseBranch,
}

/// Information extracted from an isinstance() call for narrowing and dead code detection
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
                    if let Some(original_type) = self.get_var_type(&var_id).cloned() {
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
                if let Some(original_type) = self.get_var_type(var_id).cloned() {
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
        let original_type = self.get_var_type(&var_id).cloned()?;

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

    /// Push a new narrowing scope onto `hir_types.narrowing_stack` and
    /// apply the given narrowings to `var_types` + `narrowed_union_vars`.
    /// Pair with [`Self::pop_narrowing_frame`] on scope exit — the stack
    /// records the undo information so callers don't thread a saved-state
    /// variable through their own scope.
    ///
    /// Replaces the legacy `apply_narrowings` / `restore_types` pair
    /// (S1.9d, Phase 1 §1.4). Semantics unchanged.
    pub(crate) fn push_narrowing_frame(&mut self, narrowings: &[TypeNarrowingInfo]) {
        let mut saved_var_types = IndexMap::new();
        let mut added_union_tracking = Vec::new();

        for info in narrowings {
            // Save original type.
            if let Some(original) = self.get_var_type(&info.var_id).cloned() {
                saved_var_types.insert(info.var_id, original.clone());

                // Track narrowed Union variables for unboxing and is / is
                // not comparisons. Applies when the original is a Union
                // and the narrowed type is a primitive or None (None is
                // required for `is None` comparisons on narrowed vars).
                if matches!(original, Type::Union(_))
                    && matches!(
                        info.narrowed_type,
                        Type::Int | Type::Float | Type::Bool | Type::Str | Type::None
                    )
                {
                    self.insert_narrowed_union(info.var_id, info.original_type.clone());
                    added_union_tracking.push(info.var_id);
                }
            }
            // Apply narrowed type.
            self.insert_var_type(info.var_id, info.narrowed_type.clone());
        }

        self.hir_types.narrowing_stack.push(NarrowingFrame {
            saved_var_types,
            added_union_tracking,
        });
    }

    /// Pop the innermost narrowing scope and restore the var types it
    /// overwrote. Also clears any narrowed-Union tracking the matching
    /// push added. Panics if the stack is empty — calls must be
    /// balanced with `push_narrowing_frame`.
    pub(crate) fn pop_narrowing_frame(&mut self) {
        let frame = self
            .hir_types
            .narrowing_stack
            .pop()
            .expect("pop_narrowing_frame called without a matching push");
        for (var_id, original_type) in frame.saved_var_types {
            self.insert_var_type(var_id, original_type);
        }
        for var_id in frame.added_union_tracking {
            self.remove_narrowed_union(&var_id);
        }
    }

    /// Extract isinstance information from an isinstance expression for dead code detection.
    /// This is a public wrapper for use by control_flow.rs.
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
        // Non-union types return self when excluded
        assert_eq!(narrowed, Type::None);
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
}
