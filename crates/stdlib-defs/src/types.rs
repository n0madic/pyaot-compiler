//! Core types for stdlib definitions
//!
//! These types describe the structure of stdlib modules, functions, attributes,
//! and constants in a declarative way.
//!
//! **Design principle**: No enums for identifying functions/attributes.
//! Instead, we use `&'static StdlibFunctionDef` and `&'static StdlibAttrDef`
//! references directly. This means adding a new stdlib function only requires
//! adding its definition - no separate enum to maintain.

use pyaot_core_defs::RuntimeFuncDef;

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
    /// Request object (from urllib.request module)
    Request,
    /// Hash object (from hashlib module)
    Hash,
    /// StringIO object (from io module)
    StringIO,
    /// BytesIO object (from io module)
    BytesIO,
    /// Deque object (from collections module)
    Deque,
    /// Counter object (from collections module)
    Counter,
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

    /// Append the actual user-supplied argument count as an extra trailing i64
    /// argument after all regular parameters (including filled-in defaults).
    ///
    /// This allows runtime functions to distinguish "called with N args" from
    /// "called with N+1 args where some are filled by defaults". Useful when a
    /// sentinel default value collides with a valid user-supplied value.
    pub pass_arg_count: bool,
}

impl LoweringHints {
    /// Default hints - auto-boxing enabled, no variadic conversion
    pub const DEFAULT: Self = Self {
        variadic_to_list: false,
        auto_box: true,
        pass_arg_count: false,
    };

    /// Hints for variadic functions that collect args to list
    pub const VARIADIC_TO_LIST: Self = Self {
        variadic_to_list: true,
        auto_box: true,
        pass_arg_count: false,
    };

    /// Hints with no auto-boxing (for functions that handle primitives directly)
    pub const NO_AUTO_BOX: Self = Self {
        variadic_to_list: false,
        auto_box: false,
        pass_arg_count: false,
    };

    /// Hints with no auto-boxing and an appended argument count parameter.
    /// Use for functions where a sentinel default would collide with a valid seed value.
    pub const NO_AUTO_BOX_PASS_ARG_COUNT: Self = Self {
        variadic_to_list: false,
        auto_box: false,
        pass_arg_count: true,
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
    /// Codegen descriptor for the generic RuntimeFunc::Call handler
    pub codegen: RuntimeFuncDef,
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
    /// Codegen descriptor for the generic RuntimeFunc::Call handler
    pub codegen: RuntimeFuncDef,
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
    /// Codegen descriptor for the generic RuntimeFunc::Call handler
    pub codegen: RuntimeFuncDef,
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
    /// Exception classes provided by this module
    /// (e.g. `urllib.error.HTTPError`). Each entry carries a reserved
    /// `class_id` in `[BUILTIN_EXCEPTION_COUNT, FIRST_USER_CLASS_ID)` and a
    /// `parent` `BuiltinExceptionKind` used for catch-by-base hierarchy.
    pub exceptions: &'static [&'static StdlibExceptionClass],
    /// Submodules (e.g., os contains os.path)
    pub submodules: &'static [&'static StdlibModuleDef],
}

/// Declaration of a stdlib-provided exception class.
///
/// Stdlib exceptions (like `urllib.error.HTTPError`) are not in the Python
/// `builtins` namespace and must be imported explicitly. At runtime they
/// use the same `class_id`-based catch mechanism as user-defined exception
/// classes, which is why each declaration carries a reserved `class_id`
/// from `core_defs::RESERVED_STDLIB_EXCEPTION_SLOTS` range.
#[derive(Debug, Clone, Copy)]
pub struct StdlibExceptionClass {
    /// Python class name (e.g. "HTTPError").
    pub name: &'static str,
    /// Reserved class ID in `[BUILTIN_EXCEPTION_COUNT, FIRST_USER_CLASS_ID)`.
    /// Globally unique across all stdlib modules.
    pub class_id: u8,
    /// Immediate parent in the CPython exception hierarchy — used by the
    /// runtime so `except OSError:` correctly catches a raised HTTPError.
    pub parent: pyaot_core_defs::BuiltinExceptionKind,
    /// Module that owns this exception (e.g. "urllib.error"). Informational
    /// — used for nicer error messages.
    pub module: &'static str,
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

    /// Get an exception class by name
    pub const fn get_exception(&self, name: &str) -> Option<&'static StdlibExceptionClass> {
        let mut i = 0;
        while i < self.exceptions.len() {
            if const_str_eq(self.exceptions[i].name, name) {
                return Some(self.exceptions[i]);
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
pub(crate) const fn const_str_eq(a: &str, b: &str) -> bool {
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

// ============= TypeSpec → RuntimeFuncDef conversion helpers =============

use pyaot_core_defs::runtime_func_def::{ParamType, ReturnType};

impl TypeSpec {
    /// Convert to Cranelift parameter type for codegen descriptors.
    pub const fn to_param_type(&self) -> ParamType {
        match self {
            TypeSpec::Float => ParamType::F64,
            TypeSpec::Bool => ParamType::I8,
            _ => ParamType::I64,
        }
    }

    /// Convert to Cranelift return type for codegen descriptors.
    /// Returns `None` for void (TypeSpec::None).
    pub const fn to_return_type(&self) -> Option<ReturnType> {
        match self {
            TypeSpec::None => None,
            TypeSpec::Float => Some(ReturnType::F64),
            TypeSpec::Bool => Some(ReturnType::I8),
            _ => Some(ReturnType::I64),
        }
    }
}

// Static type references for nested types
pub static TYPE_STR: TypeSpec = TypeSpec::Str;
pub static TYPE_INT: TypeSpec = TypeSpec::Int;
pub static TYPE_ANY: TypeSpec = TypeSpec::Any;
pub static TYPE_LIST_STR: TypeSpec = TypeSpec::List(&TYPE_STR);
pub static TYPE_OPTIONAL_STR: TypeSpec = TypeSpec::Optional(&TYPE_STR);
pub static TYPE_OPTIONAL_BYTES: TypeSpec = TypeSpec::Optional(&TypeSpec::Bytes);
pub static TYPE_DICT_STR_STR: TypeSpec = TypeSpec::Dict(&TYPE_STR, &TYPE_STR);
