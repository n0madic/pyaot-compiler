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
//
// §F.7d.3: the typed Int/Bool primitive boxing externs are gone.
// Lowering and codegen tag/untag Int and Bool inline via the
// `ValueFromInt` / `ValueFromBool` / `UnwrapValueInt` / `UnwrapValueBool`
// MIR instructions (see `crates/codegen-cranelift/src/instructions/tag.rs`).
// Float remains heap-boxed and uses the extern shims below.

/// rt_box_float(value: f64) -> *mut Obj
pub static RT_BOX_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_box_float", &[PF64], Some(RI64), true);
/// rt_box_none() -> *mut Obj
pub static RT_BOX_NONE: RuntimeFuncDef = RuntimeFuncDef::new("rt_box_none", &[], Some(RI64), true);
/// rt_not_implemented_singleton() -> *mut Obj
/// Returns the canonical NotImplemented sentinel pointer. Identity-compared
/// at operator-dunder dispatch sites to detect the "no dispatch handled
/// this operand" return and try the reflected dunder per CPython §3.3.8.
pub static RT_NOT_IMPLEMENTED_SINGLETON: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_not_implemented_singleton", &[], Some(RI64), false);

// ===== Unboxing operations =====

/// rt_unbox_float(obj: *mut Obj) -> f64
pub static RT_UNBOX_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_unbox_float", &[PI64], Some(RF64), false);

/// rt_unbox_bool(v: Value) -> i8 — the third checked-unbox shape
/// (`Tagged → Raw(I8)`), strict (TypeError on a non-bool tag). The codegen path
/// registers `rt_unbox_bool` by string, so this descriptor is for symmetry.
pub static RT_UNBOX_BOOL: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_unbox_bool", &[PI64], Some(RI8), false);

// ===== File I/O operations =====

/// rt_file_open(filename: *mut Obj, mode: *mut Obj, encoding: *mut Obj) -> *mut Obj
pub static RT_FILE_OPEN: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_file_open");
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
/// rt_file_is_closed(file: *mut Obj) -> i8
pub static RT_FILE_IS_CLOSED: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_file_is_closed");
/// rt_file_name(file: *mut Obj) -> *mut Obj
pub static RT_FILE_NAME: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_file_name");

// ===== Object operations (Union type dispatch) =====

/// rt_is_truthy(obj: *mut Obj) -> i8
pub static RT_IS_TRUTHY: RuntimeFuncDef = RuntimeFuncDef::unary_to_i8("rt_is_truthy");
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
/// rt_obj_neg(a: *mut Obj) -> *mut Obj — unary negation with class-dunder dispatch
pub static RT_OBJ_NEG: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_obj_neg");
/// rt_obj_pos(a: *mut Obj) -> *mut Obj — unary plus with class-dunder dispatch
pub static RT_OBJ_POS: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_obj_pos");
/// rt_obj_invert(a: *mut Obj) -> *mut Obj — bitwise invert with class-dunder dispatch
pub static RT_OBJ_INVERT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_obj_invert");
/// rt_any_getitem(obj: *mut Obj, index: i64) -> *mut Obj
pub static RT_ANY_GETITEM: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_any_getitem");
/// rt_obj_slice(obj: *mut Obj, start: i64, end: i64) -> *mut Obj — Any/HeapAny
/// runtime-dispatched slicing. Routes to the type-specific slicer based on
/// the object's `TypeTagKind`. Used by `lower_slice` when `obj_type` is
/// `Any` / `HeapAny`, replacing the silent `None`-constant fallback that
/// produced empty-shape lists for downstream consumers.
pub static RT_OBJ_SLICE: RuntimeFuncDef = RuntimeFuncDef::slice_ternary("rt_obj_slice");
/// rt_obj_slice_step(obj: *mut Obj, start: i64, end: i64, step: i64) -> *mut Obj
pub static RT_OBJ_SLICE_STEP: RuntimeFuncDef =
    RuntimeFuncDef::slice_quaternary("rt_obj_slice_step");
/// rt_obj_len(obj: *mut Obj) -> i64 — Any/HeapAny runtime-dispatched length.
/// Routes to the type-specific length helper based on `TypeTagKind`. Used
/// by `lower_len` when `select_len_func` returns `None` (Any/HeapAny),
/// replacing the silent `Const(0)` fallback that misreported the length
/// of legitimate non-empty containers when the compile-time element type
/// collapsed to `Any`.
pub static RT_OBJ_LEN: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_obj_len");
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
/// rt_dict_keys(dict: *mut Obj) -> *mut Obj
pub static RT_DICT_KEYS: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_dict_keys");
/// rt_dict_values(dict: *mut Obj) -> *mut Obj
pub static RT_DICT_VALUES: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_dict_values");
/// rt_dict_items(dict: *mut Obj) -> *mut Obj
pub static RT_DICT_ITEMS: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_dict_items");
/// rt_dict_update(dict: *mut Obj, other: *mut Obj) -> void
pub static RT_DICT_UPDATE: RuntimeFuncDef = RuntimeFuncDef::void("rt_dict_update", &[PI64, PI64]);
/// rt_dict_from_pairs(pairs: *mut Obj) -> *mut Obj
pub static RT_DICT_FROM_PAIRS: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_dict_from_pairs");
/// rt_dict_setdefault(dict: *mut Obj, key: i64, default: i64) -> *mut Obj
pub static RT_DICT_SET_DEFAULT: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_dict_setdefault");
/// rt_dict_fromkeys(keys: *mut Obj, value: *mut Obj) -> *mut Obj
pub static RT_DICT_FROM_KEYS: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_dict_fromkeys");
/// rt_dict_merge(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_DICT_MERGE: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_dict_merge");
/// rt_make_defaultdict(capacity: i64, factory_tag: i64) -> *mut Obj — both args
/// are RAW i64 (the dict capacity and the packed factory tag), the result a
/// tagged `DictObj` (`TypeTagKind::DefaultDict`).
pub static RT_MAKE_DEFAULT_DICT: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_make_defaultdict",
    &[PI64, PI64],
    Some(RI64),
    true,
    &[MirSemantic::Raw, MirSemantic::Raw],
    Some(MirSemantic::Tagged),
);
/// rt_defaultdict_get(dict: *mut Obj, key: *mut Obj) -> *mut Obj
pub static RT_DEFAULT_DICT_GET: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_defaultdict_get");
/// rt_make_counter_from_iter(iter: *mut Obj) -> *mut Obj
pub static RT_MAKE_COUNTER_FROM_ITER: RuntimeFuncDef =
    RuntimeFuncDef::ptr_unary("rt_make_counter_from_iter");
/// rt_make_counter_empty() -> *mut Obj
pub static RT_MAKE_COUNTER_EMPTY: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_make_counter_empty", &[], Some(RI64), true);
/// rt_counter_get(counter: *mut Obj, key: i64) -> *mut Obj — Counter subscript:
/// the count for `key`, or a boxed `0` for a missing key (no KeyError).
pub static RT_COUNTER_GET: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_counter_get");
/// rt_make_deque(iterable: *mut Obj) -> *mut Obj
pub static RT_MAKE_DEQUE: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_make_deque");
/// rt_deque_from_iter(iter: *mut Obj, maxlen: *mut Obj) -> *mut Obj
pub static RT_MAKE_DEQUE_FROM_ITER: RuntimeFuncDef =
    RuntimeFuncDef::ptr_binary("rt_deque_from_iter");

// ===== List operations =====

/// rt_make_list(capacity: i64) -> *mut Obj
pub static RT_MAKE_LIST: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_make_list", &[PI64], Some(RI64), true);
/// rt_list_push(list: *mut Obj, elem: i64) -> void
pub static RT_LIST_PUSH: RuntimeFuncDef = RuntimeFuncDef::void("rt_list_push", &[PI64, PI64]);
/// rt_list_set(list: *mut Obj, index: i64, value: i64) -> void
pub static RT_LIST_SET: RuntimeFuncDef = RuntimeFuncDef::void("rt_list_set", &[PI64, PI64, PI64]);
/// rt_list_get(list: *mut Obj, index: i64) -> *mut Obj
pub static RT_LIST_GET: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_list_get");
/// rt_list_get_typed(list: *mut Obj, index: i64, elem_kind: u8) -> i64
/// elem_kind: 0=Int, 1=Float (result is f64 bits), 2=Bool (result is i8 as i64)
/// The codegen descriptor system handles I64→F64 bitcast for float dest locals.
pub static RT_LIST_GET_TYPED: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_list_get_typed", &[PI64, PI64, PI8], Some(RI64), false);
/// rt_list_len(list: *mut Obj) -> i64
pub static RT_LIST_LEN: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_list_len");
/// rt_list_slice(list: *mut Obj, start: i64, stop: i64) -> *mut Obj
pub static RT_LIST_SLICE: RuntimeFuncDef = RuntimeFuncDef::slice_ternary("rt_list_slice");
/// rt_list_slice_step(list: *mut Obj, start: i64, stop: i64, step: i64) -> *mut Obj
pub static RT_LIST_SLICE_STEP: RuntimeFuncDef =
    RuntimeFuncDef::slice_quaternary("rt_list_slice_step");
/// rt_list_append(list: *mut Obj, elem: i64) -> void
pub static RT_LIST_APPEND: RuntimeFuncDef = RuntimeFuncDef::void("rt_list_append", &[PI64, PI64]);
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
/// rt_list_sort_by_keys(list: *mut Obj, keys: *mut Obj, reverse: i8) -> void
/// Stable tandem sort of `list` by the parallel `keys` list (Phase 10 — the
/// compiled `key=` callback fills `keys` before the call; no runtime callbacks).
pub static RT_LIST_SORT_BY_KEYS: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_list_sort_by_keys", &[PI64, PI64, PI8]);
/// rt_list_from_tuple(tuple: *mut Obj) -> *mut Obj
pub static RT_LIST_FROM_TUPLE: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_list_from_tuple");
/// rt_list_from_str(str: *mut Obj) -> *mut Obj
pub static RT_LIST_FROM_STR: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_list_from_str");
/// rt_list_from_bytes(bytes: *mut Obj) -> *mut Obj — each byte becomes a Python int element
pub static RT_LIST_FROM_BYTES: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_list_from_bytes");
/// rt_list_from_range(start: i64, stop: i64, step: i64) -> *mut Obj
pub static RT_LIST_FROM_RANGE: RuntimeFuncDef = RuntimeFuncDef::ptr_ternary("rt_list_from_range");
/// rt_list_from_iter(iter: *mut Obj) -> *mut Obj
pub static RT_LIST_FROM_ITER: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_list_from_iter");
/// rt_list_from_set(set: *mut Obj) -> *mut Obj
pub static RT_LIST_FROM_SET: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_list_from_set");
/// rt_list_from_dict(dict: *mut Obj) -> *mut Obj
pub static RT_LIST_FROM_DICT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_list_from_dict");
/// rt_list_from_deque(deque: *mut Obj) -> *mut Obj
pub static RT_LIST_FROM_DEQUE: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_list_from_deque");
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
/// rt_deque_delete(deque: *mut Obj, index: i64) -> void
/// `del dq[i]`; negative indices and bounds checks inside.
pub static RT_DEQUE_DELETE: RuntimeFuncDef = RuntimeFuncDef::void("rt_deque_delete", &[PI64, PI64]);
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
/// rt_list_tail_to_tuple(list: *mut Obj, start: i64) -> *mut Obj
pub static RT_LIST_TAIL_TO_TUPLE: RuntimeFuncDef =
    RuntimeFuncDef::ptr_binary("rt_list_tail_to_tuple");
/// rt_list_tail_to_tuple_float(list: *mut Obj, start: i64) -> *mut Obj
pub static RT_LIST_TAIL_TO_TUPLE_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::ptr_binary("rt_list_tail_to_tuple_float");
/// rt_list_tail_to_tuple_bool(list: *mut Obj, start: i64) -> *mut Obj
pub static RT_LIST_TAIL_TO_TUPLE_BOOL: RuntimeFuncDef =
    RuntimeFuncDef::ptr_binary("rt_list_tail_to_tuple_bool");
/// rt_list_concat(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_LIST_CONCAT: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_list_concat");
/// rt_list_repeat(list: *mut Obj, count: i64) -> *mut Obj
pub static RT_LIST_REPEAT: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_list_repeat");

// ===== Tuple operations =====

/// rt_make_tuple(size: i64) -> *mut Obj
pub static RT_MAKE_TUPLE: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_make_tuple");
/// rt_tuple_set(tuple: *mut Obj, index: i64, value: i64) -> void
pub static RT_TUPLE_SET: RuntimeFuncDef = RuntimeFuncDef::void("rt_tuple_set", &[PI64, PI64, PI64]);
/// rt_tuple_get(tuple: *mut Obj, index: i64) -> *mut Obj
pub static RT_TUPLE_GET: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_tuple_get");
/// rt_tuple_len(tuple: *mut Obj) -> i64
pub static RT_TUPLE_LEN: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_tuple_len");
/// rt_tuple_slice(tuple: *mut Obj, start: i64, stop: i64) -> *mut Obj
pub static RT_TUPLE_SLICE: RuntimeFuncDef = RuntimeFuncDef::slice_ternary("rt_tuple_slice");
/// rt_tuple_slice_step(tuple: *mut Obj, start: i64, stop: i64, step: i64) -> *mut Obj
pub static RT_TUPLE_SLICE_STEP: RuntimeFuncDef =
    RuntimeFuncDef::slice_quaternary("rt_tuple_slice_step");
/// rt_tuple_slice_to_list(tuple: *mut Obj, start: i64, stop: i64) -> *mut Obj
pub static RT_TUPLE_SLICE_TO_LIST: RuntimeFuncDef =
    RuntimeFuncDef::slice_ternary("rt_tuple_slice_to_list");
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
/// rt_call_with_captures_and_args(func_ptr: i64, captures_tuple: *mut Obj, args_tuple: *mut Obj) -> i64
///
/// Stage E: closure-trampoline entry point that respects each tuple's own
/// elem_tag when extracting arguments. See `rt_call_with_captures_and_args`
/// in `runtime/src/tuple/core.rs`.
pub static RT_CALL_WITH_CAPTURES_AND_ARGS: RuntimeFuncDef = RuntimeFuncDef {
    symbol: "rt_call_with_captures_and_args",
    params: &[PI64, PI64, PI64],
    returns: Some(RI64),
    gc_roots_result: false,
    mir_param_semantics: None,
    mir_return_semantic: None,
};
// ===== Bytes operations =====

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
/// rt_bytes_concat(a: *mut Obj, b: *mut Obj) -> *mut Obj
pub static RT_BYTES_CONCAT: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_bytes_concat");
/// rt_bytes_repeat(bytes: *mut Obj, count: i64) -> *mut Obj
pub static RT_BYTES_REPEAT: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_bytes_repeat");
/// rt_bytes_from_hex(hex_str: *mut Obj) -> *mut Obj
pub static RT_BYTES_FROM_HEX: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_bytes_from_hex");
// ===== Math operations =====

/// rt_pow_float(base: f64, exp: f64) -> f64
pub static RT_POW_FLOAT: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_pow_float", &[PF64, PF64], Some(RF64), false);
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
/// rt_int_bit_length(n: Value) -> i64 — `int.bit_length()`. The arg is a tagged
/// int/bool `Value` (bignum-aware); the count is a raw i64.
pub static RT_INT_BIT_LENGTH: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_int_bit_length");
/// rt_int_bit_count(n: Value) -> i64 — `int.bit_count()` (3.10+). Tagged
/// int/bool arg (bignum-aware); raw i64 count.
pub static RT_INT_BIT_COUNT: RuntimeFuncDef = RuntimeFuncDef::unary_to_i64("rt_int_bit_count");
/// rt_int_index(n: Value) -> Value — `int.conjugate()` / `int.__index__()`: the
/// receiver's integer value (bool → int, bignum preserved). Tagged in, tagged out.
pub static RT_INT_INDEX: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_int_index");

// ===== Comparison operations =====
// Compare(kind, op) → static defs for all valid (kind, op) combinations.
// Signature: (a: I64, b: I64) -> I8 for eq-only variants.
// Signature: (a: I64, b: I64, op_tag: I8) -> I8 for ordering variants with op_tag.
// Signature: (a: I64, b: I64) -> I8 for Obj ordering (separate functions per op).

/// rt_list_eq(a: *mut Obj, b: *mut Obj) -> i8
/// Unified list equality — dispatches by elem_tag from the ListObj at runtime.
pub static RT_CMP_LIST_EQ: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_list_eq");
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
/// rt_dict_eq(a: *mut Obj, b: *mut Obj) -> i8
/// Structural dict equality — same keys with equal values, order-independent.
pub static RT_CMP_DICT_EQ: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_dict_eq");
/// rt_set_eq(a: *mut Obj, b: *mut Obj) -> i8
/// Structural set equality — same elements, order-independent.
pub static RT_CMP_SET_EQ: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_set_eq");
/// rt_obj_eq(a: *mut Obj, b: *mut Obj) -> i8
pub static RT_CMP_OBJ_EQ: RuntimeFuncDef = RuntimeFuncDef::binary_to_i8("rt_obj_eq");
/// rt_obj_cmp(a: *mut Obj, b: *mut Obj, op_tag: u8) -> i8
/// op_tag: 0=Lt, 1=Lte, 2=Gt, 3=Gte (matches ComparisonOp::to_tag())
pub static RT_CMP_OBJ_ORD: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_obj_cmp", &[PI64, PI64, PI8], Some(RI8), false);

// ===== Container min/max operations =====
// ContainerMinMax { container, op, elem } → static defs.
// Int/Float/Tagged: rt_{container}_minmax(container: I64, is_min: I8, elem_kind: I8) -> I64
//   elem_kind: 0=int, 1=float, 2=tagged (Any — runtime compares via rt_obj_cmp,
//   returns the winning element's tagged Value bits).
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
// After §F.7c+key_return_tag: rt_{container}_minmax_with_key(container, key_fn, captures, count, is_min, key_return_tag) -> *mut Obj
/// rt_list_minmax_with_key
pub static RT_LIST_MINMAX_WITH_KEY: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_list_minmax_with_key",
    &[PI64, PI64, PI64, PI64, PI8, PI8],
    Some(RI64),
    true,
);
/// rt_tuple_minmax_with_key
pub static RT_TUPLE_MINMAX_WITH_KEY: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_tuple_minmax_with_key",
    &[PI64, PI64, PI64, PI64, PI8, PI8],
    Some(RI64),
    true,
);
/// rt_set_minmax_with_key — Set still passes a `needs_unbox: i64` hint instead of elem_tag
pub static RT_SET_MINMAX_WITH_KEY: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_set_minmax_with_key",
    &[PI64, PI64, PI64, PI64, PI8, PI8],
    Some(RI64),
    true,
);
/// rt_dict_minmax_with_key
pub static RT_DICT_MINMAX_WITH_KEY: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_dict_minmax_with_key",
    &[PI64, PI64, PI64, PI64, PI8],
    Some(RI64),
    true,
);
/// rt_str_minmax_with_key
pub static RT_STR_MINMAX_WITH_KEY: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_str_minmax_with_key",
    &[PI64, PI64, PI64, PI64, PI8],
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
/// rt_str_to_int_with_base(s: *mut Obj, base: i64) -> i64 — `int(s, base)`. The
/// string is a tagged `Value`; the base is a RAW i64 (not a tagged int), so this
/// needs the mixed `[Tagged, Raw]` semantics, NOT `binary_to_i64`'s `[Tagged,
/// Tagged]` (which would feed the runtime the base's tagged bits).
const STR_BASE_BINARY: &[MirSemantic] = &[MirSemantic::Tagged, MirSemantic::Raw];
pub static RT_STR_TO_INT_WITH_BASE: RuntimeFuncDef = RuntimeFuncDef::new_typed(
    "rt_str_to_int_with_base",
    &[PI64, PI64],
    Some(RI64),
    false,
    STR_BASE_BINARY,
    Some(MirSemantic::Raw),
);
/// rt_builtin_int(obj: Value) -> Value (boxed Int). Dynamic `int(obj)` for
/// non-statically-resolved args (Union/Any/class instance): dispatches
/// `__int__`, raises TypeError for non-convertible types.
pub static RT_BUILTIN_INT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_builtin_int");
/// rt_builtin_float(obj: Value) -> Value (boxed Float). Dynamic `float(obj)`:
/// dispatches `__float__`, raises TypeError for non-convertible types.
pub static RT_BUILTIN_FLOAT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_builtin_float");
/// rt_str_contains(haystack: *mut Obj, needle: *mut Obj) -> i8
pub static RT_STR_CONTAINS: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_str_contains", &[PI64, PI64], Some(RI8), true);
/// rt_int_to_bin(n: i64) -> *mut Obj
pub static RT_INT_TO_BIN: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_int_to_bin");
/// rt_int_to_hex(n: i64) -> *mut Obj
pub static RT_INT_TO_HEX: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_int_to_hex");
/// rt_int_to_oct(n: i64) -> *mut Obj
pub static RT_INT_TO_OCT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_int_to_oct");
/// rt_type_name(obj: *mut Obj) -> *mut Obj
pub static RT_TYPE_NAME: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_type_name");
/// rt_type_name_extract(type_str: *mut Obj) -> *mut Obj
pub static RT_TYPE_NAME_EXTRACT: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_type_name_extract");
/// rt_exc_class_name(instance: *mut Obj) -> *mut Obj
pub static RT_EXC_CLASS_NAME: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_exc_class_name");
/// rt_format(value: Value, spec: Value) -> Value  — format spec dispatch (PEP 3101)
pub static RT_FORMAT: RuntimeFuncDef = RuntimeFuncDef::ptr_binary("rt_format");

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
/// rt_repr_collection(obj: *mut Obj) -> *mut Obj
/// Handles all heap types: str, bytes, list, tuple, dict, set, and generic objects.
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
/// rt_ascii_collection(obj: *mut Obj) -> *mut Obj
/// Handles all heap types: str, bytes, list, tuple, dict, set, and generic objects.
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
pub static RT_STR_SLICE: RuntimeFuncDef = RuntimeFuncDef::slice_ternary("rt_str_slice");
/// rt_str_slice_step(s: *mut Obj, start: i64, stop: i64, step: i64) -> *mut Obj
pub static RT_STR_SLICE_STEP: RuntimeFuncDef =
    RuntimeFuncDef::slice_quaternary("rt_str_slice_step");
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
    true,
);
/// rt_str_search(s, sub, start: i64, end: i64, op_tag: i8) -> i64 (rfind variant)
pub static RT_STR_RFIND: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_str_search",
    &[PI64, PI64, PI64, PI64, PI8],
    Some(RI64),
    true,
);
/// rt_str_search(s, sub, start: i64, end: i64, op_tag: i8) -> i64 (index variant)
pub static RT_STR_INDEX: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_str_search",
    &[PI64, PI64, PI64, PI64, PI8],
    Some(RI64),
    true,
);
/// rt_str_search(s, sub, start: i64, end: i64, op_tag: i8) -> i64 (rindex variant)
pub static RT_STR_RINDEX: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_str_search",
    &[PI64, PI64, PI64, PI64, PI8],
    Some(RI64),
    true,
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
/// rt_iter_deque(container: *mut Obj) -> *mut Obj — iterates a snapshot list
pub static RT_ITER_DEQUE: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_iter_deque");
/// rt_iter_generator(container: *mut Obj) -> *mut Obj
pub static RT_ITER_GENERATOR: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_iter_generator");
/// rt_iter_value(val: Value) -> *mut Obj — dynamic dispatch for Any/HeapAny iterables
pub static RT_ITER_VALUE: RuntimeFuncDef = RuntimeFuncDef::ptr_unary("rt_iter_value");

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
/// rt_iter_reversed_deque(container: *mut Obj) -> *mut Obj — reversed snapshot list
pub static RT_ITER_REVERSED_DEQUE: RuntimeFuncDef =
    RuntimeFuncDef::ptr_unary("rt_iter_reversed_deque");
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

// --- Sorted: range (special args) ---
/// rt_sorted_range(start: i64, stop: i64, step: i64, reverse: i64) -> *mut Obj
pub static RT_SORTED_RANGE: RuntimeFuncDef = RuntimeFuncDef::ptr_quaternary("rt_sorted_range");

// --- Sorted: generic dispatchers ---
/// rt_sorted(obj, reverse: i8, container_tag) -> *mut Obj
/// Contract change (Phase 10): `reverse` is `i8`; the callback-ABI
/// `rt_sorted_with_key` is deleted (`key=` compiles to a frontend desugar
/// feeding `rt_list_sort_by_keys`).
pub static RT_SORTED: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_sorted", &[PI64, PI8, PI8], Some(RI64), true);

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
/// rt_filter_new(func_ptr: i64, iter: *mut Obj, captures: *mut Obj, capture_count: i64) -> *mut Obj
pub static RT_FILTER_NEW: RuntimeFuncDef = RuntimeFuncDef::ptr_quaternary("rt_filter_new");
/// Phase 4+ Extension E2a: parallel tagged-delivery variants. Same
/// signature as the legacy `rt_*_new`, but the runtime stores
/// `IteratorKind::MapTagged` / `FilterTagged` and the dispatcher passes
/// both the input element and the callback's return value through
/// verbatim. Lowering routes to these when the callback callee is
/// `phase4_safe` (its prologue does its own UnboxValue and its return
/// path goes through BoxValue per Phase 4 Commit 4).
/// rt_map_new_tagged(func_ptr: i64, iter: *mut Obj, captures: *mut Obj, capture_count: i64) -> *mut Obj
pub static RT_MAP_NEW_TAGGED: RuntimeFuncDef = RuntimeFuncDef::ptr_quaternary("rt_map_new_tagged");
/// rt_filter_new_tagged(func_ptr: i64, iter: *mut Obj, captures: *mut Obj, capture_count: i64) -> *mut Obj
pub static RT_FILTER_NEW_TAGGED: RuntimeFuncDef =
    RuntimeFuncDef::ptr_quaternary("rt_filter_new_tagged");
/// rt_reduce(func_ptr, iter, initial, has_initial, captures, capture_count) -> *mut Obj
pub static RT_REDUCE: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_reduce",
    &[PI64, PI64, PI64, PI64, PI64, PI64],
    Some(RI64),
    true,
);
/// Phase 4+ Extension E2b: parallel tagged-delivery variant of `rt_reduce`.
/// Passes accumulator + element to callback verbatim; callback's prologue
/// performs its own UnboxValue, and its return goes through BoxValue.
pub static RT_REDUCE_TAGGED: RuntimeFuncDef = RuntimeFuncDef::new(
    "rt_reduce_tagged",
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
// §F.7b: RT_GENERATOR_SET_LOCAL_TYPE removed — per-slot tag side-array deleted.
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
/// rt_register_class_field_count(class_id: i8, field_count: i64) -> void
pub static RT_REGISTER_CLASS_FIELD_COUNT: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_register_class_field_count", &[PI8, PI64]);
/// rt_register_class_qualname(class_id: i64, name: *mut StrObj) -> void
/// Registers a class's qualified name (e.g. "__main__.Widget") for the
/// default object repr `<__main__.Widget object at 0x..>`.
pub static RT_REGISTER_CLASS_QUALNAME: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_register_class_qualname", &[PI64, PI64]);
/// rt_register_method_name(class_id: i64, name_hash: i64, slot: i64) -> void
pub static RT_REGISTER_METHOD_NAME: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_register_method_name", &[PI64, PI64, PI64]);
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
/// rt_obj_has_method(obj: *mut u8, name_hash: i64) -> i8
/// Returns 1 if the object has a method with the given name hash, 0 otherwise.
/// Used for structural Protocol isinstance checks.
pub static RT_OBJ_HAS_METHOD: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_obj_has_method", &[PI64, PI64], Some(RI8), false);
/// rt_register_dunder_func(class_id: i64, name_hash: i64, func_ptr: i64) -> void
/// Registers a binary-op dunder method's function pointer so runtime
/// arithmetic ops (`rt_obj_add`, `rt_obj_mul`, etc.) can dispatch through
/// user-defined dunders when an operand is a class instance.
pub static RT_REGISTER_DUNDER_FUNC: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_register_dunder_func", &[PI64, PI64, PI64]);
/// rt_register_method_uniform(class_id: i64, name_hash: i64, thunk_ptr: i64) -> void
/// Registers a class method's **uniform thunk** pointer
/// (`M.<uniform>(self, __args__, __kwargs__) -> Value`) so the runtime's
/// `rt_obj_method` dispatcher can invoke an arbitrary user method on a `Dyn`
/// receiver. An inherited method registers the base's thunk under the subclass
/// id too. The exact sibling of `rt_register_dunder_func` (a fixed-ABI fn ptr
/// keyed by `(class_id, name_hash)`).
pub static RT_REGISTER_METHOD_UNIFORM: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_register_method_uniform", &[PI64, PI64, PI64]);
/// rt_register_iternext(class_id: i64, thunk_ptr: i64) -> void
/// Registers a class's compiled `<iternext>` thunk pointer
/// (`Cls.<iternext>(self) -> Value`, returning the `__next__()` result or
/// `Value::UNBOUND` when `__next__` raised `StopIteration`) so the runtime's
/// `iter_next_instance` can drive a user-class iterator. Flat `class_id → ptr`
/// (no name hash): a class has at most one iterator protocol. An inherited
/// `__next__` registers the base's thunk under the subclass id too.
pub static RT_REGISTER_ITERNEXT: RuntimeFuncDef =
    RuntimeFuncDef::void("rt_register_iternext", &[PI64, PI64]);

// ===== Struct_time field access =====

/// rt_struct_time_get_field(t: *mut Obj, field_index: u8) -> i64
/// Generic struct_time field accessor (0=year, 1=mon, 2=mday, 3=hour, 4=min, 5=sec, 6=wday, 7=yday, 8=isdst)
pub static RT_STRUCT_TIME_GET_FIELD: RuntimeFuncDef =
    RuntimeFuncDef::new("rt_struct_time_get_field", &[PI64, PI8], Some(RI64), false);
