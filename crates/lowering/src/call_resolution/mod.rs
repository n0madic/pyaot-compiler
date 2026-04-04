//! Call argument resolution
//!
//! This module handles the complex logic of resolving positional and keyword
//! arguments against function parameters, including:
//! - Runtime *args and **kwargs unpacking
//! - Default parameter handling
//! - Keyword-only parameters
//! - Building varargs tuples and kwargs dicts
#![allow(clippy::too_many_arguments)]

mod arg_binding;
mod default_params;
mod varargs;

use indexmap::IndexMap;
use pyaot_hir::{self as hir, ParamKind};
use pyaot_mir as mir;
use pyaot_utils::InternedString;

/// Result of matching keyword arguments to parameters.
/// Contains (kwonly_resolved, extra_keywords).
pub(crate) type KwargsMatchResult = (
    Vec<Option<mir::Operand>>,
    IndexMap<InternedString, mir::Operand>,
);

/// Parameters classified by their kind for easier handling.
pub(crate) struct ParamClassification<'a> {
    pub regular: Vec<&'a hir::Param>,
    pub vararg: Option<&'a hir::Param>,
    pub kwonly: Vec<&'a hir::Param>,
    pub kwarg: Option<&'a hir::Param>,
}

impl<'a> ParamClassification<'a> {
    /// Classify parameters by their kind.
    pub fn from_params(params: &'a [hir::Param]) -> Self {
        let mut regular = Vec::new();
        let mut vararg = None;
        let mut kwonly = Vec::new();
        let mut kwarg = None;

        for param in params {
            match param.kind {
                ParamKind::Regular => regular.push(param),
                ParamKind::VarPositional => vararg = Some(param),
                ParamKind::KeywordOnly => kwonly.push(param),
                ParamKind::VarKeyword => kwarg = Some(param),
            }
        }

        Self {
            regular,
            vararg,
            kwonly,
            kwarg,
        }
    }
}
