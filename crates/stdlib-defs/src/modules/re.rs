//! re module definition
//!
//! Provides regular expression matching operations.

use crate::types::{
    LoweringHints, ParamDef, StdlibClassDef, StdlibFunctionDef, StdlibMethodDef, StdlibModuleDef,
    TypeSpec, TYPE_INT, TYPE_STR,
};
#[allow(unused_imports)]
use pyaot_core_defs::runtime_func_def::{P_F64, P_I64, P_I8, R_F64, R_I64, R_I8};
use pyaot_core_defs::RuntimeFuncDef;

// ============= Match class methods =============
// These are public so they can be referenced by ObjectTypeDef in object_types.rs

/// Match.group method
pub static MATCH_GROUP: StdlibMethodDef = StdlibMethodDef {
    name: "group",
    runtime_name: "rt_match_group",
    params: &[ParamDef::optional("n", TypeSpec::Int)],
    return_type: TypeSpec::Optional(&TYPE_STR),
    min_args: 0,
    max_args: 1,
    // self (I64) + n (I64) -> Optional[str] (I64)
    codegen: RuntimeFuncDef::new("rt_match_group", &[P_I64, P_I64], Some(R_I64), false),
};

/// Match.start method
pub static MATCH_START: StdlibMethodDef = StdlibMethodDef {
    name: "start",
    runtime_name: "rt_match_start",
    params: &[],
    return_type: TypeSpec::Int,
    min_args: 0,
    max_args: 0,
    // self (I64) -> Int (I64)
    codegen: RuntimeFuncDef::new("rt_match_start", &[P_I64], Some(R_I64), false),
};

/// Match.end method
pub static MATCH_END: StdlibMethodDef = StdlibMethodDef {
    name: "end",
    runtime_name: "rt_match_end",
    params: &[],
    return_type: TypeSpec::Int,
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::new("rt_match_end", &[P_I64], Some(R_I64), false),
};

/// Match.groups method
pub static MATCH_GROUPS: StdlibMethodDef = StdlibMethodDef {
    name: "groups",
    runtime_name: "rt_match_groups",
    params: &[],
    return_type: TypeSpec::Tuple(&TYPE_STR),
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::new("rt_match_groups", &[P_I64], Some(R_I64), false),
};

/// Match.span method
pub static MATCH_SPAN: StdlibMethodDef = StdlibMethodDef {
    name: "span",
    runtime_name: "rt_match_span",
    params: &[],
    return_type: TypeSpec::Tuple(&TYPE_INT),
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::new("rt_match_span", &[P_I64], Some(R_I64), false),
};

/// Match class definition
static MATCH_CLASS: StdlibClassDef = StdlibClassDef {
    name: "Match",
    methods: &[
        MATCH_GROUP,
        MATCH_START,
        MATCH_END,
        MATCH_GROUPS,
        MATCH_SPAN,
    ],
    type_spec: Some(TypeSpec::Match),
};

// ============= re module functions =============

/// re.search function
pub static RE_SEARCH: StdlibFunctionDef = StdlibFunctionDef {
    name: "search",
    runtime_name: "rt_re_search",
    params: &[
        ParamDef::required("pattern", TypeSpec::Str),
        ParamDef::required("string", TypeSpec::Str),
    ],
    return_type: TypeSpec::Optional(&TypeSpec::Match),
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX, // Takes Str directly
    codegen: RuntimeFuncDef::new("rt_re_search", &[P_I64, P_I64], Some(R_I64), false),
};

/// re.match function
pub static RE_MATCH: StdlibFunctionDef = StdlibFunctionDef {
    name: "match",
    runtime_name: "rt_re_match",
    params: &[
        ParamDef::required("pattern", TypeSpec::Str),
        ParamDef::required("string", TypeSpec::Str),
    ],
    return_type: TypeSpec::Optional(&TypeSpec::Match),
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX, // Takes Str directly
    codegen: RuntimeFuncDef::new("rt_re_match", &[P_I64, P_I64], Some(R_I64), false),
};

/// re.sub function
pub static RE_SUB: StdlibFunctionDef = StdlibFunctionDef {
    name: "sub",
    runtime_name: "rt_re_sub",
    params: &[
        ParamDef::required("pattern", TypeSpec::Str),
        ParamDef::required("repl", TypeSpec::Str),
        ParamDef::required("string", TypeSpec::Str),
    ],
    return_type: TypeSpec::Str,
    min_args: 3,
    max_args: 3,
    hints: LoweringHints::NO_AUTO_BOX, // Takes Str directly
    codegen: RuntimeFuncDef::new("rt_re_sub", &[P_I64, P_I64, P_I64], Some(R_I64), false),
};

/// re module definition
pub static RE_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "re",
    functions: &[RE_SEARCH, RE_MATCH, RE_SUB],
    attrs: &[],
    constants: &[],
    classes: &[MATCH_CLASS],
    exceptions: &[],
    submodules: &[],
};
