//! Local variable and basic block allocation methods

use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{BlockId, LocalId};

use super::Lowering;

impl<'a> Lowering<'a> {
    /// Allocate a new local variable ID
    pub(crate) fn alloc_local_id(&mut self) -> LocalId {
        let id = LocalId::new(self.codegen.next_local_id);
        self.codegen.next_local_id += 1;
        id
    }

    /// Allocate a new local variable and add it to the function.
    /// This is a helper to reduce boilerplate when creating locals.
    pub(crate) fn alloc_and_add_local(
        &mut self,
        ty: Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        let local_id = self.alloc_local_id();
        mir_func.add_local(mir::Local {
            id: local_id,
            name: None,
            ty: ty.clone(),
            is_gc_root: ty.is_heap(),
        });
        local_id
    }

    /// Allocate a new local variable with explicit gc_root flag.
    /// Use this when you need to override the default is_heap_type behavior.
    fn alloc_and_add_local_with_gc(
        &mut self,
        ty: Type,
        is_gc_root: bool,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        let local_id = self.alloc_local_id();
        mir_func.add_local(mir::Local {
            id: local_id,
            name: None,
            ty,
            is_gc_root,
        });
        local_id
    }

    /// Allocate a local that is a GC root (for heap-allocated types like str, list, dict).
    /// Use this for values that live on the heap and need garbage collection tracking.
    pub(crate) fn alloc_gc_local(&mut self, ty: Type, mir_func: &mut mir::Function) -> LocalId {
        self.alloc_and_add_local_with_gc(ty, true, mir_func)
    }

    /// Allocate a local that is NOT a GC root (for stack values or transient pointers).
    /// Use this for temporary values that don't need GC tracking, such as:
    /// - Static string pointers (compile-time constants)
    /// - Transient pointers used only for immediate unboxing
    pub(crate) fn alloc_stack_local(&mut self, ty: Type, mir_func: &mut mir::Function) -> LocalId {
        self.alloc_and_add_local_with_gc(ty, false, mir_func)
    }

    /// Create a new basic block
    pub(crate) fn new_block(&mut self) -> mir::BasicBlock {
        let id = BlockId::from(self.codegen.next_block_id);
        self.codegen.next_block_id += 1;
        mir::BasicBlock {
            id,
            instructions: Vec::new(),
            terminator: mir::Terminator::Unreachable,
        }
    }

    /// Get mutable reference to current basic block
    pub(crate) fn current_block_mut(&mut self) -> &mut mir::BasicBlock {
        &mut self.codegen.current_blocks[self.codegen.current_block_idx]
    }

    /// Check if current block already has a terminator
    pub(crate) fn current_block_has_terminator(&self) -> bool {
        !matches!(
            self.codegen.current_blocks[self.codegen.current_block_idx].terminator,
            mir::Terminator::Unreachable
        )
    }

    /// Emit an instruction to the current basic block
    pub(crate) fn emit_instruction(&mut self, kind: mir::InstructionKind) {
        let span = self.codegen.current_span;
        self.current_block_mut()
            .instructions
            .push(mir::Instruction { kind, span });
    }

    /// Emit a runtime call: allocates a result local, emits the RuntimeCall instruction,
    /// and returns the LocalId of the result.
    ///
    /// This consolidates the common pattern of:
    ///   let dest = self.alloc_and_add_local(result_type, mir_func);
    ///   self.emit_instruction(RuntimeCall { dest, func, args });
    pub(crate) fn emit_runtime_call(
        &mut self,
        func: mir::RuntimeFunc,
        args: Vec<mir::Operand>,
        result_type: Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        let dest = self.alloc_and_add_local(result_type, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall { dest, func, args });
        dest
    }
}
