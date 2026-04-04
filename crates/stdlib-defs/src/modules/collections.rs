//! collections module definition
//!
//! Provides OrderedDict, defaultdict, Counter, and deque.

use crate::types::{
    ConstValue, LoweringHints, ParamDef, StdlibClassDef, StdlibFunctionDef, StdlibMethodDef,
    StdlibModuleDef, TypeSpec,
};
#[allow(unused_imports)]
use pyaot_core_defs::runtime_func_def::{P_F64, P_I64, P_I8, R_F64, R_I64, R_I8};
use pyaot_core_defs::RuntimeFuncDef;

// =============================================================================
// OrderedDict
// =============================================================================
// Dict already preserves insertion order. OrderedDict adds move_to_end and
// popitem(last=True/False). The constructor maps to rt_make_dict.

/// OrderedDict() constructor -- creates an empty ordered dict (same as dict)
/// The capacity parameter maps to rt_make_dict(capacity); default 0 = use default size.
static ORDERED_DICT_NEW: StdlibFunctionDef = StdlibFunctionDef {
    name: "OrderedDict",
    runtime_name: "rt_make_dict",
    params: &[ParamDef::optional_with_default(
        "capacity",
        TypeSpec::Int,
        ConstValue::Int(0),
    )],
    return_type: TypeSpec::Dict(&TypeSpec::Any, &TypeSpec::Any),
    min_args: 0,
    max_args: 0,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_make_dict", &[P_I64], Some(R_I64), false),
};

/// OrderedDict.move_to_end(key, last=True)
pub static ORDERED_DICT_MOVE_TO_END: StdlibMethodDef = StdlibMethodDef {
    name: "move_to_end",
    runtime_name: "rt_dict_move_to_end",
    params: &[
        ParamDef::required("key", TypeSpec::Any),
        ParamDef::optional_with_default("last", TypeSpec::Bool, ConstValue::Bool(true)),
    ],
    return_type: TypeSpec::None,
    min_args: 1,
    max_args: 2,
    // self (I64) + key (I64) + last (I8) -> void
    codegen: RuntimeFuncDef::void("rt_dict_move_to_end", &[P_I64, P_I64, P_I8]),
};

/// OrderedDict.popitem(last=True)
pub static ORDERED_DICT_POPITEM: StdlibMethodDef = StdlibMethodDef {
    name: "popitem",
    runtime_name: "rt_dict_popitem_ordered",
    params: &[ParamDef::optional_with_default(
        "last",
        TypeSpec::Bool,
        ConstValue::Bool(true),
    )],
    return_type: TypeSpec::Tuple(&TypeSpec::Any),
    min_args: 0,
    max_args: 1,
    // self (I64) + last (I8) -> Tuple (I64)
    codegen: RuntimeFuncDef::new(
        "rt_dict_popitem_ordered",
        &[P_I64, P_I8],
        Some(R_I64),
        false,
    ),
};

/// Helper for move_to_end via StdlibCall (used by dict method lowering)
pub static ORDERED_DICT_MOVE_TO_END_FUNC: StdlibFunctionDef = StdlibFunctionDef {
    name: "move_to_end",
    runtime_name: "rt_dict_move_to_end",
    params: &[
        ParamDef::required("dict", TypeSpec::Any),
        ParamDef::required("key", TypeSpec::Any),
        ParamDef::required("last", TypeSpec::Int),
    ],
    return_type: TypeSpec::None,
    min_args: 3,
    max_args: 3,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::void("rt_dict_move_to_end", &[P_I64, P_I64, P_I64]),
};

/// Helper for popitem via StdlibCall (used by dict method lowering)
pub static ORDERED_DICT_POPITEM_FUNC: StdlibFunctionDef = StdlibFunctionDef {
    name: "popitem",
    runtime_name: "rt_dict_popitem_ordered",
    params: &[
        ParamDef::required("dict", TypeSpec::Any),
        ParamDef::required("last", TypeSpec::Int),
    ],
    return_type: TypeSpec::Tuple(&TypeSpec::Any),
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new(
        "rt_dict_popitem_ordered",
        &[P_I64, P_I64],
        Some(R_I64),
        false,
    ),
};

/// OrderedDict class definition
static ORDERED_DICT_CLASS: StdlibClassDef = StdlibClassDef {
    name: "OrderedDict",
    methods: &[ORDERED_DICT_MOVE_TO_END, ORDERED_DICT_POPITEM],
    type_spec: Some(TypeSpec::Dict(&TypeSpec::Any, &TypeSpec::Any)),
};

// =============================================================================
// defaultdict
// =============================================================================
// defaultdict is registered as a function so `from collections import defaultdict`
// works via the existing import mechanism. The frontend intercepts calls to it
// and converts them to Builtin::DefaultDict for special lowering (factory argument).

/// defaultdict(factory) -- registered as function for import recognition.
/// Actual lowering intercepts this via runtime_name check and uses Builtin::DefaultDict.
pub static DEFAULTDICT_NEW: StdlibFunctionDef = StdlibFunctionDef {
    name: "defaultdict",
    runtime_name: "rt_make_defaultdict",
    params: &[ParamDef::optional("default_factory", TypeSpec::Any)],
    return_type: TypeSpec::Any, // Actual type inferred during lowering
    min_args: 0,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_make_defaultdict", &[P_I64], Some(R_I64), false),
};

// =============================================================================
// Counter
// =============================================================================

/// Counter.most_common(n=-1)
pub static COUNTER_MOST_COMMON: StdlibMethodDef = StdlibMethodDef {
    name: "most_common",
    runtime_name: "rt_counter_most_common",
    params: &[ParamDef::optional_with_default(
        "n",
        TypeSpec::Int,
        ConstValue::Int(-1),
    )],
    return_type: TypeSpec::List(&TypeSpec::Tuple(&TypeSpec::Any)),
    min_args: 0,
    max_args: 1,
    codegen: RuntimeFuncDef::new(
        "rt_counter_most_common",
        &[P_I64, P_I64],
        Some(R_I64),
        false,
    ),
};

/// Counter.total()
pub static COUNTER_TOTAL: StdlibMethodDef = StdlibMethodDef {
    name: "total",
    runtime_name: "rt_counter_total",
    params: &[],
    return_type: TypeSpec::Int,
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::new("rt_counter_total", &[P_I64], Some(R_I64), false),
};

/// Counter.update(iterable)
pub static COUNTER_UPDATE: StdlibMethodDef = StdlibMethodDef {
    name: "update",
    runtime_name: "rt_counter_update",
    params: &[ParamDef::required("iterable", TypeSpec::Any)],
    return_type: TypeSpec::None,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::void("rt_counter_update", &[P_I64, P_I64]),
};

/// Counter.subtract(iterable)
pub static COUNTER_SUBTRACT: StdlibMethodDef = StdlibMethodDef {
    name: "subtract",
    runtime_name: "rt_counter_subtract",
    params: &[ParamDef::required("iterable", TypeSpec::Any)],
    return_type: TypeSpec::None,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::void("rt_counter_subtract", &[P_I64, P_I64]),
};

/// Counter(iterable?) -- registered as function for import recognition.
/// Frontend intercepts and converts to Builtin::Counter.
pub static COUNTER_NEW: StdlibFunctionDef = StdlibFunctionDef {
    name: "Counter",
    runtime_name: "rt_make_counter",
    params: &[ParamDef::optional("iterable", TypeSpec::Any)],
    return_type: TypeSpec::Any,
    min_args: 0,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_make_counter", &[P_I64], Some(R_I64), false),
};

// =============================================================================
// deque
// =============================================================================

/// deque.append(elem)
pub static DEQUE_APPEND: StdlibMethodDef = StdlibMethodDef {
    name: "append",
    runtime_name: "rt_deque_append",
    params: &[ParamDef::required("x", TypeSpec::Any)],
    return_type: TypeSpec::None,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::void("rt_deque_append", &[P_I64, P_I64]),
};

/// deque.appendleft(elem)
pub static DEQUE_APPENDLEFT: StdlibMethodDef = StdlibMethodDef {
    name: "appendleft",
    runtime_name: "rt_deque_appendleft",
    params: &[ParamDef::required("x", TypeSpec::Any)],
    return_type: TypeSpec::None,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::void("rt_deque_appendleft", &[P_I64, P_I64]),
};

/// deque.pop()
pub static DEQUE_POP: StdlibMethodDef = StdlibMethodDef {
    name: "pop",
    runtime_name: "rt_deque_pop",
    params: &[],
    return_type: TypeSpec::Any,
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::new("rt_deque_pop", &[P_I64], Some(R_I64), false),
};

/// deque.popleft()
pub static DEQUE_POPLEFT: StdlibMethodDef = StdlibMethodDef {
    name: "popleft",
    runtime_name: "rt_deque_popleft",
    params: &[],
    return_type: TypeSpec::Any,
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::new("rt_deque_popleft", &[P_I64], Some(R_I64), false),
};

/// deque.extend(iterable)
pub static DEQUE_EXTEND: StdlibMethodDef = StdlibMethodDef {
    name: "extend",
    runtime_name: "rt_deque_extend",
    params: &[ParamDef::required("iterable", TypeSpec::Any)],
    return_type: TypeSpec::None,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::void("rt_deque_extend", &[P_I64, P_I64]),
};

/// deque.extendleft(iterable)
pub static DEQUE_EXTENDLEFT: StdlibMethodDef = StdlibMethodDef {
    name: "extendleft",
    runtime_name: "rt_deque_extendleft",
    params: &[ParamDef::required("iterable", TypeSpec::Any)],
    return_type: TypeSpec::None,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::void("rt_deque_extendleft", &[P_I64, P_I64]),
};

/// deque.rotate(n=1)
pub static DEQUE_ROTATE: StdlibMethodDef = StdlibMethodDef {
    name: "rotate",
    runtime_name: "rt_deque_rotate",
    params: &[ParamDef::optional_with_default(
        "n",
        TypeSpec::Int,
        ConstValue::Int(1),
    )],
    return_type: TypeSpec::None,
    min_args: 0,
    max_args: 1,
    codegen: RuntimeFuncDef::void("rt_deque_rotate", &[P_I64, P_I64]),
};

/// deque.clear()
pub static DEQUE_CLEAR: StdlibMethodDef = StdlibMethodDef {
    name: "clear",
    runtime_name: "rt_deque_clear",
    params: &[],
    return_type: TypeSpec::None,
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::void("rt_deque_clear", &[P_I64]),
};

/// deque.reverse()
pub static DEQUE_REVERSE: StdlibMethodDef = StdlibMethodDef {
    name: "reverse",
    runtime_name: "rt_deque_reverse",
    params: &[],
    return_type: TypeSpec::None,
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::void("rt_deque_reverse", &[P_I64]),
};

/// deque.copy()
pub static DEQUE_COPY: StdlibMethodDef = StdlibMethodDef {
    name: "copy",
    runtime_name: "rt_deque_copy",
    params: &[],
    return_type: TypeSpec::Deque,
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::new("rt_deque_copy", &[P_I64], Some(R_I64), false),
};

/// deque.count(value)
pub static DEQUE_COUNT: StdlibMethodDef = StdlibMethodDef {
    name: "count",
    runtime_name: "rt_deque_count",
    params: &[ParamDef::required("x", TypeSpec::Any)],
    return_type: TypeSpec::Int,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::new("rt_deque_count", &[P_I64, P_I64], Some(R_I64), false),
};

/// Helper for len(deque) -- used by StdlibCall in lowering
pub static DEQUE_LEN: StdlibFunctionDef = StdlibFunctionDef {
    name: "deque_len",
    runtime_name: "rt_deque_len",
    params: &[ParamDef::required("deque", TypeSpec::Any)],
    return_type: TypeSpec::Int,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_deque_len", &[P_I64], Some(R_I64), false),
};

/// deque(iterable?, maxlen?) -- registered as function for import recognition.
/// Frontend intercepts and converts to Builtin::Deque.
pub static DEQUE_NEW: StdlibFunctionDef = StdlibFunctionDef {
    name: "deque",
    runtime_name: "rt_make_deque",
    params: &[
        ParamDef::optional("iterable", TypeSpec::Any),
        ParamDef::optional_with_default("maxlen", TypeSpec::Int, ConstValue::Int(-1)),
    ],
    return_type: TypeSpec::Any,
    min_args: 0,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_make_deque", &[P_I64, P_I64], Some(R_I64), false),
};

// =============================================================================
// collections module
// =============================================================================

pub static COLLECTIONS_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "collections",
    functions: &[ORDERED_DICT_NEW, DEFAULTDICT_NEW, COUNTER_NEW, DEQUE_NEW],
    attrs: &[],
    constants: &[],
    classes: &[ORDERED_DICT_CLASS],
    submodules: &[],
};
