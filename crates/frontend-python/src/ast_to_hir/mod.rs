//! Convert Python AST to HIR

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::*;
use pyaot_stdlib_defs;
use pyaot_types::Type;
use pyaot_utils::{ClassId, FuncId, InternedString, Span, StringInterner, VarId};
use rustpython_parser::ast as py;
use rustpython_parser::ast::Ranged;
use std::collections::{HashMap, HashSet};

// Submodules
mod builtins;
mod classes;
mod comprehensions;
mod expressions;
mod fstrings;
mod functions;
mod lambdas;
mod operators;
mod statements;
mod types;
mod variables;

/// Information about an imported name
#[derive(Debug, Clone)]
pub struct ImportedName {
    /// Source module name
    pub module: String,
    /// Original name in the source module
    pub original_name: String,
    /// Kind of import (will be resolved later after multi-module merging)
    pub kind: ImportedNameKind,
}

/// Kind of imported name (before resolution)
#[derive(Debug, Clone)]
pub enum ImportedNameKind {
    /// Not yet resolved
    Unresolved,
    /// Resolved to a function
    Function(FuncId),
    /// Resolved to a class
    Class(ClassId),
    /// Resolved to a variable
    Variable(VarId),
}

/// Standard library item (attribute, function, or constant)
/// Uses references to definitions for Single Source of Truth
#[derive(Debug, Clone, Copy)]
pub enum StdlibItem {
    /// Attribute access (e.g., argv, environ)
    Attr(&'static pyaot_stdlib_defs::StdlibAttrDef),
    /// Function call (e.g., exit, join, search)
    Func(&'static pyaot_stdlib_defs::StdlibFunctionDef),
    /// Compile-time constant (e.g., math.pi, math.e)
    Const(&'static pyaot_stdlib_defs::StdlibConstDef),
}

/// TypeVar definition from `T = TypeVar('T', ...)`
#[derive(Debug, Clone)]
pub struct TypeVarDef {
    /// Constraint types: TypeVar('T', int, str) → [int, str]
    pub constraints: Vec<Type>,
    /// Bound type: TypeVar('T', bound=SomeType) → Some(SomeType)
    pub bound: Option<Type>,
}

/// Allocates unique IDs for variables, functions, classes, lambdas,
/// comprehensions, and context managers.
pub(crate) struct IdAllocator {
    pub(crate) next_var_id: u32,
    pub(crate) next_func_id: u32,
    pub(crate) next_class_id: u32,
    pub(crate) next_lambda_id: u32,
    pub(crate) next_comp_id: u32,
    pub(crate) next_ctx_id: u32,
}

impl IdAllocator {
    fn new() -> Self {
        Self {
            next_var_id: 0,
            next_func_id: 0,
            // Start class IDs after built-in exception tags (0..28) AND
            // the reserved stdlib-exception slot range (29..60) to avoid
            // collisions in the runtime vtable/class registry. See
            // `core-defs::FIRST_USER_CLASS_ID` for the constant and
            // `StdlibExceptionClass` for the producers.
            next_class_id: pyaot_types::FIRST_USER_CLASS_ID as u32,
            next_lambda_id: 0,
            next_comp_id: 0,
            next_ctx_id: 0,
        }
    }

    pub(crate) fn alloc_var(&mut self) -> VarId {
        let id = VarId::new(self.next_var_id);
        self.next_var_id += 1;
        id
    }

    pub(crate) fn alloc_func(&mut self) -> FuncId {
        let id = FuncId::new(self.next_func_id);
        self.next_func_id += 1;
        id
    }

    pub(crate) fn alloc_class(&mut self) -> ClassId {
        let id = ClassId::new(self.next_class_id);
        self.next_class_id += 1;
        id
    }
}

/// Maps names to their corresponding IDs in the current and module scopes.
pub(crate) struct SymbolTable {
    /// Variable names → IDs (current scope)
    pub(crate) var_map: HashMap<InternedString, VarId>,
    /// Function names → IDs
    pub(crate) func_map: HashMap<InternedString, FuncId>,
    /// Class names → IDs
    pub(crate) class_map: HashMap<InternedString, ClassId>,
    /// Module-level variable names → IDs
    pub(crate) module_var_map: HashMap<InternedString, VarId>,
}

impl SymbolTable {
    fn new() -> Self {
        Self {
            var_map: HashMap::new(),
            func_map: HashMap::new(),
            class_map: HashMap::new(),
            module_var_map: HashMap::new(),
        }
    }
}

/// Tracks scope-related state: scope stack, current class, global/nonlocal
/// declarations, cell variables, initialization tracking, and generator flags.
pub(crate) struct ScopeContext {
    /// Current class being processed (for method conversion)
    pub(crate) current_class: Option<ClassId>,
    /// Interned name of the current class. Set alongside `current_class` so
    /// that `Type::Class { .. }` values built during method body conversion
    /// use the canonical class name — `class_defs` is not populated until
    /// after the class body is fully walked, so `class_defs.get(..).name`
    /// returns `None` mid-walk and produces drifted names that break
    /// Union deduplication.
    pub(crate) current_class_name: Option<InternedString>,
    /// Variables declared as global in the current function scope
    pub(crate) global_vars: HashSet<InternedString>,
    /// Variables declared as nonlocal in the current function scope
    pub(crate) nonlocal_vars: HashSet<InternedString>,
    /// Stack of enclosing scopes' variable maps (for nonlocal lookup)
    pub(crate) scope_stack: Vec<HashMap<InternedString, VarId>>,
    /// Variables that need to be wrapped in cells (used via nonlocal in inner function)
    pub(crate) current_cell_vars: HashSet<VarId>,
    /// Variables initialized (assigned) in the current scope — for unbound detection
    pub(crate) initialized_vars: HashSet<InternedString>,
    /// Whether the current function contains yield (is a generator)
    pub(crate) current_func_is_generator: bool,
    /// Pending statements from comprehension desugaring
    pub(crate) pending_stmts: Vec<StmtId>,
    /// Variables explicitly assigned at module level (via Assign/AnnAssign)
    pub(crate) module_level_assignments: HashSet<VarId>,
}

impl ScopeContext {
    fn new() -> Self {
        Self {
            current_class: None,
            current_class_name: None,
            global_vars: HashSet::new(),
            nonlocal_vars: HashSet::new(),
            scope_stack: Vec::new(),
            current_cell_vars: HashSet::new(),
            initialized_vars: HashSet::new(),
            current_func_is_generator: false,
            pending_stmts: Vec::new(),
            module_level_assignments: HashSet::new(),
        }
    }
}

/// Tracks import-related state: imported names, modules, stdlib items.
pub(crate) struct ImportResolver {
    /// Imported names: local name → import info
    pub(crate) imported_names: HashMap<InternedString, ImportedName>,
    /// Imported modules: module alias → module path
    pub(crate) imported_modules: HashMap<InternedString, String>,
    /// Stdlib imports: "sys", "os", "re"
    pub(crate) stdlib_imports: HashSet<InternedString>,
    /// Stdlib names from "from X import Y": local name → stdlib item
    pub(crate) stdlib_names: HashMap<InternedString, StdlibItem>,
    /// Dotted imports: "pkg.submodule" → "pkg.submodule"
    pub(crate) dotted_imports: HashMap<String, String>,
}

impl ImportResolver {
    fn new() -> Self {
        Self {
            imported_names: HashMap::new(),
            imported_modules: HashMap::new(),
            stdlib_imports: HashSet::new(),
            stdlib_names: HashMap::new(),
            dotted_imports: HashMap::new(),
        }
    }
}

/// Tracks type annotation state: typing imports, aliases, TypeVars.
pub(crate) struct TypeContext {
    /// Imported names from typing module (List, Dict, Optional, etc.)
    pub(crate) typing_imports: HashSet<InternedString>,
    /// Type aliases: MyType: TypeAlias = int, or type MyType = int
    pub(crate) type_aliases: HashMap<InternedString, Type>,
    /// TypeVar definitions: T = TypeVar('T', ...)
    pub(crate) typevar_defs: HashMap<InternedString, TypeVarDef>,
}

impl TypeContext {
    fn new() -> Self {
        Self {
            typing_imports: HashSet::new(),
            type_aliases: HashMap::new(),
            typevar_defs: HashMap::new(),
        }
    }
}

pub struct AstToHir {
    pub(crate) interner: StringInterner,
    pub(crate) module: Module,
    pub(crate) ids: IdAllocator,
    pub(crate) symbols: SymbolTable,
    pub(crate) scope: ScopeContext,
    pub(crate) imports: ImportResolver,
    pub(crate) types: TypeContext,
}

impl AstToHir {
    pub fn new(module_name: &str) -> Self {
        let mut interner = StringInterner::new();
        let name = interner.intern(module_name);
        let mut module = Module::new(name);
        module.source_module_name = Some(module_name.to_string());
        Self {
            interner,
            module,
            ids: IdAllocator::new(),
            symbols: SymbolTable::new(),
            scope: ScopeContext::new(),
            imports: ImportResolver::new(),
            types: TypeContext::new(),
        }
    }

    /// Convert a RustPython AST node's range to our Span type
    pub(crate) fn span_from<T: Ranged>(node: &T) -> Span {
        let range = node.range();
        Span::new(u32::from(range.start()), u32::from(range.end()))
    }

    /// Allocate a placeholder `ClassId` for a cross-module user-class type
    /// annotation. The real class id is only known after `mir_merger`'s first
    /// pass, so we hand out ids from a reserved high range and rely on the
    /// merger to rewrite `Type::Class` references before lowering. Same
    /// `(module, name)` pair returns the same placeholder so type equality
    /// across multiple annotations (e.g. `def f(x: Resp, y: Resp)`) holds.
    pub(crate) fn alloc_external_class_ref(&mut self, module: String, name: String) -> ClassId {
        for (&id, entry) in &self.module.external_class_refs {
            if entry.0 == module && entry.1 == name {
                return id;
            }
        }
        // Placeholder ids come from the top of u32 space so they never
        // collide with local user class ids (which grow from
        // `FIRST_USER_CLASS_ID` upward).
        let next_index = self.module.external_class_refs.len() as u32;
        let class_id = ClassId::new(u32::MAX - next_index);
        self.module
            .external_class_refs
            .insert(class_id, (module, name));
        class_id
    }

    pub fn convert(mut self, ast: py::Mod) -> Result<(Module, StringInterner)> {
        match ast {
            py::Mod::Module(m) => {
                // Pre-scan: register every top-level class name before any
                // body is converted. Mirrors the `func_map` pre-existing
                // pattern for forward function references — without this,
                // `class A: def make(self) -> "B": ...; class B: ...` would
                // fail to resolve `B` while parsing `A`. Needed for PEP 563
                // string annotations and for recursive/forward class refs
                // in method signatures.
                self.prescan_top_level_classes(&m.body);
                for stmt in m.body {
                    self.convert_top_level_stmt(stmt)?;
                }
            }
            _ => {
                return Err(CompilerError::parse_error(
                    "Expected module",
                    Span::dummy(), // Mod type doesn't implement Ranged
                ));
            }
        }
        // Create synthetic __pyaot_module_init__ function for top-level statements
        self.finalize_module();
        Ok((self.module, self.interner))
    }

    /// Reserve `ClassId`s for every top-level class declaration so that
    /// forward references in method annotations resolve to the correct id.
    /// Bodies are converted later in source order — this pre-pass only
    /// populates `class_map` with the names.
    fn prescan_top_level_classes(&mut self, stmts: &[py::Stmt]) {
        for stmt in stmts {
            if let py::Stmt::ClassDef(cls) = stmt {
                let name = self.interner.intern(&cls.name);
                if self.symbols.class_map.contains_key(&name) {
                    continue;
                }
                let class_id = self.ids.alloc_class();
                self.symbols.class_map.insert(name, class_id);
            }
        }
    }

    fn convert_top_level_stmt(&mut self, stmt: py::Stmt) -> Result<()> {
        let stmt_span = Self::span_from(&stmt);
        match stmt {
            py::Stmt::FunctionDef(func_def) => {
                self.convert_function_def(func_def)?;
            }
            py::Stmt::ClassDef(class_def) => {
                self.convert_class_def(class_def)?;
            }
            // PEP 695: type MyType = int
            py::Stmt::TypeAlias(ta) => {
                self.convert_type_alias_stmt(ta, stmt_span)?;
            }
            _ => {
                // Check for TypeVar assignment: T = TypeVar('T', ...)
                if let py::Stmt::Assign(ref assign) = stmt {
                    if self.is_typevar_assignment(assign) {
                        // Use module-level scope for type resolution
                        std::mem::swap(&mut self.symbols.var_map, &mut self.symbols.module_var_map);
                        let result = self.handle_typevar_assignment(assign, stmt_span);
                        std::mem::swap(&mut self.symbols.var_map, &mut self.symbols.module_var_map);
                        result?;
                        return Ok(());
                    }
                }

                // Accept all other statements at module level (CPython semantics)
                // Use module-level variable scope
                std::mem::swap(&mut self.symbols.var_map, &mut self.symbols.module_var_map);

                // Check if this is an assignment statement - we need to track
                // explicitly assigned variables for globals detection
                let is_assignment = matches!(&stmt, py::Stmt::Assign(_) | py::Stmt::AnnAssign(_));

                // Get the target variable name before conversion
                let target_name = match &stmt {
                    py::Stmt::Assign(assign) => {
                        // Get first target (for simple assignments)
                        if let Some(py::Expr::Name(name)) = assign.targets.first() {
                            Some(self.interner.intern(&name.id))
                        } else {
                            None
                        }
                    }
                    py::Stmt::AnnAssign(ann_assign) => {
                        if let py::Expr::Name(name) = &*ann_assign.target {
                            Some(self.interner.intern(&name.id))
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                let stmt_id = self.convert_stmt(stmt)?;

                // Mark the target variable as a module-level assignment
                if is_assignment {
                    if let Some(name) = target_name {
                        if let Some(&var_id) = self.symbols.var_map.get(&name) {
                            self.scope.module_level_assignments.insert(var_id);
                        }
                    }
                }

                // Inject any pending statements from comprehensions before this statement
                let pending = self.take_pending_stmts();
                std::mem::swap(&mut self.symbols.var_map, &mut self.symbols.module_var_map);
                self.module.module_init_stmts.extend(pending);
                self.module.module_init_stmts.push(stmt_id);
            }
        }
        Ok(())
    }

    fn finalize_module(&mut self) {
        // Copy module-level variable map to the HIR module for cross-module access
        // Note: We only add explicitly assigned module-level variables to globals,
        // not loop target variables or other temporary variables.
        for (name, var_id) in &self.symbols.module_var_map {
            self.module.module_var_map.insert(*name, *var_id);
            // Only add to globals if it's in module_level_assignments
            // (i.e., it was explicitly assigned at module level via Assign/AnnAssign)
            if self.scope.module_level_assignments.contains(var_id) {
                self.module.globals.insert(*var_id);
            }
        }

        if self.module.module_init_stmts.is_empty() {
            return;
        }

        let func_id = self.ids.alloc_func();
        let func_name = self.interner.intern("__pyaot_module_init__");

        let body_stmts = self.module.module_init_stmts.clone();
        let (blocks, entry_block, try_scopes) =
            cfg_build::build_cfg_from_tree(&body_stmts, &mut self.module);
        let function = Function {
            id: func_id,
            name: func_name,
            params: Vec::new(),
            return_type: Some(Type::None),
            body: body_stmts,
            span: Span::dummy(),
            cell_vars: std::collections::HashSet::new(),
            nonlocal_vars: std::collections::HashSet::new(),
            is_generator: false,
            method_kind: MethodKind::default(), // Module init is not a method
            is_abstract: false,
            blocks,
            entry_block,
            try_scopes,
        };

        self.module.functions.push(func_id);
        self.module.func_defs.insert(func_id, function);
    }

    pub(crate) fn take_pending_stmts(&mut self) -> Vec<StmtId> {
        std::mem::take(&mut self.scope.pending_stmts)
    }

    /// Handle PEP 695 `type MyType = ...` statement
    fn convert_type_alias_stmt(&mut self, ta: py::StmtTypeAlias, stmt_span: Span) -> Result<()> {
        if let py::Expr::Name(name) = &*ta.name {
            let alias_name = self.interner.intern(&name.id);
            let aliased_type = self.convert_type_annotation(&ta.value)?;
            self.types.type_aliases.insert(alias_name, aliased_type);
            Ok(())
        } else {
            Err(CompilerError::parse_error(
                "type alias name must be a simple name",
                stmt_span,
            ))
        }
    }

    /// Check if an assignment is `T = TypeVar('T', ...)`
    fn is_typevar_assignment(&self, assign: &py::StmtAssign) -> bool {
        if assign.targets.len() != 1 {
            return false;
        }
        if !matches!(&assign.targets[0], py::Expr::Name(_)) {
            return false;
        }
        if let py::Expr::Call(call) = &*assign.value {
            if let py::Expr::Name(func_name) = &*call.func {
                if func_name.id.as_str() == "TypeVar" {
                    // Check if TypeVar was imported from typing
                    return self
                        .types
                        .typing_imports
                        .iter()
                        .any(|s| self.interner.resolve(*s) == "TypeVar");
                }
            }
        }
        false
    }

    /// Handle `T = TypeVar('T', ...)` assignment
    fn handle_typevar_assignment(
        &mut self,
        assign: &py::StmtAssign,
        _stmt_span: Span,
    ) -> Result<()> {
        let target_name = if let py::Expr::Name(name) = &assign.targets[0] {
            self.interner.intern(&name.id)
        } else {
            unreachable!("is_typevar_assignment checked this");
        };

        if let py::Expr::Call(call) = &*assign.value {
            let tv_def = self.parse_typevar_call(call)?;
            self.types.typevar_defs.insert(target_name, tv_def);
        }
        Ok(())
    }

    /// Parse a TypeVar('T', int, str, bound=X) call
    fn parse_typevar_call(&mut self, call: &py::ExprCall) -> Result<TypeVarDef> {
        let mut constraints = Vec::new();
        let mut bound = None;

        // Skip first positional arg (the name string 'T')
        for (i, arg) in call.args.iter().enumerate() {
            if i == 0 {
                continue;
            }
            constraints.push(self.convert_type_annotation(arg)?);
        }

        // Check for bound= keyword argument
        for kw in &call.keywords {
            if kw.arg.as_deref() == Some("bound") {
                bound = Some(self.convert_type_annotation(&kw.value)?);
            }
        }

        Ok(TypeVarDef { constraints, bound })
    }

    /// Infer the type from a literal expression
    pub(crate) fn infer_literal_type(&self, expr: &Expr) -> Type {
        match &expr.kind {
            ExprKind::Int(_) => Type::Int,
            ExprKind::Float(_) => Type::Float,
            ExprKind::Bool(_) => Type::Bool,
            ExprKind::Str(_) => Type::Str,
            ExprKind::Bytes(_) => Type::Bytes,
            ExprKind::None => Type::None,
            ExprKind::List(elems) => {
                // Infer element type from first element if available
                if let Some(first_elem_id) = elems.first() {
                    let first_elem = &self.module.exprs[*first_elem_id];
                    let elem_ty = self.infer_literal_type(first_elem);
                    Type::List(Box::new(elem_ty))
                } else {
                    Type::List(Box::new(Type::Any))
                }
            }
            ExprKind::Dict(pairs) => {
                if let Some((key_id, val_id)) = pairs.first() {
                    let key_ty = self.infer_literal_type(&self.module.exprs[*key_id]);
                    let val_ty = self.infer_literal_type(&self.module.exprs[*val_id]);
                    Type::Dict(Box::new(key_ty), Box::new(val_ty))
                } else {
                    Type::Dict(Box::new(Type::Any), Box::new(Type::Any))
                }
            }
            ExprKind::Tuple(elems) => {
                let elem_types: Vec<Type> = elems
                    .iter()
                    .map(|e| self.infer_literal_type(&self.module.exprs[*e]))
                    .collect();
                Type::Tuple(elem_types)
            }
            ExprKind::Set(elems) => {
                if let Some(first_elem_id) = elems.first() {
                    let first_elem = &self.module.exprs[*first_elem_id];
                    let elem_ty = self.infer_literal_type(first_elem);
                    Type::Set(Box::new(elem_ty))
                } else {
                    Type::Set(Box::new(Type::Any))
                }
            }
            // For other expressions, use the type annotation if available
            _ => expr.ty.clone().unwrap_or(Type::Any),
        }
    }
}
