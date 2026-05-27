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
        /// Pre-created exception instance (eagerly allocated with __init__ called)
        instance: Option<Operand>,
    },

    /// Re-raise current exception (diverging)
    /// Used for bare `raise` statement in except block
    Reraise,

    /// Raise from an existing exception instance (diverging)
    /// Used for `raise e` where e is a caught exception variable
    RaiseInstance { instance: Operand },
}

impl Terminator {
    /// Apply `f` to each `LocalId` used (read) by this terminator.
    ///
    /// `TrySetjmp::frame_local` is visited as a use: setjmp READS the
    /// jmp_buf storage location whose def is the upstream `ExcPushFrame`
    /// instruction. This differs from `InstructionKind::for_each_use`'s
    /// `GcPush::frame` / `ExcPushFrame::frame_local` (which ARE the defs
    /// of their frame field).
    pub fn for_each_use<F: FnMut(LocalId)>(&self, mut f: F) {
        fn push<F: FnMut(LocalId)>(op: &Operand, f: &mut F) {
            if let Operand::Local(id) = op {
                f(*id);
            }
        }
        match self {
            Terminator::Return(op) => {
                if let Some(op) = op {
                    push(op, &mut f);
                }
            }
            Terminator::Goto(_) | Terminator::Unreachable | Terminator::Reraise => {}
            Terminator::Branch { cond, .. } => push(cond, &mut f),
            Terminator::TrySetjmp { frame_local, .. } => f(*frame_local),
            Terminator::Raise { message, cause, .. } => {
                if let Some(op) = message {
                    push(op, &mut f);
                }
                if let Some(RaiseCause {
                    message: Some(op), ..
                }) = cause
                {
                    push(op, &mut f);
                }
            }
            Terminator::RaiseCustom {
                message, instance, ..
            } => {
                if let Some(op) = message {
                    push(op, &mut f);
                }
                if let Some(op) = instance {
                    push(op, &mut f);
                }
            }
            Terminator::RaiseInstance { instance } => push(instance, &mut f),
        }
    }

    /// Apply `f` to a mutable reference to each `LocalId` used by this
    /// terminator. Used by SSA renaming to substitute uses with their
    /// current top-of-stack name. Mirrors [`Self::for_each_use`].
    pub fn for_each_use_mut<F: FnMut(&mut LocalId)>(&mut self, mut f: F) {
        fn push<F: FnMut(&mut LocalId)>(op: &mut Operand, f: &mut F) {
            if let Operand::Local(id) = op {
                f(id);
            }
        }
        match self {
            Terminator::Return(op) => {
                if let Some(op) = op {
                    push(op, &mut f);
                }
            }
            Terminator::Goto(_) | Terminator::Unreachable | Terminator::Reraise => {}
            Terminator::Branch { cond, .. } => push(cond, &mut f),
            Terminator::TrySetjmp { frame_local, .. } => f(frame_local),
            Terminator::Raise { message, cause, .. } => {
                if let Some(op) = message {
                    push(op, &mut f);
                }
                if let Some(RaiseCause {
                    message: Some(op), ..
                }) = cause
                {
                    push(op, &mut f);
                }
            }
            Terminator::RaiseCustom {
                message, instance, ..
            } => {
                if let Some(op) = message {
                    push(op, &mut f);
                }
                if let Some(op) = instance {
                    push(op, &mut f);
                }
            }
            Terminator::RaiseInstance { instance } => push(instance, &mut f),
        }
    }
}

/// Cause exception for `raise X from Y`
#[derive(Debug, Clone)]
pub struct RaiseCause {
    pub exc_type: u8,
    pub message: Option<Operand>,
}
