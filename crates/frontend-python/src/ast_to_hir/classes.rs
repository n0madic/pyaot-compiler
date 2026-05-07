use super::AstToHir;
use indexmap::{IndexMap, IndexSet};
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{
    cfg_builder::{CfgBuilder, CfgStmt},
    *,
};
use pyaot_types::dunders::{dunder_kind, polymorphic_other_type};
use pyaot_types::{exception_name_to_tag, Type, TypeLattice};
use pyaot_utils::{FuncId, InternedString, Span};
use rustpython_parser::ast as py;
use std::collections::HashSet;

/// Result of parsing method decorators
#[derive(Debug, Clone, Default)]
struct ParsedDecorators {
    /// Method kind (Instance, Static, ClassMethod)
    method_kind: MethodKind,
    /// True if this method is a property getter (@property)
    is_property_getter: bool,
    /// If this is a property setter, the property name it's setting
    property_setter_for: Option<String>,
    /// True if this method is marked with @abstractmethod
    is_abstract: bool,
    /// User-defined decorators (stored for later application)
    user_decorators: Vec<py::Expr>,
}

impl AstToHir {
    /// Parse decorators from a method's decorator_list
    fn parse_method_decorators(
        &self,
        decorator_list: &[py::Expr],
        method_span: Span,
    ) -> Result<ParsedDecorators> {
        let mut result = ParsedDecorators::default();

        for decorator in decorator_list {
            match decorator {
                py::Expr::Name(name) => match name.id.as_str() {
                    "staticmethod" => {
                        result.method_kind = MethodKind::Static;
                    }
                    "classmethod" => {
                        result.method_kind = MethodKind::ClassMethod;
                    }
                    "property" => {
                        result.is_property_getter = true;
                    }
                    "abstractmethod" => {
                        result.is_abstract = true;
                    }
                    _ => {
                        // User-defined decorator - store for later application
                        result.user_decorators.push(decorator.clone());
                    }
                },
                py::Expr::Attribute(attr) => {
                    // Handle @property_name.setter syntax
                    if attr.attr.as_str() == "setter" {
                        if let py::Expr::Name(name) = &*attr.value {
                            result.property_setter_for = Some(name.id.to_string());
                        } else {
                            return Err(CompilerError::parse_error(
                                "Invalid property setter decorator syntax",
                                method_span,
                            ));
                        }
                    } else {
                        // User-defined attribute decorator (e.g., @module.decorator)
                        result.user_decorators.push(decorator.clone());
                    }
                }
                py::Expr::Call(_) => {
                    // Handle decorator calls like @decorator(args)
                    // Store as user-defined decorator for later application
                    result.user_decorators.push(decorator.clone());
                }
                _ => {
                    return Err(CompilerError::parse_error(
                        "Unsupported decorator syntax",
                        method_span,
                    ));
                }
            }
        }

        Ok(result)
    }
    pub(crate) fn convert_class_def(&mut self, class_def: py::StmtClassDef) -> Result<()> {
        let class_span = Self::span_from(&class_def);
        let class_name = self.interner.intern(&class_def.name);

        // Use the pre-reserved `ClassId` from `prescan_top_level_classes` if
        // present AND not yet populated — this is the normal top-level class
        // path. If the class_map entry already maps to an id whose body has
        // been converted (e.g. test files that declare two classes with the
        // same name), allocate a fresh id and rebind `class_map` to match
        // pre-prescan behaviour: later declarations shadow earlier ones for
        // name-resolution, but earlier ClassRef captures still see their
        // original ClassId.
        let prescanned = self.symbols.class_map.get(&class_name).copied();
        let class_id = match prescanned {
            Some(id) if !self.module.class_defs.contains_key(&id) => id,
            _ => {
                let fresh = self.ids.alloc_class();
                self.symbols.class_map.insert(class_name, fresh);
                fresh
            }
        };

        // PEP 695: `class Stack[T]:` — register scoped TypeVars before processing
        // bases and body so `T` resolves in field/method annotations.
        let pep695_saved = self.push_pep695_type_params(&class_def.type_params, class_span)?;
        // Collect TypeVar names for this class (S3.3b.1): PEP-695 params first.
        let mut class_type_params: Vec<InternedString> =
            pep695_saved.iter().map(|(n, _)| *n).collect();

        // Parse base class from bases (single inheritance only)
        // Also detect if this is an exception class or Protocol
        let mut is_exception_class = false;
        let mut is_protocol = false;
        let mut base_exception_type: Option<u8> = None;

        // Pre-filter: strip compile-time-only Generic[T] / Protocol[T] bases.
        // These carry no runtime parent; they exist purely to register the class
        // as generic/structural in the type system. Each TypeVar arg is validated
        // against the current typevar_defs so typos fail loudly.
        let mut real_base_indices: Vec<usize> = Vec::new();
        for (idx, base_expr) in class_def.bases.iter().enumerate() {
            if let py::Expr::Subscript(sub) = base_expr {
                if let py::Expr::Name(sub_name) = &*sub.value {
                    let sub_name_str = sub_name.id.as_str();
                    if sub_name_str == "Generic" || sub_name_str == "Protocol" {
                        let sub_interned = self.interner.intern(sub_name_str);
                        if self.types.typing_imports.contains(&sub_interned) {
                            let tv_names =
                                self.extract_generic_type_params(&sub.slice, class_span)?;
                            for tv_name in &tv_names {
                                if !self.types.typevar_defs.contains_key(tv_name) {
                                    let name_str = self.interner.resolve(*tv_name);
                                    return Err(CompilerError::parse_error(
                                        format!("undefined TypeVar '{}' in Generic[...]", name_str),
                                        class_span,
                                    ));
                                }
                            }
                            if sub_name_str == "Protocol" {
                                is_protocol = true;
                            } else {
                                // Generic[T, U, ...]: add names not already registered
                                // (PEP-695 params may overlap) to class_type_params.
                                for &tv in &tv_names {
                                    if !class_type_params.contains(&tv) {
                                        class_type_params.push(tv);
                                    }
                                }
                            }
                            continue; // compile-time only; not a runtime base
                        }
                    }
                }
            }
            real_base_indices.push(idx);
        }

        let base_class = if !real_base_indices.is_empty() {
            let first_base = &class_def.bases[real_base_indices[0]];
            if let py::Expr::Name(name) = first_base {
                let base_name_str = name.id.as_str();

                // Check if this is a Protocol class (from typing import Protocol)
                if base_name_str == "Protocol" {
                    let proto_interned = self.interner.intern("Protocol");
                    if self.types.typing_imports.contains(&proto_interned) {
                        is_protocol = true;
                        None // Protocol has no runtime parent
                    } else {
                        return Err(CompilerError::name_error(
                            format!(
                                "Base class '{}' must be defined before '{}'",
                                name.id, class_def.name
                            ),
                            class_span,
                        ));
                    }
                }
                // Check if inheriting from a built-in exception type
                else if let Some(exc_tag) = exception_name_to_tag(base_name_str) {
                    is_exception_class = true;
                    base_exception_type = Some(exc_tag);
                    // Built-in exceptions don't have ClassId - no base_class for them
                    None
                } else {
                    let base_name = self.interner.intern(&name.id);
                    if let Some(&base_id) = self.symbols.class_map.get(&base_name) {
                        // Check if parent is an exception class
                        if let Some(parent_def) = self.module.class_defs.get(&base_id) {
                            if parent_def.is_exception_class {
                                is_exception_class = true;
                                // Inherit the base exception type from parent
                                base_exception_type = parent_def.base_exception_type;
                            }
                        }
                        Some(base_id)
                    } else {
                        return Err(CompilerError::name_error(
                            format!(
                                "Base class '{}' must be defined before '{}'",
                                name.id, class_def.name
                            ),
                            class_span,
                        ));
                    }
                }
            } else {
                return Err(CompilerError::parse_error(
                    "Base class must be a simple name",
                    class_span,
                ));
            }
        } else {
            None
        };

        // Save current class context
        let prev_class = self.scope.current_class;
        let prev_class_name = self.scope.current_class_name;
        let prev_class_type_params = std::mem::replace(
            &mut self.scope.current_class_type_params,
            class_type_params.clone(),
        );
        self.scope.current_class = Some(class_id);
        self.scope.current_class_name = Some(class_name);

        // Parse class body: collect fields, class attributes, and methods
        let mut fields = Vec::new();
        let mut class_attrs = Vec::new();
        let mut methods = Vec::new();
        let mut init_method = None;

        // Track property getters and setters: property_name -> (getter_func_id, getter_type, getter_span)
        // Uses IndexMap for deterministic property ordering in ClassDef
        let mut property_getters: IndexMap<String, (FuncId, Type, Span)> = IndexMap::new();
        // Track property setters: property_name -> setter_func_id
        let mut property_setters: IndexMap<String, FuncId> = IndexMap::new();
        // Track abstract methods: names of methods marked with @abstractmethod
        let mut own_abstract_methods: IndexSet<InternedString> = IndexSet::new();
        // Track all method names in this class (for removing overrides from inherited abstract set)
        let mut defined_method_names: IndexSet<InternedString> = IndexSet::new();

        // Pre-scan the entire class body to discover all instance field names
        // before we start the per-method type harvester. The harvester needs
        // this set so it can also recognise cross-instance writes — patterns
        // like `child.grad += rhs` or `node.next = other` mutate a field on
        // a different instance of the same class, but historically the scan
        // only walked `self.X = ...` and so missed those write sites entirely.
        // The result was unannotated numeric fields silently typed `Int` from
        // a constructor literal even though autograd-style methods stored
        // `Float` results into them through `obj.grad += ...`, corrupting the
        // tagged-Value slot bits.
        //
        // Source of field names, in order of authority:
        //   1. Class-level annotations: `x: int` (instance field declaration)
        //   2. `__slots__ = (...)` literal tuple of names
        //   3. `self.X = ...` writes inside any method
        let class_field_names = self.collect_class_field_names(&class_def.body);

        for stmt in class_def.body {
            match stmt {
                py::Stmt::AnnAssign(ann_assign) => {
                    if let py::Expr::Name(name) = &*ann_assign.target {
                        let attr_name = self.interner.intern(&name.id);
                        let attr_type = self.convert_type_annotation(&ann_assign.annotation)?;

                        if let Some(value) = &ann_assign.value {
                            // Class attribute with value: x: int = 0
                            let initializer = self.convert_expr(*value.clone())?;
                            class_attrs.push(ClassAttribute {
                                name: attr_name,
                                ty: attr_type,
                                initializer,
                                span: class_span,
                            });
                        } else {
                            // Instance field declaration: x: int
                            fields.push(FieldDef {
                                name: attr_name,
                                ty: attr_type,
                                span: class_span,
                            });
                        }
                    }
                }
                py::Stmt::Assign(assign) => {
                    // Skip __slots__ assignments (CPython memory optimization, not needed in AOT)
                    if assign.targets.len() == 1 {
                        if let py::Expr::Name(name) = &assign.targets[0] {
                            if name.id.as_str() == "__slots__" {
                                continue;
                            }
                        }
                    }
                    // Class attribute definition: x = value
                    if assign.targets.len() == 1 {
                        if let py::Expr::Name(name) = &assign.targets[0] {
                            let attr_name = self.interner.intern(&name.id);
                            let initializer = self.convert_expr(*assign.value.clone())?;
                            let attr_type =
                                self.infer_literal_type(&self.module.exprs[initializer]);
                            class_attrs.push(ClassAttribute {
                                name: attr_name,
                                ty: attr_type,
                                initializer,
                                span: class_span,
                            });
                        }
                    }
                }
                py::Stmt::FunctionDef(func_def) => {
                    // Method definition - parse decorators first
                    let method_span = Self::span_from(&func_def);
                    let parsed_decorators =
                        self.parse_method_decorators(&func_def.decorator_list, method_span)?;

                    let method_name = func_def.name.to_string();
                    let is_init = method_name == "__init__";

                    // Get return type for properties (before converting the method)
                    let return_type = if let Some(ret_ann) = &func_def.returns {
                        self.convert_type_annotation(ret_ann)?
                    } else {
                        Type::None
                    };

                    // Scan EVERY method body for `self.field = value` assignments
                    // (Area D §D.3.6). Tuples of different shapes in different
                    // methods unify via `join`, so a field that receives `()`,
                    // `(a,)`, and `(a, b)` across methods infers as `TupleVar(T)`
                    // — not `Any`.
                    //
                    // Fields are introduced in source order — the first method
                    // to write a field establishes its layout offset; subsequent
                    // methods only widen the type. A class without `__init__`
                    // (e.g. fields only set in `reset()` / `configure()`) still
                    // gets its fields discovered via whichever method writes them
                    // first.
                    //
                    // The `class_field_names` set was pre-computed from a sweep
                    // over the entire class body. Passing it lets the per-method
                    // scan also capture cross-instance writes like
                    // `child.grad += rhs` (autograd-style) — those would
                    // otherwise be invisible because the harvester historically
                    // only saw `self.X` writes. Without it, an unannotated field
                    // refined as `Int` from `__init__` would silently store
                    // `Float` values via a different method, producing tagged-
                    // Value bits that read back as garbage pointers (the SEGV
                    // in `Value.backward()` on autograd-style code).
                    let observed = self.scan_method_for_self_fields(
                        &func_def.body,
                        &func_def.args,
                        &class_field_names,
                    );
                    for (name_str, inferred_ty) in observed {
                        // Skip fields already declared at class level.
                        let name_interned = self.interner.intern(&name_str);
                        if class_attrs.iter().any(|a| a.name == name_interned) {
                            continue;
                        }
                        if let Some(existing) = fields.iter_mut().find(|f| f.name == name_interned)
                        {
                            // Don't dilute a precise existing type with `Any`
                            // from an un-inferrable RHS (e.g. `self.x = self.x + 1`
                            // where the RHS BinOp infers to Any in our pre-lowering
                            // scan). Tuple-shape merge only kicks in when the new
                            // observation is itself a meaningful type.
                            if !matches!(inferred_ty, Type::Any) {
                                existing.ty = existing.ty.join(&inferred_ty);
                            }
                        } else {
                            // First method to reference this field introduces it.
                            fields.push(FieldDef {
                                name: name_interned,
                                ty: inferred_ty,
                                span: class_span,
                            });
                        }
                    }

                    let method_func_id =
                        self.convert_method_def(func_def, &class_def.name, &parsed_decorators)?;

                    if is_init {
                        init_method = Some(method_func_id);
                    }

                    // Apply user-defined decorators to the method
                    // User decorators are applied after built-in decorators (staticmethod, classmethod, property)
                    if !parsed_decorators.user_decorators.is_empty() {
                        // Create a mangled name for the decorated method variable
                        let mangled_name = if parsed_decorators.property_setter_for.is_some() {
                            format!("{}${}$setter", class_def.name, method_name)
                        } else {
                            format!("{}${}", class_def.name, method_name)
                        };
                        let var_name = self.interner.intern(&mangled_name);

                        // Create variable for decorated method
                        let method_var_id = self.ids.alloc_var();
                        self.symbols.module_var_map.insert(var_name, method_var_id);

                        // Start with FuncRef to the original method
                        let mut current_expr = self.module.exprs.alloc(Expr {
                            kind: ExprKind::FuncRef(method_func_id),
                            ty: None,
                            span: method_span,
                        });

                        // Apply decorators bottom-up (last decorator applied first)
                        for decorator in parsed_decorators.user_decorators.iter().rev() {
                            current_expr =
                                self.apply_decorator(decorator, current_expr, method_span)?;
                        }

                        // Create assignment: method_var = decorated_result
                        let assign_stmt = self.module.stmts.alloc(Stmt {
                            kind: StmtKind::Bind {
                                target: BindingTarget::Var(method_var_id),
                                value: current_expr,
                                type_hint: None,
                            },
                            span: method_span,
                        });
                        self.module_init_stmts.push(CfgStmt::stmt(assign_stmt));

                        // Remove from func_map so method calls go through var_map
                        self.symbols.func_map.remove(&var_name);
                    }

                    // Track property getters/setters
                    if parsed_decorators.is_property_getter {
                        property_getters.insert(
                            method_name.clone(),
                            (method_func_id, return_type, method_span),
                        );
                    }
                    if let Some(prop_name) = &parsed_decorators.property_setter_for {
                        property_setters.insert(prop_name.clone(), method_func_id);
                    }

                    // Track abstract methods and defined method names
                    let method_name_interned = self.interner.intern(&method_name);
                    defined_method_names.insert(method_name_interned);
                    if parsed_decorators.is_abstract {
                        own_abstract_methods.insert(method_name_interned);
                    }

                    // Only add to methods list if not a property getter/setter
                    // Properties are handled separately
                    if !parsed_decorators.is_property_getter
                        && parsed_decorators.property_setter_for.is_none()
                    {
                        methods.push(method_func_id);
                    }
                }
                py::Stmt::Pass(_) => {
                    // Ignore pass statements in class body
                }
                _ => {
                    return Err(CompilerError::parse_error(
                        "Only field annotations, class attributes, and method definitions supported in class body",
                        class_span,
                    ));
                }
            }
        }

        // Restore class context
        self.scope.current_class = prev_class;
        self.scope.current_class_name = prev_class_name;
        self.scope.current_class_type_params = prev_class_type_params;

        // Build PropertyDef structures from collected getters/setters
        let mut properties = Vec::new();
        for (prop_name, (getter_id, prop_ty, prop_span)) in property_getters {
            let setter_id = property_setters.get(&prop_name).copied();
            let prop_name_interned = self.interner.intern(&prop_name);
            properties.push(PropertyDef {
                name: prop_name_interned,
                getter: getter_id,
                setter: setter_id,
                ty: prop_ty,
                span: prop_span,
            });
        }

        // Compute abstract methods:
        // 1. Start with inherited abstract methods from parent (if any)
        // 2. Remove methods that are overridden in this class
        // 3. Add this class's own abstract methods
        let mut abstract_methods = if let Some(base_id) = base_class {
            // Get inherited abstract methods from parent
            if let Some(parent_def) = self.module.class_defs.get(&base_id) {
                parent_def.abstract_methods.clone()
            } else {
                IndexSet::new()
            }
        } else {
            IndexSet::new()
        };

        // Remove overridden methods (any method defined in this class overrides parent's abstract method)
        for method_name in &defined_method_names {
            abstract_methods.swap_remove(method_name);
        }

        // Add this class's own abstract methods
        for abstract_method in own_abstract_methods {
            abstract_methods.insert(abstract_method);
        }

        // Create and register the class definition
        let class_def = ClassDef {
            id: class_id,
            name: class_name,
            base_class,
            fields,
            class_attrs,
            methods,
            init_method,
            properties,
            abstract_methods,
            span: class_span,
            is_exception_class,
            base_exception_type,
            is_protocol,
            type_params: class_type_params,
        };

        self.module.class_defs.insert(class_id, class_def);

        // Restore TypeVars that were scoped to this PEP 695 class
        self.pop_pep695_type_params(pep695_saved);

        Ok(())
    }

    /// Convert a method definition with decorator handling
    fn convert_method_def(
        &mut self,
        func_def: py::StmtFunctionDef,
        class_name: &str,
        decorators: &ParsedDecorators,
    ) -> Result<FuncId> {
        let method_span = Self::span_from(&func_def);
        let func_id = self.ids.alloc_func();
        // Store the original method name for dunder method detection
        let original_method_name = func_def.name.to_string();

        // Mangle method name with class name to avoid collisions
        // e.g., Point.__init__ becomes Point$__init__
        // For property setters, append $setter to distinguish from getters
        let mangled_name = if decorators.property_setter_for.is_some() {
            format!("{}${}$setter", class_name, func_def.name)
        } else {
            format!("{}${}", class_name, func_def.name)
        };
        let func_name = self.interner.intern(&mangled_name);

        // Register function in func_map with the original method name for lookups
        // Note: Method calls use the class's method_funcs map, not func_map
        self.symbols.func_map.insert(func_name, func_id);

        // Save outer var_map and create new scope
        let outer_var_map = std::mem::take(&mut self.symbols.var_map);
        let outer_is_generator = self.scope.current_func_is_generator;
        self.scope.current_func_is_generator = false;

        // Calculate default values mapping
        let num_params = func_def.args.args.len();
        let defaults: Vec<_> = func_def.args.defaults().collect();
        let num_defaults = defaults.len();
        let first_default_idx = num_params.saturating_sub(num_defaults);

        // Convert parameters with decorator-aware handling
        let mut params = Vec::new();
        for (i, arg) in func_def.args.args.iter().enumerate() {
            let param_name = self.interner.intern(&arg.def.arg);
            let param_id = self.ids.alloc_var();
            self.symbols.var_map.insert(param_name, param_id);

            // Determine parameter type based on decorator and parameter name
            let param_type = if i == 0 {
                // First parameter (self/cls) - existing logic
                match decorators.method_kind {
                    MethodKind::Static => {
                        // @staticmethod: no special handling for first param
                        if let Some(annotation) = &arg.def.annotation {
                            Some(self.convert_type_annotation(annotation)?)
                        } else {
                            None
                        }
                    }
                    MethodKind::ClassMethod => {
                        // @classmethod: first param 'cls' represents the class type
                        if arg.def.arg.as_str() == "cls" || arg.def.arg.as_str() == "self" {
                            // Use Type::Int to represent class_id for now
                            // (The runtime will pass the class_id as an integer)
                            Some(Type::Int)
                        } else if let Some(annotation) = &arg.def.annotation {
                            Some(self.convert_type_annotation(annotation)?)
                        } else {
                            None
                        }
                    }
                    MethodKind::Instance => {
                        // Regular instance method: 'self' gets the class type.
                        // For generic classes (type_params non-empty), pre-stamp
                        // self as Type::Generic{class_id, [Var(T)..]} so the method
                        // becomes a generic template (S3.3b.1).
                        if arg.def.arg.as_str() == "self" {
                            if let Some(current_class_id) = self.scope.current_class {
                                if !self.scope.current_class_type_params.is_empty() {
                                    let args = self
                                        .scope
                                        .current_class_type_params
                                        .iter()
                                        .map(|&n| Type::Var(n))
                                        .collect();
                                    Some(Type::Generic {
                                        base: current_class_id,
                                        args,
                                    })
                                } else {
                                    // Use scope.current_class_name (set alongside current_class)
                                    // rather than reading from class_defs, which is not populated
                                    // until after the class body is walked — reading it here would
                                    // fall back to param_name and produce drifted Class type names
                                    // that break Union deduplication later.
                                    let current_class_name =
                                        self.scope.current_class_name.unwrap_or(param_name);
                                    Some(Type::Class {
                                        class_id: current_class_id,
                                        name: current_class_name,
                                    })
                                }
                            } else {
                                None
                            }
                        } else if let Some(annotation) = &arg.def.annotation {
                            Some(self.convert_type_annotation(annotation)?)
                        } else {
                            None
                        }
                    }
                }
            } else if i == 1 {
                // Second parameter — polymorphic `other` for operator dunders.
                // Per CPython Data Model §3.3.8, the `other` parameter of an
                // operator dunder is NOT constrained to `Self` — the dunder
                // must inspect it at runtime and either handle it or return
                // `NotImplemented`. The union expresses exactly that: the
                // caller may legitimately pass any member of the numeric
                // tower (for binary numeric/bitwise dunders) or anything at
                // all (for comparison dunders).
                if let Some(annotation) = &arg.def.annotation {
                    // Explicit annotation always wins.
                    Some(self.convert_type_annotation(annotation)?)
                } else if decorators.method_kind == MethodKind::Instance {
                    dunder_kind(&original_method_name).and_then(|kind| {
                        self.scope.current_class.and_then(|cid| {
                            let name = self.scope.current_class_name.unwrap_or(param_name);
                            polymorphic_other_type(
                                kind,
                                &Type::Class {
                                    class_id: cid,
                                    name,
                                },
                            )
                        })
                    })
                } else {
                    None
                }
            } else {
                // Third+ parameters - existing logic
                if let Some(annotation) = &arg.def.annotation {
                    Some(self.convert_type_annotation(annotation)?)
                } else {
                    None
                }
            };

            // Get default value if this parameter has one
            let default = if i >= first_default_idx {
                let default_idx = i - first_default_idx;
                Some(self.convert_expr((*defaults[default_idx]).clone())?)
            } else {
                None
            };

            params.push(Param {
                name: param_name,
                var: param_id,
                ty: param_type,
                default,
                kind: ParamKind::Regular,
                span: method_span,
            });
        }

        // Process *args, keyword-only, and **kwargs parameters
        params.extend(self.convert_extra_params(&func_def.args, method_span)?);

        // Convert return type. An unannotated method has no known return
        // type; record it as `None` so the type-planning pass can infer it
        // from the body (matching the regular-function convention at
        // `functions.rs`). Using `Some(Type::None)` here would short-circuit
        // Pass-2 inference and lock every unannotated method to a None
        // result — breaking any method that actually returns a value.
        let return_type = if let Some(ret_ann) = &func_def.returns {
            Some(self.convert_type_annotation(ret_ann)?)
        } else {
            None
        };

        // Convert function body
        let mut body_stmts = Vec::new();
        for stmt in func_def.body {
            let stmt = self.convert_stmt(stmt)?;
            // Inject any pending statements from comprehensions before this statement
            let pending = self.take_pending_stmts();
            body_stmts.extend(pending);
            body_stmts.push(stmt);
        }

        let method_is_generator = self.scope.current_func_is_generator;

        let mut cfg = CfgBuilder::new();
        let entry_block = cfg.new_block();
        cfg.enter(entry_block);
        cfg.lower_cfg_stmts(&body_stmts, &mut self.module);
        cfg.terminate_if_open(HirTerminator::Return(None));
        let (blocks, entry_block, try_scopes) = cfg.finish(entry_block);
        let function = Function {
            id: func_id,
            name: func_name,
            params,
            return_type,
            span: method_span,
            cell_vars: HashSet::new(),
            nonlocal_vars: HashSet::new(),
            is_generator: method_is_generator,
            method_kind: decorators.method_kind,
            is_abstract: decorators.is_abstract,
            blocks,
            entry_block,
            try_scopes,
        };

        self.module.functions.push(func_id);
        self.module.func_defs.insert(func_id, function);

        // Restore outer scope
        self.symbols.var_map = outer_var_map;
        self.scope.current_func_is_generator = outer_is_generator;

        Ok(func_id)
    }

    /// Pre-scan the entire class body to enumerate every instance field name
    /// before any per-method type harvesting runs. Three sources contribute:
    ///
    ///   1. Class-level `AnnAssign` without value (`x: int`) — explicit
    ///      instance field declarations.
    ///   2. `__slots__ = (...)` literal tuple of strings — CPython's memory
    ///      optimization, conveniently the same set of names.
    ///   3. `self.X = ...`, `self.X += ...`, `self.X: T = ...` writes inside
    ///      any method body (recursively into control flow blocks).
    ///
    /// The resulting set drives the cross-instance write detection in
    /// `scan_stmts_for_self_fields`: `<expr>.X = rhs` patterns where `expr`
    /// is **not** `self` but `X` is a known field still contribute their RHS
    /// type to the field, so a method that mutates a sibling instance's
    /// fields (autograd-style `child.grad += rhs`, linked-list `node.next =
    /// other`) widens the harvested type as it should.
    fn collect_class_field_names(&self, body: &[py::Stmt]) -> HashSet<String> {
        let mut names = HashSet::new();
        for stmt in body {
            match stmt {
                py::Stmt::AnnAssign(ann) => {
                    // Instance field declaration: `x: int` (no value).
                    if ann.value.is_none() {
                        if let py::Expr::Name(name) = &*ann.target {
                            names.insert(name.id.to_string());
                        }
                    }
                }
                py::Stmt::Assign(assign) => {
                    // `__slots__ = ('a', 'b', ...)` — extract literal names.
                    if assign.targets.len() == 1 {
                        if let py::Expr::Name(name) = &assign.targets[0] {
                            if name.id.as_str() == "__slots__" {
                                if let py::Expr::Tuple(tup) = &*assign.value {
                                    for elt in &tup.elts {
                                        if let py::Expr::Constant(c) = elt {
                                            if let py::Constant::Str(s) = &c.value {
                                                names.insert(s.clone());
                                            }
                                        }
                                    }
                                } else if let py::Expr::List(lst) = &*assign.value {
                                    for elt in &lst.elts {
                                        if let py::Expr::Constant(c) = elt {
                                            if let py::Constant::Str(s) = &c.value {
                                                names.insert(s.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                py::Stmt::FunctionDef(func_def) => {
                    Self::collect_self_field_names_in_stmts(&func_def.body, &mut names);
                }
                _ => {}
            }
        }
        names
    }

    /// Recursive helper: walk method body collecting `self.X = ...` field
    /// names. Pure name discovery — no type inference here, that happens in
    /// `scan_stmts_for_self_fields` after we know the full field set.
    fn collect_self_field_names_in_stmts(stmts: &[py::Stmt], out: &mut HashSet<String>) {
        for stmt in stmts {
            match stmt {
                py::Stmt::Assign(assign) => {
                    if assign.targets.len() == 1 {
                        if let py::Expr::Attribute(attr) = &assign.targets[0] {
                            if let py::Expr::Name(name) = &*attr.value {
                                if name.id.as_str() == "self" {
                                    out.insert(attr.attr.to_string());
                                }
                            }
                        }
                    }
                }
                py::Stmt::AugAssign(aug) => {
                    if let py::Expr::Attribute(attr) = &*aug.target {
                        if let py::Expr::Name(name) = &*attr.value {
                            if name.id.as_str() == "self" {
                                out.insert(attr.attr.to_string());
                            }
                        }
                    }
                }
                py::Stmt::AnnAssign(ann) => {
                    if let py::Expr::Attribute(attr) = &*ann.target {
                        if let py::Expr::Name(name) = &*attr.value {
                            if name.id.as_str() == "self" {
                                out.insert(attr.attr.to_string());
                            }
                        }
                    }
                }
                py::Stmt::If(if_stmt) => {
                    Self::collect_self_field_names_in_stmts(&if_stmt.body, out);
                    Self::collect_self_field_names_in_stmts(&if_stmt.orelse, out);
                }
                py::Stmt::For(for_stmt) => {
                    Self::collect_self_field_names_in_stmts(&for_stmt.body, out);
                }
                py::Stmt::While(while_stmt) => {
                    Self::collect_self_field_names_in_stmts(&while_stmt.body, out);
                }
                py::Stmt::Try(try_stmt) => {
                    Self::collect_self_field_names_in_stmts(&try_stmt.body, out);
                    for handler in &try_stmt.handlers {
                        let py::ExceptHandler::ExceptHandler(h) = handler;
                        Self::collect_self_field_names_in_stmts(&h.body, out);
                    }
                }
                _ => {}
            }
        }
    }

    /// Scan a method's body for `self.field = value` and `self.field: T = v`
    /// assignments, collecting inferred types per field name. Multiple writes
    /// within the same method unify via `join`.
    ///
    /// `known_fields` enumerates every instance field name discovered across
    /// the whole class body (see `collect_class_field_names`). It enables a
    /// secondary pattern: `<other>.X = rhs` where `X` is a known field still
    /// counts as a write, so cross-instance mutations widen the field type.
    ///
    /// Returns an IndexMap preserving first-seen order for stable codegen.
    fn scan_method_for_self_fields(
        &mut self,
        body: &[py::Stmt],
        args: &py::Arguments,
        known_fields: &HashSet<String>,
    ) -> IndexMap<String, Type> {
        // Build param name → inferred type. Annotation wins; default value
        // provides a fallback (so `children=()` infers as Tuple([]) without
        // requiring an explicit annotation). This is what makes `self._x = p`
        // reach a concrete tuple shape for unification across methods.
        let mut param_types: std::collections::HashMap<String, Type> =
            std::collections::HashMap::new();

        // Defaults in rustpython-parser align to the tail of positional args.
        // `defaults()` returns an iterator over the trailing defaults.
        let defaults: Vec<&py::Expr> = args.defaults().collect();
        let n_args = args.args.len();
        let n_defaults = defaults.len();
        let first_default_idx = n_args.saturating_sub(n_defaults);
        for (i, arg) in args.args.iter().enumerate() {
            let name = arg.def.arg.to_string();
            if let Some(ann) = arg.def.annotation.as_ref() {
                if let Ok(ty) = self.convert_type_annotation(ann) {
                    param_types.insert(name, ty);
                    continue;
                }
            }
            if i >= first_default_idx {
                let default_expr = defaults[i - first_default_idx];
                let snapshot = param_types.clone();
                let default_ty = self.infer_field_type_from_rhs(default_expr, &snapshot);
                if !matches!(default_ty, Type::Any) {
                    param_types.insert(name, default_ty);
                }
            }
        }

        let mut out: IndexMap<String, Type> = IndexMap::new();
        self.scan_stmts_for_self_fields(body, &param_types, known_fields, &mut out);
        out
    }

    /// Recursively scan statements for instance-field write patterns.
    ///
    /// Two write categories are recognised:
    ///
    ///   1. `self.X = rhs` (and `+=` / annotated forms) — primary writes.
    ///   2. `<expr>.X = rhs` where `expr` is **not** `self` and `X` is in
    ///      `known_fields` — cross-instance writes, e.g.
    ///      `child.grad += local_grad * v.grad` in autograd code or
    ///      `node.next = other` in linked-list code. Without this, methods
    ///      that mutate sibling-instance fields are invisible to type
    ///      inference and a numeric field gets harvested as `Int` from
    ///      a constructor literal even though peer methods write `Float`.
    ///
    /// Types are merged across writes via `join`, which preserves tuple-shape
    /// information — a field assigned tuples of different lengths in different
    /// branches infers as `TupleVar` instead of `Any`.
    fn scan_stmts_for_self_fields(
        &mut self,
        stmts: &[py::Stmt],
        param_types: &std::collections::HashMap<String, Type>,
        known_fields: &HashSet<String>,
        out: &mut IndexMap<String, Type>,
    ) {
        for stmt in stmts {
            match stmt {
                py::Stmt::Assign(assign) => {
                    if assign.targets.len() == 1 {
                        if let py::Expr::Attribute(attr) = &assign.targets[0] {
                            let field_name = attr.attr.to_string();
                            let is_self = matches!(
                                &*attr.value,
                                py::Expr::Name(n) if n.id.as_str() == "self"
                            );
                            // Self-write: always recorded. Non-self write
                            // counts only if the attribute is a known field
                            // of this class — otherwise we'd pollute the
                            // field set with names that belong to some
                            // other class (e.g. `logger.info = ...` style).
                            if is_self || known_fields.contains(&field_name) {
                                let ty = self.infer_field_type_from_rhs(&assign.value, param_types);
                                // Cross-instance writes only contribute if the
                                // RHS produced something concrete — `Any`
                                // would dilute a precise type without adding
                                // information.
                                if is_self || !matches!(ty, Type::Any) {
                                    out.entry(field_name)
                                        .and_modify(|prev| *prev = prev.join(&ty))
                                        .or_insert(ty);
                                }
                            }
                        }
                    }
                }
                // `<obj>.f <op>= <rhs>` — same dual treatment as `=`. The
                // augmented form is the dominant pattern for accumulator
                // fields like autograd's `child.grad += ...`, so missing it
                // here would defeat the cross-instance widening entirely.
                py::Stmt::AugAssign(aug) => {
                    if let py::Expr::Attribute(attr) = &*aug.target {
                        let field_name = attr.attr.to_string();
                        let is_self = matches!(
                            &*attr.value,
                            py::Expr::Name(n) if n.id.as_str() == "self"
                        );
                        if is_self || known_fields.contains(&field_name) {
                            let rhs_ty = self.infer_field_type_from_rhs(&aug.value, param_types);
                            if !matches!(rhs_ty, Type::Any) {
                                out.entry(field_name)
                                    .and_modify(|prev| *prev = prev.join(&rhs_ty))
                                    .or_insert(rhs_ty);
                            }
                        }
                    }
                }
                py::Stmt::AnnAssign(ann) => {
                    if let py::Expr::Attribute(attr) = &*ann.target {
                        let field_name = attr.attr.to_string();
                        let is_self = matches!(
                            &*attr.value,
                            py::Expr::Name(n) if n.id.as_str() == "self"
                        );
                        if is_self || known_fields.contains(&field_name) {
                            let ty = self
                                .convert_type_annotation(&ann.annotation)
                                .unwrap_or(Type::Any);
                            if is_self {
                                // Explicit annotation on `self.X` wins over
                                // prior inference within the same method.
                                out.insert(field_name, ty);
                            } else if !matches!(ty, Type::Any) {
                                out.entry(field_name)
                                    .and_modify(|prev| *prev = prev.join(&ty))
                                    .or_insert(ty);
                            }
                        }
                    }
                }
                // Recurse into control-flow blocks to find conditional assignments.
                py::Stmt::If(if_stmt) => {
                    self.scan_stmts_for_self_fields(&if_stmt.body, param_types, known_fields, out);
                    self.scan_stmts_for_self_fields(
                        &if_stmt.orelse,
                        param_types,
                        known_fields,
                        out,
                    );
                }
                py::Stmt::For(for_stmt) => {
                    self.scan_stmts_for_self_fields(&for_stmt.body, param_types, known_fields, out);
                }
                py::Stmt::While(while_stmt) => {
                    self.scan_stmts_for_self_fields(
                        &while_stmt.body,
                        param_types,
                        known_fields,
                        out,
                    );
                }
                py::Stmt::Try(try_stmt) => {
                    self.scan_stmts_for_self_fields(&try_stmt.body, param_types, known_fields, out);
                    for handler in &try_stmt.handlers {
                        let py::ExceptHandler::ExceptHandler(h) = handler;
                        self.scan_stmts_for_self_fields(&h.body, param_types, known_fields, out);
                    }
                }
                _ => {}
            }
        }
    }

    /// Infer field type from the RHS of `self.field = value`.
    ///
    /// Covers:
    ///   (a) parameter reference with a type annotation → annotated type.
    ///   (b) tuple literal → `Type::Generic{BUILTIN_TUPLE_CLASS_ID, [shape]}`, element types inferred
    ///       recursively (enables `join` to see real shapes and produce
    ///       a TupleVar Generic for cross-method heterogeneity).
    ///   (c) primitive literal → matching primitive type.
    ///
    /// All other shapes fall back to `Type::Any`.
    fn infer_field_type_from_rhs(
        &mut self,
        rhs: &py::Expr,
        param_types: &std::collections::HashMap<String, Type>,
    ) -> Type {
        match rhs {
            py::Expr::Name(name) => param_types
                .get(name.id.as_str())
                .cloned()
                .unwrap_or(Type::Any),
            py::Expr::Tuple(tuple) => {
                let elem_tys: Vec<Type> = tuple
                    .elts
                    .iter()
                    .map(|e| self.infer_field_type_from_rhs(e, param_types))
                    .collect();
                Type::tuple_of(elem_tys)
            }
            py::Expr::Constant(c) => match &c.value {
                py::Constant::Int(_) => Type::Int,
                py::Constant::Float(_) => Type::Float,
                py::Constant::Bool(_) => Type::Bool,
                py::Constant::Str(_) => Type::Str,
                py::Constant::None => Type::None,
                _ => Type::Any,
            },
            // Narrow numeric-BinOp inference: common idiom `self.x = self.x + 0.5`.
            // Only attempts numeric-tower promotion on the two operand types;
            // non-numeric mixes bail out to Any (conservative).
            py::Expr::BinOp(bop) => {
                let lhs = self.infer_field_type_from_rhs(&bop.left, param_types);
                let rhs = self.infer_field_type_from_rhs(&bop.right, param_types);
                match (&lhs, &rhs) {
                    (
                        Type::Float | Type::Int | Type::Bool,
                        Type::Float | Type::Int | Type::Bool,
                    ) => lhs.join(&rhs),
                    _ => Type::Any,
                }
            }
            _ => Type::Any,
        }
    }
}
