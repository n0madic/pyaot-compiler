//! Runtime function definitions for MIR

use crate::{
    CompareKind, ComparisonOp, ContainerKind, ConversionTypeKind, ElementKind, IterDirection,
    IterSourceKind, MinMaxOp, PrintKind, ReprTargetKind, SortableKind, StringFormat, ValueKind,
};
use pyaot_stdlib_defs::{StdlibAttrDef, StdlibFunctionDef, StdlibMethodDef};

/// Runtime functions
#[derive(Debug, Clone, Copy)]
pub enum RuntimeFunc {
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
    /// Allocate list
    MakeList,
    /// Allocate tuple
    MakeTuple,
    /// Allocate dict
    MakeDict,
    /// List append
    ListAppend,
    /// Set elem_tag on an empty list (for type-correct append on List(Any))
    ListSetElemTag,
    /// List set element
    ListSet,
    /// List get element
    ListGet,
    /// List length
    ListLen,
    /// List push (append during construction)
    ListPush,
    /// List slice (list[start:end])
    ListSlice,
    /// List slice with step (list[start:end:step])
    ListSliceStep,
    /// Extract list tail as tuple (list[start:] → tuple)
    /// Used for varargs collection: def f(a, *rest): f(*my_list)
    ListTailToTuple,
    /// Extract list tail as tuple, unboxing float elements
    /// Float lists store boxed FloatObj, varargs need raw f64 bits
    ListTailToTupleFloat,
    /// Extract list tail as tuple, unboxing bool elements
    /// Bool lists store boxed BoolObj, varargs need raw i8 values
    ListTailToTupleBool,
    /// List pop (remove and return element at index)
    ListPop,
    /// List insert (insert element at index)
    ListInsert,
    /// List remove (remove first occurrence of value)
    ListRemove,
    /// List clear (remove all elements)
    ListClear,
    /// List index (find first occurrence of value)
    ListIndex,
    /// List count (count occurrences of value)
    ListCount,
    /// List copy (shallow copy)
    ListCopy,
    /// List reverse (reverse in place)
    ListReverse,
    /// List extend (add elements from another iterable)
    ListExtend,
    /// List slice assignment: rt_list_slice_assign(list: *mut Obj, start: i64, stop: i64, values: *mut Obj)
    ListSliceAssign,
    /// Tuple set element (during construction)
    TupleSet,
    /// Set the heap_field_mask on a tuple (for mixed-type GC tracing)
    TupleSetHeapMask,
    /// Tuple get element
    TupleGet,
    /// Tuple length
    TupleLen,
    /// Tuple slice (tuple[start:end])
    TupleSlice,
    /// Tuple slice with step (tuple[start:end:step])
    TupleSliceStep,
    /// Tuple slice to list (for starred unpacking: *rest = tuple[start:end])
    TupleSliceToList,
    /// Tuple index (find first occurrence of value)
    TupleIndex,
    /// Tuple count (count occurrences of value)
    TupleCount,
    /// Tuple get int element (with automatic unboxing): rt_tuple_get_int(tuple, index) -> i64
    TupleGetInt,
    /// Tuple get float element (with automatic unboxing): rt_tuple_get_float(tuple, index) -> f64
    TupleGetFloat,
    /// Tuple get bool element (with automatic unboxing): rt_tuple_get_bool(tuple, index) -> i8
    TupleGetBool,
    /// Call a function pointer with arguments unpacked from a tuple.
    /// Used for *args forwarding in decorator wrappers: func(*args)
    /// rt_call_with_tuple_args(func_ptr, args_tuple) -> result
    CallWithTupleArgs,
    /// Concatenate two tuples: rt_tuple_concat(tuple1, tuple2) -> tuple
    /// Used for combining extra positional args with list-unpacked varargs
    TupleConcat,
    /// Concatenate two lists: rt_list_concat(list1, list2) -> list
    ListConcat,
    /// List get int element (with automatic unboxing): rt_list_get_int(list, index) -> i64
    ListGetInt,
    /// List get float element (with automatic unboxing): rt_list_get_float(list, index) -> f64
    ListGetFloat,
    /// List get bool element (with automatic unboxing): rt_list_get_bool(list, index) -> i8
    ListGetBool,
    /// Dict set (insert/update)
    DictSet,
    /// Dict get (lookup)
    DictGet,
    /// Dict length
    DictLen,
    /// Dict contains (key in dict)
    DictContains,
    /// Dict get with default (.get(key, default))
    DictGetDefault,
    /// Dict pop (remove and return)
    DictPop,
    /// Dict clear
    DictClear,
    /// Dict copy
    DictCopy,
    /// Dict keys
    DictKeys,
    /// Dict values
    DictValues,
    /// Dict items
    DictItems,
    /// Dict update
    DictUpdate,
    /// Dict fromkeys: rt_dict_from_keys(keys: *mut Obj, value: *mut Obj) -> *mut Obj
    DictFromKeys,
    /// Dict merge operator (dict | dict): rt_dict_merge(a: *mut Obj, b: *mut Obj) -> *mut Obj
    DictMerge,
    /// Box an integer as heap object (for dict keys)
    BoxInt,
    /// Box a boolean as heap object (for dict keys)
    BoxBool,
    /// Box a float as heap object (for list elements)
    BoxFloat,
    /// Box None as heap object (for Union types)
    BoxNone,
    /// Unbox a float from heap object (for list elements)
    UnboxFloat,
    /// Unbox an integer from heap object (for dict keys and set elements)
    UnboxInt,
    /// Unbox a boolean from heap object (for dict keys and set elements)
    UnboxBool,
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
    /// String find()
    StrFind,
    /// String rfind()
    StrRfind,
    /// String rindex (raises ValueError)
    StrRindex,
    /// String index (raises ValueError)
    StrIndex,
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
    /// Check issubclass: rt_issubclass(child: u8, parent: u8) -> i8
    IsSubclass,

    // ==================== Hash runtime functions ====================
    /// Hash an integer: rt_hash_int(value: i64) -> i64
    HashInt,
    /// Hash a string: rt_hash_str(str_obj: *mut Obj) -> i64
    HashStr,
    /// Hash a boolean: rt_hash_bool(value: i8) -> i64
    HashBool,
    /// Hash a tuple: rt_hash_tuple(tuple: *mut Obj) -> i64
    HashTuple,

    // ==================== Id runtime functions ====================
    /// Get id of heap object (returns pointer as i64): rt_id_obj(obj: *mut Obj) -> i64
    IdObj,

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
    Sorted {
        source: SortableKind,
        has_key: bool,
    },

    // ==================== Container min/max operations (unified) ====================
    /// Find min/max element in a container
    /// - For Int/Float: rt_{container}_{op}_{elem}(container) -> i64/f64
    /// - For WithKey: rt_{container}_{op}_with_key(container, key_fn) -> *mut Obj
    ContainerMinMax {
        container: ContainerKind,
        op: MinMaxOp,
        elem: ElementKind,
    },

    // ==================== Set runtime functions ====================
    /// Allocate set: rt_make_set(capacity: i64) -> *mut Obj
    MakeSet,
    /// Set add element: rt_set_add(set: *mut Obj, elem: *mut Obj)
    SetAdd,
    /// Set contains element: rt_set_contains(set: *mut Obj, elem: *mut Obj) -> i8
    SetContains,
    /// Set remove element (raises KeyError if missing): rt_set_remove(set: *mut Obj, elem: *mut Obj)
    SetRemove,
    /// Set discard element (no error if missing): rt_set_discard(set: *mut Obj, elem: *mut Obj)
    SetDiscard,
    /// Set length: rt_set_len(set: *mut Obj) -> i64
    SetLen,
    /// Set clear: rt_set_clear(set: *mut Obj)
    SetClear,
    /// Set copy: rt_set_copy(set: *mut Obj) -> *mut Obj
    SetCopy,
    /// Convert set to list: rt_set_to_list(set: *mut Obj) -> *mut Obj
    SetToList,
    /// Set union: rt_set_union(a: *mut Obj, b: *mut Obj) -> *mut Obj
    SetUnion,
    /// Set intersection: rt_set_intersection(a: *mut Obj, b: *mut Obj) -> *mut Obj
    SetIntersection,
    /// Set difference: rt_set_difference(a: *mut Obj, b: *mut Obj) -> *mut Obj
    SetDifference,
    /// Set symmetric difference: rt_set_symmetric_difference(a: *mut Obj, b: *mut Obj) -> *mut Obj
    SetSymmetricDifference,
    /// Set issubset: rt_set_issubset(a: *mut Obj, b: *mut Obj) -> i8
    SetIssubset,
    /// Set issuperset: rt_set_issuperset(a: *mut Obj, b: *mut Obj) -> i8
    SetIssuperset,
    /// Set isdisjoint: rt_set_isdisjoint(a: *mut Obj, b: *mut Obj) -> i8
    SetIsdisjoint,
    /// Set pop: rt_set_pop(set: *mut Obj) -> *mut Obj
    SetPop,
    /// Set update: rt_set_update(set: *mut Obj, iterable: *mut Obj)
    SetUpdate,
    /// Set intersection_update: rt_set_intersection_update(a: *mut Obj, b: *mut Obj)
    SetIntersectionUpdate,
    /// Set difference_update: rt_set_difference_update(a: *mut Obj, b: *mut Obj)
    SetDifferenceUpdate,
    /// Set symmetric_difference_update: rt_set_symmetric_difference_update(a: *mut Obj, b: *mut Obj)
    SetSymmetricDifferenceUpdate,

    // ==================== Bytes runtime functions ====================
    /// Allocate bytes: rt_make_bytes(data: *const u8, len: usize) -> *mut Obj
    MakeBytes,
    /// Allocate bytes filled with zeros: rt_make_bytes_zero(len: i64) -> *mut Obj
    MakeBytesZero,
    /// Create bytes from list of integers: rt_make_bytes_from_list(list: *mut Obj) -> *mut Obj
    MakeBytesFromList,
    /// Create bytes from string: rt_make_bytes_from_str(str_obj: *mut Obj) -> *mut Obj
    MakeBytesFromStr,
    /// Get byte at index: rt_bytes_get(bytes: *mut Obj, index: i64) -> i64
    BytesGet,
    /// Get bytes length: rt_bytes_len(bytes: *mut Obj) -> i64
    BytesLen,
    /// Slice bytes: rt_bytes_slice(bytes: *mut Obj, start: i64, end: i64) -> *mut Obj
    BytesSlice,
    /// Slice bytes with step: rt_bytes_slice_step(bytes: *mut Obj, start: i64, end: i64, step: i64) -> *mut Obj
    BytesSliceStep,
    /// Bytes decode to string: rt_bytes_decode(bytes: *mut Obj) -> *mut Obj
    BytesDecode,
    /// Bytes startswith: rt_bytes_startswith(bytes: *mut Obj, prefix: *mut Obj) -> i8
    BytesStartsWith,
    /// Bytes endswith: rt_bytes_endswith(bytes: *mut Obj, suffix: *mut Obj) -> i8
    BytesEndsWith,
    /// Bytes find: rt_bytes_find(bytes: *mut Obj, sub: *mut Obj) -> i64
    BytesFind,
    /// Bytes rfind: rt_bytes_rfind(bytes: *mut Obj, sub: *mut Obj) -> i64
    BytesRfind,
    /// Bytes index: rt_bytes_index(bytes: *mut Obj, sub: *mut Obj) -> i64
    BytesIndex,
    /// Bytes rindex: rt_bytes_rindex(bytes: *mut Obj, sub: *mut Obj) -> i64
    BytesRindex,
    /// Bytes count: rt_bytes_count(bytes: *mut Obj, sub: *mut Obj) -> i64
    BytesCount,
    /// Bytes replace: rt_bytes_replace(bytes: *mut Obj, old: *mut Obj, new: *mut Obj) -> *mut Obj
    BytesReplace,
    /// Bytes split: rt_bytes_split(bytes: *mut Obj, sep: *mut Obj, maxsplit: i64) -> *mut Obj
    BytesSplit,
    /// Bytes rsplit: rt_bytes_rsplit(bytes: *mut Obj, sep: *mut Obj, maxsplit: i64) -> *mut Obj
    BytesRsplit,
    /// Bytes join: rt_bytes_join(sep: *mut Obj, list: *mut Obj) -> *mut Obj
    BytesJoin,
    /// Bytes strip: rt_bytes_strip(bytes: *mut Obj, chars: *mut Obj) -> *mut Obj
    BytesStrip,
    /// Bytes lstrip: rt_bytes_lstrip(bytes: *mut Obj, chars: *mut Obj) -> *mut Obj
    BytesLstrip,
    /// Bytes rstrip: rt_bytes_rstrip(bytes: *mut Obj, chars: *mut Obj) -> *mut Obj
    BytesRstrip,
    /// Bytes upper: rt_bytes_upper(bytes: *mut Obj) -> *mut Obj
    BytesUpper,
    /// Bytes lower: rt_bytes_lower(bytes: *mut Obj) -> *mut Obj
    BytesLower,
    /// Bytes concatenation: rt_bytes_concat(a: *mut Obj, b: *mut Obj) -> *mut Obj
    BytesConcat,
    /// Bytes repetition: rt_bytes_repeat(bytes: *mut Obj, count: i64) -> *mut Obj
    BytesRepeat,
    /// Bytes.fromhex: rt_bytes_from_hex(hex_str: *mut Obj) -> *mut Obj
    BytesFromHex,
    /// byte in bytes: rt_bytes_contains(bytes: *mut Obj, byte: i64) -> i8
    BytesContains,

    // ==================== Comparison operations (unified) ====================
    /// Compare two containers or objects
    /// Replaces: ListEqInt, ListEqFloat, ListEqStr, TupleEq, TupleLt, TupleLte, TupleGt, TupleGte,
    /// StrEq, BytesEq, ObjEq, ObjLt, ObjLte, ObjGt, ObjGte (15 variants → 1 parameterized variant)
    Compare {
        kind: CompareKind,
        op: ComparisonOp,
    },

    // ==================== Runtime type dispatch for Union types ====================
    /// Check truthiness of an object with runtime type dispatch: rt_is_truthy(obj: *mut Obj) -> i8
    /// Falsy values: None, False, 0, 0.0, empty str/list/tuple/dict/set/bytes
    /// Used for truthiness narrowing in `if x:` where x is Optional[T]
    IsTruthy,
    /// Check if element is in container with runtime type dispatch: rt_obj_contains(container: *mut Obj, elem: *mut Obj) -> i8
    ObjContains,
    /// Register method name→slot mapping for a class (Protocol dispatch): rt_register_method_name(class_id, name_hash, slot)
    RegisterMethodName,
    /// Arithmetic on boxed Union objects: rt_obj_{add,sub,mul,div,floordiv,mod,pow}(a, b) -> *mut Obj
    ObjAdd,
    ObjSub,
    ObjMul,
    ObjDiv,
    ObjFloorDiv,
    ObjMod,
    ObjPow,
    /// Convert any heap object to string: rt_obj_to_str(obj: *mut Obj) -> *mut Obj
    ObjToStr,
    /// Default repr for objects without __str__ or __repr__: rt_obj_default_repr(obj: *mut Obj) -> *mut Obj
    ObjDefaultRepr,

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

    // ==================== File I/O runtime functions ====================
    /// Open a file: rt_file_open(filename: *mut Obj, mode: *mut Obj, encoding: *mut Obj) -> *mut Obj (FileObj)
    FileOpen,
    /// Read entire file: rt_file_read(file: *mut Obj) -> *mut Obj (str or bytes)
    FileRead,
    /// Read n bytes: rt_file_read_n(file: *mut Obj, n: i64) -> *mut Obj (str or bytes)
    FileReadN,
    /// Read single line: rt_file_readline(file: *mut Obj) -> *mut Obj (str)
    FileReadline,
    /// Read all lines: rt_file_readlines(file: *mut Obj) -> *mut Obj (list[str])
    FileReadlines,
    /// Write to file: rt_file_write(file: *mut Obj, data: *mut Obj) -> i64 (bytes written)
    FileWrite,
    /// Close file: rt_file_close(file: *mut Obj)
    FileClose,
    /// Flush file: rt_file_flush(file: *mut Obj)
    FileFlush,
    /// Context manager enter: rt_file_enter(file: *mut Obj) -> *mut Obj (returns self)
    FileEnter,
    /// Context manager exit: rt_file_exit(file: *mut Obj) -> i8 (returns False)
    FileExit,
    /// Check if closed: rt_file_is_closed(file: *mut Obj) -> i8
    FileIsClosed,
    /// Get filename: rt_file_name(file: *mut Obj) -> *mut Obj (str)
    FileName,

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

    // ==================== Dict methods (additional) ====================
    /// Dict setdefault: rt_dict_setdefault(dict: *mut Obj, key: *mut Obj, default: *mut Obj) -> *mut Obj
    DictSetDefault,
    /// Dict popitem: rt_dict_popitem(dict: *mut Obj) -> *mut Obj (returns tuple)
    DictPopItem,

    // ==================== DefaultDict operations ====================
    /// Create defaultdict: rt_make_defaultdict(capacity: i64, factory_tag: i64) -> *mut Obj
    MakeDefaultDict,
    /// Get from defaultdict (creates default on miss): rt_defaultdict_get(dd: *mut Obj, key: *mut Obj) -> *mut Obj
    DefaultDictGet,

    // ==================== Counter operations ====================
    /// Create counter from iterator: rt_make_counter_from_iter(iter: *mut Obj) -> *mut Obj
    MakeCounterFromIter,
    /// Create empty counter: rt_make_counter_empty() -> *mut Obj
    MakeCounterEmpty,

    // ==================== Deque operations ====================
    /// Create deque: rt_make_deque(maxlen: i64) -> *mut Obj
    MakeDeque,
    /// Create deque from iterator: rt_deque_from_iter(iter: *mut Obj, maxlen: i64) -> *mut Obj
    MakeDequeFromIter,

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

    // ==================== List methods ====================
    /// List sort in place: rt_list_sort(list: *mut Obj, reverse: i8)
    ListSort,
    /// List sort in place with key function: rt_list_sort_with_key(list: *mut Obj, reverse: i8, key_fn: fn)
    ListSortWithKey,

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

    // ==================== Map/Filter iterators ====================
    /// Create map iterator: rt_map_new(func_ptr: i64, iter: *mut Obj) -> *mut Obj
    MapNew,
    /// Create filter iterator: rt_filter_new(func_ptr: i64, iter: *mut Obj, elem_tag: i64) -> *mut Obj
    /// func_ptr=0 for truthiness filtering (filter(None, ...)), elem_tag for raw value handling
    FilterNew,

    // ==================== Collection constructors ====================
    /// Create list from tuple: rt_list_from_tuple(tuple: *mut Obj) -> *mut Obj
    ListFromTuple,
    /// Create list from string: rt_list_from_str(str: *mut Obj) -> *mut Obj
    ListFromStr,
    /// Create list from range: rt_list_from_range(start: i64, stop: i64, step: i64) -> *mut Obj
    ListFromRange,
    /// Create list from iterator: rt_list_from_iter(iter: *mut Obj, elem_tag: i64) -> *mut Obj
    ListFromIter,
    /// Create list from set: rt_list_from_set(set: *mut Obj) -> *mut Obj
    ListFromSet,
    /// Create list from dict keys: rt_list_from_dict(dict: *mut Obj) -> *mut Obj
    ListFromDict,

    /// Create tuple from list: rt_tuple_from_list(list: *mut Obj) -> *mut Obj
    TupleFromList,
    /// Create tuple from string: rt_tuple_from_str(str: *mut Obj) -> *mut Obj
    TupleFromStr,
    /// Create tuple from range: rt_tuple_from_range(start: i64, stop: i64, step: i64) -> *mut Obj
    TupleFromRange,
    /// Create tuple from iterator: rt_tuple_from_iter(iter: *mut Obj) -> *mut Obj
    TupleFromIter,
    /// Create tuple from set: rt_tuple_from_set(set: *mut Obj) -> *mut Obj
    TupleFromSet,
    /// Create tuple from dict keys: rt_tuple_from_dict(dict: *mut Obj) -> *mut Obj
    TupleFromDict,

    /// Create dict from list of pairs: rt_dict_from_pairs(list: *mut Obj) -> *mut Obj
    DictFromPairs,

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
