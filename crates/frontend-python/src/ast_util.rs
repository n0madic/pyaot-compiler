//! Small shared iterators over rustpython AST nodes — the verbatim traversal
//! idioms that recur across the free-variable analysis ([`crate::freevars`]) and
//! the lowering's AST pre-scans ([`crate::lower`]). These dedupe the *shape* of
//! a traversal step (which children a node has); the surrounding analyses stay
//! distinct (see the per-walker leaf logic in those modules).

use rustpython_parser::ast::{
    ArgWithDefault, Arguments, ExceptHandler, ExceptHandlerExceptHandler,
};

/// The positional / positional-or-keyword / keyword-only parameters of an
/// `Arguments` node, in declaration order. EXCLUDES `*args` / `**kwargs` (which
/// are `Arg`, carrying no default). The single source of the
/// `posonlyargs.iter().chain(&args).chain(&kwonlyargs)` idiom — callers project
/// each `ArgWithDefault` to its name (`awd.def.arg`), default, or annotation.
/// Centralizing it removes the latent footgun of forgetting one of the three
/// lists at a new site.
pub(crate) fn defaultable_params(args: &Arguments) -> impl Iterator<Item = &ArgWithDefault> {
    args.posonlyargs
        .iter()
        .chain(&args.args)
        .chain(&args.kwonlyargs)
}

/// Every parameter default of an `Arguments` node, in declaration order — the
/// `defaultable_params` projection used by the AST pre-scans that evaluate
/// defaults in the enclosing scope.
pub(crate) fn param_defaults(
    args: &Arguments,
) -> impl Iterator<Item = &rustpython_parser::ast::Expr> {
    defaultable_params(args).filter_map(|awd| awd.default.as_deref())
}

/// A `try`/`try*` handler list, each handler already unwrapped from the
/// single-variant [`ExceptHandler`] enum — replaces the irrefutable
/// `let ExceptHandler::ExceptHandler(h) = h;` destructure at every handler loop.
/// Takes the `&[ExceptHandler]` slice (not the `StmtTry`) so it covers `try`,
/// `try*`, and pre-borrowed handler slices alike.
pub(crate) fn try_handlers(
    handlers: &[ExceptHandler],
) -> impl Iterator<Item = &ExceptHandlerExceptHandler> {
    handlers.iter().map(|h| {
        let ExceptHandler::ExceptHandler(h) = h;
        h
    })
}
