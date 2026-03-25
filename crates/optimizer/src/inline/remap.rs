//! ID remapping for function inlining
//!
//! When inlining a function, we need to remap all LocalId and BlockId
//! to avoid conflicts with the caller function's IDs.

use indexmap::IndexMap;
use pyaot_mir::{BasicBlock, Instruction, InstructionKind, Operand, RaiseCause, Terminator};
use pyaot_utils::{BlockId, LocalId};

/// Remapper for LocalId and BlockId when inlining
#[derive(Debug)]
pub struct InlineRemapper {
    /// Map from callee LocalId to caller LocalId
    local_map: IndexMap<LocalId, LocalId>,
    /// Map from callee BlockId to caller BlockId
    block_map: IndexMap<BlockId, BlockId>,
    /// Next available LocalId in caller
    next_local: u32,
    /// Next available BlockId in caller
    next_block: u32,
}

impl InlineRemapper {
    /// Create a new remapper starting from the given IDs
    pub fn new(next_local: u32, next_block: u32) -> Self {
        Self {
            local_map: IndexMap::new(),
            block_map: IndexMap::new(),
            next_local,
            next_block,
        }
    }

    /// Get or create a remapped LocalId
    pub fn remap_local(&mut self, old: LocalId) -> LocalId {
        *self.local_map.entry(old).or_insert_with(|| {
            let new = LocalId::from(self.next_local);
            self.next_local += 1;
            new
        })
    }

    /// Get or create a remapped BlockId
    pub fn remap_block(&mut self, old: BlockId) -> BlockId {
        *self.block_map.entry(old).or_insert_with(|| {
            let new = BlockId::from(self.next_block);
            self.next_block += 1;
            new
        })
    }

    /// Allocate a fresh BlockId without associating it with a source block
    pub fn allocate_block_id(&mut self) -> BlockId {
        let id = BlockId::from(self.next_block);
        self.next_block += 1;
        id
    }

    /// Remap an operand
    pub fn remap_operand(&mut self, op: &Operand) -> Operand {
        match op {
            Operand::Local(id) => Operand::Local(self.remap_local(*id)),
            Operand::Constant(c) => Operand::Constant(c.clone()),
        }
    }

    /// Remap operands in a vector
    pub fn remap_operands(&mut self, ops: &[Operand]) -> Vec<Operand> {
        ops.iter().map(|op| self.remap_operand(op)).collect()
    }

    /// Remap an instruction
    pub fn remap_instruction(&mut self, instr: &Instruction) -> Instruction {
        let kind = match &instr.kind {
            InstructionKind::Const { dest, value } => InstructionKind::Const {
                dest: self.remap_local(*dest),
                value: value.clone(),
            },
            InstructionKind::BinOp {
                dest,
                op,
                left,
                right,
            } => InstructionKind::BinOp {
                dest: self.remap_local(*dest),
                op: *op,
                left: self.remap_operand(left),
                right: self.remap_operand(right),
            },
            InstructionKind::UnOp { dest, op, operand } => InstructionKind::UnOp {
                dest: self.remap_local(*dest),
                op: *op,
                operand: self.remap_operand(operand),
            },
            InstructionKind::Call { dest, func, args } => InstructionKind::Call {
                dest: self.remap_local(*dest),
                func: self.remap_operand(func),
                args: self.remap_operands(args),
            },
            InstructionKind::CallDirect { dest, func, args } => InstructionKind::CallDirect {
                dest: self.remap_local(*dest),
                func: *func, // FuncId stays the same
                args: self.remap_operands(args),
            },
            InstructionKind::CallNamed { dest, name, args } => InstructionKind::CallNamed {
                dest: self.remap_local(*dest),
                name: name.clone(),
                args: self.remap_operands(args),
            },
            InstructionKind::CallVirtual {
                dest,
                obj,
                slot,
                args,
            } => InstructionKind::CallVirtual {
                dest: self.remap_local(*dest),
                obj: self.remap_operand(obj),
                slot: *slot,
                args: self.remap_operands(args),
            },
            InstructionKind::FuncAddr { dest, func } => InstructionKind::FuncAddr {
                dest: self.remap_local(*dest),
                func: *func, // FuncId stays the same
            },
            InstructionKind::RuntimeCall { dest, func, args } => InstructionKind::RuntimeCall {
                dest: self.remap_local(*dest),
                func: *func,
                args: self.remap_operands(args),
            },
            InstructionKind::Copy { dest, src } => InstructionKind::Copy {
                dest: self.remap_local(*dest),
                src: self.remap_operand(src),
            },
            InstructionKind::GcPush { frame } => InstructionKind::GcPush {
                frame: self.remap_local(*frame),
            },
            InstructionKind::GcPop => InstructionKind::GcPop,
            InstructionKind::GcAlloc { dest, ty, size } => InstructionKind::GcAlloc {
                dest: self.remap_local(*dest),
                ty: ty.clone(),
                size: *size,
            },
            InstructionKind::FloatToInt { dest, src } => InstructionKind::FloatToInt {
                dest: self.remap_local(*dest),
                src: self.remap_operand(src),
            },
            InstructionKind::BoolToInt { dest, src } => InstructionKind::BoolToInt {
                dest: self.remap_local(*dest),
                src: self.remap_operand(src),
            },
            InstructionKind::IntToFloat { dest, src } => InstructionKind::IntToFloat {
                dest: self.remap_local(*dest),
                src: self.remap_operand(src),
            },
            InstructionKind::FloatBits { dest, src } => InstructionKind::FloatBits {
                dest: self.remap_local(*dest),
                src: self.remap_operand(src),
            },
            InstructionKind::IntBitsToFloat { dest, src } => InstructionKind::IntBitsToFloat {
                dest: self.remap_local(*dest),
                src: self.remap_operand(src),
            },
            InstructionKind::FloatAbs { dest, src } => InstructionKind::FloatAbs {
                dest: self.remap_local(*dest),
                src: self.remap_operand(src),
            },
            // Exception handling instructions - remap but preserve structure
            InstructionKind::ExcPushFrame { frame_local } => InstructionKind::ExcPushFrame {
                frame_local: self.remap_local(*frame_local),
            },
            InstructionKind::ExcPopFrame => InstructionKind::ExcPopFrame,
            InstructionKind::ExcGetType { dest } => InstructionKind::ExcGetType {
                dest: self.remap_local(*dest),
            },
            InstructionKind::ExcClear => InstructionKind::ExcClear,
            InstructionKind::ExcHasException { dest } => InstructionKind::ExcHasException {
                dest: self.remap_local(*dest),
            },
            InstructionKind::ExcGetCurrent { dest } => InstructionKind::ExcGetCurrent {
                dest: self.remap_local(*dest),
            },
            InstructionKind::ExcCheckType { dest, type_tag } => InstructionKind::ExcCheckType {
                dest: self.remap_local(*dest),
                type_tag: *type_tag,
            },
            InstructionKind::ExcCheckClass { dest, class_id } => InstructionKind::ExcCheckClass {
                dest: self.remap_local(*dest),
                class_id: *class_id,
            },
            InstructionKind::ExcStartHandling => InstructionKind::ExcStartHandling,
            InstructionKind::ExcEndHandling => InstructionKind::ExcEndHandling,
            InstructionKind::BuiltinAddr { dest, builtin } => InstructionKind::BuiltinAddr {
                dest: self.remap_local(*dest),
                builtin: *builtin,
            },
        };
        Instruction {
            kind,
            span: instr.span,
        }
    }

    /// Remap a terminator
    pub fn remap_terminator(&mut self, term: &Terminator) -> Terminator {
        match term {
            Terminator::Return(op) => {
                Terminator::Return(op.as_ref().map(|o| self.remap_operand(o)))
            }
            Terminator::Goto(block) => Terminator::Goto(self.remap_block(*block)),
            Terminator::Branch {
                cond,
                then_block,
                else_block,
            } => Terminator::Branch {
                cond: self.remap_operand(cond),
                then_block: self.remap_block(*then_block),
                else_block: self.remap_block(*else_block),
            },
            Terminator::Unreachable => Terminator::Unreachable,
            Terminator::TrySetjmp {
                frame_local,
                try_body,
                handler_entry,
            } => Terminator::TrySetjmp {
                frame_local: self.remap_local(*frame_local),
                try_body: self.remap_block(*try_body),
                handler_entry: self.remap_block(*handler_entry),
            },
            Terminator::Raise {
                exc_type,
                message,
                cause,
                suppress_context,
            } => Terminator::Raise {
                exc_type: *exc_type,
                message: message.as_ref().map(|o| self.remap_operand(o)),
                cause: cause.as_ref().map(|c| RaiseCause {
                    exc_type: c.exc_type,
                    message: c.message.as_ref().map(|o| self.remap_operand(o)),
                }),
                suppress_context: *suppress_context,
            },
            Terminator::RaiseCustom {
                class_id,
                message,
                instance,
            } => Terminator::RaiseCustom {
                class_id: *class_id,
                message: message.as_ref().map(|o| self.remap_operand(o)),
                instance: instance.as_ref().map(|o| self.remap_operand(o)),
            },
            Terminator::Reraise => Terminator::Reraise,
        }
    }

    /// Remap an entire basic block
    pub fn remap_block_contents(&mut self, block: &BasicBlock) -> BasicBlock {
        BasicBlock {
            id: self.remap_block(block.id),
            instructions: block
                .instructions
                .iter()
                .map(|i| self.remap_instruction(i))
                .collect(),
            terminator: self.remap_terminator(&block.terminator),
        }
    }
}
