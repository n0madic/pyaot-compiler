//! Core types for stdlib definitions
//!
//! These types describe the structure of stdlib modules, functions, attributes,
//! and constants in a declarative way.
//!
//! **Design principle**: No enums for identifying functions/attributes.
//! Instead, we use `&'static StdlibFunctionDef` and `&'static StdlibAttrDef`
//! references directly. This means adding a new stdlib function only requires
//! adding its definition - no separate enum to maintain.

// ============= Type specifications =============

/// Type specification for stdlib function parameters and return types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeSpec {
    /// int (i64)
    Int,
    /// float (f64)
    Float,
    /// bool
    Bool,
    /// str
    Str,
    /// None
    None,
    /// bytes
    Bytes,
    /// list[T] - element type is specified separately
    List(TypeSpecRef),
    /// dict[K, V] - key and value types specified separately
    Dict(TypeSpecRef, TypeSpecRef),
    /// tuple[T...] - variadic elements (homogeneous for simplicity)
    Tuple(TypeSpecRef),
    /// set[T]
    Set(TypeSpecRef),
    /// Optional[T] = Union[T, None]
    Optional(TypeSpecRef),
    /// Any (runtime-determined type)
    Any,
    /// Iterator[T]
    Iterator(TypeSpecRef),
    /// File object
    File,
    /// Match object (from re module)
    Match,
    /// struct_time object (from time module)
    StructTime,
    /// CompletedProcess object (from subprocess module)
    CompletedProcess,
    /// ParseResult object (from urllib.parse module)
    ParseResult,
    /// HTTPResponse object (from urllib.request module)
    HttpResponse,
    /// Hash object (from hashlib module)
    Hash,
    /// StringIO object (from io module)
    StringIO,
    /// BytesIO object (from io module)
    BytesIO,
}

/// Reference to another TypeSpec (used for nested types)
/// Points to a static TypeSpec
pub type TypeSpecRef = &'static TypeSpec;

/// Constant value that can be evaluated at compile time
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConstValue {
    /// Integer constant
    Int(i64),
    /// Float constant
    Float(f64),
    /// Boolean constant
    Bool(bool),
    /// String constant (static lifetime)
    Str(&'static str),
}

/// Parameter definition for a stdlib function
#[derive(Debug, Clone, Copy)]
pub struct ParamDef {
    /// Parameter name
    pub name: &'static str,
    /// Parameter type
    pub ty: TypeSpec,
    /// Whether the parameter is optional (has a default)
    pub optional: bool,
    /// Whether this is a variadic parameter (*args)
    pub variadic: bool,
    /// Default value for optional parameters (None if required)
    pub default: Option<ConstValue>,
}

impl ParamDef {
    /// Create a required parameter
    pub const fn required(name: &'static str, ty: TypeSpec) -> Self {
        Self {
            name,
            ty,
            optional: false,
            variadic: false,
            default: None,
        }
    }

    /// Create an optional parameter with default value
    pub const fn optional_with_default(
        name: &'static str,
        ty: TypeSpec,
        default: ConstValue,
    ) -> Self {
        Self {
            name,
            ty,
            optional: true,
            variadic: false,
            default: Some(default),
        }
    }

    /// Create an optional parameter (legacy, no default - use None)
    pub const fn optional(name: &'static str, ty: TypeSpec) -> Self {
        Self {
            name,
            ty,
            optional: true,
            variadic: false,
            default: None,
        }
    }

    /// Create a variadic parameter (*args)
    pub const fn variadic(name: &'static str, ty: TypeSpec) -> Self {
        Self {
            name,
            ty,
            optional: true,
            variadic: true,
            default: None,
        }
    }
}

/// Hints for lowering phase - declarative special handling
///
/// Instead of matching on runtime_name in lowering code, functions
/// declare what special handling they need via these hints.
#[derive(Debug, Clone, Copy, Default)]
pub struct LoweringHints {
    /// Collect variadic arguments into a list before calling runtime.
    /// Used by functions like os.path.join(*paths).
    pub variadic_to_list: bool,

    /// Use auto-boxing: primitives passed to Any parameters are boxed.
    /// When true, lowering will box Int/Float/Bool arguments for Any params.
    /// Default is true for most functions - set to false to disable.
    pub auto_box: bool,
}

impl LoweringHints {
    /// Default hints - auto-boxing enabled, no variadic conversion
    pub const DEFAULT: Self = Self {
        variadic_to_list: false,
        auto_box: true,
    };

    /// Hints for variadic functions that collect args to list
    pub const VARIADIC_TO_LIST: Self = Self {
        variadic_to_list: true,
        auto_box: true,
    };

    /// Hints with no auto-boxing (for functions that handle primitives directly)
    pub const NO_AUTO_BOX: Self = Self {
        variadic_to_list: false,
        auto_box: false,
    };
}

/// Function definition for a stdlib function
///
/// Identified by its static reference (`&'static StdlibFunctionDef`),
/// not by an enum variant.
#[derive(Debug, Clone, Copy)]
pub struct StdlibFunctionDef {
    /// Function name as it appears in Python (e.g., "exit", "dumps")
    pub name: &'static str,
    /// Runtime function name for codegen (e.g., "rt_sys_exit", "rt_json_dumps")
    pub runtime_name: &'static str,
    /// Function parameters
    pub params: &'static [ParamDef],
    /// Return type
    pub return_type: TypeSpec,
    /// Minimum number of required arguments
    pub min_args: usize,
    /// Maximum number of arguments (usize::MAX for variadic)
    pub max_args: usize,
    /// Lowering hints for special handling
    pub hints: LoweringHints,
}

impl StdlibFunctionDef {
    /// Check if the argument count is valid
    pub const fn valid_arg_count(&self, count: usize) -> bool {
        count >= self.min_args && count <= self.max_args
    }

    /// Check if parameter at index expects Any type (needs boxing for primitives)
    pub const fn param_is_any(&self, index: usize) -> bool {
        if index < self.params.len() {
            matches!(self.params[index].ty, TypeSpec::Any)
        } else {
            false
        }
    }
}

/// Attribute definition for a stdlib module attribute
///
/// Identified by its static reference (`&'static StdlibAttrDef`),
/// not by an enum variant.
#[derive(Debug, Clone, Copy)]
pub struct StdlibAttrDef {
    /// Attribute name (e.g., "argv", "environ")
    pub name: &'static str,
    /// Runtime getter function name (e.g., "rt_sys_get_argv")
    pub runtime_getter: &'static str,
    /// Attribute type
    pub ty: TypeSpec,
    /// Whether the attribute is writable (most are read-only)
    pub writable: bool,
}

/// Constant definition for a stdlib module constant
#[derive(Debug, Clone, Copy)]
pub struct StdlibConstDef {
    /// Constant name (e.g., "pi", "e")
    pub name: &'static str,
    /// Compile-time value
    pub value: ConstValue,
    /// Type of the constant
    pub ty: TypeSpec,
}

/// Class definition for stdlib classes (e.g., re.Match)
#[derive(Debug, Clone, Copy)]
pub struct StdlibClassDef {
    /// Class name (e.g., "Match")
    pub name: &'static str,
    /// Methods available on instances
    pub methods: &'static [StdlibMethodDef],
    /// TypeSpec for this class when used as a type annotation (e.g., time.struct_time -> StructTime)
    /// None if the class cannot be used as a type annotation
    pub type_spec: Option<TypeSpec>,
}

/// Method definition for a stdlib class
#[derive(Debug, Clone, Copy)]
pub struct StdlibMethodDef {
    /// Method name (e.g., "group", "start")
    pub name: &'static str,
    /// Runtime function name (e.g., "rt_match_group")
    pub runtime_name: &'static str,
    /// Parameters (excluding self)
    pub params: &'static [ParamDef],
    /// Return type
    pub return_type: TypeSpec,
    /// Minimum number of required arguments (excluding self)
    pub min_args: usize,
    /// Maximum number of arguments (excluding self)
    pub max_args: usize,
}

/// Module definition for a stdlib module
#[derive(Debug, Clone, Copy)]
pub struct StdlibModuleDef {
    /// Module name (e.g., "sys", "os", "os.path")
    pub name: &'static str,
    /// Functions in this module
    pub functions: &'static [StdlibFunctionDef],
    /// Attributes in this module (e.g., sys.argv)
    pub attrs: &'static [StdlibAttrDef],
    /// Constants in this module (e.g., math.pi)
    pub constants: &'static [StdlibConstDef],
    /// Classes in this module (e.g., re.Match)
    pub classes: &'static [StdlibClassDef],
    /// Submodules (e.g., os contains os.path)
    pub submodules: &'static [&'static StdlibModuleDef],
}

impl StdlibModuleDef {
    /// Get a function by name
    pub const fn get_function(&self, name: &str) -> Option<&'static StdlibFunctionDef> {
        let mut i = 0;
        while i < self.functions.len() {
            if const_str_eq(self.functions[i].name, name) {
                return Some(&self.functions[i]);
            }
            i += 1;
        }
        None
    }

    /// Get an attribute by name
    pub const fn get_attr(&self, name: &str) -> Option<&'static StdlibAttrDef> {
        let mut i = 0;
        while i < self.attrs.len() {
            if const_str_eq(self.attrs[i].name, name) {
                return Some(&self.attrs[i]);
            }
            i += 1;
        }
        None
    }

    /// Get a constant by name
    pub const fn get_constant(&self, name: &str) -> Option<&'static StdlibConstDef> {
        let mut i = 0;
        while i < self.constants.len() {
            if const_str_eq(self.constants[i].name, name) {
                return Some(&self.constants[i]);
            }
            i += 1;
        }
        None
    }

    /// Get a class by name
    pub const fn get_class(&self, name: &str) -> Option<&'static StdlibClassDef> {
        let mut i = 0;
        while i < self.classes.len() {
            if const_str_eq(self.classes[i].name, name) {
                return Some(&self.classes[i]);
            }
            i += 1;
        }
        None
    }

    /// Get a submodule by name
    pub const fn get_submodule(&self, name: &str) -> Option<&'static StdlibModuleDef> {
        let mut i = 0;
        while i < self.submodules.len() {
            if const_str_eq(self.submodules[i].name, name) {
                return Some(self.submodules[i]);
            }
            i += 1;
        }
        None
    }

    /// Check if a name exists in this module (function, attr, constant, or class)
    pub fn has_name(&self, name: &str) -> bool {
        self.get_function(name).is_some()
            || self.get_attr(name).is_some()
            || self.get_constant(name).is_some()
            || self.get_class(name).is_some()
    }
}

impl StdlibClassDef {
    /// Get a method by name
    pub const fn get_method(&self, name: &str) -> Option<&'static StdlibMethodDef> {
        let mut i = 0;
        while i < self.methods.len() {
            if const_str_eq(self.methods[i].name, name) {
                return Some(&self.methods[i]);
            }
            i += 1;
        }
        None
    }
}

/// Const-compatible string equality check
const fn const_str_eq(a: &str, b: &str) -> bool {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    if a_bytes.len() != b_bytes.len() {
        return false;
    }
    let mut i = 0;
    while i < a_bytes.len() {
        if a_bytes[i] != b_bytes[i] {
            return false;
        }
        i += 1;
    }
    true
}

// Static type references for nested types
pub static TYPE_STR: TypeSpec = TypeSpec::Str;
pub static TYPE_INT: TypeSpec = TypeSpec::Int;
pub static TYPE_ANY: TypeSpec = TypeSpec::Any;
