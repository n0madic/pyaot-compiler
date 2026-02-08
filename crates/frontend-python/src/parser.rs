//! Python parser wrapper

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_utils::Span;
use rustpython_parser as rpy;

pub fn parse_module(source: &str) -> Result<rpy::ast::Mod> {
    rpy::parse(source, rpy::Mode::Module, "<module>").map_err(|e| {
        let offset = u32::from(e.offset);
        CompilerError::parse_error(
            e.error.to_string(),
            Span::new(offset, offset.saturating_add(1)),
        )
    })
}

pub fn parse_expr(source: &str) -> Result<rpy::ast::Expr> {
    match rpy::parse(source, rpy::Mode::Expression, "<expr>").map_err(|e| {
        let offset = u32::from(e.offset);
        CompilerError::parse_error(
            e.error.to_string(),
            Span::new(offset, offset.saturating_add(1)),
        )
    })? {
        rpy::ast::Mod::Expression(expr) => Ok(*expr.body),
        _ => Err(CompilerError::parse_error(
            "Expected expression",
            Span::new(0, 1),
        )),
    }
}
