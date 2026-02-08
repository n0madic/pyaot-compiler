use super::AstToHir;
use indexmap::IndexSet;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::*;
use pyaot_types::{exception_name_to_tag, Type};
use pyaot_utils::{ClassId, FuncId, InternedString, Span};
use rustpython_parser::ast as py;

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
    pub(crate) fn alloc_class_id(&mut self) -> ClassId {
        let id = ClassId::new(self.next_class_id);
        self.next_class_id += 1;
        id
    }

    pub(crate) fn convert_class_def(&mut self, class_def: py::StmtClassDef) -> Result<()> {
        let class_span = Self::span_from(&class_def);
        let class_id = self.alloc_class_id();
        let class_name = self.interner.intern(&class_def.name);

        // Register class in class_map
        self.class_map.insert(class_name, class_id);

        // Parse base class from bases (single inheritance only)
        // Also detect if this is an exception class
        let mut is_exception_class = false;
        let mut base_exception_type: Option<u8> = None;

        let base_class = if !class_def.bases.is_empty() {
            let first_base = &class_def.bases[0];
            if let py::Expr::Name(name) = first_base {
                let base_name_str = name.id.as_str();

                // Check if inheriting from a built-in exception type
                if let Some(exc_tag) = exception_name_to_tag(base_name_str) {
                    is_exception_class = true;
                    base_exception_type = Some(exc_tag);
                    // Built-in exceptions don't have ClassId - no base_class for them
                    None
                } else {
                    let base_name = self.interner.intern(&name.id);
                    if let Some(&base_id) = self.class_map.get(&base_name) {
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
        let prev_class = self.current_class;
        self.current_class = Some(class_id);

        // Parse class body: collect fields, class attributes, and methods
        let mut fields = Vec::new();
        let mut class_attrs = Vec::new();
        let mut methods = Vec::new();
        let mut init_method = None;

        // Track property getters and setters: property_name -> (getter_func_id, getter_type, getter_span)
        let mut property_getters: std::collections::HashMap<String, (FuncId, Type, Span)> =
            std::collections::HashMap::new();
        // Track property setters: property_name -> setter_func_id
        let mut property_setters: std::collections::HashMap<String, FuncId> =
            std::collections::HashMap::new();
        // Track abstract methods: names of methods marked with @abstractmethod
        let mut own_abstract_methods: IndexSet<InternedString> = IndexSet::new();
        // Track all method names in this class (for removing overrides from inherited abstract set)
        let mut defined_method_names: IndexSet<InternedString> = IndexSet::new();

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

                    let method_func_id = self.convert_method_def(
                        func_def,
                        class_id,
                        &class_def.name,
                        &parsed_decorators,
                    )?;

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
                        let method_var_id = self.alloc_var_id();
                        self.module_var_map.insert(var_name, method_var_id);

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
                            kind: StmtKind::Assign {
                                target: method_var_id,
                                value: current_expr,
                                type_hint: None,
                            },
                            span: method_span,
                        });
                        self.module.module_init_stmts.push(assign_stmt);

                        // Remove from func_map so method calls go through var_map
                        self.func_map.remove(&var_name);
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
        self.current_class = prev_class;

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
        };

        self.module.class_defs.insert(class_id, class_def);

        Ok(())
    }

    /// Helper function to detect dunder methods that should infer second param as self type
    fn should_infer_second_param_as_self(method_name: &str) -> bool {
        matches!(
            method_name,
            "__eq__"
                | "__ne__"
                | "__lt__"
                | "__le__"
                | "__gt__"
                | "__ge__"
                | "__add__"
                | "__sub__"
                | "__mul__"
                | "__truediv__"
                | "__floordiv__"
                | "__mod__"
                | "__pow__"
                | "__and__"
                | "__or__"
                | "__xor__"
                | "__radd__"
                | "__rsub__"
                | "__rmul__"
                | "__rtruediv__"
                | "__rfloordiv__"
                | "__rmod__"
                | "__rpow__"
                | "__rand__"
                | "__ror__"
                | "__rxor__"
        )
    }

    /// Convert a method definition with decorator handling
    fn convert_method_def(
        &mut self,
        func_def: py::StmtFunctionDef,
        _class_id: ClassId,
        class_name: &str,
        decorators: &ParsedDecorators,
    ) -> Result<FuncId> {
        let method_span = Self::span_from(&func_def);
        let func_id = self.alloc_func_id();
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
        self.func_map.insert(func_name, func_id);

        // Save outer var_map and create new scope
        let outer_var_map = std::mem::take(&mut self.var_map);
        let outer_is_generator = self.current_func_is_generator;
        self.current_func_is_generator = false;

        // Calculate default values mapping
        let num_params = func_def.args.args.len();
        let defaults: Vec<_> = func_def.args.defaults().collect();
        let num_defaults = defaults.len();
        let first_default_idx = num_params.saturating_sub(num_defaults);

        // Convert parameters with decorator-aware handling
        let mut params = Vec::new();
        for (i, arg) in func_def.args.args.iter().enumerate() {
            let param_name = self.interner.intern(&arg.def.arg);
            let param_id = self.alloc_var_id();
            self.var_map.insert(param_name, param_id);

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
                        // Regular instance method: 'self' gets the class type
                        if arg.def.arg.as_str() == "self" {
                            if let Some(current_class_id) = self.current_class {
                                let current_class_name = self
                                    .module
                                    .class_defs
                                    .get(&current_class_id)
                                    .map(|c| c.name)
                                    .unwrap_or(param_name);
                                Some(Type::Class {
                                    class_id: current_class_id,
                                    name: current_class_name,
                                })
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
                // Second parameter - NEW LOGIC for dunder method inference
                if let Some(annotation) = &arg.def.annotation {
                    // Explicit annotation takes precedence
                    Some(self.convert_type_annotation(annotation)?)
                } else if decorators.method_kind == MethodKind::Instance
                    && Self::should_infer_second_param_as_self(&original_method_name)
                {
                    // Infer as same type as self for dunder methods
                    if let Some(current_class_id) = self.current_class {
                        let current_class_name = self
                            .module
                            .class_defs
                            .get(&current_class_id)
                            .map(|c| c.name)
                            .unwrap_or(param_name);
                        Some(Type::Class {
                            class_id: current_class_id,
                            name: current_class_name,
                        })
                    } else {
                        None
                    }
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

        // Process *args parameter (vararg)
        if let Some(vararg_param) = &func_def.args.vararg {
            let vararg_name = self.interner.intern(&vararg_param.arg);
            let vararg_id = self.alloc_var_id();
            self.var_map.insert(vararg_name, vararg_id);
            self.initialized_vars.insert(vararg_name);

            // Type annotation: *args: int → tuple[int, ...]
            let vararg_type = if let Some(annotation) = &vararg_param.annotation {
                let element_type = self.convert_type_annotation(annotation)?;
                Some(Type::Tuple(vec![element_type]))
            } else {
                Some(Type::Tuple(vec![Type::Any])) // Default: tuple[Any, ...]
            };

            params.push(Param {
                name: vararg_name,
                var: vararg_id,
                ty: vararg_type,
                default: None,
                kind: ParamKind::VarPositional,
                span: method_span,
            });
        }

        // Process keyword-only parameters (kwonlyargs - parameters after *args)
        for kwonly_arg in func_def.args.kwonlyargs.iter() {
            let param_name = self.interner.intern(&kwonly_arg.def.arg);
            let param_id = self.alloc_var_id();
            self.var_map.insert(param_name, param_id);
            self.initialized_vars.insert(param_name);

            let param_type = if let Some(annotation) = &kwonly_arg.def.annotation {
                Some(self.convert_type_annotation(annotation)?)
            } else {
                None
            };

            // Extract default value from AST (if present)
            let default = if let Some(default_expr) = &kwonly_arg.default {
                Some(self.convert_expr((**default_expr).clone())?)
            } else {
                None
            };

            params.push(Param {
                name: param_name,
                var: param_id,
                ty: param_type,
                default,
                kind: ParamKind::KeywordOnly,
                span: method_span,
            });
        }

        // Process **kwargs parameter (kwarg)
        if let Some(kwarg_param) = &func_def.args.kwarg {
            let kwarg_name = self.interner.intern(&kwarg_param.arg);
            let kwarg_id = self.alloc_var_id();
            self.var_map.insert(kwarg_name, kwarg_id);
            self.initialized_vars.insert(kwarg_name);

            // Type annotation: **kwargs: int → dict[str, int]
            let kwarg_type = if let Some(annotation) = &kwarg_param.annotation {
                let value_type = self.convert_type_annotation(annotation)?;
                Some(Type::Dict(Box::new(Type::Str), Box::new(value_type)))
            } else {
                Some(Type::Dict(Box::new(Type::Str), Box::new(Type::Any)))
            };

            params.push(Param {
                name: kwarg_name,
                var: kwarg_id,
                ty: kwarg_type,
                default: None,
                kind: ParamKind::VarKeyword,
                span: method_span,
            });
        }

        // Convert return type
        let return_type = if let Some(ret_ann) = &func_def.returns {
            Some(self.convert_type_annotation(ret_ann)?)
        } else {
            Some(Type::None)
        };

        // Convert function body
        let mut body_stmts = Vec::new();
        for stmt in func_def.body {
            let stmt_id = self.convert_stmt(stmt)?;
            // Inject any pending statements from comprehensions before this statement
            let pending = self.take_pending_stmts();
            body_stmts.extend(pending);
            body_stmts.push(stmt_id);
        }

        let method_is_generator = self.current_func_is_generator;

        let function = Function {
            id: func_id,
            name: func_name,
            params,
            return_type,
            body: body_stmts,
            span: method_span,
            cell_vars: std::collections::HashSet::new(),
            nonlocal_vars: std::collections::HashSet::new(),
            is_generator: method_is_generator,
            method_kind: decorators.method_kind,
            is_abstract: decorators.is_abstract,
        };

        self.module.functions.push(func_id);
        self.module.func_defs.insert(func_id, function);

        // Restore outer scope
        self.var_map = outer_var_map;
        self.current_func_is_generator = outer_is_generator;

        Ok(func_id)
    }
}
