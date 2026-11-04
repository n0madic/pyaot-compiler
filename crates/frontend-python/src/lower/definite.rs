//! Definite-assignment analysis (KNOWN_BUGS Phase 4): a named local read on a
//! path where it may not have been assigned must raise `UnboundLocalError`
//! (CPython), not observe a default value.
//!
//! A forward *must* dataflow over each finished [`HirFunction`]: the set of
//! definitely-assigned locals at a block's entry is the intersection over its
//! predecessors (including the exceptional edge into a handler, which takes the
//! protected block's ENTRY state — an exception may fire before any of its
//! assignments). A read of a named, non-parameter local outside that set marks
//! the local: it is pre-initialized with the `Unbound` sentinel at function
//! entry and made `deletable`, so lowering guards its reads with
//! `rt_check_bound` exactly like a `del`'d variable. Locals that are always
//! assigned before use (the overwhelming majority) are untouched.

use la_arena::Idx;
use pyaot_hir::{HirBlock, HirExpr, HirExprKind, HirFunction, HirStmt, HirTerminator};
use pyaot_types::SemTy;
use pyaot_utils::{LocalId, StringInterner};

use super::builder::{expr_children, stmt_operands};

fn successors(term: &HirTerminator) -> Vec<Idx<HirBlock>> {
    match term {
        HirTerminator::Jump(b) => vec![*b],
        HirTerminator::Branch { then, else_, .. } => vec![*then, *else_],
        HirTerminator::Return(_) | HirTerminator::Unreachable => Vec::new(),
    }
}

/// Collect the locals read by `root` (transitively through sub-expressions).
fn reads(f: &HirFunction, root: Idx<HirExpr>, out: &mut Vec<usize>) {
    let mut work = vec![root];
    while let Some(e) = work.pop() {
        if let HirExprKind::Local(l) = f.exprs[e].kind {
            out.push(l.index());
        }
        work.extend(expr_children(&f.exprs[e].kind));
    }
}

/// Run the analysis on `f`, guarding every possibly-unassigned named local.
pub(super) fn guard_unassigned_reads(f: &mut HirFunction, interner: &StringInterner) {
    let n = f.locals.len();
    if n == 0 {
        return;
    }
    let tracked: Vec<bool> = (0..n)
        .map(|i| {
            i >= f.params.len()
                && !interner.resolve(f.locals[i].name).is_empty()
                && !f.locals[i].deletable
        })
        .collect();
    if !tracked.iter().any(|t| *t) {
        return;
    }
    let blocks: Vec<Idx<HirBlock>> = f.blocks.iter().map(|(i, _)| i).collect();
    let pos: std::collections::HashMap<Idx<HirBlock>, usize> =
        blocks.iter().enumerate().map(|(k, b)| (*b, k)).collect();
    let nb = blocks.len();
    // `true` = definitely assigned. Untracked locals count as always assigned.
    let top = vec![true; n];
    let mut entry_state: Vec<Option<Vec<bool>>> = vec![None; nb];
    let mut init = top.clone();
    for (i, t) in tracked.iter().enumerate() {
        if *t {
            init[i] = false;
        }
    }
    entry_state[pos[&f.entry]] = Some(init);
    let meet = |slot: &mut Option<Vec<bool>>, s: &[bool]| -> bool {
        match slot {
            None => {
                *slot = Some(s.to_vec());
                true
            }
            Some(cur) => {
                let mut changed = false;
                for (c, v) in cur.iter_mut().zip(s) {
                    if *c && !*v {
                        *c = false;
                        changed = true;
                    }
                }
                changed
            }
        }
    };
    let mut work: Vec<usize> = vec![pos[&f.entry]];
    let mut flagged = vec![false; n];
    let mut rd: Vec<usize> = Vec::new();
    while let Some(k) = work.pop() {
        let Some(mut state) = entry_state[k].clone() else {
            continue;
        };
        let b = &f.blocks[blocks[k]];
        if let Some(h) = b.handler {
            if meet(&mut entry_state[pos[&h]], &state) {
                work.push(pos[&h]);
            }
        }
        for stmt in &b.stmts {
            rd.clear();
            for e in stmt_operands(stmt) {
                reads(f, e, &mut rd);
            }
            for &l in &rd {
                if !state[l] {
                    flagged[l] = true;
                }
            }
            if let HirStmt::Assign { target, value } = stmt {
                let t = target.index();
                state[t] = !matches!(f.exprs[*value].kind, HirExprKind::Unbound);
            }
        }
        rd.clear();
        match &b.term {
            HirTerminator::Branch { cond, .. } => reads(f, *cond, &mut rd),
            HirTerminator::Return(Some(v)) => reads(f, *v, &mut rd),
            _ => {}
        }
        for &l in &rd {
            if !state[l] {
                flagged[l] = true;
            }
        }
        for s in successors(&b.term) {
            if meet(&mut entry_state[pos[&s]], &state) {
                work.push(pos[&s]);
            }
        }
    }
    let marked: Vec<usize> = (0..n).filter(|i| flagged[*i] && tracked[*i]).collect();
    if marked.is_empty() {
        return;
    }
    let mut inits = Vec::with_capacity(marked.len());
    for l in marked {
        let local = &mut f.locals[l];
        local.deletable = true;
        local.pin_tagged = true;
        let span = f
            .exprs
            .iter()
            .next()
            .map(|(_, e)| e.span)
            .unwrap_or_else(pyaot_utils::Span::dummy);
        let unbound = f.exprs.alloc(HirExpr {
            kind: HirExprKind::Unbound,
            ty: SemTy::Never,
            span,
            raw_int_ok: false,
        });
        inits.push(HirStmt::Assign {
            target: LocalId::new(l as u32),
            value: unbound,
        });
    }
    let entry = f.entry;
    let stmts = &mut f.blocks[entry].stmts;
    let at = stmts
        .iter()
        .position(|s| !matches!(s, HirStmt::Line(_)))
        .unwrap_or(stmts.len());
    for (k, s) in inits.into_iter().enumerate() {
        stmts.insert(at + k, s);
    }
}
