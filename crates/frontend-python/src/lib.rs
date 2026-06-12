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
//!
//! ## Multi-module (Phase 8)
//!
//! [`parse_program`] drives import-graph discovery (DFS from the entry script)
//! and lowers *every* module into ONE shared [`HirModule`] with globally
//! allocated FuncIds / ClassIds / gen_ids / promoted-var-slots — no merge or
//! remap pass. A [`NamespaceTable`] records per-module name scopes. [`parse`] is
//! the degenerate single-file case (one module, a loader that has no modules).

#![forbid(unsafe_code)]

mod freevars;
mod lower;

use rustpython_parser::ast::{Mod, Stmt};
use rustpython_parser::{parse as rustpython_parse, Mode};

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{HirModule, HirProgram};
use pyaot_utils::{Span, StringInterner};

use lower::ProgramLowerer;

/// Provides module source text to the import-graph driver (Phase 8). The CLI's
/// implementation searches the entry script's directory; tests with no imports
/// supply a [`NoModules`] loader.
pub trait ModuleSource {
    /// Resolve a dotted module path (`["pkg", "mod"]`) to its source text and
    /// whether it is a package (`__init__.py`). `None` if the module is not found.
    fn load(&mut self, path: &[String]) -> Option<(String, bool)>;

    /// Display path for traceback `File "…"` lines. The default renders the
    /// dotted path as a relative file name; the CLI loader overrides it with
    /// the real resolved path.
    fn display_path(&self, path: &[String], is_package: bool) -> String {
        if is_package {
            format!("{}/__init__.py", path.join("/"))
        } else {
            format!("{}.py", path.join("/"))
        }
    }
}

/// A loader with no modules — every import fails. Used by [`parse`] (single-file
/// programs never import a user module; `typing` / `__future__` are intercepted
/// before the loader is consulted).
pub struct NoModules;

impl ModuleSource for NoModules {
    fn load(&mut self, _path: &[String]) -> Option<(String, bool)> {
        None
    }
}

/// Parse Python source into an [`HirModule`] (single-file, no user imports).
pub fn parse(src: &str, interner: &mut StringInterner) -> Result<HirModule> {
    let mut loader = NoModules;
    let program = parse_program(src, "<input>", &mut loader, interner)?;
    Ok(program.module)
}

/// Parse a whole program: discover the import graph from `src` (the entry
/// script) via `loader`, and lower every reachable module into one shared
/// [`HirProgram`] (Phase 8). `entry_file` is the entry script's display path
/// for traceback `File "…"` attribution (the CLI passes the path as given).
pub fn parse_program(
    src: &str,
    entry_file: &str,
    loader: &mut dyn ModuleSource,
    interner: &mut StringInterner,
) -> Result<HirProgram> {
    let body = parse_module_body(src)?;
    ProgramLowerer::new(interner, loader).run(body, src, entry_file)
}

/// Parse a module's source text into its top-level statement list.
pub(crate) fn parse_module_body(src: &str) -> Result<Vec<Stmt>> {
    let parsed = rustpython_parse(src, Mode::Module, "<input>").map_err(|e| {
        let off = e.offset.to_u32();
        CompilerError::parse_error(e.to_string(), Span::new(off, off))
    })?;
    match parsed {
        Mod::Module(m) => Ok(m.body),
        _ => Err(CompilerError::parse_error(
            "expected a module",
            Span::dummy(),
        )),
    }
}
