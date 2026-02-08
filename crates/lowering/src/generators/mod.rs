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

        // Create a result local for the yield expression
        // (yield can receive a value via send(), handled in full generator state machine)
        let result_local = self.alloc_local_id();
        mir_func.add_local(mir::Local {
            id: result_local,
            name: None,
            ty: Type::Any, // The value received from send()
            is_gc_root: false,
        });

        // Emit the yield value as the operand
        // The actual state machine logic is handled at the function level
        // Store yield value in result local and return it
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: result_local,
            src: yield_value.clone(),
        });

        Ok(yield_value)
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

        // Store the return type for later lookup when calling this generator
        let return_type = Type::Iterator(Box::new(Type::Any));
        self.insert_func_return_type(func.id, return_type);

        // 1. Create the creator function (saves parameters to generator)
        let creator_func =
            self.create_generator_creator(func, hir_module, &gen_vars, num_locals)?;

        // 2. Create the resume function (implements state machine)
        let resume_func = self.create_generator_resume(func, hir_module, &gen_vars)?;

        Ok((creator_func, resume_func))
    }
}
