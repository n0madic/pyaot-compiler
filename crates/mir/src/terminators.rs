//! MIR block terminators

use pyaot_utils::{BlockId, LocalId};

use crate::Operand;

/// Terminator instruction (ends a basic block)
#[derive(Debug, Clone)]
pub enum Terminator {
    /// Return from function
    Return(Option<Operand>),

    /// Unconditional jump
    Goto(BlockId),

    /// Conditional branch
    Branch {
        cond: Operand,
        then_block: BlockId,
        else_block: BlockId,
    },

    /// Unreachable
    Unreachable,

    // ==================== Exception handling terminators ====================
    /// setjmp-based try entry
    /// Calls setjmp on frame_local's jmp_buf
    /// Returns 0 → try_body, non-zero → handler_entry
    TrySetjmp {
        frame_local: LocalId,
        try_body: BlockId,
        handler_entry: BlockId,
    },

    /// Raise exception (diverging)
    /// exc_type: exception type tag (0 = Exception, 1 = AssertionError)
    /// message: optional string message operand
    /// cause: optional chained cause exception (`raise X from Y`)
    /// suppress_context: if true, suppress __context__ display (for `raise X from None`)
    Raise {
        exc_type: u8,
        message: Option<Operand>,
        cause: Option<RaiseCause>,
        suppress_context: bool,
    },

    /// Raise custom exception (diverging)
    /// class_id: class ID for the custom exception (27+ for user-defined, 0-26 for built-in)
    /// message: optional string message operand
    RaiseCustom {
        class_id: u8,
        message: Option<Operand>,
    },

    /// Re-raise current exception (diverging)
    /// Used for bare `raise` statement in except block
    Reraise,
}

/// Cause exception for `raise X from Y`
#[derive(Debug, Clone)]
pub struct RaiseCause {
    pub exc_type: u8,
    pub message: Option<Operand>,
}
