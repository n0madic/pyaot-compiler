use super::*;

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
        let method_uniform_thunks = self.shared.method_uniform_thunks.clone();
        let iternext_thunks = self.shared.iternext_thunks.clone();
        let copy_thunks = self.shared.copy_thunks.clone();
        let functions = self.shared.finish();
        let module = HirModule {
            functions,
            classes: self.classes,
            main: main_id,
            generators,
            global_annotations: self.global_annotations,
            deletable_globals,
            deletable_fields,
            method_uniform_thunks,
            iternext_thunks,
            copy_thunks,
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
    fn handle_import(&mut self, i: &StmtImport, my_ns: u32, col: &mut ImportCollect) -> Result<()> {
        let span = to_span(i.range());
        // Key on the statement's source start so a conditionally-nested import is
        // locatable from `lower_stmt`. Recorded for EVERY scanned import (incl.
        // typing-only / stdlib), so `lower_stmt` distinguishes a module-level
        // conditional import from one in a function body / loop / `with`.
        let key = i.range().start().to_u32();
        col.scanned_imports.insert(key);
        for alias in &i.names {
            let dotted_name = alias.name.as_str();
            // `dataclasses` is recognized syntactically (the `@dataclass`
            // decorator desugar runs in the frontend); no runtime binding needed,
            // so the import is a no-op like `typing` (no `field()` surface — that
            // is out of scope).
            if matches!(
                dotted_name,
                "typing" | "__future__" | "typing_extensions" | "dataclasses"
            ) {
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
            let entry = col.actions.entry(key).or_default();
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
        col: &mut ImportCollect,
    ) -> Result<()> {
        let span = to_span(i.range());
        // Statement start offset — see `handle_import`.
        let key = i.range().start().to_u32();
        col.scanned_imports.insert(key);
        let level = i.level.map(|l| l.to_u32() as usize).unwrap_or(0);
        let module_name = i.module.as_ref().map(|m| m.as_str());
        if level == 0 {
            if let Some(m) = module_name {
                // `from dataclasses import dataclass` is a no-op: `@dataclass` is
                // desugared syntactically in the frontend (see [`dataclass`]).
                if matches!(
                    m,
                    "typing" | "__future__" | "typing_extensions" | "dataclasses"
                ) {
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
                // `collections.namedtuple` is desugared syntactically (see
                // `synth_class`); it has no runtime binding, so skip it (the rest
                // of `collections` — Counter, deque, … — binds normally).
                if dotted == "collections" && name == "namedtuple" {
                    continue;
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
        let entry = col.actions.entry(key).or_default();
        entry.init_calls.extend(action.init_calls);
        entry.snapshots.extend(action.snapshots);
        Ok(())
    }

    /// Recursively scan a statement list for imports, descending only into
    /// module-level `if`/`try` branches (the conditional-import forms). Bodies of
    /// functions, classes, `while`/`for`/`with` are NOT entered: their imports are
    /// rejected later in `lower_stmt`. Each import loads + binds its module
    /// unconditionally (load has no runtime effect — like the `typing` carve-out);
    /// only the `<init>` call / snapshots are positional, replayed where the
    /// import textually sits.
    fn scan_imports(
        &mut self,
        stmts: &[Stmt],
        mod_path: &[String],
        is_package: bool,
        my_ns: u32,
        col: &mut ImportCollect,
    ) -> Result<()> {
        for stmt in stmts {
            match stmt {
                Stmt::Import(im) => self.handle_import(im, my_ns, col)?,
                Stmt::ImportFrom(im) => {
                    self.handle_import_from(im, mod_path, is_package, my_ns, col)?
                }
                Stmt::If(s) => {
                    self.scan_imports(&s.body, mod_path, is_package, my_ns, col)?;
                    // `orelse` covers both `elif` (a nested `If`) and `else`.
                    self.scan_imports(&s.orelse, mod_path, is_package, my_ns, col)?;
                }
                Stmt::Try(t) => {
                    self.scan_imports(&t.body, mod_path, is_package, my_ns, col)?;
                    for h in try_handlers(&t.handlers) {
                        self.scan_imports(&h.body, mod_path, is_package, my_ns, col)?;
                    }
                    self.scan_imports(&t.orelse, mod_path, is_package, my_ns, col)?;
                    self.scan_imports(&t.finalbody, mod_path, is_package, my_ns, col)?;
                }
                Stmt::TryStar(t) => {
                    self.scan_imports(&t.body, mod_path, is_package, my_ns, col)?;
                    for h in try_handlers(&t.handlers) {
                        self.scan_imports(&h.body, mod_path, is_package, my_ns, col)?;
                    }
                    self.scan_imports(&t.orelse, mod_path, is_package, my_ns, col)?;
                    self.scan_imports(&t.finalbody, mod_path, is_package, my_ns, col)?;
                }
                // Other statement kinds (function/class bodies, `while`/`for`/
                // `with`) are not descended — their imports stay out of scope.
                _ => {}
            }
        }
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
        // `@dataclass` classes and `namedtuple` assignments → synthesized classes
        // with AST dunders, BEFORE partitioning so the synthetic methods are
        // present everywhere the class is later scanned. Recurses into nested
        // bodies to cover nested forms.
        desugar_synthesized_classes(&mut body)?;
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
        // Class ids of `Protocol` classes: a protocol-typed annotation
        // erases to `Dyn`, and `isinstance(obj, P)` is a structural check.
        let mut proto_ids: HashSet<ClassId> = HashSet::new();
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
            if class_def_is_protocol(cdef) {
                proto_ids.insert(class_id);
            }
        }

        // ── Static/class method shapes (Fix A): record each `@staticmethod` /
        // `@classmethod` of a top-level class by its synthetic name `"Cls.method"`,
        // so a later-lowered `Cls.smeth(*args)` spread call can build a
        // receiver-less uniform thunk on demand (classes are lowered LAST, after
        // the call sites — Phase B below — so the shape must be known up front).
        // `is_classmethod` selects the `cls`-dropping parse, mirroring how the
        // method itself is lowered. ──
        for cdef in &classdefs {
            for stmt in &cdef.body {
                if let Stmt::FunctionDef(m) = stmt {
                    let is_classmethod = match classify_method_decorator(m) {
                        Ok(MethodDecor::Static) => false,
                        Ok(MethodDecor::Class) => true,
                        _ => continue,
                    };
                    let qual = format!("{}.{}", cdef.name.as_str(), m.name.as_str());
                    self.shared
                        .static_method_shapes
                        .insert(qual, (is_classmethod, (*m.args).clone()));
                }
            }
        }

        // ── Nested classes (FIX 2). A `class` below module top level (inside a
        // function, another class, or module-level control flow) is registered
        // here, alongside the top-level classes and BEFORE `class_map` is moved
        // into the import collector, so every downstream name-resolution site
        // (`AnnCtx::class_map`) sees it unchanged. The flat `class_map` keys on
        // the bare name, so a nested class name must be program-unique. ──
        let mut nested_classes: Vec<NestedClass> = Vec::new();
        collect_nested_classdefs(&body, &mut nested_classes);
        let mut nested_class_ids: Vec<ClassId> = Vec::with_capacity(nested_classes.len());
        for nc in &nested_classes {
            let cdef = nc.def;
            if self.next_class_id > u8::MAX as u32 {
                return Err(parse_error(
                    "too many user-defined classes across all modules (the runtime \
                     class_id is a u8, so at most 189 in [67, 255])",
                    to_span(cdef.range()),
                ));
            }
            if class_map.contains_key(cdef.name.as_str()) {
                return Err(parse_error(
                    format!(
                        "nested class `{}` collides with another class of the same name \
                         in this module; nested class names must be unique program-wide \
                         (rename it)",
                        cdef.name.as_str()
                    ),
                    to_span(cdef.range()),
                ));
            }
            let class_id = ClassId::new(self.next_class_id);
            self.next_class_id += 1;
            let iname = self.interner.intern(cdef.name.as_str());
            class_map.insert(cdef.name.as_str().to_string(), (class_id, iname));
            nested_class_ids.push(class_id);
            own_classes.push((cdef.name.as_str().to_string(), class_id, iname));
            self.class_ns.insert(class_id, my_ns);
            if class_def_is_protocol(cdef) {
                proto_ids.insert(class_id);
            }
        }
        // Capture check (A2) — runs once after every nested class is registered,
        // so `class_map` holds every class name. A method (or class-body) free
        // name that is an enclosing-function local is rejected (lifting the class
        // to module scope would silently rebind it to a global). A reference to a
        // *class* (a sibling/ancestor nested class or a top-level class) resolves
        // statically through `class_map`, so it is excluded — it is not a capture.
        for nc in &nested_classes {
            if nc.enclosing_locals.is_empty() {
                continue;
            }
            let free = freevars::class_method_free(nc.def);
            let mut caps: Vec<&str> = free
                .iter()
                .filter(|n| nc.enclosing_locals.contains(*n) && !class_map.contains_key(*n))
                .map(|s| s.as_str())
                .collect();
            if !caps.is_empty() {
                caps.sort_unstable();
                return Err(parse_error(
                    format!(
                        "a nested class whose method captures an enclosing-function \
                         local is out of scope (captures: {}); reference module \
                         globals or pass values explicitly",
                        caps.join(", ")
                    ),
                    to_span(nc.def.range()),
                ));
            }
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
            scanned_imports: HashSet::new(),
        };
        // Recursive scan: top-level imports plus those nested in module-level
        // `if`/`try` branches (conditional / optional-dependency imports). Each is
        // loaded + bound unconditionally; its runtime `<init>`/snapshot effect is
        // keyed by source offset and replayed where the import textually sits.
        self.scan_imports(&body, mod_path, is_package, my_ns, &mut col)?;
        // Restore the namespace for this module's own function reservations.
        self.shared.current_ns = my_ns;

        // ── Module type aliases. `type X = T` (PEP 695) and `X:
        // TypeAlias = T` (PEP 613) register `name → body SemTy`; the body resolves
        // through a bootstrap annotation context (class_map + module type vars are
        // known; alias-to-alias references and the `[V]` params of PEP 695 aliases
        // are out of scope — the corpus bodies use only builtin/Union types). The
        // alias names then resolve to their bodies in `named_annotation`. ──
        let mut type_aliases: HashMap<String, SemTy> = HashMap::new();
        {
            let empty_defs0: TopDefMap = HashMap::new();
            let empty_aliases0: HashMap<String, SemTy> = HashMap::new();
            let alias_ctx = AnnCtx {
                class_map: &col.class_map,
                type_vars: &module_type_vars,
                top_defs: &empty_defs0,
                promoted: &col.promoted,
                decorated: &decorated_names,
                aliases: &col.aliases,
                alias_vars: &col.alias_vars,
                stdlib: &col.stdlib,
                default_slots: None,
                type_aliases: &empty_aliases0,
                proto_ids: &proto_ids,
            };
            for &stmt in &top {
                match stmt {
                    Stmt::TypeAlias(ta) => {
                        if let Expr::Name(n) = ta.name.as_ref() {
                            let sty = annotation_to_semty(ta.value.as_ref(), &alias_ctx);
                            type_aliases.insert(n.id.as_str().to_string(), sty);
                        }
                    }
                    // PEP 613: `X: TypeAlias = T`.
                    Stmt::AnnAssign(a) => {
                        if let (Expr::Name(tgt), Expr::Name(ann)) =
                            (a.target.as_ref(), a.annotation.as_ref())
                        {
                            if ann.id.as_str() == "TypeAlias" {
                                if let Some(val) = &a.value {
                                    let sty = annotation_to_semty(val.as_ref(), &alias_ctx);
                                    type_aliases.insert(tgt.id.as_str().to_string(), sty);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // ── Mutable/computed parameter-default slots (allocated ONCE, before the
        // pre-context). For each top-level NON-generator `def`, a parameter whose
        // default is a non-literal expression (`[]`, `5 + 5`, …) takes a synthetic
        // promoted-global slot: evaluated once at the def's module-init position
        // (CPython def-time once-evaluation) and read — shared — at every
        // defaulted call. Keyed by (interned def name, interned param name). The
        // map carries ONLY top-level non-generator defs, so methods / nested /
        // decorated / generators (which never key into it, and whose contexts
        // carry `default_slots = None`) keep rejecting non-literal defaults. ──
        let mut default_slots: DefaultSlotMap = HashMap::new();
        for def in &defs {
            if body_has_yield(&def.body) {
                continue; // generators are out of scope for slot defaults
            }
            let fname = self.interner.intern(def.name.as_str());
            let dargs = def.args.as_ref();
            for awd in defaultable_params(dargs) {
                let Some(e) = &awd.default else { continue };
                if try_literal_default(&mut *self.interner, e).is_none() {
                    let pname = self.interner.intern(awd.def.arg.as_str());
                    default_slots.entry((fname, pname)).or_insert_with(|| {
                        let id = self.next_global;
                        self.next_global += 1;
                        id
                    });
                }
            }
        }
        // Method params with non-literal defaults take a slot too (§6 — CPython's
        // shared-mutable-default gotcha applies to `__init__(self, x=[])`), keyed
        // by the method's SYNTHETIC lowered name so `resolve_param_default`
        // resolves them through the class context's `default_slots`.
        for cdef in &classdefs {
            for stmt in &cdef.body {
                let Stmt::FunctionDef(m) = stmt else { continue };
                let synthetic = format!(
                    "{}.{}{}",
                    cdef.name.as_str(),
                    m.name.as_str(),
                    method_synthetic_suffix(m)
                );
                let fname = self.interner.intern(&synthetic);
                let margs = m.args.as_ref();
                for awd in defaultable_params(margs) {
                    let Some(e) = &awd.default else { continue };
                    if try_literal_default(&mut *self.interner, e).is_none() {
                        let pname = self.interner.intern(awd.def.arg.as_str());
                        default_slots.entry((fname, pname)).or_insert_with(|| {
                            let id = self.next_global;
                            self.next_global += 1;
                            id
                        });
                    }
                }
            }
        }

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
            // Top-level defs resolve their non-literal defaults to slots.
            default_slots: Some(&default_slots),
            type_aliases: &type_aliases,
            proto_ids: &proto_ids,
        };
        let mut top_defs: TopDefMap = HashMap::new();
        for def in &defs {
            let fname = self.interner.intern(def.name.as_str());
            let parsed = parse_params(
                &mut *self.interner,
                &pre_ctx,
                def.args.as_ref(),
                &FirstParam::Plain,
                fname,
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
        // The general module context carries NO `default_slots`: decorated defs,
        // the module-init body, and classes/methods all reject non-literal
        // defaults. Only the top-level-def lowering (`defs_ctx`) sees the slots.
        let module_ctx = AnnCtx {
            class_map: &col.class_map,
            type_vars: &module_type_vars,
            top_defs: &top_defs,
            promoted: &col.promoted,
            decorated: &decorated_names,
            aliases: &col.aliases,
            alias_vars: &col.alias_vars,
            stdlib: &col.stdlib,
            default_slots: None,
            type_aliases: &type_aliases,
            proto_ids: &proto_ids,
        };
        let defs_ctx = AnnCtx {
            default_slots: Some(&default_slots),
            ..module_ctx
        };

        // ── Phase B: lower own functions, the module-init body, and classes. ──
        let mut own_func_fids: HashMap<String, FuncId> = HashMap::new();
        for def in &defs {
            let name = self.interner.intern(def.name.as_str());
            let fid = lower_callable(
                &mut *self.interner,
                &defs_ctx,
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
            let _orig_fid = lower_callable(
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
            // A decorated def is lowered under `module_ctx` (no `default_slots`),
            // so any non-literal default is rejected — the `fname` is immaterial.
            let dname = self.interner.intern(d.name.as_str());
            let parsed = parse_params(
                &mut *self.interner,
                &module_ctx,
                d.args.as_ref(),
                &FirstParam::Plain,
                dname,
            )?;
            if parsed.varargs.is_some() || parsed.kwargs.is_some() || !parsed.kwonly.is_empty() {
                return Err(parse_error(
                    "a decorated function with *args/**kwargs/keyword-only params is out \
                     of scope (Phase 6D)",
                    to_span(d.range()),
                ));
            }
            let ret = match &d.returns {
                Some(e) => annotation_to_semty(e.as_ref(), &module_ctx),
                None => SemTy::Dyn,
            };
            // The decorated wrapper's slot holds the uniform thunk over the renamed
            // `<orig>` body (`orig_name` resolves to it). All params are fixed
            // (varargs/kwargs/kw-only rejected above), `pass_env=false` (top-level).
            let target = UniformTarget {
                name: orig_name,
                ret,
                pass_env: false,
                fixed: parsed
                    .fixed
                    .iter()
                    .map(ThunkParam::from_param_info)
                    .collect(),
                kwonly: vec![],
                varargs: false,
                kwargs: false,
                kw_bindable: false,
            };
            let thunk_fid =
                build_uniform_thunk(&mut *self.interner, &module_ctx, &mut self.shared, &target)?;
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
            // Hand the import actions + scanned offsets to the module-init lowerer
            // so a module-level CONDITIONAL import (inside an `if`/`try`) replays
            // its `<init>`/snapshot effect in position from `lower_stmt`.
            main.import_actions = col.actions.clone();
            main.scanned_import_offsets = col.scanned_imports.clone();
            main.set_scope_facts(&main_facts);
            main.init_cells();
            let dunder_name = main.intern("__name__");
            let name_lit = main.intern(&name_value);
            let name_val = main.alloc(HirExprKind::StrLit(name_lit), SemTy::Str, Span::dummy());
            main.write_named(dunder_name, SemTy::Str, name_val);
            for stmt in &body {
                match stmt {
                    // Top-level imports replay their precomputed init-calls +
                    // snapshots here, keyed by source offset (typing-only imports
                    // have no action). Imports nested in module-level `if`/`try`
                    // replay from `lower_stmt` (the `other` arm below) instead, so
                    // their effect runs only when the branch is taken.
                    Stmt::Import(_) | Stmt::ImportFrom(_) => {
                        let key = stmt.range().start().to_u32();
                        if let Some(action) = col.actions.get(&key) {
                            main.emit_import_action(action);
                        }
                    }
                    Stmt::FunctionDef(f) if !f.decorator_list.is_empty() => {
                        let info = &decorated_info[f.name.as_str()];
                        main.emit_decorated_rebinding(f, info.thunk_fid, info.slot)?;
                    }
                    // A plain top-level def evaluates its non-literal (slot)
                    // defaults ONCE here, at the def's textual position (CPython
                    // def-time once-evaluation), storing each shared object into
                    // its synthetic GC-rooted global slot.
                    Stmt::FunctionDef(f) => {
                        main.emit_default_slots(f, &default_slots)?;
                    }
                    // A class evaluates its methods' non-literal (slot) defaults
                    // ONCE here, at the class's textual position (§6 — CPython's
                    // shared-mutable-default for `__init__(self, x=[])`), then
                    // applies any class decorators (§5) over the class-id int.
                    Stmt::ClassDef(c) => {
                        main.emit_class_default_slots(c, &default_slots)?;
                        main.emit_class_decorators(c)?;
                    }
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

        // Gradual-completeness method dispatch (Phase B): collect every
        // method-call name `x.NAME(...)` in this module body so `lower_class`
        // builds a uniform thunk for each instance method actually invoked as a
        // method (the over-approximate `Dyn`-receiver gate). Accumulated into
        // the program-global set before this module's classes are lowered.
        {
            let mut names: HashSet<String> = HashSet::new();
            collect_method_call_names(&body, &mut names);
            for n in names {
                let id = self.interner.intern(&n);
                self.shared.dyn_method_names.insert(id);
            }
        }

        // Classes (own). `defs_ctx` carries the mutable-default slot map so a
        // method's non-literal default (`__init__(self, x=[])`, §6) resolves to
        // its synthetic global slot instead of being rejected.
        for (i, cdef) in classdefs.iter().enumerate() {
            let hclass = lower_class(
                &mut *self.interner,
                &defs_ctx,
                cdef,
                class_ids[i],
                &mut self.shared,
            )?;
            self.classes.push(hclass);
        }

        // Nested classes (FIX 2): registered above; lowered through the same
        // `lower_class` path as the top-level classes (so fields ride the same
        // field-type constraint solving, B10). The `lower_stmt` `ClassDef` arm
        // for each only validates unsupported decoration/defaults — it emits no
        // code, since use sites resolve through `class_map`, not local scope.
        for (i, nc) in nested_classes.iter().enumerate() {
            let hclass = lower_class(
                &mut *self.interner,
                &defs_ctx,
                nc.def,
                nested_class_ids[i],
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
pub(super) static OPEN_DEF: pyaot_stdlib_defs::StdlibFunctionDef = pyaot_stdlib_defs::StdlibFunctionDef {
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
pub(super) fn register_stdlib_alias(
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
pub(super) fn register_stdlib_items(
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
pub(super) fn bind_stdlib_item(
    bind: &str,
    name: &str,
    module_dotted: &str,
    module: &'static pyaot_stdlib_defs::StdlibModuleDef,
    stdlib: &mut StdlibBindings,
    span: Span,
) -> Result<()> {
    // A name can be BOTH a constructor function and a class with the same name
    // — e.g. `io.StringIO`: `StringIO(...)` constructs (the function, in
    // `funcs`), while `x: StringIO` annotates (the class type_spec, in
    // `classes`). The two live in separate binding maps, so bind each
    // independently rather than first-match-wins: otherwise the constructor
    // shadows the class and `x: StringIO` / `-> StringIO` silently degrades to
    // `Dyn` (treated as unannotated). For a class-only name (`re.Match` — no
    // constructor) the class binding is the sole one. The whole-module
    // (`import io`) path in `register_stdlib_items` already binds both.
    let mut bound = false;
    if let Some(f) = module.functions.iter().find(|f| f.name == name) {
        stdlib.funcs.insert(bind.to_string(), f);
        bound = true;
    }
    if let Some(spec) = module
        .classes
        .iter()
        .find(|c| c.name == name)
        .and_then(|c| c.type_spec.as_ref())
    {
        stdlib
            .classes
            .insert(bind.to_string(), pyaot_hir::semty_from_typespec(spec));
        bound = true;
    }
    if let Some(c) = module.constants.iter().find(|c| c.name == name) {
        stdlib.consts.insert(bind.to_string(), c);
        bound = true;
    }
    if let Some(a) = module.attrs.iter().find(|a| a.name == name) {
        stdlib.attrs.insert(bind.to_string(), a);
        bound = true;
    }
    if let Some(exc) = module.exceptions.iter().find(|e| e.name == name) {
        // A stdlib exception class (`from urllib.error import HTTPError`,
        // Phase 8D): record its reserved id + builtin parent tag for
        // `except`/`raise` resolution.
        stdlib
            .exceptions
            .insert(bind.to_string(), (exc.class_id, exc.parent.tag()));
        bound = true;
    }
    if !bound {
        return Err(parse_error(
            format!("cannot import name `{name}` from `{module_dotted}`"),
            span,
        ));
    }
    Ok(())
}

/// Resolve a relative `from`-import (`level` leading dots + optional `module`)
/// against the importing module's package path — CPython `__package__` rules.
pub(super) fn resolve_relative(
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

/// Side-effect-free literal (possibly sign-prefixed)? Such arguments skip
/// staging and stay [`ArgSrc::Plain`], so downstream slot-fill folds (`None`
/// → absent stdlib slot, int literal → float for raw-ABI params) still see
/// the AST shape.
pub(super) fn is_const_like(e: &Expr) -> bool {
    match e {
        Expr::Constant(_) => true,
        Expr::UnaryOp(u) => {
            matches!(u.op, PyUnaryOp::USub | PyUnaryOp::UAdd)
                && matches!(u.operand.as_ref(), Expr::Constant(_))
        }
        _ => false,
    }
}

