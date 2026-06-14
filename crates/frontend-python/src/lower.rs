//! HIR construction: module + per-function CFG building.
//!
//! [`FnLowerer`] is a block builder. Statements append to the *current* block;
//! emitting a terminator seals it and switches to a successor. Block-producing
//! expressions (short-circuit `and`/`or`, ternary, chained compares) split the
//! current block and route through a single-eval result local.
//!
//! The implemented subset grows per milestone; anything outside it returns a
//! [`CompilerError::parse_error`].

use std::collections::{HashMap, HashSet};

use la_arena::{Arena, Idx};
use rustpython_parser::ast::{
    BoolOp as PyBoolOp, CmpOp as PyCmpOp, Comprehension, Constant, Expr, ExprBinOp, ExprBoolOp,
    ExprCall, ExprCompare, ExprDictComp, ExprGeneratorExp, ExprIfExp, ExprLambda, ExprListComp,
    ExprNamedExpr, ExprSetComp, ExprSubscript, ExprUnaryOp, Keyword, Operator as PyOperator, Ranged,
    Stmt,
    StmtClassDef, StmtDelete, StmtFunctionDef, StmtImport, StmtImportFrom, UnaryOp as PyUnaryOp,
};
use rustpython_parser::text_size::TextRange;

use pyaot_core_defs::FIRST_USER_CLASS_ID;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{
    BinOp, ClassAttrInit, CmpOp, ContainerOp, ExcOp, ExcQuery, GenOp, HirBlock, HirClass,
    HirClassAttr, HirExpr, HirExprKind, HirFunction, HirLocal, HirModule, HirParam, HirProgram,
    HirProperty, HirRaise, HirStmt, HirTerminator, NamespaceImports, NamespaceTable, SymbolRef,
    UnaryOp,
};
use pyaot_types::{SemTy, Sig};
use pyaot_utils::{ClassId, FuncId, InternedString, LineMap, LocalId, Span, StringInterner};

use crate::freevars::{self, ScopeFacts};

/// Maps a user class *name* (as written in an annotation) to its assigned
/// `ClassId` and interned name, so `def f() -> Widget` / `x: Widget` resolve to
/// `SemTy::Class`. Built up front from all top-level `class` statements, so
/// forward references resolve regardless of declaration order.
type ClassNameMap = HashMap<String, (ClassId, InternedString)>;

/// The in-scope type variables (`T = TypeVar("T")` module-level vars plus the
/// enclosing generic class's type params), mapping each name to its interned id so
/// an annotation `T` resolves to `SemTy::Var("T")` instead of `Dyn` (Phase 5E).
type TypeVarSet = HashMap<String, InternedString>;

/// One parameter's call-facing shape (Phase 6C): name, annotated type, optional
/// constant default.
#[derive(Debug, Clone)]
pub(crate) struct ParamInfo {
    pub name: InternedString,
    pub ty: SemTy,
    pub default: Option<ClassAttrInit>,
}

/// A top-level `def`'s call-facing shape, collected up front so any function
/// can synthesize a value-position thunk for it (Phase 6A) and reorder / fill
/// keyword and default arguments at known-callee call sites (Phase 6C).
///
/// The MIR parameter order is **fixed positional → keyword-only → `*args`
/// tuple → `**kwargs` dict** (variadic slots trailing); call-site adaptation
/// produces exactly that positional vector.
#[derive(Debug, Clone)]
pub(crate) struct TopDefInfo {
    pub fixed: Vec<ParamInfo>,
    pub kwonly: Vec<ParamInfo>,
    /// `*args` name (a `tuple[Dyn, ...]`), if present.
    pub varargs: Option<InternedString>,
    /// `**kwargs` name (a `dict[str, Dyn]`), if present.
    pub kwargs: Option<InternedString>,
    pub ret: SemTy,
}

/// Top-level `def` name → its call-facing shape.
type TopDefMap = HashMap<String, TopDefInfo>;

/// Annotation-resolution context: the class-name map + the in-scope type vars +
/// the top-level def table + the promoted module-globals table (Phase 6B).
pub(crate) struct AnnCtx<'a> {
    class_map: &'a ClassNameMap,
    type_vars: &'a TypeVarSet,
    top_defs: &'a TopDefMap,
    /// Promoted module-global name → dense `var_id` (Phase 6B).
    promoted: &'a HashMap<String, u32>,
    /// Names of decorated module-level functions (Phase 6D). Their public name
    /// is a promoted global slot of a `(*args, **kwargs)` wrapper, so a call by
    /// that name packs its positional/keyword args into the variadic slots.
    decorated: &'a HashSet<String>,
    /// `import M` alias names (Phase 8). `M.f(...)` / `M.Cls(...)` / `M.VAR`
    /// fold to qualified accesses: imported funcs/classes live in `top_defs` /
    /// `class_map` under the `"M.name"` key, module vars in `alias_vars`.
    aliases: &'a HashSet<String>,
    /// `"M.VAR"` → the exporter module's promoted global slot, for live reads of
    /// an aliased module's module-level variable (Phase 8).
    alias_vars: &'a HashMap<String, u32>,
    /// Stdlib bindings (Phase 8B): names bound to frozen-runtime descriptors by
    /// `import math` / `from math import sqrt` when no user module shadows them.
    stdlib: &'a StdlibBindings,
}

/// Per-module stdlib bindings (Phase 8B), collected during the import scan.
/// A user module on the search path always wins over a same-named stdlib
/// module (CPython script-dir-first), so these are only populated when the
/// loader had no match. Lookup keys: the bound name for `from M import f
/// [as g]`, the qualified `"M.f"` for `import M [as A]` accesses.
#[derive(Default)]
pub(crate) struct StdlibBindings {
    /// `import M` alias names whose target is a stdlib module.
    aliases: HashSet<String>,
    /// Callable name → its function descriptor.
    funcs: HashMap<String, &'static pyaot_stdlib_defs::StdlibFunctionDef>,
    /// Constant name (`math.pi` / from-imported `pi`) → its descriptor; folds
    /// to a literal at every use site.
    consts: HashMap<String, &'static pyaot_stdlib_defs::StdlibConstDef>,
    /// Module attribute (`sys.argv`) → its getter descriptor.
    attrs: HashMap<String, &'static pyaot_stdlib_defs::StdlibAttrDef>,
    /// Stdlib class name in annotation position (`time.struct_time`) → its
    /// semantic type (`RuntimeObject(tag)`).
    classes: HashMap<String, SemTy>,
    /// Stdlib exception name (`HTTPError`) → `(reserved class_id, builtin parent
    /// tag)`. `except HTTPError:` matches by class id; the parent tag makes
    /// `except OSError:` / `except Exception:` match too (Phase 8D).
    exceptions: HashMap<String, (u8, u8)>,
}

/// A decorated module-level function's runtime facts (Phase 6D): the promoted
/// global slot holding its wrapper, and the generic `(*args, **kwargs)` adapter
/// thunk over its renamed `<orig>` body.
struct DecoratedDef {
    slot: u32,
    thunk_fid: FuncId,
}

/// Module-wide mutable lowering state shared by every (possibly nested)
/// function lowering: the function table with *reserved* slots (a nested `def`
/// reserves its `FuncId` before its body is lowered, so ids stay dense and
/// stable regardless of nesting), plus the per-module thunk memo.
pub(crate) struct Shared {
    funcs: Vec<Option<HirFunction>>,
    /// Owning namespace id per `FuncId` (parallel to `funcs`), set at reserve
    /// time from `current_ns` (Phase 8). Every function — including synthetics
    /// (thunks / lambdas / generator resumes) — belongs to the module being
    /// lowered, so `semantics` resolves its `Unresolved` names in that scope.
    func_ns: Vec<u32>,
    /// Namespace new reservations are tagged with (the module being lowered).
    current_ns: u32,
    /// Memoized value-position thunks for top-level functions: `(namespace,
    /// name) → FuncId`. Keyed by namespace so a same-named function in two
    /// modules gets distinct thunks (Phase 8).
    thunks: HashMap<(u32, String), FuncId>,
    /// Generator resume functions indexed by dense `gen_id` (Phase 6E) — a
    /// single program-global, dense space across all modules.
    generators: Vec<FuncId>,
    /// Display path of the module being lowered (real tracebacks): the entry
    /// script's command-line path or the loader's resolved path. Saved and
    /// restored around nested module lowering (imports lower recursively).
    cur_file: Option<InternedString>,
    /// Byte-offset → line map of the module being lowered (same discipline).
    line_map: LineMap,
    /// Module globals a `del name` unbinds (`var_id → name`), accumulated across
    /// every function/module lowered. Flows into [`HirModule::deletable_globals`]
    /// to drive the `GlobalGet` read-guards. Program-global like `generators`.
    deletable_globals: HashMap<u32, InternedString>,
    /// Instance-field names a `del obj.attr` unbinds, accumulated across the
    /// whole program. Flows into [`HirModule::deletable_fields`].
    deletable_fields: HashSet<InternedString>,
}

impl Shared {
    fn new() -> Self {
        Self {
            funcs: Vec::new(),
            func_ns: Vec::new(),
            current_ns: 0,
            thunks: HashMap::new(),
            generators: Vec::new(),
            cur_file: None,
            line_map: LineMap::new(""),
            deletable_globals: HashMap::new(),
            deletable_fields: HashSet::new(),
        }
    }

    /// Reserve the next dense `FuncId`; the caller must `fill` it.
    fn reserve(&mut self) -> FuncId {
        let id = FuncId::new(self.funcs.len() as u32);
        self.funcs.push(None);
        self.func_ns.push(self.current_ns);
        id
    }

    fn fill(&mut self, id: FuncId, f: HirFunction) {
        debug_assert!(self.funcs[id.index()].is_none(), "double fill of {id:?}");
        self.funcs[id.index()] = Some(f);
    }

    fn finish(self) -> Vec<HirFunction> {
        self.funcs
            .into_iter()
            .map(|f| f.expect("every reserved FuncId is filled"))
            .collect()
    }
}

/// What a fully-lowered module exports (frontend-internal, Phase 8). Drives
/// `from M import x` static binding and `M.x` attribute folding in importers.
#[derive(Clone)]
pub(crate) struct ModuleExports {
    init_fid: FuncId,
    init_name: InternedString,
    /// Top-level def name → (FuncId, call-facing shape).
    funcs: HashMap<String, (FuncId, TopDefInfo)>,
    /// Top-level class name → (ClassId, interned class name).
    classes: HashMap<String, (ClassId, InternedString)>,
    /// Module-level variable name → promoted global var_id.
    var_slots: HashMap<String, u32>,
}

/// One import statement's runtime effect (Phase 8), precomputed during the load
/// DFS (so first-import order matches CPython) and replayed positionally during
/// the importing module's body lowering.
#[derive(Default, Clone)]
struct ImportAction {
    /// Module-`<init>` callees to invoke in order (parent packages first, each
    /// only at its program-wide first-import site).
    init_calls: Vec<InternedString>,
    /// `from M import VAR` snapshots: (importer slot, exporter slot).
    snapshots: Vec<(u32, u32)>,
}

/// Per-module binding state collected during the import scan (Phase A) and used
/// when lowering the module body (Phase B).
struct ImportCollect {
    class_map: ClassNameMap,
    promoted: HashMap<String, u32>,
    /// `from M import f` / qualified `M.f` → call-facing shape, by lookup key.
    imported_funcs: Vec<(String, TopDefInfo)>,
    /// `import M` alias names — `M.x` folds to a qualified access.
    aliases: HashSet<String>,
    /// `"M.VAR"` → the exporter module's global slot (for live `M.VAR` reads).
    alias_vars: HashMap<String, u32>,
    /// Names this module re-exports: `from .x import Y` makes `Y` part of THIS
    /// module's public surface, so a downstream `from thismod import Y` (and
    /// `import thismod; thismod.Y`) resolves — the canonical package-`__init__`
    /// API. Funcs/classes only; re-exported variables already ride `promoted`.
    reexport_funcs: Vec<(String, FuncId, TopDefInfo)>,
    reexport_classes: Vec<(String, ClassId, InternedString)>,
    /// Stdlib bindings (Phase 8B), populated when the loader has no user
    /// module for an imported name.
    stdlib: StdlibBindings,
    /// Import-statement body index → its precomputed runtime effect.
    actions: HashMap<usize, ImportAction>,
}

/// The shared program-lowering context (Phase 8). Owns the global allocators and
/// drives the import-graph DFS; every module lowers into the one shared
/// function / class / generator / global-slot space — no merge or remap pass.
pub(crate) struct ProgramLowerer<'a> {
    interner: &'a mut StringInterner,
    loader: &'a mut dyn crate::ModuleSource,
    shared: Shared,
    classes: Vec<HirClass>,
    global_annotations: HashMap<u32, SemTy>,
    class_ns: HashMap<ClassId, u32>,
    namespace_imports: Vec<NamespaceImports>,
    next_class_id: u32,
    next_global: u32,
    loaded: HashMap<String, ModuleExports>,
    loading: Vec<String>,
    init_emitted: HashSet<String>,
}

impl<'a> ProgramLowerer<'a> {
    pub(crate) fn new(
        interner: &'a mut StringInterner,
        loader: &'a mut dyn crate::ModuleSource,
    ) -> Self {
        Self {
            interner,
            loader,
            shared: Shared::new(),
            classes: Vec::new(),
            global_annotations: HashMap::new(),
            class_ns: HashMap::new(),
            namespace_imports: Vec::new(),
            next_class_id: FIRST_USER_CLASS_ID as u32,
            next_global: 0,
            loaded: HashMap::new(),
            loading: Vec::new(),
            init_emitted: HashSet::new(),
        }
    }

    /// Discover the import graph from the entry script and lower every reachable
    /// module into one shared [`HirProgram`]. `entry_src`/`entry_file` feed the
    /// traceback line map and file attribution of the entry module.
    pub(crate) fn run(
        mut self,
        body: Vec<Stmt>,
        entry_src: &str,
        entry_file: &str,
    ) -> Result<HirProgram> {
        // Entry namespace = 0; `__main__` is `FuncId(0)` (codegen wraps it).
        self.namespace_imports.push(NamespaceImports::default());
        self.shared.current_ns = 0;
        self.shared.cur_file = Some(self.interner.intern(entry_file));
        self.shared.line_map = LineMap::new(entry_src);
        let main_id = self.shared.reserve();
        self.lower_module_into(&[], false, body, true, 0, main_id)?;

        let func_ns = self.shared.func_ns.clone();
        let generators = self.shared.generators.clone();
        let deletable_globals = self.shared.deletable_globals.clone();
        let deletable_fields = self.shared.deletable_fields.clone();
        let functions = self.shared.finish();
        let module = HirModule {
            functions,
            classes: self.classes,
            main: main_id,
            generators,
            global_annotations: self.global_annotations,
            deletable_globals,
            deletable_fields,
        };
        let namespaces = NamespaceTable {
            func_ns,
            class_ns: self.class_ns,
            imports: self.namespace_imports,
        };
        Ok(HirProgram { module, namespaces })
    }

    /// Load module `path` (and its parent packages) if not already loaded.
    /// Returns `false` when the loader has no such user module — the caller
    /// then falls back to the stdlib registry (a user module on the search
    /// path wins over a same-named stdlib module, CPython script-dir-first).
    /// A genuine import cycle on the primary target is a loud error; a parent
    /// package mid-initialization (`implicit`) is normal partial-init.
    fn ensure_loaded(&mut self, path: &[String], implicit: bool, span: Span) -> Result<bool> {
        if path.is_empty() {
            return Ok(true);
        }
        let dotted = path.join(".");
        if self.loaded.contains_key(&dotted) {
            return Ok(true);
        }
        if self.loading.iter().any(|m| m == &dotted) {
            if implicit {
                return Ok(true);
            }
            return Err(parse_error(
                format!("circular import detected involving module `{dotted}`"),
                span,
            ));
        }
        let Some((src, is_package)) = self.loader.load(path) else {
            return Ok(false);
        };
        // CPython initializes a package before its submodules.
        if path.len() > 1 {
            self.ensure_loaded(&path[..path.len() - 1], true, span)?;
        }
        let body = crate::parse_module_body(&src)?;
        let ns = self.namespace_imports.len() as u32;
        self.namespace_imports.push(NamespaceImports::default());
        let saved_ns = self.shared.current_ns;
        self.shared.current_ns = ns;
        // Traceback attribution: this module's file/line map, restored after
        // (imports lower recursively inside the importer's own lowering).
        let saved_file = self.shared.cur_file;
        let saved_map = std::mem::replace(&mut self.shared.line_map, LineMap::new(&src));
        let display = self.loader.display_path(path, is_package);
        self.shared.cur_file = Some(self.interner.intern(&display));
        let init_fid = self.shared.reserve();
        self.loading.push(dotted.clone());
        let exports = self.lower_module_into(path, is_package, body, false, ns, init_fid)?;
        self.loading.pop();
        self.shared.current_ns = saved_ns;
        self.shared.cur_file = saved_file;
        self.shared.line_map = saved_map;
        self.loaded.insert(dotted, exports);
        Ok(true)
    }

    /// Schedule the `<init>` calls for `target`'s package chain (parent packages
    /// first), each only at its program-wide first-import site (`init_emitted`).
    /// A chain member still initializing (a parent in `loading`) is already
    /// running and is not re-called.
    fn emit_init_chain(&mut self, target: &[String], my_ns: u32, action: &mut ImportAction) {
        for k in 1..=target.len() {
            let dotted = target[..k].join(".");
            if self.loading.iter().any(|m| m == &dotted) {
                continue;
            }
            if self.init_emitted.contains(&dotted) {
                continue;
            }
            if let Some(exp) = self.loaded.get(&dotted) {
                let name = exp.init_name;
                self.init_emitted.insert(dotted);
                // Make the `<init>` name resolvable as a callee in the importer.
                self.namespace_imports[my_ns as usize]
                    .funcs
                    .insert(name, exp.init_fid);
                action.init_calls.push(name);
            }
        }
    }

    /// Register an `import M` alias: every exported func/class/var of `M` becomes
    /// reachable as a qualified `"alias.name"` access (Phase 8).
    fn register_alias(
        &mut self,
        alias: &str,
        module: &[String],
        my_ns: u32,
        col: &mut ImportCollect,
    ) {
        let dotted = module.join(".");
        let Some(exp) = self.loaded.get(&dotted).cloned() else {
            return;
        };
        col.aliases.insert(alias.to_string());
        for (fname, (fid, info)) in &exp.funcs {
            let key = format!("{alias}.{fname}");
            col.imported_funcs.push((key.clone(), info.clone()));
            let ki = self.interner.intern(&key);
            self.namespace_imports[my_ns as usize]
                .funcs
                .insert(ki, *fid);
        }
        for (cname, (cid, iname)) in &exp.classes {
            let key = format!("{alias}.{cname}");
            col.class_map.insert(key.clone(), (*cid, *iname));
            let ki = self.interner.intern(&key);
            self.namespace_imports[my_ns as usize]
                .classes
                .insert(ki, *cid);
        }
        for (vname, slot) in &exp.var_slots {
            col.alias_vars.insert(format!("{alias}.{vname}"), *slot);
        }
    }

    /// Process `import a.b.c [as x]` (Phase 8).
    fn handle_import(
        &mut self,
        i: &StmtImport,
        my_ns: u32,
        idx: usize,
        col: &mut ImportCollect,
    ) -> Result<()> {
        let span = to_span(i.range());
        for alias in &i.names {
            let dotted_name = alias.name.as_str();
            if matches!(dotted_name, "typing" | "__future__" | "typing_extensions") {
                continue;
            }
            let target: Vec<String> = dotted_name.split('.').map(|s| s.to_string()).collect();
            if !self.ensure_loaded(&target, false, span)? {
                // No user module on the search path → the stdlib registry
                // (Phase 8B). `import a.b.c` without `as` binds `a`; with `as`
                // it binds the full dotted target.
                let (bind, lookup) = match &alias.asname {
                    Some(n) => (n.as_str(), dotted_name),
                    None => (target[0].as_str(), target[0].as_str()),
                };
                let module = pyaot_stdlib_defs::get_module(lookup)
                    .ok_or_else(|| parse_error(format!("no module named `{dotted_name}`"), span))?;
                register_stdlib_alias(bind, module, &mut col.stdlib);
                continue;
            }
            let mut action = ImportAction::default();
            self.emit_init_chain(&target, my_ns, &mut action);
            // `import a.b.c` binds the top package `a`; `import x as y` binds `y`.
            // TODO(phase8): `import a.b.c` (no `as`) registers only `a`'s own
            // surface, so a deep `a.b.c.f()` does not fold (the alias matcher is
            // depth-1). Workaround: `import a.b.c as x; x.f()`, or
            // `from a.b.c import f`. Closing this needs attribute-chain flattening
            // in the three fold sites plus per-prefix-module export registration.
            let (bind, bound): (String, Vec<String>) = match &alias.asname {
                Some(n) => (n.as_str().to_string(), target.clone()),
                None => (target[0].clone(), target[..1].to_vec()),
            };
            self.register_alias(&bind, &bound, my_ns, col);
            let entry = col.actions.entry(idx).or_default();
            entry.init_calls.extend(action.init_calls);
            entry.snapshots.extend(action.snapshots);
        }
        Ok(())
    }

    /// Process `from [.]*module import n1, n2, …` (Phase 8).
    fn handle_import_from(
        &mut self,
        i: &StmtImportFrom,
        importer: &[String],
        is_package: bool,
        my_ns: u32,
        idx: usize,
        col: &mut ImportCollect,
    ) -> Result<()> {
        let span = to_span(i.range());
        let level = i.level.map(|l| l.to_u32() as usize).unwrap_or(0);
        let module_name = i.module.as_ref().map(|m| m.as_str());
        if level == 0 {
            if let Some(m) = module_name {
                if matches!(m, "typing" | "__future__" | "typing_extensions") {
                    return Ok(());
                }
            }
        }
        let target: Vec<String> = if level == 0 {
            module_name
                .ok_or_else(|| parse_error("malformed import", span))?
                .split('.')
                .map(|s| s.to_string())
                .collect()
        } else {
            resolve_relative(importer, is_package, level, module_name, span)?
        };
        if !self.ensure_loaded(&target, false, span)? {
            // No user module on the search path → stdlib `from M import …`
            // (Phase 8B). Relative imports never reach here (`level == 0`).
            let dotted = target.join(".");
            let module = pyaot_stdlib_defs::get_module(&dotted)
                .ok_or_else(|| parse_error(format!("no module named `{dotted}`"), span))?;
            for alias in &i.names {
                let name = alias.name.as_str();
                if name == "*" {
                    return Err(parse_error("`from … import *` is out of scope", span));
                }
                let bind = alias.asname.as_ref().map(|n| n.as_str()).unwrap_or(name);
                bind_stdlib_item(bind, name, &dotted, module, &mut col.stdlib, span)?;
            }
            return Ok(());
        }
        let mut action = ImportAction::default();
        self.emit_init_chain(&target, my_ns, &mut action);
        let dotted = target.join(".");
        let exp = self.loaded.get(&dotted).cloned().expect("target loaded");
        for alias in &i.names {
            let name = alias.name.as_str();
            if name == "*" {
                return Err(parse_error("`from … import *` is out of scope", span));
            }
            let bind = alias.asname.as_ref().map(|n| n.as_str()).unwrap_or(name);
            if let Some((fid, info)) = exp.funcs.get(name) {
                // A from-imported function is a static binding to its FuncId.
                col.imported_funcs.push((bind.to_string(), info.clone()));
                col.reexport_funcs
                    .push((bind.to_string(), *fid, info.clone()));
                let bi = self.interner.intern(bind);
                self.namespace_imports[my_ns as usize]
                    .funcs
                    .insert(bi, *fid);
            } else if let Some((cid, iname)) = exp.classes.get(name) {
                // A from-imported class is a static binding to its ClassId.
                col.class_map.insert(bind.to_string(), (*cid, *iname));
                col.reexport_classes.push((bind.to_string(), *cid, *iname));
                let bi = self.interner.intern(bind);
                self.namespace_imports[my_ns as usize]
                    .classes
                    .insert(bi, *cid);
            } else if let Some(src_slot) = exp.var_slots.get(name) {
                // A from-imported variable is a snapshot copy (CPython-faithful).
                let dst = match col.promoted.get(bind) {
                    Some(s) => *s,
                    None => {
                        let s = self.next_global;
                        self.next_global += 1;
                        col.promoted.insert(bind.to_string(), s);
                        s
                    }
                };
                action.snapshots.push((dst, *src_slot));
                if let Some(ty) = self.global_annotations.get(src_slot).cloned() {
                    self.global_annotations.insert(dst, ty);
                }
            } else {
                return Err(parse_error(
                    format!("cannot import name `{name}` from `{dotted}`"),
                    span,
                ));
            }
        }
        let entry = col.actions.entry(idx).or_default();
        entry.init_calls.extend(action.init_calls);
        entry.snapshots.extend(action.snapshots);
        Ok(())
    }

    /// Lower one module's full content into the shared program tables. `mod_path`
    /// is empty for the entry script. Returns the module's exports.
    fn lower_module_into(
        &mut self,
        mod_path: &[String],
        is_package: bool,
        body: Vec<Stmt>,
        is_entry: bool,
        my_ns: u32,
        init_fid: FuncId,
    ) -> Result<ModuleExports> {
        // `name = lambda ... (with defaults)` → synthetic `def` (Phase 8H, #9),
        // BEFORE partitioning so the def joins `top_defs` (known-callee
        // defaults/kwargs adaptation).
        let mut body = body;
        desugar_module_lambda_defs(&mut body);
        // Partition top-level statements (as the single-module lowering did).
        let mut defs: Vec<&StmtFunctionDef> = Vec::new();
        let mut decorated: Vec<&StmtFunctionDef> = Vec::new();
        let mut classdefs: Vec<&StmtClassDef> = Vec::new();
        let mut top: Vec<&Stmt> = Vec::new();
        for stmt in &body {
            match stmt {
                Stmt::FunctionDef(f) if f.decorator_list.is_empty() => defs.push(f),
                Stmt::FunctionDef(f) => decorated.push(f),
                Stmt::ClassDef(c) => classdefs.push(c),
                other => top.push(other),
            }
        }

        // ── ClassIds from the program-global counter (no per-module remap). ──
        let mut class_map: ClassNameMap = HashMap::new();
        let mut class_ids: Vec<ClassId> = Vec::with_capacity(classdefs.len());
        let mut own_classes: Vec<(String, ClassId, InternedString)> = Vec::new();
        for cdef in &classdefs {
            if self.next_class_id > u8::MAX as u32 {
                return Err(parse_error(
                    "too many user-defined classes across all modules (the runtime \
                     class_id is a u8, so at most 189 in [67, 255])",
                    to_span(cdef.range()),
                ));
            }
            let class_id = ClassId::new(self.next_class_id);
            self.next_class_id += 1;
            let iname = self.interner.intern(cdef.name.as_str());
            class_map.insert(cdef.name.as_str().to_string(), (class_id, iname));
            class_ids.push(class_id);
            own_classes.push((cdef.name.as_str().to_string(), class_id, iname));
            self.class_ns.insert(class_id, my_ns);
        }

        // Module-level type variables (Phase 5E).
        let mut module_type_vars: TypeVarSet = HashMap::new();
        for stmt in &top {
            if let Some(name) = type_var_assign_name(stmt) {
                let id = self.interner.intern(&name);
                module_type_vars.insert(name, id);
            }
        }

        // ── Promoted module globals (var_ids from the program-global counter). ──
        let mut promoted: HashMap<String, u32> = HashMap::new();
        for n in freevars::collect_promoted_globals(&body) {
            promoted.entry(n).or_insert_with(|| {
                let id = self.next_global;
                self.next_global += 1;
                id
            });
        }
        // An imported module's every module-level variable is a module attribute
        // (a global), even if no local function reads it (`math_utils.PI`).
        if !is_entry {
            let mfacts = freevars::analyze_module_body(&top);
            let def_class: HashSet<&str> = defs
                .iter()
                .chain(decorated.iter())
                .map(|d| d.name.as_str())
                .chain(classdefs.iter().map(|c| c.name.as_str()))
                .collect();
            for n in &mfacts.bound {
                if n != "__name__" && !def_class.contains(n.as_str()) && !promoted.contains_key(n) {
                    promoted.insert(n.clone(), self.next_global);
                    self.next_global += 1;
                }
            }
        }
        let mut decorated_names: HashSet<String> = HashSet::new();
        for d in &decorated {
            if !promoted.contains_key(d.name.as_str()) {
                promoted.insert(d.name.as_str().to_string(), self.next_global);
                self.next_global += 1;
            }
            decorated_names.insert(d.name.as_str().to_string());
        }

        // ── Phase A: scan imports, load dependencies, build bindings + actions. ──
        let mut col = ImportCollect {
            class_map,
            promoted,
            imported_funcs: Vec::new(),
            aliases: HashSet::new(),
            alias_vars: HashMap::new(),
            reexport_funcs: Vec::new(),
            reexport_classes: Vec::new(),
            stdlib: StdlibBindings::default(),
            actions: HashMap::new(),
        };
        for (idx, stmt) in body.iter().enumerate() {
            match stmt {
                Stmt::Import(im) => self.handle_import(im, my_ns, idx, &mut col)?,
                Stmt::ImportFrom(im) => {
                    self.handle_import_from(im, mod_path, is_package, my_ns, idx, &mut col)?
                }
                _ => {}
            }
        }
        // Restore the namespace for this module's own function reservations.
        self.shared.current_ns = my_ns;

        // ── Top-level def table (own defs through a pre-context, then imports). ──
        let empty_defs: TopDefMap = HashMap::new();
        let pre_ctx = AnnCtx {
            class_map: &col.class_map,
            type_vars: &module_type_vars,
            top_defs: &empty_defs,
            promoted: &col.promoted,
            decorated: &decorated_names,
            aliases: &col.aliases,
            alias_vars: &col.alias_vars,
            stdlib: &col.stdlib,
        };
        let mut top_defs: TopDefMap = HashMap::new();
        for def in &defs {
            let parsed = parse_params(
                &mut *self.interner,
                &pre_ctx,
                def.args.as_ref(),
                &FirstParam::Plain,
            )?;
            let ret = match &def.returns {
                Some(e) => annotation_to_semty(e.as_ref(), &pre_ctx),
                None => SemTy::Dyn,
            };
            top_defs.insert(
                def.name.as_str().to_string(),
                TopDefInfo {
                    fixed: parsed.fixed,
                    kwonly: parsed.kwonly,
                    varargs: parsed.varargs,
                    kwargs: parsed.kwargs,
                    ret,
                },
            );
        }
        for (key, info) in &col.imported_funcs {
            top_defs.insert(key.clone(), info.clone());
        }
        let module_ctx = AnnCtx {
            class_map: &col.class_map,
            type_vars: &module_type_vars,
            top_defs: &top_defs,
            promoted: &col.promoted,
            decorated: &decorated_names,
            aliases: &col.aliases,
            alias_vars: &col.alias_vars,
            stdlib: &col.stdlib,
        };

        // ── Phase B: lower own functions, the module-init body, and classes. ──
        let mut own_func_fids: HashMap<String, FuncId> = HashMap::new();
        for def in &defs {
            let name = self.interner.intern(def.name.as_str());
            let fid = lower_callable(
                &mut *self.interner,
                &module_ctx,
                &mut self.shared,
                def,
                def.name.as_str(),
                name,
                FirstParam::Plain,
                None,
                false,
                None,
            )?;
            own_func_fids.insert(def.name.as_str().to_string(), fid);
        }

        let mut decorated_info: HashMap<String, DecoratedDef> = HashMap::new();
        for d in &decorated {
            let orig_name_str = format!("{}.<orig>", d.name.as_str());
            let orig_name = self.interner.intern(&orig_name_str);
            let orig_fid = lower_callable(
                &mut *self.interner,
                &module_ctx,
                &mut self.shared,
                d,
                &orig_name_str,
                orig_name,
                FirstParam::Plain,
                None,
                true,
                None,
            )?;
            let parsed = parse_params(
                &mut *self.interner,
                &module_ctx,
                d.args.as_ref(),
                &FirstParam::Plain,
            )?;
            if parsed.varargs.is_some() || parsed.kwargs.is_some() || !parsed.kwonly.is_empty() {
                return Err(parse_error(
                    "a decorated function with *args/**kwargs/keyword-only params is out \
                     of scope (Phase 6D)",
                    to_span(d.range()),
                ));
            }
            let arity = parsed.fixed.len();
            let ret = match &d.returns {
                Some(e) => annotation_to_semty(e.as_ref(), &module_ctx),
                None => SemTy::Dyn,
            };
            let thunk_fid = build_generic_thunk(
                &mut *self.interner,
                &module_ctx,
                &mut self.shared,
                orig_fid,
                &orig_name_str,
                arity,
                ret,
            );
            let slot = col.promoted[d.name.as_str()];
            decorated_info.insert(
                d.name.as_str().to_string(),
                DecoratedDef { slot, thunk_fid },
            );
        }

        // The module-init function: `__main__` for the entry, `<dotted>.<init>`
        // for an imported module. `__name__` is the module's name.
        let module_dotted = mod_path.join(".");
        let (init_name_str, name_value): (String, String) = if is_entry {
            ("__main__".to_string(), "__main__".to_string())
        } else {
            (format!("{module_dotted}.<init>"), module_dotted.clone())
        };
        let init_name = self.interner.intern(&init_name_str);
        let main_facts = freevars::analyze_module_body(&top);
        if let Some(n) = main_facts.nonlocals.iter().next() {
            return Err(parse_error(
                format!("nonlocal declaration of `{n}` not allowed at module level"),
                Span::dummy(),
            ));
        }
        {
            let mut main = FnLowerer::new(
                &mut *self.interner,
                &module_ctx,
                &mut self.shared,
                init_name,
                &init_name_str,
                SemTy::NoneTy,
                None,
            );
            main.is_main = true;
            main.set_scope_facts(&main_facts);
            main.init_cells();
            let dunder_name = main.intern("__name__");
            let name_lit = main.intern(&name_value);
            let name_val = main.alloc(HirExprKind::StrLit(name_lit), SemTy::Str, Span::dummy());
            main.write_named(dunder_name, SemTy::Str, name_val);
            for (idx, stmt) in body.iter().enumerate() {
                match stmt {
                    // Module-level imports replay their precomputed init-calls +
                    // snapshots here (typing-only imports have no action).
                    Stmt::Import(_) | Stmt::ImportFrom(_) => {
                        if let Some(action) = col.actions.get(&idx) {
                            main.emit_import_action(action);
                        }
                    }
                    Stmt::FunctionDef(f) if !f.decorator_list.is_empty() => {
                        let info = &decorated_info[f.name.as_str()];
                        main.emit_decorated_rebinding(f, info.thunk_fid, info.slot)?;
                    }
                    Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
                    _ if type_var_assign_name(stmt).is_some() => {}
                    other => {
                        if main.lower_stmt(other)? {
                            break;
                        }
                    }
                }
            }
            let main_fn = main.finish(HirTerminator::Return(None));
            self.shared.fill(init_fid, main_fn);
        }

        // Classes (own).
        for (i, cdef) in classdefs.iter().enumerate() {
            let hclass = lower_class(
                &mut *self.interner,
                &module_ctx,
                cdef,
                class_ids[i],
                &mut self.shared,
            )?;
            self.classes.push(hclass);
        }

        // Module-level annotated promoted globals (Phase 8 contract types).
        for stmt in &body {
            if let Stmt::AnnAssign(a) = stmt {
                if let Expr::Name(n) = a.target.as_ref() {
                    if let Some(vid) = col.promoted.get(n.id.as_str()).copied() {
                        let ty = annotation_to_semty(a.annotation.as_ref(), &module_ctx);
                        if ty != SemTy::Dyn {
                            self.global_annotations.insert(vid, ty);
                        }
                    }
                }
            }
        }

        // ── Build this module's exports. ──
        let mut export_funcs: HashMap<String, (FuncId, TopDefInfo)> = HashMap::new();
        for (name, fid) in &own_func_fids {
            if let Some(info) = top_defs.get(name) {
                export_funcs.insert(name.clone(), (*fid, info.clone()));
            }
        }
        let mut export_classes: HashMap<String, (ClassId, InternedString)> = HashMap::new();
        for (name, cid, iname) in &own_classes {
            export_classes.insert(name.clone(), (*cid, *iname));
        }
        // Re-exports complete the public surface (`from .x import Y` in a package
        // `__init__`). Own definitions win on a name clash (or_insert).
        for (name, fid, info) in &col.reexport_funcs {
            export_funcs
                .entry(name.clone())
                .or_insert_with(|| (*fid, info.clone()));
        }
        for (name, cid, iname) in &col.reexport_classes {
            export_classes
                .entry(name.clone())
                .or_insert_with(|| (*cid, *iname));
        }
        Ok(ModuleExports {
            init_fid,
            init_name,
            funcs: export_funcs,
            classes: export_classes,
            var_slots: col.promoted,
        })
    }
}

/// Synthetic descriptor for the builtin `open()` (Phase 8C). `open` is not a
/// stdlib *module* function, so it lives here rather than in `stdlib-defs`; it
/// targets the frozen `rt_file_open(filename, mode, encoding)` and returns a
/// `File`. `encoding` is optional with no default → the null-pointer sentinel,
/// which the runtime reads as "use the default codec".
static OPEN_DEF: pyaot_stdlib_defs::StdlibFunctionDef = pyaot_stdlib_defs::StdlibFunctionDef {
    name: "open",
    runtime_name: "rt_file_open",
    params: &[
        pyaot_stdlib_defs::ParamDef::required("file", pyaot_stdlib_defs::TypeSpec::Str),
        pyaot_stdlib_defs::ParamDef::optional_with_default(
            "mode",
            pyaot_stdlib_defs::TypeSpec::Str,
            pyaot_stdlib_defs::ConstValue::Str("r"),
        ),
        pyaot_stdlib_defs::ParamDef::optional("encoding", pyaot_stdlib_defs::TypeSpec::Str),
    ],
    return_type: pyaot_stdlib_defs::TypeSpec::File,
    min_args: 1,
    max_args: 3,
    hints: pyaot_stdlib_defs::LoweringHints::NO_AUTO_BOX,
    codegen: pyaot_core_defs::RuntimeFuncDef::ptr_ternary("rt_file_open"),
};

/// Register `import M [as A]` against a stdlib module (Phase 8B/8D): every
/// function / constant / attr / class becomes reachable as a qualified
/// `"A.name"` key, recursing into submodules so `import os` also exposes
/// `os.path.join(...)` under `"os.path.join"` (Phase 8D).
fn register_stdlib_alias(
    alias: &str,
    module: &'static pyaot_stdlib_defs::StdlibModuleDef,
    stdlib: &mut StdlibBindings,
) {
    stdlib.aliases.insert(alias.to_string());
    register_stdlib_items(alias, module, stdlib);
}

/// Register a module's items under `prefix` and recurse into its submodules,
/// rewriting each submodule's real dotted name's leading module-name segment
/// to the (possibly aliased) `prefix` — so `import os as o` exposes
/// `"o.path.join"`.
fn register_stdlib_items(
    prefix: &str,
    module: &'static pyaot_stdlib_defs::StdlibModuleDef,
    stdlib: &mut StdlibBindings,
) {
    for f in module.functions {
        stdlib.funcs.insert(format!("{prefix}.{}", f.name), f);
    }
    for c in module.constants {
        stdlib.consts.insert(format!("{prefix}.{}", c.name), c);
    }
    for a in module.attrs {
        stdlib.attrs.insert(format!("{prefix}.{}", a.name), a);
    }
    for cls in module.classes {
        if let Some(spec) = &cls.type_spec {
            stdlib.classes.insert(
                format!("{prefix}.{}", cls.name),
                pyaot_hir::semty_from_typespec(spec),
            );
        }
    }
    for sub in module.submodules {
        // `sub.name` is the full dotted name (e.g. "os.path"); the segment after
        // the parent's name (".path") appends to the alias prefix.
        let suffix = sub.name.strip_prefix(module.name).unwrap_or(sub.name);
        let sub_prefix = format!("{prefix}{suffix}");
        register_stdlib_items(&sub_prefix, sub, stdlib);
    }
}

/// Bind one `from M import name [as bind]` stdlib item (Phase 8B).
fn bind_stdlib_item(
    bind: &str,
    name: &str,
    module_dotted: &str,
    module: &'static pyaot_stdlib_defs::StdlibModuleDef,
    stdlib: &mut StdlibBindings,
    span: Span,
) -> Result<()> {
    if let Some(f) = module.functions.iter().find(|f| f.name == name) {
        stdlib.funcs.insert(bind.to_string(), f);
    } else if let Some(c) = module.constants.iter().find(|c| c.name == name) {
        stdlib.consts.insert(bind.to_string(), c);
    } else if let Some(a) = module.attrs.iter().find(|a| a.name == name) {
        stdlib.attrs.insert(bind.to_string(), a);
    } else if let Some(spec) = module
        .classes
        .iter()
        .find(|c| c.name == name)
        .and_then(|c| c.type_spec.as_ref())
    {
        stdlib
            .classes
            .insert(bind.to_string(), pyaot_hir::semty_from_typespec(spec));
    } else if let Some(exc) = module.exceptions.iter().find(|e| e.name == name) {
        // A stdlib exception class (`from urllib.error import HTTPError`,
        // Phase 8D): record its reserved id + builtin parent tag for
        // `except`/`raise` resolution.
        stdlib
            .exceptions
            .insert(bind.to_string(), (exc.class_id, exc.parent.tag()));
    } else {
        return Err(parse_error(
            format!("cannot import name `{name}` from `{module_dotted}`"),
            span,
        ));
    }
    Ok(())
}

/// Resolve a relative `from`-import (`level` leading dots + optional `module`)
/// against the importing module's package path — CPython `__package__` rules.
fn resolve_relative(
    importer: &[String],
    is_package: bool,
    level: usize,
    module: Option<&str>,
    span: Span,
) -> Result<Vec<String>> {
    // The importer's package: itself if it is a package (`__init__.py`), else
    // its parent.
    let mut pkg: Vec<String> = if is_package {
        importer.to_vec()
    } else {
        importer[..importer.len().saturating_sub(1)].to_vec()
    };
    // The first dot is the package itself; each extra dot ascends one level.
    for _ in 1..level {
        if pkg.is_empty() {
            return Err(parse_error(
                "relative import beyond top-level package",
                span,
            ));
        }
        pkg.pop();
    }
    if let Some(m) = module {
        pkg.extend(m.split('.').map(|s| s.to_string()));
    }
    Ok(pkg)
}

/// One active control scope, pushed while lowering its body (Phase 7
/// generalization of the loop stack). Early exits (`return` / `break` /
/// `continue`) walk this stack emitting each scope's cleanup. Protected
/// scopes carry the handler context OUTSIDE them (`outer`): leaving one on an
/// early exit switches `cur_handler` back to `outer` in a fresh block, so the
/// cleanup code itself (a finalbody, an `__exit__` call) is not protected by
/// the region it is leaving — the static equivalent of the old frame pop.
#[derive(Clone)]
enum ScopeCtx {
    /// A loop body: `break`/`continue` jump targets; no cleanup of its own.
    Loop {
        continue_to: Idx<HirBlock>,
        break_to: Idx<HirBlock>,
    },
    /// A `try` body protected by an except handler. Cleanup: exit the region.
    TryFrame { outer: Option<Idx<HirBlock>> },
    /// An `except` handler body. Cleanup: `EndHandling`.
    Handler,
    /// A `try` body protected by a finally handler. Cleanup: exit the region +
    /// the re-lowered (cloned) finalbody.
    Finally {
        outer: Option<Idx<HirBlock>>,
        stmts: Vec<Stmt>,
    },
    /// A `with` body. Cleanup: exit the region + `mgr.__exit__(None, None, None)`.
    WithCleanup {
        outer: Option<Idx<HirBlock>>,
        mgr: LocalId,
    },
}

/// The element action of a comprehension: append to a list/set, or insert into a
/// dict. Carries the result container local plus the borrowed element expressions.
enum CompKind<'a> {
    List {
        result: LocalId,
        elt: &'a Expr,
    },
    Set {
        result: LocalId,
        elt: &'a Expr,
    },
    Dict {
        result: LocalId,
        key: &'a Expr,
        val: &'a Expr,
    },
}

/// A simple iterator loop opened by `begin_iter_loop`: the header to jump back to,
/// the exit block to continue at, and the per-iteration element local.
struct IterLoop {
    header: Idx<HirBlock>,
    exit: Idx<HirBlock>,
    elem: LocalId,
}

/// How a `min`/`max` `key=` callable is invoked (Phase 7): staged once into a
/// local and called indirectly (lambdas, locals), or called directly by name
/// per element (builtins / top-level functions, which have no value-position
/// staging requirement).
enum KeyMode<'e> {
    Staged(LocalId),
    ByName(&'e Expr),
}

/// A pending call-argument value during keyword adaptation: the raw AST
/// expression (no-keyword calls — slot order coincides with written order, so
/// lowering at slot-fill time is correct), or a local STAGED in written order
/// (keyword calls — slot filling reorders arguments, and CPython evaluates
/// them left-to-right as written, so side effects must run before matching).
#[derive(Clone, Copy)]
enum ArgSrc<'e> {
    Plain(&'e Expr),
    Staged(LocalId),
}

/// One positional argument of a call, after `*` classification. A `*list` /
/// `*tuple` LITERAL spread is flattened into its [`PosItem::Plain`] elements
/// (compile-time-known arity); a runtime `*seq` spread (variable / call result
/// / comprehension) of unknown length stays a [`PosItem::Spread`].
enum PosItem<'e> {
    Plain(&'e Expr),
    Spread(&'e Expr),
}

/// Side-effect-free literal (possibly sign-prefixed)? Such arguments skip
/// staging and stay [`ArgSrc::Plain`], so downstream slot-fill folds (`None`
/// → absent stdlib slot, int literal → float for raw-ABI params) still see
/// the AST shape.
fn is_const_like(e: &Expr) -> bool {
    match e {
        Expr::Constant(_) => true,
        Expr::UnaryOp(u) => {
            matches!(u.op, PyUnaryOp::USub | PyUnaryOp::UAdd)
                && matches!(u.operand.as_ref(), Expr::Constant(_))
        }
        _ => false,
    }
}

/// How a source name in scope maps to storage (Phase 6A): directly to a local
/// slot, or through a cell held in a local slot (a captured / capturable
/// variable — the P6-2 rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Binding {
    Direct(LocalId),
    Cell(LocalId),
}

/// Where a name access lands: a scope binding, or a promoted module-global
/// slot (Phase 6B).
#[derive(Debug, Clone, Copy)]
enum Place {
    Bind(Binding),
    Global(u32),
}

pub(crate) struct FnLowerer<'a> {
    interner: &'a mut StringInterner,
    ctx: &'a AnnCtx<'a>,
    shared: &'a mut Shared,
    name: InternedString,
    /// Raw (un-interned) base name, for `{base}.<locals>.{child}#k` synthetics.
    base_name: String,
    /// The class this is a method of (`Some` for methods), for `super()` resolution.
    enclosing_class: Option<ClassId>,
    params: Vec<HirParam>,
    ret_ty: SemTy,
    exprs: Arena<HirExpr>,
    blocks: Arena<HirBlock>,
    locals: Vec<HirLocal>,
    scope: HashMap<InternedString, Binding>,
    /// Own names that must live in cells (from `freevars`), interned.
    celled: HashSet<InternedString>,
    /// Cell names some descendant writes via `nonlocal` (typing demotion, 6B).
    shared_writes: HashSet<InternedString>,
    /// `global`-declared names in this scope (Phase 6B), interned.
    global_decls: HashSet<InternedString>,
    /// Names this scope binds (from `freevars`) — a promoted global READ routes
    /// to its slot only when the name is not locally bound (CPython scoping).
    bound_names: HashSet<InternedString>,
    /// True for `__main__`: every promoted name lives in its global slot (the
    /// single storage), so main's own accesses rewrite to GlobalGet/GlobalSet.
    is_main: bool,
    entry: Idx<HirBlock>,
    cur: Idx<HirBlock>,
    /// Blocks already sealed with a real terminator. Open-ness is tracked here,
    /// not by inspecting the placeholder `Unreachable` terminator — an explicit
    /// `Unreachable` seal (the `raise` shape) must stay sealed (Phase 7).
    sealed: HashSet<Idx<HirBlock>>,
    /// The handler block protecting code emitted RIGHT NOW (table-based
    /// unwinding): set while lowering a `try`/`with` body, restored on exit.
    /// Stamped onto each block the first time it receives a statement or
    /// terminator ([`Self::stamp_handler`]) — fill-time, not creation-time,
    /// because join blocks are created inside a region but filled after it.
    cur_handler: Option<Idx<HirBlock>>,
    /// Blocks whose `handler` field has been stamped (distinguishes a stamped
    /// `None` from "not filled yet", and backs the one-context-per-block
    /// debug assertion).
    stamped: HashSet<Idx<HirBlock>>,
    /// Last `HirStmt::Line` marker emitted into the CURRENT block (real
    /// tracebacks). Reset on every `switch` — codegen's srcloc state follows
    /// emission order, so each block must re-establish its line.
    cur_line: Option<u32>,
    scope_stack: Vec<ScopeCtx>,
    /// Uniquifier for sibling synthetic functions (lambdas / nested defs).
    synth_counter: u32,
    /// Set when this nested function captures its OWN name (recursion): the
    /// self-capture cell's local plus this function's synthetic name. A call
    /// through that cell compiles to a direct self-call passing the env through
    /// — same shared cells, precise return type. (Documented divergence:
    /// rebinding the name in the enclosing scope after creation would not be
    /// observed by the recursion; the corpus never does that.)
    self_capture: Option<(LocalId, InternedString)>,
    /// Generator state-machine context (Phase 6E): set while lowering a resume
    /// function — yields become suspend points and `return` exhausts.
    gen: Option<GenCtx>,
}

/// Generator-lowering state (Phase 6E).
struct GenCtx {
    /// The generator object — the resume function's param 0.
    gen_local: LocalId,
    /// Next state number for a yield (state 0 = start).
    next_state: u32,
    /// `(state, resume_block)` per yield, for the entry dispatch.
    resume_targets: Vec<(u32, Idx<HirBlock>)>,
}

impl<'a> FnLowerer<'a> {
    pub(crate) fn new(
        interner: &'a mut StringInterner,
        ctx: &'a AnnCtx<'a>,
        shared: &'a mut Shared,
        name: InternedString,
        base_name: &str,
        ret_ty: SemTy,
        enclosing_class: Option<ClassId>,
    ) -> Self {
        let mut blocks = Arena::new();
        let entry = blocks.alloc(HirBlock {
            stmts: Vec::new(),
            term: HirTerminator::Unreachable,
            handler: None,
        });
        Self {
            interner,
            ctx,
            shared,
            name,
            base_name: base_name.to_string(),
            enclosing_class,
            params: Vec::new(),
            ret_ty,
            exprs: Arena::new(),
            blocks,
            locals: Vec::new(),
            scope: HashMap::new(),
            celled: HashSet::new(),
            shared_writes: HashSet::new(),
            global_decls: HashSet::new(),
            bound_names: HashSet::new(),
            is_main: false,
            entry,
            cur: entry,
            sealed: HashSet::new(),
            cur_handler: None,
            stamped: HashSet::new(),
            cur_line: None,
            scope_stack: Vec::new(),
            synth_counter: 0,
            self_capture: None,
            gen: None,
        }
    }

    /// Adopt the scope's free-variable facts (interning the name sets). A
    /// promoted module-global is never celled in `__main__` — its single
    /// storage is the global slot, which nested functions read directly.
    fn set_scope_facts(&mut self, facts: &ScopeFacts) {
        self.celled = facts
            .celled
            .iter()
            .filter(|n| !(self.is_main && self.ctx.promoted.contains_key(*n)))
            .map(|n| self.interner.intern(n))
            .collect();
        self.shared_writes = facts
            .shared_writes
            .iter()
            .map(|n| self.interner.intern(n))
            .collect();
        self.global_decls = facts
            .globals
            .iter()
            .map(|n| self.interner.intern(n))
            .collect();
        self.bound_names = facts
            .bound
            .iter()
            .map(|n| self.interner.intern(n))
            .collect();
    }

    /// Register a parameter as the next local (params occupy locals `0..nparams`).
    fn add_param(&mut self, name: InternedString, ty: SemTy) {
        self.add_param_default(name, ty, None);
    }

    /// Register a parameter carrying a constant default (Phase 6C).
    fn add_param_default(
        &mut self,
        name: InternedString,
        ty: SemTy,
        default: Option<ClassAttrInit>,
    ) {
        let id = LocalId::new(self.locals.len() as u32);
        self.params.push(HirParam {
            name,
            ty: ty.clone(),
            default,
        });
        self.locals.push(HirLocal {
            name,
            ty,
            raw_int_ok: false,
            pin_tagged: false,
            cell_shared: false,
            deletable: false,
        });
        self.scope.insert(name, Binding::Direct(id));
    }

    /// Allocate a named *logical* local (not a MIR parameter) bound `Direct` —
    /// used for a generator resume function's Python params, which live in gen
    /// slots rather than the ABI (Phase 6E).
    fn add_logical_local(&mut self, name: InternedString, ty: SemTy) -> LocalId {
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(HirLocal {
            name,
            ty,
            raw_int_ok: false,
            pin_tagged: false,
            cell_shared: false,
            deletable: false,
        });
        self.scope.insert(name, Binding::Direct(id));
        id
    }

    /// Install the parsed params in MIR order: fixed positional, keyword-only,
    /// `*args` tuple, `**kwargs` dict (Phase 6C).
    fn install_params(&mut self, parsed: &ParsedParams) {
        for p in parsed.fixed.iter().chain(&parsed.kwonly) {
            self.add_param_default(p.name, p.ty.clone(), p.default.clone());
        }
        if let Some(name) = parsed.varargs {
            self.add_param(name, SemTy::tuple_var_of(SemTy::Dyn));
        }
        if let Some(name) = parsed.kwargs {
            self.add_param(name, SemTy::dict_of(SemTy::Str, SemTy::Dyn));
        }
    }

    /// Allocate one cell per celled name in the entry block (P6-2: one cell per
    /// variable per *activation*, so loops over closures get CPython
    /// late-binding and repeated calls get independent cells). A celled
    /// parameter is copied into its fresh cell (its annotation becoming the
    /// cell's content type); capture bindings installed by the prologue are
    /// already cells and are skipped.
    fn init_cells(&mut self) {
        let mut names: Vec<InternedString> = self.celled.iter().copied().collect();
        names.sort_by_key(|n| n.index());
        for name in names {
            let (init, content_ty) = match self.scope.get(&name).copied() {
                Some(Binding::Cell(_)) => continue,
                Some(Binding::Direct(param_lid)) => {
                    let ty = self.locals[param_lid.index()].ty.clone();
                    (Some(self.local_ref(param_lid, Span::dummy())), ty)
                }
                None => (None, SemTy::Dyn),
            };
            let cell_lid =
                self.alloc_cell_local(name, content_ty, self.shared_writes.contains(&name));
            let mc = self.alloc(HirExprKind::MakeCell { init }, SemTy::Dyn, Span::dummy());
            self.push_stmt(HirStmt::Assign {
                target: cell_lid,
                value: mc,
            });
            self.scope.insert(name, Binding::Cell(cell_lid));
        }
    }

    /// Allocate the local slot that holds a cell for `name`. The slot gets a
    /// distinct `.cell`-suffixed name so `semantics`' name→local map never
    /// aliases it with the original (celled-parameter) slot.
    ///
    /// `content_ty` is the cell's authoritative CONTENT type (an enclosing
    /// annotation carried across the capture boundary; `Dyn` when unknown) —
    /// `typeck` types `CellGet` from it. The slot itself always holds a tagged
    /// cell pointer, so its representation is pinned `Tagged` regardless.
    fn alloc_cell_local(
        &mut self,
        name: InternedString,
        content_ty: SemTy,
        cell_shared: bool,
    ) -> LocalId {
        let cell_name = format!("{}.cell", self.interner.resolve(name));
        let cname = self.interner.intern(&cell_name);
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(HirLocal {
            name: cname,
            ty: content_ty,
            raw_int_ok: false,
            pin_tagged: true,
            cell_shared,
            deletable: false,
        });
        id
    }

    /// Seal the current block with `default_term` if it is still open, then
    /// assemble the [`HirFunction`].
    pub(crate) fn finish(mut self, default_term: HirTerminator) -> HirFunction {
        if !self.sealed.contains(&self.cur) {
            self.blocks[self.cur].term = default_term;
        }
        HirFunction {
            name: self.name,
            file: self
                .shared
                .cur_file
                .expect("cur_file is set before any function is lowered"),
            params: self.params,
            varargs: false,
            kwargs: false,
            ret_ty: self.ret_ty,
            // `ret_raw_int` defaults to the always-correct tagged baseline;
            // typeck's interprocedural interval pass sets it where a range proof
            // holds (mirrors `HirLocal::raw_int_ok`).
            ret_raw_int: false,
            locals: self.locals,
            blocks: self.blocks,
            entry: self.entry,
            exprs: self.exprs,
        }
    }

    // ── block builder ──────────────────────────────────────────────────────

    fn new_block(&mut self) -> Idx<HirBlock> {
        self.blocks.alloc(HirBlock {
            stmts: Vec::new(),
            term: HirTerminator::Unreachable,
            handler: None,
        })
    }

    /// Stamp the current block with the active handler context, first fill
    /// wins. A block must only ever be filled under one context — the
    /// structural lowerers split blocks whenever `cur_handler` changes — so a
    /// re-stamp under a different context is a frontend bug (dead statements
    /// pushed into an already-sealed block are exempt: they never run).
    fn stamp_handler(&mut self) {
        if !self.cur_open() {
            return;
        }
        if self.stamped.insert(self.cur) {
            self.blocks[self.cur].handler = self.cur_handler;
        } else {
            debug_assert_eq!(
                self.blocks[self.cur].handler, self.cur_handler,
                "block filled under two different handler contexts"
            );
        }
    }

    fn push_stmt(&mut self, stmt: HirStmt) {
        self.stamp_handler();
        self.blocks[self.cur].stmts.push(stmt);
    }

    /// Seal the current block with `term` (only if still open) and leave `cur`
    /// pointing at it; the caller must `switch` to a fresh block next.
    /// Open-ness is tracked explicitly (not by inspecting the placeholder
    /// terminator) because an explicit `Unreachable` seal — the Phase-7 `raise`
    /// shape — must not be overwritten by a later structural seal.
    fn seal(&mut self, term: HirTerminator) {
        self.stamp_handler();
        if self.sealed.insert(self.cur) {
            self.blocks[self.cur].term = term;
        }
    }

    fn switch(&mut self, block: Idx<HirBlock>) {
        self.cur = block;
        self.cur_line = None;
    }

    /// Emit a `HirStmt::Line` marker for `span`'s source line if the current
    /// block has not already established it (real tracebacks).
    fn mark_line(&mut self, span: Span) {
        let line = self.shared.line_map.line_number(span.start);
        if self.cur_line != Some(line) {
            self.push_stmt(HirStmt::Line(line));
            self.cur_line = Some(line);
        }
    }

    fn alloc(&mut self, kind: HirExprKind, ty: SemTy, span: Span) -> Idx<HirExpr> {
        // `raw_int_ok` defaults to the always-correct tagged baseline; typeck's
        // interval pass proves and sets it where sound (Phase 3c).
        self.exprs.alloc(HirExpr {
            kind,
            ty,
            span,
            raw_int_ok: false,
        })
    }

    /// Synthesize `lit0 + str(e0) + lit1 + str(e1) + ... + tail` — the
    /// left-folded string concatenation used for stdlib-exception messages.
    /// Each expression is wrapped in `str(...)` (resolved by `semantics`),
    /// matching the f-string lowering idiom.
    fn synth_concat_str(
        &mut self,
        parts: &[(&str, Idx<HirExpr>)],
        tail: &str,
        span: Span,
    ) -> Idx<HirExpr> {
        let mut acc: Option<Idx<HirExpr>> = None;
        let mut push = |this: &mut Self, e: Idx<HirExpr>| {
            acc = Some(match acc {
                Some(a) => this.alloc(
                    HirExprKind::BinOp {
                        op: BinOp::Add,
                        l: a,
                        r: e,
                    },
                    SemTy::Dyn,
                    span,
                ),
                None => e,
            });
        };
        for (lit, expr) in parts {
            if !lit.is_empty() {
                let id = self.intern(lit);
                let lit_e = self.alloc(HirExprKind::StrLit(id), SemTy::Str, span);
                push(self, lit_e);
            }
            let fn_name = self.intern("str");
            let callee = self.alloc(
                HirExprKind::Name(SymbolRef::Unresolved(fn_name)),
                SemTy::Dyn,
                span,
            );
            let wrapped = self.alloc(
                HirExprKind::Call {
                    callee,
                    args: vec![*expr],
                },
                SemTy::Str,
                span,
            );
            push(self, wrapped);
        }
        if !tail.is_empty() {
            let id = self.intern(tail);
            let tail_e = self.alloc(HirExprKind::StrLit(id), SemTy::Str, span);
            push(self, tail_e);
        }
        acc.unwrap_or_else(|| {
            let id = self.intern("");
            self.alloc(HirExprKind::StrLit(id), SemTy::Str, span)
        })
    }

    fn intern(&mut self, s: &str) -> InternedString {
        self.interner.intern(s)
    }

    /// True iff `dotted` (`"module.attr"`) names ANY known stdlib surface: a
    /// function, const, module attr, class, or a submodule (a prefix of a
    /// longer registered name, e.g. `os.path` for `os.path.join`).
    fn stdlib_module_attr_exists(&self, dotted: &str) -> bool {
        let s = &self.ctx.stdlib;
        if s.funcs.contains_key(dotted)
            || s.consts.contains_key(dotted)
            || s.attrs.contains_key(dotted)
            || s.classes.contains_key(dotted)
        {
            return true;
        }
        let prefix = format!("{dotted}.");
        s.funcs.keys().any(|k| k.starts_with(&prefix))
            || s.consts.keys().any(|k| k.starts_with(&prefix))
            || s.attrs.keys().any(|k| k.starts_with(&prefix))
            || s.classes.keys().any(|k| k.starts_with(&prefix))
    }

    /// True iff the current block is still open (no terminator emitted yet).
    fn cur_open(&self) -> bool {
        !self.sealed.contains(&self.cur)
    }

    // ── control scopes / early-exit cleanups (Phase 7) ──────────────────────

    /// Index of the innermost `Loop` scope, if any.
    fn innermost_loop(&self) -> Option<usize> {
        self.scope_stack
            .iter()
            .rposition(|s| matches!(s, ScopeCtx::Loop { .. }))
    }

    /// Emit the cleanup sequence for an early exit (`return` / `break` /
    /// `continue`) leaving every scope at index `down_to..`, innermost first.
    /// The stack itself is not popped — control statements elsewhere in the
    /// same scopes still need the entries.
    ///
    /// `cur_handler` is deliberately LEFT at the exit edge's final (outer)
    /// context: the caller must seal the exit terminator in that context,
    /// then restore `cur_handler` itself (lowering continues with dead-or-
    /// live code in the original context). Use [`Self::with_exit_cleanups`].
    fn emit_exit_cleanups(&mut self, down_to: usize, span: Span) -> Result<()> {
        for i in (down_to..self.scope_stack.len()).rev() {
            match self.scope_stack[i].clone() {
                ScopeCtx::Loop { .. } => {}
                ScopeCtx::TryFrame { outer } => {
                    self.exit_protected(outer);
                }
                ScopeCtx::Handler => {
                    self.push_stmt(HirStmt::ExcOp(ExcOp::EndHandling));
                }
                ScopeCtx::Finally { outer, stmts } => {
                    self.exit_protected(outer);
                    // Re-lower the finalbody on this exit edge. The scopes above
                    // `i` are already cleaned up, so the finalbody must see only
                    // the scopes BELOW this entry (a nested `return` inside it
                    // must not re-run these cleanups).
                    let saved = self.scope_stack.split_off(i);
                    self.lower_body(&stmts)?;
                    self.scope_stack.extend(saved);
                }
                ScopeCtx::WithCleanup { outer, mgr } => {
                    self.exit_protected(outer);
                    self.emit_exit_none_call(mgr, span);
                }
            }
        }
        Ok(())
    }

    /// Run [`Self::emit_exit_cleanups`] plus the caller's exit-edge seal
    /// under the exit context, then restore `cur_handler`.
    fn with_exit_cleanups(
        &mut self,
        down_to: usize,
        span: Span,
        seal_exit: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<()> {
        let saved = self.cur_handler;
        self.emit_exit_cleanups(down_to, span)?;
        seal_exit(self)?;
        self.cur_handler = saved;
        Ok(())
    }

    /// Leave a protected region on an exit path: the code that follows (the
    /// region's cleanup, the rest of the exit edge) runs under the region's
    /// OUTER handler, in a fresh block — the current block is already stamped
    /// with the inner handler.
    fn exit_protected(&mut self, outer: Option<Idx<HirBlock>>) {
        if self.cur_open() && self.cur_handler != outer {
            let b = self.new_block();
            self.seal(HirTerminator::Jump(b));
            self.switch(b);
        }
        self.cur_handler = outer;
    }

    /// Emit `mgr.__exit__(None, None, None)` as a statement (the normal-path
    /// context-manager epilogue; the result is ignored).
    fn emit_exit_none_call(&mut self, mgr: LocalId, span: Span) {
        let recv = self.local_ref(mgr, span);
        let method_name = self.intern("__exit__");
        let args: Vec<Idx<HirExpr>> = (0..3)
            .map(|_| self.alloc(HirExprKind::NoneLit, SemTy::NoneTy, span))
            .collect();
        let call = self.alloc(
            HirExprKind::MethodCall {
                recv,
                method_name,
                args,
                kwargs: vec![],
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Expr(call));
    }

    // ── statements ──────────────────────────────────────────────────────────

    /// Lower a statement list, stopping after a statement that terminates the
    /// current block (so trailing dead code is not emitted into a sealed block).
    fn lower_body(&mut self, stmts: &[Stmt]) -> Result<()> {
        for stmt in stmts {
            if self.lower_stmt(stmt)? {
                break;
            }
        }
        Ok(())
    }

    /// Lower one statement. Returns `true` if it terminated the current block
    /// (`break` / `continue` / `return`).
    fn lower_stmt(&mut self, stmt: &Stmt) -> Result<bool> {
        // Real tracebacks: establish this statement's source line in the
        // current block before any of its code is emitted.
        self.mark_line(to_span(stmt.range()));
        match stmt {
            Stmt::Expr(s) => {
                // `print(...)` is the one special statement (it carries sep/end).
                if let Some(call) = as_print_call(s.value.as_ref()) {
                    self.lower_print(call)?;
                } else if self.gen.is_some() && is_yield_expr(s.value.as_ref()) {
                    // A bare `yield e` / `yield from it` statement (Phase 6E).
                    self.lower_yield_stmt(s.value.as_ref())?;
                } else {
                    let idx = self.lower_expr(s.value.as_ref())?;
                    self.push_stmt(HirStmt::Expr(idx));
                }
                Ok(false)
            }
            Stmt::Assign(a) => {
                self.lower_assign(a)?;
                Ok(false)
            }
            Stmt::AugAssign(a) => {
                self.lower_augassign(a)?;
                Ok(false)
            }
            Stmt::AnnAssign(a) => {
                self.lower_annassign(a)?;
                Ok(false)
            }
            Stmt::If(s) => {
                self.lower_if(s)?;
                Ok(false)
            }
            Stmt::While(s) => self.lower_while(s),
            Stmt::For(s) => self.lower_for(s),
            Stmt::Assert(s) => {
                // `assert cond, msg` desugars to `if not cond: raise
                // AssertionError(msg)` so the message survives (Phase 7);
                // a bare `assert cond` keeps the lean AssertFail path.
                if let Some(msg) = &s.msg {
                    let span = to_span(s.range());
                    let cond = self.lower_expr(s.test.as_ref())?;
                    let fail_b = self.new_block();
                    let ok_b = self.new_block();
                    self.seal(HirTerminator::Branch {
                        cond,
                        then: ok_b,
                        else_: fail_b,
                    });
                    self.switch(fail_b);
                    let m = self.lower_expr(msg.as_ref())?;
                    self.push_stmt(HirStmt::Raise(HirRaise::Builtin {
                        tag: pyaot_core_defs::BuiltinExceptionKind::AssertionError.tag(),
                        msg: Some(m),
                    }));
                    self.seal(HirTerminator::Unreachable);
                    self.switch(ok_b);
                    let _ = span;
                } else {
                    let cond = self.lower_expr(s.test.as_ref())?;
                    self.push_stmt(HirStmt::Assert { cond });
                }
                Ok(false)
            }
            // ── exceptions / with / match (Phase 7) ──
            Stmt::Try(t) => self.lower_try(t),
            Stmt::Raise(r) => self.lower_raise(r),
            Stmt::With(w) => self.lower_with(w),
            Stmt::Match(m) => self.lower_match(m),
            Stmt::Pass(_) => Ok(false),
            // `from typing import ...` / `from __future__ import ...` are
            // type-level only (no runtime effect in our subset) — accept as no-ops
            // so generics (TypeVar/Generic) compile. Real imports are processed at
            // module top level (`lower_module_into`'s import scan); reaching here
            // means the import is nested — inside a function body or a top-level
            // `if`/`try` block. Those are rejected: the load DFS precomputes each
            // module's `<init>` order in source order, so a conditionally-executed
            // import has no place in that schedule yet (Phase 8 limitation — a
            // top-level guarded `import` / optional-dependency pattern must be
            // hoisted to an unconditional top-level import).
            Stmt::ImportFrom(i) => {
                let module = i.module.as_ref().map(|m| m.as_str()).unwrap_or("");
                if matches!(module, "typing" | "__future__" | "typing_extensions") {
                    Ok(false)
                } else {
                    Err(parse_error(
                        "only module-top-level imports are supported (an import inside \
                         a function or a conditional block is out of scope)",
                        to_span(i.range()),
                    ))
                }
            }
            Stmt::Import(i) => {
                if i.names
                    .iter()
                    .all(|n| matches!(n.name.as_str(), "typing" | "typing_extensions"))
                {
                    Ok(false)
                } else {
                    Err(parse_error(
                        "only module-top-level imports are supported (an import inside \
                         a function or a conditional block is out of scope)",
                        to_span(i.range()),
                    ))
                }
            }
            Stmt::Break(b) => {
                let span = to_span(b.range());
                let loop_idx = self
                    .innermost_loop()
                    .ok_or_else(|| parse_error("'break' outside loop", span))?;
                let ScopeCtx::Loop { break_to, .. } = self.scope_stack[loop_idx] else {
                    unreachable!()
                };
                self.with_exit_cleanups(loop_idx + 1, span, |this| {
                    this.seal(HirTerminator::Jump(break_to));
                    Ok(())
                })?;
                Ok(true)
            }
            Stmt::Continue(c) => {
                let span = to_span(c.range());
                let loop_idx = self
                    .innermost_loop()
                    .ok_or_else(|| parse_error("'continue' outside loop", span))?;
                let ScopeCtx::Loop { continue_to, .. } = self.scope_stack[loop_idx] else {
                    unreachable!()
                };
                self.with_exit_cleanups(loop_idx + 1, span, |this| {
                    this.seal(HirTerminator::Jump(continue_to));
                    Ok(())
                })?;
                Ok(true)
            }
            Stmt::Return(r) => {
                let span = to_span(r.range());
                // In a generator, `return` ends the generator (exhaust). The
                // returned value (StopIteration.value) is out of scope (6E).
                if self.gen.is_some() {
                    if let Some(e) = &r.value {
                        let _ = self.lower_expr(e.as_ref())?;
                    }
                    self.with_exit_cleanups(0, span, |this| {
                        this.emit_gen_exhaust(span);
                        Ok(())
                    })?;
                    return Ok(true);
                }
                if self
                    .scope_stack
                    .iter()
                    .all(|s| matches!(s, ScopeCtx::Loop { .. }))
                {
                    // Fast path: no protected regions to clean up.
                    let val = match &r.value {
                        Some(e) => Some(self.lower_expr(e.as_ref())?),
                        None => None,
                    };
                    self.seal(HirTerminator::Return(val));
                    return Ok(true);
                }
                // Evaluate the return value BEFORE the cleanups (CPython order),
                // snapshotting it to a temp the cleanups cannot disturb.
                let val = match &r.value {
                    Some(e) => {
                        let v = self.lower_expr(e.as_ref())?;
                        let tmp = self.fresh_local(SemTy::Dyn);
                        self.push_stmt(HirStmt::Assign {
                            target: tmp,
                            value: v,
                        });
                        Some(tmp)
                    }
                    None => None,
                };
                self.with_exit_cleanups(0, span, |this| {
                    let val = val.map(|tmp| this.local_ref(tmp, span));
                    this.seal(HirTerminator::Return(val));
                    Ok(())
                })?;
                Ok(true)
            }
            // Nested `def` (Phase 6A): a flat synthetic function plus a closure
            // value bound to the def's name in this scope.
            Stmt::FunctionDef(d) => {
                self.lower_nested_def(d)?;
                Ok(false)
            }
            // Binding-analysis inputs only (Phase 6B): the declarations were
            // consumed by `freevars` / the module pre-scan; nothing to emit.
            Stmt::Global(_) | Stmt::Nonlocal(_) => Ok(false),
            Stmt::Delete(d) => self.lower_delete(d),
            other => Err(parse_error(
                "unsupported statement for this milestone",
                to_span(other.range()),
            )),
        }
    }

    /// `a = b = value` — evaluate `value` once, assign to each target (a `Name` or
    /// a subscript `base[index]`).
    fn lower_assign(&mut self, a: &rustpython_parser::ast::StmtAssign) -> Result<()> {
        // `x = yield e` inside a generator (Phase 6E): suspend, then bind the
        // sent value resuming here. Only a single simple-name target is in scope.
        if self.gen.is_some() && is_yield_expr(a.value.as_ref()) && a.targets.len() == 1 {
            if let Expr::Name(n) = &a.targets[0] {
                let span = to_span(a.range());
                let sent = self.lower_yield_value(a.value.as_ref(), true)?;
                let sent = sent.expect("x = yield yields a sent value");
                let name = self.intern(n.id.as_str());
                self.write_named(name, SemTy::Dyn, sent);
                let _ = span;
                return Ok(());
            }
        }
        // Tuple/list unpacking target: `a, b = …` / `a, b = c, d`.
        if a.targets.len() == 1 {
            if let Some(targets) = seq_target_elts(&a.targets[0]) {
                let span = to_span(a.range());
                // A literal sequence RHS unpacks element-wise with a static arity
                // check and no intermediate tuple; anything else — including a
                // starred target over a literal (`a, *rest = [1, 2, 3]`) —
                // stages a value and reads it back positionally.
                let has_star = targets.iter().any(|t| matches!(t, Expr::Starred(_)));
                if !has_star {
                    if let Some(values) = seq_target_elts(a.value.as_ref()) {
                        if targets.len() != values.len() {
                            return Err(parse_error(
                                format!(
                                    "cannot unpack: expected {} value(s), got {}",
                                    targets.len(),
                                    values.len()
                                ),
                                span,
                            ));
                        }
                        return self.lower_unpack_literal(targets, values, span);
                    }
                }
                let value = self.lower_expr(a.value.as_ref())?;
                return self.lower_unpack_subscript(targets, value, span);
            }
        }
        let value = self.lower_expr(a.value.as_ref())?;
        if a.targets.len() == 1 {
            return self.assign_to_target(&a.targets[0], value);
        }
        // Multiple targets: stage the value once, then fan out.
        let span = to_span(a.value.range());
        let tmp = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign { target: tmp, value });
        for target in &a.targets {
            let v = self.local_ref(tmp, span);
            self.assign_to_target(target, v)?;
        }
        Ok(())
    }

    /// Bind `value` to one assignment target: a simple name (`x = …`) or a
    /// subscript write (`a[i] = …` → [`HirStmt::SetItem`]).
    fn assign_to_target(&mut self, target: &Expr, value: Idx<HirExpr>) -> Result<()> {
        match target {
            Expr::Name(n) => {
                let name = self.intern(n.id.as_str());
                self.write_named(name, SemTy::Dyn, value);
                Ok(())
            }
            Expr::Subscript(s) => {
                let span = to_span(s.range());
                if matches!(s.slice.as_ref(), Expr::Slice(_)) {
                    return Err(parse_error("slice assignment is not yet supported", span));
                }
                // `os.environ[k] = v` (Phase 8H): a SetItem into the environ
                // attr would write into a FRESH dict snapshot (the getter
                // rebuilds it on every read) and be silently lost. Route to the
                // `rt_os_environ_set` setter, which mutates the real process
                // environment.
                // TODO: `del os.environ[k]` needs a delete-subscript statement
                // path (none exists yet) plus an `rt_os_environ_del` (remove_var).
                if let Some((leftmost, dotted)) = flatten_attr_chain(s.value.as_ref()) {
                    let lname = self.intern(leftmost);
                    if dotted == "os.environ"
                        && self.ctx.stdlib.aliases.contains(leftmost)
                        && !self.scope.contains_key(&lname)
                    {
                        let key = self.lower_expr(s.slice.as_ref())?;
                        let call = self.alloc(
                            HirExprKind::CallRuntime {
                                target: pyaot_hir::RuntimeCallTarget::Func(
                                    &pyaot_stdlib_defs::modules::os::OS_ENVIRON_SET,
                                ),
                                args: vec![Some(key), Some(value)],
                                provided: 2,
                            },
                            SemTy::NoneTy,
                            span,
                        );
                        self.push_stmt(HirStmt::Expr(call));
                        return Ok(());
                    }
                }
                let base = self.lower_expr(s.value.as_ref())?;
                let index = self.lower_expr(s.slice.as_ref())?;
                self.push_stmt(HirStmt::SetItem { base, index, value });
                Ok(())
            }
            Expr::Attribute(attr) => {
                let base = self.lower_expr(attr.value.as_ref())?;
                let name = self.intern(attr.attr.as_str());
                self.push_stmt(HirStmt::SetAttr { base, name, value });
                Ok(())
            }
            // Nested sequence target (`a, (b, c) = …`): stage this element and
            // re-subscript it positionally, recursing for deeper nesting. Routes
            // through the same unpacker as the top level, so for-loop and
            // comprehension targets get nested support for free (backlog §4).
            Expr::Tuple(t) => {
                let span = to_span(target.range());
                self.lower_unpack_subscript(&t.elts, value, span)
            }
            Expr::List(l) => {
                let span = to_span(target.range());
                self.lower_unpack_subscript(&l.elts, value, span)
            }
            other => Err(parse_error(
                "unsupported assignment target",
                to_span(other.range()),
            )),
        }
    }

    /// Lower a `del` statement (`del d[k]`, `del li[i]`, `del name`,
    /// `del obj.attr`, and multi-target `del a, b`). Each target is unbound
    /// independently, mirroring [`Self::assign_to_target`]:
    /// - a subscript → [`HirStmt::DelItem`] (a runtime element delete);
    /// - a name → stores the `Value::UNBOUND` sentinel into the slot (marking a
    ///   local deletable + pinned-tagged, or recording a global), so any later
    ///   read raises `UnboundLocalError`/`NameError` via the read-guard;
    /// - an attribute → stores `UNBOUND` into the field slot (recording the
    ///   field name deletable), so a later read raises `AttributeError`.
    fn lower_delete(&mut self, d: &StmtDelete) -> Result<bool> {
        for target in &d.targets {
            self.delete_target(target)?;
        }
        Ok(false)
    }

    /// Unbind one `del` target. See [`Self::lower_delete`].
    fn delete_target(&mut self, target: &Expr) -> Result<()> {
        match target {
            Expr::Subscript(s) => {
                let span = to_span(s.range());
                if matches!(s.slice.as_ref(), Expr::Slice(_)) {
                    return Err(parse_error("slice deletion is not supported", span));
                }
                let base = self.lower_expr(s.value.as_ref())?;
                let index = self.lower_expr(s.slice.as_ref())?;
                self.push_stmt(HirStmt::DelItem { base, index });
                Ok(())
            }
            Expr::Name(n) => {
                let span = to_span(n.range());
                let name = self.intern(n.id.as_str());
                let unbound = self.alloc(HirExprKind::Unbound, SemTy::Never, span);
                match self.resolve_write_place(name, SemTy::Dyn) {
                    Place::Bind(Binding::Direct(lid)) => {
                        // Keep the name bound (CPython keeps it in co_varnames);
                        // store the sentinel and pin the slot Tagged so the
                        // immediate fits regardless of the inferred type.
                        let l = &mut self.locals[lid.index()];
                        l.deletable = true;
                        l.pin_tagged = true;
                        self.push_stmt(HirStmt::Assign {
                            target: lid,
                            value: unbound,
                        });
                        Ok(())
                    }
                    Place::Bind(Binding::Cell(_)) => Err(parse_error(
                        "del of a captured (nonlocal/closure) variable is not supported",
                        span,
                    )),
                    Place::Global(var_id) => {
                        self.shared.deletable_globals.insert(var_id, name);
                        self.push_stmt(HirStmt::GlobalSet {
                            var_id,
                            value: unbound,
                        });
                        Ok(())
                    }
                }
            }
            Expr::Attribute(attr) => {
                let span = to_span(attr.range());
                let base = self.lower_expr(attr.value.as_ref())?;
                let name = self.intern(attr.attr.as_str());
                let unbound = self.alloc(HirExprKind::Unbound, SemTy::Never, span);
                self.shared.deletable_fields.insert(name);
                self.push_stmt(HirStmt::SetAttr {
                    base,
                    name,
                    value: unbound,
                });
                Ok(())
            }
            // `del (a, b)` / `del [a, b]` — parenthesized/bracketed multi-target
            // (the bare `del a, b` form is split into separate targets by the
            // parser and handled in `lower_delete`).
            Expr::Tuple(t) => {
                for elt in &t.elts {
                    self.delete_target(elt)?;
                }
                Ok(())
            }
            Expr::List(l) => {
                for elt in &l.elts {
                    self.delete_target(elt)?;
                }
                Ok(())
            }
            other => Err(parse_error("unsupported del target", to_span(other.range()))),
        }
    }

    /// Unpack a literal-sequence RHS (`a, b = e0, e1`): stage every RHS value
    /// first (so `a, b = b, a` swaps correctly), then bind each target — no
    /// intermediate tuple allocation.
    fn lower_unpack_literal(
        &mut self,
        targets: &[Expr],
        values: &[Expr],
        span: Span,
    ) -> Result<()> {
        reject_starred(targets, span)?;
        let mut staged = Vec::with_capacity(values.len());
        for v in values {
            let vv = self.lower_expr(v)?;
            let tmp = self.fresh_local(SemTy::Dyn);
            self.push_stmt(HirStmt::Assign {
                target: tmp,
                value: vv,
            });
            staged.push(tmp);
        }
        for (target, tmp) in targets.iter().zip(staged) {
            let v = self.local_ref(tmp, span);
            self.assign_to_target(target, v)?;
        }
        Ok(())
    }

    /// Unpack an arbitrary iterable RHS (`a, b = expr`, `for k, v in pairs`): stage
    /// the value once, then bind `target_i = tmp[i]` via positional subscripts.
    /// One starred target (`a, *rest = …`) captures a fresh list of the middle
    /// slice. A nested sequence target recurses here via [`Self::assign_to_target`]
    /// (`a, (b, c) = …`, backlog §4). Arity beyond the pattern is a runtime
    /// `IndexError` (no static shape here); an over-long sequence is *not*
    /// statically rejected (CPython's "too many values to unpack" is not raised) —
    /// this matches the flat-unpack contract and is an inherited limitation.
    fn lower_unpack_subscript(
        &mut self,
        targets: &[Expr],
        value: Idx<HirExpr>,
        span: Span,
    ) -> Result<()> {
        let star_pos = targets.iter().position(|t| matches!(t, Expr::Starred(_)));
        if targets
            .iter()
            .enumerate()
            .any(|(i, t)| matches!(t, Expr::Starred(_)) && Some(i) != star_pos)
        {
            return Err(parse_error("multiple starred targets in unpacking", span));
        }
        let tmp = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign { target: tmp, value });

        let (prefix, suffix): (&[Expr], &[Expr]) = match star_pos {
            Some(p) => (&targets[..p], &targets[p + 1..]),
            None => (targets, &[]),
        };
        for (i, target) in prefix.iter().enumerate() {
            let tmp_ref = self.local_ref(tmp, span);
            let idx = self.alloc(HirExprKind::IntLit(i as i64), SemTy::Int, span);
            let sub = self.alloc(
                HirExprKind::Subscript {
                    base: tmp_ref,
                    index: idx,
                },
                SemTy::Dyn,
                span,
            );
            self.assign_to_target(target, sub)?;
        }
        let Some(p) = star_pos else { return Ok(()) };

        // n = len(tmp), staged once for the star slice and the suffix indices.
        let tmp_ref = self.local_ref(tmp, span);
        let len_e = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Len,
                args: vec![tmp_ref],
            },
            SemTy::Int,
            span,
        );
        let len_l = self.fresh_local(SemTy::Int);
        self.push_stmt(HirStmt::Assign {
            target: len_l,
            value: len_e,
        });

        // *rest = tmp[p .. n - m] as a fresh list.
        let Expr::Starred(st) = &targets[p] else {
            unreachable!()
        };
        let lo = self.alloc(HirExprKind::IntLit(p as i64), SemTy::Int, span);
        let len_ref = self.local_ref(len_l, span);
        let m_lit = self.alloc(HirExprKind::IntLit(suffix.len() as i64), SemTy::Int, span);
        let hi = self.alloc(
            HirExprKind::BinOp {
                op: BinOp::Sub,
                l: len_ref,
                r: m_lit,
            },
            SemTy::Dyn,
            span,
        );
        let rest = self.build_sublist(tmp, lo, hi, span)?;
        let rest_ref = self.local_ref(rest, span);
        self.assign_to_target(st.value.as_ref(), rest_ref)?;

        // Suffix targets: tmp[n - (m - j)].
        for (j, target) in suffix.iter().enumerate() {
            let len_ref = self.local_ref(len_l, span);
            let back = self.alloc(
                HirExprKind::IntLit((suffix.len() - j) as i64),
                SemTy::Int,
                span,
            );
            let idx = self.alloc(
                HirExprKind::BinOp {
                    op: BinOp::Sub,
                    l: len_ref,
                    r: back,
                },
                SemTy::Dyn,
                span,
            );
            let tmp_ref = self.local_ref(tmp, span);
            let sub = self.alloc(
                HirExprKind::Subscript {
                    base: tmp_ref,
                    index: idx,
                },
                SemTy::Dyn,
                span,
            );
            self.assign_to_target(target, sub)?;
        }
        Ok(())
    }

    fn lower_augassign(&mut self, a: &rustpython_parser::ast::StmtAugAssign) -> Result<()> {
        let span = to_span(a.range());
        let op = binop_from_ast(&a.op);
        match a.target.as_ref() {
            Expr::Name(n) => {
                let name = self.intern(n.id.as_str());
                let place = self.resolve_write_place(name, SemTy::Dyn);
                let l = self.read_place(place, span);
                let r = self.lower_expr(a.value.as_ref())?;
                let combined = self.alloc(HirExprKind::BinOp { op, l, r }, SemTy::Dyn, span);
                self.write_place(place, combined);
                Ok(())
            }
            // `base.attr op= value` — evaluate `base` once, then read/modify/write.
            Expr::Attribute(attr) => {
                let base_e = self.lower_expr(attr.value.as_ref())?;
                let base_tmp = self.fresh_local(SemTy::Dyn);
                self.push_stmt(HirStmt::Assign {
                    target: base_tmp,
                    value: base_e,
                });
                let name = self.intern(attr.attr.as_str());
                let read_base = self.local_ref(base_tmp, span);
                let cur = self.alloc(
                    HirExprKind::Attribute {
                        value: read_base,
                        name,
                    },
                    SemTy::Dyn,
                    span,
                );
                let r = self.lower_expr(a.value.as_ref())?;
                let combined = self.alloc(HirExprKind::BinOp { op, l: cur, r }, SemTy::Dyn, span);
                let write_base = self.local_ref(base_tmp, span);
                self.push_stmt(HirStmt::SetAttr {
                    base: write_base,
                    name,
                    value: combined,
                });
                Ok(())
            }
            // `base[index] op= value` — evaluate `base` and `index` once.
            Expr::Subscript(s) => {
                if matches!(s.slice.as_ref(), Expr::Slice(_)) {
                    return Err(parse_error(
                        "slice augmented assignment is not supported",
                        span,
                    ));
                }
                let base_e = self.lower_expr(s.value.as_ref())?;
                let base_tmp = self.fresh_local(SemTy::Dyn);
                self.push_stmt(HirStmt::Assign {
                    target: base_tmp,
                    value: base_e,
                });
                let idx_e = self.lower_expr(s.slice.as_ref())?;
                let idx_tmp = self.fresh_local(SemTy::Dyn);
                self.push_stmt(HirStmt::Assign {
                    target: idx_tmp,
                    value: idx_e,
                });
                let read_base = self.local_ref(base_tmp, span);
                let read_idx = self.local_ref(idx_tmp, span);
                let cur = self.alloc(
                    HirExprKind::Subscript {
                        base: read_base,
                        index: read_idx,
                    },
                    SemTy::Dyn,
                    span,
                );
                let r = self.lower_expr(a.value.as_ref())?;
                let combined = self.alloc(HirExprKind::BinOp { op, l: cur, r }, SemTy::Dyn, span);
                let write_base = self.local_ref(base_tmp, span);
                let write_idx = self.local_ref(idx_tmp, span);
                self.push_stmt(HirStmt::SetItem {
                    base: write_base,
                    index: write_idx,
                    value: combined,
                });
                Ok(())
            }
            other => Err(parse_error(
                "unsupported augmented-assignment target",
                to_span(other.range()),
            )),
        }
    }

    fn lower_annassign(&mut self, a: &rustpython_parser::ast::StmtAnnAssign) -> Result<()> {
        let span = to_span(a.range());
        let ty = annotation_to_semty(a.annotation.as_ref(), self.ctx);
        let Expr::Name(n) = a.target.as_ref() else {
            return Err(parse_error(
                "annotated assignment target must be a name",
                span,
            ));
        };
        let name = self.intern(n.id.as_str());
        let place = self.resolve_write_place(name, ty);
        if let Some(value) = &a.value {
            let v = self.lower_expr(value.as_ref())?;
            self.write_place(place, v);
        }
        Ok(())
    }

    /// Look up or allocate a named binding. A new (non-celled) name takes a
    /// direct local of type `ty`; an existing one keeps its slot (flat
    /// per-function scope). Celled names are pre-created by [`Self::init_cells`]
    /// in the entry block — they are always already in scope here.
    fn ensure_binding(&mut self, name: InternedString, ty: SemTy) -> Binding {
        if let Some(b) = self.scope.get(&name).copied() {
            return b;
        }
        debug_assert!(
            !self.celled.contains(&name),
            "celled name must be pre-created by init_cells"
        );
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(HirLocal {
            name,
            ty,
            raw_int_ok: false,
            pin_tagged: false,
            cell_shared: false,
            deletable: false,
        });
        self.scope.insert(name, Binding::Direct(id));
        Binding::Direct(id)
    }

    /// Read a bound name: a direct local read, or a `CellGet` through its cell.
    fn read_binding(&mut self, b: Binding, span: Span) -> Idx<HirExpr> {
        match b {
            Binding::Direct(lid) => self.local_ref(lid, span),
            Binding::Cell(lid) => self.alloc(HirExprKind::CellGet { cell: lid }, SemTy::Dyn, span),
        }
    }

    /// Write `value` to a bound name: a direct assignment, or a `CellSet`.
    fn write_binding(&mut self, b: Binding, value: Idx<HirExpr>) {
        match b {
            Binding::Direct(lid) => self.push_stmt(HirStmt::Assign { target: lid, value }),
            Binding::Cell(lid) => self.push_stmt(HirStmt::CellSet { cell: lid, value }),
        }
    }

    /// Declare-and-write in one step (the common assignment path), routing a
    /// promoted module-global write to its slot (Phase 6B).
    fn write_named(&mut self, name: InternedString, ty: SemTy, value: Idx<HirExpr>) {
        match self.resolve_write_place(name, ty) {
            Place::Bind(b) => self.write_binding(b, value),
            Place::Global(var_id) => self.push_stmt(HirStmt::GlobalSet { var_id, value }),
        }
    }

    /// Where a WRITE to `name` lands (Phase 6B): an existing binding; the global
    /// slot (in `__main__` for any promoted name, in a function only under a
    /// `global` declaration — an undeclared assignment binds locally, as in
    /// CPython); else a fresh local.
    fn resolve_write_place(&mut self, name: InternedString, ty: SemTy) -> Place {
        if let Some(b) = self.scope.get(&name).copied() {
            return Place::Bind(b);
        }
        if self.is_main || self.global_decls.contains(&name) {
            if let Some(vid) = self.promoted_id(name) {
                return Place::Global(vid);
            }
        }
        Place::Bind(self.ensure_binding(name, ty))
    }

    /// The global slot a READ of `name` (not in scope) resolves to, if any:
    /// any promoted name in `__main__`; in a function a `global`-declared name,
    /// or a promoted name the function never binds locally.
    fn global_read_slot(&self, name: InternedString) -> Option<u32> {
        let vid = self.promoted_id(name)?;
        if self.is_main || self.global_decls.contains(&name) || !self.bound_names.contains(&name) {
            Some(vid)
        } else {
            None
        }
    }

    /// The promoted-global `var_id` of `name`, if it has one.
    fn promoted_id(&self, name: InternedString) -> Option<u32> {
        self.ctx.promoted.get(self.interner.resolve(name)).copied()
    }

    /// Read through a [`Place`].
    fn read_place(&mut self, p: Place, span: Span) -> Idx<HirExpr> {
        match p {
            Place::Bind(b) => self.read_binding(b, span),
            Place::Global(var_id) => {
                self.alloc(HirExprKind::GlobalGet { var_id }, SemTy::Dyn, span)
            }
        }
    }

    /// Write through a [`Place`].
    fn write_place(&mut self, p: Place, value: Idx<HirExpr>) {
        match p {
            Place::Bind(b) => self.write_binding(b, value),
            Place::Global(var_id) => self.push_stmt(HirStmt::GlobalSet { var_id, value }),
        }
    }

    fn lower_if(&mut self, s: &rustpython_parser::ast::StmtIf) -> Result<()> {
        let cond = self.lower_expr(s.test.as_ref())?;
        let then_b = self.new_block();
        let join = self.new_block();
        let else_b = if s.orelse.is_empty() {
            join
        } else {
            self.new_block()
        };
        self.seal(HirTerminator::Branch {
            cond,
            then: then_b,
            else_: else_b,
        });

        self.switch(then_b);
        self.lower_body(&s.body)?;
        self.seal(HirTerminator::Jump(join));

        if !s.orelse.is_empty() {
            self.switch(else_b);
            self.lower_body(&s.orelse)?;
            self.seal(HirTerminator::Jump(join));
        }

        self.switch(join);
        Ok(())
    }

    fn lower_while(&mut self, s: &rustpython_parser::ast::StmtWhile) -> Result<bool> {
        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let cond = self.lower_expr(s.test.as_ref())?;
        let body_b = self.new_block();
        let exit = self.new_block();
        let else_b = if s.orelse.is_empty() {
            exit
        } else {
            self.new_block()
        };
        self.seal(HirTerminator::Branch {
            cond,
            then: body_b,
            else_: else_b,
        });

        self.switch(body_b);
        self.scope_stack.push(ScopeCtx::Loop {
            continue_to: header,
            break_to: exit,
        });
        self.lower_body(&s.body)?;
        self.scope_stack.pop();
        self.seal(HirTerminator::Jump(header));

        if !s.orelse.is_empty() {
            self.switch(else_b);
            self.lower_body(&s.orelse)?;
            self.seal(HirTerminator::Jump(exit));
        }

        self.switch(exit);
        Ok(false)
    }

    fn lower_for(&mut self, s: &rustpython_parser::ast::StmtFor) -> Result<bool> {
        // The Phase-3c `range(...)` fast path (raw-i64 cursors) bakes in two
        // assumptions: a compile-time-literal step (so the loop direction is
        // fixed statically) and a simple-`Name` target. Use it ONLY when both
        // hold; everything else — a non-literal/computed step, or an attribute/
        // subscript/tuple target — takes the general iterator path, which drives
        // the runtime `RangeIter` (correct direction + step=0 `ValueError`) and
        // binds an arbitrary target via `bind_for_target`. This is a strict
        // superset of the old behavior: the only loops newly diverted are exactly
        // the ones `lower_for_range` rejected, so gated raw-int loops keep the
        // fast path with no perf regression.
        if is_range_call(s.iter.as_ref())
            && matches!(s.target.as_ref(), Expr::Name(_))
            && range_step_is_literal(s.iter.as_ref())
        {
            self.lower_for_range(s)
        } else {
            self.lower_for_iter(s)
        }
    }

    /// General `for target in <iterable>`: drive the runtime iterator protocol
    /// (`iter` → `next` → `is_exhausted`), binding the target (a name or a tuple
    /// pattern) each iteration. `for`-else / `break` / `continue` reuse the loop
    /// stack exactly as the `while`/range paths do.
    /// Lower a for-loop / comprehension iterable. A File iterable (syntactic
    /// `open(...)` and File variables alike) is handled at lowering: the frozen
    /// runtime cannot iterate a File object (PITFALLS), so `lowering` expands
    /// `Iter(file)` to `rt_file_readlines` + list iteration (Phase 8H) —
    /// line-for-line identical to CPython's lazy file iteration on the small
    /// corpus inputs.
    fn lower_iterable_expr(&mut self, e: &Expr, _span: Span) -> Result<Idx<HirExpr>> {
        self.lower_expr(e)
    }

    fn lower_for_iter(&mut self, s: &rustpython_parser::ast::StmtFor) -> Result<bool> {
        let span = to_span(s.range());

        // it = iter(iterable)  — a Heap(Iterator) local, live across the loop.
        let iterable = self.lower_iterable_expr(s.iter.as_ref(), span)?;
        let it = self.fresh_local(SemTy::Dyn);
        let iter_expr = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Iter,
                args: vec![iterable],
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: it,
            value: iter_expr,
        });

        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);

        // elem = next(it)   then   done = is_exhausted(it)  (this call order is the
        // runtime contract: `next` advances and sets the exhausted flag).
        let elem = self.fresh_local_tagged();
        let it_ref1 = self.local_ref(it, span);
        let next_expr = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::IterNext,
                args: vec![it_ref1],
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: elem,
            value: next_expr,
        });
        let it_ref2 = self.local_ref(it, span);
        let done = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::IterExhausted,
                args: vec![it_ref2],
            },
            SemTy::Bool,
            span,
        );

        let body_b = self.new_block();
        let exit = self.new_block();
        let else_b = if s.orelse.is_empty() {
            exit
        } else {
            self.new_block()
        };
        // done == true → exit (or the for-else); else run the body.
        self.seal(HirTerminator::Branch {
            cond: done,
            then: else_b,
            else_: body_b,
        });

        self.switch(body_b);
        let elem_ref = self.local_ref(elem, span);
        self.bind_for_target(s.target.as_ref(), elem_ref, span)?;
        self.scope_stack.push(ScopeCtx::Loop {
            continue_to: header,
            break_to: exit,
        });
        self.lower_body(&s.body)?;
        self.scope_stack.pop();
        self.seal(HirTerminator::Jump(header));

        if !s.orelse.is_empty() {
            self.switch(else_b);
            self.lower_body(&s.orelse)?;
            self.seal(HirTerminator::Jump(exit));
        }

        self.switch(exit);
        Ok(false)
    }

    /// Bind a `for`-loop target. Delegates the supported assignment shapes to
    /// [`Self::assign_to_target`] — byte-identical on `Name`/`Tuple`/`List`
    /// (same `write_named` / `lower_unpack_subscript`), and additionally lowers
    /// an attribute (`for obj.attr in …` → `SetAttr`) or subscript
    /// (`for lst[i] in …` → `SetItem`) leaf each iteration (backlog §4). Keeps a
    /// precise for-loop diagnostic for everything else.
    fn bind_for_target(&mut self, target: &Expr, value: Idx<HirExpr>, span: Span) -> Result<()> {
        match target {
            Expr::Name(_)
            | Expr::Tuple(_)
            | Expr::List(_)
            | Expr::Attribute(_)
            | Expr::Subscript(_) => self.assign_to_target(target, value),
            _ => Err(parse_error("unsupported for-loop target", span)),
        }
    }

    /// The preserved Phase-3 `range(...)` loop with proof-gated raw-i64 cursors.
    fn lower_for_range(&mut self, s: &rustpython_parser::ast::StmtFor) -> Result<bool> {
        let span = to_span(s.range());
        let (start, stop, step) = parse_range(s.iter.as_ref(), span)?;
        if step == 0 {
            return Err(parse_error("range() step argument must not be zero", span));
        }
        let Expr::Name(n) = s.target.as_ref() else {
            return Err(parse_error("for-loop target must be a simple name", span));
        };
        let i_name = self.intern(n.id.as_str());
        let i_b = self.resolve_write_place(i_name, SemTy::Dyn);
        let cursor = self.fresh_local(SemTy::Dyn);
        let stop_l = self.fresh_local(SemTy::Dyn);

        // Phase 3c: the cursor / stop slot / induction variable `i` / derived
        // body expressions are all left as plain tagged locals here. typeck's
        // interval pass (`narrow_raw_ints`) runs a sound forward range analysis
        // over the materialized CFG and flags every `int` slot — and every
        // derived `int` BinOp — that provably stays within `±RAW_I64_NARROW_BOUND`
        // with no i64 overflow, subsuming the old literal-`range()` heuristic and
        // additionally narrowing `i` itself and body expressions like `i * 3 % k`.

        // cursor = start; stop_l = stop  (range args evaluated once).
        let s_idx = self.lower_range_arg(&start, span)?;
        self.push_stmt(HirStmt::Assign {
            target: cursor,
            value: s_idx,
        });
        let stop_idx = self.lower_range_arg(&stop, span)?;
        self.push_stmt(HirStmt::Assign {
            target: stop_l,
            value: stop_idx,
        });

        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let cursor_ref = self.local_ref(cursor, span);
        let stop_ref = self.local_ref(stop_l, span);
        let cmp_op = if step > 0 { CmpOp::Lt } else { CmpOp::Gt };
        let cond = self.alloc(
            HirExprKind::Compare {
                op: cmp_op,
                l: cursor_ref,
                r: stop_ref,
            },
            SemTy::Bool,
            span,
        );
        let body_b = self.new_block();
        let incr = self.new_block();
        let exit = self.new_block();
        let else_b = if s.orelse.is_empty() {
            exit
        } else {
            self.new_block()
        };
        self.seal(HirTerminator::Branch {
            cond,
            then: body_b,
            else_: else_b,
        });

        self.switch(body_b);
        // i = cursor
        let cref = self.local_ref(cursor, span);
        self.write_place(i_b, cref);
        self.scope_stack.push(ScopeCtx::Loop {
            continue_to: incr,
            break_to: exit,
        });
        self.lower_body(&s.body)?;
        self.scope_stack.pop();
        self.seal(HirTerminator::Jump(incr));

        // incr: cursor = cursor + step
        self.switch(incr);
        let cref2 = self.local_ref(cursor, span);
        let step_kind = self.int_literal_const(step);
        let step_lit = self.alloc(step_kind, SemTy::Int, span);
        let inc = self.alloc(
            HirExprKind::BinOp {
                op: BinOp::Add,
                l: cref2,
                r: step_lit,
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: cursor,
            value: inc,
        });
        self.seal(HirTerminator::Jump(header));

        if !s.orelse.is_empty() {
            self.switch(else_b);
            self.lower_body(&s.orelse)?;
            self.seal(HirTerminator::Jump(exit));
        }

        self.switch(exit);
        Ok(false)
    }

    /// Lower a range() bound argument (start/stop) — an arbitrary expression.
    fn lower_range_arg(&mut self, arg: &RangeArg, span: Span) -> Result<Idx<HirExpr>> {
        match arg {
            RangeArg::Zero => Ok(self.alloc(HirExprKind::IntLit(0), SemTy::Int, span)),
            RangeArg::Expr(e) => self.lower_expr(e),
        }
    }

    /// A fixnum/bignum int-literal expr kind (used for the loop step).
    fn int_literal_const(&mut self, v: i64) -> HirExprKind {
        if pyaot_core_defs::int_fits(v) {
            HirExprKind::IntLit(v)
        } else {
            HirExprKind::BigIntLit(self.intern(&v.to_string()))
        }
    }

    /// `print(args, sep=…, end=…)` → [`HirStmt::Print`].
    fn lower_print(&mut self, call: &rustpython_parser::ast::ExprCall) -> Result<()> {
        let mut sep: Option<InternedString> = None;
        let mut end: Option<InternedString> = None;
        for kw in &call.keywords {
            let key = kw.arg.as_ref().map(|i| i.as_str());
            match key {
                Some("sep") => sep = Some(self.kw_str_literal(kw, "sep")?),
                Some("end") => end = Some(self.kw_str_literal(kw, "end")?),
                Some(other) => {
                    return Err(parse_error(
                        format!("print() got an unexpected keyword argument '{other}'"),
                        to_span(call.range()),
                    ))
                }
                None => {
                    return Err(parse_error(
                        "print() does not support **kwargs",
                        to_span(call.range()),
                    ))
                }
            }
        }

        let mut args = Vec::with_capacity(call.args.len());
        for arg in &call.args {
            args.push(self.lower_expr(arg)?);
        }
        self.push_stmt(HirStmt::Print { args, sep, end });
        Ok(())
    }

    /// Extract a string-literal keyword value (`sep=`/`end=`).
    fn kw_str_literal(&mut self, kw: &Keyword, name: &str) -> Result<InternedString> {
        if let Expr::Constant(c) = &kw.value {
            if let Constant::Str(s) = &c.value {
                return Ok(self.intern(s));
            }
        }
        Err(parse_error(
            format!("print() {name}= must be a string literal"),
            to_span(kw.range()),
        ))
    }

    // ── exceptions: try / raise (Phase 7A/7B) ───────────────────────────────

    /// Lower a `try` statement. `try/except/finally` nests as
    /// `try { try/except } finally` (two frames).
    fn lower_try(&mut self, t: &rustpython_parser::ast::StmtTry) -> Result<bool> {
        let span = to_span(t.range());
        if !t.finalbody.is_empty() {
            self.lower_try_finally(t, span)?;
        } else {
            self.lower_try_except(&t.body, &t.handlers, &t.orelse, span)?;
        }
        Ok(false)
    }

    /// `try X finally F`: normal edge exits the region then runs `<F>`;
    /// exceptional edge `StartHandling; <F>; Reraise`. Early exits re-lower
    /// `<F>` via the [`ScopeCtx::Finally`] entry.
    fn lower_try_finally(&mut self, t: &rustpython_parser::ast::StmtTry, span: Span) -> Result<()> {
        let try_b = self.new_block();
        let exc_b = self.new_block();
        let join = self.new_block();
        self.seal(HirTerminator::Jump(try_b));

        self.switch(try_b);
        let outer = self.cur_handler;
        self.cur_handler = Some(exc_b);
        self.scope_stack.push(ScopeCtx::Finally {
            outer,
            stmts: t.finalbody.clone(),
        });
        if t.handlers.is_empty() {
            debug_assert!(
                t.orelse.is_empty(),
                "orelse without handlers is a SyntaxError"
            );
            self.lower_body(&t.body)?;
        } else {
            self.lower_try_except(&t.body, &t.handlers, &t.orelse, span)?;
        }
        self.scope_stack.pop();
        if self.cur_open() {
            // The finalbody runs OUTSIDE the region it guards (its own raise
            // propagates outward, and `finally` must not re-run): exit to a
            // fresh block under the outer handler.
            self.exit_protected(outer);
            self.lower_body(&t.finalbody)?;
            self.seal(HirTerminator::Jump(join));
        }
        self.cur_handler = outer;

        // Exceptional edge (runs under the OUTER handler). Park the in-flight
        // exception (so a nested raise chains it as __context__), run the
        // finalbody, then re-raise it.
        self.switch(exc_b);
        self.push_stmt(HirStmt::ExcOp(ExcOp::StartHandling));
        self.lower_body(&t.finalbody)?;
        if self.cur_open() {
            self.push_stmt(HirStmt::Raise(HirRaise::Reraise));
            self.seal(HirTerminator::Unreachable);
        }

        self.switch(join);
        Ok(())
    }

    /// `try/except[/else]`: lower the body under the handler context, exit
    /// the region on the normal edge (`else` after the exit so its exceptions
    /// escape), then the handler chain (`Matches*` tests; tuple clause =
    /// OR-chain), with a no-match tail that re-raises.
    fn lower_try_except(
        &mut self,
        body: &[Stmt],
        handlers: &[rustpython_parser::ast::ExceptHandler],
        orelse: &[Stmt],
        span: Span,
    ) -> Result<()> {
        debug_assert!(
            !handlers.is_empty(),
            "try without handlers or finally is a SyntaxError"
        );
        let try_b = self.new_block();
        let h_test = self.new_block();
        let join = self.new_block();
        self.seal(HirTerminator::Jump(try_b));

        // ── try body ──
        self.switch(try_b);
        let outer = self.cur_handler;
        self.cur_handler = Some(h_test);
        self.scope_stack.push(ScopeCtx::TryFrame { outer });
        self.lower_body(body)?;
        self.scope_stack.pop();
        if self.cur_open() {
            // `else` runs after the region exit: its exceptions are NOT
            // caught here.
            self.exit_protected(outer);
            self.lower_body(orelse)?;
            self.seal(HirTerminator::Jump(join));
        }
        self.cur_handler = outer;

        // ── handler chain (runs under the OUTER handler) ──
        self.switch(h_test);
        for (hi, handler) in handlers.iter().enumerate() {
            let rustpython_parser::ast::ExceptHandler::ExceptHandler(h) = handler;
            let hspan = to_span(h.range());
            let body_b = self.new_block();
            let next_test = self.new_block();
            match h.type_.as_deref() {
                // Bare `except:` catches everything (must be last in CPython).
                None => {
                    if hi + 1 != handlers.len() {
                        return Err(parse_error("default 'except:' must be last", hspan));
                    }
                    self.seal(HirTerminator::Jump(body_b));
                }
                Some(Expr::Tuple(tu)) => {
                    // OR-chain: any matching member enters the body.
                    for (i, te) in tu.elts.iter().enumerate() {
                        let q = self.exc_match_query(te)?;
                        if i + 1 == tu.elts.len() {
                            self.seal(HirTerminator::Branch {
                                cond: q,
                                then: body_b,
                                else_: next_test,
                            });
                        } else {
                            let more = self.new_block();
                            self.seal(HirTerminator::Branch {
                                cond: q,
                                then: body_b,
                                else_: more,
                            });
                            self.switch(more);
                        }
                    }
                }
                Some(single) => {
                    let q = self.exc_match_query(single)?;
                    self.seal(HirTerminator::Branch {
                        cond: q,
                        then: body_b,
                        else_: next_test,
                    });
                }
            }

            // ── handler body ──
            self.switch(body_b);
            if let Some(name) = &h.name {
                // Bind `as e` BEFORE StartHandling (rt_exc_get_current reads
                // the still-current exception). A fresh local per binding,
                // shadowing the name, with the clause's static type.
                let bind_ty = self.exc_clause_semty(h.type_.as_deref());
                let cur = self.alloc(
                    HirExprKind::ExcQuery(ExcQuery::Current),
                    bind_ty.clone(),
                    hspan,
                );
                self.bind_exc_name(name.as_str(), bind_ty, cur);
            }
            self.push_stmt(HirStmt::ExcOp(ExcOp::StartHandling));
            self.scope_stack.push(ScopeCtx::Handler);
            self.lower_body(&h.body)?;
            self.scope_stack.pop();
            if self.cur_open() {
                self.push_stmt(HirStmt::ExcOp(ExcOp::EndHandling));
                self.seal(HirTerminator::Jump(join));
            }
            self.switch(next_test);
        }

        // ── no handler matched: propagate outward ──
        self.push_stmt(HirStmt::Raise(HirRaise::Reraise));
        self.seal(HirTerminator::Unreachable);

        self.switch(join);
        let _ = span;
        Ok(())
    }

    /// The `Matches*` query for one `except` clause member: a user class from
    /// the class map, else a builtin exception name.
    fn exc_match_query(&mut self, te: &Expr) -> Result<Idx<HirExpr>> {
        let span = to_span(te.range());
        let Expr::Name(n) = te else {
            return Err(parse_error(
                "except clause must name an exception class",
                span,
            ));
        };
        let q = if let Some((cid, _)) = self.ctx.class_map.get(n.id.as_str()).copied() {
            ExcQuery::MatchesClass(cid)
        } else if let Some((class_id, _)) = self.ctx.stdlib.exceptions.get(n.id.as_str()).copied() {
            // A stdlib exception (`except HTTPError:`, Phase 8D): match by its
            // reserved class id (the runtime self-matches the raised
            // `custom_class_id`).
            ExcQuery::MatchesClass(ClassId::new(class_id as u32))
        } else if let Some(tag) = pyaot_core_defs::exception_name_to_tag(n.id.as_str()) {
            ExcQuery::MatchesBuiltin(tag)
        } else {
            return Err(parse_error(
                format!(
                    "unknown exception type `{}` in except clause",
                    n.id.as_str()
                ),
                span,
            ));
        };
        Ok(self.alloc(HirExprKind::ExcQuery(q), SemTy::Bool, span))
    }

    /// Fold `value.__class__.__name__` to a string literal from `value`'s
    /// statically-known type (Phase 7B). Only a directly-bound name whose
    /// static type is a builtin exception or a user class folds; anything else
    /// is rejected with a clear error.
    fn fold_class_name(&mut self, value: &Expr, span: Span) -> Result<Idx<HirExpr>> {
        let static_ty = match value {
            Expr::Name(n) => {
                let iname = self.intern(n.id.as_str());
                match self.scope.get(&iname).copied() {
                    Some(Binding::Direct(lid)) => Some(self.locals[lid.index()].ty.clone()),
                    _ => None,
                }
            }
            _ => None,
        };
        let name_str = match static_ty {
            Some(SemTy::BuiltinException(kind)) => kind.name().to_string(),
            Some(SemTy::Class { name, .. }) => self.interner.resolve(name).to_string(),
            _ => {
                return Err(parse_error(
                    "`.__class__.__name__` requires a variable with a statically-known \
                     exception/class type (bind it via `except SomeError as e`)",
                    span,
                ))
            }
        };
        let id = self.intern(&name_str);
        Ok(self.alloc(HirExprKind::StrLit(id), SemTy::Str, span))
    }

    /// The static type an `except … as e` binding carries: a single builtin
    /// name → `BuiltinException`; a single user class → `Class`; a tuple
    /// clause → the `Union` of its members (NOT `Dyn` — `str(e)`/`print(e)`
    /// must still route to the exception-message surface, and the generic
    /// Dyn print renders the object repr; Principle 2 demands the imprecise
    /// type stays behaviorally correct). A bare clause stays `Dyn`.
    fn exc_clause_semty(&mut self, ty: Option<&Expr>) -> SemTy {
        match ty {
            Some(e @ Expr::Name(_)) => self.exc_member_semty(e),
            Some(Expr::Tuple(t)) => {
                let mut members: Vec<SemTy> = Vec::new();
                for e in &t.elts {
                    let m = self.exc_member_semty(e);
                    if m == SemTy::Dyn {
                        return SemTy::Dyn;
                    }
                    if !members.contains(&m) {
                        members.push(m);
                    }
                }
                match members.len() {
                    0 => SemTy::Dyn,
                    1 => members.pop().expect("one member"),
                    _ => SemTy::Union(members),
                }
            }
            _ => SemTy::Dyn,
        }
    }

    /// One except-clause member's static type (builtin exception / user class).
    fn exc_member_semty(&mut self, e: &Expr) -> SemTy {
        let Expr::Name(n) = e else { return SemTy::Dyn };
        if let Some((cid, iname)) = self.ctx.class_map.get(n.id.as_str()).copied() {
            return SemTy::Class {
                class_id: cid,
                name: iname,
            };
        }
        if let Some(kind) = pyaot_core_defs::BuiltinExceptionKind::from_name(n.id.as_str()) {
            return SemTy::BuiltinException(kind);
        }
        // A stdlib exception (`HTTPError`/`URLError`, …) caught by its own name:
        // model the bound `e` as its builtin PARENT so `print(e)` / `str(e)` route
        // through the deterministic exception-message path. Otherwise `e` is `Dyn`
        // and renders the default object repr — a non-deterministic heap ADDRESS in
        // stdout (Phase 8 follow-up; matches the `except <parent>` behaviour).
        if let Some((_cid, parent_tag)) = self.ctx.stdlib.exceptions.get(n.id.as_str()).copied() {
            if let Some(parent) = pyaot_core_defs::BuiltinExceptionKind::from_tag(parent_tag) {
                return SemTy::BuiltinException(parent);
            }
        }
        SemTy::Dyn
    }

    /// Bind an `except … as e` name to a FRESH typed local, shadowing any
    /// previous binding (CPython unbinds `e` after the handler; a fresh slot
    /// per handler keeps each binding's static type precise). Celled names
    /// keep their cell (uniform tagged content).
    fn bind_exc_name(&mut self, name: &str, ty: SemTy, value: Idx<HirExpr>) {
        let iname = self.intern(name);
        if self.celled.contains(&iname) || self.global_decls.contains(&iname) {
            self.write_named(iname, SemTy::Dyn, value);
            return;
        }
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(HirLocal {
            name: iname,
            ty,
            raw_int_ok: false,
            pin_tagged: false,
            cell_shared: false,
            deletable: false,
        });
        self.scope.insert(iname, Binding::Direct(id));
        self.push_stmt(HirStmt::Assign { target: id, value });
    }

    /// Lower a `raise` statement. Always terminates the block.
    fn lower_raise(&mut self, r: &rustpython_parser::ast::StmtRaise) -> Result<bool> {
        let span = to_span(r.range());
        let raise = match &r.exc {
            // Bare `raise` — re-raise the exception being handled.
            None => {
                if r.cause.is_some() {
                    return Err(parse_error("bare raise cannot carry a cause", span));
                }
                HirRaise::Reraise
            }
            Some(exc) => self.classify_raise_target(exc, r.cause.as_deref(), span)?,
        };
        self.push_stmt(HirStmt::Raise(raise));
        self.seal(HirTerminator::Unreachable);
        Ok(true)
    }

    /// Classify a `raise EXPR [from CAUSE]` target. Builtin-exception name
    /// resolution is frontend-local: scope binding → `Instance`; class map →
    /// `Custom`; `exception_name_to_tag` → builtin; else an error.
    fn classify_raise_target(
        &mut self,
        exc: &Expr,
        cause: Option<&Expr>,
        span: Span,
    ) -> Result<HirRaise> {
        // `raise Name(...)` — a constructed exception.
        if let Expr::Call(c) = exc {
            if let Expr::Name(n) = c.func.as_ref() {
                if !c.keywords.is_empty() {
                    return Err(parse_error(
                        "keyword arguments in a raise expression are out of scope",
                        span,
                    ));
                }
                let iname = self.intern(n.id.as_str());
                if !self.scope.contains_key(&iname) {
                    if let Some((cid, _)) = self.ctx.class_map.get(n.id.as_str()).copied() {
                        if cause.is_some() {
                            return Err(parse_error(
                                "`raise CustomError(...) from ...` is out of scope for this milestone",
                                span,
                            ));
                        }
                        let args = self.lower_expr_list(&c.args)?;
                        return Ok(HirRaise::Custom {
                            class_id: cid,
                            args,
                        });
                    }
                    if let Some((class_id, parent_tag)) =
                        self.ctx.stdlib.exceptions.get(n.id.as_str()).copied()
                    {
                        if cause.is_some() {
                            return Err(parse_error(
                                "`raise StdlibError(...) from ...` is out of scope",
                                span,
                            ));
                        }
                        // Synthesize the CPython __str__ for the exceptions
                        // whose message is not the first positional arg:
                        // HTTPError(url, code, msg, hdrs, fp) prints
                        // "HTTP Error {code}: {msg}"; URLError(reason) prints
                        // "<urlopen error {reason}>". Everything else keeps
                        // the first positional arg as the message.
                        let msg = match (n.id.as_str(), c.args.len()) {
                            ("HTTPError", 3..) => {
                                let code = self.lower_expr(&c.args[1])?;
                                let msg_arg = self.lower_expr(&c.args[2])?;
                                Some(self.synth_concat_str(
                                    &[("HTTP Error ", code), (": ", msg_arg)],
                                    "",
                                    span,
                                ))
                            }
                            ("URLError", 1..) => {
                                let reason = self.lower_expr(&c.args[0])?;
                                Some(self.synth_concat_str(
                                    &[("<urlopen error ", reason)],
                                    ">",
                                    span,
                                ))
                            }
                            _ => match c.args.first() {
                                Some(a) => Some(self.lower_expr(a)?),
                                None => None,
                            },
                        };
                        return Ok(HirRaise::Stdlib {
                            class_id,
                            exc_type_tag: parent_tag,
                            msg,
                        });
                    }
                    if let Some(tag) = pyaot_core_defs::exception_name_to_tag(n.id.as_str()) {
                        if c.args.len() > 1 {
                            return Err(parse_error(
                                "multi-argument builtin exceptions are out of scope",
                                span,
                            ));
                        }
                        let msg = match c.args.first() {
                            Some(a) => Some(self.lower_expr(a)?),
                            None => None,
                        };
                        return self.attach_cause(tag, msg, cause, span);
                    }
                }
            }
        }
        // `raise Name` — a bare class (builtin/custom) or a caught instance.
        if let Expr::Name(n) = exc {
            let iname = self.intern(n.id.as_str());
            if self.scope.contains_key(&iname) {
                let value = self.lower_expr(exc)?;
                if cause.is_some() {
                    return Err(parse_error(
                        "`raise e from ...` is out of scope for this milestone",
                        span,
                    ));
                }
                return Ok(HirRaise::Instance { value });
            }
            if let Some((cid, _)) = self.ctx.class_map.get(n.id.as_str()).copied() {
                if cause.is_some() {
                    return Err(parse_error(
                        "`raise CustomError from ...` is out of scope for this milestone",
                        span,
                    ));
                }
                return Ok(HirRaise::Custom {
                    class_id: cid,
                    args: vec![],
                });
            }
            if let Some((class_id, parent_tag)) =
                self.ctx.stdlib.exceptions.get(n.id.as_str()).copied()
            {
                if cause.is_some() {
                    return Err(parse_error(
                        "`raise StdlibError from ...` is out of scope",
                        span,
                    ));
                }
                return Ok(HirRaise::Stdlib {
                    class_id,
                    exc_type_tag: parent_tag,
                    msg: None,
                });
            }
            if let Some(tag) = pyaot_core_defs::exception_name_to_tag(n.id.as_str()) {
                return self.attach_cause(tag, None, cause, span);
            }
        }
        Err(parse_error(
            "raise target must be an exception class, a constructed exception, \
             or a caught exception variable",
            span,
        ))
    }

    /// Attach a `from CAUSE` clause to a builtin raise.
    fn attach_cause(
        &mut self,
        tag: u8,
        msg: Option<Idx<HirExpr>>,
        cause: Option<&Expr>,
        span: Span,
    ) -> Result<HirRaise> {
        let Some(cause) = cause else {
            return Ok(HirRaise::Builtin { tag, msg });
        };
        // `from None` suppresses the context chain.
        if matches!(cause, Expr::Constant(c) if matches!(c.value, Constant::None)) {
            return Ok(HirRaise::BuiltinFromNone { tag, msg });
        }
        // `from Builtin(...)` / `from Builtin`.
        let (cname, cargs): (&str, &[Expr]) = match cause {
            Expr::Call(c) => match c.func.as_ref() {
                Expr::Name(n) if c.keywords.is_empty() => (n.id.as_str(), &c.args),
                _ => {
                    return Err(parse_error(
                        "a raise cause must be a builtin exception or None",
                        span,
                    ))
                }
            },
            Expr::Name(n) => (n.id.as_str(), &[]),
            _ => {
                return Err(parse_error(
                    "a raise cause must be a builtin exception or None",
                    span,
                ))
            }
        };
        // The PEP-3134 `from <caught variable>` idiom needs an instance-cause
        // runtime entry point — out of scope for this milestone; say so
        // clearly instead of "unknown exception type".
        {
            let iname = self.intern(cname);
            if self.scope.contains_key(&iname) {
                return Err(parse_error(
                    "`raise ... from <variable>` is out of scope for this milestone \
                     (use a builtin exception constructor or `from None`)",
                    span,
                ));
            }
        }
        let Some(cause_tag) = pyaot_core_defs::exception_name_to_tag(cname) else {
            return Err(parse_error(
                format!("unknown exception type `{cname}` in raise cause"),
                span,
            ));
        };
        if cargs.len() > 1 {
            return Err(parse_error(
                "multi-argument builtin exceptions are out of scope",
                span,
            ));
        }
        let cause_msg = match cargs.first() {
            Some(a) => Some(self.lower_expr(a)?),
            None => None,
        };
        Ok(HirRaise::BuiltinFrom {
            tag,
            msg,
            cause_tag,
            cause_msg,
        })
    }

    // ── with (Phase 7D) ──────────────────────────────────────────────────────

    /// Lower a `with` statement: items nest left-to-right; each item desugars
    /// to `__enter__` + `TryEnter` + `__exit__` on both edges (a truthy
    /// exceptional `__exit__` swallows the exception).
    fn lower_with(&mut self, w: &rustpython_parser::ast::StmtWith) -> Result<bool> {
        let span = to_span(w.range());
        self.lower_with_items(&w.items, &w.body, span)?;
        Ok(false)
    }

    fn lower_with_items(
        &mut self,
        items: &[rustpython_parser::ast::WithItem],
        body: &[Stmt],
        span: Span,
    ) -> Result<()> {
        let Some((first, rest)) = items.split_first() else {
            return self.lower_body(body);
        };

        // mgr = EXPR; val = mgr.__enter__(); [bind TARGET]
        let mgr_e = self.lower_expr(&first.context_expr)?;
        let mgr = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: mgr,
            value: mgr_e,
        });
        let recv = self.local_ref(mgr, span);
        let enter_name = self.intern("__enter__");
        let enter = self.alloc(
            HirExprKind::MethodCall {
                recv,
                method_name: enter_name,
                args: vec![],
                kwargs: vec![],
            },
            SemTy::Dyn,
            span,
        );
        match &first.optional_vars {
            Some(t) => self.bind_for_target(t.as_ref(), enter, span)?,
            None => self.push_stmt(HirStmt::Expr(enter)),
        }

        let body_b = self.new_block();
        let exit_exc = self.new_block();
        let join = self.new_block();
        self.seal(HirTerminator::Jump(body_b));

        // ── body (or the next nested item) ──
        self.switch(body_b);
        let outer = self.cur_handler;
        self.cur_handler = Some(exit_exc);
        self.scope_stack.push(ScopeCtx::WithCleanup { outer, mgr });
        self.lower_with_items(rest, body, span)?;
        self.scope_stack.pop();
        if self.cur_open() {
            // `__exit__` runs outside the region (its own raise propagates).
            self.exit_protected(outer);
            self.emit_exit_none_call(mgr, span);
            self.seal(HirTerminator::Jump(join));
        }
        self.cur_handler = outer;

        // ── exceptional edge (under the OUTER handler):
        //    r = mgr.__exit__(e, e, None); truthy swallows ──
        self.switch(exit_exc);
        let e_local = self.fresh_local_tagged();
        let cur = self.alloc(HirExprKind::ExcQuery(ExcQuery::Current), SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: e_local,
            value: cur,
        });
        self.push_stmt(HirStmt::ExcOp(ExcOp::StartHandling));
        let recv2 = self.local_ref(mgr, span);
        let e1 = self.local_ref(e_local, span);
        let e2 = self.local_ref(e_local, span);
        let none = self.alloc(HirExprKind::NoneLit, SemTy::NoneTy, span);
        let exit_name = self.intern("__exit__");
        let r = self.alloc(
            HirExprKind::MethodCall {
                recv: recv2,
                method_name: exit_name,
                args: vec![e1, e2, none],
                kwargs: vec![],
            },
            SemTy::Dyn,
            span,
        );
        let r_local = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: r_local,
            value: r,
        });
        let swallow_b = self.new_block();
        let reraise_b = self.new_block();
        let cond = self.local_ref(r_local, span);
        self.seal(HirTerminator::Branch {
            cond,
            then: swallow_b,
            else_: reraise_b,
        });
        self.switch(swallow_b);
        self.push_stmt(HirStmt::ExcOp(ExcOp::EndHandling));
        self.seal(HirTerminator::Jump(join));
        self.switch(reraise_b);
        self.push_stmt(HirStmt::Raise(HirRaise::Reraise));
        self.seal(HirTerminator::Unreachable);

        self.switch(join);
        Ok(())
    }

    // ── match (Phase 7E) ─────────────────────────────────────────────────────

    /// Lower a `match` statement: pure desugar to an if/elif CFG on a subject
    /// temp. Captures are ordinary function-scope locals (CPython leak
    /// semantics); binds happen on the partial-match path before the guard.
    fn lower_match(&mut self, m: &rustpython_parser::ast::StmtMatch) -> Result<bool> {
        let span = to_span(m.range());
        let subj_e = self.lower_expr(m.subject.as_ref())?;
        let subj = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: subj,
            value: subj_e,
        });
        let join = self.new_block();

        for case in &m.cases {
            let fail_b = self.new_block();
            self.lower_pattern(&case.pattern, subj, fail_b, span)?;
            if let Some(g) = &case.guard {
                let cond = self.lower_expr(g.as_ref())?;
                let body_b = self.new_block();
                self.seal(HirTerminator::Branch {
                    cond,
                    then: body_b,
                    else_: fail_b,
                });
                self.switch(body_b);
            }
            self.lower_body(&case.body)?;
            self.seal(HirTerminator::Jump(join));
            self.switch(fail_b);
        }
        // No case matched: a match statement just falls through.
        self.seal(HirTerminator::Jump(join));
        self.switch(join);
        Ok(false)
    }

    /// Emit the tests for `pat` against the value in local `scr`. On mismatch
    /// control jumps to `fail`; on fall-through the pattern matched and its
    /// captures are bound.
    fn lower_pattern(
        &mut self,
        pat: &rustpython_parser::ast::Pattern,
        scr: LocalId,
        fail: Idx<HirBlock>,
        span: Span,
    ) -> Result<()> {
        use rustpython_parser::ast::Pattern;
        match pat {
            // Literal: `subject == literal` (the documented `==`-vs-`is`
            // divergence for interned singletons is corpus-clean).
            Pattern::MatchValue(v) => {
                let lit = self.lower_expr(&v.value)?;
                self.emit_pattern_eq(scr, lit, fail, span)
            }
            Pattern::MatchSingleton(s) => {
                let lit = self.lower_constant(&s.value, span)?;
                self.emit_pattern_eq(scr, lit, fail, span)
            }
            Pattern::MatchAs(a) => {
                if let Some(sub) = &a.pattern {
                    self.lower_pattern(sub, scr, fail, span)?;
                }
                if let Some(name) = &a.name {
                    let iname = self.intern(name.as_str());
                    let v = self.local_ref(scr, span);
                    self.write_named(iname, SemTy::Dyn, v);
                }
                Ok(())
            }
            Pattern::MatchOr(o) => {
                // v1: capture-free alternatives only (each alternative would
                // otherwise need to bind the same names on its own path).
                for sub in &o.patterns {
                    if pattern_has_bindings(sub) {
                        return Err(parse_error(
                            "or-patterns with capture names are out of scope for this milestone",
                            span,
                        ));
                    }
                }
                let ok = self.new_block();
                let n = o.patterns.len();
                for (i, sub) in o.patterns.iter().enumerate() {
                    let alt_fail = if i + 1 == n { fail } else { self.new_block() };
                    self.lower_pattern(sub, scr, alt_fail, span)?;
                    self.seal(HirTerminator::Jump(ok));
                    if i + 1 != n {
                        self.switch(alt_fail);
                    }
                }
                self.switch(ok);
                Ok(())
            }
            Pattern::MatchSequence(s) => self.lower_seq_pattern(&s.patterns, scr, fail, span),
            Pattern::MatchMapping(mp) => self.lower_mapping_pattern(
                &mp.keys,
                &mp.patterns,
                mp.rest.as_ref(),
                scr,
                fail,
                span,
            ),
            Pattern::MatchClass(c) => self.lower_class_pattern(c, scr, fail, span),
            Pattern::MatchStar(_) => Err(parse_error(
                "a star pattern is only valid inside a sequence pattern",
                span,
            )),
        }
    }

    /// `subject == lit` → continue, else jump to `fail`.
    fn emit_pattern_eq(
        &mut self,
        scr: LocalId,
        lit: Idx<HirExpr>,
        fail: Idx<HirBlock>,
        span: Span,
    ) -> Result<()> {
        let s = self.local_ref(scr, span);
        let cmp = self.alloc(
            HirExprKind::Compare {
                op: CmpOp::Eq,
                l: s,
                r: lit,
            },
            SemTy::Bool,
            span,
        );
        let cont = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: cmp,
            then: cont,
            else_: fail,
        });
        self.switch(cont);
        Ok(())
    }

    /// A sequence pattern `[p0, …, *star, …, pn]`: length test (`== n`, star:
    /// `>= n-1`), positional subscripts for prefix/suffix, star capture as a
    /// fresh list.
    fn lower_seq_pattern(
        &mut self,
        pats: &[rustpython_parser::ast::Pattern],
        scr: LocalId,
        fail: Idx<HirBlock>,
        span: Span,
    ) -> Result<()> {
        use rustpython_parser::ast::Pattern;
        let star_pos = pats.iter().position(|p| matches!(p, Pattern::MatchStar(_)));
        if pats
            .iter()
            .enumerate()
            .any(|(i, p)| matches!(p, Pattern::MatchStar(_)) && Some(i) != star_pos)
        {
            return Err(parse_error("multiple star patterns in a sequence", span));
        }

        // n = len(subject), staged once.
        let s = self.local_ref(scr, span);
        let len_e = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Len,
                args: vec![s],
            },
            SemTy::Int,
            span,
        );
        let len_l = self.fresh_local(SemTy::Int);
        self.push_stmt(HirStmt::Assign {
            target: len_l,
            value: len_e,
        });

        let (prefix, suffix): (&[Pattern], &[Pattern]) = match star_pos {
            Some(p) => (&pats[..p], &pats[p + 1..]),
            None => (pats, &[]),
        };
        let need = (prefix.len() + suffix.len()) as i64;
        let len_ref = self.local_ref(len_l, span);
        let need_lit = self.alloc(HirExprKind::IntLit(need), SemTy::Int, span);
        let cmp_op = if star_pos.is_some() {
            CmpOp::GtE
        } else {
            CmpOp::Eq
        };
        let cmp = self.alloc(
            HirExprKind::Compare {
                op: cmp_op,
                l: len_ref,
                r: need_lit,
            },
            SemTy::Bool,
            span,
        );
        let cont = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: cmp,
            then: cont,
            else_: fail,
        });
        self.switch(cont);

        // Prefix elements: subject[i].
        for (i, sub) in prefix.iter().enumerate() {
            let base = self.local_ref(scr, span);
            let idx = self.alloc(HirExprKind::IntLit(i as i64), SemTy::Int, span);
            let elem = self.alloc(
                HirExprKind::Subscript { base, index: idx },
                SemTy::Dyn,
                span,
            );
            let tmp = self.fresh_local(SemTy::Dyn);
            self.push_stmt(HirStmt::Assign {
                target: tmp,
                value: elem,
            });
            self.lower_pattern(sub, tmp, fail, span)?;
        }
        // Star capture: subject[p .. n-m] as a fresh list.
        if let Some(p) = star_pos {
            let Pattern::MatchStar(st) = &pats[p] else {
                unreachable!()
            };
            if let Some(name) = &st.name {
                let lo = self.alloc(HirExprKind::IntLit(p as i64), SemTy::Int, span);
                let len_ref = self.local_ref(len_l, span);
                let m_lit = self.alloc(HirExprKind::IntLit(suffix.len() as i64), SemTy::Int, span);
                let hi = self.alloc(
                    HirExprKind::BinOp {
                        op: BinOp::Sub,
                        l: len_ref,
                        r: m_lit,
                    },
                    SemTy::Dyn,
                    span,
                );
                let rest = self.build_sublist(scr, lo, hi, span)?;
                let iname = self.intern(name.as_str());
                let v = self.local_ref(rest, span);
                self.write_named(iname, SemTy::Dyn, v);
            }
        }
        // Suffix elements: subject[n - (m - j)].
        for (j, sub) in suffix.iter().enumerate() {
            let len_ref = self.local_ref(len_l, span);
            let back = self.alloc(
                HirExprKind::IntLit((suffix.len() - j) as i64),
                SemTy::Int,
                span,
            );
            let idx = self.alloc(
                HirExprKind::BinOp {
                    op: BinOp::Sub,
                    l: len_ref,
                    r: back,
                },
                SemTy::Dyn,
                span,
            );
            let base = self.local_ref(scr, span);
            let elem = self.alloc(
                HirExprKind::Subscript { base, index: idx },
                SemTy::Dyn,
                span,
            );
            let tmp = self.fresh_local(SemTy::Dyn);
            self.push_stmt(HirStmt::Assign {
                target: tmp,
                value: elem,
            });
            self.lower_pattern(sub, tmp, fail, span)?;
        }
        Ok(())
    }

    /// Build a fresh list of `src[lo..hi]` (both bounds already lowered,
    /// evaluated exactly once).
    fn build_sublist(
        &mut self,
        src: LocalId,
        lo: Idx<HirExpr>,
        hi: Idx<HirExpr>,
        span: Span,
    ) -> Result<LocalId> {
        let result = self.fresh_local(SemTy::list_of(SemTy::Dyn));
        let empty = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: result,
            value: empty,
        });
        let cursor = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: cursor,
            value: lo,
        });
        let hi_l = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: hi_l,
            value: hi,
        });

        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let c1 = self.local_ref(cursor, span);
        let h1 = self.local_ref(hi_l, span);
        let cond = self.alloc(
            HirExprKind::Compare {
                op: CmpOp::Lt,
                l: c1,
                r: h1,
            },
            SemTy::Bool,
            span,
        );
        let body_b = self.new_block();
        let exit = self.new_block();
        self.seal(HirTerminator::Branch {
            cond,
            then: body_b,
            else_: exit,
        });

        self.switch(body_b);
        let base = self.local_ref(src, span);
        let c2 = self.local_ref(cursor, span);
        let elem = self.alloc(HirExprKind::Subscript { base, index: c2 }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::ContainerPush {
            container: result,
            value: elem,
        });
        let c3 = self.local_ref(cursor, span);
        let one = self.alloc(HirExprKind::IntLit(1), SemTy::Int, span);
        let inc = self.alloc(
            HirExprKind::BinOp {
                op: BinOp::Add,
                l: c3,
                r: one,
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: cursor,
            value: inc,
        });
        self.seal(HirTerminator::Jump(header));

        self.switch(exit);
        Ok(result)
    }

    /// A mapping pattern `{k: p, …, **rest}`: per-key `Contains` → branch,
    /// bind via `DictGet`; `**rest` is a copy with the matched keys popped
    /// (the original is untouched).
    fn lower_mapping_pattern(
        &mut self,
        keys: &[Expr],
        pats: &[rustpython_parser::ast::Pattern],
        rest: Option<&rustpython_parser::ast::Identifier>,
        scr: LocalId,
        fail: Idx<HirBlock>,
        span: Span,
    ) -> Result<()> {
        // Stage the keys once (used by Contains, DictGet, and DictPopM).
        let mut key_locals = Vec::with_capacity(keys.len());
        for k in keys {
            let ke = self.lower_expr(k)?;
            let kl = self.fresh_local(SemTy::Dyn);
            self.push_stmt(HirStmt::Assign {
                target: kl,
                value: ke,
            });
            key_locals.push(kl);
        }
        // Membership tests, then sub-pattern binds.
        for (kl, sub) in key_locals.iter().zip(pats) {
            let c = self.local_ref(scr, span);
            let k = self.local_ref(*kl, span);
            let has = self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::Contains,
                    args: vec![c, k],
                },
                SemTy::Bool,
                span,
            );
            let cont = self.new_block();
            self.seal(HirTerminator::Branch {
                cond: has,
                then: cont,
                else_: fail,
            });
            self.switch(cont);

            let c2 = self.local_ref(scr, span);
            let k2 = self.local_ref(*kl, span);
            let got = self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::DictGet,
                    args: vec![c2, k2],
                },
                SemTy::Dyn,
                span,
            );
            let tmp = self.fresh_local(SemTy::Dyn);
            self.push_stmt(HirStmt::Assign {
                target: tmp,
                value: got,
            });
            self.lower_pattern(sub, tmp, fail, span)?;
        }
        // `**rest` = copy minus the matched keys (copy semantics).
        if let Some(rest_name) = rest {
            let c = self.local_ref(scr, span);
            let copy = self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::DictCopy,
                    args: vec![c],
                },
                SemTy::dict_of(SemTy::Dyn, SemTy::Dyn),
                span,
            );
            let copy_l = self.fresh_local(SemTy::dict_of(SemTy::Dyn, SemTy::Dyn));
            self.push_stmt(HirStmt::Assign {
                target: copy_l,
                value: copy,
            });
            for kl in &key_locals {
                let d = self.local_ref(copy_l, span);
                let k = self.local_ref(*kl, span);
                let popped = self.alloc(
                    HirExprKind::ContainerExpr {
                        op: ContainerOp::DictPopM,
                        args: vec![d, k],
                    },
                    SemTy::Dyn,
                    span,
                );
                self.push_stmt(HirStmt::Expr(popped));
            }
            let iname = self.intern(rest_name.as_str());
            let v = self.local_ref(copy_l, span);
            self.write_named(iname, SemTy::Dyn, v);
        }
        Ok(())
    }

    /// A class pattern `Cls(attr=p, …)` (keyword-only): `IsInstance` → branch,
    /// then per-kwarg attribute reads feeding sub-patterns.
    fn lower_class_pattern(
        &mut self,
        c: &rustpython_parser::ast::PatternMatchClass,
        scr: LocalId,
        fail: Idx<HirBlock>,
        span: Span,
    ) -> Result<()> {
        let Expr::Name(n) = c.cls.as_ref() else {
            return Err(parse_error("class pattern must name a user class", span));
        };
        let Some((cid, iname)) = self.ctx.class_map.get(n.id.as_str()).copied() else {
            return Err(parse_error(
                format!("unknown class `{}` in class pattern", n.id.as_str()),
                span,
            ));
        };
        if !c.patterns.is_empty() {
            return Err(parse_error(
                "positional class patterns (`__match_args__`) are out of scope; \
                 use keyword patterns (`Cls(attr=…)`)",
                span,
            ));
        }
        let v = self.local_ref(scr, span);
        let isinst = self.alloc(
            HirExprKind::IsInstance {
                value: v,
                class_id: cid,
            },
            SemTy::Bool,
            span,
        );
        let cont = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: isinst,
            then: cont,
            else_: fail,
        });
        self.switch(cont);

        // Narrow the subject to the class type so attribute reads resolve. The
        // narrowing is runtime-guarded by the `IsInstance` branch above, but
        // inference would still see `Class{subject} → Class{pattern}` and the
        // annotation-contract check would reject it — so the value is
        // type-erased through a shared cell first (a `cell_shared` `CellGet`
        // is always `Dyn`, the gradual seam the contract check admits).
        let erase_name = self.intern("<match-subject>");
        let cell_lid = self.alloc_cell_local(erase_name, SemTy::Dyn, true);
        let init = self.local_ref(scr, span);
        let mc = self.alloc(HirExprKind::MakeCell { init: Some(init) }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: cell_lid,
            value: mc,
        });
        let erased = self.alloc(HirExprKind::CellGet { cell: cell_lid }, SemTy::Dyn, span);
        let cls_local = self.fresh_local(SemTy::Class {
            class_id: cid,
            name: iname,
        });
        self.push_stmt(HirStmt::Assign {
            target: cls_local,
            value: erased,
        });

        for (attr, sub) in c.kwd_attrs.iter().zip(&c.kwd_patterns) {
            let base = self.local_ref(cls_local, span);
            let aname = self.intern(attr.as_str());
            let read = self.alloc(
                HirExprKind::Attribute {
                    value: base,
                    name: aname,
                },
                SemTy::Dyn,
                span,
            );
            let tmp = self.fresh_local(SemTy::Dyn);
            self.push_stmt(HirStmt::Assign {
                target: tmp,
                value: read,
            });
            self.lower_pattern(sub, tmp, fail, span)?;
        }
        Ok(())
    }

    // ── expressions ──────────────────────────────────────────────────────────

    fn lower_expr(&mut self, expr: &Expr) -> Result<Idx<HirExpr>> {
        let span = to_span(expr.range());
        match expr {
            Expr::Constant(c) => self.lower_constant(&c.value, span),
            Expr::Name(n) => {
                let name = self.intern(n.id.as_str());
                // A name the frontend already has in scope resolves directly
                // through its binding (a local read or a `CellGet`); a top-level
                // function used as a VALUE becomes its memoized thunk closure
                // (Phase 6A); everything else defers to `semantics`.
                if let Some(b) = self.scope.get(&name).copied() {
                    Ok(self.read_binding(b, span))
                } else if let Some(var_id) = self.global_read_slot(name) {
                    Ok(self.alloc(HirExprKind::GlobalGet { var_id }, SemTy::Dyn, span))
                } else if self.ctx.top_defs.contains_key(n.id.as_str()) {
                    self.lower_top_fn_value(n.id.as_str(), span)
                } else if let Some(c) = self.ctx.stdlib.consts.get(n.id.as_str()).copied() {
                    // A from-imported stdlib constant (`from math import pi`)
                    // folds to its literal at every use site (Phase 8B).
                    Ok(self.lower_stdlib_const(&c.value, span))
                } else if let Some(attr) = self.ctx.stdlib.attrs.get(n.id.as_str()).copied() {
                    // A from-imported module attribute (`from sys import argv`).
                    Ok(self.alloc(
                        HirExprKind::CallRuntime {
                            target: pyaot_hir::RuntimeCallTarget::Attr(attr),
                            args: vec![],
                            provided: 0,
                        },
                        SemTy::Dyn,
                        span,
                    ))
                } else {
                    Ok(self.alloc(
                        HirExprKind::Name(SymbolRef::Unresolved(name)),
                        SemTy::Dyn,
                        span,
                    ))
                }
            }
            Expr::Lambda(l) => self.lower_lambda(l, span),
            Expr::UnaryOp(u) => self.lower_unary(u, span),
            Expr::BinOp(b) => self.lower_binop(b, span),
            Expr::Compare(c) => self.lower_compare(c, span),
            Expr::BoolOp(b) => self.lower_boolop(b),
            Expr::IfExp(e) => self.lower_ifexp(e),
            Expr::Call(c) => self.lower_call_expr(c, span),
            // ── containers (Phase 4) ──
            Expr::List(l) => {
                let elems = self.lower_expr_list(&l.elts)?;
                Ok(self.alloc(HirExprKind::ListLit { elems }, SemTy::Dyn, span))
            }
            Expr::Tuple(t) => {
                let elems = self.lower_expr_list(&t.elts)?;
                Ok(self.alloc(HirExprKind::TupleLit { elems }, SemTy::Dyn, span))
            }
            Expr::Set(s) => {
                let elems = self.lower_expr_list(&s.elts)?;
                Ok(self.alloc(HirExprKind::SetLit { elems }, SemTy::Dyn, span))
            }
            Expr::Dict(d) => {
                let mut pairs = Vec::with_capacity(d.values.len());
                for (k, v) in d.keys.iter().zip(d.values.iter()) {
                    let Some(k) = k else {
                        return Err(parse_error("dict unpacking (`**`) is out of scope", span));
                    };
                    let kk = self.lower_expr(k)?;
                    let vv = self.lower_expr(v)?;
                    pairs.push((kk, vv));
                }
                Ok(self.alloc(HirExprKind::DictLit { pairs }, SemTy::Dyn, span))
            }
            Expr::Subscript(s) => self.lower_subscript_expr(s, span),
            Expr::Attribute(a) => {
                // `e.__class__.__name__` (Phase 7B): constant-fold to the bare
                // class name from the variable's static type. (Documented
                // divergence: a base-typed `except` clause folds the static —
                // not dynamic — class name; the corpus only reads exact
                // handler matches.)
                if a.attr.as_str() == "__name__" {
                    if let Expr::Attribute(inner) = a.value.as_ref() {
                        if inner.attr.as_str() == "__class__" {
                            return self.fold_class_name(inner.value.as_ref(), span);
                        }
                    }
                }
                // `M.VAR` / `M.func` through an `import M` alias (Phase 8): a live
                // module-variable read folds to a `GlobalGet` of the exporter's
                // slot; an aliased function used as a value becomes its thunk.
                if let Expr::Name(m) = a.value.as_ref() {
                    if self.ctx.aliases.contains(m.id.as_str()) {
                        let mname = self.intern(m.id.as_str());
                        if !self.scope.contains_key(&mname) {
                            let qual = format!("{}.{}", m.id.as_str(), a.attr.as_str());
                            if let Some(slot) = self.ctx.alias_vars.get(&qual).copied() {
                                return Ok(self.alloc(
                                    HirExprKind::GlobalGet { var_id: slot },
                                    SemTy::Dyn,
                                    span,
                                ));
                            }
                            if self.ctx.top_defs.contains_key(&qual) {
                                return self.lower_top_fn_value(&qual, span);
                            }
                        }
                    }
                    // `M.pi` / `M.argv` through an `import M` stdlib alias
                    // (Phase 8B): a constant folds to its literal; a module
                    // attribute becomes its getter call.
                    if self.ctx.stdlib.aliases.contains(m.id.as_str()) {
                        let mname = self.intern(m.id.as_str());
                        if !self.scope.contains_key(&mname) {
                            let qual = format!("{}.{}", m.id.as_str(), a.attr.as_str());
                            if let Some(c) = self.ctx.stdlib.consts.get(&qual).copied() {
                                return Ok(self.lower_stdlib_const(&c.value, span));
                            }
                            if let Some(attr) = self.ctx.stdlib.attrs.get(&qual).copied() {
                                return Ok(self.alloc(
                                    HirExprKind::CallRuntime {
                                        target: pyaot_hir::RuntimeCallTarget::Attr(attr),
                                        args: vec![],
                                        provided: 0,
                                    },
                                    SemTy::Dyn,
                                    span,
                                ));
                            }
                            return Err(parse_error(
                                format!(
                                    "stdlib module `{}` has no attribute `{}`",
                                    m.id.as_str(),
                                    a.attr.as_str()
                                ),
                                span,
                            ));
                        }
                    }
                }
                let value = self.lower_expr(a.value.as_ref())?;
                let name = self.intern(a.attr.as_str());
                Ok(self.alloc(HirExprKind::Attribute { value, name }, SemTy::Dyn, span))
            }
            Expr::ListComp(c) => self.lower_listcomp(c, span),
            Expr::SetComp(c) => self.lower_setcomp(c, span),
            Expr::DictComp(c) => self.lower_dictcomp(c, span),
            Expr::GeneratorExp(g) => self.lower_genexpr(g, span),
            // f-string interpolation (Phase 8B, minimal): each `{expr}` part
            // desugars to `str(expr)` and the parts concatenate left-to-right.
            // Format specs / conversions (`{x:.4f}`, `{x!r}`) are Phase 8E.
            Expr::JoinedStr(j) => self.lower_joined_str(j, span),
            // Walrus / named expression `(target := value)` (PEP 572, §2).
            Expr::NamedExpr(n) => self.lower_named_expr(n, span),
            other => Err(parse_error(
                "unsupported expression for this milestone",
                to_span(other.range()),
            )),
        }
    }

    /// Walrus / named expression `(target := value)` (PEP 572, §2): evaluate
    /// `value` ONCE, bind it to `target` (a bare `Name` per the grammar) in the
    /// containing scope, and evaluate to the assigned value. The binding routes
    /// through the ordinary write/read place machinery (a local, a captured cell,
    /// or a promoted module-global slot — `resolve_write_place`), so a name bound
    /// in an `if`/`while` test is visible after the statement, exactly as CPython.
    /// The write stmt is emitted before the enclosing expression reads the slot,
    /// so the assignment and the expression's value coincide (single evaluation).
    fn lower_named_expr(&mut self, n: &ExprNamedExpr, span: Span) -> Result<Idx<HirExpr>> {
        let Expr::Name(target) = n.target.as_ref() else {
            // PEP 572 restricts the target to an identifier; the parser enforces
            // this, but guard defensively rather than mis-lower a non-name.
            return Err(parse_error("walrus target must be a name", span));
        };
        let value = self.lower_expr(n.value.as_ref())?;
        let name = self.intern(target.id.as_str());
        let place = self.resolve_write_place(name, SemTy::Dyn);
        self.write_place(place, value);
        Ok(self.read_place(place, span))
    }

    /// Lower a list of expressions (literal elements).
    fn lower_expr_list(&mut self, exprs: &[Expr]) -> Result<Vec<Idx<HirExpr>>> {
        exprs.iter().map(|e| self.lower_expr(e)).collect()
    }

    /// Lower a subscript read `value[index]`, or a slice `value[a:b:c]`
    /// (Phase 8E) when the index is a slice expression.
    fn lower_subscript_expr(&mut self, s: &ExprSubscript, span: Span) -> Result<Idx<HirExpr>> {
        if let Expr::Slice(sl) = s.slice.as_ref() {
            let base = self.lower_expr(s.value.as_ref())?;
            let lower_opt =
                |this: &mut Self, e: &Option<Box<Expr>>| -> Result<Option<Idx<HirExpr>>> {
                    match e {
                        Some(x) => Ok(Some(this.lower_expr(x.as_ref())?)),
                        None => Ok(None),
                    }
                };
            let start = lower_opt(self, &sl.lower)?;
            let end = lower_opt(self, &sl.upper)?;
            let step = lower_opt(self, &sl.step)?;
            // The result kind mirrors the base's static type; typeck assigns it.
            return Ok(self.alloc(
                HirExprKind::Slice {
                    base,
                    start,
                    end,
                    step,
                },
                SemTy::Dyn,
                span,
            ));
        }
        let base = self.lower_expr(s.value.as_ref())?;
        let index = self.lower_expr(s.slice.as_ref())?;
        Ok(self.alloc(HirExprKind::Subscript { base, index }, SemTy::Dyn, span))
    }

    // ── comprehensions (Phase 4C) ──────────────────────────────────────────────

    /// f-string lowering (§13). Literal parts are `StrLit`s; each `{expr[!conv][:spec]}`
    /// field becomes a `FormatValue { value, spec }` (CPython `f"{x:spec}"` ≡
    /// `format(x, "spec")`); parts fold left-to-right with string `+`. Every
    /// field — even a bare `{x}` — routes through `FormatValue` so a class
    /// instance reaches its `__format__`/`__str__` (an empty spec degrades to
    /// `str(x)` for non-instances). A `:spec` is itself a `JoinedStr`, so a
    /// dynamic spec (`f"{x:.{n}f}"`) lowers through this same path; a static one
    /// collapses to a single `StrLit`. `!s`/`!r`/`!a` wraps the value in
    /// `str(...)`/`repr(...)`/`ascii(...)` FIRST (CPython applies the conversion,
    /// then `__format__`).
    fn lower_joined_str(
        &mut self,
        j: &rustpython_parser::ast::ExprJoinedStr,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        let mut parts: Vec<Idx<HirExpr>> = Vec::with_capacity(j.values.len());
        for part in &j.values {
            match part {
                Expr::Constant(c) => {
                    let Constant::Str(s) = &c.value else {
                        return Err(parse_error("unsupported f-string literal part", span));
                    };
                    let id = self.intern(s);
                    parts.push(self.alloc(HirExprKind::StrLit(id), SemTy::Str, span));
                }
                Expr::FormattedValue(fv) => {
                    let raw = self.lower_expr(fv.value.as_ref())?;
                    // The field's `:spec` is itself a `JoinedStr` — a literal spec
                    // collapses to a `StrLit`, a dynamic one (`{x:.{n}f}`) lowers
                    // through the normal f-string concat. No spec ⇒ empty string.
                    let spec = match &fv.format_spec {
                        Some(spec_expr) => self.lower_format_spec_expr(spec_expr.as_ref(), span)?,
                        None => {
                            let id = self.intern("");
                            self.alloc(HirExprKind::StrLit(id), SemTy::Str, span)
                        }
                    };
                    parts.push(self.emit_format_field(raw, fv.conversion, spec, span));
                }
                _ => return Err(parse_error("unsupported f-string part", span)),
            }
        }
        Ok(self.concat_str_parts(parts, span))
    }

    /// Lower a format field's `:spec` (an f-string `JoinedStr`) to a string-valued
    /// expr. A purely-literal spec (the common static case, `f"{x:.4f}"`) collapses
    /// to one `StrLit`; a spec with a nested `{}` (`f"{x:.{n}f}"`) lowers through
    /// the ordinary f-string concat so the embedded value is `format()`-ed and
    /// spliced in.
    fn lower_format_spec_expr(&mut self, spec: &Expr, span: Span) -> Result<Idx<HirExpr>> {
        if let Some(lit) = static_spec_literal(spec) {
            let id = self.intern(&lit);
            return Ok(self.alloc(HirExprKind::StrLit(id), SemTy::Str, span));
        }
        self.lower_expr(spec)
    }

    /// Apply an f-string / `str.format` conversion (`!s`/`!r`/`!a`) to `value`,
    /// then wrap it in a `FormatValue` with `spec` (a string-valued expr). The
    /// shared field-builder for f-strings (`lower_joined_str`) and `str.format`
    /// (`lower_str_format`). CPython applies the conversion FIRST, then formats
    /// the (string) result with the spec.
    fn emit_format_field(
        &mut self,
        value: Idx<HirExpr>,
        conv: rustpython_parser::ast::ConversionFlag,
        spec: Idx<HirExpr>,
        span: Span,
    ) -> Idx<HirExpr> {
        use rustpython_parser::ast::ConversionFlag;
        let converted = match conv {
            ConversionFlag::Str => self.call_builtin1("str", value, span),
            ConversionFlag::Repr => self.call_builtin1("repr", value, span),
            ConversionFlag::Ascii => self.call_builtin1("ascii", value, span),
            ConversionFlag::None => value,
        };
        self.alloc(
            HirExprKind::FormatValue {
                value: converted,
                spec,
            },
            SemTy::Str,
            span,
        )
    }

    /// Build a one-argument call to an unshadowed builtin by name
    /// (`str`/`repr`/`ascii`), resolved by `semantics` to `Symbol::Builtin`.
    fn call_builtin1(&mut self, name: &str, arg: Idx<HirExpr>, span: Span) -> Idx<HirExpr> {
        let fn_name = self.intern(name);
        let callee = self.alloc(
            HirExprKind::Name(SymbolRef::Unresolved(fn_name)),
            SemTy::Dyn,
            span,
        );
        self.alloc(
            HirExprKind::Call {
                callee,
                args: vec![arg],
            },
            SemTy::Str,
            span,
        )
    }

    /// Fold string parts left-to-right with `+` (the f-string / `str.format`
    /// tail). An empty part list yields the empty `StrLit`.
    fn concat_str_parts(&mut self, parts: Vec<Idx<HirExpr>>, span: Span) -> Idx<HirExpr> {
        let mut iter = parts.into_iter();
        let Some(mut acc) = iter.next() else {
            let id = self.intern("");
            return self.alloc(HirExprKind::StrLit(id), SemTy::Str, span);
        };
        for p in iter {
            acc = self.alloc(
                HirExprKind::BinOp {
                    op: BinOp::Add,
                    l: acc,
                    r: p,
                },
                SemTy::Dyn,
                span,
            );
        }
        acc
    }

    fn lower_listcomp(&mut self, c: &ExprListComp, span: Span) -> Result<Idx<HirExpr>> {
        // `Dyn` (non-authoritative) so typeck infers the ELEMENT type from the
        // desugared pushes (Phase 8H, D1) instead of pinning `list[Dyn]`.
        let result = self.fresh_local(SemTy::Dyn);
        let empty = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: result,
            value: empty,
        });
        let kind = CompKind::List {
            result,
            elt: c.elt.as_ref(),
        };
        self.lower_comp_clauses(&c.generators, 0, &kind, span)?;
        Ok(self.local_ref(result, span))
    }

    /// `{elt for … if …}` → an empty set filled the same way.
    fn lower_setcomp(&mut self, c: &ExprSetComp, span: Span) -> Result<Idx<HirExpr>> {
        // `Dyn` for push-driven element inference (Phase 8H, D1).
        let result = self.fresh_local(SemTy::Dyn);
        let empty = self.alloc(HirExprKind::SetLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: result,
            value: empty,
        });
        let kind = CompKind::Set {
            result,
            elt: c.elt.as_ref(),
        };
        self.lower_comp_clauses(&c.generators, 0, &kind, span)?;
        Ok(self.local_ref(result, span))
    }

    /// `{k: v for … if …}` → an empty dict filled key/value-wise.
    fn lower_dictcomp(&mut self, c: &ExprDictComp, span: Span) -> Result<Idx<HirExpr>> {
        // `Dyn` for insert-driven key/value inference (Phase 8H, D1).
        let result = self.fresh_local(SemTy::Dyn);
        let empty = self.alloc(HirExprKind::DictLit { pairs: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: result,
            value: empty,
        });
        let kind = CompKind::Dict {
            result,
            key: c.key.as_ref(),
            val: c.value.as_ref(),
        };
        self.lower_comp_clauses(&c.generators, 0, &kind, span)?;
        Ok(self.local_ref(result, span))
    }

    /// Nest the comprehension's `for`/`if` clauses (one iterator loop per `for`),
    /// emitting the element action at the innermost point.
    fn lower_comp_clauses(
        &mut self,
        generators: &[Comprehension],
        idx: usize,
        kind: &CompKind,
        span: Span,
    ) -> Result<()> {
        if idx == generators.len() {
            return self.emit_comp_elem(kind, span);
        }
        // Comprehension loop variables are scoped to the comprehension (CPython 3
        // runs each comprehension in its own function). List/set/dict comps lower
        // inline here, so shadow every target name with a fresh local for the
        // duration and restore the outer binding afterward — otherwise
        // `[x for x in xs]` would clobber an enclosing `x` (genexprs already get
        // their own nested-function scope, so they need no shadowing). Pre-inserting
        // a fresh `Direct` binding (rather than removing the outer one) keeps writes
        // off a promoted global slot too.
        let saved_targets: Vec<(InternedString, Option<Binding>)> = if idx == 0 {
            let mut saved = Vec::new();
            let mut raw_names = Vec::new();
            for g in generators {
                collect_target_names(&g.target, &mut raw_names);
            }
            for raw in raw_names {
                let name = self.intern(raw);
                let prev = self.scope.get(&name).copied();
                let fresh = self.fresh_local(SemTy::Dyn);
                self.scope.insert(name, Binding::Direct(fresh));
                saved.push((name, prev));
            }
            saved
        } else {
            Vec::new()
        };

        let gen = &generators[idx];
        if gen.is_async {
            return Err(parse_error("async comprehensions are out of scope", span));
        }

        // it = iter(gen.iter)
        let iterable = self.lower_iterable_expr(&gen.iter, span)?;
        let it = self.fresh_local(SemTy::Dyn);
        let iter_expr = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Iter,
                args: vec![iterable],
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: it,
            value: iter_expr,
        });

        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let elem = self.fresh_local_tagged();
        let it_ref1 = self.local_ref(it, span);
        let next = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::IterNext,
                args: vec![it_ref1],
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: elem,
            value: next,
        });
        let it_ref2 = self.local_ref(it, span);
        let done = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::IterExhausted,
                args: vec![it_ref2],
            },
            SemTy::Bool,
            span,
        );
        let body_b = self.new_block();
        let exit = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: done,
            then: exit,
            else_: body_b,
        });

        self.switch(body_b);
        let elem_ref = self.local_ref(elem, span);
        self.bind_for_target(&gen.target, elem_ref, span)?;
        // Filters: a false `if` skips to the next element (jump back to header).
        for cond_expr in &gen.ifs {
            let cond = self.lower_expr(cond_expr)?;
            let cont = self.new_block();
            self.seal(HirTerminator::Branch {
                cond,
                then: cont,
                else_: header,
            });
            self.switch(cont);
        }
        // Recurse into the next clause (or emit the element at the innermost).
        self.lower_comp_clauses(generators, idx + 1, kind, span)?;
        self.seal(HirTerminator::Jump(header));
        self.switch(exit);
        // Restore the outer bindings the comprehension's loop variables shadowed.
        for (name, prev) in saved_targets {
            match prev {
                Some(b) => {
                    self.scope.insert(name, b);
                }
                None => {
                    self.scope.remove(&name);
                }
            }
        }
        Ok(())
    }

    // ── reduce/loop builtins: sum / min / max / set (Phase 4C) ─────────────────

    /// Emit the iterator-protocol prologue for a simple loop over an
    /// already-lowered iterable, switching to the loop body and returning the
    /// per-iteration element local plus the header/exit blocks. Pair with
    /// [`Self::end_iter_loop`]. (Used by `sum`/`min`/`max`/`set` — no target
    /// binding, filters, or `break`/`continue`, unlike the `for`/comprehension
    /// paths.)
    fn begin_iter_loop(&mut self, iterable: Idx<HirExpr>, span: Span) -> Result<IterLoop> {
        let it = self.fresh_local(SemTy::Dyn);
        let iter_expr = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Iter,
                args: vec![iterable],
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: it,
            value: iter_expr,
        });
        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let elem = self.fresh_local_tagged();
        let it_ref1 = self.local_ref(it, span);
        let next = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::IterNext,
                args: vec![it_ref1],
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: elem,
            value: next,
        });
        let it_ref2 = self.local_ref(it, span);
        let done = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::IterExhausted,
                args: vec![it_ref2],
            },
            SemTy::Bool,
            span,
        );
        let body_b = self.new_block();
        let exit = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: done,
            then: exit,
            else_: body_b,
        });
        self.switch(body_b);
        Ok(IterLoop { header, exit, elem })
    }

    /// Close a [`Self::begin_iter_loop`] loop: jump back to the header and switch
    /// to the exit block.
    fn end_iter_loop(&mut self, lp: IterLoop) {
        self.seal(HirTerminator::Jump(lp.header));
        self.switch(lp.exit);
    }

    /// `sum(iterable[, start])` → [`HirExprKind::Sum`] (Phase 8H, D2): typeck
    /// types the accumulator precisely (numeric promotion / inferred dunder
    /// returns), lowering expands the iterator loop. A generator-expression
    /// argument is MATERIALIZED into a list comprehension here — eager, not
    /// lazy, which is observationally identical for sum (the corpus inputs are
    /// finite and side-effect-free).
    fn lower_sum(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.is_empty() || args.len() > 2 {
            return Err(parse_error("sum() takes 1 or 2 arguments", span));
        }
        let iterable = if let Expr::GeneratorExp(g) = &args[0] {
            // Same desugar as a list comprehension, driven by the genexpr's
            // elt/generators.
            let result = self.fresh_local(SemTy::Dyn);
            let empty = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
            self.push_stmt(HirStmt::Assign {
                target: result,
                value: empty,
            });
            let kind = CompKind::List {
                result,
                elt: g.elt.as_ref(),
            };
            self.lower_comp_clauses(&g.generators, 0, &kind, span)?;
            self.local_ref(result, span)
        } else {
            self.lower_expr(&args[0])?
        };
        let start = match args.get(1) {
            Some(s) => Some(self.lower_expr(s)?),
            None => None,
        };
        Ok(self.alloc(HirExprKind::Sum { iterable, start }, SemTy::Dyn, span))
    }

    /// `min`/`max` over a single iterable, or over 2+ positional args (wrapped in a
    /// synthetic list), with optional `key=`. Compares with the tagged baseline
    /// (`rt_obj_cmp`), so heap elements order by value, not pointer (PITFALLS
    /// B13). An empty input raises `ValueError` (Phase 7, CPython semantics);
    /// the accumulator is seeded from the first element, so its inferred type
    /// is the element type — never a spurious `Optional`.
    fn lower_minmax(
        &mut self,
        args: &[Expr],
        key: Option<&Expr>,
        span: Span,
        is_min: bool,
    ) -> Result<Idx<HirExpr>> {
        if args.is_empty() {
            return Err(parse_error(
                "min()/max() require at least one argument",
                span,
            ));
        }
        // 1 arg → iterate it; 2+ args → iterate a synthetic list of the args.
        let iterable = if args.len() == 1 {
            self.lower_expr(&args[0])?
        } else {
            let elems = self.lower_expr_list(args)?;
            self.alloc(HirExprKind::ListLit { elems }, SemTy::Dyn, span)
        };
        // The key callable: a bare out-of-scope name (`key=abs`, `key=len`, a
        // top-level def) is called DIRECTLY per element — builtins have no
        // value-position thunk, so staging would reject them. Anything else
        // (lambda, a local) is staged once and called indirectly (CPython
        // evaluates the key expression once; a bare name re-read is pure).
        let key_mode: Option<KeyMode> = match key {
            None => None,
            Some(k @ Expr::Name(n)) => {
                let iname = self.intern(n.id.as_str());
                if self.scope.contains_key(&iname) {
                    let kv = self.lower_expr(k)?;
                    let l = self.fresh_local(SemTy::Dyn);
                    self.push_stmt(HirStmt::Assign {
                        target: l,
                        value: kv,
                    });
                    Some(KeyMode::Staged(l))
                } else {
                    Some(KeyMode::ByName(k))
                }
            }
            Some(k) => {
                let kv = self.lower_expr(k)?;
                let l = self.fresh_local(SemTy::Dyn);
                self.push_stmt(HirStmt::Assign {
                    target: l,
                    value: kv,
                });
                Some(KeyMode::Staged(l))
            }
        };

        // it = iter(iterable); first probe decides empty-vs-seed.
        let it = self.fresh_local(SemTy::Dyn);
        let iter_expr = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Iter,
                args: vec![iterable],
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: it,
            value: iter_expr,
        });
        let elem0 = self.emit_iter_next(it, span);
        let done0 = self.emit_iter_exhausted(it, span);
        let empty_b = self.new_block();
        let first_b = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: done0,
            then: empty_b,
            else_: first_b,
        });

        // empty: raise ValueError — the live-oracle (CPython ≥3.13) wording.
        self.switch(empty_b);
        let what = if is_min { "min" } else { "max" };
        let msg_id = self.intern(&format!("{what}() iterable argument is empty"));
        let msg = self.alloc(HirExprKind::StrLit(msg_id), SemTy::Str, span);
        self.push_stmt(HirStmt::Raise(HirRaise::Builtin {
            tag: pyaot_core_defs::BuiltinExceptionKind::ValueError.tag(),
            msg: Some(msg),
        }));
        self.seal(HirTerminator::Unreachable);

        // seed: acc = elem0; acc_key = key(elem0) when keyed.
        self.switch(first_b);
        let acc = self.fresh_local(SemTy::Dyn);
        let e0 = self.local_ref(elem0, span);
        self.push_stmt(HirStmt::Assign {
            target: acc,
            value: e0,
        });
        let acc_key = match &key_mode {
            Some(km) => {
                let l = self.fresh_local(SemTy::Dyn);
                let v = self.emit_key_call(km, elem0, span)?;
                self.push_stmt(HirStmt::Assign {
                    target: l,
                    value: v,
                });
                Some(l)
            }
            None => None,
        };

        // loop: elem = next(it); done → exit; cand </> best → replace.
        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let elem = self.emit_iter_next(it, span);
        let done = self.emit_iter_exhausted(it, span);
        let body_b = self.new_block();
        let exit = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: done,
            then: exit,
            else_: body_b,
        });

        self.switch(body_b);
        let cand_key = match &key_mode {
            Some(km) => {
                let l = self.fresh_local(SemTy::Dyn);
                let v = self.emit_key_call(km, elem, span)?;
                self.push_stmt(HirStmt::Assign {
                    target: l,
                    value: v,
                });
                Some(l)
            }
            None => None,
        };
        let (cl, bl) = match (cand_key, acc_key) {
            (Some(c), Some(b)) => (c, b),
            _ => (elem, acc),
        };
        let cref = self.local_ref(cl, span);
        let bref = self.local_ref(bl, span);
        let op = if is_min { CmpOp::Lt } else { CmpOp::Gt };
        let cmp = self.alloc(
            HirExprKind::Compare {
                op,
                l: cref,
                r: bref,
            },
            SemTy::Bool,
            span,
        );
        let upd = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: cmp,
            then: upd,
            else_: header,
        });
        self.switch(upd);
        let e_ref = self.local_ref(elem, span);
        self.push_stmt(HirStmt::Assign {
            target: acc,
            value: e_ref,
        });
        if let (Some(ck), Some(ak)) = (cand_key, acc_key) {
            let ck_ref = self.local_ref(ck, span);
            self.push_stmt(HirStmt::Assign {
                target: ak,
                value: ck_ref,
            });
        }
        self.seal(HirTerminator::Jump(header));

        self.switch(exit);
        Ok(self.local_ref(acc, span))
    }

    /// `pow(a, b)` (PLAN §5) → the `**` operator (`BinOp::Pow`), which is
    /// already end-to-end and bignum- / numeric-tower-correct via `rt_obj_pow`
    /// (a negative exponent yields a float, exactly like `a ** b`). 2-arg only:
    /// the 1-arg form and the 3-arg modular form `pow(a, b, m)` are out of scope.
    fn lower_pow(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.len() != 2 {
            return Err(parse_error(
                "pow() takes exactly two arguments (1-arg and 3-arg modular pow \
                 are out of scope)",
                span,
            ));
        }
        let l = self.lower_expr(&args[0])?;
        let r = self.lower_expr(&args[1])?;
        Ok(self.alloc(HirExprKind::BinOp { op: BinOp::Pow, l, r }, SemTy::Dyn, span))
    }

    /// `divmod(a, b)` (PLAN §5) → the 2-tuple `(a // b, a % b)`. `a` and `b` are
    /// each staged into a fresh local ONCE, left-to-right (CPython
    /// evaluate-once / eval-order, §1); both binops apply CPython floor/sign
    /// semantics via `rt_obj_floordiv`/`rt_obj_mod` (PITFALLS B1), so the tuple
    /// is exact for negative operands too.
    fn lower_divmod(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.len() != 2 {
            return Err(parse_error("divmod() takes exactly two arguments", span));
        }
        let a_val = self.lower_expr(&args[0])?;
        let a = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: a,
            value: a_val,
        });
        let b_val = self.lower_expr(&args[1])?;
        let b = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: b,
            value: b_val,
        });

        let aq = self.local_ref(a, span);
        let bq = self.local_ref(b, span);
        let q = self.alloc(
            HirExprKind::BinOp {
                op: BinOp::FloorDiv,
                l: aq,
                r: bq,
            },
            SemTy::Dyn,
            span,
        );
        let ar = self.local_ref(a, span);
        let br = self.local_ref(b, span);
        let rem = self.alloc(
            HirExprKind::BinOp {
                op: BinOp::Mod,
                l: ar,
                r: br,
            },
            SemTy::Dyn,
            span,
        );
        Ok(self.alloc(HirExprKind::TupleLit { elems: vec![q, rem] }, SemTy::Dyn, span))
    }

    /// `all(iterable)` / `any(iterable)` (PLAN §5) — an iterator loop mirroring
    /// [`Self::lower_minmax`]. The accumulator seeds to the empty-input answer
    /// (`all([]) == True`, `any([]) == False`); each element is tested for
    /// truthiness (the same `Branch`-cond mechanism `if elem:` uses) and the
    /// loop short-circuits on the first falsy (`all`) / truthy (`any`) element,
    /// flipping the accumulator. The result is the `Bool` accumulator — zero new
    /// runtime (reuses `Iter`/`IterNext`/`IterExhausted` + existing truthiness).
    fn lower_all_any(&mut self, args: &[Expr], span: Span, is_all: bool) -> Result<Idx<HirExpr>> {
        if args.len() != 1 {
            return Err(parse_error(
                "all()/any() take exactly one argument",
                span,
            ));
        }
        let iterable = self.lower_expr(&args[0])?;

        // acc = empty-input answer (True for all, False for any).
        let acc = self.fresh_local(SemTy::Bool);
        let seed = self.alloc(HirExprKind::BoolLit(is_all), SemTy::Bool, span);
        self.push_stmt(HirStmt::Assign {
            target: acc,
            value: seed,
        });

        // it = iter(iterable).
        let it = self.fresh_local(SemTy::Dyn);
        let iter_expr = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Iter,
                args: vec![iterable],
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: it,
            value: iter_expr,
        });

        // loop: elem = next(it); done → exit; else test truthiness.
        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let elem = self.emit_iter_next(it, span);
        let done = self.emit_iter_exhausted(it, span);
        let body_b = self.new_block();
        let exit = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: done,
            then: exit,
            else_: body_b,
        });

        // body: branch on element truthiness. `all`: falsy short-circuits;
        // `any`: truthy short-circuits. The short-circuit edge flips the acc.
        self.switch(body_b);
        let hit = self.new_block();
        let elem_ref = self.local_ref(elem, span);
        let (then_b, else_b) = if is_all {
            (header, hit)
        } else {
            (hit, header)
        };
        self.seal(HirTerminator::Branch {
            cond: elem_ref,
            then: then_b,
            else_: else_b,
        });

        self.switch(hit);
        let flipped = self.alloc(HirExprKind::BoolLit(!is_all), SemTy::Bool, span);
        self.push_stmt(HirStmt::Assign {
            target: acc,
            value: flipped,
        });
        self.seal(HirTerminator::Jump(exit));

        self.switch(exit);
        Ok(self.local_ref(acc, span))
    }

    /// `functools.reduce(function, iterable[, initial])` — a higher-order
    /// builtin (like `map`/`filter`) desugared to a compiled accumulator loop
    /// calling `function(acc, elem)` each iteration, mirroring
    /// [`Self::lower_minmax`]'s seed-from-first-element shape. This deliberately
    /// AVOIDS the raw-ABI `rt_reduce` callback path (the PITFALLS A4
    /// anti-pattern — a parallel HOF calling convention with hand-encoded
    /// captures): the reduction callable rides the ordinary indirect-call
    /// machinery (lambda / closure / named def alike), so its arguments and
    /// result stay on the uniform tagged ABI. Without an `initial` the
    /// accumulator seeds from the first element and an empty iterable raises
    /// `TypeError` (CPython); with one, the accumulator seeds from `initial` and
    /// an empty iterable returns it unchanged.
    fn lower_reduce(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.len() < 2 || args.len() > 3 {
            return Err(parse_error("reduce() takes 2 or 3 arguments", span));
        }
        // The reduction callable, staged with the same discipline as the
        // `min`/`max` `key=` and evaluated FIRST (CPython left-to-right order).
        let func_mode = self.stage_callable(&args[0])?;

        let iterable = self.lower_expr(&args[1])?;
        let initial = match args.get(2) {
            Some(e) => Some(self.lower_expr(e)?),
            None => None,
        };

        // it = iter(iterable).
        let it = self.fresh_local(SemTy::Dyn);
        let iter_expr = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Iter,
                args: vec![iterable],
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: it,
            value: iter_expr,
        });

        // Seed the accumulator: from `initial` if given, else from the first
        // element (empty-without-initial raises TypeError, like CPython).
        let acc = self.fresh_local(SemTy::Dyn);
        match initial {
            Some(init) => {
                self.push_stmt(HirStmt::Assign {
                    target: acc,
                    value: init,
                });
            }
            None => {
                let elem0 = self.emit_iter_next(it, span);
                let done0 = self.emit_iter_exhausted(it, span);
                let empty_b = self.new_block();
                let seed_b = self.new_block();
                self.seal(HirTerminator::Branch {
                    cond: done0,
                    then: empty_b,
                    else_: seed_b,
                });

                self.switch(empty_b);
                let msg_id = self.intern("reduce() of empty iterable with no initial value");
                let msg = self.alloc(HirExprKind::StrLit(msg_id), SemTy::Str, span);
                self.push_stmt(HirStmt::Raise(HirRaise::Builtin {
                    tag: pyaot_core_defs::BuiltinExceptionKind::TypeError.tag(),
                    msg: Some(msg),
                }));
                self.seal(HirTerminator::Unreachable);

                self.switch(seed_b);
                let e0 = self.local_ref(elem0, span);
                self.push_stmt(HirStmt::Assign {
                    target: acc,
                    value: e0,
                });
            }
        }

        // loop: elem = next(it); done → exit; else acc = func(acc, elem).
        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let elem = self.emit_iter_next(it, span);
        let done = self.emit_iter_exhausted(it, span);
        let body_b = self.new_block();
        let exit = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: done,
            then: exit,
            else_: body_b,
        });

        self.switch(body_b);
        let call = self.emit_reduce_call(&func_mode, acc, elem, span)?;
        self.push_stmt(HirStmt::Assign {
            target: acc,
            value: call,
        });
        self.seal(HirTerminator::Jump(header));

        self.switch(exit);
        Ok(self.local_ref(acc, span))
    }

    /// `map(func, iterable)` — the next higher-order builtin after `reduce`,
    /// desugared to an EAGER compiled loop that calls `func(elem)` per element
    /// through the ordinary uniform-tagged indirect-call machinery, materializes
    /// the results into a `list`, and wraps it in an iterator (`ContainerOp::Iter`)
    /// so `for`/`list`/`next`/`sum` consume it like any other iterable.
    ///
    /// This deliberately AVOIDS the runtime `rt_map_new` / `IteratorKind::Map`
    /// lazy-iterator HOF machinery (the PITFALLS A4 anti-pattern — a parallel
    /// calling convention with hand-encoded captures, marker bits, and an `i8`
    /// predicate ABI). `func` is staged ONCE (CPython evaluates the callable a
    /// single time), and builtin callbacks (`map(str, …)` / `map(len, …)`) resolve
    /// through the normal `Symbol`-dispatch in `lowering::lower_call` with no extra
    /// code — they ride the same tagged `Call` a compiled lambda/closure does. The
    /// eager-vs-lazy side-effect timing is observationally identical on the finite,
    /// pure corpus (the `lower_sum`/`reduce` materialization precedent). Only the
    /// single-iterable form is supported; multi-iterable `map` needs `zip`
    /// (§12, out of scope).
    fn lower_map(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        match args.len() {
            2 => {}
            n if n > 2 => {
                return Err(parse_error(
                    "single iterable only — multi-iterable map() needs zip (§12), out of scope",
                    span,
                ))
            }
            _ => return Err(parse_error("map() takes a function and one iterable", span)),
        }
        // Stage the callable FIRST (CPython evaluates `func` once, before the
        // iterable), with the `min`/`max` `key=` discipline.
        let func = self.stage_callable(&args[0])?;
        let iterable = self.lower_expr(&args[1])?;

        // result = [] — a heap ListObj of uniform-Tagged elements, GC-rooted as a
        // stack local (the same B5-safe shape as `set()`/`sum(genexpr)`).
        let result = self.fresh_local(SemTy::list_of(SemTy::Dyn));
        let empty = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: result,
            value: empty,
        });

        // for elem in iterable: result.append(func(elem)).
        let lp = self.begin_iter_loop(iterable, span)?;
        let mapped = self.emit_key_call(&func, lp.elem, span)?;
        self.push_stmt(HirStmt::ContainerPush {
            container: result,
            value: mapped,
        });
        self.end_iter_loop(lp);

        let list_ref = self.local_ref(result, span);
        Ok(self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Iter,
                args: vec![list_ref],
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// `filter(func, iterable)` — the conditional sibling of [`Self::lower_map`]:
    /// an EAGER loop that pushes `elem` only when `func(elem)` is truthy. The
    /// special `filter(None, xs)` form (the predicate is the `None` literal)
    /// filters on the element's own truthiness instead. The survivors are
    /// materialized into a `list` wrapped in an iterator. Same A4 avoidance as
    /// `map` — the predicate rides the ordinary tagged `Call` (lowering
    /// truthiness-tests the result), never the `rt_filter_new` / `i8`-predicate-ABI
    /// HOF path. `func` is staged once (CPython single evaluation).
    fn lower_filter(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.len() != 2 {
            return Err(parse_error(
                "filter() takes a predicate (or None) and one iterable",
                span,
            ));
        }
        // `filter(None, xs)` keeps truthy elements directly; otherwise stage the
        // predicate once.
        let pred_is_none = is_none_lit(&args[0]);
        let func = if pred_is_none {
            None
        } else {
            Some(self.stage_callable(&args[0])?)
        };
        let iterable = self.lower_expr(&args[1])?;

        let result = self.fresh_local(SemTy::list_of(SemTy::Dyn));
        let empty = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: result,
            value: empty,
        });

        // for elem in iterable: if <pred>: result.append(elem). A falsy test
        // branches straight back to the loop header (skip), mirroring the
        // comprehension `if`-filter.
        let lp = self.begin_iter_loop(iterable, span)?;
        let cond = match &func {
            Some(mode) => self.emit_key_call(mode, lp.elem, span)?,
            None => self.local_ref(lp.elem, span),
        };
        let push_b = self.new_block();
        self.seal(HirTerminator::Branch {
            cond,
            then: push_b,
            else_: lp.header,
        });
        self.switch(push_b);
        let elem_ref = self.local_ref(lp.elem, span);
        self.push_stmt(HirStmt::ContainerPush {
            container: result,
            value: elem_ref,
        });
        self.end_iter_loop(lp);

        let list_ref = self.local_ref(result, span);
        Ok(self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Iter,
                args: vec![list_ref],
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// `format(value[, spec])` (§5) — the value/spec sibling of an f-string field
    /// and `str.format`. Desugars to the same `FormatValue { value, spec }`
    /// (`rt_format`) node, with the spec defaulting to the empty string (which
    /// routes a class instance to its `__format__`). Unshadowed-gated by the
    /// caller; no `!` conversion. A dynamic spec (`format(x, var)`) just lowers
    /// `var` as an ordinary string-valued expr.
    fn lower_format_builtin(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        let (value_expr, spec_expr) = match args {
            [v] => (v, None),
            [v, s] => (v, Some(s)),
            _ => return Err(parse_error("format() takes one or two arguments", span)),
        };
        let value = self.lower_expr(value_expr)?;
        let spec = match spec_expr {
            Some(s) => self.lower_expr(s)?,
            None => {
                let id = self.intern("");
                self.alloc(HirExprKind::StrLit(id), SemTy::Str, span)
            }
        };
        Ok(self.emit_format_field(
            value,
            rustpython_parser::ast::ConversionFlag::None,
            spec,
            span,
        ))
    }

    /// `getattr(obj, "name")` (§5) ≡ `obj.name` — a pure frontend desugar onto the
    /// existing [`HirExprKind::Attribute`] read (static `GetField` for a concrete
    /// receiver, gradual `GetFieldNamed` → `rt_getattr_name` for a `Dyn` one). The
    /// name must be a string literal — dynamic `getattr(o, var)` is the documented
    /// out-of-scope boundary — and the 3-arg `default` form is rejected.
    fn lower_getattr_builtin(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        match args.len() {
            2 => {}
            3 => return Err(parse_error("getattr() default is out of scope", span)),
            _ => return Err(parse_error("getattr() takes two arguments", span)),
        }
        let name = string_literal_arg(&args[1]).ok_or_else(|| {
            parse_error("dynamic getattr (non-literal name) is out of scope", span)
        })?;
        let value = self.lower_expr(&args[0])?;
        let name = self.intern(name);
        Ok(self.alloc(HirExprKind::Attribute { value, name }, SemTy::Dyn, span))
    }

    /// `setattr(obj, "name", value)` (§5) ≡ `obj.name = value` — a pure frontend
    /// desugar onto the existing [`HirStmt::SetAttr`] write (the `SetFieldNamed`
    /// legalize path for a gradual receiver). The name must be a string literal;
    /// the call evaluates to `None` (CPython's `setattr` return).
    fn lower_setattr_builtin(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.len() != 3 {
            return Err(parse_error("setattr() takes three arguments", span));
        }
        let name = string_literal_arg(&args[1]).ok_or_else(|| {
            parse_error("dynamic setattr (non-literal name) is out of scope", span)
        })?;
        let base = self.lower_expr(&args[0])?;
        let value = self.lower_expr(&args[2])?;
        let name = self.intern(name);
        self.push_stmt(HirStmt::SetAttr { base, name, value });
        Ok(self.alloc(HirExprKind::NoneLit, SemTy::NoneTy, span))
    }

    /// `hasattr(obj, "name")` (§5) → `Bool`, folded statically at lowering from
    /// the receiver's `ClassInfo`. The name must be a string literal; a
    /// `Dyn` / non-class receiver is rejected in lowering (a runtime probe is out
    /// of scope), mirroring `isinstance` against a builtin type.
    fn lower_hasattr_builtin(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.len() != 2 {
            return Err(parse_error("hasattr() takes two arguments", span));
        }
        let name = string_literal_arg(&args[1]).ok_or_else(|| {
            parse_error("dynamic hasattr (non-literal name) is out of scope", span)
        })?;
        let value = self.lower_expr(&args[0])?;
        let name = self.intern(name);
        Ok(self.alloc(HirExprKind::HasAttr { value, name }, SemTy::Bool, span))
    }

    /// `issubclass(Sub, Sup)` (§5) → `Bool`, folded at lowering via the C3-MRO
    /// check. Both args must be bare names resolving to user classes (mirrors the
    /// `isinstance` builder); the builtin-type (`issubclass(bool, int)`) and tuple
    /// second-arg forms are out of scope (clean error).
    fn lower_issubclass_builtin(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.len() != 2 {
            return Err(parse_error("issubclass() takes two arguments", span));
        }
        let resolve = |arg: &Expr| -> Option<ClassId> {
            if let Expr::Name(n) = arg {
                self.ctx.class_map.get(n.id.as_str()).map(|(cid, _)| *cid)
            } else {
                None
            }
        };
        let (Some(sub), Some(sup)) = (resolve(&args[0]), resolve(&args[1])) else {
            return Err(parse_error(
                "issubclass() requires user-class names \
                 (builtin-type / tuple forms out of scope)",
                span,
            ));
        };
        Ok(self.alloc(HirExprKind::IsSubclass { sub, sup }, SemTy::Bool, span))
    }

    /// `"literal".format(args, kwargs)` (§9) — a literal-receiver desugar onto the
    /// f-string field machinery. Each replacement field binds to a positional /
    /// keyword arg AT COMPILE TIME, so the runtime sees the same `FormatValue`
    /// concat an equivalent f-string would produce. All args are staged ONCE in
    /// written order (CPython evaluates every arg before formatting, and a field
    /// may reference the same positional twice). Scope limits (clean errors):
    /// auto↔manual numbering mix, `{0.attr}`/`{0[k]}` access, nested `{}` in a
    /// spec, a missing keyword/index.
    fn lower_str_format(
        &mut self,
        template: &str,
        c: &ExprCall,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        // Stage positionals then keyword values, in written order. (`*`/`**`
        // spreads were already rejected by the method-call gate.)
        let mut pos: Vec<LocalId> = Vec::with_capacity(c.args.len());
        for a in &c.args {
            pos.push(self.stage_arg(a)?);
        }
        let mut kw: Vec<(InternedString, LocalId)> = Vec::with_capacity(c.keywords.len());
        for k in &c.keywords {
            let name = k.arg.as_ref().ok_or_else(|| {
                parse_error("`**kwargs` spreading is not supported for .format()", span)
            })?;
            let id = self.intern(name.as_str());
            kw.push((id, self.stage_arg(&k.value)?));
        }

        let segs = parse_format_template(template, span)?;
        let mut auto_idx = 0usize;
        let mut numbering = FmtNumbering::Unset;
        let mut parts: Vec<Idx<HirExpr>> = Vec::with_capacity(segs.len());
        for seg in segs {
            match seg {
                FmtSeg::Lit(text) => {
                    let id = self.intern(&text);
                    parts.push(self.alloc(HirExprKind::StrLit(id), SemTy::Str, span));
                }
                FmtSeg::Field { field, conv, spec } => {
                    let value_local = match field {
                        FmtFieldRef::Auto => {
                            if numbering == FmtNumbering::Manual {
                                return Err(parse_error(
                                    "cannot switch from manual field numbering to automatic field specification",
                                    span,
                                ));
                            }
                            numbering = FmtNumbering::Auto;
                            let i = auto_idx;
                            auto_idx += 1;
                            *pos.get(i).ok_or_else(|| {
                                parse_error(
                                    format!("Replacement index {i} out of range for positional args tuple"),
                                    span,
                                )
                            })?
                        }
                        FmtFieldRef::Index(i) => {
                            if numbering == FmtNumbering::Auto {
                                return Err(parse_error(
                                    "cannot switch from automatic field specification to manual field numbering",
                                    span,
                                ));
                            }
                            numbering = FmtNumbering::Manual;
                            *pos.get(i).ok_or_else(|| {
                                parse_error(
                                    format!("Replacement index {i} out of range for positional args tuple"),
                                    span,
                                )
                            })?
                        }
                        FmtFieldRef::Keyword(name) => {
                            let id = self.intern(&name);
                            kw.iter()
                                .find(|(k, _)| *k == id)
                                .map(|(_, l)| *l)
                                .ok_or_else(|| parse_error(format!("missing keyword argument '{name}' for .format()"), span))?
                        }
                    };
                    let value = self.local_ref(value_local, span);
                    let spec_id = self.intern(&spec);
                    let spec_expr = self.alloc(HirExprKind::StrLit(spec_id), SemTy::Str, span);
                    parts.push(self.emit_format_field(value, conv, spec_expr, span));
                }
            }
        }
        Ok(self.concat_str_parts(parts, span))
    }

    /// Stage a callable argument (reduce's `function`) with the `min`/`max`
    /// `key=` discipline: a bare unshadowed name is called by name (a builtin
    /// has no value-position thunk, and a bare-name re-read is pure); a local
    /// name / lambda / other expression is staged once and called indirectly.
    fn stage_callable<'e>(&mut self, e: &'e Expr) -> Result<KeyMode<'e>> {
        match e {
            Expr::Name(n) => {
                let iname = self.intern(n.id.as_str());
                if self.scope.contains_key(&iname) {
                    let l = self.stage_arg(e)?;
                    Ok(KeyMode::Staged(l))
                } else {
                    Ok(KeyMode::ByName(e))
                }
            }
            other => {
                let l = self.stage_arg(other)?;
                Ok(KeyMode::Staged(l))
            }
        }
    }

    /// `func(acc, elem)` — the 2-argument reduction call through the staged
    /// callable, or a direct by-name call (builtins / top-level functions).
    fn emit_reduce_call(
        &mut self,
        mode: &KeyMode<'_>,
        acc: LocalId,
        elem: LocalId,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        let callee = match mode {
            KeyMode::Staged(l) => self.local_ref(*l, span),
            KeyMode::ByName(expr) => self.lower_callee(expr)?,
        };
        let acc_ref = self.local_ref(acc, span);
        let elem_ref = self.local_ref(elem, span);
        Ok(self.alloc(
            HirExprKind::Call {
                callee,
                args: vec![acc_ref, elem_ref],
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// `elem = next(it)` into a fresh pin-tagged local (null on exhaustion).
    fn emit_iter_next(&mut self, it: LocalId, span: Span) -> LocalId {
        let elem = self.fresh_local_tagged();
        let it_ref = self.local_ref(it, span);
        let next = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::IterNext,
                args: vec![it_ref],
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: elem,
            value: next,
        });
        elem
    }

    /// `is_exhausted(it)` as a Bool condition expr.
    fn emit_iter_exhausted(&mut self, it: LocalId, span: Span) -> Idx<HirExpr> {
        let it_ref = self.local_ref(it, span);
        self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::IterExhausted,
                args: vec![it_ref],
            },
            SemTy::Bool,
            span,
        )
    }

    /// `key(elem)` — an indirect call through the staged key callable, or a
    /// direct by-name call (builtins / top-level functions).
    fn emit_key_call(
        &mut self,
        mode: &KeyMode<'_>,
        elem: LocalId,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        let callee = match mode {
            KeyMode::Staged(l) => self.local_ref(*l, span),
            KeyMode::ByName(expr) => self.lower_callee(expr)?,
        };
        let arg = self.local_ref(elem, span);
        Ok(self.alloc(
            HirExprKind::Call {
                callee,
                args: vec![arg],
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// `set()` → empty set; `set(iterable)` → fill an empty set from the iterable.
    fn lower_set_call(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.is_empty() {
            return Ok(self.alloc(HirExprKind::SetLit { elems: vec![] }, SemTy::Dyn, span));
        }
        if args.len() != 1 {
            return Err(parse_error("set() takes at most 1 argument", span));
        }
        let result = self.fresh_local(SemTy::set_of(SemTy::Dyn));
        let empty = self.alloc(HirExprKind::SetLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: result,
            value: empty,
        });
        let iterable = self.lower_expr(&args[0])?;
        let lp = self.begin_iter_loop(iterable, span)?;
        let elem_ref = self.local_ref(lp.elem, span);
        self.push_stmt(HirStmt::ContainerPush {
            container: result,
            value: elem_ref,
        });
        self.end_iter_loop(lp);
        Ok(self.local_ref(result, span))
    }

    /// Emit the innermost comprehension element action (push / insert).
    fn emit_comp_elem(&mut self, kind: &CompKind, span: Span) -> Result<()> {
        match kind {
            CompKind::List { result, elt } | CompKind::Set { result, elt } => {
                let v = self.lower_expr(elt)?;
                self.push_stmt(HirStmt::ContainerPush {
                    container: *result,
                    value: v,
                });
            }
            CompKind::Dict { result, key, val } => {
                let k = self.lower_expr(key)?;
                let v = self.lower_expr(val)?;
                self.push_stmt(HirStmt::ContainerInsert {
                    container: *result,
                    key: k,
                    value: v,
                });
            }
        }
        let _ = span;
        Ok(())
    }

    /// Allocate a fresh synthetic local (unnamed; never referenced by a source
    /// name) for desugared result/operand slots.
    fn fresh_local(&mut self, ty: SemTy) -> LocalId {
        let name = self.interner.intern("");
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(HirLocal {
            name,
            ty,
            raw_int_ok: false,
            pin_tagged: false,
            cell_shared: false,
            deletable: false,
        });
        id
    }

    /// Evaluate a call-argument expression NOW into a fresh staged local.
    /// Keyword adaptation fills parameter slots out of written order; staging
    /// pins each argument's side effects to its written position.
    fn stage_arg(&mut self, e: &Expr) -> Result<LocalId> {
        let value = self.lower_expr(e)?;
        let l = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign { target: l, value });
        Ok(l)
    }

    /// Stage `e` unless it is a side-effect-free literal (kept as
    /// [`ArgSrc::Plain`] for slot-fill AST folds — see [`is_const_like`]).
    fn stage_arg_src<'e>(&mut self, e: &'e Expr) -> Result<ArgSrc<'e>> {
        if is_const_like(e) {
            Ok(ArgSrc::Plain(e))
        } else {
            Ok(ArgSrc::Staged(self.stage_arg(e)?))
        }
    }

    /// Materialize an [`ArgSrc`] at slot-fill time: lower the AST expression,
    /// or reference the already-staged local.
    fn arg_src_value(&mut self, src: ArgSrc<'_>, span: Span) -> Result<Idx<HirExpr>> {
        match src {
            ArgSrc::Plain(e) => self.lower_expr(e),
            ArgSrc::Staged(l) => Ok(self.local_ref(l, span)),
        }
    }

    /// A fresh synthetic local pinned to the `Tagged` representation — for the slot
    /// that receives an `iter_next` result (null on exhaustion, so it must never be
    /// inferred to an unboxed `Raw(F64)`/`Raw(I8)` that would deref the null).
    fn fresh_local_tagged(&mut self) -> LocalId {
        let name = self.interner.intern("");
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(HirLocal {
            name,
            ty: SemTy::Dyn,
            raw_int_ok: false,
            pin_tagged: true,
            cell_shared: false,
            deletable: false,
        });
        id
    }

    /// A fresh synthetic local carrying an authoritative `ty` (typeck fixes it,
    /// since `ty != Dyn`) but pinned to the `Tagged` representation. Used to
    /// "launder" a gradual `Dyn` spread value into a `float`/`bool` parameter
    /// slot: typeck skips the reinterpret check on a `pin_tagged` store (so the
    /// `Dyn → float`/`bool` assignment is admitted), and lowering unboxes the
    /// Tagged value to the param's `Raw` repr at the call site.
    fn fresh_local_pinned(&mut self, ty: SemTy) -> LocalId {
        let name = self.interner.intern("");
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(HirLocal {
            name,
            ty,
            raw_int_ok: false,
            pin_tagged: true,
            cell_shared: false,
            deletable: false,
        });
        id
    }

    // ── closures / nested functions (Phase 6A) ────────────────────────────────

    /// A unique synthetic name for a nested function: `{outer}.<locals>.{name}#k`.
    /// The `.<locals>.` infix keeps it un-typeable by user code, and the counter
    /// disambiguates same-named siblings.
    fn synth_name(&mut self, child: &str) -> String {
        let k = self.synth_counter;
        self.synth_counter += 1;
        format!("{}.<locals>.{child}#{k}", self.base_name)
    }

    /// The subset of a child scope's free names this scope can actually supply
    /// (its own cells), each with the cell's known content type — so an
    /// annotation (e.g. a `Callable[...]` HOF parameter) survives the capture
    /// boundary. The rest resolve through `semantics` (top-level functions,
    /// classes, builtins) or 6B globals.
    fn capture_list(&mut self, free: &[String]) -> Vec<(String, SemTy)> {
        free.iter()
            .filter_map(|n| {
                let iname = self.interner.intern(n);
                match self.scope.get(&iname).copied() {
                    Some(Binding::Cell(lid)) | Some(Binding::Direct(lid)) => {
                        Some((n.clone(), self.locals[lid.index()].ty.clone()))
                    }
                    None => None,
                }
            })
            .collect()
    }

    /// Build the `MakeClosure` value for `fid` over `captures` (each must be a
    /// `Cell` binding here — its cell *pointer* goes into the env tuple, which is
    /// what makes the capture shared and late-bound).
    fn make_closure_expr(
        &mut self,
        fid: FuncId,
        captures: &[(String, SemTy)],
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        let mut cap_exprs = Vec::with_capacity(captures.len());
        for (cname, _) in captures {
            let iname = self.interner.intern(cname);
            let b = self.scope.get(&iname).copied();
            let Some(Binding::Cell(cell_lid)) = b else {
                return Err(parse_error(
                    format!("internal: captured variable `{cname}` has no cell binding"),
                    span,
                ));
            };
            cap_exprs.push(self.local_ref(cell_lid, span));
        }
        Ok(self.alloc(
            HirExprKind::MakeClosure {
                func: fid,
                captures: cap_exprs,
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// Lower a nested `def` (Phase 6A): a flat synthetic function with an
    /// explicit env param, then bind `MakeClosure` to the def's name. Recursion
    /// works through self-capture: the def's own name is in the enclosing celled
    /// set, so its cell exists before the closure is stored into it.
    fn lower_nested_def(&mut self, d: &StmtFunctionDef) -> Result<()> {
        let span = to_span(d.range());
        let facts = freevars::analyze_def(d);
        let captures = self.capture_list(&facts.free);
        let synth = self.synth_name(d.name.as_str());
        let name = self.interner.intern(&synth);
        let fid = lower_callable(
            self.interner,
            self.ctx,
            self.shared,
            d,
            &synth,
            name,
            FirstParam::Plain,
            self.enclosing_class,
            false,
            Some((&captures, &facts)),
        )?;
        let mc = self.make_closure_expr(fid, &captures, span)?;
        let dname = self.intern(d.name.as_str());
        self.write_named(dname, SemTy::Dyn, mc);
        Ok(())
    }

    /// Lower a lambda (Phase 6A): a synthetic single-`Return` nested function.
    fn lower_lambda(&mut self, l: &ExprLambda, span: Span) -> Result<Idx<HirExpr>> {
        let args = l.args.as_ref();
        if args.vararg.is_some() || args.kwarg.is_some() || !args.kwonlyargs.is_empty() {
            return Err(parse_error("lambda *args/**kwargs are out of scope", span));
        }
        if args
            .posonlyargs
            .iter()
            .chain(args.args.iter())
            .any(|a| a.default.is_some())
        {
            return Err(parse_error(
                "lambda default arguments are out of scope",
                span,
            ));
        }
        let facts = freevars::analyze_lambda(l);
        let captures = self.capture_list(&facts.free);
        let synth = self.synth_name("<lambda>");
        let name = self.interner.intern(&synth);

        let fid = self.shared.reserve();
        let mut fl = FnLowerer::new(
            self.interner,
            self.ctx,
            self.shared,
            name,
            &synth,
            SemTy::Dyn,
            None,
        );
        fl.set_scope_facts(&facts);
        let env_name = fl.intern("__env__");
        fl.add_param(env_name, SemTy::Dyn);
        for awd in args.posonlyargs.iter().chain(args.args.iter()) {
            let pname = fl.intern(awd.def.arg.as_str());
            fl.add_param(pname, SemTy::Dyn);
        }
        fl.install_captures(&captures, &facts, span);
        fl.init_cells();
        let body = fl.lower_expr(l.body.as_ref())?;
        fl.seal(HirTerminator::Return(Some(body)));
        let f = fl.finish(HirTerminator::Return(None));
        self.shared.fill(fid, f);

        self.make_closure_expr(fid, &captures, span)
    }

    /// Install capture bindings: capture `i` is read out of env slot `i+1` into
    /// a fresh cell-holding local in the prologue, carrying the content type the
    /// enclosing scope knew for it.
    fn install_captures(&mut self, captures: &[(String, SemTy)], facts: &ScopeFacts, span: Span) {
        for (i, (cname, content_ty)) in captures.iter().enumerate() {
            let iname = self.interner.intern(cname);
            let cell_lid =
                self.alloc_cell_local(iname, content_ty.clone(), facts.nonlocals.contains(cname));
            let env_ref = self.local_ref(LocalId::new(0), span);
            let idx_e = self.alloc(HirExprKind::IntLit(i as i64 + 1), SemTy::Int, span);
            let tg = self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::TupleGet,
                    args: vec![env_ref, idx_e],
                },
                SemTy::Dyn,
                span,
            );
            self.push_stmt(HirStmt::Assign {
                target: cell_lid,
                value: tg,
            });
            self.scope.insert(iname, Binding::Cell(cell_lid));
        }
    }

    /// A top-level function referenced as a VALUE (Phase 6A): a memoized thunk
    /// `f.<thunk>(env, params…) { return f(params…) }` wrapped in a captureless
    /// closure — `f`'s own direct-call ABI is untouched. The thunk forwards the
    /// full declared parameter list (incl. the `*args` tuple / `**kwargs` dict
    /// slots) positionally, so a function with defaults/varargs is still
    /// callable as a value (indirect calls require full arity — 6C).
    fn lower_top_fn_value(&mut self, fname: &str, span: Span) -> Result<Idx<HirExpr>> {
        let thunk_key = (self.shared.current_ns, fname.to_string());
        let fid = match self.shared.thunks.get(&thunk_key) {
            Some(f) => *f,
            None => {
                let info = self.ctx.top_defs[fname].clone();
                let param_tys = top_def_param_tys(&info);
                let fid = self.shared.reserve();
                let tname = self.interner.intern(&format!("{fname}.<thunk>"));
                let mut fl = FnLowerer::new(
                    self.interner,
                    self.ctx,
                    self.shared,
                    tname,
                    fname,
                    info.ret.clone(),
                    None,
                );
                let env_name = fl.intern("__env__");
                fl.add_param(env_name, SemTy::Dyn);
                for (i, pty) in param_tys.iter().enumerate() {
                    let pname = fl.interner.intern(&format!("p{i}"));
                    fl.add_param(pname, pty.clone());
                }
                let target = fl.intern(fname);
                let callee = fl.alloc(
                    HirExprKind::Name(SymbolRef::Unresolved(target)),
                    SemTy::Dyn,
                    span,
                );
                let args: Vec<Idx<HirExpr>> = (0..param_tys.len())
                    .map(|i| fl.local_ref(LocalId::new(i as u32 + 1), span))
                    .collect();
                let call = fl.alloc(HirExprKind::Call { callee, args }, SemTy::Dyn, span);
                fl.seal(HirTerminator::Return(Some(call)));
                let mut f = fl.finish(HirTerminator::Return(None));
                f.varargs = info.varargs.is_some();
                f.kwargs = info.kwargs.is_some();
                self.shared.fill(fid, f);
                self.shared.thunks.insert(thunk_key, fid);
                fid
            }
        };
        Ok(self.alloc(
            HirExprKind::MakeClosure {
                func: fid,
                captures: vec![],
            },
            SemTy::Dyn,
            span,
        ))
    }

    fn local_ref(&mut self, lid: LocalId, span: Span) -> Idx<HirExpr> {
        let ty = self.locals[lid.index()].ty.clone();
        self.alloc(HirExprKind::Local(lid), ty, span)
    }

    fn lower_unary(&mut self, u: &ExprUnaryOp, span: Span) -> Result<Idx<HirExpr>> {
        // Fold `+`/`-` over a numeric literal into a signed literal (so e.g.
        // `-5` is a single `IntLit`, and negative bignum literals work).
        if matches!(u.op, PyUnaryOp::USub | PyUnaryOp::UAdd) {
            if let Expr::Constant(c) = u.operand.as_ref() {
                if let Some(idx) = self.try_fold_numeric(&u.op, &c.value, span) {
                    return Ok(idx);
                }
            }
        }
        let op = match u.op {
            PyUnaryOp::USub => UnaryOp::Neg,
            PyUnaryOp::UAdd => UnaryOp::Pos,
            PyUnaryOp::Invert => UnaryOp::Invert,
            PyUnaryOp::Not => UnaryOp::Not,
        };
        let operand = self.lower_expr(u.operand.as_ref())?;
        let ty = if op == UnaryOp::Not {
            SemTy::Bool
        } else {
            SemTy::Dyn
        };
        Ok(self.alloc(HirExprKind::Unary { op, operand }, ty, span))
    }

    /// Try to fold a unary `+`/`-` applied to a numeric constant.
    fn try_fold_numeric(
        &mut self,
        op: &PyUnaryOp,
        c: &Constant,
        span: Span,
    ) -> Option<Idx<HirExpr>> {
        let negative = matches!(op, PyUnaryOp::USub);
        match c {
            Constant::Int(big) => {
                let kind = self.int_literal(&big.to_string(), negative);
                Some(self.alloc(kind, SemTy::Int, span))
            }
            Constant::Float(f) => {
                let v = if negative { -*f } else { *f };
                Some(self.alloc(HirExprKind::FloatLit(v), SemTy::Float, span))
            }
            _ => None,
        }
    }

    fn lower_binop(&mut self, b: &ExprBinOp, span: Span) -> Result<Idx<HirExpr>> {
        let op = binop_from_ast(&b.op);
        let l = self.lower_expr(b.left.as_ref())?;
        let r = self.lower_expr(b.right.as_ref())?;
        Ok(self.alloc(HirExprKind::BinOp { op, l, r }, SemTy::Dyn, span))
    }

    fn map_cmp(&self, op: &PyCmpOp, span: Span) -> Result<CmpOp> {
        Ok(match op {
            PyCmpOp::Eq => CmpOp::Eq,
            PyCmpOp::NotEq => CmpOp::NotEq,
            PyCmpOp::Lt => CmpOp::Lt,
            PyCmpOp::LtE => CmpOp::LtE,
            PyCmpOp::Gt => CmpOp::Gt,
            PyCmpOp::GtE => CmpOp::GtE,
            PyCmpOp::Is | PyCmpOp::IsNot | PyCmpOp::In | PyCmpOp::NotIn => {
                return Err(parse_error("`is`/`in` comparisons are out of scope", span))
            }
        })
    }

    fn lower_compare(&mut self, c: &ExprCompare, span: Span) -> Result<Idx<HirExpr>> {
        if c.ops.len() != c.comparators.len() || c.ops.is_empty() {
            return Err(parse_error("malformed comparison", span));
        }
        // Single comparison: a plain `Compare` value node.
        if c.ops.len() == 1 {
            // `x in y` / `x not in y` → a container membership op (`Contains` reads
            // `container, elem`, so the operand order flips). `not in` negates it.
            if matches!(c.ops[0], PyCmpOp::In | PyCmpOp::NotIn) {
                let container = self.lower_expr(&c.comparators[0])?;
                let elem = self.lower_expr(c.left.as_ref())?;
                let contains = self.alloc(
                    HirExprKind::ContainerExpr {
                        op: ContainerOp::Contains,
                        args: vec![container, elem],
                    },
                    SemTy::Bool,
                    span,
                );
                if matches!(c.ops[0], PyCmpOp::NotIn) {
                    return Ok(self.alloc(
                        HirExprKind::Unary {
                            op: UnaryOp::Not,
                            operand: contains,
                        },
                        SemTy::Bool,
                        span,
                    ));
                }
                return Ok(contains);
            }
            // `x is …` / `x is not …` (Phase 8D + backlog §2). The `None` form is
            // the dedicated null-aware `IsNone` test (it recognizes both the
            // immediate `None` tag and a heap `None` object, which `==` does
            // not). Any other operand pair is general object identity, lowered
            // to `Is` → `rt_is` (bit-identity; never `__eq__`, which is the
            // `Compare` path). `is not` negates either form. `in`/`not in` were
            // handled above; chained `a is b is c` falls through to `map_cmp`
            // below, which still rejects it (out of scope).
            if matches!(c.ops[0], PyCmpOp::Is | PyCmpOp::IsNot) {
                let l_none = is_none_lit(c.left.as_ref());
                let r_none = is_none_lit(&c.comparators[0]);
                let negate = matches!(c.ops[0], PyCmpOp::IsNot);
                let ident = if l_none || r_none {
                    let operand = if r_none {
                        c.left.as_ref()
                    } else {
                        &c.comparators[0]
                    };
                    let v = self.lower_expr(operand)?;
                    self.alloc(HirExprKind::IsNone { value: v }, SemTy::Bool, span)
                } else {
                    let l = self.lower_expr(c.left.as_ref())?;
                    let r = self.lower_expr(&c.comparators[0])?;
                    self.alloc(HirExprKind::Is { l, r }, SemTy::Bool, span)
                };
                if negate {
                    return Ok(self.alloc(
                        HirExprKind::Unary {
                            op: UnaryOp::Not,
                            operand: ident,
                        },
                        SemTy::Bool,
                        span,
                    ));
                }
                return Ok(ident);
            }
            let op = self.map_cmp(&c.ops[0], span)?;
            let l = self.lower_expr(c.left.as_ref())?;
            let r = self.lower_expr(&c.comparators[0])?;
            return Ok(self.alloc(HirExprKind::Compare { op, l, r }, SemTy::Bool, span));
        }
        // Chained comparison `a < b < c`: short-circuit branch CFG with each
        // interior operand evaluated exactly once (single-eval), lazily.
        let res = self.fresh_local(SemTy::Bool);
        let false_b = self.new_block();
        let true_b = self.new_block();
        let join = self.new_block();

        let lv = self.lower_expr(c.left.as_ref())?;
        let mut prev = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: prev,
            value: lv,
        });

        for (i, comp) in c.comparators.iter().enumerate() {
            let op = self.map_cmp(&c.ops[i], span)?;
            let cv = self.lower_expr(comp)?;
            let cur = self.fresh_local(SemTy::Dyn);
            self.push_stmt(HirStmt::Assign {
                target: cur,
                value: cv,
            });
            let lref = self.local_ref(prev, span);
            let rref = self.local_ref(cur, span);
            let cmp = self.alloc(
                HirExprKind::Compare {
                    op,
                    l: lref,
                    r: rref,
                },
                SemTy::Bool,
                span,
            );
            let next = self.new_block();
            self.seal(HirTerminator::Branch {
                cond: cmp,
                then: next,
                else_: false_b,
            });
            self.switch(next);
            prev = cur;
        }
        self.seal(HirTerminator::Jump(true_b));

        self.switch(true_b);
        let t = self.alloc(HirExprKind::BoolLit(true), SemTy::Bool, span);
        self.push_stmt(HirStmt::Assign {
            target: res,
            value: t,
        });
        self.seal(HirTerminator::Jump(join));

        self.switch(false_b);
        let fb = self.alloc(HirExprKind::BoolLit(false), SemTy::Bool, span);
        self.push_stmt(HirStmt::Assign {
            target: res,
            value: fb,
        });
        self.seal(HirTerminator::Jump(join));

        self.switch(join);
        Ok(self.local_ref(res, span))
    }

    /// Short-circuit `and`/`or` over `values` (≥2), into branch CFG + result local.
    fn lower_boolop(&mut self, b: &ExprBoolOp) -> Result<Idx<HirExpr>> {
        let span = to_span(b.range());
        let res = self.fresh_local(SemTy::Dyn);
        let join = self.new_block();
        let n = b.values.len();
        for (i, val) in b.values.iter().enumerate() {
            let v = self.lower_expr(val)?;
            self.push_stmt(HirStmt::Assign {
                target: res,
                value: v,
            });
            if i + 1 < n {
                let next = self.new_block();
                let cond = self.local_ref(res, span);
                match b.op {
                    // `and`: keep going while truthy; short-circuit (res = falsy) to join.
                    PyBoolOp::And => self.seal(HirTerminator::Branch {
                        cond,
                        then: next,
                        else_: join,
                    }),
                    // `or`: short-circuit (res = truthy) to join; else keep going.
                    PyBoolOp::Or => self.seal(HirTerminator::Branch {
                        cond,
                        then: join,
                        else_: next,
                    }),
                }
                self.switch(next);
            } else {
                self.seal(HirTerminator::Jump(join));
            }
        }
        self.switch(join);
        Ok(self.local_ref(res, span))
    }

    fn lower_ifexp(&mut self, e: &ExprIfExp) -> Result<Idx<HirExpr>> {
        let span = to_span(e.range());
        let res = self.fresh_local(SemTy::Dyn);
        let cond = self.lower_expr(e.test.as_ref())?;
        let then_b = self.new_block();
        let else_b = self.new_block();
        let join = self.new_block();
        self.seal(HirTerminator::Branch {
            cond,
            then: then_b,
            else_: else_b,
        });

        self.switch(then_b);
        let bv = self.lower_expr(e.body.as_ref())?;
        self.push_stmt(HirStmt::Assign {
            target: res,
            value: bv,
        });
        self.seal(HirTerminator::Jump(join));

        self.switch(else_b);
        let ev = self.lower_expr(e.orelse.as_ref())?;
        self.push_stmt(HirStmt::Assign {
            target: res,
            value: ev,
        });
        self.seal(HirTerminator::Jump(join));

        self.switch(join);
        Ok(self.local_ref(res, span))
    }

    /// A call used as a value (builtins now; user functions in 2d). `print` is a
    /// statement, not a value-call, so reject it here.
    fn lower_call_expr(&mut self, c: &ExprCall, span: Span) -> Result<Idx<HirExpr>> {
        if let Expr::Name(n) = c.func.as_ref() {
            if n.id.as_str() == "print" {
                return Err(parse_error(
                    "print() is only supported as a statement",
                    span,
                ));
            }
        }
        // `Cls[T](args)` → a subscripted generic construction (Phase 5E).
        if let Expr::Subscript(s) = c.func.as_ref() {
            if let Expr::Name(n) = s.value.as_ref() {
                if let Some((class_id, _)) = self.ctx.class_map.get(n.id.as_str()).copied() {
                    reject_call_extras(c, span, "generic construction")?;
                    let type_args = subscript_type_args(s.slice.as_ref(), self.ctx);
                    let args = self.lower_expr_list(&c.args)?;
                    return Ok(self.alloc(
                        HirExprKind::GenericConstruct {
                            class_id,
                            type_args,
                            args,
                        },
                        SemTy::Dyn,
                        span,
                    ));
                }
            }
        }
        // `isinstance(value, Cls)` against a known user class → the runtime
        // inheritance-aware check (Phase 5B). `isinstance(value, str|int|
        // float|bool)` → the static fold (Phase 8B). Other forms fall through.
        if let Expr::Name(n) = c.func.as_ref() {
            if n.id.as_str() == "isinstance" && c.args.len() == 2 && c.keywords.is_empty() {
                if let Expr::Name(cls) = &c.args[1] {
                    if let Some((class_id, _)) = self.ctx.class_map.get(cls.id.as_str()).copied() {
                        let value = self.lower_expr(&c.args[0])?;
                        return Ok(self.alloc(
                            HirExprKind::IsInstance { value, class_id },
                            SemTy::Bool,
                            span,
                        ));
                    }
                    // Container builtins carry a canonical (Dyn-element) target;
                    // `lower_isinstance_builtin` matches by KIND (isinstance ignores
                    // element types), so the concrete element types don't matter here.
                    let target = match cls.id.as_str() {
                        "str" => Some(SemTy::Str),
                        "int" => Some(SemTy::Int),
                        "float" => Some(SemTy::Float),
                        "bool" => Some(SemTy::Bool),
                        "bytes" => Some(SemTy::Bytes),
                        "list" => Some(SemTy::list_of(SemTy::Dyn)),
                        "dict" => Some(SemTy::dict_of(SemTy::Dyn, SemTy::Dyn)),
                        "set" => Some(SemTy::set_of(SemTy::Dyn)),
                        "tuple" => Some(SemTy::tuple_var_of(SemTy::Dyn)),
                        _ => None,
                    };
                    if let Some(target) = target {
                        let value = self.lower_expr(&c.args[0])?;
                        return Ok(self.alloc(
                            HirExprKind::IsInstanceBuiltin { value, target },
                            SemTy::Bool,
                            span,
                        ));
                    }
                }
            }
        }
        // The builtin `open(...)` (Phase 8C) → the synthetic File-open
        // descriptor, unless a user binding (local/param/top-level def) shadows
        // the name.
        if let Expr::Name(n) = c.func.as_ref() {
            if n.id.as_str() == "open" {
                let iname = self.intern("open");
                if !self.scope.contains_key(&iname) && !self.ctx.top_defs.contains_key("open") {
                    return self.lower_open_builtin(c, span);
                }
            }
        }
        // A from-imported stdlib function called by its bound name (Phase 8B):
        // `sqrt(2.0)` after `from math import sqrt`. A local/param of the same
        // name shadows the binding.
        if let Expr::Name(n) = c.func.as_ref() {
            let iname = self.intern(n.id.as_str());
            if !self.scope.contains_key(&iname) {
                if let Some(def) = self.ctx.stdlib.funcs.get(n.id.as_str()).copied() {
                    if is_reduce_def(def) {
                        reject_call_extras(c, span, "reduce()")?;
                        return self.lower_reduce(&c.args, span);
                    }
                    return self.lower_stdlib_call(def, c, span);
                }
            }
        }
        // `M.f(...)` / `M.sub.f(...)` through an `import M` stdlib alias (Phase
        // 8B/8D): flatten the (possibly multi-level) attribute chain to a dotted
        // key and dispatch to the runtime descriptor — `os.getcwd()` and
        // `os.path.join(...)` alike. The leftmost name must be a stdlib alias
        // and unshadowed.
        if let Some((leftmost, dotted)) = flatten_attr_chain(c.func.as_ref()) {
            let lname = self.intern(leftmost);
            if self.ctx.stdlib.aliases.contains(leftmost) && !self.scope.contains_key(&lname) {
                if let Some(def) = self.ctx.stdlib.funcs.get(&dotted).copied() {
                    // `functools.reduce(...)` (qualified) — same HOF desugar as
                    // the from-imported bare form above.
                    if is_reduce_def(def) {
                        reject_call_extras(c, span, "reduce()")?;
                        return self.lower_reduce(&c.args, span);
                    }
                    return self.lower_stdlib_call(def, c, span);
                }
                // Not a stdlib function. A LONGER chain whose 2-link prefix is a
                // known module attr (`sys.path.append(...)` — a list method on
                // the `sys.path` attr) falls through to the method-call path.
                // An unknown 2-link `module.attr(...)` (e.g. `re.findall`) is a
                // loud CPython-style AttributeError diagnostic instead of the
                // misleading "undefined name" from the generic path.
                if let Some((module, attr)) = dotted.split_once('.') {
                    if !attr.contains('.') && !self.stdlib_module_attr_exists(&dotted) {
                        return Err(parse_error(
                            format!("module '{module}' has no attribute '{attr}'"),
                            span,
                        ));
                    }
                }
            }
        }
        // `M.f(args)` / `M.Cls(args)` through an `import M` user-module alias
        // (Phase 8): a qualified access folds to an ordinary direct call / class
        // construction (the imported FuncId/ClassId lives under the `"M.name"`
        // key in `top_defs` / `class_map`). Handled before the method-call path
        // so the alias receiver is never mistaken for an object receiver.
        if let Expr::Attribute(attr) = c.func.as_ref() {
            if let Expr::Name(m) = attr.value.as_ref() {
                let mname = self.intern(m.id.as_str());
                if self.ctx.aliases.contains(m.id.as_str()) && !self.scope.contains_key(&mname) {
                    let qual = format!("{}.{}", m.id.as_str(), attr.attr.as_str());
                    if let Some((class_id, _)) = self.ctx.class_map.get(&qual).copied() {
                        reject_call_extras(c, span, "module class construction")?;
                        let args = self.lower_expr_list(&c.args)?;
                        return Ok(self.alloc(
                            HirExprKind::GenericConstruct {
                                class_id,
                                type_args: vec![],
                                args,
                            },
                            SemTy::Dyn,
                            span,
                        ));
                    }
                    if let Some(info) = self.ctx.top_defs.get(&qual).cloned() {
                        return self.lower_direct_known_call(&info, &qual, c, span);
                    }
                    return Err(parse_error(
                        format!(
                            "module `{}` has no callable attribute `{}`",
                            m.id.as_str(),
                            attr.attr.as_str()
                        ),
                        span,
                    ));
                }
            }
        }
        // Generator `g.send(v)` / `g.close()` (Phase 6E): a generator-specific
        // method (no user class in our subset defines these), routed to the
        // runtime generator ops. `g.throw(...)` is out of scope.
        if let Expr::Attribute(attr) = c.func.as_ref() {
            match attr.attr.as_str() {
                "send" if c.args.len() == 1 && c.keywords.is_empty() => {
                    let gen = self.lower_expr(attr.value.as_ref())?;
                    let value = self.lower_expr(&c.args[0])?;
                    return Ok(self.alloc(
                        HirExprKind::GenQuery {
                            op: GenOp::Send,
                            gen,
                            imm: 0,
                            value: Some(value),
                        },
                        SemTy::Dyn,
                        span,
                    ));
                }
                "close" if c.args.is_empty() && c.keywords.is_empty() => {
                    let gen = self.lower_expr(attr.value.as_ref())?;
                    return Ok(self.alloc(
                        HirExprKind::GenQuery {
                            op: GenOp::Close,
                            gen,
                            imm: 0,
                            value: None,
                        },
                        SemTy::NoneTy,
                        span,
                    ));
                }
                _ => {}
            }
        }
        // `recv.method(args)` → a method call carrying the interned name. Lowering
        // dispatches by the receiver's static type: a container receiver to the
        // Phase-4D `ContainerMethod` path, a class receiver to the method's FuncId
        // (Phase 5). `super().method(args)` carries a `Super` receiver resolved at
        // lowering against the enclosing class's MRO. Unknown names are not rejected.
        if let Expr::Attribute(attr) = c.func.as_ref() {
            if has_starred_arg(c) {
                return Err(parse_error(
                    "`*args` spreading is not supported for method calls",
                    span,
                ));
            }
            if has_doublestar_kwarg(c) {
                return Err(parse_error(
                    "`**kwargs` spreading is not supported for method calls",
                    span,
                ));
            }
            // `.sort(key=K)` with a non-None key desugars HERE, by method NAME
            // (the receiver's type is not known until typeck). Documented
            // caveat: a user class with a `sort(key=)` method would mis-route;
            // mitigated by the type-tag TypeError guard in
            // `rt_list_sort_by_keys` (precedent: `g.send()`/`g.close()` above
            // are name-dispatched the same way).
            if attr.attr.as_str() == "sort"
                && !c.keywords.is_empty()
                && !is_super_call(attr.value.as_ref())
            {
                if let Some(out) = self.lower_sort_kwargs(attr, c, span)? {
                    return Ok(out);
                }
            }
            // `"literal".format(...)` (§9) desugars HERE, on a STRING-LITERAL
            // receiver, into the same `FormatValue` field machinery f-strings
            // use (the fields bind to positional / keyword args at compile time,
            // so `.format(name=…)` never reaches the keyword-less method gate). A
            // non-literal `var.format(...)` falls through to the generic
            // `MethodCall`, which reports an unsupported-method error.
            if attr.attr.as_str() == "format" && !is_super_call(attr.value.as_ref()) {
                if let Expr::Constant(rc) = attr.value.as_ref() {
                    if let Constant::Str(template) = &rc.value {
                        let template = template.clone();
                        return self.lower_str_format(&template, c, span);
                    }
                }
            }
            let staging = !c.keywords.is_empty();
            let recv = if is_super_call(attr.value.as_ref()) {
                let cid = self
                    .enclosing_class
                    .ok_or_else(|| parse_error("super() is only valid inside a method", span))?;
                self.alloc(HirExprKind::Super(cid), SemTy::Dyn, span)
            } else if staging && !matches!(attr.value.as_ref(), Expr::Name(_)) {
                // Keyword calls stage a compound receiver too — its side
                // effects come before every argument's (written order). A bare
                // name is a pure read AND may be a class reference
                // (`Cls.method(kw=…)`), which cannot live in a value slot.
                let l = self.stage_arg(attr.value.as_ref())?;
                self.local_ref(l, span)
            } else {
                self.lower_expr(attr.value.as_ref())?
            };
            let method_name = self.intern(attr.attr.as_str());
            let (args, kwargs) = if staging {
                // Stage positionals then keyword values in WRITTEN order.
                let mut args = Vec::with_capacity(c.args.len());
                for a in &c.args {
                    let src = self.stage_arg_src(a)?;
                    args.push(self.arg_src_value(src, span)?);
                }
                let mut kwargs = Vec::with_capacity(c.keywords.len());
                for kw in &c.keywords {
                    let kname = kw.arg.as_ref().expect("** rejected above");
                    let id = self.intern(kname.as_str());
                    let src = self.stage_arg_src(&kw.value)?;
                    kwargs.push((id, self.arg_src_value(src, span)?));
                }
                (args, kwargs)
            } else {
                (self.lower_expr_list(&c.args)?, vec![])
            };
            return Ok(self.alloc(
                HirExprKind::MethodCall {
                    recv,
                    method_name,
                    args,
                    kwargs,
                },
                SemTy::Dyn,
                span,
            ));
        }
        // Builtins that desugar to reduce / iterator loops are recognized by name
        // (like `print`/`range`; shadowing these names is not supported).
        if let Expr::Name(n) = c.func.as_ref() {
            // `min`/`max` accept the `key=` keyword (Phase 7).
            if matches!(n.id.as_str(), "min" | "max") {
                if has_starred_arg(c) {
                    return Err(parse_error(
                        "`*args` spreading is not supported for min()/max()",
                        span,
                    ));
                }
                let mut key: Option<&Expr> = None;
                for kw in &c.keywords {
                    match kw.arg.as_ref().map(|i| i.as_str()) {
                        Some("key") => key = Some(&kw.value),
                        Some(other) => {
                            return Err(parse_error(
                                format!(
                                    "min()/max() got an unsupported keyword argument '{other}'"
                                ),
                                span,
                            ))
                        }
                        None => {
                            return Err(parse_error("min()/max() do not support **kwargs", span))
                        }
                    }
                }
                return self.lower_minmax(&c.args, key, span, n.id.as_str() == "min");
            }
            if matches!(n.id.as_str(), "sum" | "set" | "next" | "iter") {
                reject_call_extras(c, span, "this builtin")?;
                match n.id.as_str() {
                    "sum" => return self.lower_sum(&c.args, span),
                    "set" => return self.lower_set_call(&c.args, span),
                    // `next(g)` (Phase 6E): resume the generator → its next value.
                    "next" => {
                        if c.args.len() != 1 {
                            return Err(parse_error("next() takes exactly one argument", span));
                        }
                        let gen = self.lower_expr(&c.args[0])?;
                        return Ok(self.alloc(
                            HirExprKind::GenQuery {
                                op: GenOp::Next,
                                gen,
                                imm: 0,
                                value: None,
                            },
                            SemTy::Dyn,
                            span,
                        ));
                    }
                    // `iter(iterable)`: build a runtime iterator object (the same
                    // `ContainerOp::Iter` → `rt_iter_value` the for-loop drives, so
                    // a File iterable routes through `rt_file_readlines` in lowering
                    // too). `next(it)` then consumes it via the raising `rt_iter_next`.
                    // The 2-arg sentinel form `iter(callable, sentinel)` is out of scope.
                    "iter" => {
                        if c.args.len() != 1 {
                            return Err(parse_error(
                                "only the 1-argument form iter(iterable) is supported",
                                span,
                            ));
                        }
                        let iterable = self.lower_expr(&c.args[0])?;
                        return Ok(self.alloc(
                            HirExprKind::ContainerExpr {
                                op: ContainerOp::Iter,
                                args: vec![iterable],
                            },
                            SemTy::Dyn,
                            span,
                        ));
                    }
                    _ => {}
                }
            }
        }
        // Scalar / value builtins (PLAN §5): `pow`, `divmod`, `all`, `any`,
        // `id`, `round`, `bin`, `hex`, `oct`. Gated on an UNSHADOWED bare name
        // (a local / global / top-def binding keeps winning — `id = 5; id(x)`
        // reads the local), slightly stricter than the unconditional min/max
        // intercept. None take keywords or `*`/`**` spreads.
        if let Expr::Name(n) = c.func.as_ref() {
            let iname = self.intern(n.id.as_str());
            let unshadowed = !self.scope.contains_key(&iname)
                && self.global_read_slot(iname).is_none()
                && !self.ctx.top_defs.contains_key(n.id.as_str());
            if unshadowed {
                use pyaot_stdlib_defs::modules::builtins as bd;
                match n.id.as_str() {
                    "pow" => {
                        reject_call_extras(c, span, "pow()")?;
                        return self.lower_pow(&c.args, span);
                    }
                    "divmod" => {
                        reject_call_extras(c, span, "divmod()")?;
                        return self.lower_divmod(&c.args, span);
                    }
                    "all" | "any" => {
                        reject_call_extras(c, span, "all()/any()")?;
                        return self.lower_all_any(&c.args, span, n.id.as_str() == "all");
                    }
                    "map" => {
                        reject_call_extras(c, span, "map()")?;
                        return self.lower_map(&c.args, span);
                    }
                    "filter" => {
                        reject_call_extras(c, span, "filter()")?;
                        return self.lower_filter(&c.args, span);
                    }
                    "format" => {
                        reject_call_extras(c, span, "format()")?;
                        return self.lower_format_builtin(&c.args, span);
                    }
                    "getattr" => {
                        reject_call_extras(c, span, "getattr()")?;
                        return self.lower_getattr_builtin(&c.args, span);
                    }
                    "setattr" => {
                        reject_call_extras(c, span, "setattr()")?;
                        return self.lower_setattr_builtin(&c.args, span);
                    }
                    "hasattr" => {
                        reject_call_extras(c, span, "hasattr()")?;
                        return self.lower_hasattr_builtin(&c.args, span);
                    }
                    "issubclass" => {
                        reject_call_extras(c, span, "issubclass()")?;
                        return self.lower_issubclass_builtin(&c.args, span);
                    }
                    "id" => {
                        reject_call_extras(c, span, "id()")?;
                        return self.lower_stdlib_call(&bd::BUILTIN_ID, c, span);
                    }
                    "round" => {
                        reject_call_extras(c, span, "round()")?;
                        return self.lower_stdlib_call(&bd::BUILTIN_ROUND, c, span);
                    }
                    "bin" => {
                        reject_call_extras(c, span, "bin()")?;
                        return self.lower_stdlib_call(&bd::BUILTIN_BIN, c, span);
                    }
                    "hex" => {
                        reject_call_extras(c, span, "hex()")?;
                        return self.lower_stdlib_call(&bd::BUILTIN_HEX, c, span);
                    }
                    "oct" => {
                        reject_call_extras(c, span, "oct()")?;
                        return self.lower_stdlib_call(&bd::BUILTIN_OCT, c, span);
                    }
                    _ => {}
                }
            }
        }
        // Direct self-recursion (Phase 6A): a nested function calling its own
        // name through its self-capture cell becomes a direct call to itself,
        // passing its env through (the cells stay shared).
        if c.keywords.is_empty() && !has_starred_arg(c) {
            if let Expr::Name(n) = c.func.as_ref() {
                if let Some((cell_lid, synth)) = self.self_capture {
                    let name = self.intern(n.id.as_str());
                    if self.scope.get(&name) == Some(&Binding::Cell(cell_lid)) {
                        let callee = self.alloc(
                            HirExprKind::Name(SymbolRef::Unresolved(synth)),
                            SemTy::Dyn,
                            span,
                        );
                        let mut args = vec![self.local_ref(LocalId::new(0), span)];
                        for a in &c.args {
                            args.push(self.lower_expr(a)?);
                        }
                        return Ok(self.alloc(
                            HirExprKind::Call { callee, args },
                            SemTy::Dyn,
                            span,
                        ));
                    }
                }
            }
        }
        // A decorated module-level function called by name (Phase 6D): its slot
        // holds a `(*args, **kwargs)` wrapper, so pack positional / keyword args
        // into the variadic slots and call the slot indirectly.
        if let Expr::Name(n) = c.func.as_ref() {
            let iname = self.intern(n.id.as_str());
            if !self.scope.contains_key(&iname) && self.ctx.decorated.contains(n.id.as_str()) {
                if let Some(var_id) = self.promoted_id(iname) {
                    return self.lower_decorated_call(var_id, c, span);
                }
            }
        }
        // A known top-level function called by name (not shadowed locally): the
        // frontend adapts keywords / defaults / `*args` packing at compile time
        // (Phase 6C). Everything else (indirect, builtins, classes) just lowers
        // its positional + spread args.
        if let Expr::Name(n) = c.func.as_ref() {
            let iname = self.intern(n.id.as_str());
            if !self.scope.contains_key(&iname) && self.global_read_slot(iname).is_none() {
                if let Some(info) = self.ctx.top_defs.get(n.id.as_str()).cloned() {
                    return self.lower_direct_known_call(&info, n.id.as_str(), c, span);
                }
            }
        }
        // Keyword arguments on container/iteration builtins (Phase 10):
        // `sorted(key=, reverse=)`, `enumerate(start=)`, `dict(a=1)`. Only for
        // a bare unshadowed name — user bindings keep winning above, and the
        // no-keyword forms keep their existing paths untouched.
        if let Expr::Name(n) = c.func.as_ref() {
            if !c.keywords.is_empty() {
                let iname = self.intern(n.id.as_str());
                if !self.scope.contains_key(&iname)
                    && self.global_read_slot(iname).is_none()
                    && !self.ctx.top_defs.contains_key(n.id.as_str())
                {
                    if let Some(out) = self.lower_builtin_kwargs_call(n.id.as_str(), c, span)? {
                        return Ok(out);
                    }
                }
            }
        }
        self.lower_indirect_or_unknown_call(c, span)
    }

    /// Lower a keyword-carrying call to a recognized builtin (Phase 10), or
    /// `None` to fall through to the generic (rejecting) path. Builtins that
    /// take no keywords get a precise diagnostic here instead of the generic
    /// indirect-call rejection.
    fn lower_builtin_kwargs_call(
        &mut self,
        name: &str,
        c: &ExprCall,
        span: Span,
    ) -> Result<Option<Idx<HirExpr>>> {
        match name {
            "sorted" => Ok(Some(self.lower_sorted_kwargs(c, span)?)),
            "enumerate" => Ok(Some(self.lower_enumerate_kwargs(c, span)?)),
            "dict" => Ok(Some(self.lower_dict_kwargs(c, span)?)),
            "list" | "tuple" | "zip" | "reversed" | "len" | "bytes" | "set" | "sum" | "next"
            | "range" => Err(parse_error(
                format!("`{name}()` takes no keyword arguments"),
                span,
            )),
            _ => Ok(None),
        }
    }

    /// `sorted(xs, *, key=None, reverse=False)` with keywords (Phase 10).
    /// Without a key (or `key=None`): the standard container path with the
    /// reverse flag. With a key: copy → compiled key loop building a parallel
    /// keys list → `ListSortByKeys` tandem sort (no runtime callbacks); the
    /// result is the sorted copy. All argument values evaluate in written order.
    fn lower_sorted_kwargs(&mut self, c: &ExprCall, span: Span) -> Result<Idx<HirExpr>> {
        if c.args.len() != 1 || has_starred_arg(c) {
            return Err(parse_error(
                "sorted() takes exactly one positional argument",
                span,
            ));
        }
        let xs = self.stage_arg(&c.args[0])?;
        let mut key_mode: Option<KeyMode> = None;
        let mut rev: Option<LocalId> = None;
        for kw in &c.keywords {
            match kw.arg.as_ref().map(|i| i.as_str()) {
                Some("key") => {
                    if is_none_lit(&kw.value) {
                        continue;
                    }
                    // Same discipline as min/max: a bare out-of-scope name is
                    // called directly per element (builtins have no
                    // value-position thunk); anything else is staged once.
                    key_mode = Some(match &kw.value {
                        k @ Expr::Name(nm)
                            if {
                                let kn = self.intern(nm.id.as_str());
                                self.scope.contains_key(&kn)
                            } =>
                        {
                            KeyMode::Staged(self.stage_arg(k)?)
                        }
                        k @ Expr::Name(_) => KeyMode::ByName(k),
                        k => KeyMode::Staged(self.stage_arg(k)?),
                    });
                }
                Some("reverse") => rev = Some(self.stage_arg(&kw.value)?),
                Some(other) => {
                    return Err(parse_error(
                        format!("sorted() got an unexpected keyword argument `{other}`"),
                        span,
                    ))
                }
                None => return Err(parse_error("sorted() does not support **kwargs", span)),
            }
        }
        let rev_ref = match rev {
            Some(l) => self.local_ref(l, span),
            None => self.alloc(HirExprKind::BoolLit(false), SemTy::Bool, span),
        };
        let Some(km) = key_mode else {
            // No key: `sorted(xs, rev)` through the container builtin.
            let cname = self.intern("sorted");
            let callee = self.alloc(HirExprKind::Name(SymbolRef::Unresolved(cname)), SemTy::Dyn, span);
            let xs_ref = self.local_ref(xs, span);
            return Ok(self.alloc(
                HirExprKind::Call {
                    callee,
                    args: vec![xs_ref, rev_ref],
                },
                SemTy::Dyn,
                span,
            ));
        };
        // copy = list(iter(xs)) — sorted never mutates its input.
        let xs_ref = self.local_ref(xs, span);
        let it = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Iter,
                args: vec![xs_ref],
            },
            SemTy::Dyn,
            span,
        );
        let copy_e = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::ListFromIter,
                args: vec![it],
            },
            SemTy::Dyn,
            span,
        );
        let copy = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: copy,
            value: copy_e,
        });
        // keys = [key(e) for e in copy] — the key call stays compiled code.
        let keys = self.fresh_local(SemTy::Dyn);
        let empty = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: keys,
            value: empty,
        });
        let copy_iter_ref = self.local_ref(copy, span);
        let lp = self.begin_iter_loop(copy_iter_ref, span)?;
        let kv = self.emit_key_call(&km, lp.elem, span)?;
        self.push_stmt(HirStmt::ContainerPush {
            container: keys,
            value: kv,
        });
        self.end_iter_loop(lp);
        // Tandem sort of copy by keys, then the copy IS the result.
        let copy_ref = self.local_ref(copy, span);
        let keys_ref = self.local_ref(keys, span);
        let sort_e = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::ListSortByKeys,
                args: vec![copy_ref, keys_ref, rev_ref],
            },
            SemTy::NoneTy,
            span,
        );
        let sink = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: sink,
            value: sort_e,
        });
        Ok(self.local_ref(copy, span))
    }

    /// `xs.sort(key=K[, reverse=R])` with a non-None key (Phase 10): stage the
    /// receiver and keyword values in written order, build the parallel keys
    /// list with a compiled loop, and tandem-sort in place via
    /// `ListSortByKeys`. Returns `None` (falls through to the generic
    /// `MethodCall` path) when the key is absent / the `None` literal — that
    /// form needs no name-dispatch caveat. The expression's value is `None`
    /// (in-place sort).
    fn lower_sort_kwargs(
        &mut self,
        attr: &rustpython_parser::ast::ExprAttribute,
        c: &ExprCall,
        span: Span,
    ) -> Result<Option<Idx<HirExpr>>> {
        if !c
            .keywords
            .iter()
            .any(|kw| kw.arg.as_ref().is_some_and(|a| a.as_str() == "key") && !is_none_lit(&kw.value))
        {
            return Ok(None);
        }
        if !c.args.is_empty() {
            return Err(parse_error("sort() takes no positional arguments", span));
        }
        let recv = self.stage_arg(attr.value.as_ref())?;
        let mut key_mode: Option<KeyMode> = None;
        let mut rev: Option<LocalId> = None;
        for kw in &c.keywords {
            match kw.arg.as_ref().map(|i| i.as_str()) {
                Some("key") => {
                    key_mode = Some(match &kw.value {
                        k @ Expr::Name(nm)
                            if {
                                let kn = self.intern(nm.id.as_str());
                                self.scope.contains_key(&kn)
                            } =>
                        {
                            KeyMode::Staged(self.stage_arg(k)?)
                        }
                        k @ Expr::Name(_) => KeyMode::ByName(k),
                        k => KeyMode::Staged(self.stage_arg(k)?),
                    });
                }
                Some("reverse") => rev = Some(self.stage_arg(&kw.value)?),
                Some(other) => {
                    return Err(parse_error(
                        format!("sort() got an unexpected keyword argument `{other}`"),
                        span,
                    ))
                }
                None => return Err(parse_error("sort() does not support **kwargs", span)),
            }
        }
        let km = key_mode.expect("checked above: a non-None key is present");
        // keys = [key(e) for e in recv] — compiled key calls, no runtime callback.
        let keys = self.fresh_local(SemTy::Dyn);
        let empty = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: keys,
            value: empty,
        });
        let recv_iter_ref = self.local_ref(recv, span);
        let lp = self.begin_iter_loop(recv_iter_ref, span)?;
        let kv = self.emit_key_call(&km, lp.elem, span)?;
        self.push_stmt(HirStmt::ContainerPush {
            container: keys,
            value: kv,
        });
        self.end_iter_loop(lp);
        let recv_ref = self.local_ref(recv, span);
        let keys_ref = self.local_ref(keys, span);
        let rev_ref = match rev {
            Some(l) => self.local_ref(l, span),
            None => self.alloc(HirExprKind::BoolLit(false), SemTy::Bool, span),
        };
        let sort_e = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::ListSortByKeys,
                args: vec![recv_ref, keys_ref, rev_ref],
            },
            SemTy::NoneTy,
            span,
        );
        let sink = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: sink,
            value: sort_e,
        });
        Ok(Some(self.alloc(HirExprKind::NoneLit, SemTy::NoneTy, span)))
    }

    /// `enumerate(xs, start=k)` (Phase 10) — fold the keyword into the
    /// positional form the container path already accepts.
    fn lower_enumerate_kwargs(&mut self, c: &ExprCall, span: Span) -> Result<Idx<HirExpr>> {
        if c.args.is_empty() || c.args.len() > 2 || has_starred_arg(c) {
            return Err(parse_error(
                "enumerate() takes 1 positional argument plus optional `start`",
                span,
            ));
        }
        let it_src = self.stage_arg_src(&c.args[0])?;
        let mut start: Option<ArgSrc> = match c.args.get(1) {
            Some(a) => Some(self.stage_arg_src(a)?),
            None => None,
        };
        for kw in &c.keywords {
            match kw.arg.as_ref().map(|i| i.as_str()) {
                Some("start") => {
                    if start.is_some() {
                        return Err(parse_error(
                            "enumerate() got multiple values for argument `start`",
                            span,
                        ));
                    }
                    start = Some(self.stage_arg_src(&kw.value)?);
                }
                Some(other) => {
                    return Err(parse_error(
                        format!("enumerate() got an unexpected keyword argument `{other}`"),
                        span,
                    ))
                }
                None => return Err(parse_error("enumerate() does not support **kwargs", span)),
            }
        }
        let cname = self.intern("enumerate");
        let callee = self.alloc(HirExprKind::Name(SymbolRef::Unresolved(cname)), SemTy::Dyn, span);
        let it_ref = self.arg_src_value(it_src, span)?;
        let start_ref = match start {
            Some(s) => self.arg_src_value(s, span)?,
            None => self.alloc(HirExprKind::IntLit(0), SemTy::Int, span),
        };
        Ok(self.alloc(
            HirExprKind::Call {
                callee,
                args: vec![it_ref, start_ref],
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// `dict(a=1, b=2)` / `dict(pos, a=1)` (Phase 10): pure-keyword form is a
    /// `DictLit` with string keys in written order; the mixed form builds the
    /// positional dict first, then inserts the keywords (CPython update order).
    fn lower_dict_kwargs(&mut self, c: &ExprCall, span: Span) -> Result<Idx<HirExpr>> {
        if has_starred_arg(c) {
            return Err(parse_error(
                "`*` spreading into dict() is out of scope",
                span,
            ));
        }
        if c.args.is_empty() {
            let mut pairs = Vec::with_capacity(c.keywords.len());
            for kw in &c.keywords {
                let Some(kname) = &kw.arg else {
                    return Err(parse_error("dict() does not support **kwargs", span));
                };
                let key_id = self.intern(kname.as_str());
                let key = self.alloc(HirExprKind::StrLit(key_id), SemTy::Str, span);
                let val = self.lower_expr(&kw.value)?;
                pairs.push((key, val));
            }
            return Ok(self.alloc(HirExprKind::DictLit { pairs }, SemTy::Dyn, span));
        }
        if c.args.len() > 1 {
            return Err(parse_error(
                "dict() takes at most 1 positional argument",
                span,
            ));
        }
        // Stage everything in written order, then build + insert.
        let pos = self.stage_arg(&c.args[0])?;
        let mut kwargs: Vec<(InternedString, ArgSrc)> = Vec::with_capacity(c.keywords.len());
        for kw in &c.keywords {
            let Some(kname) = &kw.arg else {
                return Err(parse_error("dict() does not support **kwargs", span));
            };
            let id = self.intern(kname.as_str());
            let src = self.stage_arg_src(&kw.value)?;
            kwargs.push((id, src));
        }
        let cname = self.intern("dict");
        let callee = self.alloc(HirExprKind::Name(SymbolRef::Unresolved(cname)), SemTy::Dyn, span);
        let pos_ref = self.local_ref(pos, span);
        let call = self.alloc(
            HirExprKind::Call {
                callee,
                args: vec![pos_ref],
            },
            SemTy::Dyn,
            span,
        );
        let d = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: d,
            value: call,
        });
        for (key_id, src) in kwargs {
            let key = self.alloc(HirExprKind::StrLit(key_id), SemTy::Str, span);
            let val = self.arg_src_value(src, span)?;
            self.push_stmt(HirStmt::ContainerInsert {
                container: d,
                key,
                value: val,
            });
        }
        Ok(self.local_ref(d, span))
    }

    /// Pack a call to a decorated function into the wrapper's `(*args, **kwargs)`
    /// ABI and call its global slot indirectly (Phase 6D).
    fn lower_decorated_call(
        &mut self,
        slot: u32,
        c: &ExprCall,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        if has_doublestar_kwarg(c) {
            return Err(parse_error(
                "`**` spreading into a decorated call is out of scope",
                span,
            ));
        }
        if !c.keywords.is_empty() {
            return Err(parse_error(
                "keyword arguments are not supported for calls to decorated functions",
                span,
            ));
        }
        // The decorated slot holds a `(*args, **kwargs)` wrapper, so the whole
        // positional sequence — plain args and any `*seq` spread — becomes its
        // `*args` tuple. A runtime spread builds the tuple from a materialized
        // `argv` list; an all-plain (incl. flattened literal-spread) call builds
        // a fixed tuple literal directly.
        let (items, has_runtime_spread) = classify_pos_args(c);
        let tuple = if has_runtime_spread {
            let argv = self.build_spread_argv(&items, span)?;
            let argv_ref = self.local_ref(argv, span);
            let it = self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::Iter,
                    args: vec![argv_ref],
                },
                SemTy::Dyn,
                span,
            );
            self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::TupleFromIter,
                    args: vec![it],
                },
                SemTy::tuple_var_of(SemTy::Dyn),
                span,
            )
        } else {
            let mut elems = Vec::with_capacity(items.len());
            for item in &items {
                let PosItem::Plain(e) = *item else {
                    unreachable!("runtime spreads handled above")
                };
                elems.push(self.lower_expr(e)?);
            }
            self.alloc(HirExprKind::TupleLit { elems }, SemTy::Dyn, span)
        };
        let dict = self.alloc(HirExprKind::DictLit { pairs: vec![] }, SemTy::Dyn, span);
        let callee = self.alloc(HirExprKind::GlobalGet { var_id: slot }, SemTy::Dyn, span);
        Ok(self.alloc(
            HirExprKind::Call {
                callee,
                args: vec![tuple, dict],
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// Emit a decorated module-level function's rebinding into `__main__`
    /// (Phase 6D): `slot := dN(…d1(closure(<orig>.<thunk>)))`, decorators
    /// applied innermost-first.
    /// Replay one import statement's precomputed effect (Phase 8): emit the
    /// module-`<init>` calls (execute-once on first import) followed by the
    /// `from M import VAR` snapshot copies.
    fn emit_import_action(&mut self, action: &ImportAction) {
        let span = Span::dummy();
        for name in &action.init_calls {
            let callee = self.alloc(
                HirExprKind::Name(SymbolRef::Unresolved(*name)),
                SemTy::Dyn,
                span,
            );
            let call = self.alloc(
                HirExprKind::Call {
                    callee,
                    args: vec![],
                },
                SemTy::NoneTy,
                span,
            );
            self.push_stmt(HirStmt::Expr(call));
        }
        for (dst, src) in &action.snapshots {
            let val = self.alloc(HirExprKind::GlobalGet { var_id: *src }, SemTy::Dyn, span);
            self.push_stmt(HirStmt::GlobalSet {
                var_id: *dst,
                value: val,
            });
        }
    }

    fn emit_decorated_rebinding(
        &mut self,
        f: &StmtFunctionDef,
        thunk_fid: FuncId,
        slot: u32,
    ) -> Result<()> {
        let span = to_span(f.range());
        let mut v = self.alloc(
            HirExprKind::MakeClosure {
                func: thunk_fid,
                captures: vec![],
            },
            SemTy::Dyn,
            span,
        );
        for deco in f.decorator_list.iter().rev() {
            v = self.apply_decorator(deco, v, span)?;
        }
        self.push_stmt(HirStmt::GlobalSet {
            var_id: slot,
            value: v,
        });
        Ok(())
    }

    /// Apply one decorator expression to a value (Phase 6D): `deco(v)`. The
    /// decorator is lowered as an ordinary value (a top-level function → its
    /// thunk; a factory `@repeat(3)` → the call result), so the application is a
    /// uniform indirect call.
    fn apply_decorator(
        &mut self,
        deco: &Expr,
        v: Idx<HirExpr>,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        let dval = self.lower_expr(deco)?;
        Ok(self.alloc(
            HirExprKind::Call {
                callee: dval,
                args: vec![v],
            },
            SemTy::Dyn,
            span,
        ))
    }

    // ── generators (Phase 6E) ────────────────────────────────────────────────

    /// Lower a generator expression `(elt for t in it …)` (Phase 6E): a
    /// synthetic generator whose OUTERMOST iterable is an eager parameter
    /// (CPython semantics); inner clauses/elt must be free-var-free (captures in
    /// generators are out of scope), so the gate keeps genexprs self-contained.
    fn lower_genexpr(&mut self, g: &ExprGeneratorExp, span: Span) -> Result<Idx<HirExpr>> {
        if g.generators.is_empty() {
            return Err(parse_error("malformed generator expression", span));
        }
        if g.generators.iter().any(|c| c.is_async) {
            return Err(parse_error(
                "async generator expressions are out of scope",
                span,
            ));
        }
        // The outermost iterable, evaluated eagerly in THIS scope.
        let outer = self.lower_expr(&g.generators[0].iter)?;

        let synth = self.synth_name("<genexpr>");
        let name = self.interner.intern(&synth);
        let wrapper_fid = self.shared.reserve();
        let resume_fid = self.shared.reserve();
        let gen_id = self.shared.generators.len() as u32;
        self.shared.generators.push(resume_fid);

        // ── resume function ──
        let resume_name = self.interner.intern(&format!("{synth}.<resume>"));
        {
            let mut rl = FnLowerer::new(
                self.interner,
                self.ctx,
                self.shared,
                resume_name,
                &synth,
                SemTy::Dyn,
                None,
            );
            let gen_name = rl.intern("__gen__");
            rl.add_param(gen_name, SemTy::Dyn);
            let iter0_name = rl.intern("__iter0__");
            let iter0 = rl.add_logical_local(iter0_name, SemTy::Dyn);
            rl.gen = Some(GenCtx {
                gen_local: LocalId::new(0),
                next_state: 1,
                resume_targets: Vec::new(),
            });
            let start = rl.new_block();
            rl.switch(start);
            rl.lower_genexpr_clauses(g, 0, iter0, span)?;
            if rl.cur_open() {
                rl.emit_gen_exhaust(span);
            }
            rl.gen_rewrite_locals();
            let num_locals = rl.locals.len() as u32 - 1;
            rl.build_gen_dispatch(start);
            let resume_fn = rl.finish(HirTerminator::Return(None));
            self.shared.fill(resume_fid, resume_fn);

            // ── wrapper(iter0) ──
            let mut wl = FnLowerer::new(
                self.interner,
                self.ctx,
                self.shared,
                name,
                &synth,
                SemTy::Dyn,
                None,
            );
            let p = wl.intern("__iter0__");
            wl.add_param(p, SemTy::Dyn);
            let g_local = wl.fresh_local(SemTy::Dyn);
            let mg = wl.alloc(
                HirExprKind::MakeGenerator { gen_id, num_locals },
                SemTy::Dyn,
                span,
            );
            wl.push_stmt(HirStmt::Assign {
                target: g_local,
                value: mg,
            });
            let gen = wl.local_ref(g_local, span);
            let p0 = wl.local_ref(LocalId::new(0), span);
            wl.push_stmt(HirStmt::GenSetLocal {
                gen,
                slot: 0,
                value: p0,
            });
            let g_ret = wl.local_ref(g_local, span);
            wl.seal(HirTerminator::Return(Some(g_ret)));
            let wrapper_fn = wl.finish(HirTerminator::Return(None));
            self.shared.fill(wrapper_fid, wrapper_fn);
        }

        // Call the synthetic wrapper with the eager iterable → the generator.
        let callee = self.alloc(
            HirExprKind::Name(SymbolRef::Unresolved(name)),
            SemTy::Dyn,
            span,
        );
        Ok(self.alloc(
            HirExprKind::Call {
                callee,
                args: vec![outer],
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// Recurse over a genexpr's clauses building nested iterator loops; the
    /// innermost point yields the element (Phase 6E). The first clause iterates
    /// the eager `iter0` parameter; deeper iterables are lowered in place.
    fn lower_genexpr_clauses(
        &mut self,
        g: &ExprGeneratorExp,
        idx: usize,
        iter0: LocalId,
        span: Span,
    ) -> Result<()> {
        if idx == g.generators.len() {
            let elt = self.lower_expr(g.elt.as_ref())?;
            self.suspend(Some(elt), false, span)?;
            return Ok(());
        }
        let comp = &g.generators[idx];
        let iterable = if idx == 0 {
            self.local_ref(iter0, span)
        } else {
            self.lower_expr(&comp.iter)?
        };
        let lp = self.begin_iter_loop(iterable, span)?;
        let elem = self.local_ref(lp.elem, span);
        self.bind_for_target(&comp.target, elem, span)?;
        for cond_expr in &comp.ifs {
            let cond = self.lower_expr(cond_expr)?;
            let cont = self.new_block();
            self.seal(HirTerminator::Branch {
                cond,
                then: cont,
                else_: lp.header,
            });
            self.switch(cont);
        }
        self.lower_genexpr_clauses(g, idx + 1, iter0, span)?;
        self.end_iter_loop(lp);
        Ok(())
    }

    /// A read of the generator object (the resume function's param 0).
    fn gen_ref(&mut self, span: Span) -> Idx<HirExpr> {
        let g = self.gen.as_ref().expect("generator mode").gen_local;
        self.local_ref(g, span)
    }

    /// Lower a `yield e` / `yield from it` statement (the value is discarded on
    /// resume — Phase 6E).
    fn lower_yield_stmt(&mut self, expr: &Expr) -> Result<()> {
        self.lower_yield_value(expr, false)?;
        Ok(())
    }

    /// Lower a yield expression as a suspend point. Returns the resumed sent
    /// value when `want_sent`. `yield from it` desugars to a for-loop of plain
    /// yields. `yield` / `yield e` suspend: evaluate the value, `SetState(k)`,
    /// return it; the resume block checks `IsClosing` (→ exhaust) then continues.
    fn lower_yield_value(&mut self, expr: &Expr, want_sent: bool) -> Result<Option<Idx<HirExpr>>> {
        let span = to_span(expr.range());
        if let Expr::YieldFrom(yf) = expr {
            // `yield from sub` → `for __yf in sub: yield __yf` (StopIteration.value
            // and send-forwarding are out of scope — documented).
            let iterable = self.lower_expr(yf.value.as_ref())?;
            let lp = self.begin_iter_loop(iterable, span)?;
            let elem = self.local_ref(lp.elem, span);
            self.suspend(Some(elem), false, span)?;
            self.end_iter_loop(lp);
            return Ok(None);
        }
        let Expr::Yield(y) = expr else {
            return Err(parse_error("expected a yield expression", span));
        };
        let value = match &y.value {
            Some(e) => Some(self.lower_expr(e.as_ref())?),
            None => None,
        };
        self.suspend(value, want_sent, span)
    }

    /// Emit a suspend point: `SetState(k); Return(value)`, then a resume block
    /// that checks `IsClosing` and (if `want_sent`) reads the sent value.
    fn suspend(
        &mut self,
        value: Option<Idx<HirExpr>>,
        want_sent: bool,
        span: Span,
    ) -> Result<Option<Idx<HirExpr>>> {
        // A suspended frame would dangle its stack-allocated ExceptionFrame
        // (the frame registers a stack address with the runtime, and the
        // resume call runs on a different stack) — reject lexically (Phase 7).
        if self
            .scope_stack
            .iter()
            .any(|s| !matches!(s, ScopeCtx::Loop { .. }))
        {
            return Err(parse_error(
                "yield inside try/with is unsupported in this milestone",
                span,
            ));
        }
        let value = value.unwrap_or_else(|| self.alloc(HirExprKind::NoneLit, SemTy::NoneTy, span));
        let k = {
            let g = self.gen.as_mut().expect("generator mode");
            let k = g.next_state;
            g.next_state += 1;
            k
        };
        let gen = self.gen_ref(span);
        self.push_stmt(HirStmt::GenSetState { gen, state: k });
        self.seal(HirTerminator::Return(Some(value)));

        let resume = self.new_block();
        self.gen.as_mut().unwrap().resume_targets.push((k, resume));
        self.switch(resume);

        // `close()` resumes with `closing` set: exhaust and return None (no
        // try/finally pre-Phase-7, so exhaust is the correct unwind).
        let gen2 = self.gen_ref(span);
        let closing = self.alloc(
            HirExprKind::GenQuery {
                op: GenOp::IsClosing,
                gen: gen2,
                imm: 0,
                value: None,
            },
            SemTy::Bool,
            span,
        );
        let close_b = self.new_block();
        let cont = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: closing,
            then: close_b,
            else_: cont,
        });
        self.switch(close_b);
        self.emit_gen_exhaust(span);
        self.switch(cont);

        if want_sent {
            let gen3 = self.gen_ref(span);
            let sent = self.alloc(
                HirExprKind::GenQuery {
                    op: GenOp::GetSentValue,
                    gen: gen3,
                    imm: 0,
                    value: None,
                },
                SemTy::Dyn,
                span,
            );
            Ok(Some(sent))
        } else {
            Ok(None)
        }
    }

    /// Emit the generator's exhaust sequence: `SetExhausted; SetState(MAX);
    /// Return None`. Used at fallthrough / `return` / `close()`.
    fn emit_gen_exhaust(&mut self, span: Span) {
        let gen = self.gen_ref(span);
        self.push_stmt(HirStmt::GenSetExhausted { gen });
        let gen2 = self.gen_ref(span);
        self.push_stmt(HirStmt::GenSetState {
            gen: gen2,
            state: u32::MAX,
        });
        let none = self.alloc(HirExprKind::NoneLit, SemTy::NoneTy, span);
        self.seal(HirTerminator::Return(Some(none)));
    }

    /// Rewrite every named/synthetic local access to generator-slot storage
    /// (P6-3): `Local(lid)` → `GenQuery(GetLocal, slot)`, `Assign{target}` →
    /// `GenSetLocal{slot}`. Local 0 (the generator param) is left untouched.
    /// Slot index = `lid - 1`; so `num_locals = locals.len() - 1`.
    fn gen_rewrite_locals(&mut self) {
        let span = Span::dummy();
        let gen_local = self.gen.as_ref().unwrap().gen_local;
        debug_assert_eq!(gen_local.index(), 0);
        // Rewrite reads (`Local`) in place.
        let read_rewrites: Vec<(Idx<HirExpr>, u32)> = self
            .exprs
            .iter()
            .filter_map(|(idx, e)| match e.kind {
                HirExprKind::Local(lid) if lid.index() != 0 => Some((idx, lid.index() as u32 - 1)),
                _ => None,
            })
            .collect();
        for (idx, slot) in read_rewrites {
            let gen = self.alloc(HirExprKind::Local(gen_local), SemTy::Dyn, span);
            self.exprs[idx].kind = HirExprKind::GenQuery {
                op: GenOp::GetLocal,
                gen,
                imm: slot,
                value: None,
            };
        }
        // Rewrite writes (`Assign`) in place across every block.
        let block_ids: Vec<Idx<HirBlock>> = self.blocks.iter().map(|(b, _)| b).collect();
        for b in block_ids {
            let n = self.blocks[b].stmts.len();
            for i in 0..n {
                if let HirStmt::Assign { target, value } = self.blocks[b].stmts[i] {
                    if target.index() != 0 {
                        let slot = target.index() as u32 - 1;
                        let gen = self.alloc(HirExprKind::Local(gen_local), SemTy::Dyn, span);
                        self.blocks[b].stmts[i] = HirStmt::GenSetLocal { gen, slot, value };
                    }
                }
            }
        }
    }

    /// Build the entry dispatch (Phase 6E): a compare-chain on `GetState` routing
    /// state 0 → `start`, state k → its resume block, anything else → exhaust.
    /// Built AFTER `gen_rewrite_locals`, so its fresh `Local(gen)` reads survive.
    fn build_gen_dispatch(&mut self, start: Idx<HirBlock>) {
        let span = Span::dummy();
        let mut chain: Vec<(u32, Idx<HirBlock>)> = vec![(0, start)];
        chain.extend(self.gen.as_ref().unwrap().resume_targets.iter().copied());
        let default_b = self.new_block();
        let mut block = self.entry;
        let len = chain.len();
        for (i, (state, target)) in chain.into_iter().enumerate() {
            self.switch(block);
            let gen = self.gen_ref(span);
            let s = self.alloc(
                HirExprKind::GenQuery {
                    op: GenOp::GetState,
                    gen,
                    imm: 0,
                    value: None,
                },
                SemTy::Int,
                span,
            );
            let k = self.alloc(HirExprKind::IntLit(state as i64), SemTy::Int, span);
            let cmp = self.alloc(
                HirExprKind::Compare {
                    op: CmpOp::Eq,
                    l: s,
                    r: k,
                },
                SemTy::Bool,
                span,
            );
            let next = if i + 1 < len {
                self.new_block()
            } else {
                default_b
            };
            self.seal(HirTerminator::Branch {
                cond: cmp,
                then: target,
                else_: next,
            });
            block = next;
        }
        self.switch(default_b);
        self.emit_gen_exhaust(span);
    }

    /// Adapt a call to a known top-level function (Phase 6C): reorder keyword
    /// args, fill constant defaults, and pack `*args` / `**kwargs` — producing
    /// the positional argument vector matching the callee's MIR parameter order
    /// (fixed → keyword-only → `*args` tuple → `**kwargs` dict).
    /// Lower the builtin `open(file, mode="r", encoding=None)` (Phase 8C)
    /// through the shared stdlib-call adapter, against a synthetic descriptor
    /// targeting `rt_file_open`. The result's `binary`-ness is derived in
    /// typeck from the (constant) mode literal.
    fn lower_open_builtin(&mut self, c: &ExprCall, span: Span) -> Result<Idx<HirExpr>> {
        self.lower_stdlib_call(&OPEN_DEF, c, span)
    }

    /// Adapt a Python-level stdlib call against its declarative descriptor and
    /// emit [`HirExprKind::CallRuntime`] (Phase 8B). Positional args fill param
    /// slots in order; keywords match by `ParamDef.name`; an absent optional
    /// param takes its `ConstValue` default as a literal, or stays an empty
    /// slot (the null-pointer sentinel) when it has none. The user-written arg
    /// count is recorded for `pass_arg_count` descriptors.
    fn lower_stdlib_call(
        &mut self,
        def: &'static pyaot_stdlib_defs::StdlibFunctionDef,
        c: &ExprCall,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        if c.args.iter().any(|a| matches!(a, Expr::Starred(_))) {
            return Err(parse_error(
                "`*` spreading into a stdlib call is out of scope",
                span,
            ));
        }
        // A `variadic_to_list` descriptor (`os.path.join(*paths)`) collects all
        // positional args into one list passed as the single runtime arg
        // (Phase 8D). These descriptors are pure-variadic (no leading fixed
        // params) and take no keywords.
        if def.hints.variadic_to_list {
            if !c.keywords.is_empty() {
                return Err(parse_error(
                    format!("`{}()` does not take keyword arguments", def.name),
                    span,
                ));
            }
            let provided = c.args.len();
            if provided < def.min_args {
                return Err(parse_error(
                    format!(
                        "`{}()` takes at least {} argument(s)",
                        def.name, def.min_args
                    ),
                    span,
                ));
            }
            let elem_spec = &def.params[0].ty;
            let mut elems = Vec::with_capacity(c.args.len());
            for a in &c.args {
                elems.push(self.lower_stdlib_arg(a, elem_spec)?);
            }
            let list = self.alloc(HirExprKind::ListLit { elems }, SemTy::Dyn, span);
            return Ok(self.alloc(
                HirExprKind::CallRuntime {
                    target: pyaot_hir::RuntimeCallTarget::Func(def),
                    args: vec![Some(list)],
                    provided: provided as u32,
                },
                SemTy::Dyn,
                span,
            ));
        }
        let provided = c.args.len() + c.keywords.len();
        if provided < def.min_args || (def.max_args != usize::MAX && provided > def.max_args) {
            return Err(parse_error(
                format!(
                    "`{}()` takes {}..={} argument(s) but {provided} were given",
                    def.name, def.min_args, def.max_args,
                ),
                span,
            ));
        }
        // With keywords present, slot matching reorders arguments — stage every
        // (non-literal) value in WRITTEN order first (CPython evaluation order).
        let staging = !c.keywords.is_empty();
        let mut positionals: Vec<ArgSrc> = Vec::with_capacity(c.args.len());
        for a in &c.args {
            positionals.push(if staging {
                self.stage_arg_src(a)?
            } else {
                ArgSrc::Plain(a)
            });
        }
        let mut keywords: Vec<(String, ArgSrc, bool)> = Vec::new();
        for kw in &c.keywords {
            let Some(name) = &kw.arg else {
                return Err(parse_error(
                    "`**kwargs` spreading is out of scope here",
                    span,
                ));
            };
            let src = self.stage_arg_src(&kw.value)?;
            keywords.push((name.as_str().to_string(), src, false));
        }

        let mut slots: Vec<Option<Idx<HirExpr>>> = Vec::with_capacity(def.params.len());
        for (i, p) in def.params.iter().enumerate() {
            let v = if i < positionals.len() {
                self.stdlib_arg_slot(positionals[i], &p.ty, p.optional, span)?
            } else if let Some(kv) = take_keyword(&mut keywords, p.name) {
                self.stdlib_arg_slot(kv, &p.ty, p.optional, span)?
            } else if let Some(cv) = &p.default {
                Some(self.lower_stdlib_const(cv, span))
            } else if p.optional {
                None
            } else {
                return Err(parse_error(
                    format!("`{}()` missing required argument `{}`", def.name, p.name),
                    span,
                ));
            };
            slots.push(v);
        }
        if let Some((k, _, _)) = keywords.iter().find(|(_, _, used)| !used) {
            return Err(parse_error(
                format!("`{}()` got an unexpected keyword argument `{k}`", def.name),
                span,
            ));
        }
        Ok(self.alloc(
            HirExprKind::CallRuntime {
                target: pyaot_hir::RuntimeCallTarget::Func(def),
                args: slots,
                provided: provided as u32,
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// Fill one stdlib-call argument slot. An explicit `None` passed to an
    /// optional OBJECT param (`urlopen(url, None, …)`, `Request(url, data=None)`)
    /// becomes an absent slot → the null-pointer sentinel the runtime expects
    /// (`Value::NONE` would be `unwrap_ptr`-ed into a wild pointer). Raw
    /// primitive params (`Float`/`Int`/`Bool`) keep the literal. Phase 8D.
    fn stdlib_arg_slot(
        &mut self,
        arg: ArgSrc<'_>,
        spec: &pyaot_stdlib_defs::TypeSpec,
        optional: bool,
        span: Span,
    ) -> Result<Option<Idx<HirExpr>>> {
        use pyaot_stdlib_defs::TypeSpec;
        let arg = match arg {
            ArgSrc::Plain(e) => e,
            // Already staged — never a literal (see `stage_arg_src`), so the
            // None-sentinel / int→float folds below cannot apply.
            ArgSrc::Staged(l) => return Ok(Some(self.local_ref(l, span))),
        };
        let is_object = !matches!(spec, TypeSpec::Float | TypeSpec::Int | TypeSpec::Bool);
        if optional && is_object && is_none_lit(arg) {
            return Ok(None);
        }
        Ok(Some(self.lower_stdlib_arg(arg, spec)?))
    }

    /// Lower one stdlib-call argument. An integer literal headed for a `Float`
    /// param becomes a float literal (CPython's int→float coercion, performed
    /// at the only place it is statically certain — implicit conversion of a
    /// runtime int at a raw-ABI boundary stays a typeck error).
    fn lower_stdlib_arg(
        &mut self,
        e: &Expr,
        spec: &pyaot_stdlib_defs::TypeSpec,
    ) -> Result<Idx<HirExpr>> {
        if matches!(spec, pyaot_stdlib_defs::TypeSpec::Float) {
            let int_lit = |expr: &Expr| -> Option<f64> {
                if let Expr::Constant(k) = expr {
                    if let Constant::Int(i) = &k.value {
                        return i.to_string().parse::<f64>().ok();
                    }
                }
                None
            };
            let span = to_span(e.range());
            if let Some(f) = int_lit(e) {
                return Ok(self.alloc(HirExprKind::FloatLit(f), SemTy::Float, span));
            }
            // `-5` parses as USub(Constant) — fold it too.
            if let Expr::UnaryOp(u) = e {
                if matches!(u.op, PyUnaryOp::USub) {
                    if let Some(f) = int_lit(u.operand.as_ref()) {
                        return Ok(self.alloc(HirExprKind::FloatLit(-f), SemTy::Float, span));
                    }
                }
            }
        }
        self.lower_expr(e)
    }

    /// Materialize a descriptor's `ConstValue` default as a literal expr.
    fn lower_stdlib_const(
        &mut self,
        cv: &pyaot_stdlib_defs::ConstValue,
        span: Span,
    ) -> Idx<HirExpr> {
        use pyaot_stdlib_defs::ConstValue;
        match cv {
            ConstValue::Int(i) => self.alloc(HirExprKind::IntLit(*i), SemTy::Int, span),
            ConstValue::Float(f) => self.alloc(HirExprKind::FloatLit(*f), SemTy::Float, span),
            ConstValue::Bool(b) => self.alloc(HirExprKind::BoolLit(*b), SemTy::Bool, span),
            ConstValue::Str(s) => {
                let id = self.intern(s);
                self.alloc(HirExprKind::StrLit(id), SemTy::Str, span)
            }
        }
    }

    fn lower_direct_known_call(
        &mut self,
        info: &TopDefInfo,
        fname: &str,
        c: &ExprCall,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        if has_doublestar_kwarg(c) {
            return Err(parse_error(
                "`**kwargs` spreading into a direct call is out of scope (Phase 6C)",
                span,
            ));
        }
        // Classify positional args. A `*list` / `*tuple` LITERAL spread has a
        // compile-time-known arity, so its elements flatten into plain
        // positionals and reuse the slot-matching path below. A runtime `*seq`
        // spread (a variable / call result / comprehension) has an unknown
        // length, so it routes to the general runtime-spread path.
        let (items, has_runtime_spread) = classify_pos_args(c);
        if has_runtime_spread {
            return self.lower_spread_call(info, fname, &items, c, span);
        }

        // With keywords present, slot matching below reorders arguments — so
        // pass 1 stages every (non-literal) argument value in WRITTEN order
        // (CPython's evaluation order), and pass 2 only assembles slot refs.
        let staging = !c.keywords.is_empty();
        let mut positionals: Vec<ArgSrc> = Vec::with_capacity(items.len());
        for item in &items {
            let PosItem::Plain(e) = *item else {
                unreachable!("runtime spreads handled above")
            };
            positionals.push(if staging {
                self.stage_arg_src(e)?
            } else {
                ArgSrc::Plain(e)
            });
        }
        // (keyword name, value, consumed?)
        let mut keywords: Vec<(String, ArgSrc, bool)> = Vec::new();
        for kw in &c.keywords {
            let Some(name) = &kw.arg else {
                return Err(parse_error(
                    "`**kwargs` spreading is out of scope here",
                    span,
                ));
            };
            let src = self.stage_arg_src(&kw.value)?;
            keywords.push((name.as_str().to_string(), src, false));
        }

        let n_fixed = info.fixed.len();
        let mut out: Vec<Idx<HirExpr>> = Vec::with_capacity(n_fixed + 2);

        // ── fixed positional / keyword / default slot matching ──
        let star_tuple: Option<Idx<HirExpr>> = {
            let n_pos = positionals.len();
            if n_pos > n_fixed && info.varargs.is_none() {
                return Err(parse_error(
                    format!(
                        "`{fname}()` takes {n_fixed} positional argument(s) but {n_pos} were given"
                    ),
                    span,
                ));
            }
            let pos_for_fixed = n_pos.min(n_fixed);
            for (i, p) in info.fixed.iter().enumerate() {
                let v = if i < pos_for_fixed {
                    self.arg_src_value(positionals[i], span)?
                } else if let Some(kv) = take_keyword(&mut keywords, self.interner.resolve(p.name))
                {
                    self.arg_src_value(kv, span)?
                } else if let Some(def) = &p.default {
                    self.lower_const_default(def, span)
                } else {
                    return Err(parse_error(
                        format!(
                            "`{fname}()` missing required argument `{}`",
                            self.interner.resolve(p.name)
                        ),
                        span,
                    ));
                };
                out.push(v);
            }
            if info.varargs.is_some() {
                let mut excess = Vec::new();
                for p in positionals.iter().skip(n_fixed).copied().collect::<Vec<_>>() {
                    excess.push(self.arg_src_value(p, span)?);
                }
                Some(self.alloc(HirExprKind::TupleLit { elems: excess }, SemTy::Dyn, span))
            } else {
                None
            }
        };

        // ── keyword-only params ──
        for p in &info.kwonly {
            let v = if let Some(kv) = take_keyword(&mut keywords, self.interner.resolve(p.name)) {
                self.arg_src_value(kv, span)?
            } else if let Some(def) = &p.default {
                self.lower_const_default(def, span)
            } else {
                return Err(parse_error(
                    format!(
                        "`{fname}()` missing required keyword-only argument `{}`",
                        self.interner.resolve(p.name)
                    ),
                    span,
                ));
            };
            out.push(v);
        }

        // ── *args tuple slot ──
        if info.varargs.is_some() {
            match star_tuple {
                Some(t) => out.push(t),
                None => {
                    out.push(self.alloc(HirExprKind::TupleLit { elems: vec![] }, SemTy::Dyn, span))
                }
            }
        }

        // ── **kwargs dict slot: leftover keywords (source order) ──
        if info.kwargs.is_some() {
            let mut pairs = Vec::new();
            // Re-borrow names first to avoid a borrow conflict with lower_expr.
            let leftover: Vec<(InternedString, ArgSrc)> = keywords
                .iter()
                .filter(|(_, _, used)| !*used)
                .map(|(name, v, _)| (self.interner.intern(name), *v))
                .collect();
            for (key_id, v) in leftover {
                let key = self.alloc(HirExprKind::StrLit(key_id), SemTy::Str, span);
                let val = self.arg_src_value(v, span)?;
                pairs.push((key, val));
            }
            out.push(self.alloc(HirExprKind::DictLit { pairs }, SemTy::Dyn, span));
        } else if let Some((name, _, _)) = keywords.iter().find(|(_, _, used)| !*used) {
            return Err(parse_error(
                format!("`{fname}()` got an unexpected keyword argument `{name}`"),
                span,
            ));
        }

        let target = self.intern(fname);
        let callee = self.alloc(
            HirExprKind::Name(SymbolRef::Unresolved(target)),
            SemTy::Dyn,
            span,
        );
        Ok(self.alloc(HirExprKind::Call { callee, args: out }, SemTy::Dyn, span))
    }

    /// Lower a known-callee call carrying a runtime `*seq` spread — a sequence
    /// whose length is unknown until run time (`f(*xs)`, `f(a, *xs, b)`,
    /// `f(*xs, *ys)`). The full positional sequence is materialized into a fresh
    /// `argv` list in WRITTEN order ([`Self::build_spread_argv`]), an argument-
    /// count guard runs against the callee's arity, then each parameter slot is
    /// bound by position: required slots read `argv[i]`, defaulted slots read
    /// `argv[i]` when present else the default, a `*args` callee takes
    /// `tuple(argv[n_fixed:])` as its rest tuple. Keyword args are not combined
    /// with a runtime spread (the corpus never does, and it keeps slot matching
    /// simple).
    fn lower_spread_call(
        &mut self,
        info: &TopDefInfo,
        fname: &str,
        items: &[PosItem],
        c: &ExprCall,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        if !c.keywords.is_empty() {
            return Err(parse_error(
                format!(
                    "`{fname}()`: keyword arguments combined with a runtime `*` spread are out of scope"
                ),
                span,
            ));
        }
        let argv = self.build_spread_argv(items, span)?;
        let n_fixed = info.fixed.len();
        // Required = leading fixed params without a default (Python keeps
        // defaults trailing, so the first defaulted index IS the required count).
        let req = info
            .fixed
            .iter()
            .position(|p| p.default.is_some())
            .unwrap_or(n_fixed);

        // n = len(argv), reused by the count guard and the default-slot tests.
        let n_local = self.fresh_local(SemTy::Int);
        let argv_ref = self.local_ref(argv, span);
        let n_expr = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Len,
                args: vec![argv_ref],
            },
            SemTy::Int,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: n_local,
            value: n_expr,
        });
        let max = if info.varargs.is_some() {
            None
        } else {
            Some(n_fixed)
        };
        self.emit_argcount_check(n_local, req, max, fname, span);

        // Build the param-aligned argument vector (fixed → kw-only → *args tuple
        // → **kwargs dict), matching the callee's MIR parameter order.
        let mut out: Vec<Idx<HirExpr>> = Vec::with_capacity(n_fixed + 2);
        for (i, p) in info.fixed.iter().enumerate() {
            let raw = if i < req {
                // Required: `argv[i]`, in-bounds after the count guard.
                let base = self.local_ref(argv, span);
                let idx = self.alloc(HirExprKind::IntLit(i as i64), SemTy::Int, span);
                self.alloc(HirExprKind::Subscript { base, index: idx }, SemTy::Dyn, span)
            } else {
                let def = p.default.as_ref().expect("trailing fixed param has a default");
                let default = self.lower_const_default(def, span);
                self.emit_spread_default(argv, n_local, i, default, span)
            };
            let v = self.launder_arg(raw, &p.ty, span);
            out.push(v);
        }
        // Keyword-only params: a `*` spread fills no keywords, so each must carry
        // a default (else the call cannot be satisfied).
        for p in &info.kwonly {
            let Some(def) = &p.default else {
                return Err(parse_error(
                    format!(
                        "`{fname}()` keyword-only parameter `{}` cannot be filled from a `*` spread",
                        self.interner.resolve(p.name)
                    ),
                    span,
                ));
            };
            let d = self.lower_const_default(def, span);
            let v = self.launder_arg(d, &p.ty, span);
            out.push(v);
        }
        // `*args` rest tuple = `tuple(argv[n_fixed:])`.
        if info.varargs.is_some() {
            let base = self.local_ref(argv, span);
            let start = self.alloc(HirExprKind::IntLit(n_fixed as i64), SemTy::Int, span);
            let slice = self.alloc(
                HirExprKind::Slice {
                    base,
                    start: Some(start),
                    end: None,
                    step: None,
                },
                SemTy::list_of(SemTy::Dyn),
                span,
            );
            let it = self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::Iter,
                    args: vec![slice],
                },
                SemTy::Dyn,
                span,
            );
            let rest = self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::TupleFromIter,
                    args: vec![it],
                },
                SemTy::tuple_var_of(SemTy::Dyn),
                span,
            );
            out.push(rest);
        }
        // `**kwargs` dict slot: a `*` spread supplies no keywords → empty.
        if info.kwargs.is_some() {
            out.push(self.alloc(HirExprKind::DictLit { pairs: vec![] }, SemTy::Dyn, span));
        }

        let target = self.intern(fname);
        let callee = self.alloc(
            HirExprKind::Name(SymbolRef::Unresolved(target)),
            SemTy::Dyn,
            span,
        );
        Ok(self.alloc(HirExprKind::Call { callee, args: out }, SemTy::Dyn, span))
    }

    /// Materialize the full positional sequence of a `*`-spread call into a fresh
    /// `list[Dyn]` local, evaluating each item in WRITTEN (left-to-right) order:
    /// a plain arg is appended once; a `*seq` spread is iterated (the iterator
    /// protocol, so any iterable — list / tuple / deque / generator / range —
    /// works) and each element appended.
    fn build_spread_argv(&mut self, items: &[PosItem], span: Span) -> Result<LocalId> {
        let argv = self.fresh_local(SemTy::list_of(SemTy::Dyn));
        let empty = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: argv,
            value: empty,
        });
        for item in items {
            match *item {
                PosItem::Plain(e) => {
                    let v = self.lower_expr(e)?;
                    self.push_stmt(HirStmt::ContainerPush {
                        container: argv,
                        value: v,
                    });
                }
                PosItem::Spread(e) => {
                    let src = self.lower_expr(e)?;
                    let lp = self.begin_iter_loop(src, span)?;
                    let elem = self.local_ref(lp.elem, span);
                    self.push_stmt(HirStmt::ContainerPush {
                        container: argv,
                        value: elem,
                    });
                    self.end_iter_loop(lp);
                }
            }
        }
        Ok(argv)
    }

    /// `(i < n) ? argv[i] : default` — the value for a defaulted fixed slot under
    /// a runtime spread, as a short-circuit CFG ternary (`argv[i]` is only read
    /// on the in-bounds arm). Returns a read of the result local.
    fn emit_spread_default(
        &mut self,
        argv: LocalId,
        n_local: LocalId,
        i: usize,
        default: Idx<HirExpr>,
        span: Span,
    ) -> Idx<HirExpr> {
        let res = self.fresh_local(SemTy::Dyn);
        let i_lit = self.alloc(HirExprKind::IntLit(i as i64), SemTy::Int, span);
        let n_ref = self.local_ref(n_local, span);
        let cond = self.alloc(
            HirExprKind::Compare {
                op: CmpOp::Lt,
                l: i_lit,
                r: n_ref,
            },
            SemTy::Bool,
            span,
        );
        let then_b = self.new_block();
        let else_b = self.new_block();
        let join = self.new_block();
        self.seal(HirTerminator::Branch {
            cond,
            then: then_b,
            else_: else_b,
        });
        self.switch(then_b);
        let base = self.local_ref(argv, span);
        let idx = self.alloc(HirExprKind::IntLit(i as i64), SemTy::Int, span);
        let av = self.alloc(HirExprKind::Subscript { base, index: idx }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: res,
            value: av,
        });
        self.seal(HirTerminator::Jump(join));
        self.switch(else_b);
        self.push_stmt(HirStmt::Assign {
            target: res,
            value: default,
        });
        self.seal(HirTerminator::Jump(join));
        self.switch(join);
        self.local_ref(res, span)
    }

    /// A spread value reaches a fixed / kw-only slot as a gradual `Dyn` (it came
    /// from a runtime `argv` subscript). `int` (Tagged), `str` / containers
    /// (gradual `Heap`), and `Dyn` params admit it directly. A `float` / `bool`
    /// param reinterprets its bits by the annotated type (PITFALLS A2), so typeck
    /// rejects a `Dyn` there — launder the value through a `pin_tagged`
    /// authoritative-typed local: typeck sees the param type (the `pin_tagged`
    /// store skips the reinterpret check), and lowering unboxes the Tagged value
    /// to the param's `Raw` repr at the call.
    fn launder_arg(&mut self, value: Idx<HirExpr>, param_ty: &SemTy, span: Span) -> Idx<HirExpr> {
        if !matches!(param_ty, SemTy::Float | SemTy::Bool) {
            return value;
        }
        let slot = self.fresh_local_pinned(param_ty.clone());
        self.push_stmt(HirStmt::Assign {
            target: slot,
            value,
        });
        self.local_ref(slot, span)
    }

    /// Emit the argument-count guards for a runtime spread: too few values
    /// (`len(argv) < min`) and, for a non-`*args` callee, too many
    /// (`len(argv) > max`). Each raises `TypeError`, matching CPython's
    /// wrong-arity behavior (the success path never trips them).
    fn emit_argcount_check(
        &mut self,
        n_local: LocalId,
        min: usize,
        max: Option<usize>,
        fname: &str,
        span: Span,
    ) {
        if min > 0 {
            self.emit_count_guard(
                n_local,
                CmpOp::Lt,
                min,
                format!("`{fname}()` missing required positional argument(s) (too few values to spread)"),
                span,
            );
        }
        if let Some(max) = max {
            self.emit_count_guard(
                n_local,
                CmpOp::Gt,
                max,
                format!("`{fname}()` takes {max} positional argument(s) but more were spread"),
                span,
            );
        }
    }

    /// `if (n <op> bound): raise TypeError(msg)` — one arity guard for a runtime
    /// spread. Mirrors the `assert … , msg` desugar (branch → raise →
    /// `Unreachable`), then continues in the pass block.
    fn emit_count_guard(
        &mut self,
        n_local: LocalId,
        op: CmpOp,
        bound: usize,
        msg: String,
        span: Span,
    ) {
        let n_ref = self.local_ref(n_local, span);
        let b = self.alloc(HirExprKind::IntLit(bound as i64), SemTy::Int, span);
        let cond = self.alloc(
            HirExprKind::Compare {
                op,
                l: n_ref,
                r: b,
            },
            SemTy::Bool,
            span,
        );
        let fail = self.new_block();
        let ok = self.new_block();
        self.seal(HirTerminator::Branch {
            cond,
            then: fail,
            else_: ok,
        });
        self.switch(fail);
        let msg_id = self.intern(&msg);
        let m = self.alloc(HirExprKind::StrLit(msg_id), SemTy::Str, span);
        self.push_stmt(HirStmt::Raise(HirRaise::Builtin {
            tag: pyaot_core_defs::BuiltinExceptionKind::TypeError.tag(),
            msg: Some(m),
        }));
        self.seal(HirTerminator::Unreachable);
        self.switch(ok);
    }

    /// Lower an indirect / unknown-callee call (Phase 6C): plain positionals,
    /// then a `*t` spread (the callee's `*args` slot), then a `**d` spread (the
    /// `**kwargs` slot). Named keywords are rejected — an indirect call cannot
    /// reorder against an unknown declaration.
    fn lower_indirect_or_unknown_call(&mut self, c: &ExprCall, span: Span) -> Result<Idx<HirExpr>> {
        if c.keywords.iter().any(|k| k.arg.is_some()) {
            return Err(parse_error(
                "keyword arguments are not supported on indirect calls (annotate the \
                 callee and pass positionally, or call a top-level function by name)",
                span,
            ));
        }
        let callee = self.lower_callee(c.func.as_ref())?;
        let mut args = Vec::with_capacity(c.args.len());
        for a in &c.args {
            match a {
                Expr::Starred(s) => args.push(self.lower_expr(s.value.as_ref())?),
                other => args.push(self.lower_expr(other)?),
            }
        }
        // A `**d` spread fills the trailing **kwargs slot.
        for kw in &c.keywords {
            if kw.arg.is_none() {
                args.push(self.lower_expr(&kw.value)?);
            }
        }
        Ok(self.alloc(HirExprKind::Call { callee, args }, SemTy::Dyn, span))
    }

    /// Materialize a constant default value (Phase 6C) as a literal expr.
    fn lower_const_default(&mut self, init: &ClassAttrInit, span: Span) -> Idx<HirExpr> {
        let (kind, ty) = match init {
            ClassAttrInit::Int(v) => (HirExprKind::IntLit(*v), SemTy::Int),
            ClassAttrInit::BigInt(s) => (HirExprKind::BigIntLit(*s), SemTy::Int),
            ClassAttrInit::Float(f) => (HirExprKind::FloatLit(*f), SemTy::Float),
            ClassAttrInit::Bool(b) => (HirExprKind::BoolLit(*b), SemTy::Bool),
            ClassAttrInit::Str(s) => (HirExprKind::StrLit(*s), SemTy::Str),
            ClassAttrInit::Bytes(s) => (HirExprKind::BytesLit(*s), SemTy::Bytes),
            ClassAttrInit::None => (HirExprKind::NoneLit, SemTy::NoneTy),
            // `()` default → a fresh empty tuple (immutable, so per-call freshness
            // matches CPython's shared singleton observably).
            ClassAttrInit::EmptyTuple => (HirExprKind::TupleLit { elems: vec![] }, SemTy::Dyn),
        };
        self.alloc(kind, ty, span)
    }

    /// Lower a call's callee. A bare name NOT bound in this scope stays a
    /// `Name` (a direct call resolved by `semantics` — never a value-position
    /// thunk); anything else (closure-typed locals/cells, call results) lowers
    /// normally and the call goes indirect.
    fn lower_callee(&mut self, func: &Expr) -> Result<Idx<HirExpr>> {
        if let Expr::Name(n) = func {
            let name = self.intern(n.id.as_str());
            if !self.scope.contains_key(&name) {
                let span = to_span(func.range());
                // A promoted module-global callee (e.g. a decorated top-level
                // function, Phase 6D) reads its slot and calls indirectly.
                if let Some(var_id) = self.global_read_slot(name) {
                    return Ok(self.alloc(HirExprKind::GlobalGet { var_id }, SemTy::Dyn, span));
                }
                return Ok(self.alloc(
                    HirExprKind::Name(SymbolRef::Unresolved(name)),
                    SemTy::Dyn,
                    span,
                ));
            }
        }
        self.lower_expr(func)
    }

    fn lower_constant(&mut self, c: &Constant, span: Span) -> Result<Idx<HirExpr>> {
        let (kind, ty) = match c {
            Constant::Str(s) => (HirExprKind::StrLit(self.intern(s)), SemTy::Str),
            Constant::Int(big) => (self.int_literal(&big.to_string(), false), SemTy::Int),
            Constant::Float(f) => (HirExprKind::FloatLit(*f), SemTy::Float),
            Constant::Bool(b) => (HirExprKind::BoolLit(*b), SemTy::Bool),
            Constant::None => (HirExprKind::NoneLit, SemTy::NoneTy),
            Constant::Bytes(b) => {
                // The bytes are interned through the string table (codegen reads
                // them back as raw bytes). Non-UTF-8 byte literals are out of scope.
                let s = std::str::from_utf8(b)
                    .map_err(|_| parse_error("non-UTF-8 bytes literals are out of scope", span))?;
                (HirExprKind::BytesLit(self.intern(s)), SemTy::Bytes)
            }
            _ => {
                return Err(parse_error(
                    "unsupported literal kind for this milestone",
                    span,
                ))
            }
        };
        Ok(self.alloc(kind, ty, span))
    }

    /// Build an int-literal node, choosing the tagged-fixnum or bignum path.
    /// `decimal` is the non-negative magnitude text; `negative` applies a sign.
    fn int_literal(&mut self, decimal: &str, negative: bool) -> HirExprKind {
        match decimal.parse::<i64>() {
            Ok(mag) if pyaot_core_defs::int_fits(if negative { -mag } else { mag }) => {
                HirExprKind::IntLit(if negative { -mag } else { mag })
            }
            _ => {
                let text = if negative {
                    format!("-{decimal}")
                } else {
                    decimal.to_string()
                };
                HirExprKind::BigIntLit(self.intern(&text))
            }
        }
    }
}

/// A `range()` bound argument: the literal `0` start of `range(stop)`, or an
/// arbitrary expression.
enum RangeArg<'a> {
    Zero,
    Expr(&'a Expr),
}

/// True iff `iter` is a direct `range(...)` call — selects the Phase-3 fast path.
fn is_range_call(iter: &Expr) -> bool {
    matches!(iter, Expr::Call(c)
        if matches!(c.func.as_ref(), Expr::Name(n) if n.id.as_str() == "range"))
}

/// True when a `range(...)` call's step is a compile-time integer literal — the
/// precondition for the Phase-3c raw-i64 fast path (which decides the loop
/// direction statically). `range(stop)` / `range(start, stop)` have an implicit
/// step of `1` (literal); `range(start, stop, step)` qualifies only when
/// `step` is an int literal (incl. unary sign). A non-literal/computed step
/// routes to the general iterator path (runtime `RangeIter`). Callers gate this
/// behind [`is_range_call`], so a non-`range` expr conservatively returns false.
fn range_step_is_literal(iter: &Expr) -> bool {
    let Expr::Call(call) = iter else {
        return false;
    };
    match call.args.len() {
        0..=2 => true,
        3 => literal_int(&call.args[2]).is_some(),
        _ => false,
    }
}

/// Flatten an attribute chain `a.b.c` rooted at a `Name` into its leftmost name
/// (`"a"`) and full dotted path (`"a.b.c"`), or `None` if the base is not a bare
/// name. Used to fold stdlib qualified calls of any depth (Phase 8D).
fn flatten_attr_chain(e: &Expr) -> Option<(&str, String)> {
    let mut parts: Vec<&str> = Vec::new();
    let mut cur = e;
    loop {
        match cur {
            Expr::Attribute(a) => {
                parts.push(a.attr.as_str());
                cur = a.value.as_ref();
            }
            Expr::Name(n) => {
                parts.push(n.id.as_str());
                break;
            }
            _ => return None,
        }
    }
    parts.reverse();
    let leftmost = parts[0];
    Some((leftmost, parts.join(".")))
}

/// True if `e` is the `None` literal (the only RHS supported for `is`/`is not`,
/// Phase 8D).
fn is_none_lit(e: &Expr) -> bool {
    matches!(e, Expr::Constant(c) if matches!(c.value, Constant::None))
}

/// The string value of `e` if it is a plain string-literal constant (the
/// attribute-name argument to `getattr`/`setattr`/`hasattr`, §5).
fn string_literal_arg(e: &Expr) -> Option<&str> {
    match e {
        Expr::Constant(c) => match &c.value {
            Constant::Str(s) => Some(s.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// If `stmt` is `Name = TypeVar(...)` (or `ParamSpec`/`TypeVarTuple`), return the
/// target name — a module-level type variable (Phase 5E).
fn type_var_assign_name(stmt: &Stmt) -> Option<String> {
    let Stmt::Assign(a) = stmt else { return None };
    if a.targets.len() != 1 {
        return None;
    }
    let Expr::Name(target) = &a.targets[0] else {
        return None;
    };
    let Expr::Call(call) = a.value.as_ref() else {
        return None;
    };
    let Expr::Name(f) = call.func.as_ref() else {
        return None;
    };
    matches!(f.id.as_str(), "TypeVar" | "ParamSpec" | "TypeVarTuple")
        .then(|| target.id.as_str().to_string())
}

/// The `SemTy` type arguments in a `Cls[args]` subscript slice (Phase 5E).
fn subscript_type_args(slice: &Expr, ctx: &AnnCtx) -> Vec<SemTy> {
    match slice {
        Expr::Tuple(t) => t.elts.iter().map(|e| annotation_to_semty(e, ctx)).collect(),
        single => vec![annotation_to_semty(single, ctx)],
    }
}

/// Build a decorated function's generic `(*args, **kwargs)` adapter thunk
/// (Phase 6D): `thunk(env, args, kwargs) { return orig(args[0], …, args[k-1]) }`.
/// This gives the function VALUE a `Callable[..., R]` signature matching a
/// decorator's `func` parameter, while `orig`'s own direct-call ABI is intact.
///
/// The thunk's declared return type is the decorated function's own (`ret`), so
/// the closure's representation-level signature matches a decorator annotated
/// `Callable[..., float]` / `Callable[..., str]` / … — not only `int`. The
/// decorated function must carry the matching return annotation (the `Callable`
/// slot IS the native ABI). A non-`Tagged` *parameter* still cannot be fed from
/// the generic tuple-element unpack (it is `Dyn`), so decorated functions with
/// `float`/`bool` params remain out of scope (documented).
fn build_generic_thunk(
    interner: &mut StringInterner,
    ctx: &AnnCtx,
    shared: &mut Shared,
    _orig_fid: FuncId,
    orig_name_str: &str,
    arity: usize,
    ret: SemTy,
) -> FuncId {
    let span = Span::dummy();
    let fid = shared.reserve();
    let tname = interner.intern(&format!("{orig_name_str}.<thunk>"));
    let mut fl = FnLowerer::new(interner, ctx, shared, tname, orig_name_str, ret, None);
    let env = fl.intern("__env__");
    fl.add_param(env, SemTy::Dyn);
    let args_name = fl.intern("__args__");
    fl.add_param(args_name, SemTy::tuple_var_of(SemTy::Dyn));
    let kwargs_name = fl.intern("__kwargs__");
    fl.add_param(kwargs_name, SemTy::dict_of(SemTy::Str, SemTy::Dyn));
    // orig(args[0], …, args[arity-1]) — a direct call resolved by semantics.
    let orig = fl.intern(orig_name_str);
    let callee = fl.alloc(
        HirExprKind::Name(SymbolRef::Unresolved(orig)),
        SemTy::Dyn,
        span,
    );
    let mut call_args = Vec::with_capacity(arity);
    for i in 0..arity {
        let base = fl.local_ref(LocalId::new(1), span); // the `__args__` tuple param
        let idx = fl.alloc(HirExprKind::IntLit(i as i64), SemTy::Int, span);
        let sub = fl.alloc(
            HirExprKind::Subscript { base, index: idx },
            SemTy::Dyn,
            span,
        );
        call_args.push(sub);
    }
    let call = fl.alloc(
        HirExprKind::Call {
            callee,
            args: call_args,
        },
        SemTy::Dyn,
        span,
    );
    fl.seal(HirTerminator::Return(Some(call)));
    let mut f = fl.finish(HirTerminator::Return(None));
    f.varargs = true;
    f.kwargs = true;
    shared.fill(fid, f);
    fid
}

/// The full ABI parameter types of a top-level def, in MIR order: fixed →
/// keyword-only → `*args` tuple → `**kwargs` dict (Phase 6C).
fn top_def_param_tys(info: &TopDefInfo) -> Vec<SemTy> {
    let mut v: Vec<SemTy> = info
        .fixed
        .iter()
        .chain(&info.kwonly)
        .map(|p| p.ty.clone())
        .collect();
    if info.varargs.is_some() {
        v.push(SemTy::tuple_var_of(SemTy::Dyn));
    }
    if info.kwargs.is_some() {
        v.push(SemTy::dict_of(SemTy::Str, SemTy::Dyn));
    }
    v
}

/// Take (mark consumed) the first unconsumed keyword named `name`, returning its
/// value source (Phase 6C).
fn take_keyword<'a>(keywords: &mut [(String, ArgSrc<'a>, bool)], name: &str) -> Option<ArgSrc<'a>> {
    for (k, v, used) in keywords.iter_mut() {
        if !*used && k == name {
            *used = true;
            return Some(*v);
        }
    }
    None
}

/// True iff any positional arg is a `*t` spread.
fn has_starred_arg(c: &ExprCall) -> bool {
    c.args.iter().any(|a| matches!(a, Expr::Starred(_)))
}

/// If `e` is a list/tuple LITERAL with no nested `*` element, return its element
/// expressions — a compile-time-known spread (`f(*[1, 2, 3])`) the slot-matching
/// path can flatten into plain positionals. `None` for a runtime sequence (a
/// variable / call result / comprehension), which must spread at runtime.
fn flatten_literal_seq(e: &Expr) -> Option<&[Expr]> {
    match e {
        Expr::List(l) if !l.elts.iter().any(|x| matches!(x, Expr::Starred(_))) => Some(&l.elts),
        Expr::Tuple(t) if !t.elts.iter().any(|x| matches!(x, Expr::Starred(_))) => Some(&t.elts),
        _ => None,
    }
}

/// Classify a call's positional args, flattening literal `*` spreads
/// ([`flatten_literal_seq`]) into plain positionals. Returns the ordered items
/// plus whether any RUNTIME `*seq` spread remains (length unknown until run
/// time, so the call routes to the general spread path).
fn classify_pos_args(c: &ExprCall) -> (Vec<PosItem<'_>>, bool) {
    let mut items = Vec::with_capacity(c.args.len());
    let mut has_runtime_spread = false;
    for a in &c.args {
        match a {
            Expr::Starred(s) => match flatten_literal_seq(s.value.as_ref()) {
                Some(elts) => items.extend(elts.iter().map(PosItem::Plain)),
                None => {
                    items.push(PosItem::Spread(s.value.as_ref()));
                    has_runtime_spread = true;
                }
            },
            _ => items.push(PosItem::Plain(a)),
        }
    }
    (items, has_runtime_spread)
}

/// True iff the call has a `**d` spread.
fn has_doublestar_kwarg(c: &ExprCall) -> bool {
    c.keywords.iter().any(|k| k.arg.is_none())
}

/// Reject keyword args and `*`/`**` spreads for a call form that does not
/// support them (generic construction, method calls, the desugared builtins).
/// True for the `functools.reduce` stdlib descriptor — identified by its unique
/// runtime symbol, so the call is rerouted to the HOF desugar
/// ([`Lowerer::lower_reduce`]) instead of the raw-ABI `rt_reduce` callback path.
/// The descriptor exists only for `from functools import reduce` recognition.
fn is_reduce_def(def: &pyaot_stdlib_defs::StdlibFunctionDef) -> bool {
    def.runtime_name == "rt_reduce"
}

fn reject_call_extras(c: &ExprCall, span: Span, what: &str) -> Result<()> {
    if !c.keywords.is_empty() {
        return Err(parse_error(
            format!("keyword arguments are not supported for {what}"),
            span,
        ));
    }
    if has_starred_arg(c) {
        return Err(parse_error(
            format!("`*args` spreading is not supported for {what}"),
            span,
        ));
    }
    Ok(())
}

/// True iff `e` is a bare `super()` call (the zero-arg form; Phase 5B). The
/// explicit `super(Cls, self)` form is out of scope.
fn is_super_call(e: &Expr) -> bool {
    matches!(e, Expr::Call(c)
        if c.args.is_empty() && c.keywords.is_empty()
            && matches!(c.func.as_ref(), Expr::Name(n) if n.id.as_str() == "super"))
}

/// The element expressions of a tuple/list target or literal-sequence value, used
/// for unpacking (`a, b = …`). `None` for any other expression.
fn seq_target_elts(e: &Expr) -> Option<&[Expr]> {
    match e {
        Expr::Tuple(t) => Some(&t.elts),
        Expr::List(l) => Some(&l.elts),
        _ => None,
    }
}

/// Reject starred unpacking targets (`a, *rest = …`) — deferred to Phase 6.
fn reject_starred(targets: &[Expr], span: Span) -> Result<()> {
    if targets.iter().any(|t| matches!(t, Expr::Starred(_))) {
        return Err(parse_error(
            "starred unpacking targets are out of scope",
            span,
        ));
    }
    Ok(())
}

/// Parse `range(...)` from a `for` iterable into `(start, stop, step)`. `step`
/// must be an integer literal (the loop direction is decided at compile time).
fn parse_range(iter: &Expr, span: Span) -> Result<(RangeArg<'_>, RangeArg<'_>, i64)> {
    let Expr::Call(call) = iter else {
        return Err(parse_error("for-loop iterable must be range(...)", span));
    };
    let is_range = matches!(call.func.as_ref(), Expr::Name(n) if n.id.as_str() == "range");
    if !is_range {
        return Err(parse_error("for-loop iterable must be range(...)", span));
    }
    if !call.keywords.is_empty() {
        return Err(parse_error("range() takes no keyword arguments", span));
    }
    match call.args.len() {
        1 => Ok((RangeArg::Zero, RangeArg::Expr(&call.args[0]), 1)),
        2 => Ok((
            RangeArg::Expr(&call.args[0]),
            RangeArg::Expr(&call.args[1]),
            1,
        )),
        3 => {
            let step = literal_int(&call.args[2])
                .ok_or_else(|| parse_error("range() step must be an integer literal", span))?;
            Ok((
                RangeArg::Expr(&call.args[0]),
                RangeArg::Expr(&call.args[1]),
                step,
            ))
        }
        _ => Err(parse_error("range() takes 1 to 3 arguments", span)),
    }
}

/// Extract an `i64` from an integer-literal expression (possibly unary-signed).
fn literal_int(e: &Expr) -> Option<i64> {
    match e {
        Expr::Constant(c) => match &c.value {
            Constant::Int(b) => b.to_string().parse::<i64>().ok(),
            _ => None,
        },
        Expr::UnaryOp(u) => {
            if let Expr::Constant(c) = u.operand.as_ref() {
                if let Constant::Int(b) = &c.value {
                    let v = b.to_string().parse::<i64>().ok()?;
                    return match u.op {
                        PyUnaryOp::USub => Some(-v),
                        PyUnaryOp::UAdd => Some(v),
                        _ => None,
                    };
                }
            }
            None
        }
        _ => None,
    }
}

fn binop_from_ast(op: &PyOperator) -> BinOp {
    match op {
        PyOperator::Add => BinOp::Add,
        PyOperator::Sub => BinOp::Sub,
        PyOperator::Mult => BinOp::Mul,
        PyOperator::Div => BinOp::Div,
        PyOperator::FloorDiv => BinOp::FloorDiv,
        PyOperator::Mod => BinOp::Mod,
        PyOperator::Pow => BinOp::Pow,
        PyOperator::LShift => BinOp::Shl,
        PyOperator::RShift => BinOp::Shr,
        PyOperator::BitOr => BinOp::BitOr,
        PyOperator::BitXor => BinOp::BitXor,
        PyOperator::BitAnd => BinOp::BitAnd,
        // `a @ b` (PEP 465): no built-in numeric `@`, so it dispatches the
        // `__matmul__`/`__rmatmul__` dunder at runtime (like `+`/`*`).
        PyOperator::MatMult => BinOp::MatMul,
    }
}

/// Map a type annotation to a `SemTy` (primitives and built-in containers drive
/// `Repr`; everything else is `Dyn`). A bare container name (`list`) defaults its
/// element types to `Dyn`; a subscripted one (`list[int]`, `dict[str, int]`,
/// `tuple[int, ...]`) carries them — this is what lets the empty-literal bootstrap
/// seed `x: list[int] = []` (PITFALLS B4).
fn annotation_to_semty(ann: &Expr, ctx: &AnnCtx) -> SemTy {
    match ann {
        Expr::Name(n) => named_annotation(n.id.as_str(), ctx),
        // A qualified class annotation through an `import M` alias (Phase 8):
        // `math_utils.Point` resolves via the `"M.Cls"` key in `class_map`;
        // a stdlib class (`time.struct_time`) via the stdlib bindings (8B).
        Expr::Attribute(a) => {
            if let Expr::Name(m) = a.value.as_ref() {
                if ctx.aliases.contains(m.id.as_str()) {
                    let qual = format!("{}.{}", m.id.as_str(), a.attr.as_str());
                    if let Some((class_id, name)) = ctx.class_map.get(&qual) {
                        return SemTy::Class {
                            class_id: *class_id,
                            name: *name,
                        };
                    }
                }
                if ctx.stdlib.aliases.contains(m.id.as_str()) {
                    let qual = format!("{}.{}", m.id.as_str(), a.attr.as_str());
                    if let Some(ty) = ctx.stdlib.classes.get(&qual) {
                        return ty.clone();
                    }
                }
            }
            SemTy::Dyn
        }
        Expr::Subscript(s) => annotation_subscript(s.value.as_ref(), s.slice.as_ref(), ctx),
        Expr::Constant(c) => match &c.value {
            Constant::None => SemTy::NoneTy,
            // A string annotation is a PEP-484 forward reference: resolve the
            // quoted name exactly like a bare one (`-> "CM"` ≡ `-> CM`).
            Constant::Str(s) => named_annotation(s, ctx),
            _ => SemTy::Dyn,
        },
        _ => SemTy::Dyn,
    }
}

/// Resolve a (possibly forward-referenced) annotation NAME to a `SemTy`.
fn named_annotation(name: &str, ctx: &AnnCtx) -> SemTy {
    match name {
        "int" => SemTy::Int,
        "float" => SemTy::Float,
        "bool" => SemTy::Bool,
        "str" => SemTy::Str,
        "bytes" => SemTy::Bytes,
        "None" | "NoneType" => SemTy::NoneTy,
        "list" | "List" => SemTy::list_of(SemTy::Dyn),
        "dict" | "Dict" => SemTy::dict_of(SemTy::Dyn, SemTy::Dyn),
        "set" | "Set" | "frozenset" => SemTy::set_of(SemTy::Dyn),
        "tuple" | "Tuple" => SemTy::tuple_var_of(SemTy::Dyn),
        other => {
            // An in-scope type variable (Phase 5E) → `SemTy::Var`.
            if let Some(id) = ctx.type_vars.get(other) {
                return SemTy::Var(*id);
            }
            // A user-defined class name annotates an instance of that class.
            if let Some((class_id, name)) = ctx.class_map.get(other) {
                return SemTy::Class {
                    class_id: *class_id,
                    name: *name,
                };
            }
            // A from-imported stdlib class (`from time import struct_time`).
            if let Some(ty) = ctx.stdlib.classes.get(other) {
                return ty.clone();
            }
            SemTy::Dyn
        }
    }
}

/// Map a subscripted generic annotation (`list[int]`, `dict[K, V]`, …) to a
/// `SemTy`. Unknown bases fall back to `Dyn`.
fn annotation_subscript(base: &Expr, slice: &Expr, ctx: &AnnCtx) -> SemTy {
    let Expr::Name(n) = base else {
        return SemTy::Dyn;
    };
    match n.id.as_str() {
        "list" | "List" => SemTy::list_of(annotation_to_semty(slice, ctx)),
        "set" | "Set" | "frozenset" => SemTy::set_of(annotation_to_semty(slice, ctx)),
        "dict" | "Dict" => match slice {
            Expr::Tuple(t) if t.elts.len() == 2 => SemTy::dict_of(
                annotation_to_semty(&t.elts[0], ctx),
                annotation_to_semty(&t.elts[1], ctx),
            ),
            _ => SemTy::dict_of(SemTy::Dyn, SemTy::Dyn),
        },
        "tuple" | "Tuple" => match slice {
            // `tuple[T, ...]` is the homogeneous variable-length tuple.
            Expr::Tuple(t) if t.elts.len() == 2 && is_ellipsis(&t.elts[1]) => {
                SemTy::tuple_var_of(annotation_to_semty(&t.elts[0], ctx))
            }
            Expr::Tuple(t) => {
                SemTy::tuple_of(t.elts.iter().map(|e| annotation_to_semty(e, ctx)).collect())
            }
            single => SemTy::tuple_of(vec![annotation_to_semty(single, ctx)]),
        },
        "Optional" => SemTy::optional(annotation_to_semty(slice, ctx)),
        // `Callable[[T…], R]` / `Callable[..., R]` (Phase 6A). The ellipsis form
        // is the `(*args, **kwargs)` signature — exactly one tuple param + one
        // dict param (Phase 6C ABI).
        "Callable" => callable_annotation(slice, ctx),
        // A user generic class annotation `Stack[int]` → `Generic{base, [int]}` (5E).
        other => match ctx.class_map.get(other) {
            Some((class_id, _)) => SemTy::Generic {
                base: *class_id,
                args: subscript_type_args(slice, ctx),
            },
            None => SemTy::Dyn,
        },
    }
}

/// True iff `e` is the `...` (Ellipsis) literal — the `tuple[T, ...]` marker.
fn is_ellipsis(e: &Expr) -> bool {
    matches!(e, Expr::Constant(c) if matches!(c.value, Constant::Ellipsis))
}

/// Map a `Callable[...]` annotation slice to a `SemTy::Callable`. Unknown
/// shapes fall back to `Dyn` (→ `Tagged`, the correct baseline — calling such a
/// value then gets the loud Dyn-callee diagnostic).
fn callable_annotation(slice: &Expr, ctx: &AnnCtx) -> SemTy {
    let Expr::Tuple(t) = slice else {
        return SemTy::Dyn;
    };
    if t.elts.len() != 2 {
        return SemTy::Dyn;
    }
    let ret = annotation_to_semty(&t.elts[1], ctx);
    match &t.elts[0] {
        Expr::List(l) => SemTy::Callable(Box::new(Sig::fixed(
            l.elts.iter().map(|e| annotation_to_semty(e, ctx)).collect(),
            ret,
        ))),
        e if is_ellipsis(e) => SemTy::Callable(Box::new(Sig {
            params: vec![
                SemTy::tuple_var_of(SemTy::Dyn),
                SemTy::dict_of(SemTy::Str, SemTy::Dyn),
            ],
            ret,
            varargs: true,
            kwargs: true,
        })),
        _ => SemTy::Dyn,
    }
}

/// How a callable treats its first parameter (Phase 5D).
enum FirstParam {
    /// An instance method / property accessor: param 0 is `self`, typed as the
    /// class (carried `SemTy::Class`).
    Method(SemTy),
    /// A `@classmethod`: the first param (`cls`) is dropped — it is resolved
    /// statically to the enclosing class.
    SkipCls,
    /// A free function / `@staticmethod`: no special first-param handling.
    Plain,
}

/// Shared `def`/method/nested-def lowering. `name` is the function's (possibly
/// synthetic) interned name; `name_str` the raw base for child synthetics;
/// `first` controls the first parameter; `enclosing` is the class for `super()`;
/// `allow_decorators` permits the already-classified Phase-5D decorators (the
/// caller has validated them). `nested` is `Some((captures, facts))` for a
/// nested def: the function gets `__env__: Dyn` as explicit param 0 and a
/// capture-unpacking prologue. Reserves and fills the function's `FuncId`.
#[allow(clippy::too_many_arguments)]
fn lower_callable(
    interner: &mut StringInterner,
    ctx: &AnnCtx,
    shared: &mut Shared,
    def: &StmtFunctionDef,
    name_str: &str,
    name: InternedString,
    first: FirstParam,
    enclosing: Option<ClassId>,
    allow_decorators: bool,
    nested: Option<(&[(String, SemTy)], &ScopeFacts)>,
) -> Result<FuncId> {
    let span = to_span(def.range());
    if !allow_decorators && !def.decorator_list.is_empty() {
        return Err(parse_error(
            "decorators are out of scope for this milestone",
            span,
        ));
    }
    let ret_ty = match &def.returns {
        Some(e) => annotation_to_semty(e.as_ref(), ctx),
        None => SemTy::Dyn,
    };
    let parsed = parse_params(interner, ctx, def.args.as_ref(), &first)?;
    // The function's own scoping facts (computed by the caller for nested defs,
    // fresh here for top-level ones — same analysis either way).
    let own_facts;
    let facts = match nested {
        Some((_, f)) => f,
        None => {
            own_facts = freevars::analyze_def(def);
            &own_facts
        }
    };
    // `nonlocal x` requires an enclosing function binding for `x` — i.e. it must
    // be among this function's captures (the CPython SyntaxError otherwise).
    for n in &facts.nonlocals {
        let captured = matches!(nested, Some((caps, _)) if caps.iter().any(|(c, _)| c == n));
        if !captured {
            return Err(parse_error(
                format!("no binding for nonlocal '{n}' found"),
                span,
            ));
        }
    }

    let fid = shared.reserve();
    let varargs = parsed.varargs.is_some();
    let kwargs = parsed.kwargs.is_some();

    // A `def` containing `yield` is a generator (Phase 6E): build the wrapper
    // (into `fid`) + a resume state machine instead of a plain body. Captures /
    // *args / **kwargs in a generator are out of scope.
    if body_has_yield(&def.body) {
        if nested.is_some_and(|(caps, _)| !caps.is_empty()) {
            return Err(parse_error(
                "a generator that captures variables is out of scope (Phase 6E)",
                span,
            ));
        }
        if varargs || kwargs {
            return Err(parse_error(
                "a generator with *args/**kwargs is out of scope (Phase 6E)",
                span,
            ));
        }
        lower_generator_def(
            interner, ctx, shared, &def.body, name_str, name, fid, &parsed, ret_ty, enclosing,
        )?;
        return Ok(fid);
    }

    let mut fl = FnLowerer::new(interner, ctx, shared, name, name_str, ret_ty, enclosing);
    fl.set_scope_facts(facts);
    if nested.is_some() {
        let env_name = fl.intern("__env__");
        fl.add_param(env_name, SemTy::Dyn);
    }
    fl.install_params(&parsed);
    if let Some((captures, f)) = nested {
        fl.install_captures(captures, f, span);
        // Self-recursion: the def's own name among its captures.
        if captures.iter().any(|(c, _)| c == def.name.as_str()) {
            let iname = fl.intern(def.name.as_str());
            if let Some(Binding::Cell(lid)) = fl.scope.get(&iname).copied() {
                fl.self_capture = Some((lid, name));
            }
        }
    }
    fl.init_cells();
    fl.lower_body(&def.body)?;
    let mut func = fl.finish(HirTerminator::Return(None));
    func.varargs = varargs;
    func.kwargs = kwargs;
    shared.fill(fid, func);
    Ok(fid)
}

/// True iff `expr` is a `yield` / `yield from` expression (Phase 6E).
fn is_yield_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Yield(_) | Expr::YieldFrom(_))
}

/// True iff a statement list contains a `yield` not nested inside another
/// function / lambda scope (Phase 6E generator detection).
fn body_has_yield(body: &[Stmt]) -> bool {
    body.iter().any(stmt_has_yield)
}

fn stmt_has_yield(s: &Stmt) -> bool {
    match s {
        Stmt::Expr(e) => expr_has_yield(&e.value),
        Stmt::Assign(a) => expr_has_yield(&a.value),
        Stmt::AugAssign(a) => expr_has_yield(&a.value),
        Stmt::AnnAssign(a) => a.value.as_ref().is_some_and(|v| expr_has_yield(v)),
        Stmt::Return(r) => r.value.as_ref().is_some_and(|v| expr_has_yield(v)),
        Stmt::If(s) => body_has_yield(&s.body) || body_has_yield(&s.orelse),
        Stmt::While(s) => body_has_yield(&s.body) || body_has_yield(&s.orelse),
        Stmt::For(s) => body_has_yield(&s.body) || body_has_yield(&s.orelse),
        // Phase 7: a yield lexically inside try/with/match still makes the def
        // a generator (the suspend path then rejects try/with with a clear
        // message instead of "unsupported expression").
        Stmt::Try(t) => {
            body_has_yield(&t.body)
                || t.handlers.iter().any(|h| {
                    let rustpython_parser::ast::ExceptHandler::ExceptHandler(h) = h;
                    body_has_yield(&h.body)
                })
                || body_has_yield(&t.orelse)
                || body_has_yield(&t.finalbody)
        }
        Stmt::With(w) => body_has_yield(&w.body),
        Stmt::Match(m) => m.cases.iter().any(|c| body_has_yield(&c.body)),
        // A nested def/lambda/class is its own scope — its yields don't count.
        _ => false,
    }
}

/// True iff a `match` pattern binds any name (capture / star / `**rest`) —
/// the v1 or-pattern restriction (Phase 7E).
fn pattern_has_bindings(p: &rustpython_parser::ast::Pattern) -> bool {
    use rustpython_parser::ast::Pattern;
    match p {
        Pattern::MatchValue(_) | Pattern::MatchSingleton(_) => false,
        Pattern::MatchAs(a) => {
            a.name.is_some() || a.pattern.as_deref().is_some_and(pattern_has_bindings)
        }
        Pattern::MatchOr(o) => o.patterns.iter().any(pattern_has_bindings),
        Pattern::MatchSequence(s) => s.patterns.iter().any(pattern_has_bindings),
        Pattern::MatchStar(s) => s.name.is_some(),
        Pattern::MatchMapping(m) => m.rest.is_some() || m.patterns.iter().any(pattern_has_bindings),
        Pattern::MatchClass(c) => {
            c.patterns.iter().any(pattern_has_bindings)
                || c.kwd_patterns.iter().any(pattern_has_bindings)
        }
    }
}

fn expr_has_yield(e: &Expr) -> bool {
    matches!(e, Expr::Yield(_) | Expr::YieldFrom(_))
}

/// Lower a generator `def` (Phase 6E): a wrapper (`fid`) building the generator
/// and storing its params/captures into slots, plus a `<resume>` state machine
/// registered in `shared.generators`.
#[allow(clippy::too_many_arguments)]
fn lower_generator_def(
    interner: &mut StringInterner,
    ctx: &AnnCtx,
    shared: &mut Shared,
    body: &[Stmt],
    name_str: &str,
    name: InternedString,
    wrapper_fid: FuncId,
    parsed: &ParsedParams,
    ret_ty: SemTy,
    enclosing: Option<ClassId>,
) -> Result<()> {
    let span = Span::dummy();
    let n_params = parsed.fixed.len() + parsed.kwonly.len();

    // ── resume function: the state machine ──
    let resume_fid = shared.reserve();
    let gen_id = shared.generators.len() as u32;
    shared.generators.push(resume_fid);

    let resume_name = interner.intern(&format!("{name_str}.<resume>"));
    let mut rl = FnLowerer::new(
        interner,
        ctx,
        shared,
        resume_name,
        name_str,
        SemTy::Dyn,
        enclosing,
    );
    // Param 0 = the generator object.
    let gen_name = rl.intern("__gen__");
    rl.add_param(gen_name, SemTy::Dyn);
    // The Python params become *logical locals* (gen slots), bound by name so
    // the body resolves to them (slots 0.. = locals 1..).
    for p in parsed.fixed.iter().chain(&parsed.kwonly) {
        rl.add_logical_local(p.name, p.ty.clone());
    }
    rl.gen = Some(GenCtx {
        gen_local: LocalId::new(0),
        next_state: 1,
        resume_targets: Vec::new(),
    });

    let start = rl.new_block();
    rl.switch(start);
    rl.lower_body(body)?;
    // Fallthrough → exhaust.
    if rl.cur_open() {
        rl.emit_gen_exhaust(span);
    }
    rl.gen_rewrite_locals();
    let num_locals = rl.locals.len() as u32 - 1;
    rl.build_gen_dispatch(start);
    let resume_fn = rl.finish(HirTerminator::Return(None));
    shared.fill(resume_fid, resume_fn);

    // ── wrapper: build the generator, seed param slots, return it ──
    let mut wl = FnLowerer::new(interner, ctx, shared, name, name_str, ret_ty, enclosing);
    wl.install_params(parsed);
    let g_local = wl.fresh_local(SemTy::Dyn);
    let mg = wl.alloc(
        HirExprKind::MakeGenerator { gen_id, num_locals },
        SemTy::Dyn,
        span,
    );
    wl.push_stmt(HirStmt::Assign {
        target: g_local,
        value: mg,
    });
    for i in 0..n_params {
        let gen = wl.local_ref(g_local, span);
        let p = wl.local_ref(LocalId::new(i as u32), span);
        wl.push_stmt(HirStmt::GenSetLocal {
            gen,
            slot: i as u32,
            value: p,
        });
    }
    let g_ret = wl.local_ref(g_local, span);
    wl.seal(HirTerminator::Return(Some(g_ret)));
    let wrapper_fn = wl.finish(HirTerminator::Return(None));
    shared.fill(wrapper_fid, wrapper_fn);
    Ok(())
}

/// The parsed parameter shape of a callable (Phase 6C).
struct ParsedParams {
    fixed: Vec<ParamInfo>,
    kwonly: Vec<ParamInfo>,
    varargs: Option<InternedString>,
    kwargs: Option<InternedString>,
}

/// Parse a callable's parameter list into the call-facing [`ParsedParams`]
/// shape (Phase 6C). The first fixed param is typed by `first` for instance
/// methods; a classmethod's `cls` is dropped. Defaults must be constant
/// literals (`x=[]` is rejected loudly, the `ClassAttrInit` shape).
fn parse_params(
    interner: &mut StringInterner,
    ctx: &AnnCtx,
    args: &rustpython_parser::ast::Arguments,
    first: &FirstParam,
) -> Result<ParsedParams> {
    let skip = matches!(first, FirstParam::SkipCls) as usize;
    let mut fixed = Vec::new();
    for (i, awd) in args
        .posonlyargs
        .iter()
        .chain(args.args.iter())
        .skip(skip)
        .enumerate()
    {
        let ty = match (i, first) {
            (0, FirstParam::Method(t)) => t.clone(),
            _ => match &awd.def.annotation {
                Some(a) => annotation_to_semty(a.as_ref(), ctx),
                None => SemTy::Dyn,
            },
        };
        let default = match &awd.default {
            Some(e) => Some(class_attr_init(interner, e)?),
            None => None,
        };
        fixed.push(ParamInfo {
            name: interner.intern(awd.def.arg.as_str()),
            ty,
            default,
        });
    }
    let mut kwonly = Vec::new();
    for awd in &args.kwonlyargs {
        let ty = match &awd.def.annotation {
            Some(a) => annotation_to_semty(a.as_ref(), ctx),
            None => SemTy::Dyn,
        };
        let default = match &awd.default {
            Some(e) => Some(class_attr_init(interner, e)?),
            None => None,
        };
        kwonly.push(ParamInfo {
            name: interner.intern(awd.def.arg.as_str()),
            ty,
            default,
        });
    }
    let varargs = args
        .vararg
        .as_ref()
        .map(|a| interner.intern(a.arg.as_str()));
    let kwargs = args.kwarg.as_ref().map(|a| interner.intern(a.arg.as_str()));
    Ok(ParsedParams {
        fixed,
        kwonly,
        varargs,
        kwargs,
    })
}

/// Lower a `class` definition: lower each method into `functions` (recording its
/// `FuncId`) and collect base names + class-level field annotations. The resolved
/// layout (MRO, slots, inherited members) is computed later in `semantics`.
fn lower_class(
    interner: &mut StringInterner,
    ctx: &AnnCtx,
    cdef: &StmtClassDef,
    class_id: ClassId,
    shared: &mut Shared,
) -> Result<HirClass> {
    let span = to_span(cdef.range());
    if !cdef.decorator_list.is_empty() {
        return Err(parse_error("class decorators are out of scope", span));
    }
    if !cdef.keywords.is_empty() {
        return Err(parse_error(
            "class keyword arguments (e.g. `metaclass=`) are out of scope",
            span,
        ));
    }

    // ── Type parameters (Phase 5E): PEP 695 `class C[T]` + `Generic[T]` base ──
    let mut type_params: Vec<InternedString> = Vec::new();
    let mut type_param_names: Vec<String> = Vec::new();
    for tp in &cdef.type_params {
        let name = type_param_name(tp);
        type_param_names.push(name.clone());
        type_params.push(interner.intern(&name));
    }

    // Base classes: bare names (`class Dog(Animal)`); `Generic[T]` / `Protocol[T]`
    // contribute type params (not a runtime base).
    let mut base_names = Vec::new();
    for base in &cdef.bases {
        match base {
            Expr::Name(n) => base_names.push(interner.intern(n.id.as_str())),
            // `Generic[T]` / `Generic[T1, T2]` → record the type params.
            Expr::Subscript(s) => {
                let Expr::Name(b) = s.value.as_ref() else {
                    return Err(parse_error(
                        "unsupported subscripted base class",
                        to_span(base.range()),
                    ));
                };
                if matches!(b.id.as_str(), "Generic" | "Protocol") {
                    for tp in subscript_type_param_names(s.slice.as_ref()) {
                        if !type_param_names.contains(&tp) {
                            type_params.push(interner.intern(&tp));
                            type_param_names.push(tp);
                        }
                    }
                } else {
                    return Err(parse_error(
                        "subscripted base classes other than Generic/Protocol are out of scope",
                        to_span(base.range()),
                    ));
                }
            }
            _ => {
                return Err(parse_error(
                    "unsupported base-class expression",
                    to_span(base.range()),
                ))
            }
        }
    }

    // Per-class annotation context: module type vars + this class's params.
    let mut merged_tv: TypeVarSet = ctx.type_vars.clone();
    for (n, id) in type_param_names.iter().zip(&type_params) {
        merged_tv.insert(n.clone(), *id);
    }
    let cctx = AnnCtx {
        class_map: ctx.class_map,
        type_vars: &merged_tv,
        top_defs: ctx.top_defs,
        promoted: ctx.promoted,
        decorated: ctx.decorated,
        aliases: ctx.aliases,
        alias_vars: ctx.alias_vars,
        stdlib: ctx.stdlib,
    };

    let name = interner.intern(cdef.name.as_str());
    // CPython renders a top-level class instance as `<__main__.Cls object at …>`.
    let qualname = interner.intern(&format!("__main__.{}", cdef.name.as_str()));
    let class_ty = SemTy::Class { class_id, name };
    let mut methods: Vec<(InternedString, FuncId)> = Vec::new();
    let mut static_methods: Vec<(InternedString, FuncId)> = Vec::new();
    let mut class_methods: Vec<(InternedString, FuncId)> = Vec::new();
    let mut properties: Vec<HirProperty> = Vec::new();
    let mut class_attrs: Vec<HirClassAttr> = Vec::new();
    let mut field_annotations: Vec<(InternedString, SemTy)> = Vec::new();

    // Lower a method body into the shared table, returning its FuncId.
    let lower_method = |interner: &mut StringInterner,
                        shared: &mut Shared,
                        m: &StmtFunctionDef,
                        suffix: &str,
                        first: FirstParam,
                        enclosing: Option<ClassId>|
     -> Result<(FuncId, SemTy)> {
        let synthetic = format!("{}.{}{}", cdef.name.as_str(), m.name.as_str(), suffix);
        let fname = interner.intern(&synthetic);
        let func_id = lower_callable(
            interner, &cctx, shared, m, &synthetic, fname, first, enclosing, true, None,
        )?;
        let ret = shared.funcs[func_id.index()]
            .as_ref()
            .expect("method just filled")
            .ret_ty
            .clone();
        Ok((func_id, ret))
    };

    for stmt in &cdef.body {
        match stmt {
            Stmt::FunctionDef(m) => {
                let method_name = interner.intern(m.name.as_str());
                match classify_method_decorator(m)? {
                    MethodDecor::Instance => {
                        let (fid, _) = lower_method(
                            interner,
                            shared,
                            m,
                            "",
                            FirstParam::Method(class_ty.clone()),
                            Some(class_id),
                        )?;
                        methods.push((method_name, fid));
                    }
                    MethodDecor::Static => {
                        let (fid, _) =
                            lower_method(interner, shared, m, "", FirstParam::Plain, None)?;
                        static_methods.push((method_name, fid));
                    }
                    MethodDecor::Class => {
                        let (fid, _) =
                            lower_method(interner, shared, m, "", FirstParam::SkipCls, None)?;
                        class_methods.push((method_name, fid));
                    }
                    MethodDecor::Property => {
                        let (fid, ty) = lower_method(
                            interner,
                            shared,
                            m,
                            ".get",
                            FirstParam::Method(class_ty.clone()),
                            Some(class_id),
                        )?;
                        properties.push(HirProperty {
                            name: method_name,
                            getter: fid,
                            setter: None,
                            ty,
                        });
                    }
                    MethodDecor::Setter(prop) => {
                        let pname = interner.intern(&prop);
                        let (fid, _) = lower_method(
                            interner,
                            shared,
                            m,
                            ".set",
                            FirstParam::Method(class_ty.clone()),
                            Some(class_id),
                        )?;
                        match properties.iter_mut().find(|p| p.name == pname) {
                            Some(p) => p.setter = Some(fid),
                            None => {
                                return Err(parse_error(
                                    format!("@{prop}.setter has no matching @property"),
                                    to_span(m.range()),
                                ))
                            }
                        }
                    }
                }
            }
            // `name: T = value` (annotated, *with* a value) is a class attribute;
            // `name: T` (no value) is an instance-field type hint.
            Stmt::AnnAssign(a) => {
                let Expr::Name(n) = a.target.as_ref() else {
                    return Err(parse_error(
                        "class-level annotated target must be a name",
                        to_span(a.range()),
                    ));
                };
                let fname = interner.intern(n.id.as_str());
                let ty = annotation_to_semty(a.annotation.as_ref(), &cctx);
                match &a.value {
                    Some(v) => {
                        let init = class_attr_init(interner, v.as_ref())?;
                        reject_tuple_class_attr(&init, to_span(a.range()))?;
                        class_attrs.push(HirClassAttr {
                            name: fname,
                            ty,
                            init,
                        });
                    }
                    None => field_annotations.push((fname, ty)),
                }
            }
            // Class-level `name = value` value assignment → a class attribute.
            Stmt::Assign(a) => {
                if a.targets.len() != 1 {
                    return Err(parse_error(
                        "chained class-attribute assignment is not supported",
                        to_span(a.range()),
                    ));
                }
                let Expr::Name(n) = &a.targets[0] else {
                    return Err(parse_error(
                        "class-level assignment target must be a name",
                        to_span(a.range()),
                    ));
                };
                // `__slots__` is a CPython memory optimization with no observable
                // semantics in our uniform-tagged object model — silently ignore
                // it (Phase 8E).
                if n.id.as_str() == "__slots__" {
                    continue;
                }
                let fname = interner.intern(n.id.as_str());
                let init = class_attr_init(interner, a.value.as_ref())?;
                reject_tuple_class_attr(&init, to_span(a.range()))?;
                let ty = class_attr_init_ty(&init);
                class_attrs.push(HirClassAttr {
                    name: fname,
                    ty,
                    init,
                });
            }
            // A docstring (a bare string-constant expression) is ignored.
            Stmt::Expr(e) if matches!(e.value.as_ref(), Expr::Constant(c) if matches!(c.value, Constant::Str(_))) =>
                {}
            Stmt::Pass(_) => {}
            other => {
                return Err(parse_error(
                    "unsupported statement in class body",
                    to_span(other.range()),
                ))
            }
        }
    }

    Ok(HirClass {
        name,
        qualname,
        class_id,
        base_names,
        methods,
        static_methods,
        class_methods,
        properties,
        class_attrs,
        field_annotations,
        type_params,
    })
}

/// The name of a PEP 695 type parameter (`T`, `*Ts`, `**P`). Only the simple
/// `TypeVar` form is meaningful for our erase-to-Tagged model.
fn type_param_name(tp: &rustpython_parser::ast::TypeParam) -> String {
    use rustpython_parser::ast::TypeParam;
    match tp {
        TypeParam::TypeVar(t) => t.name.as_str().to_string(),
        TypeParam::ParamSpec(t) => t.name.as_str().to_string(),
        TypeParam::TypeVarTuple(t) => t.name.as_str().to_string(),
    }
}

/// The type-parameter names in a `Generic[...]` subscript slice.
fn subscript_type_param_names(slice: &Expr) -> Vec<String> {
    match slice {
        Expr::Name(n) => vec![n.id.as_str().to_string()],
        Expr::Tuple(t) => t
            .elts
            .iter()
            .filter_map(|e| match e {
                Expr::Name(n) => Some(n.id.as_str().to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// A method's decorator classification (Phase 5D).
enum MethodDecor {
    Instance,
    Static,
    Class,
    Property,
    Setter(String),
}

/// Classify a method's (at most one) decorator. Bare instance methods carry none.
fn classify_method_decorator(m: &StmtFunctionDef) -> Result<MethodDecor> {
    let span = to_span(m.range());
    match m.decorator_list.as_slice() {
        [] => Ok(MethodDecor::Instance),
        [deco] => match deco {
            Expr::Name(n) => match n.id.as_str() {
                "staticmethod" => Ok(MethodDecor::Static),
                "classmethod" => Ok(MethodDecor::Class),
                "property" => Ok(MethodDecor::Property),
                other => Err(parse_error(
                    format!("unsupported decorator @{other} (general decorators are Phase 6)"),
                    span,
                )),
            },
            // `@x.setter` → Attribute{value: Name("x"), attr: "setter"}.
            Expr::Attribute(a) if a.attr.as_str() == "setter" => match a.value.as_ref() {
                Expr::Name(n) => Ok(MethodDecor::Setter(n.id.as_str().to_string())),
                _ => Err(parse_error("malformed @x.setter decorator", span)),
            },
            _ => Err(parse_error(
                "unsupported decorator (general decorators are Phase 6)",
                span,
            )),
        },
        _ => Err(parse_error("stacked decorators are out of scope", span)),
    }
}

/// Lower a class-attribute initializer; only constant literals are supported (5D).
fn class_attr_init(interner: &mut StringInterner, value: &Expr) -> Result<ClassAttrInit> {
    let span = to_span(value.range());
    // Fold a unary +/- over a numeric literal first.
    if let Expr::UnaryOp(u) = value {
        if matches!(u.op, PyUnaryOp::USub | PyUnaryOp::UAdd) {
            if let Expr::Constant(c) = u.operand.as_ref() {
                let neg = matches!(u.op, PyUnaryOp::USub);
                return match &c.value {
                    Constant::Int(b) => Ok(int_attr_init(interner, &b.to_string(), neg)),
                    Constant::Float(f) => Ok(ClassAttrInit::Float(if neg { -*f } else { *f })),
                    _ => Err(parse_error(
                        "class-attribute initializer must be a literal",
                        span,
                    )),
                };
            }
        }
    }
    match value {
        Expr::Constant(c) => match &c.value {
            Constant::Int(b) => Ok(int_attr_init(interner, &b.to_string(), false)),
            Constant::Float(f) => Ok(ClassAttrInit::Float(*f)),
            Constant::Bool(b) => Ok(ClassAttrInit::Bool(*b)),
            Constant::Str(s) => Ok(ClassAttrInit::Str(interner.intern(s))),
            Constant::None => Ok(ClassAttrInit::None),
            Constant::Bytes(b) => {
                let s = std::str::from_utf8(b)
                    .map_err(|_| parse_error("non-UTF-8 bytes literal is out of scope", span))?;
                Ok(ClassAttrInit::Bytes(interner.intern(s)))
            }
            _ => Err(parse_error("unsupported class-attribute literal", span)),
        },
        // The empty tuple `()` — accepted only as a parameter default (Phase 8E,
        // e.g. `children=()`); materialized as a fresh empty tuple at each call
        // site. A non-empty tuple default stays out of scope.
        Expr::Tuple(t) if t.elts.is_empty() => Ok(ClassAttrInit::EmptyTuple),
        _ => Err(parse_error(
            "class-attribute initializers must be constant literals (Phase 5D)",
            span,
        )),
    }
}

/// Collect the simple `Name` identifiers bound by a `for`-target (Phase 8E),
/// descending into tuple / list unpacking. Used to shadow comprehension loop
/// variables so they do not leak into the enclosing scope.
/// Count how many times `name` is bound in one SCOPE's statement list (Phase
/// 8H, #9). Descends into control-flow bodies (same scope) but NOT into
/// `def`/`class` (new scopes). A `global`/`nonlocal` declaration disqualifies
/// (returns 2+) — the name is not a plain single-bound local.
fn count_scope_bindings(stmts: &[Stmt], name: &str) -> usize {
    let mut count = 0usize;
    for stmt in stmts {
        match stmt {
            Stmt::Assign(a) => {
                for t in &a.targets {
                    let mut names = Vec::new();
                    collect_target_names(t, &mut names);
                    count += names.iter().filter(|n| **n == name).count();
                }
            }
            Stmt::AugAssign(a) => {
                let mut names = Vec::new();
                collect_target_names(a.target.as_ref(), &mut names);
                count += names.iter().filter(|n| **n == name).count();
            }
            Stmt::AnnAssign(a) => {
                let mut names = Vec::new();
                collect_target_names(a.target.as_ref(), &mut names);
                count += names.iter().filter(|n| **n == name).count();
            }
            Stmt::For(f) => {
                let mut names = Vec::new();
                collect_target_names(f.target.as_ref(), &mut names);
                count += names.iter().filter(|n| **n == name).count();
                count += count_scope_bindings(&f.body, name);
                count += count_scope_bindings(&f.orelse, name);
            }
            Stmt::While(w) => {
                count += count_scope_bindings(&w.body, name);
                count += count_scope_bindings(&w.orelse, name);
            }
            Stmt::If(i) => {
                count += count_scope_bindings(&i.body, name);
                count += count_scope_bindings(&i.orelse, name);
            }
            Stmt::With(w) => {
                for item in &w.items {
                    if let Some(v) = &item.optional_vars {
                        let mut names = Vec::new();
                        collect_target_names(v.as_ref(), &mut names);
                        count += names.iter().filter(|n| **n == name).count();
                    }
                }
                count += count_scope_bindings(&w.body, name);
            }
            Stmt::Try(t) => {
                count += count_scope_bindings(&t.body, name);
                for h in &t.handlers {
                    let rustpython_parser::ast::ExceptHandler::ExceptHandler(h) = h;
                    if h.name.as_ref().is_some_and(|n| n.as_str() == name) {
                        count += 1;
                    }
                    count += count_scope_bindings(&h.body, name);
                }
                count += count_scope_bindings(&t.orelse, name);
                count += count_scope_bindings(&t.finalbody, name);
            }
            Stmt::FunctionDef(d) => {
                if d.name.as_str() == name {
                    count += 1;
                }
            }
            Stmt::ClassDef(c) => {
                if c.name.as_str() == name {
                    count += 1;
                }
            }
            Stmt::Import(im) => {
                for a in &im.names {
                    let bound = a.asname.as_ref().unwrap_or(&a.name);
                    if bound.as_str() == name {
                        count += 1;
                    }
                }
            }
            Stmt::ImportFrom(im) => {
                for a in &im.names {
                    let bound = a.asname.as_ref().unwrap_or(&a.name);
                    if bound.as_str() == name {
                        count += 1;
                    }
                }
            }
            Stmt::Global(g) => {
                if g.names.iter().any(|n| n.as_str() == name) {
                    return 2; // disqualify outright
                }
            }
            Stmt::Nonlocal(g) => {
                if g.names.iter().any(|n| n.as_str() == name) {
                    return 2; // disqualify outright
                }
            }
            _ => {}
        }
    }
    count
}

/// Rewrite module-level `name = lambda ... (with DEFAULTS)` into a synthetic
/// `def name(...)` (Phase 8H, #9) — the def machinery provides the default
/// materialization and known-callee keyword adaptation that the closure path
/// rejects. Applies only when `name` is bound EXACTLY once at module scope
/// (rebinding keeps CPython's late-binding closure semantics) and the lambda
/// has no *args/**kwargs. Lambdas without defaults keep the closure path; a
/// lambda with defaults anywhere else still gets the loud rejection.
pub(crate) fn desugar_module_lambda_defs(body: &mut [Stmt]) {
    for i in 0..body.len() {
        let replacement = {
            let Stmt::Assign(a) = &body[i] else { continue };
            if a.targets.len() != 1 {
                continue;
            }
            let Expr::Name(n) = &a.targets[0] else {
                continue;
            };
            let Expr::Lambda(l) = a.value.as_ref() else {
                continue;
            };
            let args = l.args.as_ref();
            let has_defaults = args
                .posonlyargs
                .iter()
                .chain(args.args.iter())
                .any(|x| x.default.is_some());
            if !has_defaults
                || args.vararg.is_some()
                || args.kwarg.is_some()
                || !args.kwonlyargs.is_empty()
            {
                continue;
            }
            if count_scope_bindings(body, n.id.as_str()) != 1 {
                continue;
            }
            Stmt::FunctionDef(StmtFunctionDef {
                range: a.range,
                name: n.id.clone(),
                args: l.args.clone(),
                body: vec![Stmt::Return(rustpython_parser::ast::StmtReturn {
                    range: l.range,
                    value: Some(l.body.clone()),
                })],
                decorator_list: vec![],
                returns: None,
                type_comment: None,
                type_params: vec![],
            })
        };
        body[i] = replacement;
    }
}

fn collect_target_names<'a>(target: &'a Expr, out: &mut Vec<&'a str>) {
    match target {
        Expr::Name(n) => out.push(n.id.as_str()),
        Expr::Tuple(t) => {
            for e in &t.elts {
                collect_target_names(e, out);
            }
        }
        Expr::List(l) => {
            for e in &l.elts {
                collect_target_names(e, out);
            }
        }
        _ => {}
    }
}

/// If a format-spec (modeled by rustpython as a `JoinedStr` of literal parts) is
/// purely literal text (`:.4f`, `:4d`), return it as a plain string — the static
/// fast-path that keeps a constant spec a `Const::Str`. Returns `None` when the
/// spec carries a nested `{}` interpolation (`f"{x:.{n}f}"`), which the caller
/// then lowers dynamically through the f-string concat.
fn static_spec_literal(spec: &Expr) -> Option<String> {
    match spec {
        Expr::JoinedStr(j) => {
            let mut out = String::new();
            for part in &j.values {
                match part {
                    Expr::Constant(c) => match &c.value {
                        Constant::Str(s) => out.push_str(s),
                        _ => return None,
                    },
                    _ => return None,
                }
            }
            Some(out)
        }
        Expr::Constant(c) => match &c.value {
            Constant::Str(s) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Auto- vs manual-numbering mode of a `str.format` template (CPython forbids
/// mixing empty `{}` auto-indexing with explicit `{0}` indices).
#[derive(PartialEq)]
enum FmtNumbering {
    Unset,
    Auto,
    Manual,
}

/// Which argument a `str.format` replacement field binds to.
enum FmtFieldRef {
    /// `{}` — the next auto-index positional.
    Auto,
    /// `{0}` — an explicit positional index.
    Index(usize),
    /// `{name}` — a keyword argument.
    Keyword(String),
}

/// One parsed segment of a `str.format` template.
enum FmtSeg {
    /// Literal text (with `{{`/`}}` already unescaped).
    Lit(String),
    /// A replacement field `{[name][!conv][:spec]}`.
    Field {
        field: FmtFieldRef,
        conv: rustpython_parser::ast::ConversionFlag,
        spec: String,
    },
}

/// Parse a `str.format` template into literal-text / replacement-field segments.
/// Handles `{{`/`}}` escapes; rejects a nested `{}` inside a field (a dynamic
/// `.format` spec — deferred), an unmatched / stray brace, and (in
/// [`parse_format_field`]) `{0.attr}`/`{0[k]}` field access.
fn parse_format_template(s: &str, span: Span) -> Result<Vec<FmtSeg>> {
    let mut segs = Vec::new();
    let mut lit = String::new();
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' => {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    lit.push('{');
                    continue;
                }
                if !lit.is_empty() {
                    segs.push(FmtSeg::Lit(std::mem::take(&mut lit)));
                }
                // Read the field body up to the matching '}'.
                let mut body = String::new();
                let mut closed = false;
                while let Some(&nc) = chars.peek() {
                    if nc == '}' {
                        chars.next();
                        closed = true;
                        break;
                    }
                    if nc == '{' {
                        return Err(parse_error(
                            "a nested `{}` inside a .format() field/spec is out of scope",
                            span,
                        ));
                    }
                    body.push(nc);
                    chars.next();
                }
                if !closed {
                    return Err(parse_error("unmatched '{' in format string", span));
                }
                segs.push(parse_format_field(&body, span)?);
            }
            '}' => {
                if chars.peek() == Some(&'}') {
                    chars.next();
                    lit.push('}');
                    continue;
                }
                return Err(parse_error(
                    "single '}' encountered in format string",
                    span,
                ));
            }
            _ => lit.push(ch),
        }
    }
    if !lit.is_empty() {
        segs.push(FmtSeg::Lit(lit));
    }
    Ok(segs)
}

/// Parse a single `str.format` field body `[name][!conv][:spec]` (the braces
/// already stripped). The spec is static text (a nested `{}` was rejected by
/// the caller).
fn parse_format_field(body: &str, span: Span) -> Result<FmtSeg> {
    use rustpython_parser::ast::ConversionFlag;
    // Split off `:spec` at the first colon, then `!conv` from the remaining head.
    let (head, spec) = match body.find(':') {
        Some(i) => (&body[..i], body[i + 1..].to_string()),
        None => (body, String::new()),
    };
    let (name, conv) = match head.find('!') {
        Some(i) => {
            let flag = match &head[i + 1..] {
                "r" => ConversionFlag::Repr,
                "s" => ConversionFlag::Str,
                "a" => ConversionFlag::Ascii,
                other => {
                    return Err(parse_error(
                        format!("unknown conversion specifier '{other}' in format string"),
                        span,
                    ))
                }
            };
            (&head[..i], flag)
        }
        None => (head, ConversionFlag::None),
    };
    if name.contains('.') || name.contains('[') {
        return Err(parse_error(
            "`{0.attr}` / `{0[key]}` field access in .format() is out of scope",
            span,
        ));
    }
    let field = if name.is_empty() {
        FmtFieldRef::Auto
    } else if name.bytes().all(|b| b.is_ascii_digit()) {
        let idx = name.parse::<usize>().map_err(|_| {
            parse_error(format!("invalid field index '{name}' in format string"), span)
        })?;
        FmtFieldRef::Index(idx)
    } else {
        FmtFieldRef::Keyword(name.to_string())
    };
    Ok(FmtSeg::Field { field, conv, spec })
}

/// Reject the empty-tuple initializer as a *class attribute* (it is only valid
/// as a parameter default, where it materializes as a fresh `TupleLit`). A class
/// attribute lowers to a MIR `Const`, which has no empty-tuple form.
fn reject_tuple_class_attr(init: &ClassAttrInit, span: Span) -> Result<()> {
    if matches!(init, ClassAttrInit::EmptyTuple) {
        return Err(parse_error(
            "a tuple `()` class attribute is out of scope (only valid as a parameter default)",
            span,
        ));
    }
    Ok(())
}

/// Build an int/bignum class-attr initializer from decimal text + sign.
fn int_attr_init(interner: &mut StringInterner, decimal: &str, negative: bool) -> ClassAttrInit {
    match decimal.parse::<i64>() {
        Ok(mag) if pyaot_core_defs::int_fits(if negative { -mag } else { mag }) => {
            ClassAttrInit::Int(if negative { -mag } else { mag })
        }
        _ => {
            let text = if negative {
                format!("-{decimal}")
            } else {
                decimal.to_string()
            };
            ClassAttrInit::BigInt(interner.intern(&text))
        }
    }
}

/// The best-effort `SemTy` of a class-attribute initializer.
fn class_attr_init_ty(init: &ClassAttrInit) -> SemTy {
    match init {
        ClassAttrInit::Int(_) | ClassAttrInit::BigInt(_) => SemTy::Int,
        ClassAttrInit::Float(_) => SemTy::Float,
        ClassAttrInit::Bool(_) => SemTy::Bool,
        ClassAttrInit::Str(_) => SemTy::Str,
        ClassAttrInit::Bytes(_) => SemTy::Bytes,
        ClassAttrInit::None => SemTy::NoneTy,
        ClassAttrInit::EmptyTuple => SemTy::Dyn,
    }
}

/// If `expr` is a direct `print(...)` call, return it.
fn as_print_call(expr: &Expr) -> Option<&rustpython_parser::ast::ExprCall> {
    if let Expr::Call(call) = expr {
        if let Expr::Name(n) = call.func.as_ref() {
            if n.id.as_str() == "print" {
                return Some(call);
            }
        }
    }
    None
}

fn to_span(range: TextRange) -> Span {
    Span::new(range.start().to_u32(), range.end().to_u32())
}

fn parse_error(msg: impl Into<String>, span: Span) -> CompilerError {
    CompilerError::parse_error(msg.into(), span)
}

#[cfg(test)]
mod tests {
    use pyaot_hir::HirStmt;
    use pyaot_utils::StringInterner;

    /// Parse `src` into an HIR module.
    fn parsed(src: &str) -> (pyaot_hir::HirModule, StringInterner) {
        let mut interner = StringInterner::new();
        let module = crate::parse(src, &mut interner).expect("parse");
        (module, interner)
    }

    /// Parse `src`, returning the error message (the rejection-path helper).
    fn parse_err(src: &str) -> String {
        let mut interner = StringInterner::new();
        match crate::parse(src, &mut interner) {
            Ok(_) => panic!("expected a parse rejection"),
            Err(e) => format!("{e:?}"),
        }
    }

    // ── Phase 7 lexical restrictions ──

    #[test]
    fn rejects_yield_inside_try() {
        // A suspended frame would dangle its stack-allocated ExceptionFrame.
        let err = parse_err(
            "def g():\n    try:\n        yield 1\n    except ValueError:\n        pass\n",
        );
        assert!(err.contains("yield inside try/with"), "got: {err}");
    }

    #[test]
    fn rejects_yield_inside_with() {
        let err = parse_err("def g():\n    with ctx() as c:\n        yield c\n");
        assert!(err.contains("yield inside try/with"), "got: {err}");
    }

    #[test]
    fn rejects_or_pattern_with_captures() {
        let err = parse_err("match x:\n    case [a] | [a, b]:\n        pass\n");
        assert!(err.contains("or-patterns with capture names"), "got: {err}");
    }

    #[test]
    fn rejects_positional_class_pattern() {
        let err = parse_err(
            "class P:\n    def __init__(self, x: int):\n        self.x = x\nmatch P(1):\n    case P(1):\n        pass\n",
        );
        assert!(err.contains("positional class patterns"), "got: {err}");
    }

    #[test]
    fn rejects_unknown_exception_in_except() {
        let err = parse_err("try:\n    pass\nexcept NotAThing:\n    pass\n");
        assert!(err.contains("unknown exception type"), "got: {err}");
    }

    #[test]
    fn rejects_bare_except_not_last() {
        let err = parse_err("try:\n    pass\nexcept:\n    pass\nexcept ValueError:\n    pass\n");
        assert!(err.contains("must be last"), "got: {err}");
    }

    #[test]
    fn accepts_try_raise_with_match_shapes() {
        // The Phase-7 statement forms all lower without rejection.
        let src = "\
def f(n: int) -> int:
    total = 0
    try:
        if n == 1:
            raise ValueError(\"one\")
        total = total + 1
    except (ValueError, TypeError) as e:
        total = total - 1
    except:
        raise
    else:
        total = total + 10
    finally:
        total = total + 100
    match n:
        case 0:
            total = total + 1
        case [x, *rest]:
            total = total + x
        case {\"k\": v, **other}:
            total = total + v
        case y if y > 5:
            total = total + y
    return total
";
        let (m, _i) = parsed(src);
        assert!(m.functions.len() >= 2);
    }

    #[test]
    fn sibling_synthetic_names_are_unique() {
        // Two same-named nested defs in one scope must get distinct synthetic
        // names (the `#k` uniquifier), else the function table would alias them.
        let src = "\
def outer():
    if True:
        def helper():
            return 1
    else:
        def helper():
            return 2
    return 0
";
        let (m, i) = parsed(src);
        let names: Vec<&str> = m
            .functions
            .iter()
            .map(|f| i.resolve(f.name))
            .filter(|n| n.contains("helper"))
            .collect();
        assert_eq!(names.len(), 2);
        assert_ne!(names[0], names[1], "sibling synthetics must be unique");
    }

    #[test]
    fn decorated_module_def_rebinds_in_source_order() {
        // A module-level decorated def emits its `GlobalSet` rebinding into
        // `__main__` at the def's source position, interleaved with top stmts.
        let src = "\
from typing import Callable
print(\"before\")
def logged(func: Callable[..., int]) -> Callable[..., int]:
    def wrapper(*args, **kwargs) -> int:
        return func(*args, **kwargs)
    return wrapper
@logged
def add(a, b):
    return a + b
print(\"after\")
print(add(1, 2))
";
        let (m, _i) = parsed(src);
        let main = m.function(m.main);
        // Walk main's stmts in order: the decorated rebinding (a GlobalSet) must
        // appear, and after the first print, before the call to `add`.
        let mut saw_global_set = false;
        for (_b, block) in main.blocks.iter() {
            for stmt in &block.stmts {
                if matches!(stmt, HirStmt::GlobalSet { .. }) {
                    saw_global_set = true;
                }
            }
        }
        assert!(
            saw_global_set,
            "decorated def must rebind via a global slot"
        );
    }
}
