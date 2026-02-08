//! subprocess module definition
//!
//! Provides support for spawning processes and interacting with them.

use crate::types::{
    ConstValue, LoweringHints, ParamDef, StdlibClassDef, StdlibFunctionDef, StdlibMethodDef,
    StdlibModuleDef, TypeSpec, TYPE_STR,
};

/// subprocess.run function
/// Simplified version that takes:
/// - args: list[str] - command and arguments
/// - capture_output: bool - whether to capture stdout/stderr (default: False)
/// - check: bool - whether to raise RuntimeError on non-zero exit (default: False)
pub static SUBPROCESS_RUN: StdlibFunctionDef = StdlibFunctionDef {
    name: "run",
    runtime_name: "rt_subprocess_run",
    params: &[
        ParamDef::required("args", TypeSpec::List(&TYPE_STR)),
        ParamDef::optional_with_default("capture_output", TypeSpec::Bool, ConstValue::Bool(false)),
        ParamDef::optional_with_default("check", TypeSpec::Bool, ConstValue::Bool(false)),
    ],
    return_type: TypeSpec::CompletedProcess,
    min_args: 1,
    max_args: 3,
    hints: LoweringHints::NO_AUTO_BOX,
};

// ============= CompletedProcess class methods =============

/// CompletedProcess.args getter
static COMPLETED_PROCESS_ARGS: StdlibMethodDef = StdlibMethodDef {
    name: "args",
    runtime_name: "rt_completed_process_get_args",
    params: &[],
    return_type: TypeSpec::List(&TYPE_STR),
    min_args: 0,
    max_args: 0,
};

/// CompletedProcess.returncode getter
static COMPLETED_PROCESS_RETURNCODE: StdlibMethodDef = StdlibMethodDef {
    name: "returncode",
    runtime_name: "rt_completed_process_get_returncode",
    params: &[],
    return_type: TypeSpec::Int,
    min_args: 0,
    max_args: 0,
};

/// CompletedProcess.stdout getter
static COMPLETED_PROCESS_STDOUT: StdlibMethodDef = StdlibMethodDef {
    name: "stdout",
    runtime_name: "rt_completed_process_get_stdout",
    params: &[],
    return_type: TypeSpec::Optional(&TYPE_STR),
    min_args: 0,
    max_args: 0,
};

/// CompletedProcess.stderr getter
static COMPLETED_PROCESS_STDERR: StdlibMethodDef = StdlibMethodDef {
    name: "stderr",
    runtime_name: "rt_completed_process_get_stderr",
    params: &[],
    return_type: TypeSpec::Optional(&TYPE_STR),
    min_args: 0,
    max_args: 0,
};

/// CompletedProcess class definition
pub static COMPLETED_PROCESS_CLASS: StdlibClassDef = StdlibClassDef {
    name: "CompletedProcess",
    methods: &[
        COMPLETED_PROCESS_ARGS,
        COMPLETED_PROCESS_RETURNCODE,
        COMPLETED_PROCESS_STDOUT,
        COMPLETED_PROCESS_STDERR,
    ],
    type_spec: Some(TypeSpec::CompletedProcess),
};

/// subprocess module definition
pub static SUBPROCESS_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "subprocess",
    functions: &[SUBPROCESS_RUN],
    attrs: &[],
    constants: &[],
    classes: &[COMPLETED_PROCESS_CLASS],
    submodules: &[],
};
