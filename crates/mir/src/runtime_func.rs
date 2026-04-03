//! Runtime function definitions for MIR

use crate::PrintKind;
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
    /// Allocate bytes from compile-time constant data (embeds data in binary)
    MakeBytes,
    // String ops: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_STR_DATA, RT_STR_LEN, RT_STR_LEN_INT,
    //   RT_STR_CONCAT, RT_STR_SLICE, RT_STR_SLICE_STEP, RT_STR_GETCHAR, RT_STR_SUBSCRIPT,
    //   RT_STR_MUL, RT_STR_UPPER, RT_STR_LOWER, RT_STR_STRIP, RT_STR_STARTSWITH, RT_STR_ENDSWITH,
    //   RT_STR_SEARCH, RT_STR_RSPLIT, RT_STR_ISASCII, RT_STR_ENCODE, RT_STR_REPLACE,
    //   RT_STR_COUNT, RT_STR_SPLIT, RT_STR_JOIN, RT_STR_LSTRIP, RT_STR_RSTRIP, RT_STR_TITLE,
    //   RT_STR_CAPITALIZE, RT_STR_SWAPCASE, RT_STR_CENTER, RT_STR_LJUST, RT_STR_RJUST,
    //   RT_STR_ZFILL, RT_STR_ISDIGIT, RT_STR_ISALPHA, RT_STR_ISALNUM, RT_STR_ISSPACE,
    //   RT_STR_ISUPPER, RT_STR_ISLOWER, RT_STR_REMOVEPREFIX, RT_STR_REMOVESUFFIX,
    //   RT_STR_SPLITLINES, RT_STR_PARTITION, RT_STR_RPARTITION, RT_STR_EXPANDTABS,
    //   RT_MAKE_STRING_BUILDER, RT_STRING_BUILDER_APPEND, RT_STRING_BUILDER_TO_STR

    // List, Tuple, Dict: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // Boxing/Unboxing: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    /// Print a value with specific kind (no newline)
    /// Only for special cases that embed string constants:
    /// - Str: embeds a null-terminated C string in the binary
    /// - None: prints the literal "None"
    PrintValue(PrintKind),
    /// Assertion failure (takes optional message pointer, embeds null-terminated string in binary)
    AssertFail,

    // Print ops: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_PRINT_NEWLINE, RT_PRINT_SEP, RT_PRINT_FLUSH,
    //   RT_PRINT_SET_STDERR, RT_PRINT_SET_STDOUT, RT_INPUT, RT_PRINT_INT, RT_PRINT_FLOAT,
    //   RT_PRINT_BOOL, RT_PRINT_STR_OBJ, RT_PRINT_BYTES_OBJ, RT_PRINT_OBJ, RT_ASSERT_FAIL_OBJ

    // Type conversions (Convert, StrToIntWithBase, StrContains): migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // Math (PowFloat, PowInt, RoundToInt, RoundToDigits, IntToChr, ChrToInt): migrated to RuntimeFunc::Call(&RuntimeFuncDef)

    // ==================== Exception handling runtime functions ====================
    // ExcPushFrame and ExcPopFrame are handled as InstructionKind, not RuntimeFunc.
    // ExcIsinstance is unused (dead code, removed).
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
    /// Check if exception matches class (with inheritance): rt_exc_isinstance_class(class_id: u8) -> i8
    ExcIsinstanceClass,
    /// Raise custom exception: rt_exc_raise_custom(class_id: u8, msg_ptr: *const u8, msg_len: usize) -> !
    ExcRaiseCustom,
    /// Register exception class name: rt_exc_register_class_name(class_id: u8, name: *const u8, len: usize)
    ExcRegisterClassName,
    /// Convert exception instance to string: rt_exc_instance_str(instance: *mut Obj) -> *mut Obj
    ExcInstanceStr,
    // Instance (class) ops: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_MAKE_INSTANCE, RT_INSTANCE_GET_FIELD,
    //   RT_INSTANCE_SET_FIELD, RT_GET_TYPE_TAG, RT_ISINSTANCE_CLASS,
    //   RT_ISINSTANCE_CLASS_INHERITED, RT_REGISTER_CLASS, RT_REGISTER_CLASS_FIELDS,
    //   RT_REGISTER_CLASS_FIELD_COUNT, RT_REGISTER_METHOD_NAME, RT_OBJECT_NEW,
    //   RT_REGISTER_DEL_FUNC, RT_REGISTER_COPY_FUNC, RT_REGISTER_DEEPCOPY_FUNC, RT_ISSUBCLASS

    // Hash + Id: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_HASH_INT, RT_HASH_STR, RT_HASH_BOOL, RT_HASH_TUPLE, RT_ID_OBJ

    // Iterator ops: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_ITER_LIST, RT_ITER_TUPLE, RT_ITER_DICT,
    //   RT_ITER_STR, RT_ITER_RANGE, RT_ITER_SET, RT_ITER_BYTES, RT_ITER_GENERATOR,
    //   RT_ITER_REVERSED_*, RT_ITER_NEXT, RT_ITER_NEXT_NO_EXC, RT_ITER_IS_EXHAUSTED,
    //   RT_ITER_ENUMERATE, RT_SORTED_*, RT_ZIP_NEW, RT_ZIP3_NEW, RT_ZIPN_NEW, RT_ZIP_NEXT,
    //   RT_ITER_ZIP, RT_MAP_NEW, RT_FILTER_NEW, RT_REDUCE_NEW, RT_CHAIN_NEW, RT_ISLICE_NEW

    // Container min/max (ContainerMinMax): migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_LIST_MINMAX, RT_TUPLE_MINMAX, RT_SET_MINMAX,
    //   RT_DICT_MINMAX, RT_STR_MINMAX, RT_LIST_MINMAX_WITH_KEY, RT_TUPLE_MINMAX_WITH_KEY,
    //   RT_SET_MINMAX_WITH_KEY, RT_DICT_MINMAX_WITH_KEY, RT_STR_MINMAX_WITH_KEY

    // Set ops: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_MAKE_SET, RT_SET_ADD, RT_SET_CONTAINS,
    //   RT_SET_REMOVE, RT_SET_DISCARD, RT_SET_LEN, RT_SET_CLEAR, RT_SET_COPY, RT_SET_TO_LIST,
    //   RT_SET_UNION, RT_SET_INTERSECTION, RT_SET_DIFFERENCE, RT_SET_SYMMETRIC_DIFFERENCE,
    //   RT_SET_ISSUBSET, RT_SET_ISSUPERSET, RT_SET_ISDISJOINT, RT_SET_POP, RT_SET_UPDATE,
    //   RT_SET_INTERSECTION_UPDATE, RT_SET_DIFFERENCE_UPDATE, RT_SET_SYMMETRIC_DIFFERENCE_UPDATE

    // Bytes: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs

    // Comparison operations (Compare): migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_CMP_LIST_INT_EQ, RT_CMP_LIST_FLOAT_EQ,
    //   RT_CMP_LIST_STR_EQ, RT_CMP_LIST_EQ, RT_CMP_LIST_ORD, RT_CMP_TUPLE_EQ, RT_CMP_TUPLE_ORD,
    //   RT_CMP_STR_EQ, RT_CMP_BYTES_EQ, RT_CMP_OBJ_EQ, RT_CMP_OBJ_LT, RT_CMP_OBJ_LTE,
    //   RT_CMP_OBJ_GT, RT_CMP_OBJ_GTE

    // Object ops: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_IS_TRUTHY, RT_OBJ_CONTAINS, RT_OBJ_TO_STR,
    //   RT_OBJ_DEFAULT_REPR, RT_OBJ_ADD, RT_OBJ_SUB, RT_OBJ_MUL, RT_OBJ_DIV, RT_OBJ_FLOORDIV,
    //   RT_OBJ_MOD, RT_OBJ_POW, RT_ANY_GETITEM

    // Global variable storage: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_GLOBAL_GET_INT, RT_GLOBAL_GET_FLOAT,
    //   RT_GLOBAL_GET_BOOL, RT_GLOBAL_GET_PTR, RT_GLOBAL_SET_INT, RT_GLOBAL_SET_FLOAT,
    //   RT_GLOBAL_SET_BOOL, RT_GLOBAL_SET_PTR

    // Class attribute storage: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_CLASS_ATTR_GET_INT, RT_CLASS_ATTR_GET_FLOAT,
    //   RT_CLASS_ATTR_GET_BOOL, RT_CLASS_ATTR_GET_PTR, RT_CLASS_ATTR_SET_INT,
    //   RT_CLASS_ATTR_SET_FLOAT, RT_CLASS_ATTR_SET_BOOL, RT_CLASS_ATTR_SET_PTR

    // Cell functions for nonlocal support: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_MAKE_CELL_INT, RT_MAKE_CELL_FLOAT,
    //   RT_MAKE_CELL_BOOL, RT_MAKE_CELL_PTR, RT_CELL_GET_INT, RT_CELL_GET_FLOAT,
    //   RT_CELL_GET_BOOL, RT_CELL_GET_PTR, RT_CELL_SET_INT, RT_CELL_SET_FLOAT,
    //   RT_CELL_SET_BOOL, RT_CELL_SET_PTR

    // Generator ops: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_MAKE_GENERATOR, RT_GENERATOR_GET_STATE,
    //   RT_GENERATOR_SET_STATE, RT_GENERATOR_GET_LOCAL, RT_GENERATOR_SET_LOCAL,
    //   RT_GENERATOR_GET_LOCAL_PTR, RT_GENERATOR_SET_LOCAL_PTR, RT_GENERATOR_SET_LOCAL_TYPE,
    //   RT_GENERATOR_SET_EXHAUSTED, RT_GENERATOR_IS_EXHAUSTED, RT_GENERATOR_SEND,
    //   RT_GENERATOR_GET_SENT_VALUE, RT_GENERATOR_CLOSE, RT_GENERATOR_IS_CLOSING

    // File I/O: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_FILE_OPEN, RT_FILE_READ, RT_FILE_READ_N,
    //   RT_FILE_READLINE, RT_FILE_READLINES, RT_FILE_WRITE, RT_FILE_CLOSE, RT_FILE_FLUSH,
    //   RT_FILE_ENTER, RT_FILE_EXIT, RT_FILE_IS_CLOSED, RT_FILE_NAME

    // Input, Number formatting, Repr/Ascii: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // Dict additional, DefaultDict, Counter, Deque: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // ListSort, ListSortWithKey: migrated to RuntimeFunc::Call(&RuntimeFuncDef)

    // Introspection (TypeName, TypeNameExtract, ExcClassName): migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // See core-defs/src/runtime_func_def.rs: RT_TYPE_NAME, RT_TYPE_NAME_EXTRACT, RT_EXC_CLASS_NAME

    // Collection constructors (List/Tuple/Dict): migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // StringBuilder: migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // Format builtin (FormatValue): migrated to RuntimeFunc::Call(&RuntimeFuncDef)
    // Generic subscript (AnyGetItem): migrated to RuntimeFunc::Call(&RuntimeFuncDef)
}
