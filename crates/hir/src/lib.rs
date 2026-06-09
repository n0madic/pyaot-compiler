//! # HIR — High-level IR (CFG-only)
//!
//! Control-flow-graph IR: functions own `blocks`, an `entry` block, and a flat
//! `exprs` arena; structured control flow lives in an [`HirTerminator`], not in
//! nested statement variants. Generators are desugared into regular functions at
//! this level (Phase 6).
//!
//! Every typed slot carries a [`pyaot_types::SemTy`] **only** — physical
//! representation ([`pyaot_types::Repr`]) is assigned later at the lowering
//! boundary, never stored here. There is no representation-ambiguous `Any` here.
//!
//! ## Arena strategy
//!
//! * **Intra-function** `blocks` / `exprs` use [`la_arena`] (`Idx<_>` handles).
//! * **Cross-function** references use the [`pyaot_utils::FuncId`] newtype: a
//!   module's `functions` is a `Vec<HirFunction>` indexed by `FuncId`, so a
//!   function's identity survives unchanged across the lowering boundary into
//!   MIR (HIR `FuncId` == MIR `FuncId`).
//!
//! ## Name-resolution vocabulary
//!
//! [`SymbolRef`], [`Symbol`], and [`ResolveResult`] are the *shapes* of name
//! resolution and live here because [`SymbolRef`] is embedded directly in
//! [`HirExprKind::Name`]. The `pyaot-semantics` crate owns the *algorithm* that
//! fills them (`resolve`), while `pyaot-typeck` and `pyaot-lowering` consume the
//! result — all three already depend on this crate, so the vocabulary lives at
//! the shared root rather than forcing extra cross-crate dependencies.
//!
//! ## Phase 1 scope
//!
//! Only the seam shapes needed to push `print("hello")` through the pipeline
//! exist. Reserved for later phases (intentionally absent, not forgotten):
//! `try_scopes`, branch/loop terminator variants, a unified `BindingTarget`,
//! and the bulk of [`HirExprKind`] / [`HirStmt`] variants.

#![forbid(unsafe_code)]

use la_arena::{Arena, Idx};

use pyaot_types::SemTy;
use pyaot_utils::{FuncId, InternedString, Span, SymbolId};

// Re-exported so the resolution-vocabulary consumers (`semantics`) can name
// `Symbol::Builtin`'s payload without each taking a direct `core-defs` dep.
pub use pyaot_core_defs::BuiltinFunctionKind;

// ============================================================================
// Module / function structure
// ============================================================================

/// A whole compilation unit. Module-level code is lowered into a synthetic
/// `__main__` function (named by [`HirModule::main`]) — the one function codegen
/// wraps in the C `main`, and exactly what Phase 8 module-body execution reuses.
#[derive(Debug)]
pub struct HirModule {
    /// All functions, indexed by [`FuncId`]. `__main__` is one of these.
    pub functions: Vec<HirFunction>,
    /// The synthetic module-body function.
    pub main: FuncId,
}

impl HirModule {
    pub fn function(&self, id: FuncId) -> &HirFunction {
        &self.functions[id.index()]
    }

    pub fn function_mut(&mut self, id: FuncId) -> &mut HirFunction {
        &mut self.functions[id.index()]
    }
}

/// A function parameter. (Phase 1 emits none; reserved for Phase 2 functions.)
#[derive(Debug, Clone)]
pub struct HirParam {
    pub name: InternedString,
    pub ty: SemTy,
}

/// A single function: a flat `exprs` arena plus a CFG of `blocks`.
#[derive(Debug)]
pub struct HirFunction {
    pub name: InternedString,
    pub params: Vec<HirParam>,
    pub ret_ty: SemTy,
    pub blocks: Arena<HirBlock>,
    pub entry: Idx<HirBlock>,
    pub exprs: Arena<HirExpr>,
}

/// A basic block: a straight-line list of statements ending in exactly one
/// terminator.
#[derive(Debug)]
pub struct HirBlock {
    pub stmts: Vec<HirStmt>,
    pub term: HirTerminator,
}

// ============================================================================
// Expressions
// ============================================================================

/// An expression node. Carries its [`SemTy`] (semantic type **only**) and source
/// [`Span`]. The literal's type is set at parse time; everything else is left
/// `SemTy::Dyn` for `pyaot-typeck` to refine.
#[derive(Debug, Clone)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub ty: SemTy,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum HirExprKind {
    /// A string literal; bytes are interned.
    StrLit(InternedString),
    /// A call `callee(args...)`. Callee and args index this function's `exprs`.
    Call {
        callee: Idx<HirExpr>,
        args: Vec<Idx<HirExpr>>,
    },
    /// A name reference, resolved by `pyaot-semantics`.
    Name(SymbolRef),
    // Reserved: NumLit, BoolLit, NoneLit, BinOp, Attribute, Subscript, ...
}

// ============================================================================
// Statements / terminators
// ============================================================================

#[derive(Debug, Clone)]
pub enum HirStmt {
    /// An expression evaluated for its side effects.
    Expr(Idx<HirExpr>),
    // Reserved: Assign, Return-as-stmt is modelled by the terminator instead, ...
}

/// How a block ends. Phase 1 only needs `Return`.
#[derive(Debug, Clone)]
pub enum HirTerminator {
    Return(Option<Idx<HirExpr>>),
    // Reserved: Branch { cond, then, else_ }, Jump(BlockId), Unreachable, ...
}

// ============================================================================
// Name-resolution vocabulary (shapes; algorithm lives in pyaot-semantics)
// ============================================================================

/// A name occurrence. The parser emits [`SymbolRef::Unresolved`]; `semantics`
/// rewrites it in place to [`SymbolRef::Resolved`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolRef {
    Unresolved(InternedString),
    Resolved(SymbolId),
}

/// A resolved symbol.
///
/// `print` is **not** a first-class builtin (`BuiltinFunctionKind::from_name`
/// returns `None` for it), so it gets its own variant — this is the honest home
/// for the `print` special-case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symbol {
    BuiltinPrint,
    Builtin(BuiltinFunctionKind),
    // Reserved: Local(LocalId), Global(..), Function(FuncId), Class(ClassId), ...
}

/// The output of name resolution: a table of [`Symbol`]s indexed by
/// [`SymbolId`]. `semantics` produces it; `typeck` and `lowering` consume it.
#[derive(Debug, Default)]
pub struct ResolveResult {
    symbols: Vec<Symbol>,
}

impl ResolveResult {
    pub fn new() -> Self {
        Self { symbols: Vec::new() }
    }

    /// Intern a resolved symbol, returning its [`SymbolId`].
    pub fn intern(&mut self, sym: Symbol) -> SymbolId {
        let id = SymbolId::new(self.symbols.len() as u32);
        self.symbols.push(sym);
        id
    }

    pub fn symbol(&self, id: SymbolId) -> Symbol {
        self.symbols[id.index()]
    }
}
