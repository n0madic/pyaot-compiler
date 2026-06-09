//! # Semantics — name resolution
//!
//! Resolves each [`SymbolRef::Unresolved`] occurrence to a concrete [`Symbol`],
//! rewriting it in place and recording the symbol table in a [`ResolveResult`].
//!
//! The *scope* is reconstructed from the HIR itself, not a side channel: a
//! function's `locals` table carries each local's name, so `name → LocalId` is
//! read straight off it; the module's functions give `name → FuncId`. Resolution
//! precedence is **local → function → builtin** (`print`/`range` are the special
//! non-first-class builtins, then the first-class [`BuiltinFunctionKind`]s).
//! A name in none of those is a `NameError`.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{BuiltinFunctionKind, HirExprKind, HirModule, ResolveResult, Symbol, SymbolRef};
use pyaot_utils::{FuncId, InternedString, LocalId, Span, StringInterner, SymbolId};

/// Resolve every name in `module`, mutating each [`SymbolRef`] in place.
pub fn resolve(module: &mut HirModule, interner: &StringInterner) -> Result<ResolveResult> {
    let mut result = ResolveResult::new();

    // Global: every top-level function name → its FuncId.
    let mut func_map: HashMap<InternedString, FuncId> = HashMap::new();
    for (i, f) in module.functions.iter().enumerate() {
        func_map.insert(f.name, FuncId::new(i as u32));
    }

    for func in module.functions.iter_mut() {
        // Per-function local scope, read off the locals table.
        let mut local_map: HashMap<InternedString, LocalId> = HashMap::new();
        for (li, loc) in func.locals.iter().enumerate() {
            local_map.insert(loc.name, LocalId::new(li as u32));
        }

        // A given name resolves identically within one function → cache it.
        let mut cache: HashMap<InternedString, SymbolId> = HashMap::new();
        for (_idx, expr) in func.exprs.iter_mut() {
            let span = expr.span;
            if let HirExprKind::Name(symref) = &mut expr.kind {
                if let SymbolRef::Unresolved(name) = *symref {
                    let sym_id = match cache.get(&name) {
                        Some(id) => *id,
                        None => {
                            let symbol = resolve_name(name, &local_map, &func_map, interner, span)?;
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

fn resolve_name(
    name: InternedString,
    locals: &HashMap<InternedString, LocalId>,
    funcs: &HashMap<InternedString, FuncId>,
    interner: &StringInterner,
    span: Span,
) -> Result<Symbol> {
    if let Some(local) = locals.get(&name) {
        return Ok(Symbol::Local(*local));
    }
    if let Some(func) = funcs.get(&name) {
        return Ok(Symbol::Function(*func));
    }
    let text = interner.resolve(name);
    match text {
        "print" => Ok(Symbol::BuiltinPrint),
        "range" => Ok(Symbol::BuiltinRange),
        _ => {
            if let Some(kind) = BuiltinFunctionKind::from_name(text) {
                Ok(Symbol::Builtin(kind))
            } else {
                Err(CompilerError::name_error(text, span))
            }
        }
    }
}
