//! Free-variable analysis (Phase 6A/6B).
//!
//! A single bottom-up walk per scope computing
//! `free(f) = (reads(f) ∪ ⋃ free(children)) − bound(f)` — transitive bubbling of
//! a grandchild's reads falls out of the recursion. This is an A3-safe forward
//! pre-pass (the `collect_classes` precedent): it feeds the lowering of each
//! scope and never reopens an earlier result.
//!
//! The analysis is purely syntactic: a "free" name may later resolve to a
//! builtin / top-level function / promoted global rather than an enclosing
//! local — the *capture* decision happens at the def site, where the enclosing
//! scope filters its own visible bindings.

use std::collections::HashSet;

use rustpython_parser::ast::{
    Comprehension, Expr, ExprGeneratorExp, ExprLambda, Stmt, StmtFunctionDef,
};

/// The scoping facts of one function-like scope (a `def`, a lambda, a genexpr,
/// or the module body).
#[derive(Debug, Default)]
pub(crate) struct ScopeFacts {
    /// Names this scope binds (params, assignment / loop / comprehension
    /// targets, nested `def` names) — minus `global`/`nonlocal` declarations.
    pub bound: HashSet<String>,
    /// Free names in first-occurrence order: reads (plus `nonlocal` targets)
    /// not bound here, including those bubbled up from nested scopes.
    pub free: Vec<String>,
    /// Own bound names that appear in some descendant's free set — these must
    /// live in cells (the P6-2 rule).
    pub celled: HashSet<String>,
    /// `global`-declared names in this scope.
    pub globals: HashSet<String>,
    /// `nonlocal`-declared names in this scope (always also in `free`).
    pub nonlocals: HashSet<String>,
    /// Names some *descendant* declares `nonlocal` (or this scope itself does)
    /// — i.e. cells written through from another function. The owner demotes
    /// the cell's inferred type to `Dyn` for these (a cross-function write is
    /// invisible to per-function inference, so a precise join would be unsound).
    pub shared_writes: HashSet<String>,
    /// Every `global`-declared name in this scope OR any descendant (bubbled) —
    /// input to the module's promoted-global scan (Phase 6B).
    pub global_decls: HashSet<String>,
}

#[derive(Default)]
struct Walker {
    raw_bound: HashSet<String>,
    reads: Vec<String>,
    reads_seen: HashSet<String>,
    child_free: Vec<String>,
    child_free_seen: HashSet<String>,
    globals: HashSet<String>,
    nonlocals: HashSet<String>,
    shared_writes: HashSet<String>,
    child_global_decls: HashSet<String>,
}

impl Walker {
    fn read(&mut self, name: &str) {
        if self.reads_seen.insert(name.to_string()) {
            self.reads.push(name.to_string());
        }
    }

    fn bind(&mut self, name: &str) {
        self.raw_bound.insert(name.to_string());
    }

    /// Merge a nested scope's facts: its free names bubble into ours; its
    /// `nonlocal` targets (and anything bubbled through it) mark shared-write
    /// cells.
    fn child(&mut self, facts: &ScopeFacts) {
        for n in &facts.free {
            if self.child_free_seen.insert(n.clone()) {
                self.child_free.push(n.clone());
            }
        }
        for n in facts.nonlocals.iter().chain(&facts.shared_writes) {
            self.shared_writes.insert(n.clone());
        }
        for n in &facts.global_decls {
            self.child_global_decls.insert(n.clone());
        }
    }

    fn finish(self) -> ScopeFacts {
        let mut bound = self.raw_bound;
        for n in self.globals.iter().chain(&self.nonlocals) {
            bound.remove(n);
        }
        let mut free = Vec::new();
        let mut seen = HashSet::new();
        for n in self.reads.iter().chain(&self.child_free) {
            if !bound.contains(n) && !self.globals.contains(n) && seen.insert(n.clone()) {
                free.push(n.clone());
            }
        }
        let celled: HashSet<String> = self
            .child_free
            .iter()
            .filter(|n| bound.contains(*n))
            .cloned()
            .collect();
        let mut global_decls = self.child_global_decls;
        global_decls.extend(self.globals.iter().cloned());
        ScopeFacts {
            bound,
            free,
            celled,
            globals: self.globals,
            nonlocals: self.nonlocals,
            shared_writes: self.shared_writes,
            global_decls,
        }
    }

    // ── statements ──

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Expr(e) => self.expr(&e.value),
            Stmt::Assign(a) => {
                self.expr(&a.value);
                for t in &a.targets {
                    self.target(t);
                }
            }
            Stmt::AugAssign(a) => {
                self.expr(&a.value);
                // `x += v` both reads and binds `x`.
                if let Expr::Name(n) = a.target.as_ref() {
                    self.read(n.id.as_str());
                }
                self.target(&a.target);
            }
            Stmt::AnnAssign(a) => {
                // The annotation itself is type-level (class names resolve via
                // the class map, never via capture) — skip it.
                if let Some(v) = &a.value {
                    self.expr(v);
                }
                self.target(&a.target);
            }
            Stmt::If(s) => {
                self.expr(&s.test);
                self.stmts(&s.body);
                self.stmts(&s.orelse);
            }
            Stmt::While(s) => {
                self.expr(&s.test);
                self.stmts(&s.body);
                self.stmts(&s.orelse);
            }
            Stmt::For(s) => {
                self.expr(&s.iter);
                self.target(&s.target);
                self.stmts(&s.body);
                self.stmts(&s.orelse);
            }
            Stmt::Assert(s) => {
                self.expr(&s.test);
                if let Some(m) = &s.msg {
                    self.expr(m);
                }
            }
            Stmt::Return(r) => {
                if let Some(v) = &r.value {
                    self.expr(v);
                }
            }
            Stmt::FunctionDef(d) => {
                // Decorators and parameter defaults evaluate in THIS scope.
                for deco in &d.decorator_list {
                    self.expr(deco);
                }
                for awd in d.args.posonlyargs.iter().chain(&d.args.args).chain(&d.args.kwonlyargs)
                {
                    if let Some(dflt) = &awd.default {
                        self.expr(dflt);
                    }
                }
                self.bind(d.name.as_str());
                let facts = analyze_def(d);
                self.child(&facts);
            }
            Stmt::ClassDef(c) => {
                for deco in &c.decorator_list {
                    self.expr(deco);
                }
                for b in &c.bases {
                    self.expr(b);
                }
                self.bind(c.name.as_str());
                // Class bodies inside functions are rejected by the lowering;
                // no need to walk them here.
            }
            Stmt::Global(g) => {
                for n in &g.names {
                    self.globals.insert(n.as_str().to_string());
                }
            }
            Stmt::Nonlocal(g) => {
                for n in &g.names {
                    self.nonlocals.insert(n.as_str().to_string());
                    // A nonlocal target must be captured from the enclosing
                    // scope even if it is only ever written here.
                    self.read(n.as_str());
                }
            }
            // ── exceptions / with / match (Phase 7) ──
            Stmt::Try(t) => {
                self.stmts(&t.body);
                for h in &t.handlers {
                    let rustpython_parser::ast::ExceptHandler::ExceptHandler(h) = h;
                    if let Some(ty) = &h.type_ {
                        self.expr(ty);
                    }
                    if let Some(name) = &h.name {
                        self.bind(name.as_str());
                    }
                    self.stmts(&h.body);
                }
                self.stmts(&t.orelse);
                self.stmts(&t.finalbody);
            }
            Stmt::Raise(r) => {
                if let Some(e) = &r.exc {
                    self.expr(e);
                }
                if let Some(c) = &r.cause {
                    self.expr(c);
                }
            }
            Stmt::With(w) => {
                for item in &w.items {
                    self.expr(&item.context_expr);
                    if let Some(t) = &item.optional_vars {
                        self.target(t);
                    }
                }
                self.stmts(&w.body);
            }
            Stmt::Match(m) => {
                self.expr(&m.subject);
                for case in &m.cases {
                    self.pattern(&case.pattern);
                    if let Some(g) = &case.guard {
                        self.expr(g);
                    }
                    self.stmts(&case.body);
                }
            }
            // No names: control / no-ops / type-level imports.
            Stmt::Pass(_) | Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::Import(_) | Stmt::ImportFrom(_) => {}
            // Anything else is outside the lowered subset (rejected there).
            _ => {}
        }
    }

    /// A `match` pattern (Phase 7E): captures bind; value/class-name reads read.
    fn pattern(&mut self, p: &rustpython_parser::ast::Pattern) {
        use rustpython_parser::ast::Pattern;
        match p {
            Pattern::MatchValue(v) => self.expr(&v.value),
            Pattern::MatchSingleton(_) => {}
            Pattern::MatchSequence(s) => {
                for sub in &s.patterns {
                    self.pattern(sub);
                }
            }
            Pattern::MatchMapping(m) => {
                for k in &m.keys {
                    self.expr(k);
                }
                for sub in &m.patterns {
                    self.pattern(sub);
                }
                if let Some(rest) = &m.rest {
                    self.bind(rest.as_str());
                }
            }
            Pattern::MatchClass(c) => {
                self.expr(&c.cls);
                for sub in &c.patterns {
                    self.pattern(sub);
                }
                for sub in &c.kwd_patterns {
                    self.pattern(sub);
                }
            }
            Pattern::MatchStar(s) => {
                if let Some(name) = &s.name {
                    self.bind(name.as_str());
                }
            }
            Pattern::MatchAs(a) => {
                if let Some(sub) = &a.pattern {
                    self.pattern(sub);
                }
                if let Some(name) = &a.name {
                    self.bind(name.as_str());
                }
            }
            Pattern::MatchOr(o) => {
                for sub in &o.patterns {
                    self.pattern(sub);
                }
            }
        }
    }

    fn stmts(&mut self, body: &[Stmt]) {
        for s in body {
            self.stmt(s);
        }
    }

    /// An assignment target: `Name`s bind; subscript / attribute bases are
    /// reads; tuple/list patterns recurse.
    fn target(&mut self, t: &Expr) {
        match t {
            Expr::Name(n) => self.bind(n.id.as_str()),
            Expr::Tuple(tt) => {
                for e in &tt.elts {
                    self.target(e);
                }
            }
            Expr::List(l) => {
                for e in &l.elts {
                    self.target(e);
                }
            }
            Expr::Starred(s) => self.target(&s.value),
            Expr::Subscript(s) => {
                self.expr(&s.value);
                self.expr(&s.slice);
            }
            Expr::Attribute(a) => self.expr(&a.value),
            other => self.expr(other),
        }
    }

    // ── expressions ──

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Name(n) => self.read(n.id.as_str()),
            Expr::Constant(_) => {}
            Expr::UnaryOp(u) => self.expr(&u.operand),
            Expr::BinOp(b) => {
                self.expr(&b.left);
                self.expr(&b.right);
            }
            Expr::BoolOp(b) => {
                for v in &b.values {
                    self.expr(v);
                }
            }
            Expr::Compare(c) => {
                self.expr(&c.left);
                for v in &c.comparators {
                    self.expr(v);
                }
            }
            Expr::IfExp(t) => {
                self.expr(&t.test);
                self.expr(&t.body);
                self.expr(&t.orelse);
            }
            Expr::Call(c) => {
                self.expr(&c.func);
                for a in &c.args {
                    self.expr(a);
                }
                for k in &c.keywords {
                    self.expr(&k.value);
                }
            }
            Expr::Attribute(a) => self.expr(&a.value),
            Expr::Subscript(s) => {
                self.expr(&s.value);
                self.expr(&s.slice);
            }
            Expr::List(l) => {
                for x in &l.elts {
                    self.expr(x);
                }
            }
            Expr::Tuple(t) => {
                for x in &t.elts {
                    self.expr(x);
                }
            }
            Expr::Set(s) => {
                for x in &s.elts {
                    self.expr(x);
                }
            }
            Expr::Dict(d) => {
                for k in d.keys.iter().flatten() {
                    self.expr(k);
                }
                for v in &d.values {
                    self.expr(v);
                }
            }
            Expr::Starred(s) => self.expr(&s.value),
            Expr::Slice(s) => {
                for part in [&s.lower, &s.upper, &s.step].into_iter().flatten() {
                    self.expr(part);
                }
            }
            // Comprehensions lower INLINE in the enclosing scope (the Phase-4C
            // model), so their targets bind here.
            Expr::ListComp(c) => self.comp(&c.generators, std::slice::from_ref(&c.elt)),
            Expr::SetComp(c) => self.comp(&c.generators, std::slice::from_ref(&c.elt)),
            Expr::DictComp(c) => {
                let kv = [(*c.key).clone(), (*c.value).clone()];
                self.comp(&c.generators, &kv);
            }
            // A genexpr IS its own scope (Phase 6E): the outermost iterable is
            // evaluated eagerly here; everything else belongs to the child.
            Expr::GeneratorExp(g) => {
                if let Some(first) = g.generators.first() {
                    self.expr(&first.iter);
                }
                let facts = analyze_genexpr(g);
                self.child(&facts);
            }
            Expr::Lambda(l) => {
                // Lambda defaults evaluate in this scope.
                for awd in l.args.posonlyargs.iter().chain(&l.args.args).chain(&l.args.kwonlyargs)
                {
                    if let Some(dflt) = &awd.default {
                        self.expr(dflt);
                    }
                }
                let facts = analyze_lambda(l);
                self.child(&facts);
            }
            Expr::Yield(y) => {
                if let Some(v) = &y.value {
                    self.expr(v);
                }
            }
            Expr::YieldFrom(y) => self.expr(&y.value),
            Expr::NamedExpr(n) => {
                self.expr(&n.value);
                self.target(&n.target);
            }
            // Anything else is outside the lowered subset.
            _ => {}
        }
    }

    /// Inline comprehension clauses: bind every `for` target, read iters /
    /// filters / element expressions.
    fn comp(&mut self, generators: &[Comprehension], elts: &[Expr]) {
        for g in generators {
            self.expr(&g.iter);
            self.target(&g.target);
            for c in &g.ifs {
                self.expr(c);
            }
        }
        for e in elts {
            self.expr(e);
        }
    }
}

/// Bind every parameter name of an `Arguments` node.
fn bind_params(w: &mut Walker, args: &rustpython_parser::ast::Arguments) {
    for awd in args.posonlyargs.iter().chain(&args.args).chain(&args.kwonlyargs) {
        w.bind(awd.def.arg.as_str());
    }
    if let Some(v) = &args.vararg {
        w.bind(v.arg.as_str());
    }
    if let Some(k) = &args.kwarg {
        w.bind(k.arg.as_str());
    }
}

/// Analyze a `def`'s scope.
pub(crate) fn analyze_def(def: &StmtFunctionDef) -> ScopeFacts {
    let mut w = Walker::default();
    bind_params(&mut w, &def.args);
    w.stmts(&def.body);
    w.finish()
}

/// Analyze a lambda's scope.
pub(crate) fn analyze_lambda(l: &ExprLambda) -> ScopeFacts {
    let mut w = Walker::default();
    bind_params(&mut w, &l.args);
    w.expr(&l.body);
    w.finish()
}

/// Analyze a generator expression's scope (Phase 6E): every `for` target binds
/// inside; the OUTERMOST iterable is excluded (evaluated eagerly outside).
pub(crate) fn analyze_genexpr(g: &ExprGeneratorExp) -> ScopeFacts {
    let mut w = Walker::default();
    for (i, gen) in g.generators.iter().enumerate() {
        if i > 0 {
            w.expr(&gen.iter);
        }
        w.target(&gen.target);
        for c in &gen.ifs {
            w.expr(c);
        }
    }
    w.expr(&g.elt);
    w.finish()
}

/// Analyze the module body as `__main__`'s scope (only the statements lowered
/// into `__main__` — top-level `def`s/`class`es are partitioned out and reach
/// module names as globals, not captures). `__name__` is pre-bound.
pub(crate) fn analyze_module_body(stmts: &[&Stmt]) -> ScopeFacts {
    let mut w = Walker::default();
    w.bind("__name__");
    for s in stmts {
        w.stmt(s);
    }
    w.finish()
}

/// The promoted module globals (Phase 6B), in deterministic first-qualification
/// order (the dense `var_id` is the index): every `global`-declared name
/// anywhere, plus every module-assigned name that some function reads. Names of
/// top-level `def`s / `class`es are excluded — they resolve through the symbol
/// table, not global slots.
pub(crate) fn collect_promoted_globals(body: &[Stmt]) -> Vec<String> {
    // Partition exactly like the module lowering does.
    let mut def_or_class: HashSet<String> = HashSet::new();
    let mut top: Vec<&Stmt> = Vec::new();
    let mut fn_facts: Vec<ScopeFacts> = Vec::new();
    for stmt in body {
        match stmt {
            Stmt::FunctionDef(d) => {
                def_or_class.insert(d.name.as_str().to_string());
                fn_facts.push(analyze_def(d));
            }
            Stmt::ClassDef(c) => {
                def_or_class.insert(c.name.as_str().to_string());
                for m in &c.body {
                    if let Stmt::FunctionDef(d) = m {
                        fn_facts.push(analyze_def(d));
                    }
                }
            }
            other => top.push(other),
        }
    }
    let main_facts = analyze_module_body(&top);

    // Names assigned at module level (main's bound set, minus the pre-bound
    // `__name__`, minus def/class names).
    let module_assigned: HashSet<&String> = main_facts
        .bound
        .iter()
        .filter(|n| *n != "__name__" && !def_or_class.contains(*n))
        .collect();
    // Names functions read (their bubbled free sets) or declare `global`.
    let mut promoted: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let push = |n: &String, promoted: &mut Vec<String>, seen: &mut HashSet<String>| {
        if !def_or_class.contains(n) && seen.insert(n.clone()) {
            promoted.push(n.clone());
        }
    };
    // 1. `global`-declared names (anywhere) always get a slot.
    for facts in fn_facts.iter().chain(std::iter::once(&main_facts)) {
        let mut decls: Vec<&String> = facts.global_decls.iter().collect();
        decls.sort();
        for n in decls {
            push(n, &mut promoted, &mut seen);
        }
    }
    // 2. Module-assigned names read inside some function.
    for facts in &fn_facts {
        for n in &facts.free {
            if module_assigned.contains(n) {
                push(n, &mut promoted, &mut seen);
            }
        }
    }
    promoted
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustpython_parser::{parse, Mode};

    fn def_facts(src: &str) -> ScopeFacts {
        let parsed = parse(src, Mode::Module, "<test>").expect("parse");
        let rustpython_parser::ast::Mod::Module(m) = parsed else { panic!("module") };
        let Stmt::FunctionDef(d) = &m.body[0] else { panic!("def") };
        analyze_def(d)
    }

    #[test]
    fn transitive_bubbling_three_levels() {
        // `x` is read only by the innermost def; it must bubble through `mid`
        // so `outer` cells it.
        let facts = def_facts(
            "def outer():\n    x = 1\n    def mid():\n        def inner():\n            return x\n        return inner\n    return mid\n",
        );
        assert!(facts.celled.contains("x"), "x must be celled in outer");
        assert!(facts.free.is_empty(), "outer itself has no free names");
    }

    #[test]
    fn lambda_capture_and_builtin_not_bound() {
        let facts = def_facts(
            "def outer(n):\n    f = lambda y: y + n\n    return f\n",
        );
        assert!(facts.celled.contains("n"));
        // `f` is bound but never captured.
        assert!(!facts.celled.contains("f"));
    }

    #[test]
    fn nonlocal_joins_owner_celled_and_marks_shared_write() {
        let facts = def_facts(
            "def outer():\n    n = 0\n    def inc():\n        nonlocal n\n        n = n + 1\n    return inc\n",
        );
        assert!(facts.celled.contains("n"));
        assert!(facts.shared_writes.contains("n"));
    }

    #[test]
    fn genexpr_outer_iterable_is_eager() {
        // `xs` is read in the enclosing scope (eager); `x` is bound inside the
        // genexpr and never escapes.
        let facts = def_facts(
            "def outer(xs):\n    g = (x * 2 for x in xs)\n    return g\n",
        );
        // `xs` is read directly here (eager), NOT via the genexpr child — it
        // must not be celled; the genexpr receives it as an eager parameter.
        assert!(!facts.celled.contains("xs"));
        assert!(!facts.celled.contains("x"));
    }

    #[test]
    fn promoted_globals_collects_global_decls_and_module_reads() {
        use rustpython_parser::ast::Mod;
        let src = "\
counter = 0
scale = 7
def bump():
    global counter
    counter = counter + 1
def scaled(x):
    return x * scale
";
        let parsed = parse(src, Mode::Module, "<test>").expect("parse");
        let Mod::Module(m) = parsed else { panic!("module") };
        let promoted = collect_promoted_globals(&m.body);
        // `counter` (a `global` decl) and `scale` (module-assigned, read in a
        // function) are promoted; the def names are not.
        assert!(promoted.contains(&"counter".to_string()));
        assert!(promoted.contains(&"scale".to_string()));
        assert!(!promoted.contains(&"bump".to_string()));
        assert!(!promoted.contains(&"scaled".to_string()));
    }

    #[test]
    fn global_declaration_is_not_free_or_bound() {
        let facts = def_facts(
            "def f():\n    global g\n    g = 1\n    return g\n",
        );
        assert!(facts.globals.contains("g"));
        assert!(!facts.bound.contains("g"));
        assert!(!facts.free.contains(&"g".to_string()));
    }
}
