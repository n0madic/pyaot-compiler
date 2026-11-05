//! Per-member runtime-class dispatchers.
//!
//! A member reached through a receiver statically typed as class `C` may be
//! overridden in a subclass of `C`. A method with an ABI-compatible override
//! rides `CallVirtual`, but a dunder (resolved to a direct call), a property,
//! a static/class method or a class attribute read through an instance, and a
//! method whose override changes the ABI (an extra defaulted parameter, a
//! different parameter / return representation) would otherwise bind to
//! `C`'s own definition — silently wrong, or a crash on an ABI mismatch.
//!
//! For each such `(C, member)` this module plans a small synthesized MIR
//! function — the *dispatcher* — that reads the receiver's runtime class id
//! (`rt_instance_class_id`) and calls the definition that class actually
//! resolves to, coercing every argument (checked) to that definition's
//! parameter representation and filling parameters it adds from its own
//! defaults. Dispatchers take and return the `Tagged` baseline, so the call
//! sites stay ordinary direct calls with one uniform ABI; typeck types the
//! result as the join over every definition.

use std::collections::HashMap;

use pyaot_hir::{ClassTable, HirFunction, HirLocal, HirModule, HirParam};
use pyaot_types::{Repr, SemTy};
use pyaot_utils::{ClassId, FuncId, InternedString, StringInterner};

use crate::{is_dunder, FnSig};


/// The member kind a dispatcher selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MemberKind {
    /// An instance method (including dunders): `recv.m(args)`.
    Method,
    /// A `@property` getter: `recv.p`.
    PropGet,
    /// A `@property` setter: `recv.p = v`.
    PropSet,
    /// A `@staticmethod` / `@classmethod` called through an instance:
    /// `recv.sm(args)` (the receiver only selects; it is not passed).
    StaticViaInst,
    /// A class attribute read through an instance: `recv.attr`.
    ClassAttr,
    /// `super().m(args)` inside a method of the static class: the definition
    /// after that class in the RECEIVER's MRO (cooperative multiple
    /// inheritance — a diamond's `super()` chain visits every branch).
    SuperMethod,
}

/// What one runtime class resolves the member to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    Func(FuncId),
    Attr { owner: ClassId, idx: u32 },
}

/// One planned dispatcher.
pub(crate) struct DispatchSpec {
    pub(crate) fid: FuncId,
    pub(crate) kind: MemberKind,
    /// The receiver's static class's own resolution (the fallback arm).
    pub(crate) base: Target,
    /// Every other distinct resolution with the runtime class ids selecting it.
    pub(crate) arms: Vec<(Vec<ClassId>, Target)>,
    /// The dispatcher's parameter count (receiver included).
    pub(crate) n_params: usize,
    pub(crate) name: InternedString,
    pub(crate) file: InternedString,
}

/// The planned dispatchers, keyed by `(static class, member name, kind)`.
#[derive(Default)]
pub(crate) struct Dispatchers {
    pub(crate) specs: Vec<DispatchSpec>,
    index: HashMap<(ClassId, InternedString, MemberKind), FuncId>,
}

impl Dispatchers {
    /// The dispatcher for `name` of `kind` on a receiver statically typed `cid`,
    /// if that member is resolved differently by some subclass.
    pub(crate) fn get(&self, cid: ClassId, name: InternedString, kind: MemberKind) -> Option<FuncId> {
        self.index.get(&(cid, name, kind)).copied()
    }
}

fn resolve(classes: &ClassTable, k: ClassId, name: InternedString, kind: MemberKind) -> Option<Target> {
    let info = classes.get(k)?;
    match kind {
        MemberKind::Method => info.method(name).map(|m| Target::Func(m.func_id)),
        MemberKind::PropGet => info.property(name).map(|p| Target::Func(p.getter)),
        MemberKind::PropSet => info.property(name).and_then(|p| p.setter).map(Target::Func),
        MemberKind::StaticViaInst => info
            .static_method(name)
            .or_else(|| info.class_method(name))
            .map(|m| Target::Func(m.func_id)),
        MemberKind::SuperMethod => classes.resolve_super_method(k, name).map(Target::Func),
        MemberKind::ClassAttr => {
            info.class_attr(name).map(|a| Target::Attr {
                owner: a.owner,
                idx: a.attr_idx,
            })
        }
    }
}

/// The definition `super().name` reaches from a method of `anchor` when the
/// receiver's runtime class is `k`: the first class AFTER `anchor` in `k`'s
/// MRO whose own body defines `name`.
fn super_target(
    classes: &ClassTable,
    k: ClassId,
    anchor: ClassId,
    name: InternedString,
) -> Option<Target> {
    let mro = &classes.get(k)?.mro;
    let at = mro.iter().position(|c| *c == anchor)?;
    for c in &mro[at + 1..] {
        if let Some((_, fid)) = classes
            .get(*c)
            .and_then(|i| i.own_methods.iter().find(|(n, _)| *n == name))
        {
            return Some(Target::Func(*fid));
        }
    }
    None
}

/// True iff calling `b` with `a`'s calling convention is exact: same
/// parameter count, representations, return representation and variadics.
fn abi_compatible(sigs: &[FnSig], a: FuncId, b: FuncId) -> bool {
    let (sa, sb) = (&sigs[a.index()], &sigs[b.index()]);
    // `self` (param 0) is each class's own instance repr — the vtable call
    // passes the receiver unchanged, so only the remaining params must agree.
    sa.params.len() == sb.params.len()
        && sa.params.get(1..) == sb.params.get(1..)
        && sa.ret == sb.ret
        && sa.varargs == sb.varargs
        && sa.kwargs == sb.kwargs
        && sa.defaults == sb.defaults
        && sa.param_names == sb.param_names
}

/// Plan every dispatcher the program needs, assigning FuncIds from
/// `first_fid` and appending their signatures to `sigs`.
pub(crate) fn plan(
    module: &HirModule,
    classes: &ClassTable,
    interner: &StringInterner,
    sigs: &mut Vec<FnSig>,
) -> Dispatchers {
    let mut out = Dispatchers::default();
    let mut class_ids: Vec<ClassId> = classes.iter().map(|c| c.class_id).collect();
    class_ids.sort_by_key(|c| c.0);
    let file = module.functions[module.main.index()].file;
    for &cid in &class_ids {
        let info = classes.get(cid).expect("listed class");
        // Every runtime class an instance typed `cid` can have.
        let family: Vec<ClassId> = class_ids
            .iter()
            .copied()
            .filter(|k| *k == cid || classes.get(*k).is_some_and(|i| i.mro.contains(&cid)))
            .collect();
        if family.len() < 2 {
            continue;
        }
        let mut members: Vec<(InternedString, MemberKind)> = Vec::new();
        members.extend(info.methods.iter().map(|m| (m.name, MemberKind::Method)));
        for p in &info.properties {
            members.push((p.name, MemberKind::PropGet));
            members.push((p.name, MemberKind::PropSet));
        }
        members.extend(info.static_methods.iter().map(|m| (m.name, MemberKind::StaticViaInst)));
        members.extend(info.class_methods.iter().map(|m| (m.name, MemberKind::StaticViaInst)));
        members.extend(info.class_attrs.iter().map(|a| (a.name, MemberKind::ClassAttr)));
        members.extend(info.methods.iter().map(|m| (m.name, MemberKind::SuperMethod)));
        for (name, kind) in members {
            if out.index.contains_key(&(cid, name, kind)) {
                continue;
            }
            let Some(base) = resolve(classes, cid, name, kind) else {
                continue;
            };
            let mut arms: Vec<(Vec<ClassId>, Target)> = Vec::new();
            for &k in family.iter().filter(|k| **k != cid) {
                // A subclass that replaced the member with another kind keeps
                // the static resolution (outside this dispatcher's domain).
                let t = if kind == MemberKind::SuperMethod {
                    super_target(classes, k, cid, name).unwrap_or(base)
                } else {
                    resolve(classes, k, name, kind).unwrap_or(base)
                };
                if t == base {
                    continue;
                }
                match arms.iter_mut().find(|(_, at)| *at == t) {
                    Some((ks, _)) => ks.push(k),
                    None => arms.push((vec![k], t)),
                }
            }
            if arms.is_empty() {
                continue;
            }
            let base_fid = match base {
                Target::Func(f) => Some(f),
                Target::Attr { .. } => None,
            };
            // A plain method whose every override shares its exact ABI keeps the
            // vtable `CallVirtual`; dunders always take a dispatcher (their call
            // sites resolve to a direct call).
            if kind == MemberKind::Method && !is_dunder(interner.resolve(name)) {
                let all_compatible = arms.iter().all(|(_, t)| match (t, base_fid) {
                    (Target::Func(f), Some(b)) => abi_compatible(sigs, b, *f),
                    _ => false,
                });
                if all_compatible {
                    continue;
                }
            }
            // The dispatcher's calling convention: the static resolution's
            // parameter list (receiver first), every slot `Tagged`.
            let (params, names, defaults, varargs, kwargs) = match (kind, base_fid) {
                (
                MemberKind::Method
                | MemberKind::PropGet
                | MemberKind::PropSet
                | MemberKind::SuperMethod,
                Some(b),
            ) => {
                    let s = &sigs[b.index()];
                    (
                        vec![Repr::Tagged; s.params.len()],
                        s.param_names.clone(),
                        s.defaults.clone(),
                        s.varargs,
                        s.kwargs,
                    )
                }
                (MemberKind::StaticViaInst, Some(b)) => {
                    let s = &sigs[b.index()];
                    let mut names = vec![name];
                    names.extend(s.param_names.iter().copied());
                    let mut defaults = vec![None];
                    defaults.extend(s.defaults.iter().cloned());
                    (
                        vec![Repr::Tagged; s.params.len() + 1],
                        names,
                        defaults,
                        s.varargs,
                        s.kwargs,
                    )
                }
                _ => (
                    vec![Repr::Tagged],
                    vec![name],
                    vec![None],
                    false,
                    false,
                ),
            };
            let fid = FuncId::new(sigs.len() as u32);
            let n_params = params.len();
            sigs.push(FnSig {
                params,
                param_names: names,
                defaults,
                ret: Repr::Tagged,
                varargs,
                kwargs,
            });
            out.index.insert((cid, name, kind), fid);
            out.specs.push(DispatchSpec {
                fid,
                kind,
                base,
                arms,
                n_params,
                name,
                file,
            });
        }
    }
    out
}

/// A synthetic, body-less HIR function standing in for a dispatcher so the
/// ordinary [`crate::FnLower`] builder (coercions, defaults, blocks) can emit
/// its MIR: `n` `Dyn` (→ `Tagged`) parameter locals.
pub(crate) fn stub_function(spec: &DispatchSpec) -> HirFunction {
    let mut blocks = la_arena::Arena::new();
    let entry = blocks.alloc(pyaot_hir::HirBlock {
        stmts: Vec::new(),
        term: pyaot_hir::HirTerminator::Unreachable,
        handler: None,
    });
    HirFunction {
        name: spec.name,
        file: spec.file,
        params: (0..spec.n_params)
            .map(|_| HirParam {
                name: spec.name,
                ty: SemTy::Dyn,
                default: None,
            })
            .collect(),
        varargs: false,
        kwargs: false,
        ret_ty: SemTy::Dyn,
        ret_raw_int: false,
        locals: (0..spec.n_params)
            .map(|_| HirLocal::new(spec.name, SemTy::Dyn))
            .collect(),
        blocks,
        entry,
        exprs: la_arena::Arena::new(),
    }
}
