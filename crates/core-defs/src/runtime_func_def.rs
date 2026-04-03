//! Declarative runtime function descriptors.
//!
//! Instead of one enum variant per runtime function, describe each function
//! declaratively with its symbol name, parameter types, return type, and GC
//! behavior. A single generic codegen handler emits Cranelift IR for any
//! `RuntimeFuncDef`.

/// Cranelift parameter type for a runtime function argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    /// 64-bit integer or pointer (Cranelift `I64`)
    I64,
    /// 64-bit float (Cranelift `F64`)
    F64,
    /// 8-bit value: bool, tag byte (Cranelift `I8`)
    I8,
    /// 32-bit integer: var_id, generator index/state (Cranelift `I32`)
    I32,
}

/// Cranelift return type for a runtime function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnType {
    /// 64-bit integer or pointer
    I64,
    /// 64-bit float
    F64,
    /// 8-bit value (bool)
    I8,
    /// 32-bit integer
    I32,
}

/// Declarative description of a runtime function's ABI.
///
/// Used by codegen to build a Cranelift signature and emit a call instruction
/// without per-variant match arms.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeFuncDef {
    /// Symbol name for linking (e.g., `"rt_list_append"`)
    pub symbol: &'static str,
    /// Parameter types in order
    pub params: &'static [ParamType],
    /// Return type, or `None` for void functions
    pub returns: Option<ReturnType>,
    /// Whether the return value is a GC-managed heap pointer.
    /// When `true`, codegen calls `update_gc_root_if_needed` after storing
    /// the result.
    pub gc_roots_result: bool,
}

// Shorthand aliases for use in static definitions
#[allow(unused_imports)]
use ParamType::{F64 as PF64, I32 as PI32, I64 as PI64, I8 as PI8};
#[allow(unused_imports)]
use ReturnType::{F64 as RF64, I32 as RI32, I64 as RI64, I8 as RI8};

impl RuntimeFuncDef {
    /// General constructor.
    pub const fn new(
        symbol: &'static str,
        params: &'static [ParamType],
        returns: Option<ReturnType>,
        gc_roots_result: bool,
    ) -> Self {
        Self {
            symbol,
            params,
            returns,
            gc_roots_result,
        }
    }

    /// Unary: one I64 param, returns I64, GC-tracked.
    /// Typical for `(obj) -> obj` functions.
    pub const fn ptr_unary(symbol: &'static str) -> Self {
        Self {
            symbol,
            params: &[PI64],
            returns: Some(RI64),
            gc_roots_result: true,
        }
    }

    /// Binary: two I64 params, returns I64, GC-tracked.
    /// Typical for `(obj, obj) -> obj` functions.
    pub const fn ptr_binary(symbol: &'static str) -> Self {
        Self {
            symbol,
            params: &[PI64, PI64],
            returns: Some(RI64),
            gc_roots_result: true,
        }
    }

    /// Ternary: three I64 params, returns I64, GC-tracked.
    pub const fn ptr_ternary(symbol: &'static str) -> Self {
        Self {
            symbol,
            params: &[PI64, PI64, PI64],
            returns: Some(RI64),
            gc_roots_result: true,
        }
    }

    /// Quaternary: four I64 params, returns I64, GC-tracked.
    pub const fn ptr_quaternary(symbol: &'static str) -> Self {
        Self {
            symbol,
            params: &[PI64, PI64, PI64, PI64],
            returns: Some(RI64),
            gc_roots_result: true,
        }
    }

    /// Void function (no return value).
    pub const fn void(symbol: &'static str, params: &'static [ParamType]) -> Self {
        Self {
            symbol,
            params,
            returns: None,
            gc_roots_result: false,
        }
    }

    /// Unary returning raw i64 (not GC-tracked).
    /// Typical for `len()`, hash, etc.
    pub const fn unary_to_i64(symbol: &'static str) -> Self {
        Self {
            symbol,
            params: &[PI64],
            returns: Some(RI64),
            gc_roots_result: false,
        }
    }

    /// Binary returning raw i64 (not GC-tracked).
    pub const fn binary_to_i64(symbol: &'static str) -> Self {
        Self {
            symbol,
            params: &[PI64, PI64],
            returns: Some(RI64),
            gc_roots_result: false,
        }
    }

    /// Unary returning i8 (bool result, not GC-tracked).
    pub const fn unary_to_i8(symbol: &'static str) -> Self {
        Self {
            symbol,
            params: &[PI64],
            returns: Some(RI8),
            gc_roots_result: false,
        }
    }

    /// Binary returning i8 (bool result, not GC-tracked).
    pub const fn binary_to_i8(symbol: &'static str) -> Self {
        Self {
            symbol,
            params: &[PI64, PI64],
            returns: Some(RI8),
            gc_roots_result: false,
        }
    }
}

// =============================================================================
// Static runtime function definitions
// =============================================================================

// ===== Hash operations =====

/// rt_hash_int(value: i64) -> i64
pub static RT_HASH_INT: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_hash_int");
/// rt_hash_str(str_obj: *mut Obj) -> i64
pub static RT_HASH_STR: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_hash_str");
/// rt_hash_bool(value: i8) -> i64
pub static RT_HASH_BOOL: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_hash_bool", &[PI8], Some(RI64), false);
/// rt_hash_tuple(tuple: *mut Obj) -> i64
pub static RT_HASH_TUPLE: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_hash_tuple");

// ===== Id operations =====

/// rt_id_obj(obj: *mut Obj) -> i64
pub static RT_ID_OBJ: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_id_obj");

// ===== Boxing operations =====

/// rt_box_int(value: i64) -> *mut Obj
pub static RT_BOX_INT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_box_int");
/// rt_box_bool(value: i8) -> *mut Obj
pub static RT_BOX_BOOL: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_box_bool", &[PI8], Some(RI64), true);
/// rt_box_float(value: f64) -> *mut Obj
pub static RT_BOX_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_box_float", &[PF64], Some(RI64), true);
/// rt_box_none() -> *mut Obj
pub static RT_BOX_NONE: RuntimeFuncDef = RuntimeFuncDef::new("rt_box_none", &[], Some(RI64), true);

// ===== Unboxing operations =====

/// rt_unbox_int(obj: *mut Obj) -> i64
pub static RT_UNBOX_INT: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_unbox_int");
/// rt_unbox_float(obj: *mut Obj) -> f64
pub static RT_UNBOX_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_unbox_float", &[PI64], Some(RF64), false);
/// rt_unbox_bool(obj: *mut Obj) -> i8
pub static RT_UNBOX_BOOL: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_unbox_bool");

// ===== File I/O operations =====

/// rt_file_open(filename: *mut Obj, mode: *mut Obj, encoding: *mut Obj) -> *mut Obj
pub static RT_FILE_OPEN: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_file_open");
/// rt_file_read(file: *mut Obj) -> *mut Obj
pub static RT_FILE_READ: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_file_read");
/// rt_file_read_n(file: *mut Obj, n: i64) -> *mut Obj
pub static RT_FILE_READ_N: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_file_read_n");
/// rt_file_readline(file: *mut Obj) -> *mut Obj
pub static RT_FILE_READLINE: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_file_readline");
/// rt_file_readlines(file: *mut Obj) -> *mut Obj
pub static RT_FILE_READLINES: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_file_readlines");
/// rt_file_write(file: *mut Obj, data: *mut Obj) -> i64 (bytes written)
pub static RT_FILE_WRITE: RuntimeFuncDef = RuntimeFuncDef::binary_to_i64("rt_file_write");
/// rt_file_close(file: *mut Obj) -> void
pub static RT_FILE_CLOSE: RuntimeFuncDef = RuntimeFuncDef::void("rt_file_close", &[PI64]);
/// rt_file_flush(file: *mut Obj) -> void
pub static RT_FILE_FLUSH: RuntimeFuncDef = RuntimeFuncDef::void("rt_file_flush", &[PI64]);
/// rt_file_enter(file: *mut Obj) -> *mut Obj (returns self)
pub static RT_FILE_ENTER: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_file_enter");
/// rt_file_exit(file: *mut Obj) -> i8 (returns False)
pub static RT_FILE_EXIT: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_file_exit");
/// rt_file_is_closed(file: *mut Obj) -> i8
pub static RT_FILE_IS_CLOSED: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_file_is_closed");
/// rt_file_name(file: *mut Obj) -> *mut Obj
pub static RT_FILE_NAME: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_file_name");

// ===== Object operations (Union type dispatch) =====

/// rt_is_truthy(obj: *mut Obj) -> i8
pub static RT_IS_TRUTHY: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_is_truthy");
/// rt_obj_contains(container: *mut Obj, elem: *mut Obj) -> i8
pub static RT_OBJ_CONTAINS: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_obj_contains");
/// rt_obj_to_str(obj: *mut Obj) -> *mut Obj
pub static RT_OBJ_TO_STR: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_obj_to_str");
/// rt_obj_default_repr(obj: *mut Obj) -> *mut Obj
pub static RT_OBJ_DEFAULT_REPR: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_obj_default_repr");
/// rt_obj_add(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_OBJ_ADD: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_obj_add");
/// rt_obj_sub(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_OBJ_SUB: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_obj_sub");
/// rt_obj_mul(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_OBJ_MUL: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_obj_mul");
/// rt_obj_div(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_OBJ_DIV: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_obj_div");
/// rt_obj_floordiv(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_OBJ_FLOORDIV: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_obj_floordiv");
/// rt_obj_mod(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_OBJ_MOD: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_obj_mod");
/// rt_obj_pow(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_OBJ_POW: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_obj_pow");
/// rt_any_getitem(obj: *mut Obj, index: i64) -> *mut Obj
pub static RT_ANY_GETITEM: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_any_getitem");

// ===== Set operations =====

/// rt_make_set(capacity: i64) -> *mut Obj
pub static RT_MAKE_SET: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_make_set");
/// rt_set_add(set: *mut Obj, elem: *mut Obj) -> void
pub static RT_SET_ADD: RuntimeFuncDef = RuntimeFuncDef::void("rt_set_add", &[PI64, PI64]);
/// rt_set_contains(set: *mut Obj, elem: *mut Obj) -> i8
pub static RT_SET_CONTAINS: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_set_contains");
/// rt_set_remove(set: *mut Obj, elem: *mut Obj) -> void
pub static RT_SET_REMOVE: RuntimeFuncDef = RuntimeFuncDef::void("rt_set_remove", &[PI64, PI64]);
/// rt_set_discard(set: *mut Obj, elem: *mut Obj) -> void
pub static RT_SET_DISCARD: RuntimeFuncDef = RuntimeFuncDef::void("rt_set_discard", &[PI64, PI64]);
/// rt_set_len(set: *mut Obj) -> i64
pub static RT_SET_LEN: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_set_len");
/// rt_set_clear(set: *mut Obj) -> void
pub static RT_SET_CLEAR: RuntimeFuncDef = RuntimeFuncDef::void("rt_set_clear", &[PI64]);
/// rt_set_copy(set: *mut Obj) -> *mut Obj
pub static RT_SET_COPY: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_set_copy");
/// rt_set_to_list(set: *mut Obj) -> *mut Obj
pub static RT_SET_TO_LIST: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_set_to_list");
/// rt_set_union(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_SET_UNION: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_set_union");
/// rt_set_intersection(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_SET_INTERSECTION: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_set_intersection");
/// rt_set_difference(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_SET_DIFFERENCE: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_set_difference");
/// rt_set_symmetric_difference(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_SET_SYMMETRIC_DIFFERENCE: RuntimeFuncDef =
    RuntimeFuncDef::ptr_binary("rt_set_symmetric_difference");
/// rt_set_issubset(a: *mut Obj, b: *mut Obj) -> i8
pub static RT_SET_ISSUBSET: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_set_issubset");
/// rt_set_issuperset(a: *mut Obj, b: *mut Obj) -> i8
pub static RT_SET_ISSUPERSET: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_set_issuperset");
/// rt_set_isdisjoint(a: *mut Obj, b: *mut Obj) -> i8
pub static RT_SET_ISDISJOINT: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_set_isdisjoint");
/// rt_set_pop(set: *mut Obj) -> *mut Obj
pub static RT_SET_POP: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_set_pop");
/// rt_set_update(set: *mut Obj, other: *mut Obj) -> void
pub static RT_SET_UPDATE: RuntimeFuncDef = RuntimeFuncDef::void("rt_set_update", &[PI64, PI64]);
/// rt_set_intersection_update(set: *mut Obj, other: *mut Obj) -> void
pub static RT_SET_INTERSECTION_UPDATE: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_set_intersection_update", &[PI64, PI64]);
/// rt_set_difference_update(set: *mut Obj, other: *mut Obj) -> void
pub static RT_SET_DIFFERENCE_UPDATE: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_set_difference_update", &[PI64, PI64]);
/// rt_set_symmetric_difference_update(set: *mut Obj, other: *mut Obj) -> void
pub static RT_SET_SYMMETRIC_DIFFERENCE_UPDATE: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_set_symmetric_difference_update", &[PI64, PI64]);

// ===== Dict operations =====

/// rt_make_dict(capacity: i64) -> *mut Obj
pub static RT_MAKE_DICT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_make_dict");
/// rt_dict_set(dict: *mut Obj, key: i64, value: i64) -> void
pub static RT_DICT_SET: RuntimeFuncDef = RuntimeFuncDef::void("rt_dict_set", &[PI64, PI64, PI64]);
/// rt_dict_get(dict: *mut Obj, key: i64) -> *mut Obj
pub static RT_DICT_GET: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_dict_get");
/// rt_dict_len(dict: *mut Obj) -> i64
pub static RT_DICT_LEN: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_dict_len");
/// rt_dict_contains(dict: *mut Obj, key: i64) -> i8
pub static RT_DICT_CONTAINS: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_dict_contains");
/// rt_dict_get_default(dict: *mut Obj, key: i64, default: i64) -> *mut Obj
pub static RT_DICT_GET_DEFAULT: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_dict_get_default");
/// rt_dict_pop(dict: *mut Obj, key: i64) -> *mut Obj
pub static RT_DICT_POP: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_dict_pop");
/// rt_dict_clear(dict: *mut Obj) -> void
pub static RT_DICT_CLEAR: RuntimeFuncDef = RuntimeFuncDef::void("rt_dict_clear", &[PI64]);
/// rt_dict_copy(dict: *mut Obj) -> *mut Obj
pub static RT_DICT_COPY: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_dict_copy");
/// rt_dict_keys(dict: *mut Obj, elem_tag: i8) -> *mut Obj
pub static RT_DICT_KEYS: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_dict_keys", &[PI64, PI8], Some(RI64), true);
/// rt_dict_values(dict: *mut Obj, elem_tag: i8) -> *mut Obj
pub static RT_DICT_VALUES: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_dict_values", &[PI64, PI8], Some(RI64), true);
/// rt_dict_items(dict: *mut Obj) -> *mut Obj
pub static RT_DICT_ITEMS: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_dict_items");
/// rt_dict_update(dict: *mut Obj, other: *mut Obj) -> void
pub static RT_DICT_UPDATE: RuntimeFuncDef = RuntimeFuncDef::void("rt_dict_update", &[PI64, PI64]);
/// rt_dict_from_pairs(pairs: *mut Obj) -> *mut Obj
pub static RT_DICT_FROM_PAIRS: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_dict_from_pairs");
/// rt_dict_setdefault(dict: *mut Obj, key: i64, default: i64) -> *mut Obj
pub static RT_DICT_SET_DEFAULT: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_dict_setdefault");
/// rt_dict_popitem(dict: *mut Obj) -> *mut Obj
pub static RT_DICT_POP_ITEM: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_dict_popitem");
/// rt_dict_fromkeys(keys: *mut Obj, value: *mut Obj) -> *mut Obj
pub static RT_DICT_FROM_KEYS: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_dict_fromkeys");
/// rt_dict_merge(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_DICT_MERGE: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_dict_merge");
/// rt_make_defaultdict(default_factory: *mut Obj, initial: *mut Obj) -> *mut Obj
pub static RT_MAKE_DEFAULT_DICT: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_make_defaultdict");
/// rt_defaultdict_get(dict: *mut Obj, key: *mut Obj) -> *mut Obj
pub static RT_DEFAULT_DICT_GET: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_defaultdict_get");
/// rt_make_counter_from_iter(iter: *mut Obj) -> *mut Obj
pub static RT_MAKE_COUNTER_FROM_ITER: RuntimeFuncDef =
    RuntimeFuncDef::ptr_unary("rt_make_counter_from_iter");
/// rt_make_counter_empty() -> *mut Obj
pub static RT_MAKE_COUNTER_EMPTY: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_make_counter_empty", &[], Some(RI64), true);
/// rt_make_deque(iterable: *mut Obj) -> *mut Obj
pub static RT_MAKE_DEQUE: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_make_deque");
/// rt_deque_from_iter(iter: *mut Obj, maxlen: *mut Obj) -> *mut Obj
pub static RT_MAKE_DEQUE_FROM_ITER: RuntimeFuncDef =
    RuntimeFuncDef::ptr_binary("rt_deque_from_iter");

// ===== List operations =====

/// rt_make_list(capacity: i64, elem_tag: i8) -> *mut Obj
pub static RT_MAKE_LIST: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_make_list", &[PI64, PI8], Some(RI64), true);
/// rt_list_push(list: *mut Obj, elem: i64) -> void
pub static RT_LIST_PUSH: RuntimeFuncDef = RuntimeFuncDef::void("rt_list_push", &[PI64, PI64]);
/// rt_list_set(list: *mut Obj, index: i64, value: i64) -> void
pub static RT_LIST_SET: RuntimeFuncDef = RuntimeFuncDef::void("rt_list_set", &[PI64, PI64, PI64]);
/// rt_list_get(list: *mut Obj, index: i64) -> *mut Obj
pub static RT_LIST_GET: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_list_get");
/// rt_list_get_int(list: *mut Obj, index: i64) -> i64
pub static RT_LIST_GET_INT: RuntimeFuncDef = RuntimeFuncDef::binary_to_i64("rt_list_get_int");
/// rt_list_get_float(list: *mut Obj, index: i64) -> f64
pub static RT_LIST_GET_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_list_get_float", &[PI64, PI64], Some(RF64), false);
/// rt_list_get_bool(list: *mut Obj, index: i64) -> i8
pub static RT_LIST_GET_BOOL: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_list_get_bool");
/// rt_list_len(list: *mut Obj) -> i64
pub static RT_LIST_LEN: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_list_len");
/// rt_list_slice(list: *mut Obj, start: i64, stop: i64) -> *mut Obj
pub static RT_LIST_SLICE: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_list_slice");
/// rt_list_slice_step(list: *mut Obj, start: i64, stop: i64, step: i64) -> *mut Obj
pub static RT_LIST_SLICE_STEP: RuntimeFuncDef =
    RuntimeFuncDef::ptr_quaternary("rt_list_slice_step");
/// rt_list_append(list: *mut Obj, elem: i64) -> void
pub static RT_LIST_APPEND: RuntimeFuncDef = RuntimeFuncDef::void("rt_list_append", &[PI64, PI64]);
/// rt_list_set_elem_tag(list: *mut Obj, tag: u8) -> void
pub static RT_LIST_SET_ELEM_TAG: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_list_set_elem_tag", &[PI64, PI8]);
/// rt_list_pop(list: *mut Obj, index: i64) -> *mut Obj
pub static RT_LIST_POP: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_list_pop");
/// rt_list_insert(list: *mut Obj, index: i64, elem: i64) -> void
pub static RT_LIST_INSERT: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_list_insert", &[PI64, PI64, PI64]);
/// rt_list_remove(list: *mut Obj, elem: i64) -> i8
pub static RT_LIST_REMOVE: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_list_remove");
/// rt_list_clear(list: *mut Obj) -> void
pub static RT_LIST_CLEAR: RuntimeFuncDef = RuntimeFuncDef::void("rt_list_clear", &[PI64]);
/// rt_list_index(list: *mut Obj, elem: i64) -> i64
pub static RT_LIST_INDEX: RuntimeFuncDef = RuntimeFuncDef::binary_to_i64("rt_list_index");
/// rt_list_count(list: *mut Obj, elem: i64) -> i64
pub static RT_LIST_COUNT: RuntimeFuncDef = RuntimeFuncDef::binary_to_i64("rt_list_count");
/// rt_list_copy(list: *mut Obj) -> *mut Obj
pub static RT_LIST_COPY: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_list_copy");
/// rt_list_reverse(list: *mut Obj) -> void
pub static RT_LIST_REVERSE: RuntimeFuncDef = RuntimeFuncDef::void("rt_list_reverse", &[PI64]);
/// rt_list_extend(list: *mut Obj, other: *mut Obj) -> void
pub static RT_LIST_EXTEND: RuntimeFuncDef = RuntimeFuncDef::void("rt_list_extend", &[PI64, PI64]);
/// rt_list_sort(list: *mut Obj, reverse: i8) -> void
pub static RT_LIST_SORT: RuntimeFuncDef = RuntimeFuncDef::void("rt_list_sort", &[PI64, PI8]);
/// rt_list_sort_with_key(list: *mut Obj, reverse: i8, key_fn: i64, elem_tag: i64, captures: i64, capture_count: i64) -> void
pub static RT_LIST_SORT_WITH_KEY: RuntimeFuncDef = RuntimeFuncDef::void(
    "rt_list_sort_with_key",
    &[PI64, PI8, PI64, PI64, PI64, PI64],
);
/// rt_list_from_tuple(tuple: *mut Obj) -> *mut Obj
pub static RT_LIST_FROM_TUPLE: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_list_from_tuple");
/// rt_list_from_str(str: *mut Obj) -> *mut Obj
pub static RT_LIST_FROM_STR: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_list_from_str");
/// rt_list_from_range(start: i64, stop: i64, step: i64) -> *mut Obj
pub static RT_LIST_FROM_RANGE: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_list_from_range");
/// rt_list_from_iter(iter: *mut Obj, elem_tag: i64) -> *mut Obj
pub static RT_LIST_FROM_ITER: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_list_from_iter", &[PI64, PI64], Some(RI64), false);
/// rt_list_from_set(set: *mut Obj) -> *mut Obj
pub static RT_LIST_FROM_SET: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_list_from_set");
/// rt_list_from_dict(dict: *mut Obj) -> *mut Obj
pub static RT_LIST_FROM_DICT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_list_from_dict");
/// rt_list_tail_to_tuple(list: *mut Obj, start: i64) -> *mut Obj
pub static RT_LIST_TAIL_TO_TUPLE: RuntimeFuncDef =
    RuntimeFuncDef::ptr_binary("rt_list_tail_to_tuple");
/// rt_list_tail_to_tuple_float(list: *mut Obj, start: i64) -> *mut Obj
pub static RT_LIST_TAIL_TO_TUPLE_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::ptr_binary("rt_list_tail_to_tuple_float");
/// rt_list_tail_to_tuple_bool(list: *mut Obj, start: i64) -> *mut Obj
pub static RT_LIST_TAIL_TO_TUPLE_BOOL: RuntimeFuncDef =
    RuntimeFuncDef::ptr_binary("rt_list_tail_to_tuple_bool");
/// rt_list_slice_assign(list: *mut Obj, start: i64, stop: i64, values: *mut Obj) -> void
pub static RT_LIST_SLICE_ASSIGN: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_list_slice_assign", &[PI64, PI64, PI64, PI64]);
/// rt_list_concat(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_LIST_CONCAT: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_list_concat");

// ===== Tuple operations =====

/// rt_make_tuple(size: i64, elem_tag: i8) -> *mut Obj
pub static RT_MAKE_TUPLE: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_make_tuple", &[PI64, PI8], Some(RI64), true);
/// rt_tuple_set(tuple: *mut Obj, index: i64, value: i64) -> void
pub static RT_TUPLE_SET: RuntimeFuncDef = RuntimeFuncDef::void("rt_tuple_set", &[PI64, PI64, PI64]);
/// rt_tuple_set_heap_mask(tuple: *mut Obj, mask: i64) -> void
pub static RT_TUPLE_SET_HEAP_MASK: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_tuple_set_heap_mask", &[PI64, PI64]);
/// rt_tuple_get(tuple: *mut Obj, index: i64) -> *mut Obj
pub static RT_TUPLE_GET: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_tuple_get");
/// rt_tuple_len(tuple: *mut Obj) -> i64
pub static RT_TUPLE_LEN: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_tuple_len");
/// rt_tuple_slice(tuple: *mut Obj, start: i64, stop: i64) -> *mut Obj
pub static RT_TUPLE_SLICE: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_tuple_slice");
/// rt_tuple_slice_step(tuple: *mut Obj, start: i64, stop: i64, step: i64) -> *mut Obj
pub static RT_TUPLE_SLICE_STEP: RuntimeFuncDef =
    RuntimeFuncDef::ptr_quaternary("rt_tuple_slice_step");
/// rt_tuple_slice_to_list(tuple: *mut Obj, start: i64, stop: i64) -> *mut Obj
pub static RT_TUPLE_SLICE_TO_LIST: RuntimeFuncDef =
    RuntimeFuncDef::ptr_ternary("rt_tuple_slice_to_list");
/// rt_tuple_get_int(tuple: *mut Obj, index: i64) -> i64
pub static RT_TUPLE_GET_INT: RuntimeFuncDef = RuntimeFuncDef::binary_to_i64("rt_tuple_get_int");
/// rt_tuple_get_float(tuple: *mut Obj, index: i64) -> f64
pub static RT_TUPLE_GET_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_tuple_get_float", &[PI64, PI64], Some(RF64), false);
/// rt_tuple_get_bool(tuple: *mut Obj, index: i64) -> i8
pub static RT_TUPLE_GET_BOOL: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_tuple_get_bool");
/// rt_tuple_from_list(list: *mut Obj) -> *mut Obj
pub static RT_TUPLE_FROM_LIST: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_tuple_from_list");
/// rt_tuple_from_str(str: *mut Obj) -> *mut Obj
pub static RT_TUPLE_FROM_STR: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_tuple_from_str");
/// rt_tuple_from_range(start: i64, stop: i64, step: i64) -> *mut Obj
pub static RT_TUPLE_FROM_RANGE: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_tuple_from_range");
/// rt_tuple_from_iter(iter: *mut Obj) -> *mut Obj
pub static RT_TUPLE_FROM_ITER: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_tuple_from_iter");
/// rt_tuple_from_set(set: *mut Obj) -> *mut Obj
pub static RT_TUPLE_FROM_SET: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_tuple_from_set");
/// rt_tuple_from_dict(dict: *mut Obj) -> *mut Obj
pub static RT_TUPLE_FROM_DICT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_tuple_from_dict");
/// rt_tuple_concat(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_TUPLE_CONCAT: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_tuple_concat");
/// rt_tuple_index(tuple: *mut Obj, elem: i64) -> i64
pub static RT_TUPLE_INDEX: RuntimeFuncDef = RuntimeFuncDef::binary_to_i64("rt_tuple_index");
/// rt_tuple_count(tuple: *mut Obj, elem: i64) -> i64
pub static RT_TUPLE_COUNT: RuntimeFuncDef = RuntimeFuncDef::binary_to_i64("rt_tuple_count");
/// rt_call_with_tuple_args(func_ptr: i64, args_tuple: *mut Obj) -> i64
pub static RT_CALL_WITH_TUPLE_ARGS: RuntimeFuncDef =
    RuntimeFuncDef::binary_to_i64("rt_call_with_tuple_args");

// ===== Bytes operations =====

/// rt_make_bytes(data_ptr: i64, len: i64) -> *mut Obj
pub static RT_MAKE_BYTES: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_make_bytes");
/// rt_make_bytes_zero(len: i64) -> *mut Obj
pub static RT_MAKE_BYTES_ZERO: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_make_bytes_zero");
/// rt_make_bytes_from_list(list: *mut Obj) -> *mut Obj
pub static RT_MAKE_BYTES_FROM_LIST: RuntimeFuncDef =
    RuntimeFuncDef::ptr_unary("rt_make_bytes_from_list");
/// rt_make_bytes_from_str(str: *mut Obj) -> *mut Obj
pub static RT_MAKE_BYTES_FROM_STR: RuntimeFuncDef =
    RuntimeFuncDef::ptr_unary("rt_make_bytes_from_str");
/// rt_bytes_get(bytes: *mut Obj, index: i64) -> i64
pub static RT_BYTES_GET: RuntimeFuncDef = RuntimeFuncDef::binary_to_i64("rt_bytes_get");
/// rt_bytes_len(bytes: *mut Obj) -> i64
pub static RT_BYTES_LEN: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_bytes_len");
/// rt_bytes_slice(bytes: *mut Obj, start: i64, stop: i64) -> *mut Obj
pub static RT_BYTES_SLICE: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_bytes_slice");
/// rt_bytes_slice_step(bytes: *mut Obj, start: i64, stop: i64, step: i64) -> *mut Obj
pub static RT_BYTES_SLICE_STEP: RuntimeFuncDef =
    RuntimeFuncDef::ptr_quaternary("rt_bytes_slice_step");
/// rt_bytes_decode(bytes: *mut Obj, encoding: i64) -> *mut Obj
pub static RT_BYTES_DECODE: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_bytes_decode");
/// rt_bytes_startswith(bytes: *mut Obj, prefix: *mut Obj) -> i8
pub static RT_BYTES_STARTS_WITH: RuntimeFuncDef =
    RuntimeFuncDef::binary_to_i8("rt_bytes_startswith");
/// rt_bytes_endswith(bytes: *mut Obj, suffix: *mut Obj) -> i8
pub static RT_BYTES_ENDS_WITH: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_bytes_endswith");
/// rt_bytes_find(bytes: *mut Obj, sub: *mut Obj) -> i64
pub static RT_BYTES_FIND: RuntimeFuncDef = RuntimeFuncDef::binary_to_i64("rt_bytes_find");
/// rt_bytes_rfind(bytes: *mut Obj, sub: *mut Obj) -> i64
pub static RT_BYTES_RFIND: RuntimeFuncDef = RuntimeFuncDef::binary_to_i64("rt_bytes_rfind");
/// rt_bytes_search(bytes: *mut Obj, sub: *mut Obj, op_tag: u8) -> i64 (index variant)
pub static RT_BYTES_INDEX: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_bytes_search", &[PI64, PI64, PI8], Some(RI64), false);
/// rt_bytes_search(bytes: *mut Obj, sub: *mut Obj, op_tag: u8) -> i64 (rindex variant)
pub static RT_BYTES_RINDEX: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_bytes_search", &[PI64, PI64, PI8], Some(RI64), false);
/// rt_bytes_count(bytes: *mut Obj, sub: *mut Obj) -> i64
pub static RT_BYTES_COUNT: RuntimeFuncDef = RuntimeFuncDef::binary_to_i64("rt_bytes_count");
/// rt_bytes_replace(bytes: *mut Obj, old: *mut Obj, new: *mut Obj) -> *mut Obj
pub static RT_BYTES_REPLACE: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_bytes_replace");
/// rt_bytes_split(bytes: *mut Obj, sep: *mut Obj, maxsplit: i64) -> *mut Obj
pub static RT_BYTES_SPLIT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_bytes_split", &[PI64, PI64, PI64], Some(RI64), true);
/// rt_bytes_rsplit(bytes: *mut Obj, sep: *mut Obj, maxsplit: i64) -> *mut Obj
pub static RT_BYTES_RSPLIT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_bytes_rsplit", &[PI64, PI64, PI64], Some(RI64), true);
/// rt_bytes_join(sep: *mut Obj, iterable: *mut Obj) -> *mut Obj
pub static RT_BYTES_JOIN: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_bytes_join");
/// rt_bytes_strip(bytes: *mut Obj, chars: *mut Obj) -> *mut Obj
pub static RT_BYTES_STRIP: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_bytes_strip");
/// rt_bytes_lstrip(bytes: *mut Obj, chars: *mut Obj) -> *mut Obj
pub static RT_BYTES_LSTRIP: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_bytes_lstrip");
/// rt_bytes_rstrip(bytes: *mut Obj, chars: *mut Obj) -> *mut Obj
pub static RT_BYTES_RSTRIP: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_bytes_rstrip");
/// rt_bytes_upper(bytes: *mut Obj) -> *mut Obj
pub static RT_BYTES_UPPER: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_bytes_upper");
/// rt_bytes_lower(bytes: *mut Obj) -> *mut Obj
pub static RT_BYTES_LOWER: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_bytes_lower");
/// rt_bytes_concat(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_BYTES_CONCAT: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_bytes_concat");
/// rt_bytes_repeat(bytes: *mut Obj, count: i64) -> *mut Obj
pub static RT_BYTES_REPEAT: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_bytes_repeat");
/// rt_bytes_from_hex(hex_str: *mut Obj) -> *mut Obj
pub static RT_BYTES_FROM_HEX: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_bytes_from_hex");
/// rt_bytes_contains(bytes: *mut Obj, sub: *mut Obj) -> i8
pub static RT_BYTES_CONTAINS: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_bytes_contains", &[PI64, PI64], Some(RI8), false);
