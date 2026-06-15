//! # codegen-cranelift — typed MIR → native code
//!
//! Lowers typed MIR to Cranelift IR and emits an object file. Each
//! [`MirFunction`] becomes a Cranelift function; the runtime provides **no** C
//! `main`, so this backend emits `main(argc, argv)` that calls `rt_init` → the
//! module-body function (`__main__`) → `rt_shutdown` → `return 0`.
//!
//! ## The ABI is one function
//!
//! [`clif_ty`] maps [`pyaot_types::Repr`] → a Cranelift `Type`. It *is* the ABI:
//! there is no second logical-type mapper and no per-function ABI flags.
//!
//! ## Locals are Cranelift `Variable`s
//!
//! Each MIR local is a Cranelift `Variable` (typed by `clif_ty`), so values flow
//! naturally across blocks (loop counters, branch joins) via Cranelift's SSA
//! construction. GC shadow frames (milestone 2c) store rooted locals into a
//! frame roots array on definition; the root set derives from
//! `Repr::is_gc_root()`, never a stored flag.

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::path::Path;

use cranelift_codegen::ir::{
    condcodes::IntCC, instructions::BlockArg, types, AbiParam, BlockCall, ExceptionTable,
    ExceptionTableData, ExceptionTableItem, InstBuilder, MemFlags, SigRef, Signature, StackSlot,
    StackSlotData, StackSlotKind, TrapCode, Type, Value,
};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::FinalizedMachExceptionHandler;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{default_libcall_names, DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use pyaot_core_defs::tag;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_mir::{
    classify_coercion, BinOp, CmpOp, Coercion, Const, ContainerCmpOp, ContainerOp, GenOp,
    LocalDecl, MirFunction, MirInst, MirProgram, MirTerminator, Operand, PrintKind, UnaryOp,
};
use pyaot_types::{RawKind, Repr};
use pyaot_utils::{InternedString, LocalId, StringInterner};

const FLOAT_VALUE_OFFSET: i32 = pyaot_core_defs::layout::FLOAT_OBJ_VALUE_OFFSET;

/// **The single `Repr` → Cranelift `Type` mapping — this is the ABI.**
fn clif_ty(repr: &Repr) -> Type {
    match repr {
        Repr::Raw(RawKind::I64) => types::I64,
        Repr::Raw(RawKind::F64) => types::F64,
        Repr::Raw(RawKind::I8) => types::I8,
        Repr::Raw(RawKind::I32) => types::I32,
        Repr::Tagged | Repr::Heap(_) | Repr::FuncPtr(_) | Repr::Closure(_) => types::I64,
        Repr::Never => types::I64,
    }
}

/// The imported runtime functions. Declaring an import that is never *used*
/// emits no relocation, so this can cover the whole Phase-2 surface up front.
struct RuntimeFns {
    init: FuncId,
    shutdown: FuncId,
    make_str: FuncId,
    bigint_from_str: FuncId,
    box_float: FuncId,
    unbox_float_checked: FuncId,
    unbox_int_checked: FuncId,
    unbox_bool_checked: FuncId,
    add_int: FuncId,
    sub_int: FuncId,
    mul_int: FuncId,
    obj_add: FuncId,
    obj_sub: FuncId,
    obj_mul: FuncId,
    obj_matmul: FuncId,
    obj_div: FuncId,
    obj_floordiv: FuncId,
    obj_mod: FuncId,
    obj_pow: FuncId,
    obj_neg: FuncId,
    obj_pos: FuncId,
    obj_invert: FuncId,
    obj_eq: FuncId,
    obj_cmp: FuncId,
    is_truthy: FuncId,
    obj_bitand: FuncId,
    obj_bitor: FuncId,
    obj_ior: FuncId,
    obj_bitxor: FuncId,
    obj_lshift: FuncId,
    obj_rshift: FuncId,
    builtin_abs: FuncId,
    builtin_hash: FuncId,
    builtin_int: FuncId,
    builtin_float: FuncId,
    builtin_str: FuncId,
    builtin_repr: FuncId,
    builtin_bool: FuncId,
    builtin_len: FuncId,
    builtin_ord: FuncId,
    builtin_chr: FuncId,
    builtin_type: FuncId,
    builtin_ascii: FuncId,
    assert_fail: FuncId,
    print_int: FuncId,
    print_float: FuncId,
    print_bool: FuncId,
    print_none: FuncId,
    print_str_obj: FuncId,
    print_obj: FuncId,
    print_sep: FuncId,
    print_newline: FuncId,
    gc_push: FuncId,
    gc_pop: FuncId,
    // ── containers (Phase 4) ──
    make_list: FuncId,
    make_dict: FuncId,
    make_set: FuncId,
    make_tuple: FuncId,
    /// Allocates a `Closure`-tagged env tuple (`MakeClosure` slot 0 + captures).
    make_closure: FuncId,
    make_bytes: FuncId,
    list_push: FuncId,
    list_set: FuncId,
    dict_set: FuncId,
    set_add: FuncId,
    tuple_set: FuncId,
    /// Guards an indirect call's callee (must be a closure tuple) — `TypeError`
    /// on a non-callable value instead of a bad slot-0 read.
    call_check: FuncId,
    list_get: FuncId,
    dict_get: FuncId,
    tuple_get: FuncId,
    bytes_get: FuncId,
    str_get: FuncId,
    any_getitem: FuncId,
    obj_len: FuncId,
    obj_contains: FuncId,
    list_concat: FuncId,
    list_repeat: FuncId,
    tuple_concat: FuncId,
    bytes_concat: FuncId,
    bytes_repeat: FuncId,
    list_cmp: FuncId,
    tuple_cmp: FuncId,
    // ── iterator protocol (Phase 4B) ──
    iter_value: FuncId,
    iter_next: FuncId,
    iter_next_no_exc: FuncId,
    iter_is_exhausted: FuncId,
    // ── iteration builtins (Phase 4C) ──
    iter_enumerate: FuncId,
    zip_new: FuncId,
    zipn_new: FuncId,
    list_from_iter: FuncId,
    tuple_from_iter: FuncId,
    dict_from_pairs: FuncId,
    make_bytes_from_list: FuncId,
    make_bytes_zero: FuncId,
    make_bytes_from_str: FuncId,
    sorted: FuncId,
    iter_reversed_list: FuncId,
    iter_range: FuncId,
    // ── container methods (Phase 4D) ──
    list_pop: FuncId,
    list_insert: FuncId,
    list_extend: FuncId,
    list_index: FuncId,
    list_count: FuncId,
    list_clear: FuncId,
    list_copy: FuncId,
    list_reverse: FuncId,
    list_sort: FuncId,
    list_sort_by_keys: FuncId,
    dict_get_default: FuncId,
    dict_keys: FuncId,
    dict_values: FuncId,
    dict_items: FuncId,
    dict_pop: FuncId,
    dict_setdefault: FuncId,
    dict_update: FuncId,
    dict_clear: FuncId,
    dict_copy: FuncId,
    dict_merge: FuncId,
    set_remove: FuncId,
    set_discard: FuncId,
    set_update: FuncId,
    set_union: FuncId,
    set_intersection: FuncId,
    set_difference: FuncId,
    set_copy: FuncId,
    set_clear: FuncId,
    // ── builtin-type methods (§9) ──
    tuple_index: FuncId,
    tuple_count: FuncId,
    set_issubset: FuncId,
    set_issuperset: FuncId,
    set_isdisjoint: FuncId,
    set_intersection_update: FuncId,
    set_difference_update: FuncId,
    set_symmetric_difference_update: FuncId,
    set_symmetric_difference: FuncId,
    list_remove: FuncId,
    dict_popitem: FuncId,
    // ── classes (Phase 5) ──
    make_instance: FuncId,
    instance_get_field: FuncId,
    instance_set_field: FuncId,
    register_class: FuncId,
    register_class_field_count: FuncId,
    register_class_qualname: FuncId,
    // ── inheritance / dispatch (Phase 5B) ──
    register_vtable: FuncId,
    register_method_name: FuncId,
    register_field_name: FuncId,
    getattr_name: FuncId,
    setattr_name: FuncId,
    vtable_lookup_by_name: FuncId,
    isinstance_inherited: FuncId,
    // ── dunders (Phase 5C) ──
    register_dunder_func: FuncId,
    // ── class attributes (Phase 5D) ──
    class_attr_get_ptr: FuncId,
    class_attr_set_ptr: FuncId,
    // ── closures / cells / globals (Phase 6) ──
    make_cell_ptr: FuncId,
    cell_get_ptr: FuncId,
    cell_set_ptr: FuncId,
    global_get_ptr: FuncId,
    global_set_ptr: FuncId,
    // ── generators (Phase 6E) ──
    make_generator: FuncId,
    gen_get_local: FuncId,
    gen_set_local: FuncId,
    gen_get_state: FuncId,
    gen_set_state: FuncId,
    gen_get_sent_value: FuncId,
    gen_set_exhausted: FuncId,
    gen_is_closing: FuncId,
    gen_send: FuncId,
    gen_close: FuncId,
    // ── exceptions (Phase 7, table-based unwinding) ──
    /// `rt_exc_register_table(ptr, count)` — called once from `main` before
    /// user code; hands the runtime this module's PC→handler table.
    exc_register_table: FuncId,
    /// `rt_tb_register_table(ptr, count)` — the PC→(function, file, line)
    /// traceback table (real tracebacks), registered alongside.
    tb_register_table: FuncId,
    exc_raise: FuncId,
    exc_raise_from: FuncId,
    exc_raise_from_none: FuncId,
    exc_raise_stdlib: FuncId,
    exc_raise_custom_with_instance: FuncId,
    exc_raise_instance: FuncId,
    exc_reraise: FuncId,
    exc_start_handling: FuncId,
    exc_end_handling: FuncId,
    /// `rt_exc_isinstance_class` serves BOTH builtin and user-class clauses:
    /// builtin tags are runtime class ids, and the registry walk is what lets
    /// `except ValueError` catch a user `class LimitError(ValueError)` (the
    /// exact-match `rt_exc_isinstance` cannot, so it is not imported).
    exc_isinstance_class: FuncId,
    exc_get_current: FuncId,
    exc_instance_str: FuncId,
    exc_register_class_name: FuncId,
    str_data: FuncId,
    str_len: FuncId,
    /// Runtime helpers that can NEVER reach a raise path: inside a protected
    /// block these stay plain `call`s (everything else becomes a `try_call`
    /// with the handler edge). Conservative — a function missing here only
    /// costs a needless exceptional edge, never a missed handler.
    never_raises: HashSet<FuncId>,
}

impl RuntimeFns {
    fn declare(m: &mut ObjectModule, cc: CallConv, ptr: Type) -> Result<Self> {
        let ti = types::I64;
        let t8 = types::I8;
        let t32 = types::I32;
        let tf = types::F64;
        let mut d = |name: &str, p: &[Type], r: &[Type]| declare_import(m, cc, name, p, r);
        let mut fns = Self {
            init: d("rt_init", &[t32, ptr], &[])?,
            shutdown: d("rt_shutdown", &[], &[])?,
            make_str: d("rt_make_str", &[ptr, ti], &[ti])?,
            bigint_from_str: d("rt_bigint_from_str", &[ptr, ti], &[ti])?,
            box_float: d("rt_box_float", &[tf], &[ti])?,
            unbox_float_checked: d("rt_unbox_float", &[ti], &[tf])?,
            unbox_int_checked: d("rt_unbox_int", &[ti], &[ti])?,
            unbox_bool_checked: d("rt_unbox_bool", &[ti], &[t8])?,
            // Raw i64 arithmetic (Phase 3c): used only on range-proven cursors.
            // These RAISE OverflowError on i64 overflow (unlike CPython's bignum
            // promotion), so they are correct only where overflow provably cannot
            // occur — lowering emits them solely for literal-bounded cursors.
            add_int: d("rt_add_int", &[ti, ti], &[ti])?,
            sub_int: d("rt_sub_int", &[ti, ti], &[ti])?,
            mul_int: d("rt_mul_int", &[ti, ti], &[ti])?,
            obj_add: d("rt_obj_add", &[ti, ti], &[ti])?,
            obj_sub: d("rt_obj_sub", &[ti, ti], &[ti])?,
            obj_mul: d("rt_obj_mul", &[ti, ti], &[ti])?,
            obj_matmul: d("rt_obj_matmul", &[ti, ti], &[ti])?,
            obj_div: d("rt_obj_div", &[ti, ti], &[ti])?,
            obj_floordiv: d("rt_obj_floordiv", &[ti, ti], &[ti])?,
            obj_mod: d("rt_obj_mod", &[ti, ti], &[ti])?,
            obj_pow: d("rt_obj_pow", &[ti, ti], &[ti])?,
            obj_neg: d("rt_obj_neg", &[ti], &[ti])?,
            obj_pos: d("rt_obj_pos", &[ti], &[ti])?,
            obj_invert: d("rt_obj_invert", &[ti], &[ti])?,
            obj_eq: d("rt_obj_eq", &[ti, ti], &[t8])?,
            obj_cmp: d("rt_obj_cmp", &[ti, ti, t8], &[t8])?,
            is_truthy: d("rt_is_truthy", &[ti], &[t8])?,
            obj_bitand: d("rt_obj_bitand", &[ti, ti], &[ti])?,
            obj_bitor: d("rt_obj_bitor", &[ti, ti], &[ti])?,
            obj_ior: d("rt_obj_ior", &[ti, ti], &[ti])?,
            obj_bitxor: d("rt_obj_bitxor", &[ti, ti], &[ti])?,
            obj_lshift: d("rt_obj_lshift", &[ti, ti], &[ti])?,
            obj_rshift: d("rt_obj_rshift", &[ti, ti], &[ti])?,
            builtin_abs: d("rt_builtin_abs", &[ti], &[ti])?,
            builtin_hash: d("rt_builtin_hash", &[ti], &[ti])?,
            builtin_int: d("rt_builtin_int", &[ti], &[ti])?,
            builtin_float: d("rt_builtin_float", &[ti], &[ti])?,
            builtin_str: d("rt_builtin_str", &[ti], &[ti])?,
            builtin_repr: d("rt_builtin_repr", &[ti], &[ti])?,
            builtin_bool: d("rt_builtin_bool", &[ti], &[ti])?,
            builtin_len: d("rt_builtin_len", &[ti], &[ti])?,
            builtin_ord: d("rt_builtin_ord", &[ti], &[ti])?,
            builtin_chr: d("rt_builtin_chr", &[ti], &[ti])?,
            builtin_type: d("rt_builtin_type", &[ti], &[ti])?,
            builtin_ascii: d("rt_builtin_ascii", &[ti], &[ti])?,
            assert_fail: d("rt_assert_fail", &[ptr], &[])?,
            print_int: d("rt_print_int_value", &[ti], &[])?,
            print_float: d("rt_print_float_value", &[tf], &[])?,
            print_bool: d("rt_print_bool_value", &[t8], &[])?,
            print_none: d("rt_print_none_value", &[], &[])?,
            print_str_obj: d("rt_print_str_obj", &[ti], &[])?,
            print_obj: d("rt_print_obj", &[ti], &[])?,
            print_sep: d("rt_print_sep", &[], &[])?,
            print_newline: d("rt_print_newline", &[], &[])?,
            gc_push: d("gc_push", &[ptr], &[])?,
            gc_pop: d("gc_pop", &[], &[])?,
            // Containers (all take/return tagged `Value` = i64 unless noted).
            make_list: d("rt_make_list", &[ti], &[ti])?,
            make_dict: d("rt_make_dict", &[ti], &[ti])?,
            make_set: d("rt_make_set", &[ti], &[ti])?,
            make_tuple: d("rt_make_tuple", &[ti], &[ti])?,
            make_closure: d("rt_make_closure", &[ti], &[ti])?,
            make_bytes: d("rt_make_bytes", &[ptr, ptr], &[ti])?,
            list_push: d("rt_list_push", &[ti, ti], &[])?,
            list_set: d("rt_list_set", &[ti, ti, ti], &[])?,
            dict_set: d("rt_dict_set", &[ti, ti, ti], &[])?,
            set_add: d("rt_set_add", &[ti, ti], &[])?,
            tuple_set: d("rt_tuple_set", &[ti, ti, ti], &[])?,
            call_check: d("rt_call_check", &[ti], &[ti])?,
            list_get: d("rt_list_get", &[ti, ti], &[ti])?,
            dict_get: d("rt_dict_get", &[ti, ti], &[ti])?,
            tuple_get: d("rt_tuple_get", &[ti, ti], &[ti])?,
            bytes_get: d("rt_bytes_get", &[ti, ti], &[ti])?,
            str_get: d("rt_str_subscript", &[ti, ti], &[ti])?,
            any_getitem: d("rt_any_getitem", &[ti, ti], &[ti])?,
            obj_len: d("rt_obj_len", &[ti], &[ti])?,
            obj_contains: d("rt_obj_contains", &[ti, ti], &[t8])?,
            list_concat: d("rt_list_concat", &[ti, ti], &[ti])?,
            list_repeat: d("rt_list_repeat", &[ti, ti], &[ti])?,
            tuple_concat: d("rt_tuple_concat", &[ti, ti], &[ti])?,
            bytes_concat: d("rt_bytes_concat", &[ti, ti], &[ti])?,
            bytes_repeat: d("rt_bytes_repeat", &[ti, ti], &[ti])?,
            list_cmp: d("rt_list_cmp", &[ti, ti, t8], &[t8])?,
            tuple_cmp: d("rt_tuple_cmp", &[ti, ti, t8], &[t8])?,
            iter_value: d("rt_iter_value", &[ti], &[ti])?,
            // The raising variant (StopIteration on exhaustion) — `next(x)`.
            iter_next: d("rt_iter_next", &[ti], &[ti])?,
            iter_next_no_exc: d("rt_iter_next_no_exc", &[ti], &[ti])?,
            iter_is_exhausted: d("rt_iter_is_exhausted", &[ti], &[t8])?,
            iter_enumerate: d("rt_iter_enumerate", &[ti, ti], &[ti])?,
            zip_new: d("rt_zip_new", &[ti, ti], &[ti])?,
            // rt_zipn_new(iters_list: Value, count: i64) -> iterator
            zipn_new: d("rt_zipn_new", &[ti, ti], &[ti])?,
            list_from_iter: d("rt_list_from_iter", &[ti], &[ti])?,
            tuple_from_iter: d("rt_tuple_from_iter", &[ti], &[ti])?,
            dict_from_pairs: d("rt_dict_from_pairs", &[ti], &[ti])?,
            make_bytes_from_list: d("rt_make_bytes_from_list", &[ti], &[ti])?,
            // `rt_make_bytes_zero(len)` — the length is a raw `i64`; `rt_make_bytes_from_str(str)`
            // takes a tagged str pointer. Both are codegen-level `I64`.
            make_bytes_zero: d("rt_make_bytes_zero", &[ti], &[ti])?,
            make_bytes_from_str: d("rt_make_bytes_from_str", &[ti], &[ti])?,
            sorted: d("rt_sorted", &[ti, t8, t8], &[ti])?,
            iter_reversed_list: d("rt_iter_reversed_list", &[ti], &[ti])?,
            iter_range: d("rt_iter_range", &[ti, ti, ti], &[ti])?,
            list_pop: d("rt_list_pop", &[ti, ti], &[ti])?,
            list_insert: d("rt_list_insert", &[ti, ti, ti], &[])?,
            list_extend: d("rt_list_extend", &[ti, ti], &[])?,
            list_index: d("rt_list_index", &[ti, ti], &[ti])?,
            list_count: d("rt_list_count", &[ti, ti], &[ti])?,
            list_clear: d("rt_list_clear", &[ti], &[])?,
            list_copy: d("rt_list_copy", &[ti], &[ti])?,
            list_reverse: d("rt_list_reverse", &[ti], &[])?,
            list_sort: d("rt_list_sort", &[ti, t8], &[])?,
            list_sort_by_keys: d("rt_list_sort_by_keys", &[ti, ti, t8], &[])?,
            dict_get_default: d("rt_dict_get_default", &[ti, ti, ti], &[ti])?,
            dict_keys: d("rt_dict_keys", &[ti], &[ti])?,
            dict_values: d("rt_dict_values", &[ti], &[ti])?,
            dict_items: d("rt_dict_items", &[ti], &[ti])?,
            dict_pop: d("rt_dict_pop", &[ti, ti], &[ti])?,
            dict_setdefault: d("rt_dict_setdefault", &[ti, ti, ti], &[ti])?,
            dict_update: d("rt_dict_update", &[ti, ti], &[])?,
            dict_clear: d("rt_dict_clear", &[ti], &[])?,
            dict_copy: d("rt_dict_copy", &[ti], &[ti])?,
            dict_merge: d("rt_dict_merge", &[ti, ti], &[ti])?,
            set_remove: d("rt_set_remove", &[ti, ti], &[])?,
            set_discard: d("rt_set_discard", &[ti, ti], &[])?,
            set_update: d("rt_set_update", &[ti, ti], &[])?,
            set_union: d("rt_set_union", &[ti, ti], &[ti])?,
            set_intersection: d("rt_set_intersection", &[ti, ti], &[ti])?,
            set_difference: d("rt_set_difference", &[ti, ti], &[ti])?,
            set_copy: d("rt_set_copy", &[ti], &[ti])?,
            set_clear: d("rt_set_clear", &[ti], &[])?,
            // §9 builtin-type methods. tuple index/count → raw i64; set
            // comparisons → i8; set *_update mutate in place (void); popitem
            // returns a fresh 2-tuple Value.
            tuple_index: d("rt_tuple_index", &[ti, ti], &[ti])?,
            tuple_count: d("rt_tuple_count", &[ti, ti], &[ti])?,
            set_issubset: d("rt_set_issubset", &[ti, ti], &[t8])?,
            set_issuperset: d("rt_set_issuperset", &[ti, ti], &[t8])?,
            set_isdisjoint: d("rt_set_isdisjoint", &[ti, ti], &[t8])?,
            set_intersection_update: d("rt_set_intersection_update", &[ti, ti], &[])?,
            set_difference_update: d("rt_set_difference_update", &[ti, ti], &[])?,
            set_symmetric_difference_update: d(
                "rt_set_symmetric_difference_update",
                &[ti, ti],
                &[],
            )?,
            // new-set symmetric difference (distinct from the *_update above);
            // list.remove returns i8 (1 on success / ValueError on miss) — the
            // result is discarded (a None-returning mutation).
            set_symmetric_difference: d("rt_set_symmetric_difference", &[ti, ti], &[ti])?,
            list_remove: d("rt_list_remove", &[ti, ti], &[t8])?,
            dict_popitem: d("rt_dict_popitem", &[ti], &[ti])?,
            // Classes (Phase 5). `class_id` is a `u8` → `i8` at the ABI; instance
            // values + the qualname `StrObj` are tagged `Value` = i64.
            make_instance: d("rt_make_instance", &[t8, ti], &[ti])?,
            instance_get_field: d("rt_instance_get_field", &[ti, ti], &[ti])?,
            instance_set_field: d("rt_instance_set_field", &[ti, ti, ti], &[])?,
            register_class: d("rt_register_class", &[t8, t8], &[])?,
            register_class_field_count: d("rt_register_class_field_count", &[t8, ti], &[])?,
            register_class_qualname: d("rt_register_class_qualname", &[ti, ti], &[])?,
            // Inheritance / dispatch. The vtable ptr is a code/data address (i64);
            // `rt_vtable_lookup_by_name` takes the instance Value + name hash → fn ptr.
            register_vtable: d("rt_register_vtable", &[t8, ti], &[])?,
            register_method_name: d("rt_register_method_name", &[ti, ti, ti], &[])?,
            register_field_name: d("rt_register_field_name", &[ti, ti, ti], &[])?,
            getattr_name: d("rt_getattr_name", &[ti, ti], &[ti])?,
            setattr_name: d("rt_setattr_name", &[ti, ti, ti], &[])?,
            vtable_lookup_by_name: d("rt_vtable_lookup_by_name", &[ti, ti], &[ti])?,
            isinstance_inherited: d("rt_isinstance_class_inherited", &[ti, ti], &[t8])?,
            register_dunder_func: d("rt_register_dunder_func", &[ti, ti, ti], &[])?,
            // Class attributes by (class_id: u8, attr_idx: u32) → tagged Value.
            class_attr_get_ptr: d("rt_class_attr_get_ptr", &[t8, t32], &[ti])?,
            class_attr_set_ptr: d("rt_class_attr_set_ptr", &[t8, t32, ti], &[])?,
            // Cells hold full tagged Value bits (P6-2: ONLY the ptr variants —
            // the typed int/float/bool cell variants hide heap pointers from
            // the GC and are never emitted). Globals likewise (GC-rooted).
            make_cell_ptr: d("rt_make_cell_ptr", &[ti], &[ti])?,
            cell_get_ptr: d("rt_cell_get_ptr", &[ti], &[ti])?,
            cell_set_ptr: d("rt_cell_set_ptr", &[ti, ti], &[])?,
            global_get_ptr: d("rt_global_get_ptr", &[t32], &[ti])?,
            global_set_ptr: d("rt_global_set_ptr", &[t32, ti], &[])?,
            // Generators (Phase 6E). Slot reads/writes use the Ptr variants
            // (full tagged Value bits, GC-traced); state/slot indices are u32.
            make_generator: d("rt_make_generator", &[t32, t32], &[ti])?,
            gen_get_local: d("rt_generator_get_local_ptr", &[ti, t32], &[ti])?,
            gen_set_local: d("rt_generator_set_local_ptr", &[ti, t32, ti], &[])?,
            gen_get_state: d("rt_generator_get_state", &[ti], &[t32])?,
            gen_set_state: d("rt_generator_set_state", &[ti, t32], &[])?,
            gen_get_sent_value: d("rt_generator_get_sent_value", &[ti], &[ti])?,
            gen_set_exhausted: d("rt_generator_set_exhausted", &[ti], &[])?,
            gen_is_closing: d("rt_generator_is_closing", &[ti], &[t8])?,
            gen_send: d("rt_generator_send", &[ti, ti], &[ti])?,
            gen_close: d("rt_generator_close", &[ti], &[])?,
            // Exceptions (Phase 7, table-based unwinding). Tags / class ids
            // are u8 at the ABI; message pointers + lengths come from
            // rt_str_data/rt_str_len of a StrObj.
            exc_register_table: d("rt_exc_register_table", &[ptr, ti], &[])?,
            tb_register_table: d("rt_tb_register_table", &[ptr, ti], &[])?,
            exc_raise: d("rt_exc_raise", &[t8, ptr, ti], &[])?,
            exc_raise_from: d("rt_exc_raise_from", &[t8, ptr, ti, t8, ptr, ti], &[])?,
            exc_raise_from_none: d("rt_exc_raise_from_none", &[t8, ptr, ti], &[])?,
            exc_raise_stdlib: d("rt_exc_raise_stdlib", &[t8, t8, ptr, ti], &[])?,
            exc_raise_custom_with_instance: d(
                "rt_exc_raise_custom_with_instance",
                &[t8, ptr, ti, ti],
                &[],
            )?,
            exc_raise_instance: d("rt_exc_raise_instance", &[ti], &[])?,
            exc_reraise: d("rt_exc_reraise", &[], &[])?,
            exc_start_handling: d("rt_exc_start_handling", &[], &[])?,
            exc_end_handling: d("rt_exc_end_handling", &[], &[])?,
            exc_isinstance_class: d("rt_exc_isinstance_class", &[t8], &[t8])?,
            exc_get_current: d("rt_exc_get_current", &[], &[ti])?,
            exc_instance_str: d("rt_exc_instance_str", &[ti], &[ti])?,
            exc_register_class_name: d("rt_exc_register_class_name", &[t8, ptr, ti], &[])?,
            str_data: d("rt_str_data", &[ti], &[ptr])?,
            str_len: d("rt_str_len", &[ti], &[ti])?,
            never_raises: HashSet::new(),
        };
        fns.never_raises = [
            fns.gc_push,
            fns.gc_pop,
            fns.str_data,
            fns.str_len,
            fns.exc_start_handling,
            fns.exc_end_handling,
            fns.exc_isinstance_class,
            fns.exc_get_current,
        ]
        .into_iter()
        .collect();
        Ok(fns)
    }
}

/// Build an [`AbiParam`] for a GENERATED function's signature. Plain — both
/// sides of every generated↔generated call are Cranelift+SystemV, which
/// assumes no sub-word extension (Cranelift's `get_ext_mode` ignores
/// extension attributes for SystemV), so caller and callee agree by
/// construction.
fn abi_param(ty: Type) -> AbiParam {
    AbiParam::new(ty)
}

/// Import-signature param: sub-word integers are declared (and passed) as
/// I32. Rust/Clang callees may assume `zeroext` on `u8`/`bool` parameters
/// (the Apple aarch64 ABI mandates the extension; LLVM emits the assumption
/// on x86-64 SysV too), but Cranelift's SystemV lowering IGNORES `uext`
/// attributes — so the widening lives in the signature itself, with an
/// explicit `uextend` at each call site ([`FnGen::call`] inserts it on type
/// mismatch). The register-level contents satisfy both sides: low byte is
/// the value, high bits are zero.
fn import_param(ty: Type) -> AbiParam {
    if ty == types::I8 || ty == types::I16 {
        AbiParam::new(types::I32)
    } else {
        AbiParam::new(ty)
    }
}

fn declare_import(
    module: &mut ObjectModule,
    cc: CallConv,
    name: &str,
    params: &[Type],
    returns: &[Type],
) -> Result<FuncId> {
    let mut sig = Signature::new(cc);
    sig.params.extend(params.iter().copied().map(import_param));
    // Returns keep their narrow type: Cranelift only ever reads the low bits
    // of a sub-word return, so no extension assumption is made either way.
    sig.returns
        .extend(returns.iter().copied().map(AbiParam::new));
    module
        .declare_function(name, Linkage::Import, &sig)
        .map_err(|e| cg_error(format!("declare import `{name}`: {e}")))
}

/// Cranelift optimization level for [`compile`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptLevel {
    /// No Cranelift optimization (`opt_level=none`) — the always-sound mode
    /// the whole corpus was brought up on.
    None,
    /// `opt_level=speed` — egraph optimization.
    Speed,
    /// `opt_level=speed_and_size`.
    SpeedAndSize,
}

/// Codegen knobs threaded from the CLI into [`compile`].
///
/// Exception handling is table-based (`try_call` + the unwind table), so no
/// opt-level/aliasing interaction with the exceptional path remains: handler
/// edges are ordinary CFG edges Cranelift understands (PITFALLS B17 now
/// covers the unwinder's frame-walk preconditions instead).
#[derive(Debug, Clone, Copy)]
pub struct CodegenOptions {
    pub opt_level: OptLevel,
    /// `enable_alias_analysis` Cranelift flag (only meaningful at `Speed`+).
    pub alias_analysis: bool,
}

impl Default for CodegenOptions {
    fn default() -> Self {
        Self {
            opt_level: OptLevel::Speed,
            alias_analysis: true,
        }
    }
}

/// Compile a [`MirProgram`] to a native object file at `out_obj`. `interner`
/// resolves each function's Python name into its symbol
/// (`pyaot_fn_<i>_<name>`), so backtraces and profiles read as Python.
pub fn compile(
    program: &MirProgram,
    out_obj: &Path,
    opts: &CodegenOptions,
    interner: &StringInterner,
) -> Result<()> {
    let mut flag_builder = settings::builder();
    flag_builder
        .set("is_pic", "true")
        .map_err(|e| cg_error(format!("set is_pic: {e}")))?;
    flag_builder
        .set("use_colocated_libcalls", "false")
        .map_err(|e| cg_error(format!("set use_colocated_libcalls: {e}")))?;
    let opt_level = match opts.opt_level {
        OptLevel::None => "none",
        OptLevel::Speed => "speed",
        OptLevel::SpeedAndSize => "speed_and_size",
    };
    flag_builder
        .set("opt_level", opt_level)
        .map_err(|e| cg_error(format!("set opt_level: {e}")))?;
    flag_builder
        .set(
            "enable_alias_analysis",
            if opts.alias_analysis { "true" } else { "false" },
        )
        .map_err(|e| cg_error(format!("set enable_alias_analysis: {e}")))?;
    // The runtime unwinder walks the FP chain to find handlers — every frame
    // (leaf functions included) must link it (mandatory on macOS arm64
    // anyway; this makes Linux x86_64 behave the same).
    flag_builder
        .set("preserve_frame_pointers", "true")
        .map_err(|e| cg_error(format!("set preserve_frame_pointers: {e}")))?;
    let flags = settings::Flags::new(flag_builder);

    let isa_builder =
        cranelift_native::builder().map_err(|e| cg_error(format!("host ISA detection: {e}")))?;
    let isa = isa_builder
        .finish(flags)
        .map_err(|e| cg_error(format!("ISA finish: {e}")))?;

    let builder = ObjectBuilder::new(isa, "pyaot_module", default_libcall_names())
        .map_err(|e| cg_error(format!("object builder: {e}")))?;
    let mut module = ObjectModule::new(builder);

    let ptr_ty = module.target_config().pointer_type();
    // `CallConv::SystemV` everywhere — `try_call` callees must use a
    // convention with `supports_exceptions()` (SystemV/Tail), which rules out
    // `AppleAarch64`. On aarch64 SystemV is standard AAPCS64; it diverges from
    // Apple's convention only for sub-8-byte stack arguments and varargs,
    // neither of which crosses our seam (every cross-ABI argument is a
    // register-class i64/f64/pointer, and sub-word register args are
    // explicitly `uext`-extended via [`abi_param`]).
    let call_conv = CallConv::SystemV;

    let rt = RuntimeFns::declare(&mut module, call_conv, ptr_ty)?;

    // One data object per interned string (literal bytes or big-int decimals).
    // Store the byte length alongside the id (Cranelift does not expose it back).
    let mut data_ids: HashMap<InternedString, (DataId, u32)> = HashMap::new();
    for (interned, bytes) in program.str_pool.iter() {
        let name = format!("pyaot_str_{}", interned.index());
        let data_id = module
            .declare_data(&name, Linkage::Local, false, false)
            .map_err(|e| cg_error(format!("declare data `{name}`: {e}")))?;
        let mut desc = DataDescription::new();
        desc.define(bytes.to_vec().into_boxed_slice());
        module
            .define_data(data_id, &desc)
            .map_err(|e| cg_error(format!("define data `{name}`: {e}")))?;
        data_ids.insert(interned, (data_id, bytes.len() as u32));
    }

    // Declare every MIR function (so calls can reference forward / recursively).
    let mut func_ids: Vec<FuncId> = Vec::with_capacity(program.funcs.len());
    for (i, mf) in program.funcs.iter().enumerate() {
        let mut sig = Signature::new(call_conv);
        for p in &mf.params {
            sig.params.push(abi_param(clif_ty(p)));
        }
        sig.returns.push(AbiParam::new(clif_ty(&mf.ret)));
        // Symbol = index + sanitized Python name (Phase 9E debug polish):
        // `pyaot_fn_7___add__` in a backtrace/profile beats `pyaot_fn_7`. The
        // index prefix keeps symbols unique across same-named methods.
        let py_name: String = interner
            .resolve(mf.name)
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let name = format!("pyaot_fn_{i}_{py_name}");
        let id = module
            .declare_function(&name, Linkage::Local, &sig)
            .map_err(|e| cg_error(format!("declare `{name}`: {e}")))?;
        func_ids.push(id);
    }

    // One static vtable data object per class (Phase 5B): `[num_slots: u64,
    // fn_ptr…]`, each fn_ptr a relocation to the resolved method's address. Its
    // pointer is registered in `__pyaot_classinit` via `rt_register_vtable`.
    let mut vtable_ids: HashMap<u32, DataId> = HashMap::new();
    for c in &program.classes {
        if c.vtable.is_empty() {
            continue;
        }
        let num_slots = c.vtable.len();
        let mut bytes = vec![0u8; pyaot_core_defs::layout::vtable_data_size(num_slots)];
        bytes[0..8].copy_from_slice(&(num_slots as u64).to_le_bytes());
        let name = format!("pyaot_vtable_{}", c.class_id.0);
        let data_id = module
            .declare_data(&name, Linkage::Local, false, false)
            .map_err(|e| cg_error(format!("declare vtable `{name}`: {e}")))?;
        let mut desc = DataDescription::new();
        desc.define(bytes.into_boxed_slice());
        for (slot, fid) in c.vtable.iter().enumerate() {
            let cl_fid = func_ids[fid.index()];
            let fref = module.declare_func_in_data(cl_fid, &mut desc);
            desc.write_function_addr(
                pyaot_core_defs::layout::vtable_slot_offset(slot) as u32,
                fref,
            );
        }
        module
            .define_data(data_id, &desc)
            .map_err(|e| cg_error(format!("define vtable `{name}`: {e}")))?;
        vtable_ids.insert(c.class_id.0, data_id);
    }

    // Define each function body, harvesting its protected call sites for the
    // program-wide unwind table and its line ranges for the traceback table.
    let mut exc_sites: Vec<(FuncId, Vec<ExcSite>)> = Vec::new();
    let mut tb_funcs: Vec<(FuncId, String, String, FnTb)> = Vec::new();
    let mut trampolines = Trampolines::default();
    for (i, mf) in program.funcs.iter().enumerate() {
        let (sites, tb) = define_function(
            &mut module,
            mf,
            func_ids[i],
            &func_ids,
            &rt,
            &data_ids,
            ptr_ty,
            call_conv,
            &mut trampolines,
        )?;
        if !sites.is_empty() {
            exc_sites.push((func_ids[i], sites));
        }
        // Display name: module bodies (`__main__`, imported-module `<init>`s)
        // print as CPython's `<module>`; everything else by the Python name.
        let py_name = interner.resolve(mf.name);
        let display = if py_name == "__main__" || py_name.ends_with(".<init>") {
            "<module>".to_string()
        } else {
            py_name.to_string()
        };
        tb_funcs.push((
            func_ids[i],
            display,
            interner.resolve(mf.file).to_string(),
            tb,
        ));
    }
    define_trampolines(&mut module, trampolines)?;

    let exc_table = emit_exc_table(&mut module, &exc_sites)?;
    let tb_table = emit_tb_table(&mut module, &tb_funcs)?;

    // Class registration (`__pyaot_classinit`) runs before `__main__`, so every
    // class is registered before any instance is created (incl. module-top-level
    // ones). Emitted only when the program defines classes.
    let classinit = if program.classes.is_empty() {
        None
    } else {
        Some(emit_classinit(
            &mut module,
            program,
            &rt,
            &data_ids,
            &vtable_ids,
            &func_ids,
            ptr_ty,
            call_conv,
        )?)
    };

    emit_main(
        &mut module,
        func_ids[program.entry.index()],
        classinit,
        exc_table,
        tb_table,
        &rt,
        ptr_ty,
        call_conv,
    )?;
    emit_generator_dispatch(&mut module, program, &func_ids, ptr_ty, call_conv)?;

    let product = module.finish();
    let bytes = product
        .emit()
        .map_err(|e| cg_error(format!("object emit: {e}")))?;
    std::fs::write(out_obj, bytes)
        .map_err(|e| cg_error(format!("write {}: {e}", out_obj.display())))?;
    Ok(())
}

/// Emit `__pyaot_generator_resume(gen) -> gen`: the dispatcher the runtime's
/// `rt_generator_next/send/close` call (Phase 6E). It loads the generator's
/// stored `func_id` (a `u32` at the frozen `GENERATOR_FUNC_ID_OFFSET`) and
/// compare-chains it against each `gen_id` → calls the matching resume fn and
/// returns its result; an unmatched id (or an empty table) traps. The resume
/// fns have the compiled signature `(gen: i64) -> i64`, so the dispatcher
/// reuses the platform pointer type as the `Value` ABI.
fn emit_generator_dispatch(
    module: &mut ObjectModule,
    program: &MirProgram,
    func_ids: &[FuncId],
    ptr_ty: Type,
    cc: CallConv,
) -> Result<()> {
    let mut sig = Signature::new(cc);
    sig.params.push(AbiParam::new(ptr_ty));
    sig.returns.push(AbiParam::new(ptr_ty));
    let id = module
        .declare_function("__pyaot_generator_resume", Linkage::Export, &sig)
        .map_err(|e| cg_error(format!("declare generator dispatch: {e}")))?;
    let mut ctx = module.make_context();
    ctx.func.signature = sig.clone();
    let mut fctx = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);
        let gen = builder.block_params(entry)[0];

        // Empty table → keep today's trap (no generator is ever created).
        if program.generators.is_empty() {
            builder.ins().trap(TrapCode::unwrap_user(2));
            builder.finalize();
        } else {
            let offset = pyaot_core_defs::layout::GENERATOR_FUNC_ID_OFFSET;
            let func_id_val = builder
                .ins()
                .load(types::I32, MemFlags::trusted(), gen, offset);
            // Compare-chain: for each gen_id, if func_id == gen_id call its
            // resume fn and return; else fall to the next test.
            for (gen_id, resume_fid) in program.generators.iter().enumerate() {
                let matches = builder.create_block();
                let next = builder.create_block();
                let want = builder.ins().iconst(types::I32, gen_id as i64);
                let eq = builder.ins().icmp(IntCC::Equal, func_id_val, want);
                builder.ins().brif(eq, matches, &[], next, &[]);

                builder.switch_to_block(matches);
                builder.seal_block(matches);
                let fref = module.declare_func_in_func(func_ids[resume_fid.index()], builder.func);
                let call = builder.ins().call(fref, &[gen]);
                let res = builder.inst_results(call)[0];
                builder.ins().return_(&[res]);

                builder.switch_to_block(next);
                builder.seal_block(next);
            }
            // Unmatched func_id → trap (a corrupt generator object).
            builder.ins().trap(TrapCode::unwrap_user(2));
            builder.finalize();
        }
    }
    module
        .define_function(id, &mut ctx)
        .map_err(|e| cg_error(format!("define generator dispatch: {e}")))?;
    module.clear_context(&mut ctx);
    Ok(())
}

/// Emit `__pyaot_classinit`: register every class (`rt_register_class` with its
/// real parent, `_field_count`, `_qualname`; 5B adds vtables + method names, 5C
/// dunders). Called from `main()` between `rt_init` and `__main__` so all classes
/// are registered before any instance is created.
#[allow(clippy::too_many_arguments)]
fn emit_classinit(
    module: &mut ObjectModule,
    program: &MirProgram,
    rt: &RuntimeFns,
    data_ids: &HashMap<InternedString, (DataId, u32)>,
    vtable_ids: &HashMap<u32, DataId>,
    func_ids: &[FuncId],
    _ptr_ty: Type,
    cc: CallConv,
) -> Result<FuncId> {
    let sig = Signature::new(cc);
    let id = module
        .declare_function("__pyaot_classinit", Linkage::Local, &sig)
        .map_err(|e| cg_error(format!("declare __pyaot_classinit: {e}")))?;
    let mut ctx = module.make_context();
    ctx.func.signature = sig;
    let mut fctx = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let entry = builder.create_block();
        builder.switch_to_block(entry);
        builder.seal_block(entry);

        for c in &program.classes {
            let cid8 = builder.ins().iconst(types::I32, c.class_id.0 as i64);
            // Runtime parent: a user parent; else, for an exception class, its
            // builtin base tag (builtin tags ARE runtime class ids, so the
            // registry walk reaches the pre-seeded builtin hierarchy — 7C);
            // else 255 = no parent (NO_PARENT sentinel).
            let parent = c
                .parent
                .map(|p| p.0 as i64)
                .or(c.exception_base.map(|t| t as i64))
                .unwrap_or(255);
            let parent8 = builder.ins().iconst(types::I32, parent);
            let rc = module.declare_func_in_func(rt.register_class, builder.func);
            builder.ins().call(rc, &[cid8, parent8]);

            // Exception classes also register their bare name for display.
            if c.exception_base.is_some() {
                let (data_id, len) = *data_ids
                    .get(&c.name)
                    .ok_or_else(|| cg_error("missing data object for exception class name"))?;
                let gv = module.declare_data_in_func(data_id, builder.func);
                let nptr = builder.ins().global_value(_ptr_ty, gv);
                let nlen = builder.ins().iconst(types::I64, len as i64);
                let cid8n = builder.ins().iconst(types::I32, c.class_id.0 as i64);
                let regn = module.declare_func_in_func(rt.exc_register_class_name, builder.func);
                builder.ins().call(regn, &[cid8n, nptr, nlen]);
            }

            let cid8b = builder.ins().iconst(types::I32, c.class_id.0 as i64);
            let fc = builder.ins().iconst(types::I64, c.field_count as i64);
            let rfc = module.declare_func_in_func(rt.register_class_field_count, builder.func);
            builder.ins().call(rfc, &[cid8b, fc]);

            // qualname: build the StrObj, then register it for the default repr.
            let (data_id, len) = *data_ids
                .get(&c.qualname)
                .ok_or_else(|| cg_error("missing data object for class qualname"))?;
            let gv = module.declare_data_in_func(data_id, builder.func);
            let qptr = builder.ins().global_value(_ptr_ty, gv);
            let qlen = builder.ins().iconst(types::I64, len as i64);
            let mks = module.declare_func_in_func(rt.make_str, builder.func);
            let scall = builder.ins().call(mks, &[qptr, qlen]);
            let str_v = builder.inst_results(scall)[0];
            let cid64 = builder.ins().iconst(types::I64, c.class_id.0 as i64);
            let rqn = module.declare_func_in_func(rt.register_class_qualname, builder.func);
            builder.ins().call(rqn, &[cid64, str_v]);

            // Vtable + per-method name→slot registrations (Phase 5B), so the
            // dynamic `rt_vtable_lookup_by_name` path always resolves.
            if let Some(vt_data) = vtable_ids.get(&c.class_id.0) {
                let gv = module.declare_data_in_func(*vt_data, builder.func);
                let vptr = builder.ins().global_value(_ptr_ty, gv);
                let cid8c = builder.ins().iconst(types::I32, c.class_id.0 as i64);
                let rv = module.declare_func_in_func(rt.register_vtable, builder.func);
                builder.ins().call(rv, &[cid8c, vptr]);

                for (name_hash, slot) in &c.method_names {
                    let cidh = builder.ins().iconst(types::I64, c.class_id.0 as i64);
                    let hashv = builder.ins().iconst(types::I64, *name_hash as i64);
                    let slotv = builder.ins().iconst(types::I64, *slot as i64);
                    let rmn = module.declare_func_in_func(rt.register_method_name, builder.func);
                    builder.ins().call(rmn, &[cidh, hashv, slotv]);
                }
            }

            // Field name→slot registrations (Phase 8H, D4) — OUTSIDE the
            // vtable if-let: classes without a vtable still have fields, and
            // the by-name `rt_getattr_name` path must resolve them.
            for (name_hash, slot) in &c.field_names {
                let cidh = builder.ins().iconst(types::I64, c.class_id.0 as i64);
                let hashv = builder.ins().iconst(types::I64, *name_hash as i64);
                let slotv = builder.ins().iconst(types::I64, *slot as i64);
                let rfn = module.declare_func_in_func(rt.register_field_name, builder.func);
                builder.ins().call(rfn, &[cidh, hashv, slotv]);
            }

            // Dunder function registrations (Phase 5C): so the runtime's
            // registry-dispatched ops (`rt_obj_add`/`rt_obj_neg`/the default-repr
            // path) resolve `a + b` / `print(a)` for instances of this class.
            for (name_hash, fid) in &c.dunders {
                let cidd = builder.ins().iconst(types::I64, c.class_id.0 as i64);
                let hashv = builder.ins().iconst(types::I64, *name_hash as i64);
                let fref = module.declare_func_in_func(func_ids[fid.index()], builder.func);
                let addr = builder.ins().func_addr(_ptr_ty, fref);
                let rdf = module.declare_func_in_func(rt.register_dunder_func, builder.func);
                builder.ins().call(rdf, &[cidd, hashv, addr]);
            }

            // Class-attribute initializers (Phase 5D): materialize each literal and
            // store it into its (class_id, attr_idx) slot.
            for (attr_idx, val) in &c.class_attr_inits {
                let v = materialize_const(&mut builder, module, rt, data_ids, _ptr_ty, val)?;
                let cida = builder.ins().iconst(types::I32, c.class_id.0 as i64);
                let idxa = builder.ins().iconst(types::I32, *attr_idx as i64);
                let setp = module.declare_func_in_func(rt.class_attr_set_ptr, builder.func);
                builder.ins().call(setp, &[cida, idxa, v]);
            }
        }
        builder.ins().return_(&[]);
        builder.finalize();
    }
    module
        .define_function(id, &mut ctx)
        .map_err(|e| cg_error(format!("define __pyaot_classinit: {e}")))?;
    module.clear_context(&mut ctx);
    Ok(id)
}

/// Materialize a [`Const`] into a Cranelift `Value` in a free builder context
/// (used by `__pyaot_classinit` for class-attribute initializers). Mirrors
/// `FnGen::lower_const`, but standalone (no per-function state).
fn materialize_const(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    rt: &RuntimeFns,
    data_ids: &HashMap<InternedString, (DataId, u32)>,
    ptr_ty: Type,
    val: &Const,
) -> Result<Value> {
    let str_data = |module: &mut ObjectModule,
                    builder: &mut FunctionBuilder,
                    id: InternedString|
     -> Result<(Value, Value)> {
        let (data_id, len) = *data_ids
            .get(&id)
            .ok_or_else(|| cg_error("missing data object for class-attr literal"))?;
        let gv = module.declare_data_in_func(data_id, builder.func);
        let ptr = builder.ins().global_value(ptr_ty, gv);
        let len_val = builder.ins().iconst(types::I64, len as i64);
        Ok((ptr, len_val))
    };
    let call1 = |module: &mut ObjectModule,
                 builder: &mut FunctionBuilder,
                 fid: FuncId,
                 args: &[Value]|
     -> Value {
        let fref = module.declare_func_in_func(fid, builder.func);
        let inst = builder.ins().call(fref, args);
        builder.inst_results(inst)[0]
    };
    let v = match val {
        Const::Int(i) => {
            let tagged = ((*i) << tag::INT_SHIFT) | (tag::INT_TAG as i64);
            builder.ins().iconst(types::I64, tagged)
        }
        Const::Bool(b) => {
            let tagged = if *b {
                ((1i64) << tag::BOOL_SHIFT) | (tag::BOOL_TAG as i64)
            } else {
                tag::BOOL_TAG as i64
            };
            builder.ins().iconst(types::I64, tagged)
        }
        Const::None => builder.ins().iconst(types::I64, tag::NONE_TAG as i64),
        // Never produced for class-attr initializers; materialize the raw null
        // Value for exhaustiveness.
        Const::NullPtr => builder.ins().iconst(types::I64, 0),
        // The `Value::UNBOUND` sentinel (`RESERVED_TAG` immediate).
        Const::Unbound => builder.ins().iconst(types::I64, tag::RESERVED_TAG as i64),
        Const::Float(f) => {
            let fv = builder.ins().f64const(*f);
            call1(module, builder, rt.box_float, &[fv])
        }
        Const::Str(id) => {
            let (ptr, len) = str_data(module, builder, *id)?;
            call1(module, builder, rt.make_str, &[ptr, len])
        }
        Const::Bytes(id) => {
            let (ptr, len) = str_data(module, builder, *id)?;
            call1(module, builder, rt.make_bytes, &[ptr, len])
        }
        Const::BigIntStr(id) => {
            let (ptr, len) = str_data(module, builder, *id)?;
            call1(module, builder, rt.bigint_from_str, &[ptr, len])
        }
    };
    Ok(v)
}

/// Emit the unwind-table data object: `count` 24-byte records
/// `{ func_addr: ptr (relocated), site_off: u32, handler_off: u32,
/// frame_off: u32, _pad: u32 }`, one per protected machine call site (layout
/// pinned in `pyaot_core_defs::layout`). The runtime resolves each record to
/// an absolute return-PC at registration time and binary-searches it when a
/// raise unwinds. Returns `None` when the program has no protected sites.
fn emit_exc_table(
    module: &mut ObjectModule,
    sites: &[(FuncId, Vec<ExcSite>)],
) -> Result<Option<(DataId, u64)>> {
    use pyaot_core_defs::layout::{
        EXC_RECORD_FRAME_OFF_OFFSET, EXC_RECORD_HANDLER_OFF_OFFSET, EXC_RECORD_SITE_OFF_OFFSET,
        EXC_TABLE_RECORD_SIZE,
    };
    let count: usize = sites.iter().map(|(_, s)| s.len()).sum();
    if count == 0 {
        return Ok(None);
    }
    let rec = EXC_TABLE_RECORD_SIZE as usize;
    let mut bytes = vec![0u8; count * rec];
    {
        let mut off = 0usize;
        for (_, fsites) in sites {
            for s in fsites {
                let w = |b: &mut [u8], at: usize, v: u32| {
                    b[at..at + 4].copy_from_slice(&v.to_le_bytes())
                };
                w(&mut bytes, off + EXC_RECORD_SITE_OFF_OFFSET as usize, s.ret_off);
                w(
                    &mut bytes,
                    off + EXC_RECORD_HANDLER_OFF_OFFSET as usize,
                    s.handler_off,
                );
                w(&mut bytes, off + EXC_RECORD_FRAME_OFF_OFFSET as usize, s.frame_off);
                off += rec;
            }
        }
    }
    let data_id = module
        .declare_data("pyaot_exc_table", Linkage::Local, false, false)
        .map_err(|e| cg_error(format!("declare exc table: {e}")))?;
    let mut desc = DataDescription::new();
    desc.set_align(8);
    desc.define(bytes.into_boxed_slice());
    let mut off = 0usize;
    for (fid, fsites) in sites {
        let fref = module.declare_func_in_data(*fid, &mut desc);
        for _ in fsites {
            // func_addr at record offset 0 — a pointer-sized relocation.
            desc.write_function_addr(off as u32, fref);
            off += rec;
        }
    }
    module
        .define_data(data_id, &desc)
        .map_err(|e| cg_error(format!("define exc table: {e}")))?;
    Ok(Some((data_id, count as u64)))
}

/// Emit the traceback-table blob (real tracebacks): `count` fixed records —
/// one per compiled Python function — followed by an auxiliary area holding
/// the display-name/file strings and the per-function line entries (layout
/// pinned in `pyaot_core_defs::layout`). Offsets are relative to the blob
/// base; only `func_addr` needs a relocation.
fn emit_tb_table(
    module: &mut ObjectModule,
    funcs: &[(FuncId, String, String, FnTb)],
) -> Result<Option<(DataId, u64)>> {
    use pyaot_core_defs::layout::{
        TB_LOC_ENTRY_SIZE, TB_RECORD_CODE_SIZE_OFFSET, TB_RECORD_FILE_LEN_OFFSET,
        TB_RECORD_FILE_OFF_OFFSET, TB_RECORD_LOC_OFF_OFFSET, TB_RECORD_NAME_LEN_OFFSET,
        TB_RECORD_NAME_OFF_OFFSET, TB_RECORD_SIZE,
    };
    if funcs.is_empty() {
        return Ok(None);
    }
    let rec = TB_RECORD_SIZE as usize;
    let mut bytes = vec![0u8; funcs.len() * rec];
    let w32 = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    // Auxiliary area appended after the records; strings are interned so the
    // (typically single) file path is stored once.
    let mut aux_str: HashMap<&str, u32> = HashMap::new();
    for (i, (_, name, file, tb)) in funcs.iter().enumerate() {
        let base = i * rec;
        w32(&mut bytes, base + TB_RECORD_CODE_SIZE_OFFSET as usize, tb.code_size);
        for (s, off_at, len_at) in [
            (name, TB_RECORD_NAME_OFF_OFFSET, TB_RECORD_NAME_LEN_OFFSET),
            (file, TB_RECORD_FILE_OFF_OFFSET, TB_RECORD_FILE_LEN_OFFSET),
        ] {
            let off = if let Some(&o) = aux_str.get(s.as_str()) {
                o
            } else {
                let o = bytes.len() as u32;
                bytes.extend_from_slice(s.as_bytes());
                // SAFETY of the borrow: keys live as long as `funcs`.
                aux_str.insert(s.as_str(), o);
                o
            };
            w32(&mut bytes, base + off_at as usize, off);
            w32(&mut bytes, base + len_at as usize, s.len() as u32);
        }
        // Line area: count + (start, end, line) triples. 4-align it.
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
        let loc_off = bytes.len() as u32;
        bytes.extend_from_slice(&(tb.locs.len() as u32).to_le_bytes());
        for (start, end, line) in &tb.locs {
            bytes.extend_from_slice(&start.to_le_bytes());
            bytes.extend_from_slice(&end.to_le_bytes());
            bytes.extend_from_slice(&line.to_le_bytes());
        }
        debug_assert_eq!(TB_LOC_ENTRY_SIZE, 12);
        w32(&mut bytes, base + TB_RECORD_LOC_OFF_OFFSET as usize, loc_off);
    }
    let data_id = module
        .declare_data("pyaot_tb_table", Linkage::Local, false, false)
        .map_err(|e| cg_error(format!("declare tb table: {e}")))?;
    let mut desc = DataDescription::new();
    desc.set_align(8);
    desc.define(bytes.into_boxed_slice());
    for (i, (fid, ..)) in funcs.iter().enumerate() {
        let fref = module.declare_func_in_data(*fid, &mut desc);
        desc.write_function_addr((i * rec) as u32, fref);
    }
    module
        .define_data(data_id, &desc)
        .map_err(|e| cg_error(format!("define tb table: {e}")))?;
    Ok(Some((data_id, funcs.len() as u64)))
}

/// `main(argc, argv)` → rt_init → register unwind + traceback tables →
/// (classinit) → call `__main__` → rt_shutdown → 0.
#[allow(clippy::too_many_arguments)]
fn emit_main(
    module: &mut ObjectModule,
    entry_fn: FuncId,
    classinit: Option<FuncId>,
    exc_table: Option<(DataId, u64)>,
    tb_table: Option<(DataId, u64)>,
    rt: &RuntimeFns,
    ptr_ty: Type,
    cc: CallConv,
) -> Result<()> {
    let mut sig = Signature::new(cc);
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(ptr_ty));
    sig.returns.push(AbiParam::new(types::I32));
    let main_id = module
        .declare_function("main", Linkage::Export, &sig)
        .map_err(|e| cg_error(format!("declare main: {e}")))?;

    let mut ctx = module.make_context();
    ctx.func.signature = sig;
    let mut fctx = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);

        let argc = builder.block_params(entry)[0];
        let argv = builder.block_params(entry)[1];

        let init = module.declare_func_in_func(rt.init, builder.func);
        builder.ins().call(init, &[argc, argv]);

        // Hand the runtime the PC→handler unwind table before any user code
        // can raise.
        if let Some((data_id, count)) = exc_table {
            let gv = module.declare_data_in_func(data_id, builder.func);
            let ptr = builder.ins().global_value(ptr_ty, gv);
            let cnt = builder.ins().iconst(types::I64, count as i64);
            let reg = module.declare_func_in_func(rt.exc_register_table, builder.func);
            builder.ins().call(reg, &[ptr, cnt]);
        }
        // And the PC→(function, file, line) traceback table.
        if let Some((data_id, count)) = tb_table {
            let gv = module.declare_data_in_func(data_id, builder.func);
            let ptr = builder.ins().global_value(ptr_ty, gv);
            let cnt = builder.ins().iconst(types::I64, count as i64);
            let reg = module.declare_func_in_func(rt.tb_register_table, builder.func);
            builder.ins().call(reg, &[ptr, cnt]);
        }

        // Register all classes before running module-body code (which may build
        // instances at the top level).
        if let Some(classinit) = classinit {
            let ci = module.declare_func_in_func(classinit, builder.func);
            builder.ins().call(ci, &[]);
        }

        let entry_ref = module.declare_func_in_func(entry_fn, builder.func);
        builder.ins().call(entry_ref, &[]);

        let shutdown = module.declare_func_in_func(rt.shutdown, builder.func);
        builder.ins().call(shutdown, &[]);

        let zero = builder.ins().iconst(types::I32, 0);
        builder.ins().return_(&[zero]);
        builder.finalize();
    }
    module
        .define_function(main_id, &mut ctx)
        .map_err(|e| cg_error(format!("define main: {e}")))?;
    module.clear_context(&mut ctx);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn define_function(
    module: &mut ObjectModule,
    mf: &MirFunction,
    cl_func_id: FuncId,
    func_ids: &[FuncId],
    rt: &RuntimeFns,
    data_ids: &HashMap<InternedString, (DataId, u32)>,
    ptr_ty: Type,
    cc: CallConv,
    trampolines: &mut Trampolines,
) -> Result<(Vec<ExcSite>, FnTb)> {
    let mut sig = Signature::new(cc);
    for p in &mf.params {
        sig.params.push(abi_param(clif_ty(p)));
    }
    sig.returns.push(AbiParam::new(clif_ty(&mf.ret)));

    let mut ctx = module.make_context();
    ctx.func.signature = sig;
    let mut fctx = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fctx);

        // One Cranelift block per MIR block. Exception handlers and raise
        // paths are marked cold (Phase 9C.5) so layout moves them out of the
        // hot instruction stream.
        let cl_blocks: Vec<_> = mf.blocks.iter().map(|_| builder.create_block()).collect();
        for (i, cold) in pyaot_mir::cold_blocks(mf).into_iter().enumerate() {
            if cold {
                builder.set_cold_block(cl_blocks[i]);
            }
        }

        // Declare a Variable per MIR local. Cranelift assigns indices 0..n in
        // declaration order, so Variable index == LocalId.
        for local in &mf.locals {
            builder.declare_var(clif_ty(&local.repr));
        }

        // GC root set (PITFALLS B15): a local gets a slot in the frame roots
        // array iff its `Repr::is_gc_root()` holds (Invariant 5 — rootness
        // derives from `Repr`) AND the liveness analysis proves it live
        // across / used by a potential allocation
        // ([`pyaot_mir::roots_needed`], on this final post-optimizer MIR).
        // Rooted locals are stored on every def, so the GC sees the value for
        // the whole function; `nroots == 0` legitimately takes the leaf path
        // even in functions that allocate.
        let needed = pyaot_mir::roots_needed(mf);
        let mut root_slot_of = vec![None; mf.locals.len()];
        let mut nroots: u32 = 0;
        for (i, local) in mf.locals.iter().enumerate() {
            if local.repr.is_gc_root() && needed[i] {
                root_slot_of[i] = Some(nroots);
                nroots += 1;
            }
        }
        let (roots_slot, frame_slot) = if nroots > 0 {
            let roots = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                nroots * 8,
                3,
            ));
            let frame = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                pyaot_core_defs::layout::SHADOW_FRAME_SIZE,
                3,
            ));
            (Some(roots), Some(frame))
        } else {
            (None, None)
        };

        let entry_idx = mf.entry.index();
        builder.append_block_params_for_function_params(cl_blocks[entry_idx]);

        let mut fb = FnGen {
            module,
            builder: &mut builder,
            cl_blocks: &cl_blocks,
            func_ids,
            rt,
            data_ids,
            locals: &mf.locals,
            program_ret: clif_ty(&mf.ret),
            ptr_ty,
            cc,
            root_slot_of,
            nroots,
            roots_slot,
            frame_slot,
            cur_handler: None,
            trampolines,
        };

        for (bi, mblock) in mf.blocks.iter().enumerate() {
            fb.builder.switch_to_block(cl_blocks[bi]);
            // Calls in a protected block route their exceptional edge to the
            // handler (try_call); plain blocks compile exactly as before.
            fb.cur_handler = mblock.handler.map(|h| h.index());
            if bi == entry_idx {
                // GC frame setup must precede any rooted store (incl. params).
                fb.emit_gc_prologue();
                // Prologue: define parameter variables from block params.
                let params: Vec<Value> = fb.builder.block_params(cl_blocks[bi]).to_vec();
                for (i, pv) in params.iter().enumerate() {
                    fb.def_local(LocalId::from(i), *pv);
                }
            }
            for inst in &mblock.insts {
                fb.lower_inst(inst)?;
            }
            fb.lower_terminator(&mblock.term)?;
        }

        builder.seal_all_blocks();
        builder.finalize();
    }
    module
        .define_function(cl_func_id, &mut ctx)
        .map_err(|e| cg_error(format!("define function: {e}")))?;

    // Harvest this function's exception metadata: every emitted `try_call`
    // produced a machine call site carrying its handler's code offset. The
    // records feed the program-wide PC→handler table the runtime unwinder
    // searches at raise time.
    let mut sites = Vec::new();
    let compiled = ctx
        .compiled_code()
        .ok_or_else(|| cg_error("compiled_code missing after define_function"))?;
    for site in compiled.buffer.call_sites() {
        for h in site.exception_handlers {
            match h {
                FinalizedMachExceptionHandler::Default(off) => {
                    let frame_off = site.frame_offset.ok_or_else(|| {
                        cg_error(
                            "call site with handler but no frame_offset \
                             (preserve_frame_pointers must be on)"
                                                        )
                    })?;
                    sites.push(ExcSite {
                        ret_off: site.ret_addr,
                        handler_off: *off,
                        frame_off,
                    });
                }
                FinalizedMachExceptionHandler::Tag(..)
                | FinalizedMachExceptionHandler::Context(_) => {
                    return Err(cg_error(
                        "unexpected tagged/context exception handler (codegen only \
                         emits Default edges)"
                                                ));
                }
            }
        }
    }

    // Harvest the traceback metadata (real tracebacks): the function's code
    // size plus the line ranges the `LineMarker`-driven srclocs produced.
    let tb = FnTb {
        code_size: compiled.buffer.total_size(),
        locs: compiled
            .buffer
            .get_srclocs_sorted()
            .iter()
            .filter(|s| !s.loc.is_default())
            .map(|s| (s.start, s.end, s.loc.bits()))
            .collect(),
    };

    module.clear_context(&mut ctx);
    Ok((sites, tb))
}

/// A function's traceback metadata: machine-code size and the sorted
/// `[start, end) → source line` ranges harvested from Cranelift srclocs.
struct FnTb {
    code_size: u32,
    locs: Vec<(u32, u32, u32)>,
}

/// One protected machine call site of a compiled function: the return-address
/// offset, its handler's entry offset (both relative to the function start),
/// and the FP-to-SP distance the unwinder needs to reconstruct SP.
struct ExcSite {
    ret_off: u32,
    handler_off: u32,
    frame_off: u32,
}

/// Tail-convention trampolines for protected calls.
///
/// Cranelift treats the EXCEPTIONAL edge of a `try_call` as clobbering ALL
/// registers only when the callee uses `CallConv::Tail`; for a SystemV
/// callee the edge keeps callee-saved registers live — it assumes an
/// Itanium-style unwinder that restores callee-saves from CFI while
/// walking. Our unwinder restores SP/FP only, so every protected call must
/// present a Tail callee: a one-jump trampoline forwarding to the real
/// (SystemV) target. With the Tail callee, every value live into the
/// handler is spilled to the protected frame and reloaded there — exactly
/// what the SP/FP-only resume provides. The trampoline's own frame sits
/// between the protected frame and the raise and is skipped by the FP walk
/// like any other frame.
#[derive(Default)]
struct Trampolines {
    /// target → its Tail trampoline (direct calls).
    direct: HashMap<FuncId, FuncId>,
    /// signature key → the Tail trampoline taking `(args…, fnptr)` (indirect).
    indirect: HashMap<String, FuncId>,
    /// Declared trampolines awaiting body emission.
    pending: Vec<PendingTramp>,
}

enum PendingTramp {
    /// `tramp(args…) -> target(args…)`.
    Direct { tramp: FuncId, target: FuncId },
    /// `tramp(args…, fnptr) -> fnptr(args…)` through `sig`.
    Indirect { tramp: FuncId, sig: Signature },
}

impl Trampolines {
    fn direct(&mut self, module: &mut ObjectModule, target: FuncId, sig: &Signature) -> FuncId {
        if let Some(t) = self.direct.get(&target) {
            return *t;
        }
        let mut tsig = sig.clone();
        tsig.call_conv = CallConv::Tail;
        let name = format!("pyaot_exc_tramp_d{}", target.as_u32());
        let t = module
            .declare_function(&name, Linkage::Local, &tsig)
            .expect("trampoline declaration cannot fail: unique local symbol");
        self.direct.insert(target, t);
        self.pending.push(PendingTramp::Direct { tramp: t, target });
        t
    }

    fn indirect(&mut self, module: &mut ObjectModule, sig: &Signature) -> FuncId {
        let key = sig.to_string();
        if let Some(t) = self.indirect.get(&key) {
            return *t;
        }
        let mut tsig = sig.clone();
        tsig.call_conv = CallConv::Tail;
        tsig.params.push(AbiParam::new(types::I64)); // trailing target fnptr
        let name = format!("pyaot_exc_tramp_i{}", self.indirect.len());
        let t = module
            .declare_function(&name, Linkage::Local, &tsig)
            .expect("trampoline declaration cannot fail: unique local symbol");
        self.indirect.insert(key, t);
        self.pending.push(PendingTramp::Indirect {
            tramp: t,
            sig: sig.clone(),
        });
        t
    }
}

/// Emit the bodies of all declared trampolines (after every user function is
/// defined, so direct targets exist).
fn define_trampolines(module: &mut ObjectModule, tramps: Trampolines) -> Result<()> {
    let mut fctx = FunctionBuilderContext::new();
    for p in tramps.pending {
        let mut ctx = module.make_context();
        let tramp_id = match &p {
            PendingTramp::Direct { tramp, .. } | PendingTramp::Indirect { tramp, .. } => *tramp,
        };
        ctx.func.signature = module
            .declarations()
            .get_function_decl(tramp_id)
            .signature
            .clone();
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fctx);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);
            let params: Vec<Value> = builder.block_params(entry).to_vec();
            let results: Vec<Value> = match &p {
                PendingTramp::Direct { target, .. } => {
                    let fref = module.declare_func_in_func(*target, builder.func);
                    let call = builder.ins().call(fref, &params);
                    builder.inst_results(call).to_vec()
                }
                PendingTramp::Indirect { sig, .. } => {
                    let sigref = builder.import_signature(sig.clone());
                    let (fnptr, args) = params
                        .split_last()
                        .expect("indirect trampoline has the fnptr param");
                    let call = builder.ins().call_indirect(sigref, *fnptr, args);
                    builder.inst_results(call).to_vec()
                }
            };
            builder.ins().return_(&results);
            builder.finalize();
        }
        module
            .define_function(tramp_id, &mut ctx)
            .map_err(|e| cg_error(format!("define trampoline: {e}")))?;
        module.clear_context(&mut ctx);
    }
    Ok(())
}

/// Per-function codegen context.
struct FnGen<'a, 'b> {
    module: &'a mut ObjectModule,
    builder: &'a mut FunctionBuilder<'b>,
    cl_blocks: &'a [cranelift_codegen::ir::Block],
    func_ids: &'a [FuncId],
    rt: &'a RuntimeFns,
    data_ids: &'a HashMap<InternedString, (DataId, u32)>,
    /// The MIR function's local Repr table — drives per-operand dispatch (a
    /// `Raw(F64)`/`Raw(I64)` arithmetic operand inlines, a `Tagged` one calls
    /// `rt_obj_*`). This is the same `Repr` the verifier checked; codegen never
    /// re-derives it (Principle 6).
    locals: &'a [LocalDecl],
    program_ret: Type,
    ptr_ty: Type,
    /// The platform call convention (for `CallVirtual`'s indirect-call signature).
    cc: CallConv,
    /// Per-local GC roots-array index (`Some` iff the local is a GC root).
    root_slot_of: Vec<Option<u32>>,
    nroots: u32,
    roots_slot: Option<StackSlot>,
    frame_slot: Option<StackSlot>,
    /// The handler MIR-block index protecting the block being lowered, if
    /// any: every raising call becomes a `try_call` with this handler as its
    /// exceptional edge.
    cur_handler: Option<usize>,
    /// Program-wide Tail-trampoline registry for protected calls.
    trampolines: &'a mut Trampolines,
}

impl FnGen<'_, '_> {
    fn use_local(&mut self, id: LocalId) -> Value {
        self.builder.use_var(Variable::from_u32(id.index() as u32))
    }

    fn use_operand(&mut self, op: &Operand) -> Value {
        match op {
            Operand::Local(id) => self.use_local(*id),
        }
    }

    /// The declared representation of an operand (drives arithmetic dispatch).
    fn operand_repr(&self, op: &Operand) -> &Repr {
        match op {
            Operand::Local(id) => &self.locals[id.index()].repr,
        }
    }

    /// Define a local. If it is a GC root, mirror the value into the frame roots
    /// array (store-on-def) so the collector can find it (PITFALLS B15).
    fn def_local(&mut self, id: LocalId, val: Value) {
        self.builder
            .def_var(Variable::from_u32(id.index() as u32), val);
        if let Some(slot_idx) = self.root_slot_of[id.index()] {
            let rs = self.roots_slot.expect("rooted local needs a roots slot");
            self.builder
                .ins()
                .stack_store(val, rs, (slot_idx * 8) as i32);
        }
    }

    /// Emit the GC frame prologue: zero the roots array, fill the `ShadowFrame`,
    /// and `gc_push` it. No-op for leaf functions (`nroots == 0`).
    fn emit_gc_prologue(&mut self) {
        if self.nroots == 0 {
            return;
        }
        use pyaot_core_defs::layout::{SHADOW_FRAME_NROOTS_OFFSET, SHADOW_FRAME_ROOTS_OFFSET};
        let roots = self.roots_slot.unwrap();
        let frame = self.frame_slot.unwrap();
        let zero = self.builder.ins().iconst(types::I64, 0);
        for i in 0..self.nroots {
            self.builder.ins().stack_store(zero, roots, (i * 8) as i32);
        }
        let nroots_v = self.builder.ins().iconst(types::I64, self.nroots as i64);
        self.builder
            .ins()
            .stack_store(nroots_v, frame, SHADOW_FRAME_NROOTS_OFFSET);
        let roots_addr = self.builder.ins().stack_addr(self.ptr_ty, roots, 0);
        self.builder
            .ins()
            .stack_store(roots_addr, frame, SHADOW_FRAME_ROOTS_OFFSET);
        let frame_addr = self.builder.ins().stack_addr(self.ptr_ty, frame, 0);
        self.call(self.rt.gc_push, &[frame_addr]);
    }

    /// `gc_pop` before a return (paired with the prologue's `gc_push`).
    fn emit_gc_epilogue(&mut self) {
        if self.nroots > 0 {
            self.call(self.rt.gc_pop, &[]);
        }
    }

    /// Widen call arguments to the declared parameter types (`I8`/`I16`
    /// values into `I32` params — see [`import_param`]). A no-op for
    /// already-matching signatures.
    fn adapt_args(&mut self, params: &[AbiParam], args: &[Value]) -> Vec<Value> {
        args.iter()
            .zip(params)
            .map(|(&v, p)| {
                let have = self.builder.func.dfg.value_type(v);
                if have != p.value_type && (have == types::I8 || have == types::I16) {
                    self.builder.ins().uextend(p.value_type, v)
                } else {
                    v
                }
            })
            .collect()
    }

    /// Call a runtime/user function, returning its single result (if any).
    /// Inside a protected block, any callee that can raise is routed through
    /// a `CallConv::Tail` trampoline emitted as a `try_call` whose
    /// exceptional edge lands at the handler block (see [`Trampolines`] for
    /// why the indirection is mandatory); the never-raising helpers (and all
    /// calls in unprotected blocks) stay plain `call`s with no unwind
    /// metadata.
    fn call(&mut self, fid: FuncId, args: &[Value]) -> Option<Value> {
        let fref = self.module.declare_func_in_func(fid, self.builder.func);
        let sig_ref = self.builder.func.dfg.ext_funcs[fref].signature;
        let params = self.builder.func.dfg.signatures[sig_ref].params.clone();
        let args = self.adapt_args(&params, args);
        if let Some(h) = self.cur_handler {
            if !self.rt.never_raises.contains(&fid) {
                let sig = self.builder.func.dfg.signatures[sig_ref].clone();
                let tramp = self.trampolines.direct(self.module, fid, &sig);
                let tref = self.module.declare_func_in_func(tramp, self.builder.func);
                let tramp_sig_ref = self.builder.func.dfg.ext_funcs[tref].signature;
                return self.emit_protected_call(tramp_sig_ref, h, |b, et| {
                    b.ins().try_call(tref, &args, et)
                });
            }
        }
        let inst = self.builder.ins().call(fref, &args);
        let results = self.builder.inst_results(inst);
        results.first().copied()
    }

    /// Indirect-call counterpart of [`Self::call`] (closures, vtable
    /// methods). The protected form becomes a DIRECT `try_call` to a
    /// per-signature Tail trampoline taking the function pointer as its
    /// trailing parameter.
    fn call_indirect(&mut self, sig_ref: SigRef, callee: Value, args: &[Value]) -> Option<Value> {
        if let Some(h) = self.cur_handler {
            let sig = self.builder.func.dfg.signatures[sig_ref].clone();
            let tramp = self.trampolines.indirect(self.module, &sig);
            let tref = self.module.declare_func_in_func(tramp, self.builder.func);
            let tramp_sig_ref = self.builder.func.dfg.ext_funcs[tref].signature;
            let mut targs = args.to_vec();
            targs.push(callee);
            return self.emit_protected_call(tramp_sig_ref, h, |b, et| {
                b.ins().try_call(tref, &targs, et)
            });
        }
        let inst = self.builder.ins().call_indirect(sig_ref, callee, args);
        self.builder.inst_results(inst).first().copied()
    }

    /// Emit a `try_call` (a block terminator): the normal edge continues in a
    /// fresh block whose params are bound to the callee's returns
    /// (`BlockArg::TryCallRet`); the exceptional edge is a catch-all
    /// `Default` into the handler block (exception payloads unused — the
    /// exception object lives in runtime thread-local state). Lowering then
    /// resumes in the continuation block.
    fn emit_protected_call(
        &mut self,
        sig_ref: SigRef,
        handler_mir_block: usize,
        emit: impl FnOnce(&mut FunctionBuilder<'_>, ExceptionTable) -> cranelift_codegen::ir::Inst,
    ) -> Option<Value> {
        let ret_types: Vec<Type> = self.builder.func.dfg.signatures[sig_ref]
            .returns
            .iter()
            .map(|p| p.value_type)
            .collect();
        let cont = self.builder.create_block();
        let mut rets = Vec::with_capacity(ret_types.len());
        for ty in &ret_types {
            rets.push(self.builder.append_block_param(cont, *ty));
        }
        let handler_block = self.cl_blocks[handler_mir_block];
        let pool = &mut self.builder.func.dfg.value_lists;
        let normal = BlockCall::new(
            cont,
            (0..ret_types.len() as u32).map(BlockArg::TryCallRet),
            pool,
        );
        let exceptional = BlockCall::new(handler_block, std::iter::empty::<BlockArg>(), pool);
        let et = self.builder.func.dfg.exception_tables.push(ExceptionTableData::new(
            sig_ref,
            normal,
            [ExceptionTableItem::Default(exceptional)],
        ));
        emit(self.builder, et);
        self.builder.switch_to_block(cont);
        rets.first().copied()
    }

    /// Declare (idempotently) the import for a stdlib runtime descriptor and
    /// return its `FuncId` (Phase 8B). The Cranelift signature comes straight
    /// from the descriptor's register classes; `declare_function` with
    /// `Linkage::Import` returns the same id on repeat declarations, so no
    /// separate cache is needed.
    fn runtime_fn(&mut self, def: &'static pyaot_core_defs::RuntimeFuncDef) -> Result<FuncId> {
        use pyaot_core_defs::runtime_func_def::{ParamType, ReturnType};
        let pt = |p: &ParamType| match p {
            ParamType::I64 => types::I64,
            ParamType::F64 => types::F64,
            ParamType::I8 => types::I8,
            ParamType::I32 => types::I32,
        };
        let params: Vec<Type> = def.params.iter().map(pt).collect();
        let returns: Vec<Type> = match def.returns {
            Some(ReturnType::I64) => vec![types::I64],
            Some(ReturnType::F64) => vec![types::F64],
            Some(ReturnType::I8) => vec![types::I8],
            Some(ReturnType::I32) => vec![types::I32],
            None => vec![],
        };
        declare_import(self.module, self.cc, def.symbol, &params, &returns)
    }

    fn lower_inst(&mut self, inst: &MirInst) -> Result<()> {
        match inst {
            // Real tracebacks: stamp subsequent instructions with this source
            // line; `get_srclocs_sorted()` hands the PC ranges back after
            // compilation for the traceback table.
            MirInst::LineMarker(line) => {
                self.builder
                    .set_srcloc(cranelift_codegen::ir::SourceLoc::new(*line));
                Ok(())
            }
            MirInst::Const { dst, val } => self.lower_const(*dst, val),
            MirInst::Coerce(c) => {
                self.lower_coerce(c.dst(), c.src(), c.from(), c.to(), c.checked())
            }
            MirInst::BinOp { dst, op, l, r } => self.lower_binop(*dst, *op, l, r),
            MirInst::Unary { dst, op, operand } => self.lower_unary(*dst, *op, operand),
            MirInst::Compare { dst, op, l, r } => self.lower_compare(*dst, *op, l, r),
            MirInst::Truthy { dst, operand } => {
                let v = self.use_operand(operand);
                let r = self.call(self.rt.is_truthy, &[v]).unwrap();
                self.def_local(*dst, r);
                Ok(())
            }
            MirInst::Call { dst, func, args } => {
                let vals: Vec<Value> = args.iter().map(|a| self.use_operand(a)).collect();
                let fid = self.func_ids[func.index()];
                let res = self.call(fid, &vals);
                if let (Some(d), Some(v)) = (dst, res) {
                    self.def_local(*d, v);
                }
                Ok(())
            }
            MirInst::CallBuiltin { dst, kind, args } => {
                let vals: Vec<Value> = args.iter().map(|a| self.use_operand(a)).collect();
                let fid = self.builtin_fn(*kind)?;
                let res = self.call(fid, &vals);
                if let (Some(d), Some(v)) = (dst, res) {
                    self.def_local(*d, v);
                }
                Ok(())
            }
            MirInst::CallContainer { dst, op, args } => self.lower_call_container(dst, *op, args),
            MirInst::CallRuntime { dst, def, args } => {
                let vals: Vec<Value> = args.iter().map(|a| self.use_operand(a)).collect();
                let fid = self.runtime_fn(def)?;
                let res = self.call(fid, &vals);
                if let (Some(d), Some(v)) = (dst, res) {
                    self.def_local(*d, v);
                }
                Ok(())
            }
            MirInst::MakeInstance {
                dst,
                class_id,
                field_count,
            } => {
                let cid = self.builder.ins().iconst(types::I8, class_id.0 as i64);
                let fc = self.builder.ins().iconst(types::I64, *field_count);
                let v = self.call(self.rt.make_instance, &[cid, fc]).unwrap();
                self.def_local(*dst, v);
                Ok(())
            }
            MirInst::GetField { dst, base, slot } => {
                let b = self.use_operand(base);
                let slot_v = self.builder.ins().iconst(types::I64, *slot as i64);
                let v = self.call(self.rt.instance_get_field, &[b, slot_v]).unwrap();
                self.def_local(*dst, v);
                Ok(())
            }
            MirInst::SetField { base, slot, value } => {
                let b = self.use_operand(base);
                let slot_v = self.builder.ins().iconst(types::I64, *slot as i64);
                let v = self.use_operand(value);
                self.call(self.rt.instance_set_field, &[b, slot_v, v]);
                Ok(())
            }
            MirInst::GetFieldNamed {
                dst,
                base,
                name_hash,
            } => {
                let b = self.use_operand(base);
                let h = self.builder.ins().iconst(types::I64, *name_hash as i64);
                let v = self.call(self.rt.getattr_name, &[b, h]).unwrap();
                self.def_local(*dst, v);
                Ok(())
            }
            MirInst::SetFieldNamed {
                base,
                name_hash,
                value,
            } => {
                let b = self.use_operand(base);
                let h = self.builder.ins().iconst(types::I64, *name_hash as i64);
                let v = self.use_operand(value);
                self.call(self.rt.setattr_name, &[b, h, v]);
                Ok(())
            }
            MirInst::CallVirtual {
                dst,
                recv,
                name_hash,
                args,
                ret,
            } => self.lower_call_virtual(dst, recv, *name_hash, args, ret),
            MirInst::IsInstance {
                dst,
                value,
                class_id,
            } => {
                let v = self.use_operand(value);
                let cid = self.builder.ins().iconst(types::I64, class_id.0 as i64);
                let r = self.call(self.rt.isinstance_inherited, &[v, cid]).unwrap();
                self.def_local(*dst, r);
                Ok(())
            }
            MirInst::GetClassAttr {
                dst,
                class_id,
                attr_idx,
            } => {
                let cid = self.builder.ins().iconst(types::I8, class_id.0 as i64);
                let idx = self.builder.ins().iconst(types::I32, *attr_idx as i64);
                let v = self.call(self.rt.class_attr_get_ptr, &[cid, idx]).unwrap();
                self.def_local(*dst, v);
                Ok(())
            }
            MirInst::SetClassAttr {
                class_id,
                attr_idx,
                value,
            } => {
                let cid = self.builder.ins().iconst(types::I8, class_id.0 as i64);
                let idx = self.builder.ins().iconst(types::I32, *attr_idx as i64);
                let v = self.use_operand(value);
                self.call(self.rt.class_attr_set_ptr, &[cid, idx, v]);
                Ok(())
            }
            MirInst::AssertFail => {
                let null = self.builder.ins().iconst(self.ptr_ty, 0);
                self.call(self.rt.assert_fail, &[null]);
                Ok(())
            }
            MirInst::Print { kind, arg } => self.lower_print(*kind, arg),
            // ── closures / cells / globals (Phase 6) ──
            MirInst::MakeClosure {
                dst,
                func,
                captures,
            } => self.lower_make_closure(*dst, *func, captures),
            MirInst::CallIndirect {
                dst,
                callee,
                args,
                sig,
            } => self.lower_call_indirect(dst, callee, args, sig),
            MirInst::MakeCell { dst, init } => {
                let iv = self.use_operand(init);
                let v = self.call(self.rt.make_cell_ptr, &[iv]).unwrap();
                self.def_local(*dst, v);
                Ok(())
            }
            MirInst::CellGet { dst, cell } => {
                let c = self.use_operand(cell);
                let v = self.call(self.rt.cell_get_ptr, &[c]).unwrap();
                self.def_local(*dst, v);
                Ok(())
            }
            MirInst::CellSet { cell, value } => {
                let c = self.use_operand(cell);
                let v = self.use_operand(value);
                self.call(self.rt.cell_set_ptr, &[c, v]);
                Ok(())
            }
            MirInst::GlobalGet { dst, var_id } => {
                let id = self.builder.ins().iconst(types::I32, *var_id as i64);
                let v = self.call(self.rt.global_get_ptr, &[id]).unwrap();
                self.def_local(*dst, v);
                Ok(())
            }
            MirInst::GlobalSet { var_id, value } => {
                let id = self.builder.ins().iconst(types::I32, *var_id as i64);
                let v = self.use_operand(value);
                self.call(self.rt.global_set_ptr, &[id, v]);
                Ok(())
            }
            // ── generators (Phase 6E) ──
            MirInst::MakeGenerator {
                dst,
                gen_id,
                num_locals,
            } => {
                let gid = self.builder.ins().iconst(types::I32, *gen_id as i64);
                let nl = self.builder.ins().iconst(types::I32, *num_locals as i64);
                let v = self.call(self.rt.make_generator, &[gid, nl]).unwrap();
                self.def_local(*dst, v);
                Ok(())
            }
            MirInst::GenOpInst {
                dst,
                op,
                gen,
                imm,
                value,
            } => self.lower_gen_op(dst, *op, gen, *imm, value),
            // ── exceptions (Phase 7) ──
            MirInst::ExcOp(op) => {
                let fid = match op {
                    pyaot_mir::ExcOp::StartHandling => self.rt.exc_start_handling,
                    pyaot_mir::ExcOp::EndHandling => self.rt.exc_end_handling,
                };
                self.call(fid, &[]);
                Ok(())
            }
            MirInst::ExcQuery { dst, query } => {
                let v = match query {
                    pyaot_mir::ExcQuery::Current => {
                        self.call(self.rt.exc_get_current, &[]).unwrap()
                    }
                    pyaot_mir::ExcQuery::MatchesBuiltin(tag) => {
                        // Builtin tags ARE runtime class ids, so the registry
                        // walk covers both a raised builtin AND a user subclass
                        // (`class LimitError(ValueError)` caught by
                        // `except ValueError`) — `rt_exc_isinstance` would only
                        // exact-match builtins.
                        let t = self.builder.ins().iconst(types::I8, *tag as i64);
                        self.call(self.rt.exc_isinstance_class, &[t]).unwrap()
                    }
                    pyaot_mir::ExcQuery::MatchesClass(cid) => {
                        let c = self.builder.ins().iconst(types::I8, cid.0 as i64);
                        self.call(self.rt.exc_isinstance_class, &[c]).unwrap()
                    }
                };
                self.def_local(*dst, v);
                Ok(())
            }
            MirInst::ExcInstanceStr { dst, value } => {
                let v = self.use_operand(value);
                let s = self.call(self.rt.exc_instance_str, &[v]).unwrap();
                self.def_local(*dst, s);
                Ok(())
            }
            MirInst::Raise(r) => self.lower_raise(r),
        }
    }

    /// Read a raise message operand's bytes: `(rt_str_data(v), rt_str_len(v))`,
    /// or `(null, 0)` for a message-less raise. The runtime copies the bytes
    /// before any unwinding (B2), and the StrObj itself is a rooted MIR temp.
    fn msg_ptr_len(&mut self, msg: &Option<Operand>) -> (Value, Value) {
        match msg {
            Some(op) => {
                let v = self.use_operand(op);
                let ptr = self.call(self.rt.str_data, &[v]).unwrap();
                let len = self.call(self.rt.str_len, &[v]).unwrap();
                (ptr, len)
            }
            None => {
                let ptr = self.builder.ins().iconst(self.ptr_ty, 0);
                let len = self.builder.ins().iconst(types::I64, 0);
                (ptr, len)
            }
        }
    }

    /// Lower a `Raise` (Phase 7). The runtime call never returns; the block's
    /// `Unreachable` terminator traps right after (dead).
    fn lower_raise(&mut self, r: &pyaot_mir::MirRaise) -> Result<()> {
        use pyaot_mir::MirRaise as R;
        match r {
            R::Builtin { tag, msg } => {
                let (ptr, len) = self.msg_ptr_len(msg);
                let t = self.builder.ins().iconst(types::I8, *tag as i64);
                self.call(self.rt.exc_raise, &[t, ptr, len]);
            }
            R::BuiltinFromNone { tag, msg } => {
                let (ptr, len) = self.msg_ptr_len(msg);
                let t = self.builder.ins().iconst(types::I8, *tag as i64);
                self.call(self.rt.exc_raise_from_none, &[t, ptr, len]);
            }
            R::BuiltinFrom {
                tag,
                msg,
                cause_tag,
                cause_msg,
            } => {
                let (ptr, len) = self.msg_ptr_len(msg);
                let (cptr, clen) = self.msg_ptr_len(cause_msg);
                let t = self.builder.ins().iconst(types::I8, *tag as i64);
                let ct = self.builder.ins().iconst(types::I8, *cause_tag as i64);
                self.call(self.rt.exc_raise_from, &[t, ptr, len, ct, cptr, clen]);
            }
            R::CustomWithInstance {
                class_id,
                msg,
                instance,
            } => {
                let (ptr, len) = self.msg_ptr_len(msg);
                let cid = self.builder.ins().iconst(types::I8, class_id.0 as i64);
                let inst = self.use_operand(instance);
                self.call(
                    self.rt.exc_raise_custom_with_instance,
                    &[cid, ptr, len, inst],
                );
            }
            R::Stdlib {
                class_id,
                exc_type_tag,
                msg,
            } => {
                let (ptr, len) = self.msg_ptr_len(msg);
                let t = self.builder.ins().iconst(types::I8, *exc_type_tag as i64);
                let cid = self.builder.ins().iconst(types::I8, *class_id as i64);
                self.call(self.rt.exc_raise_stdlib, &[t, cid, ptr, len]);
            }
            R::Instance { value } => {
                let v = self.use_operand(value);
                self.call(self.rt.exc_raise_instance, &[v]);
            }
            R::Reraise => {
                self.call(self.rt.exc_reraise, &[]);
            }
        }
        Ok(())
    }

    /// Lower a generator state-machine op (Phase 6E) to its runtime call. Slot /
    /// state immediates are `u32`; `GetState` zero-extends the `u32` result to
    /// the `Raw(I64)` the verifier expects.
    fn lower_gen_op(
        &mut self,
        dst: &Option<LocalId>,
        op: GenOp,
        gen: &Operand,
        imm: u32,
        value: &Option<Operand>,
    ) -> Result<()> {
        let g = self.use_operand(gen);
        let imm_v = self.builder.ins().iconst(types::I32, imm as i64);
        match op {
            GenOp::GetLocal => {
                let v = self.call(self.rt.gen_get_local, &[g, imm_v]).unwrap();
                self.def_local(dst.unwrap(), v);
            }
            GenOp::SetLocal => {
                let val = self.use_operand(value.as_ref().unwrap());
                self.call(self.rt.gen_set_local, &[g, imm_v, val]);
            }
            GenOp::GetState => {
                let s = self.call(self.rt.gen_get_state, &[g]).unwrap();
                let wide = self.builder.ins().uextend(types::I64, s);
                self.def_local(dst.unwrap(), wide);
            }
            GenOp::SetState => {
                self.call(self.rt.gen_set_state, &[g, imm_v]);
            }
            GenOp::GetSentValue => {
                let v = self.call(self.rt.gen_get_sent_value, &[g]).unwrap();
                self.def_local(dst.unwrap(), v);
            }
            GenOp::SetExhausted => {
                self.call(self.rt.gen_set_exhausted, &[g]);
            }
            GenOp::IsClosing => {
                let v = self.call(self.rt.gen_is_closing, &[g]).unwrap();
                self.def_local(dst.unwrap(), v);
            }
            GenOp::Next => {
                // `next(x)` raises StopIteration on exhaustion (CPython
                // semantics) — route through the raising `rt_iter_next`, NOT
                // `rt_generator_next` (which returns the bare resume result and
                // would silently surface `None` past the end). The for-loop
                // path stays on the non-raising `rt_iter_next_no_exc`.
                let v = self.call(self.rt.iter_next, &[g]).unwrap();
                self.def_local(dst.unwrap(), v);
            }
            GenOp::Send => {
                let val = self.use_operand(value.as_ref().unwrap());
                let v = self.call(self.rt.gen_send, &[g, val]).unwrap();
                self.def_local(dst.unwrap(), v);
            }
            GenOp::Close => {
                self.call(self.rt.gen_close, &[g]);
            }
        }
        Ok(())
    }

    /// Lower `MakeClosure` (Phase 6A): a `Closure`-tagged runtime tuple of `1+N`
    /// slots (same `TupleObj` layout as a plain tuple, distinct tag so a data
    /// `tuple` is never mistaken for a callable — `rt_call_check`). Slot 0 holds the
    /// target's code address **int-tagged** (`(addr << 3) | 1`) so the GC's `is_ptr`
    /// check skips it when tracing slots; slots `1..=N` hold the captured cells
    /// (tagged Values, traced normally).
    fn lower_make_closure(
        &mut self,
        dst: LocalId,
        func: pyaot_utils::FuncId,
        captures: &[Operand],
    ) -> Result<()> {
        let count = self
            .builder
            .ins()
            .iconst(types::I64, 1 + captures.len() as i64);
        let env = self.call(self.rt.make_closure, &[count]).unwrap();
        // Root the env tuple immediately: the capture stores below call into the
        // runtime, and a later allocation must not collect it.
        self.def_local(dst, env);

        let fref = self
            .module
            .declare_func_in_func(self.func_ids[func.index()], self.builder.func);
        let addr = self.builder.ins().func_addr(self.ptr_ty, fref);
        let shifted = self.builder.ins().ishl_imm(addr, tag::INT_SHIFT as i64);
        let tagged_addr = self.builder.ins().bor_imm(shifted, tag::INT_TAG as i64);
        let slot0 = self.builder.ins().iconst(types::I64, 0);
        self.call(self.rt.tuple_set, &[env, slot0, tagged_addr]);

        for (i, cap) in captures.iter().enumerate() {
            let idx = self.builder.ins().iconst(types::I64, i as i64 + 1);
            let v = self.use_operand(cap);
            self.call(self.rt.tuple_set, &[env, idx, v]);
        }
        Ok(())
    }

    /// Lower `CallIndirect` (Phase 6A): read slot 0 of the env tuple, untag the
    /// code address, and `call_indirect` with the env tuple itself as arg 0.
    /// The Cranelift signature is `(I64 env, clif_ty(params)…) → clif_ty(ret)` —
    /// a pure function of the carried `SigRepr` (Invariant 3 / PITFALLS A4).
    fn lower_call_indirect(
        &mut self,
        dst: &Option<LocalId>,
        callee: &Operand,
        args: &[Operand],
        sig: &pyaot_types::SigRepr,
    ) -> Result<()> {
        // Guard the callee: a non-callable value (an immediate or non-tuple heap
        // object) raises `TypeError` instead of mis-reading slot 0. Returns the
        // (verified) env tuple.
        let raw_callee = self.use_operand(callee);
        let env = self.call(self.rt.call_check, &[raw_callee]).unwrap();
        let slot0 = self.builder.ins().iconst(types::I64, 0);
        let tagged_addr = self.call(self.rt.tuple_get, &[env, slot0]).unwrap();
        let fnaddr = self
            .builder
            .ins()
            .sshr_imm(tagged_addr, tag::INT_SHIFT as i64);

        let mut csig = Signature::new(self.cc);
        csig.params.push(AbiParam::new(types::I64)); // env tuple
        for p in &sig.params {
            csig.params.push(abi_param(clif_ty(p)));
        }
        csig.returns.push(AbiParam::new(clif_ty(&sig.ret)));
        let sigref = self.builder.import_signature(csig);

        let mut call_args = Vec::with_capacity(args.len() + 1);
        call_args.push(env);
        for a in args {
            call_args.push(self.use_operand(a));
        }
        let res = self.call_indirect(sigref, fnaddr, &call_args);
        if let (Some(d), Some(v)) = (dst, res) {
            self.def_local(*d, v);
        }
        Ok(())
    }

    fn lower_const(&mut self, dst: LocalId, val: &Const) -> Result<()> {
        let v = match val {
            Const::Int(i) => {
                // A raw-repr destination takes the plain integer in its register
                // class (Phase 8B descriptor-ABI immediates — field indexes, arg
                // counts); a Tagged one takes the int-tagged Value bits.
                if let Repr::Raw(_) = &self.locals[dst.index()].repr {
                    let ty = clif_ty(&self.locals[dst.index()].repr);
                    self.builder.ins().iconst(ty, *i)
                } else {
                    let tagged = ((*i) << tag::INT_SHIFT) | (tag::INT_TAG as i64);
                    self.builder.ins().iconst(types::I64, tagged)
                }
            }
            // The null-pointer `Value` — raw bits 0 (pointer tag, null payload):
            // the stdlib "absent optional object" sentinel (Phase 8B).
            Const::NullPtr => self.builder.ins().iconst(types::I64, 0),
            Const::Bool(b) => {
                let tagged = if *b {
                    ((1i64) << tag::BOOL_SHIFT) | (tag::BOOL_TAG as i64)
                } else {
                    tag::BOOL_TAG as i64
                };
                self.builder.ins().iconst(types::I64, tagged)
            }
            Const::None => self.builder.ins().iconst(types::I64, tag::NONE_TAG as i64),
            // The `Value::UNBOUND` sentinel (`RESERVED_TAG` immediate).
            Const::Unbound => self.builder.ins().iconst(types::I64, tag::RESERVED_TAG as i64),
            Const::Float(f) => self.builder.ins().f64const(*f),
            Const::Str(id) => {
                let (ptr, len) = self.str_data(*id)?;
                self.call(self.rt.make_str, &[ptr, len]).unwrap()
            }
            Const::BigIntStr(id) => {
                let (ptr, len) = self.str_data(*id)?;
                self.call(self.rt.bigint_from_str, &[ptr, len]).unwrap()
            }
            Const::Bytes(id) => {
                let (ptr, len) = self.str_data(*id)?;
                self.call(self.rt.make_bytes, &[ptr, len]).unwrap()
            }
        };
        self.def_local(dst, v);
        Ok(())
    }

    /// Materialize a string-pool data object's pointer + byte length.
    fn str_data(&mut self, id: InternedString) -> Result<(Value, Value)> {
        let (data_id, len) = *self
            .data_ids
            .get(&id)
            .ok_or_else(|| cg_error("missing data object for interned string"))?;
        let gv = self.module.declare_data_in_func(data_id, self.builder.func);
        let ptr = self.builder.ins().global_value(self.ptr_ty, gv);
        let len_val = self.builder.ins().iconst(types::I64, len as i64);
        Ok((ptr, len_val))
    }

    fn lower_coerce(
        &mut self,
        dst: LocalId,
        src: &Operand,
        from: &Repr,
        to: &Repr,
        checked: bool,
    ) -> Result<()> {
        // A checked unbox (Phase 8H, D3) validates the tag at runtime —
        // `rt_unbox_float` / `rt_unbox_int` raise TypeError on mismatch.
        if checked {
            let s = self.use_operand(src);
            let v = match to {
                Repr::Raw(RawKind::F64) => self.call(self.rt.unbox_float_checked, &[s]).unwrap(),
                Repr::Raw(RawKind::I64) => self.call(self.rt.unbox_int_checked, &[s]).unwrap(),
                Repr::Raw(RawKind::I8) => self.call(self.rt.unbox_bool_checked, &[s]).unwrap(),
                other => return Err(cg_error(format!("illegal checked coercion to {other:?}"))),
            };
            self.def_local(dst, v);
            return Ok(());
        }
        let kind = classify_coercion(from, to)
            .ok_or_else(|| cg_error(format!("illegal coercion {from:?} -> {to:?}")))?;
        let s = self.use_operand(src);
        let v = match kind {
            Coercion::Noop | Coercion::HeapToTagged | Coercion::TaggedToHeap => s,
            Coercion::BoxFloat => self.call(self.rt.box_float, &[s]).unwrap(),
            Coercion::UnboxFloat => {
                self.builder
                    .ins()
                    .load(types::F64, MemFlags::trusted(), s, FLOAT_VALUE_OFFSET)
            }
            Coercion::TagInt => {
                let shifted = self.builder.ins().ishl_imm(s, tag::INT_SHIFT as i64);
                self.builder.ins().bor_imm(shifted, tag::INT_TAG as i64)
            }
            Coercion::UntagInt => self.builder.ins().sshr_imm(s, tag::INT_SHIFT as i64),
            Coercion::TagBool => {
                let wide = self.builder.ins().uextend(types::I64, s);
                let shifted = self.builder.ins().ishl_imm(wide, tag::BOOL_SHIFT as i64);
                self.builder.ins().bor_imm(shifted, tag::BOOL_TAG as i64)
            }
            Coercion::UntagBool => {
                let shifted = self.builder.ins().ushr_imm(s, tag::BOOL_SHIFT as i64);
                let bit = self.builder.ins().band_imm(shifted, 1);
                self.builder.ins().ireduce(types::I8, bit)
            }
        };
        self.def_local(dst, v);
        Ok(())
    }

    fn lower_binop(&mut self, dst: LocalId, op: BinOp, l: &Operand, r: &Operand) -> Result<()> {
        let lrepr = self.operand_repr(l).clone();
        let a = self.use_operand(l);
        let b = self.use_operand(r);
        // The verifier guarantees both operands and `dst` share `lrepr`, and that
        // a `Raw` operand carries only the ops its kind supports (`Raw(F64)`:
        // `Add`/`Sub`/`Mul`; `Raw(I64)`: those plus `Mod`/`FloorDiv`). Dispatch on
        // it: `Raw(F64)` inlines IEEE float arithmetic (no box, no call);
        // `Raw(I64)` inlines machine integer arithmetic; `Tagged` calls the
        // tag-dispatched, bignum-safe `rt_obj_*` shims.
        let v = match (&lrepr, op) {
            (Repr::Raw(RawKind::F64), BinOp::Add) => self.builder.ins().fadd(a, b),
            (Repr::Raw(RawKind::F64), BinOp::Sub) => self.builder.ins().fsub(a, b),
            (Repr::Raw(RawKind::F64), BinOp::Mul) => self.builder.ins().fmul(a, b),
            // Raw i64 (range-proven cursors): checked machine arithmetic that
            // raises on i64 overflow — sound only because lowering proved range.
            (Repr::Raw(RawKind::I64), BinOp::Add) => self.call(self.rt.add_int, &[a, b]).unwrap(),
            (Repr::Raw(RawKind::I64), BinOp::Sub) => self.call(self.rt.sub_int, &[a, b]).unwrap(),
            (Repr::Raw(RawKind::I64), BinOp::Mul) => self.call(self.rt.mul_int, &[a, b]).unwrap(),
            // Raw i64 `% //` (Phase 3c): inline `srem`/`sdiv` with the Python
            // floor-toward-−∞ correction. typeck's interval pass proved the
            // divisor statically positive (hence nonzero and never `INT_MIN/-1`),
            // so the machine ops cannot trap and no zero-check branch is needed.
            (Repr::Raw(RawKind::I64), BinOp::Mod) => self.raw_floor_mod(a, b),
            (Repr::Raw(RawKind::I64), BinOp::FloorDiv) => self.raw_floor_div(a, b),
            (_, BinOp::Add) => self.call(self.rt.obj_add, &[a, b]).unwrap(),
            (_, BinOp::Sub) => self.call(self.rt.obj_sub, &[a, b]).unwrap(),
            (_, BinOp::Mul) => self.call(self.rt.obj_mul, &[a, b]).unwrap(),
            // `@` (PEP 465) is always Tagged → the runtime dispatches the
            // `__matmul__`/`__rmatmul__` dunder (no built-in numeric `@`).
            (_, BinOp::MatMul) => self.call(self.rt.obj_matmul, &[a, b]).unwrap(),
            (_, BinOp::Div) => self.call(self.rt.obj_div, &[a, b]).unwrap(),
            (_, BinOp::FloorDiv) => self.call(self.rt.obj_floordiv, &[a, b]).unwrap(),
            (_, BinOp::Mod) => self.call(self.rt.obj_mod, &[a, b]).unwrap(),
            (_, BinOp::Pow) => self.call(self.rt.obj_pow, &[a, b]).unwrap(),
            // Bitwise/shift dispatch on the tag in the runtime (bignum-safe);
            // operands are Tagged, never raw-unboxed (Invariant 2).
            (_, BinOp::BitAnd) => self.call(self.rt.obj_bitand, &[a, b]).unwrap(),
            (_, BinOp::BitOr) => self.call(self.rt.obj_bitor, &[a, b]).unwrap(),
            // `|=` — in-place merge for `dict`/`set` (returns the same object),
            // numeric/TypeError delegated to `rt_obj_bitor` inside the runtime.
            (_, BinOp::IOr) => self.call(self.rt.obj_ior, &[a, b]).unwrap(),
            (_, BinOp::BitXor) => self.call(self.rt.obj_bitxor, &[a, b]).unwrap(),
            (_, BinOp::Shl) => self.call(self.rt.obj_lshift, &[a, b]).unwrap(),
            (_, BinOp::Shr) => self.call(self.rt.obj_rshift, &[a, b]).unwrap(),
        };
        self.def_local(dst, v);
        Ok(())
    }

    /// Branchless Python floor modulo on raw i64 (`a % b`, result takes the sign
    /// of the divisor). `srem` truncates toward zero; when the remainder is
    /// nonzero and its sign differs from the divisor's, add the divisor to floor
    /// toward −∞. The divisor is proven statically positive by typeck, so `srem`
    /// never traps.
    fn raw_floor_mod(&mut self, a: Value, b: Value) -> Value {
        let zero = self.builder.ins().iconst(types::I64, 0);
        let r = self.builder.ins().srem(a, b);
        let r_ne = self.builder.ins().icmp(IntCC::NotEqual, r, zero);
        let xor = self.builder.ins().bxor(r, b);
        let diff_sign = self.builder.ins().icmp(IntCC::SignedLessThan, xor, zero);
        let adjust = self.builder.ins().band(r_ne, diff_sign);
        let addend = self.builder.ins().select(adjust, b, zero);
        self.builder.ins().iadd(r, addend)
    }

    /// Branchless Python floor division on raw i64 (`a // b`, rounds toward −∞).
    /// `sdiv` truncates toward zero; subtract 1 when the (truncated) remainder is
    /// nonzero and its sign differs from the divisor's. Divisor proven positive,
    /// so `sdiv`/`srem` never trap.
    fn raw_floor_div(&mut self, a: Value, b: Value) -> Value {
        let zero = self.builder.ins().iconst(types::I64, 0);
        let q = self.builder.ins().sdiv(a, b);
        let r = self.builder.ins().srem(a, b);
        let r_ne = self.builder.ins().icmp(IntCC::NotEqual, r, zero);
        let xor = self.builder.ins().bxor(r, b);
        let diff_sign = self.builder.ins().icmp(IntCC::SignedLessThan, xor, zero);
        let adjust = self.builder.ins().band(r_ne, diff_sign);
        let one = self.builder.ins().iconst(types::I64, 1);
        let dec = self.builder.ins().select(adjust, one, zero);
        self.builder.ins().isub(q, dec)
    }

    fn lower_unary(&mut self, dst: LocalId, op: UnaryOp, operand: &Operand) -> Result<()> {
        let a = self.use_operand(operand);
        let v = match op {
            UnaryOp::Neg => self.call(self.rt.obj_neg, &[a]).unwrap(),
            UnaryOp::Pos => self.call(self.rt.obj_pos, &[a]).unwrap(),
            UnaryOp::Invert => self.call(self.rt.obj_invert, &[a]).unwrap(),
            UnaryOp::Not => {
                // `not x` = logical-negate truthiness → Raw(I8).
                let t = self.call(self.rt.is_truthy, &[a]).unwrap();
                self.builder.ins().bxor_imm(t, 1)
            }
        };
        self.def_local(dst, v);
        Ok(())
    }

    fn lower_compare(&mut self, dst: LocalId, op: CmpOp, l: &Operand, r: &Operand) -> Result<()> {
        let lrepr = self.operand_repr(l).clone();
        let a = self.use_operand(l);
        let b = self.use_operand(r);
        // Raw i64 (range-proven cursors): a signed machine `icmp` yielding the
        // `I8` boolean directly — no boxing, no `rt_obj_*` call. Bounded fixnums
        // compare identically to Python ints.
        if lrepr == Repr::Raw(RawKind::I64) {
            let cc = match op {
                CmpOp::Eq => IntCC::Equal,
                CmpOp::NotEq => IntCC::NotEqual,
                CmpOp::Lt => IntCC::SignedLessThan,
                CmpOp::LtE => IntCC::SignedLessThanOrEqual,
                CmpOp::Gt => IntCC::SignedGreaterThan,
                CmpOp::GtE => IntCC::SignedGreaterThanOrEqual,
            };
            let v = self.builder.ins().icmp(cc, a, b);
            self.def_local(dst, v);
            return Ok(());
        }
        let v = match op {
            CmpOp::Eq => self.call(self.rt.obj_eq, &[a, b]).unwrap(),
            CmpOp::NotEq => {
                let eq = self.call(self.rt.obj_eq, &[a, b]).unwrap();
                self.builder.ins().bxor_imm(eq, 1)
            }
            CmpOp::Lt | CmpOp::LtE | CmpOp::Gt | CmpOp::GtE => {
                let op_tag = match op {
                    CmpOp::Lt => 0i64,
                    CmpOp::LtE => 1,
                    CmpOp::Gt => 2,
                    CmpOp::GtE => 3,
                    _ => unreachable!(),
                };
                let tag_v = self.builder.ins().iconst(types::I8, op_tag);
                self.call(self.rt.obj_cmp, &[a, b, tag_v]).unwrap()
            }
        };
        self.def_local(dst, v);
        Ok(())
    }

    /// Lower a `CallContainer`: select the runtime function from the op and emit
    /// the call. `ListCmp`/`TupleCmp` append the runtime `op_tag` immediate. The
    /// `Value`-typed return is stored directly into `dst` (heap pointers are
    /// bit-identical to tagged values).
    fn lower_call_container(
        &mut self,
        dst: &Option<LocalId>,
        op: ContainerOp,
        args: &[Operand],
    ) -> Result<()> {
        let mut vals: Vec<Value> = args.iter().map(|a| self.use_operand(a)).collect();
        let fid = match op {
            ContainerOp::ListNew => self.rt.make_list,
            ContainerOp::DictNew => self.rt.make_dict,
            ContainerOp::SetNew => self.rt.make_set,
            ContainerOp::TupleNew => self.rt.make_tuple,
            ContainerOp::ListPush => self.rt.list_push,
            ContainerOp::ListSet => self.rt.list_set,
            ContainerOp::DictSet => self.rt.dict_set,
            ContainerOp::SetAdd => self.rt.set_add,
            ContainerOp::TupleSet => self.rt.tuple_set,
            ContainerOp::ListGet => self.rt.list_get,
            ContainerOp::DictGet => self.rt.dict_get,
            ContainerOp::TupleGet => self.rt.tuple_get,
            ContainerOp::BytesGet => self.rt.bytes_get,
            ContainerOp::StrGet => self.rt.str_get,
            ContainerOp::AnyGetItem => self.rt.any_getitem,
            ContainerOp::Len => self.rt.obj_len,
            ContainerOp::Contains => self.rt.obj_contains,
            ContainerOp::ListConcat => self.rt.list_concat,
            ContainerOp::ListRepeat => self.rt.list_repeat,
            ContainerOp::TupleConcat => self.rt.tuple_concat,
            ContainerOp::BytesConcat => self.rt.bytes_concat,
            ContainerOp::BytesRepeat => self.rt.bytes_repeat,
            ContainerOp::ListCmp(c) => {
                let tag = self.builder.ins().iconst(types::I8, cmp_op_tag(c));
                vals.push(tag);
                self.rt.list_cmp
            }
            ContainerOp::TupleCmp(c) => {
                let tag = self.builder.ins().iconst(types::I8, cmp_op_tag(c));
                vals.push(tag);
                self.rt.tuple_cmp
            }
            ContainerOp::Iter => self.rt.iter_value,
            ContainerOp::IterNext => self.rt.iter_next_no_exc,
            ContainerOp::IterExhausted => self.rt.iter_is_exhausted,
            ContainerOp::Enumerate => self.rt.iter_enumerate,
            ContainerOp::Zip => self.rt.zip_new,
            ContainerOp::ZipN => self.rt.zipn_new,
            ContainerOp::ListFromIter => self.rt.list_from_iter,
            ContainerOp::TupleFromIter => self.rt.tuple_from_iter,
            ContainerOp::DictFromPairs => self.rt.dict_from_pairs,
            ContainerOp::BytesFromList => self.rt.make_bytes_from_list,
            ContainerOp::BytesZero => self.rt.make_bytes_zero,
            ContainerOp::BytesFromStr => self.rt.make_bytes_from_str,
            ContainerOp::Reversed => self.rt.iter_reversed_list,
            ContainerOp::RangeIter => self.rt.iter_range,
            ContainerOp::Sorted => {
                // rt_sorted(list, reverse, container_tag=0=List). The input is
                // pre-materialized to a list, so the tag is always List; the
                // reverse flag arrives as the op's Raw(I8) argument.
                let tag = self.builder.ins().iconst(types::I8, 0);
                vals.push(tag);
                self.rt.sorted
            }
            // ── container methods (Phase 4D) ──
            ContainerOp::ListPop => self.rt.list_pop,
            ContainerOp::ListInsert => self.rt.list_insert,
            ContainerOp::ListExtend => self.rt.list_extend,
            ContainerOp::ListIndexOf => self.rt.list_index,
            ContainerOp::ListCount => self.rt.list_count,
            ContainerOp::ListClear => self.rt.list_clear,
            ContainerOp::ListCopy => self.rt.list_copy,
            ContainerOp::ListReverse => self.rt.list_reverse,
            // rt_list_sort(list, reverse) — the reverse flag is the op's
            // Raw(I8) argument.
            ContainerOp::ListSortMut => self.rt.list_sort,
            ContainerOp::ListSortByKeys => self.rt.list_sort_by_keys,
            ContainerOp::DictGetDefault => self.rt.dict_get_default,
            ContainerOp::DictKeys => self.rt.dict_keys,
            ContainerOp::DictValues => self.rt.dict_values,
            ContainerOp::DictItems => self.rt.dict_items,
            ContainerOp::DictPopM => self.rt.dict_pop,
            ContainerOp::DictSetdefault => self.rt.dict_setdefault,
            ContainerOp::DictUpdate => self.rt.dict_update,
            ContainerOp::DictClear => self.rt.dict_clear,
            ContainerOp::DictCopy => self.rt.dict_copy,
            ContainerOp::DictMerge => self.rt.dict_merge,
            ContainerOp::SetRemove => self.rt.set_remove,
            ContainerOp::SetDiscard => self.rt.set_discard,
            ContainerOp::SetUpdate => self.rt.set_update,
            ContainerOp::SetUnion => self.rt.set_union,
            ContainerOp::SetIntersection => self.rt.set_intersection,
            ContainerOp::SetDifference => self.rt.set_difference,
            ContainerOp::SetCopy => self.rt.set_copy,
            ContainerOp::SetClear => self.rt.set_clear,
            // §9 builtin-type methods.
            ContainerOp::TupleIndexOf => self.rt.tuple_index,
            ContainerOp::TupleCount => self.rt.tuple_count,
            ContainerOp::SetIsSubset => self.rt.set_issubset,
            ContainerOp::SetIsSuperset => self.rt.set_issuperset,
            ContainerOp::SetIsDisjoint => self.rt.set_isdisjoint,
            ContainerOp::SetIntersectionUpdate => self.rt.set_intersection_update,
            ContainerOp::SetDifferenceUpdate => self.rt.set_difference_update,
            ContainerOp::SetSymmetricDifferenceUpdate => self.rt.set_symmetric_difference_update,
            ContainerOp::SetSymmetricDifference => self.rt.set_symmetric_difference,
            ContainerOp::ListRemove => self.rt.list_remove,
            ContainerOp::DictPopitem => self.rt.dict_popitem,
        };
        let res = self.call(fid, &vals);
        if let (Some(d), Some(v)) = (dst, res) {
            self.def_local(*d, v);
        }
        Ok(())
    }

    /// Lower a `CallVirtual` (Phase 5B): resolve the function pointer for the
    /// receiver's actual class via `rt_vtable_lookup_by_name`, then `call_indirect`
    /// with a signature built from the operand reprs + the resolved return repr.
    fn lower_call_virtual(
        &mut self,
        dst: &Option<LocalId>,
        recv: &Operand,
        name_hash: u64,
        args: &[Operand],
        ret: &Repr,
    ) -> Result<()> {
        let recv_v = self.use_operand(recv);
        let hash_v = self.builder.ins().iconst(types::I64, name_hash as i64);
        let fnptr = self
            .call(self.rt.vtable_lookup_by_name, &[recv_v, hash_v])
            .unwrap();

        // Indirect-call signature: (self: I64, args…) -> ret.
        let mut sig = Signature::new(self.cc);
        sig.params
            .push(abi_param(clif_ty(self.operand_repr(recv))));
        for a in args {
            sig.params
                .push(abi_param(clif_ty(self.operand_repr(a))));
        }
        sig.returns.push(AbiParam::new(clif_ty(ret)));
        let sigref = self.builder.import_signature(sig);

        let mut call_args = Vec::with_capacity(args.len() + 1);
        call_args.push(recv_v);
        for a in args {
            call_args.push(self.use_operand(a));
        }
        let res = self.call_indirect(sigref, fnptr, &call_args);
        if let (Some(d), Some(v)) = (dst, res) {
            self.def_local(*d, v);
        }
        Ok(())
    }

    fn builtin_fn(&self, kind: pyaot_mir::BuiltinFunctionKind) -> Result<FuncId> {
        use pyaot_mir::BuiltinFunctionKind as K;
        Ok(match kind {
            K::Abs => self.rt.builtin_abs,
            K::Hash => self.rt.builtin_hash,
            K::Int => self.rt.builtin_int,
            K::Float => self.rt.builtin_float,
            K::Str => self.rt.builtin_str,
            K::Repr => self.rt.builtin_repr,
            K::Bool => self.rt.builtin_bool,
            K::Len => self.rt.builtin_len,
            K::Ord => self.rt.builtin_ord,
            K::Chr => self.rt.builtin_chr,
            K::Type => self.rt.builtin_type,
            K::Ascii => self.rt.builtin_ascii,
        })
    }

    fn lower_print(&mut self, kind: PrintKind, arg: &Option<Operand>) -> Result<()> {
        match kind {
            PrintKind::Sep => {
                self.call(self.rt.print_sep, &[]);
            }
            PrintKind::Newline => {
                self.call(self.rt.print_newline, &[]);
            }
            PrintKind::None_ => {
                self.call(self.rt.print_none, &[]);
            }
            PrintKind::StrObj => {
                let v = self.use_operand(arg.as_ref().unwrap());
                self.call(self.rt.print_str_obj, &[v]);
            }
            PrintKind::Obj => {
                let v = self.use_operand(arg.as_ref().unwrap());
                self.call(self.rt.print_obj, &[v]);
            }
            PrintKind::Float => {
                let v = self.use_operand(arg.as_ref().unwrap());
                self.call(self.rt.print_float, &[v]);
            }
            PrintKind::Bool => {
                let v = self.use_operand(arg.as_ref().unwrap());
                self.call(self.rt.print_bool, &[v]);
            }
            PrintKind::Int => {
                let v = self.use_operand(arg.as_ref().unwrap());
                self.call(self.rt.print_int, &[v]);
            }
        }
        Ok(())
    }

    fn lower_terminator(&mut self, term: &MirTerminator) -> Result<()> {
        match term {
            MirTerminator::Return(None) => {
                let v = self.default_ret();
                self.emit_gc_epilogue();
                self.builder.ins().return_(&[v]);
            }
            MirTerminator::Return(Some(op)) => {
                let v = self.use_operand(op);
                self.emit_gc_epilogue();
                self.builder.ins().return_(&[v]);
            }
            MirTerminator::Jump(target) => {
                let blk = self.cl_blocks[target.index()];
                self.builder.ins().jump(blk, &[]);
            }
            MirTerminator::Branch { cond, then, else_ } => {
                let c = self.use_operand(cond);
                let t = self.cl_blocks[then.index()];
                let e = self.cl_blocks[else_.index()];
                self.builder.ins().brif(c, t, &[], e, &[]);
            }
            MirTerminator::Unreachable => {
                self.builder.ins().trap(TrapCode::unwrap_user(1));
            }
        }
        Ok(())
    }

    /// A value of the function's return type for `Return(None)` (None-returning
    /// functions have a `Tagged` return → the tagged `None` singleton).
    fn default_ret(&mut self) -> Value {
        if self.program_ret == types::F64 {
            self.builder.ins().f64const(0.0)
        } else if self.program_ret == types::I8 {
            self.builder.ins().iconst(types::I8, 0)
        } else if self.program_ret == types::I32 {
            self.builder.ins().iconst(types::I32, 0)
        } else {
            self.builder.ins().iconst(types::I64, tag::NONE_TAG as i64)
        }
    }
}

fn cg_error(msg: impl Into<String>) -> CompilerError {
    CompilerError::codegen_error(msg.into(), None)
}

/// The runtime `op_tag` for a container ordering comparison (`0=Lt, 1=Lte,
/// 2=Gt, 3=Gte`, matching `rt_obj_cmp`/`rt_list_cmp`). Equality never reaches the
/// typed comparator (it rides the tagged `rt_obj_eq` baseline), so it maps to 0.
fn cmp_op_tag(op: ContainerCmpOp) -> i64 {
    match op {
        ContainerCmpOp::Lt => 0,
        ContainerCmpOp::LtE => 1,
        ContainerCmpOp::Gt => 2,
        ContainerCmpOp::GtE => 3,
        ContainerCmpOp::Eq | ContainerCmpOp::NotEq => 0,
    }
}
