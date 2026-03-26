//! Kind enums for typed MIR operations
//!
//! These enums classify values and operations to reduce the number of
//! RuntimeFunc variants through parameterization.

/// Value kind for typed storage operations (cells, globals, class attrs)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueKind {
    Int,   // i64
    Float, // f64
    Bool,  // i8 (also used for None)
    Ptr,   // *mut Obj (heap types)
}

impl ValueKind {
    /// Runtime function suffix: "_int", "_float", "_bool", "_ptr"
    pub fn suffix(&self) -> &'static str {
        match self {
            ValueKind::Int => "_int",
            ValueKind::Float => "_float",
            ValueKind::Bool => "_bool",
            ValueKind::Ptr => "_ptr",
        }
    }
}

/// String format for repr/ascii operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StringFormat {
    /// Standard repr() - show string representation
    Repr,
    /// ascii() - like repr but escapes non-ASCII characters
    Ascii,
}

impl StringFormat {
    /// Runtime function prefix: "rt_repr_" or "rt_ascii_"
    pub fn prefix(&self) -> &'static str {
        match self {
            StringFormat::Repr => "rt_repr_",
            StringFormat::Ascii => "rt_ascii_",
        }
    }
}

/// Container kind for min/max operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainerKind {
    /// List container
    List,
    /// Tuple container
    Tuple,
    /// Set container
    Set,
    /// Dict container (operates on keys)
    Dict,
    /// String container (operates on characters)
    Str,
}

impl ContainerKind {
    /// Runtime function name part: "list", "tuple", "set", "dict", "str"
    pub fn name(&self) -> &'static str {
        match self {
            ContainerKind::List => "list",
            ContainerKind::Tuple => "tuple",
            ContainerKind::Set => "set",
            ContainerKind::Dict => "dict",
            ContainerKind::Str => "str",
        }
    }
}

/// Min/max operation kind
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MinMaxOp {
    /// Find minimum value
    Min,
    /// Find maximum value
    Max,
}

impl MinMaxOp {
    /// Runtime function name part: "min", "max"
    pub fn name(&self) -> &'static str {
        match self {
            MinMaxOp::Min => "min",
            MinMaxOp::Max => "max",
        }
    }
}

/// Element type kind for min/max operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElementKind {
    /// Integer elements - returns i64
    Int,
    /// Float elements - returns f64
    Float,
    /// Use key function - returns *mut Obj
    WithKey,
}

impl ElementKind {
    /// Runtime function suffix: "_int", "_float", "_with_key"
    pub fn suffix(&self) -> &'static str {
        match self {
            ElementKind::Int => "_int",
            ElementKind::Float => "_float",
            ElementKind::WithKey => "_with_key",
        }
    }
}

/// Iterator source kind for iterator creation operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IterSourceKind {
    /// List container
    List,
    /// Tuple container
    Tuple,
    /// Dict container (iterates over keys)
    Dict,
    /// String container (iterates over characters)
    Str,
    /// Range iterator (requires start, stop, step args)
    Range,
    /// Set container
    Set,
    /// Bytes container (iterates over integers)
    Bytes,
    /// Generator/iterator (returns itself)
    Generator,
}

impl IterSourceKind {
    /// Runtime function name part: "list", "tuple", "dict", "str", "range", "set", "bytes", "generator"
    pub fn name(&self) -> &'static str {
        match self {
            IterSourceKind::List => "list",
            IterSourceKind::Tuple => "tuple",
            IterSourceKind::Dict => "dict",
            IterSourceKind::Str => "str",
            IterSourceKind::Range => "range",
            IterSourceKind::Set => "set",
            IterSourceKind::Bytes => "bytes",
            IterSourceKind::Generator => "generator",
        }
    }

    /// Whether this source requires three arguments (start, stop, step) instead of one object
    pub fn requires_range_args(&self) -> bool {
        matches!(self, IterSourceKind::Range)
    }
}

/// Iterator direction for iterator creation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IterDirection {
    /// Forward iteration
    Forward,
    /// Reversed iteration
    Reversed,
}

impl IterDirection {
    /// Runtime function prefix: "" for forward, "reversed_" for reversed
    pub fn prefix(&self) -> &'static str {
        match self {
            IterDirection::Forward => "",
            IterDirection::Reversed => "reversed_",
        }
    }
}

/// Sortable container kind for sorted() operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SortableKind {
    /// List container
    List,
    /// Tuple container
    Tuple,
    /// Dict container (sorts keys)
    Dict,
    /// String container (sorts characters)
    Str,
    /// Set container (sorts elements)
    Set,
    /// Range (requires start, stop, step args; no key function support)
    Range,
}

impl SortableKind {
    /// Runtime function name part: "list", "tuple", "dict", "str", "set", "range"
    pub fn name(&self) -> &'static str {
        match self {
            SortableKind::List => "list",
            SortableKind::Tuple => "tuple",
            SortableKind::Dict => "dict",
            SortableKind::Str => "str",
            SortableKind::Set => "set",
            SortableKind::Range => "range",
        }
    }

    /// Whether this source requires special args (start, stop, step) instead of one container
    pub fn is_range(&self) -> bool {
        matches!(self, SortableKind::Range)
    }
}

/// Comparison operation for container/object comparisons
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComparisonOp {
    /// Equality comparison (==)
    Eq,
    /// Less than comparison (<)
    Lt,
    /// Less than or equal comparison (<=)
    Lte,
    /// Greater than comparison (>)
    Gt,
    /// Greater than or equal comparison (>=)
    Gte,
}

impl ComparisonOp {
    /// Runtime function suffix: "_eq", "_lt", "_lte", "_gt", "_gte"
    pub fn suffix(&self) -> &'static str {
        match self {
            ComparisonOp::Eq => "_eq",
            ComparisonOp::Lt => "_lt",
            ComparisonOp::Lte => "_lte",
            ComparisonOp::Gt => "_gt",
            ComparisonOp::Gte => "_gte",
        }
    }
}

/// Comparable target kind for comparison operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompareKind {
    /// List equality comparison with integer elements
    ListInt,
    /// List equality comparison with float elements
    ListFloat,
    /// List equality comparison with string elements
    ListStr,
    /// Tuple comparison (supports all comparison ops)
    Tuple,
    /// List ordering comparison (uses elem_tag for dispatch at runtime)
    List,
    /// String equality comparison
    Str,
    /// Bytes equality comparison
    Bytes,
    /// Generic object comparison for Union types (supports all ops)
    Obj,
}

impl CompareKind {
    /// Get the runtime function name for this comparison kind and operation.
    /// Returns the full function name like "rt_list_eq_int", "rt_tuple_lt", "rt_obj_eq".
    ///
    /// # Panics (debug only)
    /// Panics if an ordering op (Lt/Lte/Gt/Gte) is used with an Eq-only kind.
    pub fn runtime_func_name(&self, op: ComparisonOp) -> &'static str {
        debug_assert!(
            self.supports_ordering() || matches!(op, ComparisonOp::Eq),
            "CompareKind::{self:?} only supports Eq, but got {op:?}"
        );
        match (self, op) {
            // List comparisons only support Eq
            (CompareKind::ListInt, _) => "rt_list_eq_int",
            (CompareKind::ListFloat, _) => "rt_list_eq_float",
            (CompareKind::ListStr, _) => "rt_list_eq_str",
            // List ordering comparisons (uses elem_tag at runtime)
            (CompareKind::List, ComparisonOp::Eq) => "rt_list_eq_int", // fallback; equality should use type-specific variants
            (CompareKind::List, ComparisonOp::Lt) => "rt_list_lt",
            (CompareKind::List, ComparisonOp::Lte) => "rt_list_lte",
            (CompareKind::List, ComparisonOp::Gt) => "rt_list_gt",
            (CompareKind::List, ComparisonOp::Gte) => "rt_list_gte",
            // String and bytes only support Eq
            (CompareKind::Str, _) => "rt_str_eq",
            (CompareKind::Bytes, _) => "rt_bytes_eq",
            // Tuple supports all comparison ops
            (CompareKind::Tuple, ComparisonOp::Eq) => "rt_tuple_eq",
            (CompareKind::Tuple, ComparisonOp::Lt) => "rt_tuple_lt",
            (CompareKind::Tuple, ComparisonOp::Lte) => "rt_tuple_lte",
            (CompareKind::Tuple, ComparisonOp::Gt) => "rt_tuple_gt",
            (CompareKind::Tuple, ComparisonOp::Gte) => "rt_tuple_gte",
            // Object supports all comparison ops
            (CompareKind::Obj, ComparisonOp::Eq) => "rt_obj_eq",
            (CompareKind::Obj, ComparisonOp::Lt) => "rt_obj_lt",
            (CompareKind::Obj, ComparisonOp::Lte) => "rt_obj_lte",
            (CompareKind::Obj, ComparisonOp::Gt) => "rt_obj_gt",
            (CompareKind::Obj, ComparisonOp::Gte) => "rt_obj_gte",
        }
    }

    /// Whether this kind supports ordering comparisons (Lt, Lte, Gt, Gte).
    /// Only Tuple and Obj support ordering; List/Str/Bytes only support Eq.
    pub fn supports_ordering(&self) -> bool {
        matches!(
            self,
            CompareKind::List | CompareKind::Tuple | CompareKind::Obj
        )
    }
}

/// Print operation kind for typed print operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrintKind {
    /// Integer (i64 value)
    Int,
    /// Float (f64 value)
    Float,
    /// Bool (i8 value)
    Bool,
    /// None (no argument)
    None,
    /// Raw string pointer (for legacy/direct string printing)
    Str,
    /// Heap-allocated string object (*mut Obj)
    StrObj,
    /// Bytes object (*mut Obj)
    BytesObj,
    /// Generic heap object with runtime dispatch (*mut Obj)
    Obj,
}

impl PrintKind {
    /// Get the runtime function name: "rt_print_{suffix}"
    pub fn runtime_func_name(&self) -> &'static str {
        match self {
            PrintKind::Int => "rt_print_int_value",
            PrintKind::Float => "rt_print_float_value",
            PrintKind::Bool => "rt_print_bool_value",
            PrintKind::None => "rt_print_none_value",
            PrintKind::Str => "rt_print_str_value",
            PrintKind::StrObj => "rt_print_str_obj",
            PrintKind::BytesObj => "rt_print_bytes_obj",
            PrintKind::Obj => "rt_print_obj",
        }
    }

    /// Whether this kind takes an argument (None doesn't)
    pub fn has_argument(&self) -> bool {
        !matches!(self, PrintKind::None)
    }

    /// Whether this kind takes a heap object pointer (vs raw value)
    pub fn is_heap_type(&self) -> bool {
        matches!(
            self,
            PrintKind::StrObj | PrintKind::BytesObj | PrintKind::Obj
        )
    }
}

/// Target type kind for repr/ascii operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReprTargetKind {
    /// Integer - raw i64 value
    Int,
    /// Float - raw f64 value
    Float,
    /// Bool - raw i8 value
    Bool,
    /// None - no argument needed
    None,
    /// String object pointer
    Str,
    /// List object pointer
    List,
    /// Tuple object pointer
    Tuple,
    /// Dict object pointer
    Dict,
    /// Set object pointer
    Set,
    /// Bytes object pointer
    Bytes,
    /// Generic object pointer (runtime dispatch)
    Obj,
}

impl ReprTargetKind {
    /// Runtime function suffix: "int", "float", "bool", "none", "str", etc.
    pub fn suffix(&self) -> &'static str {
        match self {
            ReprTargetKind::Int => "int",
            ReprTargetKind::Float => "float",
            ReprTargetKind::Bool => "bool",
            ReprTargetKind::None => "none",
            ReprTargetKind::Str => "str",
            ReprTargetKind::List => "list",
            ReprTargetKind::Tuple => "tuple",
            ReprTargetKind::Dict => "dict",
            ReprTargetKind::Set => "set",
            ReprTargetKind::Bytes => "bytes",
            ReprTargetKind::Obj => "obj",
        }
    }

    /// Whether this type uses raw primitive value (not object pointer)
    pub fn is_primitive(&self) -> bool {
        matches!(
            self,
            ReprTargetKind::Int | ReprTargetKind::Float | ReprTargetKind::Bool
        )
    }

    /// Whether this type takes no arguments (None)
    pub fn is_nullary(&self) -> bool {
        matches!(self, ReprTargetKind::None)
    }
}

/// Type kind for conversion operations between primitive types and strings
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConversionTypeKind {
    /// Integer (i64)
    Int,
    /// Float (f64)
    Float,
    /// Boolean (i8)
    Bool,
    /// None type — unlike `PrintKind::None` and `ReprTargetKind::None`, conversion
    /// functions with this variant (e.g. `rt_none_to_str`) still take an argument.
    None,
    /// String object (*mut Obj)
    Str,
}

impl ConversionTypeKind {
    /// Runtime function name part: "int", "float", "bool", "none", "str"
    pub fn name(&self) -> &'static str {
        match self {
            ConversionTypeKind::Int => "int",
            ConversionTypeKind::Float => "float",
            ConversionTypeKind::Bool => "bool",
            ConversionTypeKind::None => "none",
            ConversionTypeKind::Str => "str",
        }
    }

    /// Build runtime function name for conversion: "rt_{from}_to_{to}"
    pub fn runtime_func_name(from: ConversionTypeKind, to: ConversionTypeKind) -> String {
        format!("rt_{}_to_{}", from.name(), to.name())
    }
}
