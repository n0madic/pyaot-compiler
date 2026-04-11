//! Generator desugaring support
//!
//! Generators are transformed into state machines at HIR level:
//! 1. The original function becomes a "creator" that returns a generator object
//! 2. A separate "resume" function implements the state machine
//!
//! Variables are persisted across yields by saving them to the generator object's
//! locals array before each yield and restoring them on resume.
//!
//! This module is organized into submodules:
//! - `desugaring`: HIR-level generator desugaring pass (main entry point)
//! - `vars`: Variable collection for generator functions
//! - `for_loop`: For-loop pattern detection
//! - `while_loop`: While-loop pattern detection
//! - `utils`: Helper functions for yield info collection

pub(crate) mod desugaring;
pub(crate) mod for_loop;
pub(crate) mod utils;
pub(crate) mod vars;
pub(crate) mod while_loop;

use pyaot_hir as hir;
use pyaot_types::Type;
use pyaot_utils::VarId;

/// Information about a while-loop generator pattern
#[derive(Debug, Clone)]
pub(crate) struct WhileLoopGenerator {
    /// Statements before the while loop (initialization)
    pub(crate) init_stmts: Vec<hir::StmtId>,
    /// The while condition expression
    pub(crate) cond: hir::ExprId,
    /// Yield sections in the loop (supports multiple yields)
    pub(crate) yield_sections: Vec<YieldSection>,
    /// Statements after last yield (update)
    pub(crate) update_stmts: Vec<hir::StmtId>,
}

/// Information about a yield section in a while-loop generator
#[derive(Debug, Clone)]
pub(crate) struct YieldSection {
    /// Statements to execute before this yield
    pub(crate) stmts_before: Vec<hir::StmtId>,
    /// The yield expression (None for `yield` without value)
    pub(crate) yield_expr: Option<hir::ExprId>,
}

/// Information about a for-loop generator pattern
#[derive(Debug, Clone)]
pub(crate) struct ForLoopGenerator {
    /// The loop variable
    pub(crate) target_var: VarId,
    /// The iterable expression (what we iterate over)
    pub(crate) iter_expr: hir::ExprId,
    /// The yield expression inside the loop
    pub(crate) yield_expr: Option<hir::ExprId>,
    /// Optional filter condition (from `if cond` in comprehension)
    pub(crate) filter_cond: Option<hir::ExprId>,
    /// Trailing yield expressions after the for-loop (from `yield from X; yield Y`)
    pub(crate) trailing_yields: Vec<Option<hir::ExprId>>,
}

/// Information about a variable that needs to persist across yields
#[derive(Debug, Clone)]
pub(crate) struct GeneratorVar {
    /// The HIR variable ID
    pub(crate) var_id: VarId,
    /// Index in generator's locals array
    pub(crate) gen_local_idx: u32,
    /// Type of the variable
    pub(crate) ty: Type,
    /// Whether this is a parameter (set in creator)
    pub(crate) is_param: bool,
}

/// Information about a yield point in a generator
#[derive(Debug, Clone)]
pub(crate) struct YieldInfo {
    /// The expression being yielded (None means yield None)
    pub(crate) yield_value: Option<hir::ExprId>,
    /// The variable that receives the sent value (if yield is in assignment context)
    pub(crate) assignment_target: Option<VarId>,
}
