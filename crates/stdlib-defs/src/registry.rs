//! Registry for stdlib module lookups
//!
//! Provides functions to look up stdlib modules, functions, attributes, and constants.

use crate::modules;
use crate::types::{
    StdlibAttrDef, StdlibClassDef, StdlibConstDef, StdlibFunctionDef, StdlibModuleDef,
};

/// Get a module definition by name (supports dotted names like "os.path")
pub fn get_module(name: &str) -> Option<&'static StdlibModuleDef> {
    modules::get_module(name)
}

/// Check if a module name is a known stdlib module
pub fn is_stdlib_module(name: &str) -> bool {
    modules::is_stdlib_module(name)
}

/// Get the root module name from a dotted path
pub fn get_root_module(name: &str) -> &str {
    modules::get_root_module(name)
}

/// Get a function definition by module and function name
pub fn get_function(module_name: &str, func_name: &str) -> Option<&'static StdlibFunctionDef> {
    let module = get_module(module_name)?;
    module.get_function(func_name)
}

/// Get an attribute definition by module and attribute name
pub fn get_attr(module_name: &str, attr_name: &str) -> Option<&'static StdlibAttrDef> {
    let module = get_module(module_name)?;
    module.get_attr(attr_name)
}

/// Get a constant definition by module and constant name
pub fn get_constant(module_name: &str, const_name: &str) -> Option<&'static StdlibConstDef> {
    let module = get_module(module_name)?;
    module.get_constant(const_name)
}

/// Get a class definition by module and class name
pub fn get_class(module_name: &str, class_name: &str) -> Option<&'static StdlibClassDef> {
    let module = get_module(module_name)?;
    module.get_class(class_name)
}

/// Get the TypeSpec for a class used as a type annotation (e.g., time.struct_time)
/// Returns None if the class doesn't exist or cannot be used as a type annotation
pub fn get_class_type(module_name: &str, class_name: &str) -> Option<crate::types::TypeSpec> {
    let class = get_class(module_name, class_name)?;
    class.type_spec
}

/// Item kinds that can be imported from a stdlib module
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdlibItemKind {
    Function,
    Attr,
    Constant,
    Class,
}

/// Result of looking up a name in a stdlib module
#[derive(Debug, Clone, Copy)]
pub enum StdlibItem {
    Function(&'static StdlibFunctionDef),
    Attr(&'static StdlibAttrDef),
    Constant(&'static StdlibConstDef),
    Class(&'static StdlibClassDef),
}

impl StdlibItem {
    pub fn kind(&self) -> StdlibItemKind {
        match self {
            StdlibItem::Function(_) => StdlibItemKind::Function,
            StdlibItem::Attr(_) => StdlibItemKind::Attr,
            StdlibItem::Constant(_) => StdlibItemKind::Constant,
            StdlibItem::Class(_) => StdlibItemKind::Class,
        }
    }
}

/// Look up any item (function, attr, constant, class) in a module by name
pub fn get_item(module_name: &str, item_name: &str) -> Option<StdlibItem> {
    let module = get_module(module_name)?;

    // Check functions first (most common)
    if let Some(func) = module.get_function(item_name) {
        return Some(StdlibItem::Function(func));
    }

    // Check attributes
    if let Some(attr) = module.get_attr(item_name) {
        return Some(StdlibItem::Attr(attr));
    }

    // Check constants
    if let Some(constant) = module.get_constant(item_name) {
        return Some(StdlibItem::Constant(constant));
    }

    // Check classes
    if let Some(class) = module.get_class(item_name) {
        return Some(StdlibItem::Class(class));
    }

    None
}

/// List all function names in a module
pub fn list_functions(module_name: &str) -> Vec<&'static str> {
    match get_module(module_name) {
        Some(module) => module.functions.iter().map(|f| f.name).collect(),
        None => Vec::new(),
    }
}

/// List all attribute names in a module
pub fn list_attrs(module_name: &str) -> Vec<&'static str> {
    match get_module(module_name) {
        Some(module) => module.attrs.iter().map(|a| a.name).collect(),
        None => Vec::new(),
    }
}

/// List all constant names in a module
pub fn list_constants(module_name: &str) -> Vec<&'static str> {
    match get_module(module_name) {
        Some(module) => module.constants.iter().map(|c| c.name).collect(),
        None => Vec::new(),
    }
}

/// List all class names in a module
pub fn list_classes(module_name: &str) -> Vec<&'static str> {
    match get_module(module_name) {
        Some(module) => module.classes.iter().map(|c| c.name).collect(),
        None => Vec::new(),
    }
}

/// List all item names in a module (functions + attrs + constants + classes)
pub fn list_all_names(module_name: &str) -> Vec<&'static str> {
    let mut names = Vec::new();
    if let Some(module) = get_module(module_name) {
        names.extend(module.functions.iter().map(|f| f.name));
        names.extend(module.attrs.iter().map(|a| a.name));
        names.extend(module.constants.iter().map(|c| c.name));
        names.extend(module.classes.iter().map(|c| c.name));
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_module() {
        assert!(get_module("sys").is_some());
        assert!(get_module("os").is_some());
        assert!(get_module("os.path").is_some());
        assert!(get_module("re").is_some());
        assert!(get_module("json").is_some());
        assert!(get_module("unknown").is_none());
    }

    #[test]
    fn test_is_stdlib_module() {
        assert!(is_stdlib_module("sys"));
        assert!(is_stdlib_module("os"));
        assert!(is_stdlib_module("os.path"));
        assert!(is_stdlib_module("re"));
        assert!(is_stdlib_module("json"));
        assert!(!is_stdlib_module("mymodule"));
    }

    #[test]
    fn test_get_function() {
        let exit = get_function("sys", "exit").unwrap();
        assert_eq!(exit.name, "exit");
        assert_eq!(exit.runtime_name, "rt_sys_exit");

        let join = get_function("os.path", "join").unwrap();
        assert_eq!(join.name, "join");
        assert_eq!(join.runtime_name, "rt_os_path_join");

        assert!(get_function("sys", "nonexistent").is_none());
    }

    #[test]
    fn test_get_attr() {
        let argv = get_attr("sys", "argv").unwrap();
        assert_eq!(argv.name, "argv");
        assert_eq!(argv.runtime_getter, "rt_sys_get_argv");

        let environ = get_attr("os", "environ").unwrap();
        assert_eq!(environ.name, "environ");

        assert!(get_attr("sys", "nonexistent").is_none());
    }

    #[test]
    fn test_get_item() {
        // Function
        let item = get_item("sys", "exit").unwrap();
        assert!(matches!(item, StdlibItem::Function(_)));

        // Attr
        let item = get_item("sys", "argv").unwrap();
        assert!(matches!(item, StdlibItem::Attr(_)));

        // Class
        let item = get_item("re", "Match").unwrap();
        assert!(matches!(item, StdlibItem::Class(_)));

        // Not found
        assert!(get_item("sys", "nonexistent").is_none());
    }

    #[test]
    fn test_list_functions() {
        let funcs = list_functions("sys");
        assert!(funcs.contains(&"exit"));
        assert!(funcs.contains(&"intern"));
    }

    #[test]
    fn test_get_class_type() {
        use crate::types::TypeSpec;

        // time.struct_time -> StructTime
        let struct_time_type = get_class_type("time", "struct_time").unwrap();
        assert!(matches!(struct_time_type, TypeSpec::StructTime));

        // re.Match -> Match
        let match_type = get_class_type("re", "Match").unwrap();
        assert!(matches!(match_type, TypeSpec::Match));

        // hashlib.Hash -> Hash
        let hash_type = get_class_type("hashlib", "Hash").unwrap();
        assert!(matches!(hash_type, TypeSpec::Hash));

        // io.StringIO -> StringIO
        let stringio_type = get_class_type("io", "StringIO").unwrap();
        assert!(matches!(stringio_type, TypeSpec::StringIO));

        // io.BytesIO -> BytesIO
        let bytesio_type = get_class_type("io", "BytesIO").unwrap();
        assert!(matches!(bytesio_type, TypeSpec::BytesIO));

        // Non-existent class
        assert!(get_class_type("time", "nonexistent").is_none());

        // Non-existent module
        assert!(get_class_type("nonexistent", "struct_time").is_none());
    }

    #[test]
    fn test_cpython_api_arg_counts() {
        // os.path.join requires at least 1 argument
        let join = get_function("os.path", "join").unwrap();
        assert_eq!(join.min_args, 1);
        assert!(!join.valid_arg_count(0));
        assert!(join.valid_arg_count(1));
        assert!(join.valid_arg_count(5));

        // math.log accepts 1 or 2 arguments
        let log = get_function("math", "log").unwrap();
        assert_eq!(log.min_args, 1);
        assert_eq!(log.max_args, 2);
        assert!(!log.valid_arg_count(0));
        assert!(log.valid_arg_count(1));
        assert!(log.valid_arg_count(2));
        assert!(!log.valid_arg_count(3));

        // time.strftime accepts 1 or 2 arguments
        let strftime = get_function("time", "strftime").unwrap();
        assert_eq!(strftime.min_args, 1);
        assert_eq!(strftime.max_args, 2);
        assert!(strftime.valid_arg_count(1));
        assert!(strftime.valid_arg_count(2));
        assert!(!strftime.valid_arg_count(0));

        // random.choices accepts 1-3 arguments (weights is optional)
        let choices = get_function("random", "choices").unwrap();
        assert_eq!(choices.min_args, 1);
        assert_eq!(choices.max_args, 3);
        assert!(choices.valid_arg_count(1));
        assert!(choices.valid_arg_count(2));
        assert!(choices.valid_arg_count(3));
        assert!(!choices.valid_arg_count(0));
    }
}
