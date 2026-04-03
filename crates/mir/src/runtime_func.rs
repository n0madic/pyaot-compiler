//! Runtime function definitions for MIR

use crate::{
    CompareKind, ComparisonOp, ContainerKind, ConversionTypeKind, ElementKind, IterDirection,
    IterSourceKind, MinMaxOp, PrintKind, ReprTargetKind, SearchOp, SortableKind, StringFormat,
    ValueKind,
};
use pyaot_core_defs::RuntimeFuncDef;
use pyaot_stdlib_defs::{StdlibAttrDef, StdlibFunctionDef, StdlibMethodDef};

/// Runtime functions
#[derive(Debug, Clone, Copy)]
pub enum RuntimeFunc {
    // ==================== Descriptor-based call (generic) ====================
    /// Call a runtime function described by a static descriptor.
    /// The generic codegen handler builds the Cranelift signature, loads args,
    /// emits the call, and handles GC root tracking — all from the descriptor.
    Call(&'static RuntimeFuncDef),

    // ==================== Stdlib calls (generic) ====================
    /// Call a stdlib function by definition (Single Source of Truth)
    /// Codegen uses func_def.runtime_name for the function name
    /// and func_def.return_type + func_def.params for signature
    StdlibCall(&'static StdlibFunctionDef),

    /// Get a stdlib attribute by definition (Single Source of Truth)
    /// Codegen uses attr_def.runtime_getter for the function name
    StdlibAttrGet(&'static StdlibAttrDef),

    /// Get an object field by definition (Single Source of Truth)
    /// Codegen uses field_def.runtime_getter for the function name
    /// This replaces individual field getter variants (e.g., StructTimeGetTmYear)
    ObjectFieldGet(&'static pyaot_stdlib_defs::ObjectFieldDef),

    /// Call an object method by definition (Single Source of Truth)
    /// Codegen uses method_def.runtime_name for the function name
    /// This replaces individual method variants (e.g., MatchGroup, MatchStart)
    ObjectMethodCall(&'static StdlibMethodDef),

    // ==================== String operations ====================
    /// Allocate string on heap (takes data pointer and length)
    MakeStr,
    /// Get string data pointer
    StrData,
    /// Get string length
    StrLen,
    /// Get string length as int (for len() builtin)
    StrLenInt,
    /// Allocate bytes from compile-time constant data (embeds data in binary)
    MakeBytes,
    // List, Tuple, Dict: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs
    // Boxing/Unboxing: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_BOX_INT, RT_BOX_BOOL, RT_BOX_FLOAT, RT_BOX_NONE,
    //   RT_UNBOX_INT, RT_UNBOX_FLOAT, RT_UNBOX_BOOL
    /// String concatenation (str + str)
    StrConcat,
    /// String slicing (str[start:end])
    StrSlice,
    /// String slicing with step (str[start:end:step])
    StrSliceStep,
    /// Get character at byte index (internal, used by loop iteration)
    StrGetChar,
    /// Python subscript s[char_index]: char_index is a codepoint index (may be negative)
    StrSubscript,
    /// String multiplication (str * int)
    StrMul,
    /// String upper()
    StrUpper,
    /// String lower()
    StrLower,
    /// String strip()
    StrStrip,
    /// String startswith()
    StrStartsWith,
    /// String endswith()
    StrEndsWith,
    /// String search: find/rfind/index/rindex with operation tag
    StrSearch(SearchOp),
    /// String rsplit()
    StrRsplit,
    /// String isascii check
    StrIsAscii,
    /// String encode to bytes
    StrEncode,
    /// String replace()
    StrReplace,
    /// Print a value with specific kind (no newline)
    /// - Int: i64 value
    /// - Float: f64 value
    /// - Bool: i8 value
    /// - None: no argument
    /// - Str: raw string pointer
    /// - StrObj: heap-allocated string object
    /// - BytesObj: bytes object
    /// - Obj: generic heap object with runtime dispatch
    PrintValue(PrintKind),
    /// Print newline (end='\n')
    PrintNewline,
    /// Print separator (sep=' ')
    PrintSep,
    /// Redirect print to stderr: rt_print_set_stderr()
    PrintSetStderr,
    /// Redirect print back to stdout: rt_print_set_stdout()
    PrintSetStdout,
    /// Flush output stream: rt_print_flush()
    PrintFlush,
    /// Assertion failure (takes optional message pointer)
    AssertFail,
    /// Assertion failure with string object message
    AssertFailObj,

    // ==================== Type conversion runtime functions ====================
    /// Unified type conversion: rt_{from}_to_{to}
    /// Supports: int→str, float→str, bool→str, none→str, str→int, str→float
    Convert {
        from: ConversionTypeKind,
        to: ConversionTypeKind,
    },
    /// Convert string to int with base: rt_str_to_int_with_base(s: *mut Obj, base: i64) -> i64
    StrToIntWithBase,
    /// String contains (substring check): rt_str_contains(needle, haystack) -> bool
    StrContains,

    // ==================== Math runtime functions ====================
    /// Power function for floats: pow(base, exp) -> f64
    PowFloat,
    /// Power function for integers: base ** exp -> i64
    PowInt,
    /// Round to integer: round(x) -> i64
    RoundToInt,
    /// Round to N digits: round(x, ndigits) -> f64
    RoundToDigits,
    // ==================== Character/Code runtime functions ====================
    /// Convert int to character: chr(i) -> str
    IntToChr,
    /// Convert character to int: ord(s) -> i64
    ChrToInt,

    // ==================== Exception handling runtime functions ====================
    /// Push exception frame: rt_exc_push_frame(frame_ptr)
    ExcPushFrame,
    /// Pop exception frame: rt_exc_pop_frame()
    ExcPopFrame,
    /// Call setjmp on frame: rt_exc_setjmp(frame_ptr) -> i32
    ExcSetjmp,
    /// Raise exception: rt_exc_raise(exc_type, msg_ptr, msg_len) -> !
    ExcRaise,
    /// Re-raise current exception: rt_exc_reraise() -> !
    ExcReraise,
    /// Get exception type: rt_exc_get_type() -> i32
    ExcGetType,
    /// Clear exception: rt_exc_clear()
    ExcClear,
    /// Check if exception pending: rt_exc_has_exception() -> i8
    ExcHasException,
    /// Get current exception as string: rt_exc_get_current() -> *mut Obj
    ExcGetCurrent,
    /// Check if exception matches type: rt_exc_isinstance(type_tag: u8) -> i8
    ExcIsinstance,
    /// Check if exception matches class (with inheritance): rt_exc_isinstance_class(class_id: u8) -> i8
    ExcIsinstanceClass,
    /// Raise custom exception: rt_exc_raise_custom(class_id: u8, msg_ptr: *const u8, msg_len: usize) -> !
    ExcRaiseCustom,
    /// Register exception class name: rt_exc_register_class_name(class_id: u8, name: *const u8, len: usize)
    ExcRegisterClassName,
    /// Convert exception instance to string: rt_exc_instance_str(instance: *mut Obj) -> *mut Obj
    ExcInstanceStr,

    // ==================== Instance (class) runtime functions ====================
    /// Create instance: rt_make_instance(class_id, field_count) -> *mut Obj
    MakeInstance,
    /// Get field: rt_instance_get_field(inst, offset) -> *mut Obj
    InstanceGetField,
    /// Set field: rt_instance_set_field(inst, offset, value)
    InstanceSetField,

    // ==================== Type checking runtime functions ====================
    /// Get type tag of heap object: rt_get_type_tag(obj) -> u8
    GetTypeTag,
    /// Check isinstance for class: rt_isinstance_class(obj, class_id) -> i8
    IsinstanceClass,
    /// Check isinstance for class with inheritance: rt_isinstance_class_inherited(obj, class_id) -> i8
    /// Walks the parent chain to check if obj is an instance of class_id or any of its subclasses
    IsinstanceClassInherited,
    /// Register a class with its parent for inheritance: rt_register_class(class_id, parent_class_id)
    RegisterClass,
    /// Register which class fields are heap objects for GC: rt_register_class_fields(class_id, heap_field_mask)
    RegisterClassFields,
    /// Register field count for object.__new__: rt_register_class_field_count(class_id, field_count)
    RegisterClassFieldCount,
    /// Create instance by class_id (object.__new__): rt_object_new(class_id) -> *mut Obj
    ObjectNew,
    /// Register __del__ function pointer: rt_register_del_func(class_id, func_ptr)
    RegisterDelFunc,
    /// Register __copy__ function pointer: rt_register_copy_func(class_id, func_ptr)
    RegisterCopyFunc,
    /// Register __deepcopy__ function pointer: rt_register_deepcopy_func(class_id, func_ptr)
    RegisterDeepCopyFunc,
    /// Check issubclass: rt_issubclass(child: u8, parent: u8) -> i8
    IsSubclass,

    // Hash + Id: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_HASH_INT, RT_HASH_STR, RT_HASH_BOOL, RT_HASH_TUPLE, RT_ID_OBJ

    // ==================== Iterator runtime functions ====================
    /// Create iterator from source with direction (unified variant)
    /// - For Range: rt_iter_{reversed_}range(start: i64, stop: i64, step: i64) -> *mut Obj
    /// - For others: rt_iter_{reversed_}{source}(container: *mut Obj) -> *mut Obj
    MakeIterator {
        source: IterSourceKind,
        direction: IterDirection,
    },
    /// Get next element from iterator (raises StopIteration when exhausted): rt_iter_next(iter: *mut Obj) -> *mut Obj
    IterNext,
    /// Get next element from iterator (no exception, for for-loops): rt_iter_next_no_exc(iter: *mut Obj) -> *mut Obj
    IterNextNoExc,
    /// Check if iterator/generator is exhausted: rt_iter_is_exhausted(iter: *mut Obj) -> i8
    IterIsExhausted,
    /// Create enumerate iterator: rt_iter_enumerate(inner: *mut Obj, start: i64) -> *mut Obj
    IterEnumerate,

    // ==================== Sorted runtime functions (unified) ====================
    /// Create sorted list from a container
    /// - Without key: rt_sorted_{source}(container, reverse) -> *mut Obj
    /// - With key: rt_sorted_{source}_with_key(container, reverse, key_fn) -> *mut Obj
    /// - Range (no key): rt_sorted_range(start, stop, step, reverse) -> *mut Obj
    Sorted { source: SortableKind, has_key: bool },

    // ==================== Container min/max operations (unified) ====================
    /// Find min/max element in a container
    /// - For Int/Float: rt_{container}_{op}_{elem}(container) -> i64/f64
    /// - For WithKey: rt_{container}_{op}_with_key(container, key_fn) -> *mut Obj
    ContainerMinMax {
        container: ContainerKind,
        op: MinMaxOp,
        elem: ElementKind,
    },

    // Set ops: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_MAKE_SET, RT_SET_ADD, RT_SET_CONTAINS,
    //   RT_SET_REMOVE, RT_SET_DISCARD, RT_SET_LEN, RT_SET_CLEAR, RT_SET_COPY, RT_SET_TO_LIST,
    //   RT_SET_UNION, RT_SET_INTERSECTION, RT_SET_DIFFERENCE, RT_SET_SYMMETRIC_DIFFERENCE,
    //   RT_SET_ISSUBSET, RT_SET_ISSUPERSET, RT_SET_ISDISJOINT, RT_SET_POP, RT_SET_UPDATE,
    //   RT_SET_INTERSECTION_UPDATE, RT_SET_DIFFERENCE_UPDATE, RT_SET_SYMMETRIC_DIFFERENCE_UPDATE

    // Bytes: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs

    // ==================== Comparison operations (unified) ====================
    /// Compare two containers or objects
    /// Replaces: ListEqInt, ListEqFloat, ListEqStr, TupleEq, TupleLt, TupleLte, TupleGt, TupleGte,
    /// StrEq, BytesEq, ObjEq, ObjLt, ObjLte, ObjGt, ObjGte (15 variants → 1 parameterized variant)
    Compare { kind: CompareKind, op: ComparisonOp },

    /// Register method name→slot mapping for a class (Protocol dispatch): rt_register_method_name(class_id, name_hash, slot)
    RegisterMethodName,

    // Object ops: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_IS_TRUTHY, RT_OBJ_CONTAINS, RT_OBJ_TO_STR,
    //   RT_OBJ_DEFAULT_REPR, RT_OBJ_ADD, RT_OBJ_SUB, RT_OBJ_MUL, RT_OBJ_DIV, RT_OBJ_FLOORDIV,
    //   RT_OBJ_MOD, RT_OBJ_POW, RT_ANY_GETITEM

    // ==================== Global variable storage ====================
    /// Get global: rt_global_get_{int,float,bool,ptr}(var_id) -> value
    GlobalGet(ValueKind),
    /// Set global: rt_global_set_{int,float,bool,ptr}(var_id, value)
    GlobalSet(ValueKind),

    // ==================== Class attribute storage ====================
    /// Get class attr: rt_class_attr_get_{int,float,bool,ptr}(class_id, attr_idx) -> value
    ClassAttrGet(ValueKind),
    /// Set class attr: rt_class_attr_set_{int,float,bool,ptr}(class_id, attr_idx, value)
    ClassAttrSet(ValueKind),

    // ==================== Cell functions for nonlocal support ====================
    /// Create cell: rt_make_cell_{int,float,bool,ptr}(value) -> *mut Obj
    MakeCell(ValueKind),
    /// Get from cell: rt_cell_get_{int,float,bool,ptr}(cell) -> value
    CellGet(ValueKind),
    /// Set in cell: rt_cell_set_{int,float,bool,ptr}(cell, value)
    CellSet(ValueKind),

    // ==================== Generator runtime functions ====================
    /// Create generator object: rt_make_generator(func_id: u32, state_size: usize) -> *mut Obj
    MakeGenerator,
    /// Get generator state: rt_generator_get_state(gen: *mut Obj) -> u32
    GeneratorGetState,
    /// Set generator state: rt_generator_set_state(gen: *mut Obj, state: u32)
    GeneratorSetState,
    /// Get generator local by index: rt_generator_get_local(gen: *mut Obj, index: u32) -> i64
    GeneratorGetLocal,
    /// Set generator local by index: rt_generator_set_local(gen: *mut Obj, index: u32, value: i64)
    GeneratorSetLocal,
    /// Get generator local pointer by index: rt_generator_get_local_ptr(gen: *mut Obj, index: u32) -> *mut Obj
    GeneratorGetLocalPtr,
    /// Set generator local pointer by index: rt_generator_set_local_ptr(gen: *mut Obj, index: u32, value: *mut Obj)
    GeneratorSetLocalPtr,
    /// Set generator local type tag: rt_generator_set_local_type(gen: *mut Obj, index: u32, type_tag: u8)
    GeneratorSetLocalType,
    /// Mark generator as exhausted: rt_generator_set_exhausted(gen: *mut Obj)
    GeneratorSetExhausted,
    /// Check if generator is exhausted: rt_generator_is_exhausted(gen: *mut Obj) -> i8
    GeneratorIsExhausted,
    /// Send value to generator: rt_generator_send(gen: *mut Obj, value: i64) -> *mut Obj
    GeneratorSend,
    /// Get sent value from generator: rt_generator_get_sent_value(gen: *mut Obj) -> i64
    GeneratorGetSentValue,
    /// Close generator: rt_generator_close(gen: *mut Obj)
    GeneratorClose,
    /// Check if generator is closing: rt_generator_is_closing(gen: *mut Obj) -> i8
    GeneratorIsClosing,

    // File I/O: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_FILE_OPEN, RT_FILE_READ, RT_FILE_READ_N,
    //   RT_FILE_READLINE, RT_FILE_READLINES, RT_FILE_WRITE, RT_FILE_CLOSE, RT_FILE_FLUSH,
    //   RT_FILE_ENTER, RT_FILE_EXIT, RT_FILE_IS_CLOSED, RT_FILE_NAME

    // ==================== New builtin functions ====================
    /// Read line from stdin: rt_input(prompt: *mut Obj) -> *mut Obj (str)
    Input,
    /// Integer to binary string: rt_int_to_bin(n: i64) -> *mut Obj (str)
    IntToBin,
    /// Integer to hex string: rt_int_to_hex(n: i64) -> *mut Obj (str)
    IntToHex,
    /// Integer to octal string: rt_int_to_oct(n: i64) -> *mut Obj (str)
    IntToOct,
    /// Format integer as binary without prefix: rt_int_fmt_bin(n: i64) -> *mut Obj (str)
    IntFmtBin,
    /// Format integer as lowercase hex without prefix: rt_int_fmt_hex(n: i64) -> *mut Obj (str)
    IntFmtHex,
    /// Format integer as uppercase hex without prefix: rt_int_fmt_hex_upper(n: i64) -> *mut Obj (str)
    IntFmtHexUpper,
    /// Format integer as octal without prefix: rt_int_fmt_oct(n: i64) -> *mut Obj (str)
    IntFmtOct,
    /// Format integer with grouping separator: rt_int_fmt_grouped(n: i64, sep: i64) -> *mut Obj (str)
    IntFmtGrouped,
    /// Format float with precision and grouping: rt_float_fmt_grouped(f: f64, precision: i64, sep: i64) -> *mut Obj (str)
    FloatFmtGrouped,
    /// String representation: rt_repr_* or rt_ascii_* (obj) -> *mut Obj (str)
    ///
    /// - Repr: Standard repr() - show string representation
    /// - Ascii: Like repr but escapes non-ASCII characters
    ///
    /// Target kind determines the runtime function suffix and argument type
    ToStringRepr(ReprTargetKind, StringFormat),

    // Dict additional, DefaultDict, Counter, Deque: migrated to RuntimeFunc::Call(&RuntimeFuncDef)

    // ==================== String methods ====================
    /// String count: rt_str_count(s: *mut Obj, sub: *mut Obj) -> i64
    StrCount,
    /// String split: rt_str_split(s: *mut Obj, sep: *mut Obj, maxsplit: i64) -> *mut Obj (list)
    StrSplit,
    /// String join: rt_str_join(sep: *mut Obj, list: *mut Obj) -> *mut Obj (str)
    StrJoin,
    /// String lstrip: rt_str_lstrip(s: *mut Obj, chars: *mut Obj) -> *mut Obj (str)
    StrLstrip,
    /// String rstrip: rt_str_rstrip(s: *mut Obj, chars: *mut Obj) -> *mut Obj (str)
    StrRstrip,
    /// String title: rt_str_title(s: *mut Obj) -> *mut Obj (str)
    StrTitle,
    /// String capitalize: rt_str_capitalize(s: *mut Obj) -> *mut Obj (str)
    StrCapitalize,
    /// String swapcase: rt_str_swapcase(s: *mut Obj) -> *mut Obj (str)
    StrSwapcase,
    /// String center: rt_str_center(s: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj (str)
    StrCenter,
    /// String ljust: rt_str_ljust(s: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj (str)
    StrLjust,
    /// String rjust: rt_str_rjust(s: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj (str)
    StrRjust,
    /// String zfill: rt_str_zfill(s: *mut Obj, width: i64) -> *mut Obj (str)
    StrZfill,
    /// String isdigit: rt_str_isdigit(s: *mut Obj) -> i8
    StrIsDigit,
    /// String isalpha: rt_str_isalpha(s: *mut Obj) -> i8
    StrIsAlpha,
    /// String isalnum: rt_str_isalnum(s: *mut Obj) -> i8
    StrIsAlnum,
    /// String isspace: rt_str_isspace(s: *mut Obj) -> i8
    StrIsSpace,
    /// String isupper: rt_str_isupper(s: *mut Obj) -> i8
    StrIsUpper,
    /// String islower: rt_str_islower(s: *mut Obj) -> i8
    StrIsLower,
    /// String removeprefix: rt_str_removeprefix(s: *mut Obj, prefix: *mut Obj) -> *mut Obj
    StrRemovePrefix,
    /// String removesuffix: rt_str_removesuffix(s: *mut Obj, suffix: *mut Obj) -> *mut Obj
    StrRemoveSuffix,
    /// String splitlines: rt_str_splitlines(s: *mut Obj) -> *mut Obj (list[str])
    StrSplitLines,
    /// String partition: rt_str_partition(s: *mut Obj, sep: *mut Obj) -> *mut Obj (tuple)
    StrPartition,
    /// String rpartition: rt_str_rpartition(s: *mut Obj, sep: *mut Obj) -> *mut Obj (tuple)
    StrRpartition,
    /// String expandtabs: rt_str_expandtabs(s: *mut Obj, tabsize: i64) -> *mut Obj
    StrExpandTabs,

    // ListSort, ListSortWithKey: migrated to RuntimeFunc::Call(&RuntimeFuncDef)

    // ==================== Zip iterator ====================
    /// Create zip iterator: rt_zip_new(iter1: *mut Obj, iter2: *mut Obj) -> *mut Obj
    ZipNew,
    /// Create zip iterator with 3 iterables: rt_zip3_new(iter1: *mut Obj, iter2: *mut Obj, iter3: *mut Obj) -> *mut Obj
    Zip3New,
    /// Create zip iterator with N iterables: rt_zipn_new(iters: *mut Obj, num_iters: i64) -> *mut Obj
    ZipNNew,
    /// Get next from zip: rt_zip_next(zip: *mut Obj) -> *mut Obj (tuple or null)
    ZipNext,
    /// Create iterator from zip: rt_iter_zip(zip: *mut Obj) -> *mut Obj
    IterZip,

    // ==================== Introspection ====================
    /// Get type name: rt_type_name(obj: *mut Obj) -> *mut Obj (str)
    /// Returns full type representation like "<class 'int'>"
    TypeName,
    /// Extract type name from type string: rt_type_name_extract(type_str: *mut Obj) -> *mut Obj (str)
    /// Extracts "int" from "<class 'int'>" for __name__ attribute access
    TypeNameExtract,
    /// Get exception class name: rt_exc_class_name(instance: *mut Obj) -> *mut Obj (str)
    /// Returns "<class 'ValueError'>" etc. for __class__ attribute on exceptions
    ExcClassName,

    // ==================== Map/Filter iterators ====================
    /// Create map iterator: rt_map_new(func_ptr: i64, iter: *mut Obj) -> *mut Obj
    MapNew,
    /// Create filter iterator: rt_filter_new(func_ptr: i64, iter: *mut Obj, elem_tag: i64) -> *mut Obj
    /// func_ptr=0 for truthiness filtering (filter(None, ...)), elem_tag for raw value handling
    FilterNew,

    // Collection constructors (List/Tuple/Dict): migrated to RuntimeFunc::Call(&RuntimeFuncDef)

    // ==================== StringBuilder for efficient string concatenation ====================
    /// Create StringBuilder: rt_make_string_builder(capacity: i64) -> *mut Obj
    MakeStringBuilder,
    /// Append to StringBuilder: rt_string_builder_append(builder: *mut Obj, str: *mut Obj)
    StringBuilderAppend,
    /// Finalize StringBuilder to string: rt_string_builder_to_str(builder: *mut Obj) -> *mut Obj
    StringBuilderToStr,

    // ==================== Format builtin ====================
    /// Format a value with a format spec: rt_format_value(value: *mut Obj, spec: *mut Obj) -> *mut Obj
    FormatValue,

    // ==================== Reduce (functools) ====================
    /// Reduce an iterable: rt_reduce(func_ptr: i64, iter: *mut Obj, initial: *mut Obj, captures: *mut Obj, capture_count: i64) -> *mut Obj
    ReduceNew,

    // ==================== itertools ====================
    /// Create chain iterator: rt_chain_new(iters: *mut Obj, num_iters: i64) -> *mut Obj
    ChainNew,
    /// Create islice iterator: rt_islice_new(iter: *mut Obj, start: i64, stop: i64, step: i64) -> *mut Obj
    ISliceNew,

    // ==================== Generic subscript ====================
    /// Runtime-dispatched subscript for Any-typed objects:
    /// rt_any_getitem(obj: *mut Obj, index: i64) -> *mut Obj
    AnyGetItem,
}
