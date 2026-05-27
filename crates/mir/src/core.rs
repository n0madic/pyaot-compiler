//! Core MIR structures: Module, Function, Local, BasicBlock

use std::cell::OnceCell;

use indexmap::IndexMap;
use pyaot_types::{Type, TypeVarDef};
use pyaot_utils::{BlockId, ClassId, FuncId, InternedString, LocalId, Span};
use std::collections::HashMap;

use crate::dom_tree::DomTree;
use crate::{Instruction, Terminator};

/// Explicit classification of what kind of function this MIR `Function`
/// represents.  Replaces the string-prefix heuristics that previously
/// scattered `name.starts_with("__lambda_")` / `.contains('$')` checks
/// across the codebase (Stage E.5 of the Strong-Typed MIR Rewrite plan).
///
/// Lowering sets this at construction time; all downstream passes (optimizer,
/// verifier, codegen) read `kind` instead of inspecting the name string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionKind {
    /// Ordinary top-level or module-level function (`def foo(...)`).
    Regular,
    /// Explicit lambda (`lambda x: x`) or nested closure lifted by the
    /// frontend (`__lambda_*` / `__nested_*` name prefix).
    Lambda,
    /// Generator-expression creator (`(x for x in xs)` desugared to a
    /// `__genexp_*` function).  Receives its captures as implicit leading
    /// params rather than via the closure-tuple ABI.
    GenexpCreator,
    /// Method defined inside a `class` block (`ClassName$method` name pattern).
    ClassMethod,
    /// The synthetic `__pyaot_module_init__` entry-point that evaluates
    /// module-level statements.
    ModuleInit,
    /// The coroutine resume trampoline synthesised by generator desugaring
    /// (function id ≥ `RESUME_FUNC_ID_OFFSET`).
    GeneratorResume,
}

impl FunctionKind {
    /// Infer `FunctionKind` from a function name and its numeric id, mirroring
    /// exactly the heuristics that were previously spread across the codebase:
    ///
    /// * `__lambda_*` / `__nested_*` → `Lambda`
    /// * `__genexp_*`                → `GenexpCreator`
    /// * contains `$`               → `ClassMethod`
    /// * `__pyaot_module_init__`     → `ModuleInit`
    /// * id ≥ `RESUME_FUNC_ID_OFFSET`→ `GeneratorResume`
    /// * everything else             → `Regular`
    pub fn from_name_and_id(name: &str, id: pyaot_utils::FuncId) -> Self {
        if name.starts_with("__lambda_") || name.starts_with("__nested_") {
            return FunctionKind::Lambda;
        }
        if name.starts_with("__genexp_") {
            return FunctionKind::GenexpCreator;
        }
        if name == "__pyaot_module_init__" {
            return FunctionKind::ModuleInit;
        }
        if id.0 >= pyaot_utils::RESUME_FUNC_ID_OFFSET {
            return FunctionKind::GeneratorResume;
        }
        if name.contains('$') {
            return FunctionKind::ClassMethod;
        }
        FunctionKind::Regular
    }
}

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
    /// After Phase 2 storage uniformity, `field_types` is the single
    /// source of truth — refined types from cross-instance writes are
    /// folded into it post type-planning, so all reads/writes route
    /// through the generic tagged-`Value` path.
    pub field_types: IndexMap<InternedString, Type>,
    /// **Strong-Typed MIR Rewrite (Phase 2c)**: parallel MirType field
    /// for the storage representation of each field. When `Some(mt)`,
    /// the verifier and codegen consult this directly; when `None`,
    /// fallback is `type_to_mir_type_storage(&field_types[name])`.
    /// Phase 6 deletes `field_types` and promotes this to the sole
    /// representation source.
    pub field_mir_types: IndexMap<InternedString, crate::types::MirType>,
    /// Single-inheritance parent, if any.
    pub base_class: Option<ClassId>,
    /// Whether this class is a Protocol and therefore should keep
    /// name-based virtual dispatch semantics rather than exact slot
    /// resolution from its own nominal type.
    pub is_protocol: bool,
}

impl ClassMetadata {
    /// Resolved MirType for a class field. Returns explicit
    /// `field_mir_types` entry if present, otherwise translates the
    /// logical `field_types[name]` at storage level (primitives →
    /// `Tagged`). Returns `None` if the field doesn't exist.
    pub fn resolved_field_mir_type(&self, name: &InternedString) -> Option<crate::types::MirType> {
        if let Some(mt) = self.field_mir_types.get(name) {
            return Some(mt.clone());
        }
        self.field_types
            .get(name)
            .map(crate::types::type_to_mir_type_storage)
    }
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
    /// TypeVar definitions from the source module, threaded through from
    /// `hir::Module` for the monomorphizer (S3.3).
    pub typevar_defs: HashMap<InternedString, TypeVarDef>,
}

/// MIR Function with CFG
#[derive(Debug, Clone)]
pub struct Function {
    pub id: FuncId,
    pub name: String,
    /// Explicit function classification.  Set by lowering at construction time
    /// via `FunctionKind::from_name_and_id`; inherited by monomorphize clones.
    /// All downstream passes read this instead of inspecting the name string.
    pub kind: FunctionKind,
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
    /// True when this function has at least one `Type::Var` in its parameter
    /// or return type — it is a generic template awaiting monomorphisation
    /// (S3.3). WPA skips templates; the MonomorphizePass clones them per
    /// concrete call-site and clears this flag on every specialisation.
    pub is_generic_template: bool,
    /// Distinct TypeVar names used in the function signature.
    /// Empty when `is_generic_template` is false.
    pub typevar_params: Vec<InternedString>,
    /// `Some(idx)` when this function is a decorator wrapper — `idx` is the
    /// position of its fn-pointer parameter (the captured original function),
    /// always 0 in the current ABI. This is a *structural* template marker,
    /// orthogonal to `is_generic_template`: wrapper'ы не имеют `Type::Var` в
    /// сигнатуре, но семантически параметрические по сигнатуре захваченной
    /// функции. `MonomorphizePass` (S3.3b.2) использует это поле для
    /// специализации wrapper'ов per captured-fn-id.
    pub wrapper_fn_ptr_capture_index: Option<usize>,
    /// Phase 4 (Storage-Uniform Commit 4): true when lowering applied the
    /// return-ABI flip — the function's `return_type` field is now
    /// `Type::Any` and the function body contains a `BoxValue` before
    /// each `Terminator::Return` operand for primitive return types.
    /// Callers receive a tagged Value and downstream consumers must
    /// `UnboxValue` if they want the underlying primitive. WPA's
    /// `materialize_function_return_types` reads this flag to protect
    /// the `Type::Any` return from re-narrowing (the body's Return
    /// operands carry primitive types from the un-flipped state, which
    /// would otherwise be observed by `infer_function_return_type`).
    pub phase4_return_abi_flipped: bool,
    /// Phase 4 Commit 4: the *original* primitive return type before
    /// the flip (one of `Int` / `Bool` / `Float`). Recorded only when
    /// `phase4_return_abi_flipped == true`. Consumed by the
    /// post-merge rewriter `rewrite_phase4_callee_returns` so it can
    /// retype an `Any`-typed CallDirect dest (multi-module path, where
    /// the caller's lowering didn't see the callee's `func_def`) back
    /// to the original primitive shape and insert `UnboxValue`.
    pub phase4_original_return_type: Option<Type>,
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
    /// **Strong-Typed MIR Rewrite (Phase 2b)**: parallel typed
    /// signature for this function. When `Some(sig)`, this is the
    /// canonical ABI contract; when `None`, derive from `params` +
    /// `return_type` via the legacy `Type` field.
    ///
    /// Phase 2b populates this for new functions; Phase 4 codegen
    /// uses it to emit Cranelift signatures. Phase 6 deletes the
    /// legacy `return_type` field once all consumers migrate.
    pub signature: Option<crate::types::Signature>,
}

impl Function {
    /// True when the function is lambda-like (explicit lambda or nested
    /// closure lifted by the frontend).  Reads `kind` — no string scan.
    pub fn is_lambda_like(&self) -> bool {
        self.kind == FunctionKind::Lambda
    }

    /// True when the function is a generator-expression creator.
    /// Reads `kind` — no string scan.
    pub fn is_genexp_creator(&self) -> bool {
        self.kind == FunctionKind::GenexpCreator
    }

    /// True when the function is a class method (`ClassName$method` pattern).
    /// Reads `kind` — no string scan.
    pub fn is_class_method(&self) -> bool {
        self.kind == FunctionKind::ClassMethod
    }

    /// Resolved typed signature. Returns explicit `signature` if set,
    /// otherwise builds one on-the-fly from legacy `params` +
    /// `return_type` via **register-level translation on both sides**.
    ///
    /// Legacy MIR ABI passes raw primitive values across `CallDirect`
    /// edges (e.g., raw `i64` for `Type::Int`). The fallback signature
    /// therefore maps primitives to `Raw(K)` for both params and
    /// return, matching legacy codegen. Phase 2b functions that
    /// populate `signature` explicitly use the canonical Tagged ABI
    /// (storage-level); the verifier consults `signature` first.
    pub fn resolved_signature(&self) -> crate::types::Signature {
        if let Some(sig) = &self.signature {
            return sig.clone();
        }
        let params: Vec<crate::types::MirType> =
            self.params.iter().map(|p| p.resolved_mir_type()).collect();
        let return_type = crate::types::type_to_mir_type_register(&self.return_type);
        crate::types::Signature {
            params,
            return_type,
        }
    }
}

/// Local variable in MIR
#[derive(Debug, Clone)]
pub struct Local {
    pub id: LocalId,
    pub name: Option<InternedString>,
    pub ty: Type,
    /// Phase 4+ Extension Step E1 (per-param ABI granularity): true for
    /// function-parameter Locals whose `ty == Type::Any` is part of the
    /// ABI contract — narrowing them back to a primitive would invalidate
    /// the lowering-emitted prologue `UnboxValue` or capture-prologue
    /// unbox sequence. Set by lowering at the param-construction site
    /// whenever `needs_prologue_unbox` is true (lambda captures, lambda
    /// user-param Phase 4 flip, regular function user-param Phase 4 flip,
    /// generator resume frame state). WPA's `refine_function_params` and
    /// `materialize_function_types` skip narrowing for these Locals.
    /// Body-local Locals (allocated by `add_local`) always default to
    /// `false`; only params ever set this flag to `true`.
    pub abi_immutable: bool,
    /// Stage F.2 pre-flight: true for Locals that back a HIR-level
    /// program variable (allocated via `Lowering::get_or_create_local`
    /// or equivalent). False for compiler-synthesized temporaries
    /// (allocated via `Lowering::alloc_and_add_local` or downstream
    /// pass-temps in `abi_repair`, monomorphize, SSA construction etc.).
    ///
    /// Replaces the legacy `mir_ty: None` sentinel that previously
    /// distinguished these two classes. Used by
    /// `Lowering::operand_is_guaranteed_tagged` (and the BinOp/Compare
    /// dispatchers that consult it) to decide whether an `Any`-typed
    /// operand can safely route through `rt_obj_*` (compiler temps,
    /// always tagged) or must use the raw BinOp path (program variables
    /// that may carry raw primitive bits from legacy trampolines).
    pub is_var_local: bool,
    /// Strong-Typed MIR Rewrite (Phase 2): authoritative physical
    /// representation type. When `Some`, this is the canonical type and
    /// the verifier checks against it. When `None`, the verifier falls
    /// back to translating `self.ty` (a logical `pyaot_types::Type`)
    /// via `type_to_mir_type_register`. Lowering sub-steps 2a-2f
    /// progressively populate this field; Phase 6 deletes `self.ty`
    /// and promotes `mir_ty` to the sole representation source.
    ///
    /// Stored as `Option` so the field can be `None` for legacy Locals
    /// that haven't been migrated yet; this allows the migration to
    /// proceed subsystem-by-subsystem without one massive cascade.
    pub mir_ty: Option<crate::types::MirType>,
}

impl Local {
    /// Resolved physical representation type. Returns `self.mir_ty` if
    /// populated (Phase 2+ migration), otherwise falls back to
    /// translating `self.ty` at register-level interpretation
    /// (primitives → `Raw(K)`).
    ///
    /// Used by the Verifier and by code that has migrated to MirType
    /// awareness. Legacy code continues to read `self.ty` directly.
    pub fn resolved_mir_type(&self) -> crate::types::MirType {
        if let Some(t) = &self.mir_ty {
            return t.clone();
        }
        crate::types::type_to_mir_type_register(&self.ty)
    }

    /// Single source of truth for GC-rooting: delegates to
    /// [`MirType::needs_gc_root`] via [`Self::resolved_mir_type`].
    ///
    /// Rules:
    /// * `Heap(_)` → true (heap pointer must be tracked)
    /// * `Tagged` → true (may carry a heap pointer — Box(Float) etc.)
    /// * `Closure(_)` → true (closure tuple itself is `gc_alloc`-allocated)
    /// * `FuncPtr(_)` → false (text-segment code pointer; not GC-managed)
    /// * `Raw(_)` → false (primitive — no GC tracking)
    /// * `Var(_)` → conservative `false` (TypeVar is erased before codegen)
    /// * `Never` → false (control doesn't reach)
    ///
    /// Sites that need a "Tagged slot, not GC-tracked" override (e.g. raw
    /// code addresses transiently held before boxing) must allocate the
    /// Local with `MirType::FuncPtr(sig)` or `MirType::Raw(I64)` via
    /// `alloc_and_add_local_with_mir_ty` — the previous `is_gc_root` bool
    /// field has been removed; the MirType is now the only signal.
    pub fn computed_is_gc_root(&self) -> bool {
        self.resolved_mir_type().needs_gc_root()
    }

    /// Stage F.2 helper — true iff `resolved_mir_type()` contains any
    /// `Var(_)` leaf. Mirrors `Type::contains_var` for callers migrating
    /// off `local.ty.contains_var()`.
    pub fn resolved_contains_var(&self) -> bool {
        self.resolved_mir_type().contains_var()
    }
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
            typevar_defs: HashMap::new(),
        }
    }

    pub fn add_function(&mut self, func: Function) {
        self.functions.insert(func.id, func);
    }

    /// Stage B.1 of Strong-Typed MIR Rewrite plan v2: resolve a vtable
    /// slot to the typed callee signature. Walks the inheritance chain
    /// when the slot is not defined locally on the class.
    ///
    /// Returns `None` when the class is unknown, the slot is out of
    /// bounds, or the referenced function id is not in this module
    /// (e.g., cross-module call resolved later).
    pub fn vtable_slot_signature(
        &self,
        class_id: pyaot_utils::ClassId,
        slot: usize,
    ) -> Option<crate::types::Signature> {
        let mut cur_id = Some(class_id);
        while let Some(cid) = cur_id {
            if let Some(vt) = self.vtables.iter().find(|v| v.class_id == cid) {
                if let Some(entry) = vt.entries.iter().find(|e| e.slot == slot) {
                    if let Some(func) = self.functions.get(&entry.method_func_id) {
                        return Some(func.resolved_signature());
                    }
                    return None;
                }
            }
            cur_id = self.class_info.get(&cid).and_then(|cm| cm.base_class);
        }
        None
    }

    /// Stage B.1: resolve the function id for a vtable slot. Useful for
    /// devirt and verifier when the signature alone is not enough.
    pub fn vtable_slot_func_id(
        &self,
        class_id: pyaot_utils::ClassId,
        slot: usize,
    ) -> Option<FuncId> {
        let mut cur_id = Some(class_id);
        while let Some(cid) = cur_id {
            if let Some(vt) = self.vtables.iter().find(|v| v.class_id == cid) {
                if let Some(entry) = vt.entries.iter().find(|e| e.slot == slot) {
                    return Some(entry.method_func_id);
                }
            }
            cur_id = self.class_info.get(&cid).and_then(|cm| cm.base_class);
        }
        None
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

        // Boundary coercion: see `add_local`. Function params and the
        // return type travel the same path to `type_to_cranelift` and
        // must be free of `Never` (top-level or container parameter).
        let demote = |t: Type| -> Type {
            match t {
                Type::Never => Type::Any,
                other => other.demote_never_params_to_any(),
            }
        };
        let params: Vec<Local> = params
            .into_iter()
            .map(|p| Local {
                ty: demote(p.ty),
                ..p
            })
            .collect();
        let return_type = demote(return_type);

        let kind = FunctionKind::from_name_and_id(&name, id);
        Self {
            id,
            kind,
            name,
            params,
            return_type,
            locals: IndexMap::new(),
            blocks,
            entry_block,
            span,
            is_ssa: false,
            is_generic_template: false,
            typevar_params: Vec::new(),
            wrapper_fn_ptr_capture_index: None,
            phase4_return_abi_flipped: false,
            phase4_original_return_type: None,
            dom_tree_cache: OnceCell::new(),
            signature: None,
        }
    }

    pub fn add_local(&mut self, mut local: Local) -> LocalId {
        // Boundary coercion at the absolute MIR-input edge. Lowering's
        // empty-literal seed pipeline produces `list[Never]` /
        // `dict[Never, Never]` etc. so `TypeLattice::join` correctly
        // narrows through usage observation (`Never` is bottom, identity
        // in `join`). When an empty container is never refined by usage,
        // the residual `Never` must not reach codegen — `type_to_cranelift`
        // panics on `Type::Never`. We demote here so every direct or
        // indirect `add_local` call gets the same treatment.
        local.ty = match local.ty {
            pyaot_types::Type::Never => pyaot_types::Type::Any,
            other => other.demote_never_params_to_any(),
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    use pyaot_utils::FuncId;

    /// Build a `Function` with a non-resume id (0) so `from_name_and_id`
    /// doesn't accidentally classify it as `GeneratorResume`.
    fn mk(name: &str) -> Function {
        Function::new(
            FuncId::from(0u32),
            name.to_string(),
            Vec::new(),
            pyaot_types::Type::None,
            None,
        )
    }

    /// Build a `Function` with an explicit id (for resume-offset tests).
    fn mk_id(id: u32, name: &str) -> Function {
        Function::new(
            FuncId::from(id),
            name.to_string(),
            Vec::new(),
            pyaot_types::Type::None,
            None,
        )
    }

    // ── FunctionKind::from_name_and_id unit tests ─────────────────────────

    #[test]
    fn kind_lambda_from_lambda_prefix() {
        assert_eq!(
            FunctionKind::from_name_and_id("__lambda_3", FuncId::from(0u32)),
            FunctionKind::Lambda
        );
        assert_eq!(
            FunctionKind::from_name_and_id("__lambda_0", FuncId::from(0u32)),
            FunctionKind::Lambda
        );
    }

    #[test]
    fn kind_lambda_from_nested_prefix() {
        assert_eq!(
            FunctionKind::from_name_and_id("__nested_func", FuncId::from(0u32)),
            FunctionKind::Lambda
        );
    }

    #[test]
    fn kind_genexp_from_genexp_prefix() {
        assert_eq!(
            FunctionKind::from_name_and_id("__genexp_0", FuncId::from(0u32)),
            FunctionKind::GenexpCreator
        );
        assert_eq!(
            FunctionKind::from_name_and_id("__genexp_42", FuncId::from(0u32)),
            FunctionKind::GenexpCreator
        );
    }

    #[test]
    fn kind_module_init_exact_match() {
        assert_eq!(
            FunctionKind::from_name_and_id("__pyaot_module_init__", FuncId::from(0u32)),
            FunctionKind::ModuleInit
        );
    }

    #[test]
    fn kind_generator_resume_from_id_offset() {
        let resume_id = pyaot_utils::RESUME_FUNC_ID_OFFSET;
        assert_eq!(
            FunctionKind::from_name_and_id("some_func_resume", FuncId::from(resume_id)),
            FunctionKind::GeneratorResume
        );
        assert_eq!(
            FunctionKind::from_name_and_id("some_func_resume", FuncId::from(resume_id + 5)),
            FunctionKind::GeneratorResume
        );
    }

    #[test]
    fn kind_class_method_from_dollar_in_name() {
        assert_eq!(
            FunctionKind::from_name_and_id("MyClass$__init__", FuncId::from(0u32)),
            FunctionKind::ClassMethod
        );
        assert_eq!(
            FunctionKind::from_name_and_id("Foo$bar", FuncId::from(0u32)),
            FunctionKind::ClassMethod
        );
    }

    #[test]
    fn kind_regular_for_plain_names() {
        assert_eq!(
            FunctionKind::from_name_and_id("regular_function", FuncId::from(0u32)),
            FunctionKind::Regular
        );
        assert_eq!(
            FunctionKind::from_name_and_id("main", FuncId::from(0u32)),
            FunctionKind::Regular
        );
    }

    // ── is_* predicate tests (read from kind field, not string) ──────────

    #[test]
    fn is_lambda_like_matches_lambda_and_nested() {
        assert!(mk("__lambda_3").is_lambda_like());
        assert!(mk("__lambda_0").is_lambda_like());
        assert!(mk("__nested_func").is_lambda_like());
        assert!(!mk("regular_function").is_lambda_like());
        assert!(!mk("__genexp_5").is_lambda_like());
        assert!(!mk("Class$method").is_lambda_like());
    }

    #[test]
    fn is_genexp_creator_matches_genexp_prefix() {
        assert!(mk("__genexp_0").is_genexp_creator());
        assert!(mk("__genexp_42").is_genexp_creator());
        assert!(!mk("__lambda_0").is_genexp_creator());
        assert!(!mk("regular").is_genexp_creator());
    }

    #[test]
    fn is_class_method_matches_dollar_in_name() {
        assert!(mk("MyClass$__init__").is_class_method());
        assert!(mk("Foo$bar").is_class_method());
        assert!(!mk("regular_function").is_class_method());
        assert!(!mk("__lambda_0").is_class_method());
    }

    #[test]
    fn function_new_sets_kind_via_heuristic() {
        assert_eq!(mk("__lambda_1").kind, FunctionKind::Lambda);
        assert_eq!(mk("__nested_x").kind, FunctionKind::Lambda);
        assert_eq!(mk("__genexp_0").kind, FunctionKind::GenexpCreator);
        assert_eq!(mk("__pyaot_module_init__").kind, FunctionKind::ModuleInit);
        assert_eq!(mk("Foo$bar").kind, FunctionKind::ClassMethod);
        assert_eq!(mk("regular").kind, FunctionKind::Regular);
        let resume_id = pyaot_utils::RESUME_FUNC_ID_OFFSET;
        assert_eq!(
            mk_id(resume_id, "foo_resume").kind,
            FunctionKind::GeneratorResume
        );
    }
}
