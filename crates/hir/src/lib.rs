//! High-level Intermediate Representation (HIR)
//!
//! This is a desugared, type-annotated representation of Python code.

#![forbid(unsafe_code)]

use indexmap::IndexMap;
use indexmap::IndexSet;
use la_arena::Arena;
pub use pyaot_core_defs::BuiltinFunctionKind;
use pyaot_types::{BuiltinExceptionKind, Type};
use pyaot_utils::{ClassId, FuncId, InternedString, Span, VarId};
use std::collections::HashSet;

/// Method kind for class methods (staticmethod, classmethod, instance method)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MethodKind {
    /// Regular instance method (receives self)
    #[default]
    Instance,
    /// @staticmethod - no self/cls parameter
    Static,
    /// @classmethod - receives cls as first parameter
    ClassMethod,
}

/// Property definition for @property decorators
#[derive(Debug, Clone)]
pub struct PropertyDef {
    /// Property name
    pub name: InternedString,
    /// Getter function ID
    pub getter: FuncId,
    /// Optional setter function ID
    pub setter: Option<FuncId>,
    /// Property type (return type of getter)
    pub ty: Type,
    /// Source location
    pub span: Span,
}

pub type ExprId = la_arena::Idx<Expr>;
pub type StmtId = la_arena::Idx<Stmt>;

/// Import declaration
#[derive(Debug, Clone)]
pub struct ImportDecl {
    /// Module path (e.g., "utils" or "pkg.submodule")
    pub module_path: String,
    /// Kind of import
    pub kind: ImportKind,
    /// True if this is a package import (directory with __init__.py)
    pub is_package: bool,
    /// Source location
    pub span: Span,
}

/// Kind of import
#[derive(Debug, Clone)]
pub enum ImportKind {
    /// `import module` or `import module as alias`
    Module { alias: Option<InternedString> },
    /// `from module import name1, name2` or `from module import name as alias`
    FromImport {
        names: Vec<(InternedString, Option<InternedString>)>,
    },
}

/// An imported symbol
#[derive(Debug, Clone)]
pub struct ImportedSymbol {
    /// Source module name
    pub module: String,
    /// Original name in the source module
    pub original_name: String,
    /// Kind of the imported symbol
    pub kind: ImportedKind,
}

/// Kind of imported symbol
#[derive(Debug, Clone)]
pub enum ImportedKind {
    /// Imported function
    Function(FuncId),
    /// Imported class
    Class(ClassId),
    /// Imported global variable
    Variable(VarId),
}

/// HIR Module (top-level)
#[derive(Debug)]
pub struct Module {
    pub name: InternedString,
    pub functions: Vec<FuncId>,
    pub func_defs: IndexMap<FuncId, Function>,
    pub class_defs: IndexMap<ClassId, ClassDef>,
    pub stmts: Arena<Stmt>,
    pub exprs: Arena<Expr>,
    /// Top-level statements to execute at module init (CPython semantics)
    pub module_init_stmts: Vec<StmtId>,
    /// Variables declared as global (shared across all functions)
    pub globals: IndexSet<VarId>,
    /// Import declarations
    pub imports: Vec<ImportDecl>,
    /// Map from local name to imported symbol
    pub imported_symbols: IndexMap<InternedString, ImportedSymbol>,
    /// Source module name (without mangling prefix)
    pub source_module_name: Option<String>,
    /// Map from module-level variable name to VarId (for cross-module access)
    pub module_var_map: IndexMap<InternedString, VarId>,
}

/// Class definition
#[derive(Debug, Clone)]
pub struct ClassDef {
    pub id: ClassId,
    pub name: InternedString,
    /// Base class for single inheritance (None if no parent)
    pub base_class: Option<ClassId>,
    pub fields: Vec<FieldDef>,
    /// Class attributes (shared across all instances)
    pub class_attrs: Vec<ClassAttribute>,
    pub methods: Vec<FuncId>,
    pub init_method: Option<FuncId>,
    /// Property definitions (@property getters/setters)
    pub properties: Vec<PropertyDef>,
    /// Set of abstract method names that are not yet implemented in this class
    /// (inherited from parent - overridden methods)
    pub abstract_methods: IndexSet<InternedString>,
    pub span: Span,
    /// True if this class inherits from Exception or a subclass
    pub is_exception_class: bool,
    /// For exception classes: the base exception type tag (0-12 for built-in exceptions)
    /// None if not an exception class or if inheriting from a custom exception
    pub base_exception_type: Option<u8>,
}

/// Class field definition
#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: InternedString,
    pub ty: Type,
    pub span: Span,
}

/// Class attribute definition (shared across all instances)
#[derive(Debug, Clone)]
pub struct ClassAttribute {
    pub name: InternedString,
    pub ty: Type,
    pub initializer: ExprId,
    pub span: Span,
}

/// Function definition
#[derive(Debug, Clone)]
pub struct Function {
    pub id: FuncId,
    pub name: InternedString,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub body: Vec<StmtId>,
    pub span: Span,
    /// Variables that need to be wrapped in cells (used by inner functions via nonlocal)
    pub cell_vars: HashSet<VarId>,
    /// Variables accessed via nonlocal from enclosing scope
    pub nonlocal_vars: HashSet<VarId>,
    /// True if this function contains yield expressions (is a generator)
    pub is_generator: bool,
    /// Method kind: Instance (default), Static (@staticmethod), or ClassMethod (@classmethod)
    pub method_kind: MethodKind,
    /// True if this method is marked with @abstractmethod
    pub is_abstract: bool,
}

/// Parameter kind distinguishes regular, *args, and **kwargs parameters
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    Regular,       // Regular positional/keyword parameter
    VarPositional, // *args
    KeywordOnly,   // Keyword-only parameter (after *args or bare *)
    VarKeyword,    // **kwargs
}

/// Function parameter
#[derive(Debug, Clone)]
pub struct Param {
    pub name: InternedString,
    pub var: VarId,
    pub ty: Option<Type>,
    pub default: Option<ExprId>,
    pub kind: ParamKind,
    pub span: Span,
}

/// Unpacking target (for nested patterns)
#[derive(Debug, Clone)]
pub enum UnpackTarget {
    /// Simple variable
    Var(VarId),
    /// Nested pattern: (a, (b, c))
    Nested(Vec<UnpackTarget>),
}

/// HIR Statement
#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    /// Expression statement
    Expr(ExprId),

    /// Assignment: target = value (with optional type hint from annotation)
    Assign {
        target: VarId,
        value: ExprId,
        type_hint: Option<Type>,
    },

    /// Unpacking assignment: a, b, c = value or a, *rest, b = value
    /// Supports extended unpacking with starred expression.
    /// - `before_star`: targets before the starred expression
    /// - `starred`: the starred variable (collects remaining elements as a list)
    /// - `after_star`: targets after the starred expression
    UnpackAssign {
        before_star: Vec<VarId>,
        starred: Option<VarId>,
        after_star: Vec<VarId>,
        value: ExprId,
    },

    /// Nested unpacking assignment: (a, (b, c)) = value
    /// Supports arbitrarily nested patterns (no starred expressions in nested contexts)
    NestedUnpackAssign {
        targets: Vec<UnpackTarget>,
        value: ExprId,
    },

    /// Return statement
    Return(Option<ExprId>),

    /// If statement
    If {
        cond: ExprId,
        then_block: Vec<StmtId>,
        else_block: Vec<StmtId>,
    },

    /// While loop (with optional else block, executed if loop completes without break)
    While {
        cond: ExprId,
        body: Vec<StmtId>,
        else_block: Vec<StmtId>,
    },

    /// For loop (over range or iterable, with optional else block)
    For {
        target: VarId,
        iter: ExprId,
        body: Vec<StmtId>,
        else_block: Vec<StmtId>,
    },

    /// For loop with tuple unpacking: for a, b in items
    ForUnpack {
        targets: Vec<VarId>,
        iter: ExprId,
        body: Vec<StmtId>,
        else_block: Vec<StmtId>,
    },

    /// For loop with starred unpacking: for first, *rest, last in items
    ForUnpackStarred {
        before_star: Vec<VarId>,
        starred: Option<VarId>,
        after_star: Vec<VarId>,
        iter: ExprId,
        body: Vec<StmtId>,
        else_block: Vec<StmtId>,
    },

    /// Break
    Break,

    /// Continue
    Continue,

    /// Raise exception, optionally with a cause (`raise X from Y`)
    Raise {
        exc: Option<ExprId>,
        cause: Option<ExprId>,
    },

    /// Try-except-else-finally
    Try {
        body: Vec<StmtId>,
        handlers: Vec<ExceptHandler>,
        else_block: Vec<StmtId>,
        finally_block: Vec<StmtId>,
    },

    /// Pass (no-op)
    Pass,

    /// Assert statement: assert cond or assert cond, msg
    Assert { cond: ExprId, msg: Option<ExprId> },

    /// Indexed assignment: obj[index] = value (for dict/list)
    IndexAssign {
        obj: ExprId,
        index: ExprId,
        value: ExprId,
    },

    /// Field assignment: obj.field = value (for class instances)
    FieldAssign {
        obj: ExprId,
        field: InternedString,
        value: ExprId,
    },

    /// Class attribute assignment: ClassName.attr = value (for class-level variables)
    ClassAttrAssign {
        class_id: ClassId,
        attr: InternedString,
        value: ExprId,
    },

    /// Delete indexed item: del obj[index] (for dict/list)
    IndexDelete { obj: ExprId, index: ExprId },

    /// Match statement (Python 3.10+ pattern matching)
    Match {
        subject: ExprId,
        cases: Vec<MatchCase>,
    },
}

/// Match case for match statement
#[derive(Debug, Clone)]
pub struct MatchCase {
    pub pattern: Pattern,
    pub guard: Option<ExprId>,
    pub body: Vec<StmtId>,
}

/// Pattern for match statement
#[derive(Debug, Clone)]
pub enum Pattern {
    /// Literal value: case 1, case "hello"
    MatchValue(ExprId),
    /// Singleton: case True, case False, case None
    MatchSingleton(MatchSingletonKind),
    /// Capture/wildcard/as: case x, case _, case pattern as name
    MatchAs {
        pattern: Option<Box<Pattern>>,
        name: Option<VarId>,
    },
    /// Sequence: case [x, y], case (a, b)
    MatchSequence { patterns: Vec<Pattern> },
    /// Star in sequence: case [first, *rest]
    MatchStar(Option<VarId>),
    /// Or alternatives: case 1 | 2 | 3
    MatchOr(Vec<Pattern>),
    /// Mapping: case {"key": val, **rest}
    MatchMapping {
        keys: Vec<ExprId>,
        patterns: Vec<Pattern>,
        rest: Option<VarId>,
    },
    /// Class: case Point(x=0, y=0)
    MatchClass {
        cls: ExprId,
        patterns: Vec<Pattern>,
        kwd_attrs: Vec<InternedString>,
        kwd_patterns: Vec<Pattern>,
    },
}

/// Singleton kinds for MatchSingleton pattern
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchSingletonKind {
    True,
    False,
    None,
}

/// Exception handler
#[derive(Debug, Clone)]
pub struct ExceptHandler {
    pub ty: Option<Type>,
    pub name: Option<VarId>,
    pub body: Vec<StmtId>,
}

/// HIR Expression
#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Option<Type>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    /// Integer literal
    Int(i64),

    /// Float literal
    Float(f64),

    /// Boolean literal
    Bool(bool),

    /// String literal
    Str(InternedString),

    /// Bytes literal
    Bytes(Vec<u8>),

    /// None literal
    None,

    /// Variable reference
    Var(VarId),

    /// Function reference (for calls)
    FuncRef(FuncId),

    /// Binary operation
    BinOp {
        op: BinOp,
        left: ExprId,
        right: ExprId,
    },

    /// Unary operation
    UnOp { op: UnOp, operand: ExprId },

    /// Comparison
    Compare {
        left: ExprId,
        op: CmpOp,
        right: ExprId,
    },

    /// Logical operation (and, or)
    LogicalOp {
        op: LogicalOp,
        left: ExprId,
        right: ExprId,
    },

    /// Function call
    Call {
        func: ExprId,
        args: Vec<CallArg>,
        kwargs: Vec<KeywordArg>,
        kwargs_unpack: Option<ExprId>, // **kwargs expression to unpack
    },

    /// Built-in function call (print, range, etc.)
    BuiltinCall {
        builtin: Builtin,
        args: Vec<ExprId>,
        kwargs: Vec<KeywordArg>,
    },

    /// Ternary expression: value_if_true if cond else value_if_false
    IfExpr {
        cond: ExprId,
        then_val: ExprId,
        else_val: ExprId,
    },

    /// List literal
    List(Vec<ExprId>),

    /// Tuple literal
    Tuple(Vec<ExprId>),

    /// Dict literal
    Dict(Vec<(ExprId, ExprId)>),

    /// Set literal
    Set(Vec<ExprId>),

    /// Index operation: obj[index]
    Index { obj: ExprId, index: ExprId },

    /// Slice operation: obj[start:end:step]
    Slice {
        obj: ExprId,
        start: Option<ExprId>,
        end: Option<ExprId>,
        step: Option<ExprId>,
    },

    /// Method call: obj.method(args, **kwargs)
    MethodCall {
        obj: ExprId,
        method: InternedString,
        args: Vec<ExprId>,
        kwargs: Vec<KeywordArg>,
    },

    /// Field/attribute access: obj.field
    Attribute { obj: ExprId, attr: InternedString },

    /// Class reference (for instantiation)
    ClassRef(ClassId),

    /// Class attribute reference: ClassName.attr (for class-level variables)
    ClassAttrRef {
        class_id: ClassId,
        attr: InternedString,
    },

    /// Type reference (for isinstance, type annotations as values)
    TypeRef(Type),

    /// Closure (lambda with captures)
    /// func: the generated function ID, captures: expressions for captured variables
    Closure { func: FuncId, captures: Vec<ExprId> },

    /// Yield expression (creates a generator function)
    /// value: optional value to yield (None yields None)
    Yield(Option<ExprId>),

    /// super().method(args) call for inheritance
    /// Calls the parent class's method with the given arguments
    SuperCall {
        method: InternedString,
        args: Vec<ExprId>,
    },

    /// Reference to imported symbol: `greet` from `from utils import greet`
    /// The module and name are resolved at compile time to actual FuncId/ClassId/VarId
    ImportedRef {
        /// Source module name
        module: String,
        /// Original name in the source module
        name: String,
    },

    /// Module attribute access: `utils.greet` from `import utils`
    /// Used when accessing a symbol via module prefix
    ModuleAttr {
        /// Module name
        module: String,
        /// Attribute being accessed
        attr: InternedString,
    },

    // ==================== Built-in Function References ====================
    /// Reference to a first-class builtin function (len, str, int, etc.)
    /// Used when builtins are passed as values (e.g., map(str, items))
    BuiltinRef(BuiltinFunctionKind),

    // ==================== Standard Library ====================
    /// Access to stdlib attributes (e.g., sys.argv, os.environ)
    /// Uses reference to definition for Single Source of Truth
    StdlibAttr(&'static pyaot_stdlib_defs::StdlibAttrDef),

    /// Call to stdlib function (e.g., sys.exit(), os.path.join(), re.search())
    /// Uses reference to definition for Single Source of Truth
    StdlibCall {
        func: &'static pyaot_stdlib_defs::StdlibFunctionDef,
        args: Vec<ExprId>,
    },

    /// Compile-time constant from stdlib (e.g., math.pi, math.e)
    /// These are inlined as literal values at compile time
    StdlibConst(&'static pyaot_stdlib_defs::StdlibConstDef),
}

/// Binary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    // Bitwise operators
    BitAnd,
    BitOr,
    BitXor,
    LShift,
    RShift,
}

/// Unary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    Invert, // Bitwise NOT (~)
}

/// Comparison operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    NotEq,
    Lt,
    LtE,
    Gt,
    GtE,
    In,
    NotIn,
    Is,
    IsNot,
}

/// Logical operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOp {
    And,
    Or,
}

/// Built-in functions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Print,
    Range,
    Len,
    Str,         // str() conversion function
    Int,         // int() conversion function
    Float,       // float() conversion function
    Bool,        // bool() conversion function
    Bytes,       // bytes() constructor
    Abs,         // abs() absolute value
    Pow,         // pow() exponentiation
    Min,         // min() minimum value
    Max,         // max() maximum value
    Round,       // round() round float
    Sum,         // sum() sum sequence
    All,         // all() test all true
    Any,         // any() test any true
    Chr,         // chr() int to character
    Ord,         // ord() character to int
    Isinstance,  // isinstance() type check
    Issubclass,  // issubclass() subclass check
    Hash,        // hash() hash value
    Id,          // id() object identity
    Iter,        // iter() create iterator
    Next,        // next() get next element from iterator
    Reversed,    // reversed() create reverse iterator
    Sorted,      // sorted() return new sorted list
    Set,         // set() constructor
    Open,        // open() file open
    Enumerate,   // enumerate() create (index, element) iterator
    Divmod,      // divmod(a, b) -> (a // b, a % b)
    Input,       // input(prompt) -> str
    Bin,         // bin(n) -> str
    Hex,         // hex(n) -> str
    Oct,         // oct(n) -> str
    FmtBin,      // format int as binary without prefix
    FmtHex,      // format int as lowercase hex without prefix
    FmtHexUpper, // format int as uppercase hex without prefix
    FmtOct,      // format int as octal without prefix
    Repr,        // repr(obj) -> str
    Ascii,       // ascii(obj) -> str (like repr but escapes non-ASCII)
    Type,        // type(obj) -> type string
    Callable,    // callable(obj) -> bool
    Hasattr,     // hasattr(obj, name) -> bool
    Getattr,     // getattr(obj, name, default) -> value
    Setattr,     // setattr(obj, name, value)
    Zip,         // zip(iter1, iter2) -> iterator of tuples
    Map,         // map(func, iterable) -> iterator
    Filter,      // filter(func, iterable) -> iterator
    Format,      // format(value, format_spec) -> str
    Reduce,      // functools.reduce(func, iterable, initial?) -> value
    Chain,       // itertools.chain(*iterables) -> iterator
    ISlice, // itertools.islice(iterable, stop) or islice(iterable, start, stop[, step]) -> iterator
    List,   // list() / list(iterable) -> list constructor
    Tuple,  // tuple() / tuple(iterable) -> tuple constructor
    Dict,   // dict() / dict(**kwargs) / dict(iterable) -> dict constructor
    /// Built-in exception constructors (Exception, ValueError, TypeError, etc.)
    /// Uses BuiltinExceptionKind from types crate
    BuiltinException(BuiltinExceptionKind),
}

/// Argument in a function call (regular or starred)
#[derive(Debug, Clone)]
pub enum CallArg {
    Regular(ExprId), // Normal positional argument
    Starred(ExprId), // *args unpacking at call site
}

/// Keyword argument in a function call
#[derive(Debug, Clone)]
pub struct KeywordArg {
    pub name: InternedString,
    pub value: ExprId,
    pub span: Span,
}

// ==================== Standard Library Support ====================
// Re-exported from stdlib-defs (Single Source of Truth)
pub use pyaot_stdlib_defs::{StdlibAttrDef, StdlibConstDef, StdlibFunctionDef, StdlibModuleDef};

// Re-export BuiltinFunctionKind for first-class builtin support

impl Module {
    pub fn new(name: InternedString) -> Self {
        Self {
            name,
            functions: Vec::new(),
            func_defs: IndexMap::new(),
            class_defs: IndexMap::new(),
            stmts: Arena::new(),
            exprs: Arena::new(),
            module_init_stmts: Vec::new(),
            globals: IndexSet::new(),
            imports: Vec::new(),
            imported_symbols: IndexMap::new(),
            source_module_name: None,
            module_var_map: IndexMap::new(),
        }
    }
}
