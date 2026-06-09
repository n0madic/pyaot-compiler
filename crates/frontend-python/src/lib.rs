//! # frontend-python — parse + desugar → HIR
//!
//! Parses with `rustpython-parser` (`Mode::Module`) and lowers statements into a
//! CFG of [`HirBlock`]s. Module-level code becomes the synthetic `__main__`
//! function; `def`s become their own functions (Phase 2d). Short-circuit
//! `and`/`or`, ternaries, and chained comparisons are desugared here into branch
//! CFG + single-eval result locals (they are block-producing).
//!
//! Literal types are assigned here; every other node starts `SemTy::Dyn` and is
//! refined by `pyaot-typeck`. Any AST node kind outside the implemented subset
//! returns a [`CompilerError::parse_error`] — that error *is* the phase
//! allowlist.

#![forbid(unsafe_code)]

mod lower;

use rustpython_parser::ast::{Mod, Stmt};
use rustpython_parser::{parse as rustpython_parse, Mode};

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::HirModule;
use pyaot_utils::{Span, StringInterner};

use lower::ModuleLowerer;

/// Parse Python source into an [`HirModule`].
pub fn parse(src: &str, interner: &mut StringInterner) -> Result<HirModule> {
    let parsed = rustpython_parse(src, Mode::Module, "<input>").map_err(|e| {
        let off = e.offset.to_u32();
        CompilerError::parse_error(e.to_string(), Span::new(off, off))
    })?;

    let body: Vec<Stmt> = match parsed {
        Mod::Module(m) => m.body,
        _ => return Err(CompilerError::parse_error("expected a module", Span::dummy())),
    };

    ModuleLowerer::new(interner).lower_module(body)
}
