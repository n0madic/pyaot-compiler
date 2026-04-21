//! Core MIR structures: Module, Function, Local, BasicBlock

use std::cell::OnceCell;

use indexmap::IndexMap;
use pyaot_types::Type;
use pyaot_utils::{BlockId, ClassId, FuncId, InternedString, LocalId, Span};

use crate::dom_tree::DomTree;
use crate::{Instruction, Terminator};

/// Class metadata needed by optimizer passes (WPA field inference in
/// particular). Populated by lowering at the end of HIR→MIR, read — and
/// in-place refined — by `optimizer::type_inference::wpa_field_inference`.
/// Strictly a subset of `lowering::LoweredClassInfo`; only the parts the
/// optimizer needs.
#[derive(Debug, Clone)]
pub struct ClassMetadata {
    pub class_id: ClassId,
    /// `Some(init_func_id)` when the class defines `__init__`. Optimizer
    /// sees fields through the init body only.
    pub init_func_id: Option<FuncId>,
    /// Field name → storage offset (matches the `Constant::Int` operand
    /// passed to `rt_instance_set_field`).
    pub field_offsets: IndexMap<InternedString, usize>,
    /// Field name → (refinable) type. Starts at whatever lowering wrote;
    /// WPA field inference joins across all `__init__` call sites.
    pub field_types: IndexMap<InternedString, Type>,
    /// Single-inheritance parent, if any.
    pub base_class: Option<ClassId>,
    /// Whether this class is a Protocol and therefore should keep
    /// name-based virtual dispatch semantics rather than exact slot
    /// resolution from its own nominal type.
    pub is_protocol: bool,
}

/// Entry in a class vtable mapping slot to method function
#[derive(Debug, Clone)]
pub struct VtableEntry {
    pub slot: usize,
    pub name_hash: u64,
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
    /// Per-class metadata visible to optimizer passes. Populated by
    /// lowering at end of HIR→MIR; refined in place by
    /// `wpa_field_inference`.
    pub class_info: IndexMap<ClassId, ClassMetadata>,
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
    /// If true, the SSA property checker (`crate::ssa_check`) runs on this
    /// function and will fail the build on invariant violations. Default is
    /// `false`; Phase 1 of the architecture refactor flips individual
    /// functions to `true` after rewriting them in proper SSA form.
    pub is_ssa: bool,
    /// Lazily-computed dominator tree (Cooper–Harvey–Kennedy). Populated on
    /// first call to `dom_tree()`. CFG-mutating passes must call
    /// `invalidate_dom_tree()` to drop a stale cache.
    ///
    /// Marked `pub` with `#[doc(hidden)]` so external test crates can
    /// construct `Function` via struct literal (e.g. `OnceCell::new()`).
    /// Do not read or write this field directly — use `dom_tree()` and
    /// `invalidate_dom_tree()`.
    #[doc(hidden)]
    pub dom_tree_cache: OnceCell<DomTree>,
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
            class_info: IndexMap::new(),
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
            is_ssa: false,
            dom_tree_cache: OnceCell::new(),
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

    /// Memoised dominator tree over the current CFG. Computed on first call;
    /// call `invalidate_dom_tree()` after mutating block structure or
    /// terminators to force recomputation on the next query.
    pub fn dom_tree(&self) -> &DomTree {
        self.dom_tree_cache.get_or_init(|| DomTree::compute(self))
    }

    /// Drop the cached dominator tree. Every pass that adds, removes, or
    /// re-terminates blocks must call this before handing the function on.
    pub fn invalidate_dom_tree(&mut self) {
        self.dom_tree_cache.take();
    }
}
