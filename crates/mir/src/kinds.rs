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

    /// Get static RuntimeFuncDef for GlobalGet of this kind
    pub fn global_get_def(&self) -> &'static pyaot_core_defs::RuntimeFuncDef {
        use pyaot_core_defs::runtime_func_def::*;
        match self {
            ValueKind::Int => &RT_GLOBAL_GET_INT,
            ValueKind::Float => &RT_GLOBAL_GET_FLOAT,
            ValueKind::Bool => &RT_GLOBAL_GET_BOOL,
            ValueKind::Ptr => &RT_GLOBAL_GET_PTR,
        }
    }

    /// Get static RuntimeFuncDef for GlobalSet of this kind
    pub fn global_set_def(&self) -> &'static pyaot_core_defs::RuntimeFuncDef {
        use pyaot_core_defs::runtime_func_def::*;
        match self {
            ValueKind::Int => &RT_GLOBAL_SET_INT,
            ValueKind::Float => &RT_GLOBAL_SET_FLOAT,
            ValueKind::Bool => &RT_GLOBAL_SET_BOOL,
            ValueKind::Ptr => &RT_GLOBAL_SET_PTR,
        }
    }

    /// Get static RuntimeFuncDef for ClassAttrGet of this kind
    pub fn class_attr_get_def(&self) -> &'static pyaot_core_defs::RuntimeFuncDef {
        use pyaot_core_defs::runtime_func_def::*;
        match self {
            ValueKind::Int => &RT_CLASS_ATTR_GET_INT,
            ValueKind::Float => &RT_CLASS_ATTR_GET_FLOAT,
            ValueKind::Bool => &RT_CLASS_ATTR_GET_BOOL,
            ValueKind::Ptr => &RT_CLASS_ATTR_GET_PTR,
        }
    }

    /// Get static RuntimeFuncDef for ClassAttrSet of this kind
    pub fn class_attr_set_def(&self) -> &'static pyaot_core_defs::RuntimeFuncDef {
        use pyaot_core_defs::runtime_func_def::*;
        match self {
            ValueKind::Int => &RT_CLASS_ATTR_SET_INT,
            ValueKind::Float => &RT_CLASS_ATTR_SET_FLOAT,
            ValueKind::Bool => &RT_CLASS_ATTR_SET_BOOL,
            ValueKind::Ptr => &RT_CLASS_ATTR_SET_PTR,
        }
    }

    /// Get static RuntimeFuncDef for MakeCell of this kind
    pub fn make_cell_def(&self) -> &'static pyaot_core_defs::RuntimeFuncDef {
        use pyaot_core_defs::runtime_func_def::*;
        match self {
            ValueKind::Int => &RT_MAKE_CELL_INT,
            ValueKind::Float => &RT_MAKE_CELL_FLOAT,
            ValueKind::Bool => &RT_MAKE_CELL_BOOL,
            ValueKind::Ptr => &RT_MAKE_CELL_PTR,
        }
    }

    /// Get static RuntimeFuncDef for CellGet of this kind
    pub fn cell_get_def(&self) -> &'static pyaot_core_defs::RuntimeFuncDef {
        use pyaot_core_defs::runtime_func_def::*;
        match self {
            ValueKind::Int => &RT_CELL_GET_INT,
            ValueKind::Float => &RT_CELL_GET_FLOAT,
            ValueKind::Bool => &RT_CELL_GET_BOOL,
            ValueKind::Ptr => &RT_CELL_GET_PTR,
        }
    }

    /// Get static RuntimeFuncDef for CellSet of this kind
    pub fn cell_set_def(&self) -> &'static pyaot_core_defs::RuntimeFuncDef {
        use pyaot_core_defs::runtime_func_def::*;
        match self {
            ValueKind::Int => &RT_CELL_SET_INT,
            ValueKind::Float => &RT_CELL_SET_FLOAT,
            ValueKind::Bool => &RT_CELL_SET_BOOL,
            ValueKind::Ptr => &RT_CELL_SET_PTR,
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

    /// Get the static RuntimeFuncDef for minmax (Int/Float variant).
    /// Signature: (container, is_min, elem_kind) -> i64
    pub fn minmax_def(&self) -> &'static pyaot_core_defs::RuntimeFuncDef {
        use pyaot_core_defs::runtime_func_def::*;
        match self {
            ContainerKind::List => &RT_LIST_MINMAX,
            ContainerKind::Tuple => &RT_TUPLE_MINMAX,
            ContainerKind::Set => &RT_SET_MINMAX,
            ContainerKind::Dict => &RT_DICT_MINMAX,
            ContainerKind::Str => &RT_STR_MINMAX,
        }
    }

    /// Get the static RuntimeFuncDef for minmax with key function.
    /// Signature: (container, key_fn, elem_tag, captures, count, is_min) -> *mut Obj
    pub fn minmax_with_key_def(&self) -> &'static pyaot_core_defs::RuntimeFuncDef {
        use pyaot_core_defs::runtime_func_def::*;
        match self {
            ContainerKind::List => &RT_LIST_MINMAX_WITH_KEY,
            ContainerKind::Tuple => &RT_TUPLE_MINMAX_WITH_KEY,
            ContainerKind::Set => &RT_SET_MINMAX_WITH_KEY,
            ContainerKind::Dict => &RT_DICT_MINMAX_WITH_KEY,
            ContainerKind::Str => &RT_STR_MINMAX_WITH_KEY,
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

    /// Numeric tag for passing to generic runtime minmax functions.
    /// Min=0, Max=1
    pub fn to_tag(&self) -> u8 {
        match self {
            MinMaxOp::Min => 0,
            MinMaxOp::Max => 1,
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

    /// Get the static RuntimeFuncDef for creating an iterator from this source and direction.
    pub fn iterator_def(
        &self,
        direction: IterDirection,
    ) -> &'static pyaot_core_defs::RuntimeFuncDef {
        use pyaot_core_defs::runtime_func_def::*;
        match (direction, self) {
            (IterDirection::Forward, IterSourceKind::List) => &RT_ITER_LIST,
            (IterDirection::Forward, IterSourceKind::Tuple) => &RT_ITER_TUPLE,
            (IterDirection::Forward, IterSourceKind::Dict) => &RT_ITER_DICT,
            (IterDirection::Forward, IterSourceKind::Str) => &RT_ITER_STR,
            (IterDirection::Forward, IterSourceKind::Range) => &RT_ITER_RANGE,
            (IterDirection::Forward, IterSourceKind::Set) => &RT_ITER_SET,
            (IterDirection::Forward, IterSourceKind::Bytes) => &RT_ITER_BYTES,
            (IterDirection::Forward, IterSourceKind::Generator) => &RT_ITER_GENERATOR,
            (IterDirection::Reversed, IterSourceKind::List) => &RT_ITER_REVERSED_LIST,
            (IterDirection::Reversed, IterSourceKind::Tuple) => &RT_ITER_REVERSED_TUPLE,
            (IterDirection::Reversed, IterSourceKind::Dict) => &RT_ITER_REVERSED_DICT,
            (IterDirection::Reversed, IterSourceKind::Str) => &RT_ITER_REVERSED_STR,
            (IterDirection::Reversed, IterSourceKind::Range) => &RT_ITER_REVERSED_RANGE,
            (IterDirection::Reversed, IterSourceKind::Set) => &RT_ITER_REVERSED_SET,
            (IterDirection::Reversed, IterSourceKind::Bytes) => &RT_ITER_REVERSED_BYTES,
            (IterDirection::Reversed, IterSourceKind::Generator) => &RT_ITER_REVERSED_GENERATOR,
        }
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

    /// Get the static RuntimeFuncDef for sorted() on this source.
    /// - `has_key=false`: returns the no-key variant (Range uses special 4-arg version,
    ///   Set/Dict use 3-arg version with elem_tag, others use 2-arg version)
    /// - `has_key=true`: returns the with-key variant (6-arg version)
    pub fn sorted_def(&self, has_key: bool) -> &'static pyaot_core_defs::RuntimeFuncDef {
        use pyaot_core_defs::runtime_func_def::*;
        if has_key {
            match self {
                SortableKind::List => &RT_SORTED_LIST_WITH_KEY,
                SortableKind::Tuple => &RT_SORTED_TUPLE_WITH_KEY,
                SortableKind::Str => &RT_SORTED_STR_WITH_KEY,
                SortableKind::Set => &RT_SORTED_SET_WITH_KEY,
                SortableKind::Dict => &RT_SORTED_DICT_WITH_KEY,
                SortableKind::Range => unreachable!("sorted() with key is not supported for Range"),
            }
        } else {
            match self {
                SortableKind::List => &RT_SORTED_LIST,
                SortableKind::Tuple => &RT_SORTED_TUPLE,
                SortableKind::Str => &RT_SORTED_STR,
                SortableKind::Set => &RT_SORTED_SET,
                SortableKind::Dict => &RT_SORTED_DICT,
                SortableKind::Range => &RT_SORTED_RANGE,
            }
        }
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

    /// Numeric tag for passing to generic runtime comparison functions.
    /// Lt=0, Lte=1, Gt=2, Gte=3. Eq is not used with generic cmp functions.
    pub fn to_tag(&self) -> u8 {
        match self {
            ComparisonOp::Lt => 0,
            ComparisonOp::Lte => 1,
            ComparisonOp::Gt => 2,
            ComparisonOp::Gte => 3,
            ComparisonOp::Eq => unreachable!("Eq does not use generic cmp dispatch"),
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
    /// Get the static RuntimeFuncDef for this comparison kind and operation.
    pub fn runtime_func_def(&self, op: ComparisonOp) -> &'static pyaot_core_defs::RuntimeFuncDef {
        use pyaot_core_defs::runtime_func_def::*;
        match (self, op) {
            (CompareKind::ListInt, _) => &RT_CMP_LIST_INT_EQ,
            (CompareKind::ListFloat, _) => &RT_CMP_LIST_FLOAT_EQ,
            (CompareKind::ListStr, _) => &RT_CMP_LIST_STR_EQ,
            (CompareKind::List, ComparisonOp::Eq) => &RT_CMP_LIST_EQ,
            (CompareKind::List, _) => &RT_CMP_LIST_ORD,
            (CompareKind::Str, _) => &RT_CMP_STR_EQ,
            (CompareKind::Bytes, _) => &RT_CMP_BYTES_EQ,
            (CompareKind::Tuple, ComparisonOp::Eq) => &RT_CMP_TUPLE_EQ,
            (CompareKind::Tuple, _) => &RT_CMP_TUPLE_ORD,
            (CompareKind::Obj, ComparisonOp::Eq) => &RT_CMP_OBJ_EQ,
            (CompareKind::Obj, _) => &RT_CMP_OBJ_ORD,
        }
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
    /// Bytes object pointer
    Bytes,
    /// Collection (list, tuple, dict, set) or generic object - runtime type-dispatched
    Collection,
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
            ReprTargetKind::Bytes => "bytes",
            ReprTargetKind::Collection => "collection",
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

    /// Get the static RuntimeFuncDef for this repr/ascii target kind and format.
    pub fn runtime_func_def(
        &self,
        format: StringFormat,
    ) -> &'static pyaot_core_defs::RuntimeFuncDef {
        use pyaot_core_defs::runtime_func_def::*;
        match (format, self) {
            (StringFormat::Repr, ReprTargetKind::Int) => &RT_REPR_INT,
            (StringFormat::Repr, ReprTargetKind::Float) => &RT_REPR_FLOAT,
            (StringFormat::Repr, ReprTargetKind::Bool) => &RT_REPR_BOOL,
            (StringFormat::Repr, ReprTargetKind::None) => &RT_REPR_NONE,
            (StringFormat::Repr, ReprTargetKind::Str) => &RT_REPR_STR,
            (StringFormat::Repr, ReprTargetKind::Bytes) => &RT_REPR_BYTES,
            (StringFormat::Repr, ReprTargetKind::Collection) => &RT_REPR_COLLECTION,
            (StringFormat::Ascii, ReprTargetKind::Int) => &RT_ASCII_INT,
            (StringFormat::Ascii, ReprTargetKind::Float) => &RT_ASCII_FLOAT,
            (StringFormat::Ascii, ReprTargetKind::Bool) => &RT_ASCII_BOOL,
            (StringFormat::Ascii, ReprTargetKind::None) => &RT_ASCII_NONE,
            (StringFormat::Ascii, ReprTargetKind::Str) => &RT_ASCII_STR,
            (StringFormat::Ascii, ReprTargetKind::Bytes) => &RT_ASCII_BYTES,
            (StringFormat::Ascii, ReprTargetKind::Collection) => &RT_ASCII_COLLECTION,
        }
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

    /// Get the static RuntimeFuncDef for this conversion (from, to) pair.
    pub fn convert_def(
        from: ConversionTypeKind,
        to: ConversionTypeKind,
    ) -> &'static pyaot_core_defs::RuntimeFuncDef {
        use pyaot_core_defs::runtime_func_def::*;
        match (from, to) {
            (ConversionTypeKind::Int, ConversionTypeKind::Str) => &RT_INT_TO_STR,
            (ConversionTypeKind::Float, ConversionTypeKind::Str) => &RT_FLOAT_TO_STR,
            (ConversionTypeKind::Bool, ConversionTypeKind::Str) => &RT_BOOL_TO_STR,
            (ConversionTypeKind::None, ConversionTypeKind::Str) => &RT_NONE_TO_STR,
            (ConversionTypeKind::Str, ConversionTypeKind::Int) => &RT_STR_TO_INT,
            (ConversionTypeKind::Str, ConversionTypeKind::Float) => &RT_STR_TO_FLOAT,
            _ => unreachable!(
                "Unsupported conversion: {:?} -> {:?}",
                from.name(),
                to.name()
            ),
        }
    }
}

/// Search operation kind for string/bytes search methods
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SearchOp {
    /// find() - first occurrence, returns -1 if not found
    Find,
    /// rfind() - last occurrence, returns -1 if not found
    Rfind,
    /// index() - first occurrence, raises ValueError if not found
    Index,
    /// rindex() - last occurrence, raises ValueError if not found
    Rindex,
}

impl SearchOp {
    /// Runtime function suffix: "_find", "_rfind", "_index", "_rindex"
    pub fn suffix(&self) -> &'static str {
        match self {
            SearchOp::Find => "_find",
            SearchOp::Rfind => "_rfind",
            SearchOp::Index => "_index",
            SearchOp::Rindex => "_rindex",
        }
    }

    /// Numeric tag for passing to generic runtime search functions.
    /// Find=0, Rfind=1, Index=2, Rindex=3
    pub fn to_tag(&self) -> u8 {
        match self {
            SearchOp::Find => 0,
            SearchOp::Rfind => 1,
            SearchOp::Index => 2,
            SearchOp::Rindex => 3,
        }
    }
}

/// Element type kind for typed collection element access
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GetElementKind {
    /// Integer elements - returns i64
    Int,
    /// Float elements - returns f64
    Float,
    /// Bool elements - returns i8
    Bool,
}

impl GetElementKind {
    /// Runtime function suffix: "_int", "_float", "_bool"
    pub fn suffix(&self) -> &'static str {
        match self {
            GetElementKind::Int => "_int",
            GetElementKind::Float => "_float",
            GetElementKind::Bool => "_bool",
        }
    }

    /// Numeric tag for passing to rt_list_get_typed.
    /// Int=0, Float=1, Bool=2.
    pub fn to_tag(&self) -> u8 {
        match self {
            GetElementKind::Int => 0,
            GetElementKind::Float => 1,
            GetElementKind::Bool => 2,
        }
    }
}
