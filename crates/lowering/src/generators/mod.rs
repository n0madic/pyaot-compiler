//! Generator lowering support
//!
//! Generators are transformed into state machines:
//! 1. The original function becomes a "creator" that returns a generator object
//! 2. A separate "resume" function implements the state machine
//!
//! Variables are persisted across yields by saving them to the generator object's
//! locals array before each yield and restoring them on resume.
//!
//! This module is organized into submodules:
//! - `vars`: Variable collection for generator functions
//! - `creator`: Generator creator function generation
//! - `resume`: Generic resume/state machine generation
//! - `while_loop`: While-loop pattern detection and resume generation
//! - `for_loop`: For-loop pattern detection and resume generation
//! - `utils`: Helper functions for expression lowering in generators

mod creator;
mod for_loop;
mod resume;
mod utils;
mod vars;
mod while_loop;

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::context::Lowering;
use crate::utils::get_iterable_info;
use for_loop::detect_for_loop_generator;
use vars::collect_generator_vars;

/// Information about a while-loop generator pattern
#[derive(Debug)]
pub(super) struct WhileLoopGenerator {
    /// Statements before the while loop (initialization)
    pub(super) init_stmts: Vec<hir::StmtId>,
    /// The while condition expression
    pub(super) cond: hir::ExprId,
    /// Yield sections in the loop (supports multiple yields)
    pub(super) yield_sections: Vec<YieldSection>,
    /// Statements after last yield (update)
    pub(super) update_stmts: Vec<hir::StmtId>,
}

/// Information about a yield section in a while-loop generator
#[derive(Debug)]
pub(super) struct YieldSection {
    /// Statements to execute before this yield
    pub(super) stmts_before: Vec<hir::StmtId>,
    /// The yield expression (None for `yield` without value)
    pub(super) yield_expr: Option<hir::ExprId>,
}

/// Information about a for-loop generator pattern
#[derive(Debug)]
pub(super) struct ForLoopGenerator {
    /// The loop variable
    pub(super) target_var: VarId,
    /// The iterable expression (what we iterate over)
    pub(super) iter_expr: hir::ExprId,
    /// The yield expression inside the loop
    pub(super) yield_expr: Option<hir::ExprId>,
    /// Optional filter condition (from `if cond` in comprehension)
    pub(super) filter_cond: Option<hir::ExprId>,
    /// Trailing yield expressions after the for-loop (from `yield from X; yield Y`)
    pub(super) trailing_yields: Vec<Option<hir::ExprId>>,
}

/// Information about a variable that needs to persist across yields
#[derive(Debug, Clone)]
pub struct GeneratorVar {
    /// The HIR variable ID
    pub var_id: VarId,
    /// Index in generator's locals array
    pub gen_local_idx: u32,
    /// Type of the variable
    pub ty: Type,
    /// Whether this is a parameter (set in creator)
    pub is_param: bool,
}

/// Information about a yield point in a generator
#[derive(Debug, Clone)]
pub struct YieldInfo {
    /// The expression being yielded (None means yield None)
    pub yield_value: Option<hir::ExprId>,
    /// The variable that receives the sent value (if yield is in assignment context)
    pub assignment_target: Option<VarId>,
}

impl<'a> Lowering<'a> {
    /// Lower a yield expression
    /// This is called during generator body lowering
    pub fn lower_yield_expr(
        &mut self,
        value: Option<hir::ExprId>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Get the yielded value (or None if no value)
        let yield_value = if let Some(val_id) = value {
            let val_expr = &hir_module.exprs[val_id];
            self.lower_expr(val_expr, hir_module, mir_func)?
        } else {
            mir::Operand::Constant(mir::Constant::None)
        };

        // The generator object is passed as an implicit first parameter to the resume function.
        // We need access to the generator context to:
        // 1. Save locals to the generator object
        // 2. Update the generator state
        // 3. Return the yielded value

        // If we're in a generator context (has generator parameter), emit save/state/return
        // Otherwise, this is an error (yield outside generator)

        // Return the yield value directly - the actual state machine logic
        // is handled at the function level in lower_generator_function()
        Ok(yield_value)
    }

    /// Infer the element type yielded by a generator function.
    ///
    /// For the simple for-loop pattern `for x in iterable: yield x` (where the yield
    /// expression is exactly the loop variable), the element type is the element type
    /// of the iterable. For attribute access patterns like `yield v.field` where `v`
    /// is the loop variable and the iterable element is a class instance, the field
    /// type is resolved from the class definition.
    ///
    /// Falls back to `Type::Any` for patterns we cannot statically infer.
    fn infer_generator_yield_type(
        &mut self,
        func: &hir::Function,
        hir_module: &hir::Module,
    ) -> Type {
        let ty = self.infer_generator_yield_type_raw(func, hir_module);
        // Generator resume functions always return i64 through the iterator
        // protocol, so Bool (i8) must be widened to Int to avoid a Cranelift
        // type mismatch when the caller stores the IterNext result.
        match ty {
            Type::Bool => Type::Int,
            other => other,
        }
    }

    /// Raw yield type inference without generator-protocol normalization.
    fn infer_generator_yield_type_raw(
        &mut self,
        func: &hir::Function,
        hir_module: &hir::Module,
    ) -> Type {
        // Try to detect the for-loop generator pattern
        if let Some(for_gen) = detect_for_loop_generator(&func.body, hir_module) {
            // First compute the iterable element type; we need it for attribute resolution.
            let iter_type = self.get_type_of_expr_id(for_gen.iter_expr, hir_module);
            let elem_ty = get_iterable_info(&iter_type).map(|(_kind, ty)| ty);

            if let Some(yield_eid) = for_gen.yield_expr {
                let yield_expr = &hir_module.exprs[yield_eid];

                // Fast path: try direct type inference from the expression.
                // Works when `v` is already registered in var_types (e.g. simple `yield v`
                // where the loop variable has a known type annotation on the for-stmt).
                let yield_ty = self.get_type_of_expr_id(yield_eid, hir_module);
                if yield_ty != Type::Any {
                    return yield_ty;
                }

                // Slow path: the yield expression is `v.attr` where `v` is the loop
                // variable.  `get_expr_type` returned Any because the loop variable is
                // not yet registered in var_types at inference time.  Resolve the
                // attribute type by looking it up in the element class's field_types.
                if let hir::ExprKind::Attribute { obj, attr } = &yield_expr.kind {
                    let obj_expr = &hir_module.exprs[*obj];
                    if let hir::ExprKind::Var(var_id) = &obj_expr.kind {
                        if *var_id == for_gen.target_var {
                            if let Some(Type::Class { class_id, .. }) = &elem_ty {
                                if let Some(class_info) = self.get_class_info(class_id) {
                                    if let Some(field_ty) = class_info.field_types.get(attr) {
                                        return field_ty.clone();
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Fallback: return the raw iterable element type (handles `yield v` pattern
            // when direct inference above returned Any for some reason).
            if let Some(ty) = elem_ty {
                if ty != Type::Any {
                    return ty;
                }
            }
        }
        Type::Any
    }

    /// Lower a generator function
    /// This creates:
    /// 1. A creator function that allocates and returns a generator object
    /// 2. A resume function that implements the state machine
    pub fn lower_generator_function(
        &mut self,
        func: &hir::Function,
        hir_module: &hir::Module,
    ) -> Result<(mir::Function, mir::Function)> {
        // Collect all variables used in the generator
        let gen_vars = collect_generator_vars(func, hir_module);
        let num_locals = gen_vars.len() as u32 + 5; // Variables + some extra for sent values etc.

        // Infer the yield element type for the generator's return type.
        // For simple for-loop generators (e.g. `v for v in list[float]`), this
        // propagates the concrete element type so callers like min()/max() can
        // emit correct float comparisons instead of defaulting to integer ops.
        let yield_elem_type = self.infer_generator_yield_type(func, hir_module);

        // Store the return type for later lookup when calling this generator
        let return_type = Type::Iterator(Box::new(yield_elem_type));
        self.insert_func_return_type(func.id, return_type);

        // 1. Create the creator function (saves parameters to generator)
        let creator_func =
            self.create_generator_creator(func, hir_module, &gen_vars, num_locals)?;

        // 2. Create the resume function (implements state machine)
        let resume_func = self.create_generator_resume(func, hir_module, &gen_vars)?;

        Ok((creator_func, resume_func))
    }
}
