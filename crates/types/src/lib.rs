//! Type system for the Python AOT compiler

#![forbid(unsafe_code)]

pub mod exceptions;
pub mod type_tags;

pub use exceptions::{
    exception_name_to_tag, exception_tag_to_name, is_builtin_exception_name, BuiltinException,
    BuiltinExceptionKind, BUILTIN_EXCEPTIONS, BUILTIN_EXCEPTION_COUNT,
};

pub use type_tags::{is_type_tag_name, type_tag_to_name, TypeTagKind, TYPE_TAG_COUNT};

use pyaot_utils::{ClassId, InternedString};

/// Type identifier
pub type TypeId = u32;

/// The type representation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    /// Primitive types
    Int,
    Float,
    Bool,
    Str,
    Bytes,
    None,

    /// Generic list
    List(Box<Type>),

    /// Generic dict
    Dict(Box<Type>, Box<Type>),

    /// defaultdict (dict subtype with factory for missing keys)
    DefaultDict(Box<Type>, Box<Type>),

    /// Generic set
    Set(Box<Type>),

    /// Tuple with specific element types
    Tuple(Vec<Type>),

    /// Union type (includes Optional via Union[T, None])
    /// Stored as a Vec with unique elements (normalized)
    Union(Vec<Type>),

    /// Function type
    Function {
        params: Vec<Type>,
        ret: Box<Type>,
    },

    /// Type variable (for generics)
    Var(InternedString),

    /// Unknown/Any type (for gradual typing)
    Any,

    /// User-defined class type
    Class {
        class_id: ClassId,
        name: InternedString,
    },

    /// Iterator type (holds element type)
    Iterator(Box<Type>),

    /// Built-in exception types (for try/except type checking)
    /// Uses BuiltinExceptionKind enum from exceptions module
    BuiltinException(BuiltinExceptionKind),

    /// File type (for open() builtin)
    File,

    /// Runtime object type from stdlib (StructTime, CompletedProcess, etc.)
    /// Uses TypeTagKind as single source of truth from core-defs.
    /// Field/method definitions are in stdlib-defs/object_types.rs
    RuntimeObject(TypeTagKind),

    /// Bottom type (empty union, uninhabited type)
    /// Represents the type with no values - used for empty unions
    /// and unreachable code. Never is a subtype of all types.
    Never,
}

impl Type {
    /// Create an Optional type (sugar for Union[T, None])
    pub fn optional(t: Type) -> Type {
        // Ensure unique elements
        if t == Type::None {
            Type::None
        } else {
            Type::Union(vec![t, Type::None])
        }
    }

    /// Check if this is None type
    pub fn is_none(&self) -> bool {
        matches!(self, Type::None)
    }

    /// Check if this is a built-in exception type
    pub fn is_builtin_exception(&self) -> bool {
        matches!(self, Type::BuiltinException(_))
    }

    /// Get the exception type tag for built-in exceptions (0-13).
    /// Returns None if not a built-in exception type.
    pub fn builtin_exception_type_tag(&self) -> Option<u8> {
        match self {
            Type::BuiltinException(kind) => Some(kind.tag()),
            _ => None,
        }
    }

    /// Get the BuiltinExceptionKind if this is a built-in exception type.
    pub fn as_builtin_exception(&self) -> Option<BuiltinExceptionKind> {
        match self {
            Type::BuiltinException(kind) => Some(*kind),
            _ => None,
        }
    }

    /// Returns true if values of this type are heap-allocated and need pointer storage/GC tracking.
    /// Raw primitives (Int, Bool, Float, None) are stored as immediate values.
    /// Compile-time-only types (Function, Var, Never) are not heap-allocated.
    pub fn is_heap(&self) -> bool {
        !matches!(
            self,
            Type::Int
                | Type::Bool
                | Type::Float
                | Type::None
                | Type::Function { .. }
                | Type::Var(_)
                | Type::Never
        )
    }

    /// Check if this is a Union type
    pub fn is_union(&self) -> bool {
        matches!(self, Type::Union(_))
    }

    /// Normalize a union type (flatten nested unions, remove duplicates)
    pub fn normalize_union(types: Vec<Type>) -> Type {
        let mut result = Vec::new();
        for ty in types {
            match ty {
                Type::Union(inner) => {
                    for t in inner {
                        if !result.contains(&t) {
                            result.push(t);
                        }
                    }
                }
                t => {
                    if !result.contains(&t) {
                        result.push(t);
                    }
                }
            }
        }

        if result.is_empty() {
            Type::Never
        } else if result.len() == 1 {
            result
                .into_iter()
                .next()
                .expect("union simplification must produce at least one type when len==1")
        } else {
            Type::Union(result)
        }
    }

    /// Narrow a Union type when isinstance(x, T) is true.
    /// Returns the narrowed type (the intersection of self with target).
    /// For Union[int, str] narrowed to int -> int
    /// For non-Union types, returns self if it matches target, otherwise Never.
    ///
    /// Returns Type::Never if no types match the target, indicating unreachable code.
    pub fn narrow_to(&self, target: &Type) -> Type {
        match self {
            Type::Union(types) => {
                // Find all types in the union that match the target
                let matching: Vec<Type> = types
                    .iter()
                    .filter(|t| Self::types_match_for_isinstance(t, target))
                    .cloned()
                    .collect();

                if matching.is_empty() {
                    // No matching types - return Never (unreachable code)
                    Type::Never
                } else if matching.len() == 1 {
                    matching
                        .into_iter()
                        .next()
                        .expect("type narrowing must produce at least one type when len==1")
                } else {
                    Type::Union(matching)
                }
            }
            // For non-Union types, return self if it matches target
            _ => {
                if Self::types_match_for_isinstance(self, target) {
                    self.clone()
                } else {
                    // Doesn't match - return Never (unreachable code)
                    Type::Never
                }
            }
        }
    }

    /// Narrow a Union type when isinstance(x, T) is false.
    /// Returns the type excluding the target.
    /// For Union[int, str] excluding int -> str
    pub fn narrow_excluding(&self, excluded: &Type) -> Type {
        match self {
            Type::Union(types) => {
                // Keep all types that don't match the excluded type
                let remaining: Vec<Type> = types
                    .iter()
                    .filter(|t| !Self::types_match_for_isinstance(t, excluded))
                    .cloned()
                    .collect();

                if remaining.is_empty() {
                    // All types excluded - return Never (bottom type)
                    Type::Never
                } else if remaining.len() == 1 {
                    remaining
                        .into_iter()
                        .next()
                        .expect("type exclusion must produce at least one type when len==1")
                } else {
                    Type::Union(remaining)
                }
            }
            // For non-Union types, return self if it doesn't match excluded
            _ => self.clone(),
        }
    }

    /// Check if two types match for isinstance purposes.
    /// This is used for type narrowing - we check if the runtime type
    /// would be considered an instance of the target type.
    fn types_match_for_isinstance(actual: &Type, target: &Type) -> bool {
        match (actual, target) {
            // Exact primitive matches
            (Type::Int, Type::Int) => true,
            (Type::Float, Type::Float) => true,
            (Type::Bool, Type::Bool) => true,
            (Type::Str, Type::Str) => true,
            (Type::Bytes, Type::Bytes) => true,
            (Type::None, Type::None) => true,

            // Container types - match by container kind (ignore element types)
            (Type::List(_), Type::List(_)) => true,
            (Type::Dict(_, _), Type::Dict(_, _)) => true,
            (Type::DefaultDict(_, _), Type::DefaultDict(_, _)) => true,
            // defaultdict is a subtype of dict
            (Type::DefaultDict(_, _), Type::Dict(_, _)) => true,
            (Type::Set(_), Type::Set(_)) => true,
            (Type::Tuple(_), Type::Tuple(_)) => true,

            // Class types - match by class_id
            // Note: Inheritance is handled at runtime via rt_isinstance_class_inherited.
            // This compile-time check only handles exact class matches.
            // For full inheritance support, the class hierarchy context would be needed.
            (
                Type::Class {
                    class_id: id1,
                    name: _,
                },
                Type::Class {
                    class_id: id2,
                    name: _,
                },
            ) => id1 == id2,

            // Iterator types
            (Type::Iterator(_), Type::Iterator(_)) => true,

            // File type
            (Type::File, Type::File) => true,

            // Runtime object types match by TypeTagKind
            (Type::RuntimeObject(k1), Type::RuntimeObject(k2)) => k1 == k2,

            // Built-in exception types match by kind
            (Type::BuiltinException(k1), Type::BuiltinException(k2)) => k1 == k2,

            // Everything else doesn't match
            _ => false,
        }
    }

    /// Check if type is a subtype of another
    pub fn is_subtype_of(&self, other: &Type) -> bool {
        match (self, other) {
            // Reflexivity
            (a, b) if a == b => true,

            // Never is subtype of everything (bottom type)
            (Type::Never, _) => true,
            // Nothing is subtype of Never (except Never itself, handled by reflexivity)
            (_, Type::Never) => false,

            // Any is supertype of everything
            (_, Type::Any) => true,
            (Type::Any, _) => false,

            // Bool is subtype of Int (Python semantics: isinstance(True, int) == True)
            (Type::Bool, Type::Int) => true,

            // None is subtype of Optional[T]
            (Type::None, Type::Union(set)) if set.contains(&Type::None) => true,

            // Union subtyping: all members of left must be subtypes of right
            (Type::Union(left), right) => left.iter().all(|t| t.is_subtype_of(right)),

            // Right is union: left must be subtype of at least one member
            (left, Type::Union(right)) => right.iter().any(|t| left.is_subtype_of(t)),

            // Container subtyping design decision:
            //
            // IMPORTANT: Mutable containers (List, Set, Dict) use COVARIANT subtyping here,
            // which is technically unsound but practically useful. In a fully sound type system,
            // mutable containers should be INVARIANT (list[int] is NOT a subtype of list[int|str]).
            //
            // Why covariance is unsound for mutable containers:
            // ```python
            // x: list[int] = [1, 2]
            // y: list[int|str] = x    # Allowed by covariance
            // y.append("hello")       # Valid for list[int|str]
            // z = x[2] + 1            # Runtime error: x now contains a string!
            // ```
            //
            // Why we use covariance anyway:
            // 1. Type inference for literals: When you write `x: list[int|str] = [1, 2, 3]`,
            //    the literal [1, 2, 3] has type list[int]. Without covariance, this assignment
            //    would be rejected, requiring explicit type annotations on the literal.
            // 2. Practical compatibility: Most Python code doesn't exploit this unsoundness,
            //    and the convenience outweighs the theoretical risk.
            //
            // Any element type is compatible (for empty containers).
            (Type::List(a), Type::List(b)) => {
                **a == Type::Any || **b == Type::Any || a.is_subtype_of(b)
            }
            (Type::Set(a), Type::Set(b)) => {
                **a == Type::Any || **b == Type::Any || a.is_subtype_of(b)
            }
            (Type::Dict(k1, v1), Type::Dict(k2, v2))
            | (Type::DefaultDict(k1, v1), Type::DefaultDict(k2, v2))
            | (Type::DefaultDict(k1, v1), Type::Dict(k2, v2)) => {
                (**k1 == Type::Any || **k2 == Type::Any || k1.is_subtype_of(k2))
                    && (**v1 == Type::Any || **v2 == Type::Any || v1.is_subtype_of(v2))
            }
            // Tuple is immutable, so covariance is sound
            (Type::Tuple(ts1), Type::Tuple(ts2)) => {
                ts1.len() == ts2.len()
                    && ts1
                        .iter()
                        .zip(ts2.iter())
                        .all(|(t1, t2)| *t1 == Type::Any || t1.is_subtype_of(t2))
            }

            // Function types (contravariant in params, covariant in return)
            (
                Type::Function {
                    params: p1,
                    ret: r1,
                },
                Type::Function {
                    params: p2,
                    ret: r2,
                },
            ) => {
                p1.len() == p2.len()
                    && p2
                        .iter()
                        .zip(p1.iter())
                        .all(|(t2, t1)| t2.is_subtype_of(t1))
                    && r1.is_subtype_of(r2)
            }

            // Class types - same class only for compile-time subtyping.
            // Inheritance-based subtyping would require the class hierarchy context.
            // Runtime isinstance checks handle inheritance via rt_isinstance_class_inherited.
            (
                Type::Class {
                    class_id: id1,
                    name: _,
                },
                Type::Class {
                    class_id: id2,
                    name: _,
                },
            ) => id1 == id2,

            // Iterator types - covariant in element type
            (Type::Iterator(a), Type::Iterator(b)) => **a == Type::Any || a.is_subtype_of(b),

            _ => false,
        }
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Int => write!(f, "int"),
            Type::Float => write!(f, "float"),
            Type::Bool => write!(f, "bool"),
            Type::Str => write!(f, "str"),
            Type::Bytes => write!(f, "bytes"),
            Type::None => write!(f, "None"),
            Type::List(t) => write!(f, "list[{}]", t),
            Type::Dict(k, v) => write!(f, "dict[{}, {}]", k, v),
            Type::DefaultDict(k, v) => write!(f, "defaultdict[{}, {}]", k, v),
            Type::Set(t) => write!(f, "set[{}]", t),
            Type::Tuple(ts) => {
                write!(f, "tuple[")?;
                for (i, t) in ts.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", t)?;
                }
                write!(f, "]")
            }
            Type::Union(ts) => {
                let types: Vec<_> = ts.iter().collect();
                for (i, t) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{}", t)?;
                }
                Ok(())
            }
            Type::Function { params, ret } => {
                write!(f, "(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", ret)
            }
            Type::Var(name) => write!(f, "'{:?}", name),
            Type::Any => write!(f, "Any"),
            Type::Class { name, .. } => write!(f, "{:?}", name),
            Type::Iterator(t) => write!(f, "Iterator[{}]", t),
            Type::BuiltinException(kind) => write!(f, "{}", kind),
            Type::File => write!(f, "File"),
            Type::RuntimeObject(kind) => write!(f, "{}", kind),
            Type::Never => write!(f, "Never"),
        }
    }
}

/// Type environment for type checking
#[derive(Debug, Default)]
pub struct TypeEnv {
    types: Vec<Type>,
}

impl TypeEnv {
    pub fn new() -> Self {
        Self { types: Vec::new() }
    }

    pub fn insert(&mut self, ty: Type) -> TypeId {
        let id = self.types.len() as TypeId;
        self.types.push(ty);
        id
    }

    pub fn get(&self, id: TypeId) -> Option<&Type> {
        self.types.get(id as usize)
    }
}

// ============================================================================
// TypeSpec conversion (from stdlib-defs)
// ============================================================================

use pyaot_stdlib_defs::TypeSpec;

/// Convert TypeSpec (from stdlib-defs) to Type
/// Used for type inference from stdlib function definitions
pub fn typespec_to_type(spec: &TypeSpec) -> Type {
    match spec {
        TypeSpec::Int => Type::Int,
        TypeSpec::Float => Type::Float,
        TypeSpec::Bool => Type::Bool,
        TypeSpec::Str => Type::Str,
        TypeSpec::None => Type::None,
        TypeSpec::Bytes => Type::Bytes,
        TypeSpec::List(elem) => Type::List(Box::new(typespec_to_type(elem))),
        TypeSpec::Dict(k, v) => {
            Type::Dict(Box::new(typespec_to_type(k)), Box::new(typespec_to_type(v)))
        }
        TypeSpec::Tuple(elem) => Type::Tuple(vec![typespec_to_type(elem)]),
        TypeSpec::Set(elem) => Type::Set(Box::new(typespec_to_type(elem))),
        TypeSpec::Optional(inner) => Type::optional(typespec_to_type(inner)),
        TypeSpec::Any => Type::Any,
        TypeSpec::Iterator(elem) => Type::Iterator(Box::new(typespec_to_type(elem))),
        TypeSpec::File => Type::File,
        // Runtime object types - use TypeTagKind as single source of truth
        TypeSpec::Match => Type::RuntimeObject(TypeTagKind::Match),
        TypeSpec::StructTime => Type::RuntimeObject(TypeTagKind::StructTime),
        TypeSpec::CompletedProcess => Type::RuntimeObject(TypeTagKind::CompletedProcess),
        TypeSpec::ParseResult => Type::RuntimeObject(TypeTagKind::ParseResult),
        TypeSpec::HttpResponse => Type::RuntimeObject(TypeTagKind::HttpResponse),
        TypeSpec::Hash => Type::RuntimeObject(TypeTagKind::Hash),
        TypeSpec::StringIO => Type::RuntimeObject(TypeTagKind::StringIO),
        TypeSpec::BytesIO => Type::RuntimeObject(TypeTagKind::BytesIO),
        TypeSpec::Deque => Type::RuntimeObject(TypeTagKind::Deque),
        TypeSpec::Counter => Type::RuntimeObject(TypeTagKind::Counter),
    }
}

#[cfg(test)]
mod tests;
