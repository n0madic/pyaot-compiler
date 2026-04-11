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

// Shorthand aliases for use in static definitions (private, used within this module)
#[allow(unused_imports)]
use ParamType::{F64 as PF64, I32 as PI32, I64 as PI64, I8 as PI8};
#[allow(unused_imports)]
use ReturnType::{F64 as RF64, I32 as RI32, I64 as RI64, I8 as RI8};

// Public shorthand aliases for use in other crates (e.g., stdlib-defs codegen fields)
pub const P_I64: ParamType = ParamType::I64;
pub const P_F64: ParamType = ParamType::F64;
pub const P_I8: ParamType = ParamType::I8;
pub const P_I32: ParamType = ParamType::I32;
pub const R_I64: ReturnType = ReturnType::I64;
pub const R_F64: ReturnType = ReturnType::F64;
pub const R_I8: ReturnType = ReturnType::I8;
pub const R_I32: ReturnType = ReturnType::I32;

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

// ===== Math operations =====

/// rt_pow_float(base: f64, exp: f64) -> f64
pub static RT_POW_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_pow_float", &[PF64, PF64], Some(RF64), false);
/// rt_pow_int(base: i64, exp: i64) -> i64
pub static RT_POW_INT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_pow_int", &[PI64, PI64], Some(RI64), false);
/// rt_round_to_int(x: f64) -> i64
pub static RT_ROUND_TO_INT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_round_to_int", &[PF64], Some(RI64), false);
/// rt_round_to_digits(x: f64, ndigits: i64) -> f64
pub static RT_ROUND_TO_DIGITS: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_round_to_digits", &[PF64, PI64], Some(RF64), false);
/// rt_int_to_chr(code: i64) -> *mut Obj
pub static RT_INT_TO_CHR: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_int_to_chr");
/// rt_chr_to_int(s: *mut Obj) -> i64
pub static RT_CHR_TO_INT: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_chr_to_int");

// ===== Comparison operations =====
// Compare(kind, op) → static defs for all valid (kind, op) combinations.
// Signature: (a: I64, b: I64) -> I8 for eq-only variants.
// Signature: (a: I64, b: I64, op_tag: I8) -> I8 for ordering variants with op_tag.
// Signature: (a: I64, b: I64) -> I8 for Obj ordering (separate functions per op).

/// rt_list_eq_int(a: *mut Obj, b: *mut Obj) -> i8
pub static RT_CMP_LIST_INT_EQ: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_list_eq_int");
/// rt_list_eq_float(a: *mut Obj, b: *mut Obj) -> i8
pub static RT_CMP_LIST_FLOAT_EQ: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_list_eq_float");
/// rt_list_eq_str(a: *mut Obj, b: *mut Obj) -> i8
pub static RT_CMP_LIST_STR_EQ: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_list_eq_str");
/// rt_list_eq_int(a: *mut Obj, b: *mut Obj) -> i8 (List Eq fallback)
pub static RT_CMP_LIST_EQ: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_list_eq_int");
/// rt_list_cmp(a: *mut Obj, b: *mut Obj, op_tag: i8) -> i8
pub static RT_CMP_LIST_ORD: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_list_cmp", &[PI64, PI64, PI8], Some(RI8), false);
/// rt_tuple_eq(a: *mut Obj, b: *mut Obj) -> i8
pub static RT_CMP_TUPLE_EQ: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_tuple_eq");
/// rt_tuple_cmp(a: *mut Obj, b: *mut Obj, op_tag: i8) -> i8
pub static RT_CMP_TUPLE_ORD: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_tuple_cmp", &[PI64, PI64, PI8], Some(RI8), false);
/// rt_str_eq(a: *mut Obj, b: *mut Obj) -> i8
pub static RT_CMP_STR_EQ: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_str_eq");
/// rt_bytes_eq(a: *mut Obj, b: *mut Obj) -> i8
pub static RT_CMP_BYTES_EQ: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_bytes_eq");
/// rt_obj_eq(a: *mut Obj, b: *mut Obj) -> i8
pub static RT_CMP_OBJ_EQ: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_obj_eq");
/// rt_obj_cmp(a: *mut Obj, b: *mut Obj, op_tag: u8) -> i8
/// op_tag: 0=Lt, 1=Lte, 2=Gt, 3=Gte (matches ComparisonOp::to_tag())
pub static RT_CMP_OBJ_ORD: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_obj_cmp", &[PI64, PI64, PI8], Some(RI8), false);

// ===== Container min/max operations =====
// ContainerMinMax { container, op, elem } → static defs.
// Int/Float: rt_{container}_minmax(container: I64, is_min: I8, elem_kind: I8) -> I64
// WithKey: rt_{container}_minmax_with_key(container: I64, key_fn: I64, elem_tag: I64, captures: I64, count: I64, is_min: I8) -> I64

/// rt_list_minmax(list: *mut Obj, is_min: i8, elem_kind: i8) -> i64
pub static RT_LIST_MINMAX: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_list_minmax", &[PI64, PI8, PI8], Some(RI64), false);
/// rt_tuple_minmax(tuple: *mut Obj, is_min: i8, elem_kind: i8) -> i64
pub static RT_TUPLE_MINMAX: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_tuple_minmax", &[PI64, PI8, PI8], Some(RI64), false);
/// rt_set_minmax(set: *mut Obj, is_min: i8, elem_kind: i8) -> i64
pub static RT_SET_MINMAX: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_set_minmax", &[PI64, PI8, PI8], Some(RI64), false);
/// rt_dict_minmax(dict: *mut Obj, is_min: i8, elem_kind: i8) -> i64
pub static RT_DICT_MINMAX: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_dict_minmax", &[PI64, PI8, PI8], Some(RI64), false);
/// rt_str_minmax(str: *mut Obj, is_min: i8, elem_kind: i8) -> i64
pub static RT_STR_MINMAX: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_str_minmax", &[PI64, PI8, PI8], Some(RI64), false);
/// rt_list_minmax_with_key(list: *mut Obj, key_fn: i64, elem_tag: i64, captures: i64, count: i64, is_min: i8) -> *mut Obj
pub static RT_LIST_MINMAX_WITH_KEY: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_list_minmax_with_key",
    &[PI64, PI64, PI64, PI64, PI64, PI8],
    Some(RI64),
    true,
);
/// rt_tuple_minmax_with_key(tuple: *mut Obj, key_fn: i64, elem_tag: i64, captures: i64, count: i64, is_min: i8) -> *mut Obj
pub static RT_TUPLE_MINMAX_WITH_KEY: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_tuple_minmax_with_key",
    &[PI64, PI64, PI64, PI64, PI64, PI8],
    Some(RI64),
    true,
);
/// rt_set_minmax_with_key(set: *mut Obj, key_fn: i64, elem_tag: i64, captures: i64, count: i64, is_min: i8) -> *mut Obj
pub static RT_SET_MINMAX_WITH_KEY: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_set_minmax_with_key",
    &[PI64, PI64, PI64, PI64, PI64, PI8],
    Some(RI64),
    true,
);
/// rt_dict_minmax_with_key(dict: *mut Obj, key_fn: i64, elem_tag: i64, captures: i64, count: i64, is_min: i8) -> *mut Obj
pub static RT_DICT_MINMAX_WITH_KEY: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_dict_minmax_with_key",
    &[PI64, PI64, PI64, PI64, PI64, PI8],
    Some(RI64),
    true,
);
/// rt_str_minmax_with_key(str: *mut Obj, key_fn: i64, elem_tag: i64, captures: i64, count: i64, is_min: i8) -> *mut Obj
pub static RT_STR_MINMAX_WITH_KEY: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_str_minmax_with_key",
    &[PI64, PI64, PI64, PI64, PI64, PI8],
    Some(RI64),
    true,
);

// ===== Conversion operations =====

/// rt_int_to_str(value: i64) -> *mut Obj
pub static RT_INT_TO_STR: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_int_to_str");
/// rt_float_to_str(value: f64) -> *mut Obj
pub static RT_FLOAT_TO_STR: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_float_to_str", &[PF64], Some(RI64), true);
/// rt_bool_to_str(value: i8) -> *mut Obj
pub static RT_BOOL_TO_STR: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_bool_to_str", &[PI8], Some(RI64), true);
/// rt_none_to_str() -> *mut Obj
pub static RT_NONE_TO_STR: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_none_to_str", &[], Some(RI64), true);
/// rt_str_to_int(s: *mut Obj) -> i64
pub static RT_STR_TO_INT: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_str_to_int");
/// rt_str_to_float(s: *mut Obj) -> f64
pub static RT_STR_TO_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_str_to_float", &[PI64], Some(RF64), false);
/// rt_str_to_int_with_base(s: *mut Obj, base: i64) -> i64
pub static RT_STR_TO_INT_WITH_BASE: RuntimeFuncDef =
    RuntimeFuncDef::binary_to_i64("rt_str_to_int_with_base");
/// rt_str_contains(haystack: *mut Obj, needle: *mut Obj) -> i8
pub static RT_STR_CONTAINS: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_str_contains", &[PI64, PI64], Some(RI8), true);
/// rt_int_to_bin(n: i64) -> *mut Obj
pub static RT_INT_TO_BIN: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_int_to_bin");
/// rt_int_to_hex(n: i64) -> *mut Obj
pub static RT_INT_TO_HEX: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_int_to_hex");
/// rt_int_to_oct(n: i64) -> *mut Obj
pub static RT_INT_TO_OCT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_int_to_oct");
/// rt_int_fmt_bin(n: i64) -> *mut Obj
pub static RT_INT_FMT_BIN: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_int_fmt_bin");
/// rt_int_fmt_hex(n: i64) -> *mut Obj
pub static RT_INT_FMT_HEX: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_int_fmt_hex");
/// rt_int_fmt_hex_upper(n: i64) -> *mut Obj
pub static RT_INT_FMT_HEX_UPPER: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_int_fmt_hex_upper");
/// rt_int_fmt_oct(n: i64) -> *mut Obj
pub static RT_INT_FMT_OCT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_int_fmt_oct");
/// rt_int_fmt_grouped(n: i64, sep: i64) -> *mut Obj
pub static RT_INT_FMT_GROUPED: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_int_fmt_grouped");
/// rt_float_fmt_grouped(f: f64, precision: i64, sep: i64) -> *mut Obj
pub static RT_FLOAT_FMT_GROUPED: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_float_fmt_grouped",
    &[PF64, PI64, PI64],
    Some(RI64),
    true,
);
/// rt_type_name(obj: *mut Obj) -> *mut Obj
pub static RT_TYPE_NAME: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_type_name");
/// rt_type_name_extract(type_str: *mut Obj) -> *mut Obj
pub static RT_TYPE_NAME_EXTRACT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_type_name_extract");
/// rt_exc_class_name(instance: *mut Obj) -> *mut Obj
pub static RT_EXC_CLASS_NAME: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_exc_class_name");
/// rt_format_value(value: *mut Obj, spec: *mut Obj) -> *mut Obj
pub static RT_FORMAT_VALUE: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_format_value");

// ===== ToStringRepr operations (repr/ascii) =====
// ToStringRepr(target_kind, format) → static defs for all valid combinations.
// Function name pattern: "{format.prefix()}{target_kind.suffix()}"

/// rt_repr_int(value: i64) -> *mut Obj
pub static RT_REPR_INT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_repr_int");
/// rt_repr_float(value: f64) -> *mut Obj
pub static RT_REPR_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_repr_float", &[PF64], Some(RI64), true);
/// rt_repr_bool(value: i8) -> *mut Obj
pub static RT_REPR_BOOL: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_repr_bool", &[PI8], Some(RI64), true);
/// rt_repr_none() -> *mut Obj
pub static RT_REPR_NONE: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_repr_none", &[], Some(RI64), true);
/// rt_repr_str(s: *mut Obj) -> *mut Obj
pub static RT_REPR_STR: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_repr_str");
/// rt_repr_bytes(b: *mut Obj) -> *mut Obj
pub static RT_REPR_BYTES: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_repr_bytes");
/// rt_repr_collection(obj: *mut Obj) -> *mut Obj
pub static RT_REPR_COLLECTION: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_repr_collection");
/// rt_ascii_int(value: i64) -> *mut Obj (same as repr for int)
pub static RT_ASCII_INT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_ascii_int");
/// rt_ascii_float(value: f64) -> *mut Obj (same as repr for float)
pub static RT_ASCII_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_ascii_float", &[PF64], Some(RI64), true);
/// rt_ascii_bool(value: i8) -> *mut Obj (same as repr for bool)
pub static RT_ASCII_BOOL: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_ascii_bool", &[PI8], Some(RI64), true);
/// rt_ascii_none() -> *mut Obj (same as repr for None)
pub static RT_ASCII_NONE: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_ascii_none", &[], Some(RI64), true);
/// rt_ascii_str(s: *mut Obj) -> *mut Obj
pub static RT_ASCII_STR: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_ascii_str");
/// rt_ascii_bytes(b: *mut Obj) -> *mut Obj
pub static RT_ASCII_BYTES: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_ascii_bytes");
/// rt_ascii_collection(obj: *mut Obj) -> *mut Obj
pub static RT_ASCII_COLLECTION: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_ascii_collection");

// ===== String operations =====

/// rt_str_data(s: *mut Obj) -> i64 (data pointer, not GC-tracked)
pub static RT_STR_DATA: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_str_data");
/// rt_str_len(s: *mut Obj) -> i64 (byte length, not GC-tracked)
pub static RT_STR_LEN: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_str_len");
/// rt_str_len_int(s: *mut Obj) -> i64 (codepoint length for len() builtin, GC-tracked)
pub static RT_STR_LEN_INT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_str_len_int");
/// rt_str_concat(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_STR_CONCAT: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_str_concat");
/// rt_str_slice(s: *mut Obj, start: i64, stop: i64) -> *mut Obj
pub static RT_STR_SLICE: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_str_slice");
/// rt_str_slice_step(s: *mut Obj, start: i64, stop: i64, step: i64) -> *mut Obj
pub static RT_STR_SLICE_STEP: RuntimeFuncDef = RuntimeFuncDef::ptr_quaternary("rt_str_slice_step");
/// rt_str_getchar(s: *mut Obj, byte_index: i64) -> *mut Obj
pub static RT_STR_GETCHAR: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_str_getchar");
/// rt_str_subscript(s: *mut Obj, char_index: i64) -> *mut Obj
pub static RT_STR_SUBSCRIPT: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_str_subscript");
/// rt_str_mul(s: *mut Obj, count: i64) -> *mut Obj
pub static RT_STR_MUL: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_str_mul");
/// rt_str_upper(s: *mut Obj) -> *mut Obj
pub static RT_STR_UPPER: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_str_upper");
/// rt_str_lower(s: *mut Obj) -> *mut Obj
pub static RT_STR_LOWER: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_str_lower");
/// rt_str_strip(s: *mut Obj) -> *mut Obj
pub static RT_STR_STRIP: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_str_strip");
/// rt_str_startswith(s: *mut Obj, prefix: *mut Obj) -> i8
pub static RT_STR_STARTSWITH: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_str_startswith");
/// rt_str_endswith(s: *mut Obj, suffix: *mut Obj) -> i8
pub static RT_STR_ENDSWITH: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_str_endswith");
/// rt_str_search(s: *mut Obj, sub: *mut Obj, op_tag: i8) -> i64 (find variant)
pub static RT_STR_FIND: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_str_search", &[PI64, PI64, PI8], Some(RI64), true);
/// rt_str_search(s: *mut Obj, sub: *mut Obj, op_tag: i8) -> i64 (rfind variant)
pub static RT_STR_RFIND: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_str_search", &[PI64, PI64, PI8], Some(RI64), true);
/// rt_str_search(s: *mut Obj, sub: *mut Obj, op_tag: i8) -> i64 (index variant)
pub static RT_STR_INDEX: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_str_search", &[PI64, PI64, PI8], Some(RI64), true);
/// rt_str_search(s: *mut Obj, sub: *mut Obj, op_tag: i8) -> i64 (rindex variant)
pub static RT_STR_RINDEX: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_str_search", &[PI64, PI64, PI8], Some(RI64), true);
/// rt_str_rsplit(s: *mut Obj, sep: *mut Obj, maxsplit: i64) -> *mut Obj
pub static RT_STR_RSPLIT: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_str_rsplit");
/// rt_str_isascii(s: *mut Obj) -> i8
pub static RT_STR_ISASCII: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_str_isascii");
/// rt_str_encode(s: *mut Obj, encoding: *mut Obj) -> *mut Obj
pub static RT_STR_ENCODE: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_str_encode");
/// rt_str_replace(s: *mut Obj, old: *mut Obj, new: *mut Obj) -> *mut Obj
pub static RT_STR_REPLACE: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_str_replace");
/// rt_str_count(s: *mut Obj, sub: *mut Obj) -> i64
pub static RT_STR_COUNT: RuntimeFuncDef = RuntimeFuncDef::binary_to_i64("rt_str_count");
/// rt_str_split(s: *mut Obj, sep: *mut Obj, maxsplit: i64) -> *mut Obj
pub static RT_STR_SPLIT: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_str_split");
/// rt_str_join(sep: *mut Obj, list: *mut Obj) -> *mut Obj
pub static RT_STR_JOIN: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_str_join");
/// rt_str_lstrip(s: *mut Obj, chars: *mut Obj) -> *mut Obj
pub static RT_STR_LSTRIP: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_str_lstrip");
/// rt_str_rstrip(s: *mut Obj, chars: *mut Obj) -> *mut Obj
pub static RT_STR_RSTRIP: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_str_rstrip");
/// rt_str_title(s: *mut Obj) -> *mut Obj
pub static RT_STR_TITLE: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_str_title");
/// rt_str_capitalize(s: *mut Obj) -> *mut Obj
pub static RT_STR_CAPITALIZE: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_str_capitalize");
/// rt_str_swapcase(s: *mut Obj) -> *mut Obj
pub static RT_STR_SWAPCASE: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_str_swapcase");
/// rt_str_center(s: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj
pub static RT_STR_CENTER: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_str_center");
/// rt_str_ljust(s: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj
pub static RT_STR_LJUST: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_str_ljust");
/// rt_str_rjust(s: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj
pub static RT_STR_RJUST: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_str_rjust");
/// rt_str_zfill(s: *mut Obj, width: i64) -> *mut Obj
pub static RT_STR_ZFILL: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_str_zfill");
/// rt_str_isdigit(s: *mut Obj) -> i8
pub static RT_STR_ISDIGIT: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_str_isdigit");
/// rt_str_isalpha(s: *mut Obj) -> i8
pub static RT_STR_ISALPHA: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_str_isalpha");
/// rt_str_isalnum(s: *mut Obj) -> i8
pub static RT_STR_ISALNUM: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_str_isalnum");
/// rt_str_isspace(s: *mut Obj) -> i8
pub static RT_STR_ISSPACE: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_str_isspace");
/// rt_str_isupper(s: *mut Obj) -> i8
pub static RT_STR_ISUPPER: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_str_isupper");
/// rt_str_islower(s: *mut Obj) -> i8
pub static RT_STR_ISLOWER: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_str_islower");
/// rt_str_removeprefix(s: *mut Obj, prefix: *mut Obj) -> *mut Obj
pub static RT_STR_REMOVEPREFIX: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_str_removeprefix");
/// rt_str_removesuffix(s: *mut Obj, suffix: *mut Obj) -> *mut Obj
pub static RT_STR_REMOVESUFFIX: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_str_removesuffix");
/// rt_str_splitlines(s: *mut Obj) -> *mut Obj
pub static RT_STR_SPLITLINES: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_str_splitlines");
/// rt_str_partition(s: *mut Obj, sep: *mut Obj) -> *mut Obj
pub static RT_STR_PARTITION: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_str_partition");
/// rt_str_rpartition(s: *mut Obj, sep: *mut Obj) -> *mut Obj
pub static RT_STR_RPARTITION: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_str_rpartition");
/// rt_str_expandtabs(s: *mut Obj, tabsize: i64) -> *mut Obj
pub static RT_STR_EXPANDTABS: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_str_expandtabs");
/// rt_make_string_builder(capacity: i64) -> *mut Obj
pub static RT_MAKE_STRING_BUILDER: RuntimeFuncDef =
    RuntimeFuncDef::ptr_unary("rt_make_string_builder");
/// rt_string_builder_append(builder: *mut Obj, s: *mut Obj) -> void
pub static RT_STRING_BUILDER_APPEND: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_string_builder_append", &[PI64, PI64]);
/// rt_string_builder_to_str(builder: *mut Obj) -> *mut Obj
pub static RT_STRING_BUILDER_TO_STR: RuntimeFuncDef =
    RuntimeFuncDef::ptr_unary("rt_string_builder_to_str");

// ===== Print operations =====

/// rt_print_newline() -> void
pub static RT_PRINT_NEWLINE: RuntimeFuncDef = RuntimeFuncDef::void("rt_print_newline", &[]);
/// rt_print_sep() -> void
pub static RT_PRINT_SEP: RuntimeFuncDef = RuntimeFuncDef::void("rt_print_sep", &[]);
/// rt_print_flush() -> void
pub static RT_PRINT_FLUSH: RuntimeFuncDef = RuntimeFuncDef::void("rt_print_flush", &[]);
/// rt_print_set_stderr() -> void
pub static RT_PRINT_SET_STDERR: RuntimeFuncDef = RuntimeFuncDef::void("rt_print_set_stderr", &[]);
/// rt_print_set_stdout() -> void
pub static RT_PRINT_SET_STDOUT: RuntimeFuncDef = RuntimeFuncDef::void("rt_print_set_stdout", &[]);
/// rt_input(prompt: *mut Obj) -> *mut Obj
pub static RT_INPUT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_input");
/// rt_print_int_value(value: i64) -> void
pub static RT_PRINT_INT: RuntimeFuncDef = RuntimeFuncDef::void("rt_print_int_value", &[PI64]);
/// rt_print_float_value(value: f64) -> void
pub static RT_PRINT_FLOAT: RuntimeFuncDef = RuntimeFuncDef::void("rt_print_float_value", &[PF64]);
/// rt_print_bool_value(value: i8) -> void
pub static RT_PRINT_BOOL: RuntimeFuncDef = RuntimeFuncDef::void("rt_print_bool_value", &[PI8]);
/// rt_print_str_obj(s: *mut Obj) -> void
pub static RT_PRINT_STR_OBJ: RuntimeFuncDef = RuntimeFuncDef::void("rt_print_str_obj", &[PI64]);
/// rt_print_bytes_obj(b: *mut Obj) -> void
pub static RT_PRINT_BYTES_OBJ: RuntimeFuncDef = RuntimeFuncDef::void("rt_print_bytes_obj", &[PI64]);
/// rt_print_obj(obj: *mut Obj) -> void
pub static RT_PRINT_OBJ: RuntimeFuncDef = RuntimeFuncDef::void("rt_print_obj", &[PI64]);
/// rt_assert_fail_obj(msg: *mut Obj) -> void (diverges, but declared void for codegen)
pub static RT_ASSERT_FAIL_OBJ: RuntimeFuncDef = RuntimeFuncDef::void("rt_assert_fail_obj", &[PI64]);

// ===== Iterator operations =====

// --- MakeIterator: forward variants ---
/// rt_iter_list(container: *mut Obj) -> *mut Obj
pub static RT_ITER_LIST: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_iter_list");
/// rt_iter_tuple(container: *mut Obj) -> *mut Obj
pub static RT_ITER_TUPLE: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_iter_tuple");
/// rt_iter_dict(container: *mut Obj) -> *mut Obj
pub static RT_ITER_DICT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_iter_dict");
/// rt_iter_str(container: *mut Obj) -> *mut Obj
pub static RT_ITER_STR: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_iter_str");
/// rt_iter_range(start: i64, stop: i64, step: i64) -> *mut Obj
pub static RT_ITER_RANGE: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_iter_range");
/// rt_iter_set(container: *mut Obj) -> *mut Obj
pub static RT_ITER_SET: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_iter_set");
/// rt_iter_bytes(container: *mut Obj) -> *mut Obj
pub static RT_ITER_BYTES: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_iter_bytes");
/// rt_iter_generator(container: *mut Obj) -> *mut Obj
pub static RT_ITER_GENERATOR: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_iter_generator");

// --- MakeIterator: reversed variants ---
/// rt_iter_reversed_list(container: *mut Obj) -> *mut Obj
pub static RT_ITER_REVERSED_LIST: RuntimeFuncDef =
    RuntimeFuncDef::ptr_unary("rt_iter_reversed_list");
/// rt_iter_reversed_tuple(container: *mut Obj) -> *mut Obj
pub static RT_ITER_REVERSED_TUPLE: RuntimeFuncDef =
    RuntimeFuncDef::ptr_unary("rt_iter_reversed_tuple");
/// rt_iter_reversed_dict(container: *mut Obj) -> *mut Obj
pub static RT_ITER_REVERSED_DICT: RuntimeFuncDef =
    RuntimeFuncDef::ptr_unary("rt_iter_reversed_dict");
/// rt_iter_reversed_str(container: *mut Obj) -> *mut Obj
pub static RT_ITER_REVERSED_STR: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_iter_reversed_str");
/// rt_iter_reversed_range(start: i64, stop: i64, step: i64) -> *mut Obj
pub static RT_ITER_REVERSED_RANGE: RuntimeFuncDef =
    RuntimeFuncDef::ptr_ternary("rt_iter_reversed_range");
/// rt_iter_reversed_set(container: *mut Obj) -> *mut Obj
pub static RT_ITER_REVERSED_SET: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_iter_reversed_set");
/// rt_iter_reversed_bytes(container: *mut Obj) -> *mut Obj
pub static RT_ITER_REVERSED_BYTES: RuntimeFuncDef =
    RuntimeFuncDef::ptr_unary("rt_iter_reversed_bytes");
/// rt_iter_reversed_generator(container: *mut Obj) -> *mut Obj
pub static RT_ITER_REVERSED_GENERATOR: RuntimeFuncDef =
    RuntimeFuncDef::ptr_unary("rt_iter_reversed_generator");

// --- Iterator core ops ---
/// rt_iter_next(iter: *mut Obj) -> *mut Obj
pub static RT_ITER_NEXT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_iter_next");
/// rt_iter_next_no_exc(iter: *mut Obj) -> *mut Obj
pub static RT_ITER_NEXT_NO_EXC: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_iter_next_no_exc");
/// rt_iter_is_exhausted(iter: *mut Obj) -> i8
pub static RT_ITER_IS_EXHAUSTED: RuntimeFuncDef =
    RuntimeFuncDef::unary_to_i8("rt_iter_is_exhausted");
/// rt_iter_enumerate(inner: *mut Obj, start: i64) -> *mut Obj
pub static RT_ITER_ENUMERATE: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_iter_enumerate");

// --- Sorted: no key, no elem_tag (List, Tuple, Str) ---
/// rt_sorted_list(container: *mut Obj, reverse: i64) -> *mut Obj
pub static RT_SORTED_LIST: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_sorted_list");
/// rt_sorted_tuple(container: *mut Obj, reverse: i64) -> *mut Obj
pub static RT_SORTED_TUPLE: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_sorted_tuple");
/// rt_sorted_str(container: *mut Obj, reverse: i64) -> *mut Obj
pub static RT_SORTED_STR: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_sorted_str");

// --- Sorted: no key, with elem_tag (Set, Dict) ---
/// rt_sorted_set(container: *mut Obj, reverse: i64, elem_tag: u8) -> *mut Obj
pub static RT_SORTED_SET: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_sorted_set", &[PI64, PI64, PI8], Some(RI64), true);
/// rt_sorted_dict(container: *mut Obj, reverse: i64, elem_tag: u8) -> *mut Obj
pub static RT_SORTED_DICT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_sorted_dict", &[PI64, PI64, PI8], Some(RI64), true);

// --- Sorted: range (special args) ---
/// rt_sorted_range(start: i64, stop: i64, step: i64, reverse: i64) -> *mut Obj
pub static RT_SORTED_RANGE: RuntimeFuncDef = RuntimeFuncDef::ptr_quaternary("rt_sorted_range");

// --- Sorted: with key (container, reverse, key_fn, elem_tag, captures, capture_count) ---
/// rt_sorted_list_with_key(container, reverse, key_fn, elem_tag, captures, capture_count) -> *mut Obj
pub static RT_SORTED_LIST_WITH_KEY: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_sorted_list_with_key",
    &[PI64, PI64, PI64, PI64, PI64, PI64],
    Some(RI64),
    true,
);
/// rt_sorted_tuple_with_key(container, reverse, key_fn, elem_tag, captures, capture_count) -> *mut Obj
pub static RT_SORTED_TUPLE_WITH_KEY: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_sorted_tuple_with_key",
    &[PI64, PI64, PI64, PI64, PI64, PI64],
    Some(RI64),
    true,
);
/// rt_sorted_str_with_key(container, reverse, key_fn, elem_tag, captures, capture_count) -> *mut Obj
pub static RT_SORTED_STR_WITH_KEY: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_sorted_str_with_key",
    &[PI64, PI64, PI64, PI64, PI64, PI64],
    Some(RI64),
    true,
);
/// rt_sorted_set_with_key(container, reverse, key_fn, elem_tag, captures, capture_count) -> *mut Obj
pub static RT_SORTED_SET_WITH_KEY: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_sorted_set_with_key",
    &[PI64, PI64, PI64, PI64, PI64, PI64],
    Some(RI64),
    true,
);
/// rt_sorted_dict_with_key(container, reverse, key_fn, elem_tag, captures, capture_count) -> *mut Obj
pub static RT_SORTED_DICT_WITH_KEY: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_sorted_dict_with_key",
    &[PI64, PI64, PI64, PI64, PI64, PI64],
    Some(RI64),
    true,
);

// --- Zip operations ---
/// rt_zip_new(iter1: *mut Obj, iter2: *mut Obj) -> *mut Obj
pub static RT_ZIP_NEW: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_zip_new");
/// rt_zip3_new(iter1: *mut Obj, iter2: *mut Obj, iter3: *mut Obj) -> *mut Obj
pub static RT_ZIP3_NEW: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_zip3_new");
/// rt_zipn_new(iters: *mut Obj, num_iters: i64) -> *mut Obj
pub static RT_ZIPN_NEW: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_zipn_new");
/// rt_zip_next(zip: *mut Obj) -> *mut Obj
pub static RT_ZIP_NEXT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_zip_next");

// --- Map/Filter/Reduce ---
/// rt_map_new(func_ptr: i64, iter: *mut Obj, captures: *mut Obj, capture_count: i64) -> *mut Obj
pub static RT_MAP_NEW: RuntimeFuncDef = RuntimeFuncDef::ptr_quaternary("rt_map_new");
/// rt_filter_new(func_ptr: i64, iter: *mut Obj, elem_tag: i64, captures: *mut Obj, capture_count: i64) -> *mut Obj
pub static RT_FILTER_NEW: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_filter_new",
    &[PI64, PI64, PI64, PI64, PI64],
    Some(RI64),
    true,
);
/// rt_reduce(func_ptr, iter, initial, has_initial, captures, capture_count) -> *mut Obj
pub static RT_REDUCE: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_reduce",
    &[PI64, PI64, PI64, PI64, PI64, PI64],
    Some(RI64),
    true,
);

// --- itertools ---
/// rt_chain_new(iters: *mut Obj, num_iters: i64) -> *mut Obj
pub static RT_CHAIN_NEW: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_chain_new");
/// rt_islice_new(iter: *mut Obj, start: i64, stop: i64, step: i64) -> *mut Obj
pub static RT_ISLICE_NEW: RuntimeFuncDef = RuntimeFuncDef::ptr_quaternary("rt_islice_new");

// ===== Generator operations =====

/// rt_make_generator(func_id: u32, num_locals: u32) -> *mut Obj
pub static RT_MAKE_GENERATOR: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_make_generator", &[PI32, PI32], Some(RI64), true);
/// rt_generator_get_state(gen: *mut Obj) -> u32
pub static RT_GENERATOR_GET_STATE: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_generator_get_state", &[PI64], Some(RI32), false);
/// rt_generator_set_state(gen: *mut Obj, state: u32) -> void
pub static RT_GENERATOR_SET_STATE: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_generator_set_state", &[PI64, PI32]);
/// rt_generator_get_local(gen: *mut Obj, index: u32) -> i64
pub static RT_GENERATOR_GET_LOCAL: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_generator_get_local", &[PI64, PI32], Some(RI64), false);
/// rt_generator_set_local(gen: *mut Obj, index: u32, value: i64) -> void
pub static RT_GENERATOR_SET_LOCAL: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_generator_set_local", &[PI64, PI32, PI64]);
/// rt_generator_get_local_ptr(gen: *mut Obj, index: u32) -> *mut Obj
pub static RT_GENERATOR_GET_LOCAL_PTR: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_generator_get_local_ptr",
    &[PI64, PI32],
    Some(RI64),
    true,
);
/// rt_generator_set_local_ptr(gen: *mut Obj, index: u32, value: *mut Obj) -> void
pub static RT_GENERATOR_SET_LOCAL_PTR: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_generator_set_local_ptr", &[PI64, PI32, PI64]);
/// rt_generator_set_local_type(gen: *mut Obj, index: u32, type_tag: u8) -> void
pub static RT_GENERATOR_SET_LOCAL_TYPE: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_generator_set_local_type", &[PI64, PI32, PI8]);
/// rt_generator_set_exhausted(gen: *mut Obj) -> void
pub static RT_GENERATOR_SET_EXHAUSTED: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_generator_set_exhausted", &[PI64]);
/// rt_generator_is_exhausted(gen: *mut Obj) -> i8
pub static RT_GENERATOR_IS_EXHAUSTED: RuntimeFuncDef =
    RuntimeFuncDef::unary_to_i8("rt_generator_is_exhausted");
/// rt_generator_send(gen: *mut Obj, value: i64) -> *mut Obj
pub static RT_GENERATOR_SEND: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_generator_send");
/// rt_generator_get_sent_value(gen: *mut Obj) -> i64
pub static RT_GENERATOR_GET_SENT_VALUE: RuntimeFuncDef =
    RuntimeFuncDef::unary_to_i64("rt_generator_get_sent_value");
/// rt_generator_close(gen: *mut Obj) -> void
pub static RT_GENERATOR_CLOSE: RuntimeFuncDef = RuntimeFuncDef::void("rt_generator_close", &[PI64]);
/// rt_generator_is_closing(gen: *mut Obj) -> i8
pub static RT_GENERATOR_IS_CLOSING: RuntimeFuncDef =
    RuntimeFuncDef::unary_to_i8("rt_generator_is_closing");

// ===== Global variable storage =====
// rt_global_get_{int,float,bool,ptr}(var_id: i32) -> value
pub static RT_GLOBAL_GET_INT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_global_get_int", &[PI32], Some(RI64), false);
pub static RT_GLOBAL_GET_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_global_get_float", &[PI32], Some(RF64), false);
pub static RT_GLOBAL_GET_BOOL: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_global_get_bool", &[PI32], Some(RI8), false);
pub static RT_GLOBAL_GET_PTR: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_global_get_ptr", &[PI32], Some(RI64), true);
// rt_global_set_{int,float,bool,ptr}(var_id: i32, value) -> void
pub static RT_GLOBAL_SET_INT: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_global_set_int", &[PI32, PI64]);
pub static RT_GLOBAL_SET_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_global_set_float", &[PI32, PF64]);
pub static RT_GLOBAL_SET_BOOL: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_global_set_bool", &[PI32, PI8]);
pub static RT_GLOBAL_SET_PTR: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_global_set_ptr", &[PI32, PI64]);

// ===== Class attribute storage =====
// rt_class_attr_get_{int,float,bool,ptr}(class_id: i8, attr_idx: i32) -> value
pub static RT_CLASS_ATTR_GET_INT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_class_attr_get_int", &[PI8, PI32], Some(RI64), false);
pub static RT_CLASS_ATTR_GET_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_class_attr_get_float", &[PI8, PI32], Some(RF64), false);
pub static RT_CLASS_ATTR_GET_BOOL: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_class_attr_get_bool", &[PI8, PI32], Some(RI8), false);
pub static RT_CLASS_ATTR_GET_PTR: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_class_attr_get_ptr", &[PI8, PI32], Some(RI64), true);
// rt_class_attr_set_{int,float,bool,ptr}(class_id: i8, attr_idx: i32, value) -> void
pub static RT_CLASS_ATTR_SET_INT: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_class_attr_set_int", &[PI8, PI32, PI64]);
pub static RT_CLASS_ATTR_SET_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_class_attr_set_float", &[PI8, PI32, PF64]);
pub static RT_CLASS_ATTR_SET_BOOL: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_class_attr_set_bool", &[PI8, PI32, PI8]);
pub static RT_CLASS_ATTR_SET_PTR: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_class_attr_set_ptr", &[PI8, PI32, PI64]);

// ===== Cell storage (nonlocal variables) =====
// rt_make_cell_{int,float,bool,ptr}(value) -> *mut Obj
pub static RT_MAKE_CELL_INT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_make_cell_int", &[PI64], Some(RI64), true);
pub static RT_MAKE_CELL_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_make_cell_float", &[PF64], Some(RI64), true);
pub static RT_MAKE_CELL_BOOL: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_make_cell_bool", &[PI8], Some(RI64), true);
pub static RT_MAKE_CELL_PTR: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_make_cell_ptr", &[PI64], Some(RI64), true);
// rt_cell_get_{int,float,bool,ptr}(cell: *mut Obj) -> value
pub static RT_CELL_GET_INT: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_cell_get_int");
pub static RT_CELL_GET_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_cell_get_float", &[PI64], Some(RF64), false);
pub static RT_CELL_GET_BOOL: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_cell_get_bool");
pub static RT_CELL_GET_PTR: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_cell_get_ptr");
// rt_cell_set_{int,float,bool,ptr}(cell: *mut Obj, value) -> void
pub static RT_CELL_SET_INT: RuntimeFuncDef = RuntimeFuncDef::void("rt_cell_set_int", &[PI64, PI64]);
pub static RT_CELL_SET_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_cell_set_float", &[PI64, PF64]);
pub static RT_CELL_SET_BOOL: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_cell_set_bool", &[PI64, PI8]);
pub static RT_CELL_SET_PTR: RuntimeFuncDef = RuntimeFuncDef::void("rt_cell_set_ptr", &[PI64, PI64]);

// ===== Instance operations =====
/// rt_make_instance(class_id: i8, field_count: i64) -> *mut Obj
pub static RT_MAKE_INSTANCE: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_make_instance", &[PI8, PI64], Some(RI64), true);
/// rt_instance_get_field(inst: *mut Obj, offset: i64) -> i64
pub static RT_INSTANCE_GET_FIELD: RuntimeFuncDef =
    RuntimeFuncDef::ptr_binary("rt_instance_get_field");
/// rt_instance_set_field(inst: *mut Obj, offset: i64, value: i64) -> void
pub static RT_INSTANCE_SET_FIELD: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_instance_set_field", &[PI64, PI64, PI64]);
/// rt_get_type_tag(obj: *mut Obj) -> i64
pub static RT_GET_TYPE_TAG: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_get_type_tag");
/// rt_isinstance_class(obj: *mut Obj, class_id: i64) -> i8
pub static RT_ISINSTANCE_CLASS: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_isinstance_class", &[PI64, PI64], Some(RI8), false);
/// rt_isinstance_class_inherited(obj: *mut Obj, target_class_id: i64) -> i8
pub static RT_ISINSTANCE_CLASS_INHERITED: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_isinstance_class_inherited",
    &[PI64, PI64],
    Some(RI8),
    false,
);
/// rt_register_class(class_id: i8, parent_class_id: i8) -> void
pub static RT_REGISTER_CLASS: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_register_class", &[PI8, PI8]);
/// rt_register_class_fields(class_id: i8, heap_field_mask: i64) -> void
pub static RT_REGISTER_CLASS_FIELDS: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_register_class_fields", &[PI8, PI64]);
/// rt_register_class_field_count(class_id: i8, field_count: i64) -> void
pub static RT_REGISTER_CLASS_FIELD_COUNT: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_register_class_field_count", &[PI8, PI64]);
/// rt_register_method_name(class_id: i64, name_hash: i64, slot: i64) -> void
pub static RT_REGISTER_METHOD_NAME: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_register_method_name", &[PI64, PI64, PI64]);
/// rt_object_new(class_id: i8) -> *mut Obj
pub static RT_OBJECT_NEW: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_object_new", &[PI8], Some(RI64), false);
/// rt_register_del_func(class_id: i8, func_ptr: i64) -> void
pub static RT_REGISTER_DEL_FUNC: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_register_del_func", &[PI8, PI64]);
/// rt_register_copy_func(class_id: i8, func_ptr: i64) -> void
pub static RT_REGISTER_COPY_FUNC: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_register_copy_func", &[PI8, PI64]);
/// rt_register_deepcopy_func(class_id: i8, func_ptr: i64) -> void
pub static RT_REGISTER_DEEPCOPY_FUNC: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_register_deepcopy_func", &[PI8, PI64]);
/// rt_issubclass(child_tag: i64, parent_tag: i64) -> i8
pub static RT_ISSUBCLASS: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_issubclass", &[PI64, PI64], Some(RI8), false);
