//! Core MIR structures: Module, Function, Local, BasicBlock

use indexmap::IndexMap;
use pyaot_types::Type;
use pyaot_utils::{BlockId, ClassId, FuncId, InternedString, LocalId, Span};

use crate::{Instruction, Terminator};

/// Entry in a class vtable mapping slot to method function
#[derive(Debug, Clone)]
pub struct VtableEntry {
    pub slot: usize,
    pub method_func_id: FuncId,
}

/// Vtable information for a class
#[derive(Debug, Clone)]
pub struct VtableInfo {
    pub class_id: ClassId,
    pub entries: Vec<VtableEntry>,
}

/// MIR Module
#[derive(Debug)]
pub struct Module {
    pub functions: IndexMap<FuncId, Function>,
    pub vtables: Vec<VtableInfo>,
    /// Module initialization function order (for multi-module compilation)
    /// Each entry is (module_name, init_func_id)
    pub module_init_order: Vec<(String, FuncId)>,
}

/// MIR Function with CFG
#[derive(Debug, Clone)]
pub struct Function {
    pub id: FuncId,
    pub name: String,
    pub params: Vec<Local>,
    pub return_type: Type,
    pub locals: IndexMap<LocalId, Local>,
    pub blocks: IndexMap<BlockId, BasicBlock>,
    pub entry_block: BlockId,
    /// Source location of the function definition (for DWARF DW_TAG_subprogram)
    pub span: Option<Span>,
}

/// Local variable in MIR
#[derive(Debug, Clone)]
pub struct Local {
    pub id: LocalId,
    pub name: Option<InternedString>,
    pub ty: Type,
    pub is_gc_root: bool, // true if this holds a GC-managed pointer
}

/// Basic block in CFG
#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: BlockId,
    pub instructions: Vec<Instruction>,
    pub terminator: Terminator,
}

impl Module {
    pub fn new() -> Self {
        Self {
            functions: IndexMap::new(),
            vtables: Vec::new(),
            module_init_order: Vec::new(),
        }
    }

    pub fn add_function(&mut self, func: Function) {
        self.functions.insert(func.id, func);
    }
}

impl Default for Module {
    fn default() -> Self {
        Self::new()
    }
}

impl Function {
    pub fn new(
        id: FuncId,
        name: String,
        params: Vec<Local>,
        return_type: Type,
        span: Option<pyaot_utils::Span>,
    ) -> Self {
        let entry_block = BlockId::from(0u32);
        let mut blocks = IndexMap::new();
        blocks.insert(
            entry_block,
            BasicBlock {
                id: entry_block,
                instructions: Vec::new(),
                terminator: Terminator::Unreachable,
            },
        );

        Self {
            id,
            name,
            params,
            return_type,
            locals: IndexMap::new(),
            blocks,
            entry_block,
            span,
        }
    }

    pub fn add_local(&mut self, local: Local) -> LocalId {
        let id = local.id;
        self.locals.insert(id, local);
        id
    }

    pub fn block_mut(&mut self, id: BlockId) -> &mut BasicBlock {
        self.blocks.get_mut(&id).expect("invalid block id")
    }
}
