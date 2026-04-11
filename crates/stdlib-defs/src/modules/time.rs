//! time module definition
//!
//! Provides time-related functions.

use crate::types::{
    ConstValue, LoweringHints, ParamDef, StdlibClassDef, StdlibFunctionDef, StdlibModuleDef,
    TypeSpec,
};
#[allow(unused_imports)]
use pyaot_core_defs::runtime_func_def::{P_F64, P_I64, P_I8, R_F64, R_I64, R_I8};
use pyaot_core_defs::RuntimeFuncDef;

/// time.sleep(seconds) - Pause execution for the given number of seconds
pub static TIME_SLEEP: StdlibFunctionDef = StdlibFunctionDef {
    name: "sleep",
    runtime_name: "rt_time_sleep",
    params: &[ParamDef::required("seconds", TypeSpec::Float)],
    return_type: TypeSpec::None,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::void("rt_time_sleep", &[P_F64]),
};

/// time.time() - Return current Unix timestamp as float
pub static TIME_TIME: StdlibFunctionDef = StdlibFunctionDef {
    name: "time",
    runtime_name: "rt_time_time",
    params: &[],
    return_type: TypeSpec::Float,
    min_args: 0,
    max_args: 0,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_time_time", &[], Some(R_F64), false),
};

/// time.monotonic() - Return monotonic clock value for measuring intervals
pub static TIME_MONOTONIC: StdlibFunctionDef = StdlibFunctionDef {
    name: "monotonic",
    runtime_name: "rt_time_monotonic",
    params: &[],
    return_type: TypeSpec::Float,
    min_args: 0,
    max_args: 0,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_time_monotonic", &[], Some(R_F64), false),
};

/// time.perf_counter() - Return high-resolution performance counter
pub static TIME_PERF_COUNTER: StdlibFunctionDef = StdlibFunctionDef {
    name: "perf_counter",
    runtime_name: "rt_time_perf_counter",
    params: &[],
    return_type: TypeSpec::Float,
    min_args: 0,
    max_args: 0,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_time_perf_counter", &[], Some(R_F64), false),
};

/// time.ctime([seconds]) - Convert seconds to readable local time string
/// If seconds is not provided (or negative), uses current time
pub static TIME_CTIME: StdlibFunctionDef = StdlibFunctionDef {
    name: "ctime",
    runtime_name: "rt_time_ctime",
    params: &[ParamDef::optional_with_default(
        "seconds",
        TypeSpec::Float,
        ConstValue::Float(-1.0), // Sentinel: use current time
    )],
    return_type: TypeSpec::Str,
    min_args: 0,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_time_ctime", &[P_F64], Some(R_I64), false),
};

/// time.localtime([seconds]) - Convert seconds to local time struct_time
/// If seconds is not provided (or negative), uses current time
pub static TIME_LOCALTIME: StdlibFunctionDef = StdlibFunctionDef {
    name: "localtime",
    runtime_name: "rt_time_localtime",
    params: &[ParamDef::optional_with_default(
        "seconds",
        TypeSpec::Float,
        ConstValue::Float(-1.0), // Sentinel: use current time
    )],
    return_type: TypeSpec::StructTime,
    min_args: 0,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_time_localtime", &[P_F64], Some(R_I64), false),
};

/// time.gmtime([seconds]) - Convert seconds to UTC struct_time
/// If seconds is not provided (or negative), uses current time
pub static TIME_GMTIME: StdlibFunctionDef = StdlibFunctionDef {
    name: "gmtime",
    runtime_name: "rt_time_gmtime",
    params: &[ParamDef::optional_with_default(
        "seconds",
        TypeSpec::Float,
        ConstValue::Float(-1.0), // Sentinel: use current time
    )],
    return_type: TypeSpec::StructTime,
    min_args: 0,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_time_gmtime", &[P_F64], Some(R_I64), false),
};

/// time.mktime(t) - Convert struct_time to seconds since epoch
pub static TIME_MKTIME: StdlibFunctionDef = StdlibFunctionDef {
    name: "mktime",
    runtime_name: "rt_time_mktime",
    params: &[ParamDef::required("t", TypeSpec::StructTime)],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_time_mktime", &[P_I64], Some(R_F64), false),
};

/// time.strftime(format[, t]) - Format struct_time to string
/// When t is omitted, uses current local time (like CPython).
/// Common format codes: %Y (year), %m (month), %d (day), %H (hour), %M (minute), %S (second)
pub static TIME_STRFTIME: StdlibFunctionDef = StdlibFunctionDef {
    name: "strftime",
    runtime_name: "rt_time_strftime",
    params: &[
        ParamDef::required("format", TypeSpec::Str),
        ParamDef::optional("t", TypeSpec::StructTime),
    ],
    return_type: TypeSpec::Str,
    min_args: 1,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_time_strftime", &[P_I64, P_I64], Some(R_I64), false),
};

/// time.strptime(string, format) - Parse string to struct_time
/// Common format codes: %Y (year), %m (month), %d (day), %H (hour), %M (minute), %S (second)
pub static TIME_STRPTIME: StdlibFunctionDef = StdlibFunctionDef {
    name: "strptime",
    runtime_name: "rt_time_strptime",
    params: &[
        ParamDef::required("string", TypeSpec::Str),
        ParamDef::required("format", TypeSpec::Str),
    ],
    return_type: TypeSpec::StructTime,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_time_strptime", &[P_I64, P_I64], Some(R_I64), false),
};

/// struct_time class definition.
/// Fields are exposed via ObjectTypeDef in object_types.rs (uses rt_struct_time_get_field).
pub static STRUCT_TIME_CLASS: StdlibClassDef = StdlibClassDef {
    name: "struct_time",
    methods: &[],
    type_spec: Some(TypeSpec::StructTime),
};

/// time module definition
pub static TIME_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "time",
    functions: &[
        TIME_SLEEP,
        TIME_TIME,
        TIME_MONOTONIC,
        TIME_PERF_COUNTER,
        TIME_CTIME,
        TIME_LOCALTIME,
        TIME_GMTIME,
        TIME_MKTIME,
        TIME_STRFTIME,
        TIME_STRPTIME,
    ],
    attrs: &[],
    constants: &[],
    classes: &[STRUCT_TIME_CLASS],
    submodules: &[],
};
