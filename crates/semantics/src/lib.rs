//! # Semantics — name resolution + class collection
//!
//! Resolves each [`SymbolRef::Unresolved`] occurrence to a concrete [`Symbol`],
//! rewriting it in place and recording the symbol table in a [`ResolveResult`].
//!
//! The *scope* is reconstructed from the HIR itself, not a side channel: a
//! function's `locals` table carries each local's name, so `name → LocalId` is
//! read straight off it; the module's functions give `name → FuncId` and its
//! classes give `name → ClassId`. Resolution precedence is **local →
//! function/class → builtin** (`print`/`range` are the special non-first-class
//! builtins, then the first-class [`BuiltinFunctionKind`]s). A name in none of
//! those is a `NameError`.
//!
//! [`collect_classes`] is the Phase-5 class-collection pass — a single forward
//! terminal pass (no feedback into `resolve`, so it does not reopen the A3
//! fixpoint). It runs *after* `resolve`, *before* `typeck`, and produces the
//! [`ClassTable`]: C3 MRO, parent-first slot layout, best-effort field types
//! (B10/D5), and the method table (own + inherited, with vtable slots).

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{
    BuiltinFunctionKind, ClassAttrInfo, ClassInfo, ClassTable, ContainerOp, FieldInfo, HirClass,
    HirExprKind, HirFunction, HirModule, HirStmt, MethodInfo, NamespaceTable, PropertyInfo,
    ResolveResult, Symbol, SymbolRef,
};
use pyaot_types::SemTy;
use pyaot_utils::{ClassId, FuncId, InternedString, LocalId, Span, StringInterner, SymbolId};

/// Per-namespace name maps: index by namespace id to get that module's
/// `name → FuncId` / `name → ClassId` view (own defs overlaid by imports).
type NamespaceMaps = (
    Vec<HashMap<InternedString, FuncId>>,
    Vec<HashMap<InternedString, ClassId>>,
);

/// Per-namespace function/class name maps (Phase 8): a module sees its own
/// top-level definitions plus its imported bindings. A single-file program has
/// exactly one namespace, recovering the original global behavior.
fn build_namespace_maps(module: &HirModule, namespaces: &NamespaceTable) -> NamespaceMaps {
    let n_ns = namespaces.imports.len().max(1);
    let mut func_maps: Vec<HashMap<InternedString, FuncId>> = vec![HashMap::new(); n_ns];
    for (i, f) in module.functions.iter().enumerate() {
        let ns = namespaces.func_ns.get(i).copied().unwrap_or(0) as usize;
        func_maps[ns.min(n_ns - 1)].insert(f.name, FuncId::new(i as u32));
    }
    let mut class_maps: Vec<HashMap<InternedString, ClassId>> = vec![HashMap::new(); n_ns];
    for c in &module.classes {
        let ns = namespaces.class_ns.get(&c.class_id).copied().unwrap_or(0) as usize;
        class_maps[ns.min(n_ns - 1)].insert(c.name, c.class_id);
    }
    // Imported bindings overlay the own definitions (`from m import x` shadows a
    // same-named local def — the corpus never conflicts).
    for (ns, imp) in namespaces.imports.iter().enumerate() {
        for (name, fid) in &imp.funcs {
            func_maps[ns].insert(*name, *fid);
        }
        for (name, cid) in &imp.classes {
            class_maps[ns].insert(*name, *cid);
        }
    }
    (func_maps, class_maps)
}

/// Resolve every name in `module`, mutating each [`SymbolRef`] in place. Name
/// scope is per-namespace (Phase 8): a function resolves names through its
/// owning module's definitions + imports (`namespaces.func_ns[fid]`).
pub fn resolve(
    module: &mut HirModule,
    namespaces: &NamespaceTable,
    interner: &StringInterner,
) -> Result<ResolveResult> {
    let mut result = ResolveResult::new();
    let (func_maps, class_maps) = build_namespace_maps(module, namespaces);
    let n_ns = func_maps.len();

    for (fi, func) in module.functions.iter_mut().enumerate() {
        let ns = (namespaces.func_ns.get(fi).copied().unwrap_or(0) as usize).min(n_ns - 1);
        let func_map = &func_maps[ns];
        let class_map = &class_maps[ns];

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
                            let symbol = resolve_name(
                                name, &local_map, func_map, class_map, interner, span,
                            )?;
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
    classes: &HashMap<InternedString, ClassId>,
    interner: &StringInterner,
    span: Span,
) -> Result<Symbol> {
    if let Some(local) = locals.get(&name) {
        return Ok(Symbol::Local(*local));
    }
    // Function / class share the global namespace; in valid Python a name is at
    // most one of them, so order between the two is immaterial.
    if let Some(class_id) = classes.get(&name) {
        return Ok(Symbol::Class(*class_id));
    }
    if let Some(func) = funcs.get(&name) {
        return Ok(Symbol::Function(*func));
    }
    let text = interner.resolve(name);
    match text {
        "print" => Ok(Symbol::BuiltinPrint),
        "range" => Ok(Symbol::BuiltinRange),
        _ => {
            // Container / iteration builtins are checked before the frozen
            // first-class builtins so `len` routes through the shared container
            // read path (the frozen `BuiltinFunctionKind::Len` then goes unused) —
            // but *after* local / function scope, so user shadowing still wins.
            if let Some(op) = ContainerOp::from_name(text) {
                Ok(Symbol::Container(op))
            } else if let Some(kind) = BuiltinFunctionKind::from_name(text) {
                Ok(Symbol::Builtin(kind))
            } else {
                Err(CompilerError::name_error(text, span))
            }
        }
    }
}

// ============================================================================
// Class collection (Phase 5)
// ============================================================================

/// Build the [`ClassTable`] from a resolved module: resolve base names → ClassIds,
/// compute the C3 MRO, lay out instance fields (parent-first, slot-stable),
/// best-effort field types (D5), and the method table (own + inherited, with
/// vtable slots). A single forward terminal pass; no feedback into `resolve`.
pub fn collect_classes(
    module: &HirModule,
    namespaces: &NamespaceTable,
    interner: &StringInterner,
) -> Result<ClassTable> {
    let mut table = ClassTable::new();
    if module.classes.is_empty() {
        return Ok(table);
    }

    // ClassId → &HirClass, for base resolution. Base *names* resolve per-namespace
    // (Phase 8): a class's bases are looked up in its own module's class map plus
    // its imported classes — so cross-module inheritance and same-name classes in
    // different modules both resolve correctly.
    let (_func_maps, class_maps) = {
        // Reuse the same per-namespace class maps the name resolver builds.
        let n_ns = namespaces.imports.len().max(1);
        let mut class_maps: Vec<HashMap<InternedString, ClassId>> = vec![HashMap::new(); n_ns];
        for c in &module.classes {
            let ns = namespaces.class_ns.get(&c.class_id).copied().unwrap_or(0) as usize;
            class_maps[ns.min(n_ns - 1)].insert(c.name, c.class_id);
        }
        for (ns, imp) in namespaces.imports.iter().enumerate() {
            for (name, cid) in &imp.classes {
                class_maps[ns].insert(*name, *cid);
            }
        }
        (Vec::<()>::new(), class_maps)
    };
    let n_ns = class_maps.len();
    let mut by_id: HashMap<ClassId, &HirClass> = HashMap::new();
    for c in &module.classes {
        by_id.insert(c.class_id, c);
    }

    // Resolve base names → parent ClassIds (declaration order). A base that is
    // not a user class is tried against the builtin exception names (Phase 7C):
    // on a hit it contributes no C3 input (an opaque root) and marks the class
    // as an exception class via `exception_base`.
    let mut parents: HashMap<ClassId, Vec<ClassId>> = HashMap::new();
    let mut direct_exc_base: HashMap<ClassId, pyaot_hir::BuiltinExceptionKind> = HashMap::new();
    for c in &module.classes {
        let ns =
            (namespaces.class_ns.get(&c.class_id).copied().unwrap_or(0) as usize).min(n_ns - 1);
        let name_to_id = &class_maps[ns];
        let mut bases = Vec::new();
        for base in &c.base_names {
            let base_name = interner.resolve(*base);
            // `object` is the implicit root and carries no fields/methods.
            if base_name == "object" {
                continue;
            }
            match name_to_id.get(base) {
                Some(bid) => bases.push(*bid),
                None => match pyaot_hir::BuiltinExceptionKind::from_name(base_name) {
                    Some(kind) => {
                        direct_exc_base.insert(c.class_id, kind);
                    }
                    None => {
                        return Err(CompilerError::semantic_error(
                            format!("unknown base class `{base_name}`"),
                            Span::dummy(),
                        ))
                    }
                },
            }
        }
        parents.insert(c.class_id, bases);
    }

    // C3 linearization → MRO per class (cached so diamonds are computed once).
    let mut mro_cache: HashMap<ClassId, Vec<ClassId>> = HashMap::new();
    for c in &module.classes {
        let mut visiting = HashSet::new();
        c3_linearize(c.class_id, &parents, &mut mro_cache, &mut visiting)?;
    }

    // Build each class's resolved info.
    for c in &module.classes {
        let mro = mro_cache[&c.class_id].clone();
        let parent = parents[&c.class_id].first().copied();

        // ── Fields: merge each base's resolved layout, then append own fields ──
        //
        // Slot-stability requires a field to occupy the SAME slot in a class as in
        // every base that declares it. Building each base's *already-resolved*
        // layout (bases are processed first — lower class_id) as a prefix gives
        // single inheritance the parent-first guarantee for free. Multiple
        // inheritance where two bases place *different* fields at the same slot has
        // **no** consistent layout (`B{a,b}` + `C{a,c}` both want slot 1) — reject
        // loudly rather than silently mis-address a by-slot `GetField`/`SetField`
        // (PITFALLS "fail loud"; method dispatch is by-name and stays correct).
        let mut fields: Vec<FieldInfo> = Vec::new();
        for base in &parents[&c.class_id] {
            let binfo = match table.get(*base) {
                Some(b) => b,
                None => {
                    // A user-class base that is not yet in `table` was declared
                    // LATER in the file (a forward reference). CPython binds class
                    // statements at run time, so `class Sub(Base): ...` before
                    // `class Base: ...` raises `NameError: name 'Base' is not
                    // defined`. Reject it here rather than silently dropping the
                    // base's field layout (which made pyaot accept and mis-run a
                    // program CPython rejects). A base absent from `by_id` is a
                    // builtin (Exception/Protocol/object) with no user field
                    // layout — skip it as before.
                    if let Some(bhc) = by_id.get(base) {
                        return Err(CompilerError::semantic_error(
                            format!("name `{}` is not defined", interner.resolve(bhc.name)),
                            Span::dummy(),
                        ));
                    }
                    continue;
                }
            };
            for (bslot, bf) in binfo.fields.iter().enumerate() {
                match fields.get(bslot) {
                    Some(existing) if existing.name != bf.name => {
                        return Err(CompilerError::semantic_error(
                            format!(
                                "cannot lay out instance fields for class `{}`: base \
                                 classes place different fields (`{}` and `{}`) at slot \
                                 {} — multiple inheritance with instance fields in more \
                                 than one branch is unsupported (no consistent layout)",
                                interner.resolve(c.name),
                                interner.resolve(existing.name),
                                interner.resolve(bf.name),
                                bslot,
                            ),
                            Span::dummy(),
                        ));
                    }
                    Some(_) => {} // same field at the same slot — consistent.
                    None if bslot == fields.len() => fields.push(bf.clone()),
                    None => {
                        return Err(CompilerError::semantic_error(
                            format!(
                                "cannot lay out instance fields for class `{}`: base \
                                 layouts are not slot-compatible (gap at slot {})",
                                interner.resolve(c.name),
                                bslot,
                            ),
                            Span::dummy(),
                        ));
                    }
                }
            }
        }
        // Own fields declared directly by this class, after the inherited ones.
        for fi in discover_fields(by_id[&c.class_id], module, interner) {
            match fields.iter_mut().find(|f| f.name == fi.name) {
                Some(existing) => {
                    if existing.ty == SemTy::Dyn && fi.ty != SemTy::Dyn {
                        existing.ty = fi.ty;
                    }
                }
                None => fields.push(fi),
            }
        }

        // ── Methods: own + inherited; MRO order gives override precedence ──
        let mut methods: Vec<MethodInfo> = Vec::new();
        let mut slot_of: HashMap<InternedString, usize> = HashMap::new();
        // Assign vtable slots base-first so a base method keeps its slot in
        // subclasses (slot-stability); an override reuses the inherited slot.
        for ancestor in mro.iter().rev() {
            let Some(ac) = by_id.get(ancestor) else {
                continue;
            };
            for (mname, _fid) in &ac.methods {
                if !slot_of.contains_key(mname) {
                    let slot = slot_of.len();
                    slot_of.insert(*mname, slot);
                }
            }
        }
        let num_vtable_slots = slot_of.len();
        // Resolve each method name to the most-derived definition along the MRO
        // (self first), recording its FuncId + stable slot.
        let mut seen: HashMap<InternedString, ()> = HashMap::new();
        for ancestor in &mro {
            let Some(ac) = by_id.get(ancestor) else {
                continue;
            };
            for (mname, fid) in &ac.methods {
                if seen.contains_key(mname) {
                    continue;
                }
                seen.insert(*mname, ());
                methods.push(MethodInfo {
                    name: *mname,
                    func_id: *fid,
                    slot: slot_of[mname],
                });
            }
        }

        // Decorated members + class attributes (Phase 5D), own-only: a `@property`/
        // `@staticmethod`/`@classmethod`/class attribute is not inherited in 5D.
        let static_methods = c
            .static_methods
            .iter()
            .map(|(n, f)| MethodInfo {
                name: *n,
                func_id: *f,
                slot: 0,
            })
            .collect();
        let class_methods = c
            .class_methods
            .iter()
            .map(|(n, f)| MethodInfo {
                name: *n,
                func_id: *f,
                slot: 0,
            })
            .collect();
        let properties: Vec<PropertyInfo> = c
            .properties
            .iter()
            .map(|p| PropertyInfo {
                name: p.name,
                getter: p.getter,
                setter: p.setter,
                ty: p.ty.clone(),
            })
            .collect();
        let class_attrs: Vec<ClassAttrInfo> = c
            .class_attrs
            .iter()
            .enumerate()
            .map(|(i, a)| ClassAttrInfo {
                name: a.name,
                ty: a.ty.clone(),
                attr_idx: i as u32,
                init: a.init.clone(),
            })
            .collect();

        // The effective builtin exception base (Phase 7C): a direct builtin
        // base, else inherited through the first user parent (already resolved
        // — bases are declared before subclasses).
        let exception_base = direct_exc_base
            .get(&c.class_id)
            .copied()
            .or_else(|| parent.and_then(|p| table.get(p).and_then(|info| info.exception_base)));

        table.insert(ClassInfo {
            class_id: c.class_id,
            name: c.name,
            qualname: c.qualname,
            parent,
            mro,
            fields,
            methods,
            own_methods: c.methods.clone(),
            static_methods,
            class_methods,
            properties,
            class_attrs,
            num_vtable_slots,
            type_params: c.type_params.clone(),
            exception_base,
            is_protocol: c.is_protocol,
        });
    }

    Ok(table)
}

/// Discover the instance fields a class *directly* declares: class-level
/// annotations first (in declaration order), then `self.x = …` writes scanned
/// across its method bodies (first-appearance order). Best-effort field types
/// (D5): annotation → assigned-param type → assigned-literal type → `Dyn`.
fn discover_fields(c: &HirClass, module: &HirModule, interner: &StringInterner) -> Vec<FieldInfo> {
    let mut fields: Vec<FieldInfo> = Vec::new();

    // 1. Class-level annotations.
    for (name, ty) in &c.field_annotations {
        if !fields.iter().any(|f| f.name == *name) {
            fields.push(FieldInfo {
                name: *name,
                ty: ty.clone(),
            });
        }
    }

    // 2. `self.<name> = value` writes across this class's methods.
    for (_mname, fid) in &c.methods {
        let func = &module.functions[fid.index()];
        // `self` is parameter 0 (D1); its LocalId is 0.
        let self_lid = LocalId::new(0);
        for (_b, block) in func.blocks.iter() {
            for stmt in &block.stmts {
                let HirStmt::SetAttr { base, name, value } = stmt else {
                    continue;
                };
                if !is_local_ref(func, *base, self_lid) {
                    continue;
                }
                let ty = best_effort_field_ty(func, *value);
                match fields.iter_mut().find(|f| f.name == *name) {
                    Some(existing) => {
                        // A class-level annotation is authoritative; otherwise the
                        // first concrete (non-`Dyn`) write wins, else stay `Dyn`.
                        if existing.ty == SemTy::Dyn && ty != SemTy::Dyn {
                            existing.ty = ty;
                        }
                    }
                    None => fields.push(FieldInfo { name: *name, ty }),
                }
            }
        }
    }
    let _ = interner;
    fields
}

/// True iff expr `idx` is a direct read of local `want` (`self`).
fn is_local_ref(func: &HirFunction, idx: la_arena::Idx<pyaot_hir::HirExpr>, want: LocalId) -> bool {
    matches!(func.exprs[idx].kind, HirExprKind::Local(lid) if lid == want)
}

/// Best-effort static type of a field's assigned value, using only syntactic
/// information available before `typeck` runs (D5): a read of an annotated
/// parameter/local carries its declared type; a literal carries its parse-time
/// type; everything else is `Dyn` (→ Tagged, correct but imprecise).
fn best_effort_field_ty(func: &HirFunction, value: la_arena::Idx<pyaot_hir::HirExpr>) -> SemTy {
    match &func.exprs[value].kind {
        HirExprKind::Local(lid) => func.locals[lid.index()].ty.clone(),
        // Literals already carry their type from the frontend.
        HirExprKind::IntLit(_) | HirExprKind::BigIntLit(_) => SemTy::Int,
        HirExprKind::FloatLit(_) => SemTy::Float,
        HirExprKind::BoolLit(_) => SemTy::Bool,
        HirExprKind::StrLit(_) => SemTy::Str,
        HirExprKind::BytesLit(_) => SemTy::Bytes,
        HirExprKind::NoneLit => SemTy::NoneTy,
        _ => SemTy::Dyn,
    }
}

/// C3 linearization (`mro`) for `cid`, memoized in `cache`. Errors on an
/// inconsistent hierarchy (no valid linearization — a real Python `TypeError`).
fn c3_linearize(
    cid: ClassId,
    parents: &HashMap<ClassId, Vec<ClassId>>,
    cache: &mut HashMap<ClassId, Vec<ClassId>>,
    visiting: &mut HashSet<ClassId>,
) -> Result<Vec<ClassId>> {
    if let Some(m) = cache.get(&cid) {
        return Ok(m.clone());
    }
    // Cycle guard: a base that is still on the recursion stack means the
    // hierarchy is cyclic (`class A(B)` / `class B(A)`, or `class A(A)`). Without
    // this the recursion never reaches a cached/base case and overflows the
    // stack, aborting the compiler — surface a real Python `TypeError` instead.
    if !visiting.insert(cid) {
        return Err(CompilerError::semantic_error(
            "cannot create a consistent method resolution order (MRO): cyclic inheritance",
            Span::dummy(),
        ));
    }
    let bases = parents.get(&cid).cloned().unwrap_or_default();
    // L[C] = C + merge(L[B1], …, L[Bn], [B1, …, Bn]).
    let mut seqs: Vec<Vec<ClassId>> = Vec::new();
    for b in &bases {
        seqs.push(c3_linearize(*b, parents, cache, visiting)?);
    }
    if !bases.is_empty() {
        seqs.push(bases.clone());
    }
    let mut result = vec![cid];
    result.extend(c3_merge(seqs, cid)?);
    cache.insert(cid, result.clone());
    // Off the recursion stack: a sibling sharing a base (diamond) must not read
    // this as a back-edge (the cache short-circuits revisits anyway).
    visiting.remove(&cid);
    Ok(result)
}

/// The C3 `merge` step: repeatedly take the head of some sequence that does not
/// appear in any other sequence's tail, append it, and remove it everywhere.
fn c3_merge(mut seqs: Vec<Vec<ClassId>>, cid: ClassId) -> Result<Vec<ClassId>> {
    let mut out = Vec::new();
    loop {
        seqs.retain(|s| !s.is_empty());
        if seqs.is_empty() {
            return Ok(out);
        }
        // Find a good head: appears in no sequence's tail.
        let mut chosen: Option<ClassId> = None;
        for s in &seqs {
            let head = s[0];
            let in_tail = seqs.iter().any(|t| t[1..].contains(&head));
            if !in_tail {
                chosen = Some(head);
                break;
            }
        }
        let Some(head) = chosen else {
            return Err(CompilerError::semantic_error(
                format!(
                    "cannot create a consistent method resolution order (MRO) for class id {}",
                    cid.0
                ),
                Span::dummy(),
            ));
        };
        out.push(head);
        for s in &mut seqs {
            s.retain(|c| *c != head);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyaot_utils::ClassId;

    /// Parse + resolve + collect; return the class table and interner.
    fn collected(src: &str) -> (ClassTable, StringInterner) {
        let mut interner = StringInterner::new();
        let mut module = pyaot_frontend_python::parse(src, &mut interner).expect("parse");
        let ns = NamespaceTable::single(module.functions.len());
        resolve(&mut module, &ns, &interner).expect("resolve");
        let table = collect_classes(&module, &ns, &interner).expect("collect_classes");
        (table, interner)
    }

    /// Parse + resolve, then return the `collect_classes` result (for rejections).
    fn try_collect(src: &str) -> Result<ClassTable> {
        let mut interner = StringInterner::new();
        let mut module = pyaot_frontend_python::parse(src, &mut interner).expect("parse");
        let ns = NamespaceTable::single(module.functions.len());
        resolve(&mut module, &ns, &interner).expect("resolve");
        collect_classes(&module, &ns, &interner)
    }

    #[test]
    fn rejects_mi_with_conflicting_field_layouts() {
        // B places `b` at slot 1, C places `c` at slot 1 — no consistent layout for
        // D(B, C). Reject loudly rather than silently mis-address a by-slot field
        // (review fix #2). The fields must actually conflict in BOTH branches.
        let conflict = "\
class A:
    def __init__(self, a: int):
        self.a = a
class B(A):
    def __init__(self, a: int, b: int):
        self.a = a
        self.b = b
class C(A):
    def __init__(self, a: int, c: int):
        self.a = a
        self.c = c
class D(B, C):
    pass
";
        assert!(try_collect(conflict).is_err());

        // A method-only diamond (no fields) has no layout conflict — must succeed.
        let method_only = "\
class A:
    def who(self) -> int:
        return 1
class B(A):
    def who(self) -> int:
        return 2
class C(A):
    def who(self) -> int:
        return 3
class D(B, C):
    pass
";
        assert!(try_collect(method_only).is_ok());
    }

    #[test]
    fn single_inheritance_fields_are_parent_first() {
        // Slot-stability for single inheritance survives the merged-layout rewrite:
        // a base field keeps slot 0 in the subclass, own fields append after.
        let src = "\
class Animal:
    def __init__(self, name: str):
        self.name = name
class Dog(Animal):
    def __init__(self, name: str, breed: str):
        self.name = name
        self.breed = breed
";
        let (t, mut i) = collected(src);
        let name = i.intern("name");
        let breed = i.intern("breed");
        let dog = t.get(cid_of(&t, &i, "Dog")).unwrap();
        assert_eq!(dog.field_slot(name), Some(0));
        assert_eq!(dog.field_slot(breed), Some(1));
        assert_eq!(
            t.get(cid_of(&t, &i, "Animal")).unwrap().field_slot(name),
            Some(0)
        );
    }

    fn cid_of(table: &ClassTable, interner: &StringInterner, name: &str) -> ClassId {
        table
            .iter()
            .find(|c| interner.resolve(c.name) == name)
            .unwrap_or_else(|| panic!("no class {name}"))
            .class_id
    }

    fn mro_names(table: &ClassTable, interner: &StringInterner, name: &str) -> Vec<String> {
        let cid = cid_of(table, interner, name);
        table
            .get(cid)
            .unwrap()
            .mro
            .iter()
            .map(|c| interner.resolve(table.get(*c).unwrap().name).to_string())
            .collect()
    }

    const DIAMOND: &str = "\
class A:
    def who(self) -> str:
        return \"A\"
class B(A):
    def who(self) -> str:
        return \"B\"
class C(A):
    def who(self) -> str:
        return \"C\"
class D(B, C):
    pass
";

    #[test]
    fn c3_linearization_diamond() {
        let (t, i) = collected(DIAMOND);
        assert_eq!(mro_names(&t, &i, "D"), ["D", "B", "C", "A"]);
        assert_eq!(mro_names(&t, &i, "B"), ["B", "A"]);
        assert_eq!(mro_names(&t, &i, "A"), ["A"]);
    }

    #[test]
    fn rejects_cyclic_inheritance() {
        // A mutual cycle (`class A(B)` / `class B(A)`) and a self-cycle both reach
        // C3 linearization; without the recursion guard they overflow the stack and
        // abort the compiler. Reject loudly with a real `TypeError`-shaped error.
        let mutual = "\
class A(B):
    pass
class B(A):
    pass
";
        assert!(try_collect(mutual).is_err());

        let self_cycle = "\
class A(A):
    pass
";
        assert!(try_collect(self_cycle).is_err());
    }

    #[test]
    fn rejects_forward_base_reference() {
        // A base class declared LATER than its subclass is a forward reference;
        // CPython raises `NameError` at the `class Sub(Base):` line. Reject it
        // rather than silently dropping the base's field layout.
        let forward = "\
class Sub(Base):
    pass
class Base:
    pass
";
        assert!(try_collect(forward).is_err());

        // The normal (base-before-subclass) order is accepted.
        let normal = "\
class Base:
    pass
class Sub(Base):
    pass
";
        assert!(try_collect(normal).is_ok());
    }

    #[test]
    fn nominal_subtype_oracle() {
        let (t, i) = collected(DIAMOND);
        let (a, b, c, d) = (
            cid_of(&t, &i, "A"),
            cid_of(&t, &i, "B"),
            cid_of(&t, &i, "C"),
            cid_of(&t, &i, "D"),
        );
        assert!(t.is_subclass(d, a)); // D <: A (via the diamond)
        assert!(t.is_subclass(d, b));
        assert!(t.is_subclass(d, c));
        assert!(t.is_subclass(b, a));
        assert!(!t.is_subclass(a, d)); // not the other way
        assert!(!t.is_subclass(b, c)); // siblings
    }

    #[test]
    fn slot_stability_fields_and_methods() {
        // A base field/method keeps the same slot in every subclass.
        let src = "\
class Animal:
    def __init__(self, name: str):
        self.name = name
    def speak(self) -> str:
        return \"...\"
class Dog(Animal):
    def __init__(self, name: str, breed: str):
        self.name = name
        self.breed = breed
    def speak(self) -> str:
        return \"Woof\"
    def fetch(self) -> str:
        return \"ok\"
";
        let (t, mut i) = collected(src);
        let name = i.intern("name");
        let speak = i.intern("speak");
        let animal = t.get(cid_of(&t, &i, "Animal")).unwrap();
        let dog = t.get(cid_of(&t, &i, "Dog")).unwrap();
        // `name` is slot 0 in both Animal and Dog.
        assert_eq!(animal.field_slot(name), Some(0));
        assert_eq!(dog.field_slot(name), Some(0));
        // `breed` is appended after the inherited field.
        assert_eq!(dog.field_count(), 2);
        // `speak` occupies the same vtable slot in Animal and Dog (override reuses).
        assert_eq!(
            animal.method(speak).unwrap().slot,
            dog.method(speak).unwrap().slot
        );
        // Dog's resolved `speak` is its OWN override, not Animal's.
        assert_ne!(
            animal.method(speak).unwrap().func_id,
            dog.method(speak).unwrap().func_id
        );
    }

    /// Parse + resolve; return the module, resolve result, and interner.
    fn resolved(src: &str) -> (HirModule, ResolveResult, StringInterner) {
        let mut interner = StringInterner::new();
        let mut module = pyaot_frontend_python::parse(src, &mut interner).expect("parse");
        let ns = NamespaceTable::single(module.functions.len());
        let result = resolve(&mut module, &ns, &interner).expect("resolve");
        (module, result, interner)
    }

    #[test]
    fn decorated_public_name_is_not_a_function_symbol() {
        // The decorated `add`'s body is renamed `add.<orig>`; the public name is
        // a promoted global slot, never a `Symbol::Function`, so a missed path
        // fails loudly as NameError rather than a silent unwrapped call (6D).
        let src = "\
from typing import Callable
def logged(func: Callable[..., int]) -> Callable[..., int]:
    def wrapper(*args, **kwargs) -> int:
        return func(*args, **kwargs)
    return wrapper
@logged
def add(a, b):
    return a + b
print(add(1, 2))
";
        let (module, result, mut interner) = resolved(src);
        // No resolved symbol maps the public name `add` to a Function.
        let add = interner.intern("add");
        // The renamed body exists as a function.
        assert!(module
            .functions
            .iter()
            .any(|f| interner.resolve(f.name) == "add.<orig>"));
        // ... and a `<uniform>` value-call adapter thunk for it.
        assert!(module
            .functions
            .iter()
            .any(|f| interner.resolve(f.name).contains("add.<orig>.<uniform>")));
        // Every resolved `add` name occurrence is NOT a Symbol::Function.
        for func in &module.functions {
            for (_idx, expr) in func.exprs.iter() {
                if let HirExprKind::Name(SymbolRef::Resolved(id)) = expr.kind {
                    if let Symbol::Function(fid) = result.symbol(id) {
                        assert_ne!(
                            module.functions[fid.index()].name,
                            add,
                            "the public decorated name must not resolve to a Function symbol"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn nested_synthetic_functions_resolve() {
        // Synthetic nested-def / lambda functions are ordinary entries in the
        // function table and resolve like any other (no dangling names).
        let src = "\
from typing import Callable
def outer() -> Callable[[int], int]:
    def inner(x: int) -> int:
        return x + 1
    return inner
g = outer()
print(g(41))
";
        let (module, _result, interner) = resolved(src);
        // The nested `inner` got a synthetic `<locals>` name.
        assert!(module
            .functions
            .iter()
            .any(|f| interner.resolve(f.name).contains("<locals>")));
    }

    #[test]
    fn super_and_override_resolution() {
        let src = "\
class Animal:
    def speak(self) -> str:
        return \"...\"
class Dog(Animal):
    def speak(self) -> str:
        return \"Woof\"
";
        let (t, mut i) = collected(src);
        let speak = i.intern("speak");
        let dog = cid_of(&t, &i, "Dog");
        let animal = cid_of(&t, &i, "Animal");
        // super().speak() from Dog resolves to Animal.speak.
        let sup = t.resolve_super_method(dog, speak).expect("super speak");
        assert_eq!(sup, t.get(animal).unwrap().method(speak).unwrap().func_id);
        // speak is overridden below Animal (by Dog) → polymorphic.
        assert!(t.method_overridden_below(animal, speak));
        // ... but not below Dog (no further override).
        assert!(!t.method_overridden_below(dog, speak));
    }
}
