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

pub struct AstToHir {
    pub(crate) interner: StringInterner,
    pub(crate) module: Module,
    pub(crate) next_var_id: u32,
    pub(crate) next_func_id: u32,
    pub(crate) next_class_id: u32,
    pub(crate) next_lambda_id: u32,
    pub(crate) next_comp_id: u32,
    pub(crate) next_ctx_id: u32,

    // Track variable names to IDs (current scope)
    pub(crate) var_map: HashMap<InternedString, VarId>,
    // Track function names to IDs
    pub(crate) func_map: HashMap<InternedString, FuncId>,
    // Track class names to IDs
    pub(crate) class_map: HashMap<InternedString, ClassId>,
    // Track module-level variable names to IDs
    pub(crate) module_var_map: HashMap<InternedString, VarId>,
    // Current class being processed (for method conversion)
    pub(crate) current_class: Option<ClassId>,
    // Pending statements from comprehension desugaring (to be injected before containing stmt)
    pub(crate) pending_stmts: Vec<StmtId>,
    // Track imported names from typing module (List, Dict, Optional, etc.)
    pub(crate) typing_imports: HashSet<InternedString>,
    // Track variables declared as global in the current function scope
    pub(crate) global_vars: HashSet<InternedString>,
    // Track variables declared as nonlocal in the current function scope
    pub(crate) nonlocal_vars: HashSet<InternedString>,
    // Stack of enclosing scopes' variable maps (for nonlocal lookup)
    pub(crate) scope_stack: Vec<HashMap<InternedString, VarId>>,
    // Variables in the current function that need to be wrapped in cells
    // (because they are used via nonlocal in an inner function)
    pub(crate) current_cell_vars: HashSet<VarId>,
    // Variables that have been initialized (assigned) in the current scope
    // Used to detect unbound nonlocal errors at compile time
    pub(crate) initialized_vars: HashSet<InternedString>,
    // Track whether the current function contains yield (is a generator)
    pub(crate) current_func_is_generator: bool,
    // Track imported names: local name -> import info
    pub(crate) imported_names: HashMap<InternedString, ImportedName>,
    // Track imported modules: module alias -> module path
    pub(crate) imported_modules: HashMap<InternedString, String>,
    // Track variables explicitly assigned at module level (via Assign/AnnAssign)
    // These should be treated as globals for cross-module access
    pub(crate) module_level_assignments: HashSet<VarId>,
    // Track stdlib imports: "sys", "os", "re"
    pub(crate) stdlib_imports: HashSet<InternedString>,
    // Track stdlib names from "from X import Y": local name -> stdlib item
    pub(crate) stdlib_names: HashMap<InternedString, StdlibItem>,
    // Track dotted imports: "pkg.submodule" -> "pkg.submodule"
    // For resolving chained attribute access like pkg.sub.func()
    pub(crate) dotted_imports: HashMap<String, String>,
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
            next_var_id: 0,
            next_func_id: 0,
            next_class_id: 0,
            next_lambda_id: 0,
            next_comp_id: 0,
            next_ctx_id: 0,
            var_map: HashMap::new(),
            func_map: HashMap::new(),
            class_map: HashMap::new(),
            module_var_map: HashMap::new(),
            current_class: None,
            pending_stmts: Vec::new(),
            typing_imports: HashSet::new(),
            global_vars: HashSet::new(),
            nonlocal_vars: HashSet::new(),
            scope_stack: Vec::new(),
            current_cell_vars: HashSet::new(),
            initialized_vars: HashSet::new(),
            current_func_is_generator: false,
            imported_names: HashMap::new(),
            imported_modules: HashMap::new(),
            module_level_assignments: HashSet::new(),
            stdlib_imports: HashSet::new(),
            stdlib_names: HashMap::new(),
            dotted_imports: HashMap::new(),
        }
    }

    /// Convert a RustPython AST node's range to our Span type
    pub(crate) fn span_from<T: Ranged>(node: &T) -> Span {
        let range = node.range();
        Span::new(u32::from(range.start()), u32::from(range.end()))
    }

    pub fn convert(mut self, ast: py::Mod) -> Result<(Module, StringInterner)> {
        match ast {
            py::Mod::Module(m) => {
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

    fn convert_top_level_stmt(&mut self, stmt: py::Stmt) -> Result<()> {
        match stmt {
            py::Stmt::FunctionDef(func_def) => {
                self.convert_function_def(func_def)?;
            }
            py::Stmt::ClassDef(class_def) => {
                self.convert_class_def(class_def)?;
            }
            _ => {
                // Accept all other statements at module level (CPython semantics)
                // Use module-level variable scope
                std::mem::swap(&mut self.var_map, &mut self.module_var_map);

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
                        if let Some(&var_id) = self.var_map.get(&name) {
                            self.module_level_assignments.insert(var_id);
                        }
                    }
                }

                // Inject any pending statements from comprehensions before this statement
                let pending = self.take_pending_stmts();
                std::mem::swap(&mut self.var_map, &mut self.module_var_map);
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
        for (name, var_id) in &self.module_var_map {
            self.module.module_var_map.insert(*name, *var_id);
            // Only add to globals if it's in module_level_assignments
            // (i.e., it was explicitly assigned at module level via Assign/AnnAssign)
            if self.module_level_assignments.contains(var_id) {
                self.module.globals.insert(*var_id);
            }
        }

        if self.module.module_init_stmts.is_empty() {
            return;
        }

        let func_id = self.alloc_func_id();
        let func_name = self.interner.intern("__pyaot_module_init__");

        let function = Function {
            id: func_id,
            name: func_name,
            params: Vec::new(),
            return_type: Some(Type::None),
            body: self.module.module_init_stmts.clone(),
            span: Span::dummy(),
            cell_vars: std::collections::HashSet::new(),
            nonlocal_vars: std::collections::HashSet::new(),
            is_generator: false,
            method_kind: MethodKind::default(), // Module init is not a method
            is_abstract: false,
        };

        self.module.functions.push(func_id);
        self.module.func_defs.insert(func_id, function);
    }

    pub(crate) fn take_pending_stmts(&mut self) -> Vec<StmtId> {
        std::mem::take(&mut self.pending_stmts)
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
