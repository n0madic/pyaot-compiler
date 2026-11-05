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

/// Stage B.2 of Strong-Typed MIR Rewrite plan v2: MirType semantic
/// annotation for runtime function parameters/return.
///
/// Distinguishes runtime callers' interpretation of an `i64` register —
/// is it raw bits (e.g., array index), a tagged `Value`, or a heap
/// pointer? Cranelift sees them all as i64 but the MIR verifier needs
/// to know the semantic to do per-arg type validation.
///
/// Default mapping when not explicitly annotated (see
/// `infer_mir_semantic`):
/// * `ParamType::I64` → `MirSemantic::Raw` (conservative — caller may
///   store a raw integer here)
/// * `ParamType::F64` → `MirSemantic::Raw`
/// * `ParamType::I8` → `MirSemantic::Raw`
/// * `ParamType::I32` → `MirSemantic::Raw`
///
/// Functions that actually pass tagged Values or heap pointers in their
/// I64 slots MUST annotate explicitly to enable the verifier check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirSemantic {
    /// Raw register value matching the Cranelift type class.
    Raw,
    /// Tagged `Value` (any of TAG_PTR/TAG_INT/TAG_BOOL/TAG_NONE).
    Tagged,
    /// Heap pointer with statically-known shape. Includes the tag
    /// kind so the verifier knows the concrete heap shape expected.
    Heap(crate::tag_kinds::TypeTagKind),
}

impl MirSemantic {
    /// Default semantic for a given Cranelift register class. Used when
    /// the RuntimeFuncDef hasn't been annotated yet.
    pub const fn infer_param(pt: ParamType) -> Self {
        match pt {
            ParamType::I64 | ParamType::F64 | ParamType::I8 | ParamType::I32 => MirSemantic::Raw,
        }
    }

    /// Default semantic for a return type.
    pub const fn infer_return(rt: ReturnType) -> Self {
        match rt {
            ReturnType::I64 | ReturnType::F64 | ReturnType::I8 | ReturnType::I32 => {
                MirSemantic::Raw
            }
        }
    }
}

/// Declarative description of a runtime function's ABI.
///
/// Used by codegen to build a Cranelift signature and emit a call instruction
/// without per-variant match arms.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeFuncDef {
    /// Symbol name for linking (e.g., `"rt_list_append"`)
    pub symbol: &'static str,
    /// Parameter types in order (Cranelift register class).
    pub params: &'static [ParamType],
    /// Return type, or `None` for void functions.
    pub returns: Option<ReturnType>,
    /// Whether the return value is a GC-managed heap pointer.
    /// When `true`, codegen calls `update_gc_root_if_needed` after storing
    /// the result.
    pub gc_roots_result: bool,
    /// Stage B.2: explicit per-parameter MirSemantic annotation. Length
    /// must match `params` when populated. When `None`, the verifier
    /// uses `MirSemantic::infer_param` (defaults to Raw). Sub-agent
    /// migration progressively fills this for ~200 definitions to enable
    /// strict per-arg type validation in the verifier.
    pub mir_param_semantics: Option<&'static [MirSemantic]>,
    /// Stage B.2: explicit return MirSemantic annotation. When `None`,
    /// the verifier uses `MirSemantic::infer_return`.
    pub mir_return_semantic: Option<MirSemantic>,
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

// Stage B.2: well-known Tagged-passing semantic slices. Helpers use these
// when the "ptr_*" naming convention indicates the param/return is a
// tagged Value (`*mut Obj` cast to i64). Sub-agent migration can refine
// per-function later (Heap shape annotations when shape is known).
const TAGGED_1: &[MirSemantic] = &[MirSemantic::Tagged];
const TAGGED_2: &[MirSemantic] = &[MirSemantic::Tagged, MirSemantic::Tagged];
const TAGGED_3: &[MirSemantic] = &[
    MirSemantic::Tagged,
    MirSemantic::Tagged,
    MirSemantic::Tagged,
];
const TAGGED_4: &[MirSemantic] = &[
    MirSemantic::Tagged,
    MirSemantic::Tagged,
    MirSemantic::Tagged,
    MirSemantic::Tagged,
];
// Slice ABIs (`rt_*_slice[_step]`): a tagged object receiver followed by RAW i64
// start/stop[/step] bounds. The runtime reads the bounds as machine integers
// (with `i64::MIN`/`i64::MAX` sentinels for "default start/stop"), so they must
// be passed Raw — the generic `ptr_ternary`/`ptr_quaternary` Tagged default is
// wrong here (Phase 8E).
const SLICE_TERNARY: &[MirSemantic] = &[MirSemantic::Tagged, MirSemantic::Raw, MirSemantic::Raw];
const SLICE_QUATERNARY: &[MirSemantic] = &[
    MirSemantic::Tagged,
    MirSemantic::Raw,
    MirSemantic::Raw,
    MirSemantic::Raw,
];

impl RuntimeFuncDef {
    /// General constructor (no explicit MIR semantic). Verifier falls back
    /// to inferring from the Cranelift register class.
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
            mir_param_semantics: None,
            mir_return_semantic: None,
        }
    }

    /// Stage B.2: constructor with explicit MIR semantics for both
    /// params and return. Use when the function's argument/return
    /// interpretation differs from the default Cranelift register class
    /// inference (e.g., I64 register holds a Tagged Value rather than
    /// raw integer).
    pub const fn new_typed(
        symbol: &'static str,
        params: &'static [ParamType],
        returns: Option<ReturnType>,
        gc_roots_result: bool,
        mir_param_semantics: &'static [MirSemantic],
        mir_return_semantic: Option<MirSemantic>,
    ) -> Self {
        Self {
            symbol,
            params,
            returns,
            gc_roots_result,
            mir_param_semantics: Some(mir_param_semantics),
            mir_return_semantic,
        }
    }

    /// Unary: one I64 param, returns I64, GC-tracked.
    /// Typical for `(obj) -> obj` functions — both param and return
    /// carry a Tagged Value.
    pub const fn ptr_unary(symbol: &'static str) -> Self {
        Self {
            symbol,
            params: &[PI64],
            returns: Some(RI64),
            gc_roots_result: true,
            mir_param_semantics: Some(TAGGED_1),
            mir_return_semantic: Some(MirSemantic::Tagged),
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
            mir_param_semantics: Some(TAGGED_2),
            mir_return_semantic: Some(MirSemantic::Tagged),
        }
    }

    /// Ternary: three I64 params, returns I64, GC-tracked.
    pub const fn ptr_ternary(symbol: &'static str) -> Self {
        Self {
            symbol,
            params: &[PI64, PI64, PI64],
            returns: Some(RI64),
            gc_roots_result: true,
            mir_param_semantics: Some(TAGGED_3),
            mir_return_semantic: Some(MirSemantic::Tagged),
        }
    }

    /// Quaternary: four I64 params, returns I64, GC-tracked.
    pub const fn ptr_quaternary(symbol: &'static str) -> Self {
        Self {
            symbol,
            params: &[PI64, PI64, PI64, PI64],
            returns: Some(RI64),
            gc_roots_result: true,
            mir_param_semantics: Some(TAGGED_4),
            mir_return_semantic: Some(MirSemantic::Tagged),
        }
    }

    /// Void function (no return value).
    pub const fn void(symbol: &'static str, params: &'static [ParamType]) -> Self {
        Self {
            symbol,
            params,
            returns: None,
            gc_roots_result: false,
            mir_param_semantics: None,
            mir_return_semantic: None,
        }
    }

    /// Unary returning raw i64 (not GC-tracked).
    /// Typical for `len()`, hash, etc. — param is Tagged Value, return is Raw.
    pub const fn unary_to_i64(symbol: &'static str) -> Self {
        Self {
            symbol,
            params: &[PI64],
            returns: Some(RI64),
            gc_roots_result: false,
            mir_param_semantics: Some(TAGGED_1),
            mir_return_semantic: Some(MirSemantic::Raw),
        }
    }

    /// Binary returning raw i64 (not GC-tracked).
    pub const fn binary_to_i64(symbol: &'static str) -> Self {
        Self {
            symbol,
            params: &[PI64, PI64],
            returns: Some(RI64),
            gc_roots_result: false,
            mir_param_semantics: Some(TAGGED_2),
            mir_return_semantic: Some(MirSemantic::Raw),
        }
    }

    /// Unary returning i8 (bool result, not GC-tracked).
    pub const fn unary_to_i8(symbol: &'static str) -> Self {
        Self {
            symbol,
            params: &[PI64],
            returns: Some(RI8),
            gc_roots_result: false,
            mir_param_semantics: Some(TAGGED_1),
            mir_return_semantic: Some(MirSemantic::Raw),
        }
    }

    /// Binary returning i8 (bool result, not GC-tracked).
    pub const fn binary_to_i8(symbol: &'static str) -> Self {
        Self {
            symbol,
            params: &[PI64, PI64],
            returns: Some(RI8),
            gc_roots_result: false,
            mir_param_semantics: Some(TAGGED_2),
            mir_return_semantic: Some(MirSemantic::Raw),
        }
    }

    /// Slice ABI `f(obj, start, stop) -> obj`: a tagged receiver and RAW i64
    /// bounds (Phase 8E). Like `ptr_ternary` but bounds are Raw, not Tagged.
    pub const fn slice_ternary(symbol: &'static str) -> Self {
        Self {
            symbol,
            params: &[PI64, PI64, PI64],
            returns: Some(RI64),
            gc_roots_result: true,
            mir_param_semantics: Some(SLICE_TERNARY),
            mir_return_semantic: Some(MirSemantic::Tagged),
        }
    }

    /// Slice ABI `f(obj, start, stop, step) -> obj`: a tagged receiver and RAW
    /// i64 bounds + step (Phase 8E).
    pub const fn slice_quaternary(symbol: &'static str) -> Self {
        Self {
            symbol,
            params: &[PI64, PI64, PI64, PI64],
            returns: Some(RI64),
            gc_roots_result: true,
            mir_param_semantics: Some(SLICE_QUATERNARY),
            mir_return_semantic: Some(MirSemantic::Tagged),
        }
    }

    /// Stage B.2 helper: lookup the semantic for parameter `idx`,
    /// falling back to `MirSemantic::infer_param` when not explicitly
    /// annotated.
    pub fn param_semantic(&self, idx: usize) -> MirSemantic {
        if let Some(sems) = self.mir_param_semantics {
            if idx < sems.len() {
                return sems[idx];
            }
        }
        // Fallback: infer from Cranelift register class.
        if idx < self.params.len() {
            MirSemantic::infer_param(self.params[idx])
        } else {
            MirSemantic::Raw
        }
    }

    /// Returns `true` if any explicitly-annotated parameter has
    /// `MirSemantic::Tagged`.
    ///
    /// Used by WPA parameter ABI inference to detect calls into the
    /// tagged-Value dispatch family without fragile symbol-name prefix checks.
    /// Functions declared via `ptr_*` / `unary_to_i64` / `binary_to_i8`
    /// constructors all have at least one Tagged param. Functions declared via
    /// bare `new()` with no `mir_param_semantics` (e.g. `RT_OBJ_HAS_METHOD`)
    /// return `false` — correct, since those args are raw heap pointer + raw
    /// integer, not tagged Values.
    pub fn any_param_tagged(&self) -> bool {
        self.mir_param_semantics
            .is_some_and(|sems| sems.contains(&MirSemantic::Tagged))
    }

    /// Stage B.2 helper: resolved return semantic with fallback.
    ///
    /// Resolution order:
    /// 1. Explicit `mir_return_semantic` annotation (`new_typed`/`ptr_*`).
    /// 2. `gc_roots_result == true` ⇒ `MirSemantic::Tagged` — the def
    ///    returns a heap-managed pointer (boxed primitive, container,
    ///    instance, runtime object). Without this fallback, a def
    ///    declared via the bare `new(..., Some(RI64), true)` constructor
    ///    would inherit `MirSemantic::Raw` from `infer_return` and
    ///    downstream classification at
    ///    `optimizer::type_inference::materialize_function_types` would
    ///    treat the dest local as a raw-bits producer — the
    ///    producer-aware narrowing pass would then flip its `mir_ty`
    ///    Tagged→Raw, and consumers reading the slot would interpret
    ///    the heap pointer bits as a raw integer. The `gc_roots_result`
    ///    flag is the single source of truth for "heap-managed return".
    /// 3. Default `infer_return` from the Cranelift register class (Raw
    ///    for all non-void return types).
    pub fn return_semantic(&self) -> Option<MirSemantic> {
        if let Some(s) = self.mir_return_semantic {
            return Some(s);
        }
        if self.gc_roots_result {
            return Some(MirSemantic::Tagged);
        }
        self.returns.map(MirSemantic::infer_return)
    }
}

// =============================================================================
// Static runtime function definitions
// =============================================================================

// ===== Hash operations =====

/// rt_hash_str(str_obj: *mut Obj) -> i64
pub static RT_HASH_STR: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_hash_str");

// ===== Singletons =====

/// rt_not_implemented_singleton() -> *mut Obj
/// Returns the canonical NotImplemented sentinel pointer. Identity-compared
/// at operator-dunder dispatch sites to detect the "no dispatch handled
/// this operand" return and try the reflected dunder per CPython §3.3.8.
pub static RT_NOT_IMPLEMENTED_SINGLETON: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_not_implemented_singleton", &[], Some(RI64), false);

// ===== File I/O operations =====

/// rt_file_read(file: *mut Obj) -> *mut Obj
pub static RT_FILE_READ: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_file_read");
/// rt_file_read_n(file: *mut Obj, n: i64) -> *mut Obj
/// The count `n` is a RAW i64, not a tagged Value — so this cannot use
/// `ptr_binary` (which marks both params Tagged). The first param is the
/// tagged File pointer; the second is the raw read length.
pub static RT_FILE_READ_N: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_file_read_n",
    &[PI64, PI64],
    Some(RI64),
    true,
    &[MirSemantic::Tagged, MirSemantic::Raw],
    Some(MirSemantic::Tagged),
);
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

// ===== Object operations (Union type dispatch) =====

/// rt_value_or(v, d) -> Value — `v`, or `d` when `v` is the empty (null)
/// slot: an unassigned instance field shadowing a class attribute.
pub static RT_VALUE_OR: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_value_or");
/// rt_instance_class_id(obj) -> i64 — a user instance's class id, else -1.
pub static RT_INSTANCE_CLASS_ID: RuntimeFuncDef =
    RuntimeFuncDef::unary_to_i64("rt_instance_class_id");
/// rt_is_none(obj: *mut Obj) -> i8
pub static RT_IS_NONE: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_is_none");
/// rt_is(a: *mut Obj, b: *mut Obj) -> i8 — bit-identity (`a is b`).
pub static RT_IS: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_is");
/// rt_isinstance_builtin(obj: Value, kind: i64) -> i8 — `isinstance(obj, T)`
/// against a builtin type `T` for a gradual (`Dyn`/`Union`) value, by tag.
/// `kind` is a raw [`crate::isinstance_kind`] code; `obj` is Tagged.
pub static RT_ISINSTANCE_BUILTIN: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_isinstance_builtin",
    &[PI64, PI64],
    Some(RI8),
    false,
    &[MirSemantic::Tagged, MirSemantic::Raw],
    Some(MirSemantic::Raw),
);
/// rt_getattr_name_or_default(obj: Value, name_hash: i64, default: Value) -> Value.
/// The 3-arg `getattr` sibling of `rt_getattr_name`: returns `default` (the
/// third arg) on a miss/non-instance instead of raising `AttributeError`. The
/// `name_hash` slot is RAW (the FNV-1a hash, passed verbatim, never tagged);
/// `obj`/`default`/result are Tagged.
pub static RT_GETATTR_NAME_OR_DEFAULT: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_getattr_name_or_default",
    &[PI64, PI64, PI64],
    Some(RI64),
    true,
    &[MirSemantic::Tagged, MirSemantic::Raw, MirSemantic::Tagged],
    Some(MirSemantic::Tagged),
);
/// rt_obj_slice(obj: *mut Obj, start: i64, end: i64) -> *mut Obj — Any/HeapAny
/// runtime-dispatched slicing. Routes to the type-specific slicer based on
/// the object's `TypeTagKind`. Used by `lower_slice` when `obj_type` is
/// `Any` / `HeapAny`, replacing the silent `None`-constant fallback that
/// produced empty-shape lists for downstream consumers.
pub static RT_OBJ_SLICE: RuntimeFuncDef = RuntimeFuncDef::slice_ternary("rt_obj_slice");
/// rt_obj_slice_step(obj: *mut Obj, start: i64, end: i64, step: i64) -> *mut Obj
pub static RT_OBJ_SLICE_STEP: RuntimeFuncDef =
    RuntimeFuncDef::slice_quaternary("rt_obj_slice_step");
/// rt_obj_method(recv: Value, name_hash: i64, args_tuple: Value, kwargs: Value)
/// -> Value — the gradual-completeness method dispatcher for a `Dyn`/`Union`
/// receiver (the method analogue of `rt_obj_len`/`rt_any_getitem`). Decides by
/// the receiver's runtime tag exactly as CPython resolves `type(obj).method`:
/// a container receiver routes to the typed `rt_list_*`/`rt_dict_*`/`rt_set_*`
/// family; an `Instance` routes through its per-method uniform thunk
/// (`METHOD_UNIFORM_REGISTRY`); anything else raises `AttributeError`. The
/// positional args ride a `tuple[Tagged]` (arg 2), keywords a dict or the null
/// sentinel (arg 3); `name_hash` is the RAW FNV-1a method-name hash.
pub static RT_OBJ_METHOD: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_obj_method",
    &[PI64, PI64, PI64, PI64],
    Some(RI64),
    true,
    &[
        MirSemantic::Tagged,
        MirSemantic::Raw,
        MirSemantic::Tagged,
        MirSemantic::Tagged,
    ],
    Some(MirSemantic::Tagged),
);

// ===== Dict operations =====

/// rt_obj_getitem(obj: Value, key: Value) -> Value — `obj[key]` on a
/// statically-unknown base with a non-`int` key: tag-dispatched, the key stays
/// a tagged Value (dict family by key with each flavor's missing-key rule;
/// sequences take an int key).
pub static RT_OBJ_GETITEM: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_obj_getitem");
/// rt_dict_fromkeys(keys: *mut Obj, value: *mut Obj) -> *mut Obj
pub static RT_DICT_FROM_KEYS: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_dict_fromkeys");
/// rt_defaultdict_get(dict: *mut Obj, key: *mut Obj) -> *mut Obj
pub static RT_DEFAULT_DICT_GET: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_defaultdict_get");
/// rt_counter_get(counter: *mut Obj, key: i64) -> *mut Obj — Counter subscript:
/// the count for `key`, or a boxed `0` for a missing key (no KeyError).
pub static RT_COUNTER_GET: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_counter_get");

// ===== frozenset operators (`| & - ^`) =====
// Each returns a NEW frozenset (the result of the corresponding set algebra,
// retagged `FrozenSet`). Methods reuse the same `rt_frozenset_*` symbols via the
// `object_types` method registry.
/// rt_frozenset_union(a, b) -> frozenset
pub static RT_FROZENSET_UNION: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_frozenset_union");
/// rt_frozenset_intersection(a, b) -> frozenset
pub static RT_FROZENSET_INTERSECTION: RuntimeFuncDef =
    RuntimeFuncDef::ptr_binary("rt_frozenset_intersection");
/// rt_frozenset_difference(a, b) -> frozenset
pub static RT_FROZENSET_DIFFERENCE: RuntimeFuncDef =
    RuntimeFuncDef::ptr_binary("rt_frozenset_difference");
/// rt_frozenset_symmetric_difference(a, b) -> frozenset
pub static RT_FROZENSET_SYMMETRIC_DIFFERENCE: RuntimeFuncDef =
    RuntimeFuncDef::ptr_binary("rt_frozenset_symmetric_difference");

// ===== bytearray subscript / slice / concat =====
/// rt_bytearray_get(ba, index) -> tagged int. A tagged receiver + RAW i64 index
/// (like list/deque get); the byte is returned as a boxed int Value.
pub static RT_BYTEARRAY_GET: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_bytearray_get",
    &[PI64, PI64],
    Some(RI64),
    true,
    &[MirSemantic::Tagged, MirSemantic::Raw],
    Some(MirSemantic::Tagged),
);
/// rt_bytearray_set(ba, index, value) -> void. `ba[i] = v`; RAW i64 index,
/// tagged int value (0-255, else ValueError).
pub static RT_BYTEARRAY_SET: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_bytearray_set", &[PI64, PI64, PI64]);
/// rt_bytearray_slice(ba, start, stop) -> bytearray
pub static RT_BYTEARRAY_SLICE: RuntimeFuncDef = RuntimeFuncDef::slice_ternary("rt_bytearray_slice");
/// rt_bytearray_slice_step(ba, start, stop, step) -> bytearray
pub static RT_BYTEARRAY_SLICE_STEP: RuntimeFuncDef =
    RuntimeFuncDef::slice_quaternary("rt_bytearray_slice_step");
/// rt_bytearray_concat(a, b) -> bytearray (`ba + bytes-like`)
pub static RT_BYTEARRAY_CONCAT: RuntimeFuncDef =
    RuntimeFuncDef::ptr_binary("rt_bytearray_concat");

// ===== List operations =====

/// rt_list_slice(list: *mut Obj, start: i64, stop: i64) -> *mut Obj
pub static RT_LIST_SLICE: RuntimeFuncDef = RuntimeFuncDef::slice_ternary("rt_list_slice");
/// rt_list_slice_step(list: *mut Obj, start: i64, stop: i64, step: i64) -> *mut Obj
pub static RT_LIST_SLICE_STEP: RuntimeFuncDef =
    RuntimeFuncDef::slice_quaternary("rt_list_slice_step");
/// rt_unpack_seq(v: Value) -> Value — the source of a positional unpack on a
/// `Dyn` value: `v` itself when it is a list/tuple/str/bytes (int-indexable),
/// else a fresh list materialized through the iterator protocol. Contract
/// addition backing `ContainerOp::UnpackSource`.
pub static RT_UNPACK_SEQ: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_unpack_seq");
/// rt_deque_get(deque: *mut Obj, index: i64) -> *mut Obj
/// O(1) ring-buffer access; negative indices and bounds checks handled inside.
/// The deque is a tagged Value; the index is a RAW i64 (like list get/set).
pub static RT_DEQUE_GET: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_deque_get",
    &[PI64, PI64],
    Some(RI64),
    true,
    &[MirSemantic::Tagged, MirSemantic::Raw],
    Some(MirSemantic::Tagged),
);
/// rt_deque_set(deque: *mut Obj, index: i64, value: i64) -> void
/// Element assignment `dq[i] = v`; negative indices and bounds checks inside.
pub static RT_DEQUE_SET: RuntimeFuncDef = RuntimeFuncDef::void("rt_deque_set", &[PI64, PI64, PI64]);
/// rt_list_delete(list: *mut Obj, index: i64) -> void
/// `del li[i]`; negative indices and bounds checks inside; raises IndexError on
/// OOB. The index is a RAW i64 (like list get/set), the list is a tagged Value.
pub static RT_LIST_DELETE: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_list_delete",
    &[PI64, PI64],
    None,
    false,
    &[MirSemantic::Tagged, MirSemantic::Raw],
    None,
);
/// rt_list_setslice(list, start, stop, step, values) -> void
/// `list[start:stop:step] = values`; the list + values are tagged Values, the
/// bounds RAW i64 (`i64::MIN`/`i64::MAX`/`1` sentinels for an absent
/// start/stop/step). `step == 1` grows/shrinks the list; an extended slice
/// (`step != 1`) requires `len(values) == len(slice)` (ValueError otherwise).
/// A non-list receiver raises TypeError.
pub static RT_LIST_SETSLICE: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_list_setslice",
    &[PI64, PI64, PI64, PI64, PI64],
    None,
    false,
    &[
        MirSemantic::Tagged,
        MirSemantic::Raw,
        MirSemantic::Raw,
        MirSemantic::Raw,
        MirSemantic::Tagged,
    ],
    None,
);
/// rt_list_delslice(list, start, stop, step) -> void
/// `del list[start:stop:step]`; the list is a tagged Value, the bounds RAW i64.
/// A non-list receiver raises TypeError.
pub static RT_LIST_DELSLICE: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_list_delslice",
    &[PI64, PI64, PI64, PI64],
    None,
    false,
    &[
        MirSemantic::Tagged,
        MirSemantic::Raw,
        MirSemantic::Raw,
        MirSemantic::Raw,
    ],
    None,
);
/// rt_dict_delete(dict: *mut Obj, key: *mut Obj) -> void
/// `del d[k]`; raises KeyError (with the key's repr) when the key is absent.
/// Both the dict and the key are tagged Values.
pub static RT_DICT_DELETE: RuntimeFuncDef = RuntimeFuncDef::void("rt_dict_delete", &[PI64, PI64]);
/// rt_dict_move_to_end(dict: *mut Obj, key: *mut Obj, last: i64) -> void
/// `OrderedDict.move_to_end(key, last)`; the dict and key are tagged Values, the
/// `last` flag a RAW i64 (1 → move to end, 0 → move to front). Raises KeyError
/// when the key is absent.
pub static RT_DICT_MOVE_TO_END: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_dict_move_to_end",
    &[PI64, PI64, PI64],
    None,
    false,
    &[MirSemantic::Tagged, MirSemantic::Tagged, MirSemantic::Raw],
    None,
);
/// rt_dict_popitem_ordered(dict: *mut Obj, last: i64) -> *mut Obj
/// `OrderedDict.popitem(last)` — pop and return a `(key, value)` 2-tuple. The
/// dict is a tagged Value, `last` a RAW i64 (1 → LIFO/end, 0 → FIFO/front).
/// LIFO-identical to plain `dict.popitem()` when `last == 1`. Raises KeyError
/// on an empty dict.
pub static RT_DICT_POPITEM_ORDERED: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_dict_popitem_ordered",
    &[PI64, PI64],
    Some(RI64),
    true,
    &[MirSemantic::Tagged, MirSemantic::Raw],
    Some(MirSemantic::Tagged),
);
/// rt_any_delitem(container: *mut Obj, index: i64) -> void
/// Runtime-dispatched `del container[index]` for a statically-unknown base
/// (deque, gradual `Dyn`). Mirrors `rt_any_getitem`: the index is a RAW i64
/// (re-boxed internally for the Dict arm); List→list_delete, Dict→dict_delete,
/// Deque→deque_delete.
pub static RT_ANY_DELITEM: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_any_delitem",
    &[PI64, PI64],
    None,
    false,
    &[MirSemantic::Tagged, MirSemantic::Raw],
    None,
);
/// rt_check_bound(value: Value, kind: i64, name: *mut Obj) -> Value
/// The `del`-slot read guard: returns `value` unchanged unless it is the
/// `Value::UNBOUND` sentinel, in which case it raises (by `kind`):
/// 0 → UnboundLocalError, 1 → NameError, 2 → AttributeError. `name` is the
/// slot/attribute name (a tagged `StrObj`) used to format the message.
pub static RT_CHECK_BOUND: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_check_bound",
    &[PI64, PI64, PI64],
    Some(RI64),
    true,
    &[MirSemantic::Tagged, MirSemantic::Raw, MirSemantic::Tagged],
    Some(MirSemantic::Tagged),
);

// ===== Tuple operations =====

/// rt_tuple_slice(tuple: *mut Obj, start: i64, stop: i64) -> *mut Obj
pub static RT_TUPLE_SLICE: RuntimeFuncDef = RuntimeFuncDef::slice_ternary("rt_tuple_slice");
/// rt_tuple_slice_step(tuple: *mut Obj, start: i64, stop: i64, step: i64) -> *mut Obj
pub static RT_TUPLE_SLICE_STEP: RuntimeFuncDef =
    RuntimeFuncDef::slice_quaternary("rt_tuple_slice_step");
// ===== Bytes operations =====

/// rt_bytes_slice(bytes: *mut Obj, start: i64, stop: i64) -> *mut Obj
pub static RT_BYTES_SLICE: RuntimeFuncDef = RuntimeFuncDef::slice_ternary("rt_bytes_slice");
/// rt_bytes_slice_step(bytes: *mut Obj, start: i64, stop: i64, step: i64) -> *mut Obj
pub static RT_BYTES_SLICE_STEP: RuntimeFuncDef =
    RuntimeFuncDef::slice_quaternary("rt_bytes_slice_step");
/// rt_bytes_decode(bytes: *mut Obj, encoding: *mut Obj, errors: *mut Obj) -> *mut Obj
pub static RT_BYTES_DECODE: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_bytes_decode");
/// rt_bytes_startswith(bytes: *mut Obj, prefix: *mut Obj) -> i8
pub static RT_BYTES_STARTS_WITH: RuntimeFuncDef =
    RuntimeFuncDef::binary_to_i8("rt_bytes_startswith");
/// rt_bytes_endswith(bytes: *mut Obj, suffix: *mut Obj) -> i8
pub static RT_BYTES_ENDS_WITH: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_bytes_endswith");
/// rt_bytes_find(bytes, sub, start: i64, end: i64) -> i64. `start`/`end` ride
/// RAW i64 slots (§9 — absent → 0 / i64::MAX, clamped to len).
pub static RT_BYTES_FIND: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_bytes_find",
    &[PI64, PI64, PI64, PI64],
    Some(RI64),
    false,
    BYTES_SEARCH_QUATERNARY,
    Some(MirSemantic::Raw),
);
/// rt_bytes_rfind(bytes, sub, start: i64, end: i64) -> i64.
pub static RT_BYTES_RFIND: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_bytes_rfind",
    &[PI64, PI64, PI64, PI64],
    Some(RI64),
    false,
    BYTES_SEARCH_QUATERNARY,
    Some(MirSemantic::Raw),
);
/// rt_bytes_search(bytes: *mut Obj, sub: *mut Obj, op_tag: u8) -> i64 (index variant)
pub static RT_BYTES_INDEX: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_bytes_search", &[PI64, PI64, PI8], Some(RI64), false);
/// rt_bytes_search(bytes: *mut Obj, sub: *mut Obj, op_tag: u8) -> i64 (rindex variant)
pub static RT_BYTES_RINDEX: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_bytes_search", &[PI64, PI64, PI8], Some(RI64), false);
/// rt_bytes_count(bytes: *mut Obj, sub: *mut Obj) -> i64
pub static RT_BYTES_COUNT: RuntimeFuncDef = RuntimeFuncDef::binary_to_i64("rt_bytes_count");
/// rt_bytes_replace(bytes, old, new, count: i64) -> *mut Obj. `count` rides a
/// RAW i64 slot (`-1` = unlimited, §9).
pub static RT_BYTES_REPLACE: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_bytes_replace",
    &[PI64, PI64, PI64, PI64],
    Some(RI64),
    true,
    REPLACE_QUATERNARY,
    Some(MirSemantic::Tagged),
);
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

// ===== Math operations =====

/// rt_int_bit_length(n: Value) -> i64 — `int.bit_length()`. The arg is a tagged
/// int/bool `Value` (bignum-aware); the count is a raw i64.
pub static RT_INT_BIT_LENGTH: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_int_bit_length");
/// rt_int_bit_count(n: Value) -> i64 — `int.bit_count()` (3.10+). Tagged
/// int/bool arg (bignum-aware); raw i64 count.
pub static RT_INT_BIT_COUNT: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_int_bit_count");
/// rt_int_index(n: Value) -> Value — `int.conjugate()` / `int.__index__()`: the
/// receiver's integer value (bool → int, bignum preserved). Tagged in, tagged out.
pub static RT_INT_INDEX: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_int_index");

// ===== Conversion operations =====

/// rt_str_to_int_with_base(s: *mut Obj, base: i64) -> Value — `int(s, base)`. The
/// string is a tagged `Value`; the base is a RAW i64 (not a tagged int), so this
/// needs the mixed `[Tagged, Raw]` semantics, NOT `binary_to_i64`'s `[Tagged,
/// Tagged]` (which would feed the runtime the base's tagged bits). The RESULT is
/// a tagged int `Value` (fixnum or heap `BigInt`), so it is GC-rooted and read
/// back as `Tagged` — large literals must promote to bignum, never truncate (A6).
const STR_BASE_BINARY: &[MirSemantic] = &[MirSemantic::Tagged, MirSemantic::Raw];
pub static RT_STR_TO_INT_WITH_BASE: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_str_to_int_with_base",
    &[PI64, PI64],
    Some(RI64),
    true,
    STR_BASE_BINARY,
    Some(MirSemantic::Tagged),
);
/// rt_type_name_extract(type_str: *mut Obj) -> *mut Obj
pub static RT_TYPE_NAME_EXTRACT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_type_name_extract");
/// rt_generator_throw(gen: Value, exc: Value) -> Value — `generator.throw(exc)`:
/// raise `exc` at the suspended yield; the next yielded value.
/// rt_exc_set_instance_message(instance: Value, msg: Value) — `BaseException.__init__`
/// on a user exception instance (`super().__init__(msg)`); `msg` null = no argument.
pub static RT_GENERATOR_THROW: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_generator_throw");
/// rt_exc_set_instance_message(instance, msg) — see above.
pub static RT_EXC_SET_INSTANCE_MESSAGE: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_exc_set_instance_message", &[PI64, PI64]);
/// rt_format(value: Value, spec: Value) -> Value  — format spec dispatch (PEP 3101)
pub static RT_FORMAT: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_format");

// ===== String operations =====

/// rt_str_slice(s: *mut Obj, start: i64, stop: i64) -> *mut Obj
pub static RT_STR_SLICE: RuntimeFuncDef = RuntimeFuncDef::slice_ternary("rt_str_slice");
/// rt_str_slice_step(s: *mut Obj, start: i64, stop: i64, step: i64) -> *mut Obj
pub static RT_STR_SLICE_STEP: RuntimeFuncDef =
    RuntimeFuncDef::slice_quaternary("rt_str_slice_step");
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
// `rt_str_search(s, sub, start, end, op_tag) -> i64`: a tagged str receiver and
// substring, RAW i64 `start`/`end` codepoint bounds (§9 — absent → 0 / i64::MAX,
// clamped to the length), and a RAW i8 `op_tag`. The `new` (inferred-Raw)
// semantics accept the Tagged recv/sub (the verifier's I64+Raw slot is lenient)
// and the Raw bounds.
/// rt_str_search(s, sub, start: i64, end: i64, op_tag: i8) -> i64 (find variant)
pub static RT_STR_FIND: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_str_search",
    &[PI64, PI64, PI64, PI64, PI8],
    Some(RI64),
    false, // a raw i64 position, not a heap pointer
);
/// rt_str_search(s, sub, start: i64, end: i64, op_tag: i8) -> i64 (rfind variant)
pub static RT_STR_RFIND: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_str_search",
    &[PI64, PI64, PI64, PI64, PI8],
    Some(RI64),
    false, // a raw i64 position, not a heap pointer
);
/// rt_str_search(s, sub, start: i64, end: i64, op_tag: i8) -> i64 (index variant)
pub static RT_STR_INDEX: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_str_search",
    &[PI64, PI64, PI64, PI64, PI8],
    Some(RI64),
    false, // a raw i64 position, not a heap pointer
);
/// rt_str_search(s, sub, start: i64, end: i64, op_tag: i8) -> i64 (rindex variant)
pub static RT_STR_RINDEX: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_str_search",
    &[PI64, PI64, PI64, PI64, PI8],
    Some(RI64),
    false, // a raw i64 position, not a heap pointer
);
/// rt_str_rsplit(s: *mut Obj, sep: *mut Obj, maxsplit: i64) -> *mut Obj.
/// `maxsplit` is read as a RAW machine integer (`-1` = unlimited), so the
/// generic `ptr_ternary` all-Tagged default is wrong — see `STR_SPLIT_TERNARY`.
pub static RT_STR_RSPLIT: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_str_rsplit",
    &[PI64, PI64, PI64],
    Some(RI64),
    true,
    STR_SPLIT_TERNARY,
    Some(MirSemantic::Tagged),
);
/// rt_str_isascii(s: *mut Obj) -> i8
pub static RT_STR_ISASCII: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_str_isascii");
/// rt_str_encode(s: *mut Obj, encoding: *mut Obj, errors: *mut Obj) -> *mut Obj
pub static RT_STR_ENCODE: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_str_encode");
/// rt_str_replace(s, old, new, count: i64) -> *mut Obj. `count` rides a RAW i64
/// slot (`-1` = unlimited, §9), so the generic `ptr_ternary` all-Tagged default
/// is wrong — see `REPLACE_QUATERNARY`.
pub static RT_STR_REPLACE: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_str_replace",
    &[PI64, PI64, PI64, PI64],
    Some(RI64),
    true,
    REPLACE_QUATERNARY,
    Some(MirSemantic::Tagged),
);
/// rt_str_count(s: *mut Obj, sub: *mut Obj) -> i64
pub static RT_STR_COUNT: RuntimeFuncDef = RuntimeFuncDef::binary_to_i64("rt_str_count");
/// rt_str_split(s: *mut Obj, sep: *mut Obj, maxsplit: i64) -> *mut Obj.
/// `maxsplit` is read as a RAW machine integer (`-1` = unlimited), so the
/// generic `ptr_ternary` all-Tagged default is wrong — see `STR_SPLIT_TERNARY`.
pub static RT_STR_SPLIT: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_str_split",
    &[PI64, PI64, PI64],
    Some(RI64),
    true,
    STR_SPLIT_TERNARY,
    Some(MirSemantic::Tagged),
);
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
// Alignment ABIs (`rt_str_center`/`ljust`/`rjust`/`zfill`): a tagged str
// receiver, a RAW i64 width, and (except zfill) a tagged fillchar. The width
// is read as a machine integer by the runtime, so the generic
// `ptr_ternary`/`ptr_binary` all-Tagged default is wrong here (Phase 8H).
const ALIGN_TERNARY: &[MirSemantic] = &[MirSemantic::Tagged, MirSemantic::Raw, MirSemantic::Tagged];
const ALIGN_BINARY: &[MirSemantic] = &[MirSemantic::Tagged, MirSemantic::Raw];

// Split ABIs (`rt_str_split`/`rt_str_rsplit`): a tagged str receiver, a tagged
// separator (null = whitespace split), and a RAW i64 `maxsplit` (`-1` =
// unlimited). The count is a machine integer — passing it Tagged would misread
// the tag bits as the count (B16), so it needs an explicit Raw slot (§9).
const STR_SPLIT_TERNARY: &[MirSemantic] =
    &[MirSemantic::Tagged, MirSemantic::Tagged, MirSemantic::Raw];
// `rt_str_expandtabs`: a tagged str receiver and a RAW i64 `tabsize`.
const STR_TABS_BINARY: &[MirSemantic] = &[MirSemantic::Tagged, MirSemantic::Raw];
// Replace ABIs (`rt_str_replace`/`rt_bytes_replace`): a tagged receiver, tagged
// `old`/`new`, and a RAW i64 `count` (`-1` = unlimited, §9).
const REPLACE_QUATERNARY: &[MirSemantic] = &[
    MirSemantic::Tagged,
    MirSemantic::Tagged,
    MirSemantic::Tagged,
    MirSemantic::Raw,
];
// Bytes search ABIs (`rt_bytes_find`/`rt_bytes_rfind`): a tagged bytes receiver
// and substring, plus RAW i64 `start`/`end` bounds (§9).
const BYTES_SEARCH_QUATERNARY: &[MirSemantic] = &[
    MirSemantic::Tagged,
    MirSemantic::Tagged,
    MirSemantic::Raw,
    MirSemantic::Raw,
];

/// rt_str_center(s: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj
pub static RT_STR_CENTER: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_str_center",
    &[PI64, PI64, PI64],
    Some(RI64),
    true,
    ALIGN_TERNARY,
    Some(MirSemantic::Tagged),
);
/// rt_str_ljust(s: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj
pub static RT_STR_LJUST: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_str_ljust",
    &[PI64, PI64, PI64],
    Some(RI64),
    true,
    ALIGN_TERNARY,
    Some(MirSemantic::Tagged),
);
/// rt_str_rjust(s: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj
pub static RT_STR_RJUST: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_str_rjust",
    &[PI64, PI64, PI64],
    Some(RI64),
    true,
    ALIGN_TERNARY,
    Some(MirSemantic::Tagged),
);
/// rt_str_zfill(s: *mut Obj, width: i64) -> *mut Obj
pub static RT_STR_ZFILL: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_str_zfill",
    &[PI64, PI64],
    Some(RI64),
    true,
    ALIGN_BINARY,
    Some(MirSemantic::Tagged),
);
/// rt_str_isdecimal(s: *mut Obj) -> i8
pub static RT_STR_ISDECIMAL: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_str_isdecimal");
/// rt_str_isdigit(s: *mut Obj) -> i8
pub static RT_STR_ISDIGIT: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_str_isdigit");
/// rt_str_isnumeric(s: *mut Obj) -> i8
pub static RT_STR_ISNUMERIC: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_str_isnumeric");
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
/// rt_str_expandtabs(s: *mut Obj, tabsize: i64) -> *mut Obj.
/// `tabsize` is read as a RAW machine integer (default 8), so the generic
/// `ptr_binary` all-Tagged default is wrong — see `STR_TABS_BINARY`.
pub static RT_STR_EXPANDTABS: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_str_expandtabs",
    &[PI64, PI64],
    Some(RI64),
    true,
    STR_TABS_BINARY,
    Some(MirSemantic::Tagged),
);

// ===== Print operations =====

/// rt_print_set_stderr() -> void
pub static RT_PRINT_SET_STDERR: RuntimeFuncDef = RuntimeFuncDef::void("rt_print_set_stderr", &[]);
/// rt_print_set_stdout() -> void
pub static RT_PRINT_SET_STDOUT: RuntimeFuncDef = RuntimeFuncDef::void("rt_print_set_stdout", &[]);
/// rt_print_flush() -> void — flush the current print target (`print(..., flush=True)`).
pub static RT_PRINT_FLUSH: RuntimeFuncDef = RuntimeFuncDef::void("rt_print_flush", &[]);

// ===== Instance operations =====
/// rt_object_new(class_id: i64) -> *mut Obj (a tagged heap instance, GC-rooted
/// by the caller). `class_id` is the untagged `cls`-as-int value (§3).
pub static RT_OBJECT_NEW: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_object_new",
    &[PI64],
    Some(RI64),
    true,
    &[MirSemantic::Raw],
    Some(MirSemantic::Tagged),
);
/// rt_obj_has_method(obj: *mut u8, name_hash: i64) -> i8
/// Returns 1 if the object has a method with the given name hash, 0 otherwise.
/// Used for structural Protocol isinstance checks.
pub static RT_OBJ_HAS_METHOD: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_obj_has_method", &[PI64, PI64], Some(RI8), false);

// ===== Struct_time field access =====

/// rt_struct_time_get_field(t: *mut Obj, field_index: u8) -> i64
/// Generic struct_time field accessor (0=year, 1=mon, 2=mday, 3=hour, 4=min, 5=sec, 6=wday, 7=yday, 8=isdst)
pub static RT_STRUCT_TIME_GET_FIELD: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_struct_time_get_field", &[PI64, PI8], Some(RI64), false);
