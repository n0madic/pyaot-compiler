//! Type checking context and module entry point

use indexmap::IndexMap;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::Module;
use pyaot_types::Type;
use pyaot_utils::{ClassId, FuncId, InternedString, StringInterner, VarId};

/// Type checker for HIR
pub struct TypeChecker<'a> {
    pub(crate) interner: &'a StringInterner,
    /// Variable types (accumulated during checking)
    pub(crate) var_types: IndexMap<VarId, Type>,
    /// Current function's expected return type (for return validation)
    pub(crate) expected_return_type: Option<Type>,
    /// Class info for method/field resolution
    pub(crate) class_info: IndexMap<ClassId, ClassInfo>,
    /// Accumulated type errors
    pub(crate) errors: Vec<CompilerError>,
}

/// Information about a class for type checking
#[derive(Debug, Clone)]
pub(crate) struct ClassInfo {
    pub(crate) name: InternedString,
    pub(crate) fields: IndexMap<InternedString, Type>,
    pub(crate) methods: IndexMap<InternedString, FuncId>,
    /// Property types (from @property decorator)
    pub(crate) properties: IndexMap<InternedString, Type>,
    /// Properties that have setters
    pub(crate) property_setters: IndexMap<InternedString, bool>,
}

impl<'a> TypeChecker<'a> {
    pub fn new(interner: &'a StringInterner) -> Self {
        Self {
            interner,
            var_types: IndexMap::new(),
            expected_return_type: None,
            class_info: IndexMap::new(),
            errors: Vec::new(),
        }
    }

    /// Add an error to the accumulated errors
    pub(crate) fn add_error(&mut self, error: CompilerError) {
        self.errors.push(error);
    }

    /// Type check a module
    pub fn check_module(&mut self, module: &Module) -> Result<()> {
        // Clear any previous errors
        self.errors.clear();

        // Build class info map
        self.build_class_info(module);

        // Check all function bodies
        for func in module.func_defs.values() {
            self.var_types.clear();

            // Add function parameters to var_types
            for param in &func.params {
                if let Some(ty) = &param.ty {
                    self.var_types.insert(param.var, ty.clone());
                }
            }

            // Set expected return type
            self.expected_return_type = func.return_type.clone();

            // Check function body
            self.check_stmts(&func.body, module)?;
        }

        // Check module-level statements
        self.var_types.clear();
        self.expected_return_type = None;
        self.check_stmts(&module.module_init_stmts, module)?;

        // Return first accumulated error if any
        if let Some(error) = self.errors.pop() {
            return Err(error);
        }

        Ok(())
    }

    /// Build class info map from module
    fn build_class_info(&mut self, module: &Module) {
        for (class_id, class_def) in &module.class_defs {
            let mut fields = IndexMap::new();
            for field in &class_def.fields {
                fields.insert(field.name, field.ty.clone());
            }

            let mut methods = IndexMap::new();
            for &method_id in &class_def.methods {
                if let Some(func) = module.func_defs.get(&method_id) {
                    methods.insert(func.name, method_id);
                }
            }

            // Build property info from PropertyDef
            let mut properties = IndexMap::new();
            let mut property_setters = IndexMap::new();
            for prop in &class_def.properties {
                properties.insert(prop.name, prop.ty.clone());
                property_setters.insert(prop.name, prop.setter.is_some());
            }

            self.class_info.insert(
                *class_id,
                ClassInfo {
                    name: class_def.name,
                    fields,
                    methods,
                    properties,
                    property_setters,
                },
            );
        }
    }
}
