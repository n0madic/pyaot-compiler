//! # Semantics — name resolution, scopes, MRO
//!
//! Resolves [`SymbolRef::Unresolved`] occurrences in the HIR to concrete
//! [`Symbol`]s, rewriting each in place to [`SymbolRef::Resolved`] and recording
//! the symbol table in a [`ResolveResult`] for `typeck` / `lowering` to consume.
//!
//! This is the honest home for the `print` special-case: `print` is **not** a
//! first-class builtin (`BuiltinFunctionKind::from_name("print") == None`), so it
//! is resolved to [`Symbol::BuiltinPrint`] here rather than being faked into the
//! builtin table.
//!
//! ## Phase 1 scope
//!
//! Flat top-level resolution of `print` and the first-class builtins. Scopes,
//! transitive closure-capture (free-variable bubbling), and C3 MRO linearization
//! are reserved — their shapes will hang off [`ResolveResult`] as it grows.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{BuiltinFunctionKind, HirExprKind, HirModule, ResolveResult, Symbol, SymbolRef};
use pyaot_utils::{InternedString, Span, StringInterner, SymbolId};

/// Resolve every name in `module`, mutating each [`SymbolRef`] in place.
///
/// Takes `interner` (unlike the illustrative `resolve(&mut HirModule)`) because
/// resolution must read the interned identifier text to recognise `print` and
/// the builtins.
pub fn resolve(module: &mut HirModule, interner: &StringInterner) -> Result<ResolveResult> {
    let mut result = ResolveResult::new();
    // Dedup: the same name resolves to the same symbol, so it gets one SymbolId.
    let mut cache: HashMap<InternedString, SymbolId> = HashMap::new();

    for func in module.functions.iter_mut() {
        for (_idx, expr) in func.exprs.iter_mut() {
            let span = expr.span;
            if let HirExprKind::Name(symref) = &mut expr.kind {
                if let SymbolRef::Unresolved(name) = *symref {
                    let sym_id = match cache.get(&name) {
                        Some(id) => *id,
                        None => {
                            let symbol = resolve_name(name, interner, span)?;
                            let id = result.intern(symbol);
                            cache.insert(name, id);
                            id
                        }
                    };
                    *symref = SymbolRef::Resolved(sym_id);
                }
            }
        }
    }

    Ok(result)
}

fn resolve_name(name: InternedString, interner: &StringInterner, span: Span) -> Result<Symbol> {
    let text = interner.resolve(name);
    if text == "print" {
        Ok(Symbol::BuiltinPrint)
    } else if let Some(kind) = BuiltinFunctionKind::from_name(text) {
        Ok(Symbol::Builtin(kind))
    } else {
        Err(CompilerError::name_error(text, span))
    }
}
