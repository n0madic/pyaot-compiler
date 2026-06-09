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
//! ## Locals
//!
//! Each function owns a flat [`HirLocal`] table; a [`pyaot_utils::LocalId`] is an
//! index into it. Parameters occupy the first `params.len()` slots, so HIR
//! `LocalId` maps 1:1 onto the MIR `LocalId` Repr table. The frontend allocates
//! every named local up front and refers to them by `Symbol::Local`; `typeck`
//! refines each [`HirLocal::ty`] (so `repr_of` can pick `Raw` for float/bool
//! locals) but the always-correct tagged baseline holds even if it does not.
//!
//! ## Name-resolution vocabulary
//!
//! [`SymbolRef`], [`Symbol`], and [`ResolveResult`] are the *shapes* of name
//! resolution and live here because [`SymbolRef`] is embedded directly in
//! [`HirExprKind::Name`]. The `pyaot-semantics` crate owns the *algorithm* that
//! fills them (`resolve`), while `pyaot-typeck` and `pyaot-lowering` consume the
//! result.

#![forbid(unsafe_code)]

use la_arena::{Arena, Idx};

use pyaot_types::SemTy;
use pyaot_utils::{FuncId, InternedString, LocalId, Span, SymbolId};

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

/// A function parameter. The annotation drives the parameter's `Repr` (and hence
/// the ABI). Parameters are also mirrored as the first locals.
#[derive(Debug, Clone)]
pub struct HirParam {
    pub name: InternedString,
    pub ty: SemTy,
}

/// A local slot. Index into [`HirFunction::locals`] is the [`LocalId`].
#[derive(Debug, Clone)]
pub struct HirLocal {
    pub name: InternedString,
    pub ty: SemTy,
}

/// A function: a flat `exprs` arena, a `locals` table, and a CFG of `blocks`.
#[derive(Debug)]
pub struct HirFunction {
    pub name: InternedString,
    pub params: Vec<HirParam>,
    pub ret_ty: SemTy,
    pub locals: Vec<HirLocal>,
    pub blocks: Arena<HirBlock>,
    pub entry: Idx<HirBlock>,
    pub exprs: Arena<HirExpr>,
}

impl HirFunction {
    pub fn local_ty(&self, id: LocalId) -> &SemTy {
        &self.locals[id.index()].ty
    }
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
/// [`Span`]. Literal types are set at parse time; everything else starts
/// `SemTy::Dyn` and is refined by `pyaot-typeck`.
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
    /// An integer literal that fits a tagged fixnum (`i64`, the codegen tags it).
    IntLit(i64),
    /// An integer literal too large for `i64`; carries the decimal text for
    /// `rt_bigint_from_str`.
    BigIntLit(InternedString),
    FloatLit(f64),
    BoolLit(bool),
    NoneLit,
    /// A name reference, resolved by `pyaot-semantics`.
    Name(SymbolRef),
    /// A direct reference to a local slot. The frontend emits this for reads it
    /// can resolve from its own scope (named locals it has allocated, and the
    /// synthetic result locals produced by desugaring short-circuit `and`/`or`,
    /// ternaries, and chained comparisons). Already resolved, so `semantics`
    /// leaves it untouched.
    Local(LocalId),
    /// A binary arithmetic / bitwise / shift operator (never short-circuiting).
    BinOp {
        op: BinOp,
        l: Idx<HirExpr>,
        r: Idx<HirExpr>,
    },
    /// A unary operator.
    Unary {
        op: UnaryOp,
        operand: Idx<HirExpr>,
    },
    /// A single comparison `l <op> r`. Chained comparisons are desugared by the
    /// frontend into short-circuit branch CFG with single-eval operands.
    Compare {
        op: CmpOp,
        l: Idx<HirExpr>,
        r: Idx<HirExpr>,
    },
    /// A call `callee(args...)`. Callee and args index this function's `exprs`.
    Call {
        callee: Idx<HirExpr>,
        args: Vec<Idx<HirExpr>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Pos,
    Invert,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    NotEq,
    Lt,
    LtE,
    Gt,
    GtE,
}

/// Short-circuit boolean operators. Used by the frontend's CFG desugaring; not a
/// standalone expression node (the desugaring produces a result local instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolOp {
    And,
    Or,
}

// ============================================================================
// Statements / terminators
// ============================================================================

#[derive(Debug, Clone)]
pub enum HirStmt {
    /// An expression evaluated for its side effects.
    Expr(Idx<HirExpr>),
    /// Assign `value` into a local. Augmented and multiple assignment desugar to
    /// a sequence of these in the frontend.
    Assign {
        target: LocalId,
        value: Idx<HirExpr>,
    },
    /// `assert cond` — the message expression (Phase 7) is dropped here.
    Assert { cond: Idx<HirExpr> },
    /// `print(args, sep=…, end=…)`. `print` is *the* special builtin: `sep`/`end`
    /// are string-literal options that a generic `Call` (no keywords field)
    /// cannot carry, so it gets a dedicated statement. `sep`/`end` are `None` for
    /// the defaults (`' '` between args, `'\n'` trailing); `Some` carries an
    /// interned literal (possibly empty). `typeck` infers each arg's type, and
    /// `lowering` expands this into the `MirInst::Print` sequence with per-arg
    /// `PrintKind` dispatch.
    Print {
        args: Vec<Idx<HirExpr>>,
        sep: Option<InternedString>,
        end: Option<InternedString>,
    },
}

/// How a block ends.
#[derive(Debug, Clone)]
pub enum HirTerminator {
    Return(Option<Idx<HirExpr>>),
    Jump(Idx<HirBlock>),
    Branch {
        cond: Idx<HirExpr>,
        then: Idx<HirBlock>,
        else_: Idx<HirBlock>,
    },
    /// Provably unreachable (e.g. the fall-through of an `assert` fail block).
    Unreachable,
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
/// `print` / `range` are **not** first-class builtins
/// (`BuiltinFunctionKind::from_name` returns `None`), so they get their own
/// variants — the honest home for their special-casing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symbol {
    BuiltinPrint,
    BuiltinRange,
    Builtin(BuiltinFunctionKind),
    Local(LocalId),
    Function(FuncId),
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
