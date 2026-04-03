//! os module definition
//!
//! Provides access to operating system dependent functionality.

use crate::types::{
    ConstValue, LoweringHints, ParamDef, StdlibAttrDef, StdlibFunctionDef, StdlibModuleDef,
    TypeSpec, TYPE_STR,
};
#[allow(unused_imports)]
use pyaot_core_defs::runtime_func_def::{P_F64, P_I64, P_I8, R_F64, R_I64, R_I8};
use pyaot_core_defs::RuntimeFuncDef;

/// os.environ attribute
pub static OS_ENVIRON: StdlibAttrDef = StdlibAttrDef {
    name: "environ",
    runtime_getter: "rt_os_get_environ",
    ty: TypeSpec::Dict(&TYPE_STR, &TYPE_STR),
    writable: false,
    codegen: RuntimeFuncDef::new("rt_os_get_environ", &[], Some(R_I64), false),
};

/// os.name attribute - OS type ('posix', 'nt', etc.)
pub static OS_NAME: StdlibAttrDef = StdlibAttrDef {
    name: "name",
    runtime_getter: "rt_os_get_name",
    ty: TypeSpec::Str,
    writable: false,
    codegen: RuntimeFuncDef::new("rt_os_get_name", &[], Some(R_I64), false),
};

// ============= File/directory operations =============

/// os.remove function
pub static OS_REMOVE: StdlibFunctionDef = StdlibFunctionDef {
    name: "remove",
    runtime_name: "rt_os_remove",
    params: &[ParamDef::required("path", TypeSpec::Str)],
    return_type: TypeSpec::None,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::void("rt_os_remove", &[P_I64]),
};

/// os.getcwd function - get current working directory
pub static OS_GETCWD: StdlibFunctionDef = StdlibFunctionDef {
    name: "getcwd",
    runtime_name: "rt_os_getcwd",
    params: &[],
    return_type: TypeSpec::Str,
    min_args: 0,
    max_args: 0,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_os_getcwd", &[], Some(R_I64), false),
};

/// os.chdir function - change current working directory
pub static OS_CHDIR: StdlibFunctionDef = StdlibFunctionDef {
    name: "chdir",
    runtime_name: "rt_os_chdir",
    params: &[ParamDef::required("path", TypeSpec::Str)],
    return_type: TypeSpec::None,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::void("rt_os_chdir", &[P_I64]),
};

/// os.listdir function - list files in directory
/// When no path is provided, uses current directory
pub static OS_LISTDIR: StdlibFunctionDef = StdlibFunctionDef {
    name: "listdir",
    runtime_name: "rt_os_listdir",
    params: &[ParamDef::optional("path", TypeSpec::Str)],
    return_type: TypeSpec::List(&TYPE_STR),
    min_args: 0,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_os_listdir", &[P_I64], Some(R_I64), false),
};

/// os.mkdir function - create a directory
pub static OS_MKDIR: StdlibFunctionDef = StdlibFunctionDef {
    name: "mkdir",
    runtime_name: "rt_os_mkdir",
    params: &[ParamDef::required("path", TypeSpec::Str)],
    return_type: TypeSpec::None,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::void("rt_os_mkdir", &[P_I64]),
};

/// os.makedirs function - create directories recursively
pub static OS_MAKEDIRS: StdlibFunctionDef = StdlibFunctionDef {
    name: "makedirs",
    runtime_name: "rt_os_makedirs",
    params: &[
        ParamDef::required("path", TypeSpec::Str),
        ParamDef::optional_with_default("exist_ok", TypeSpec::Bool, ConstValue::Bool(false)),
    ],
    return_type: TypeSpec::None,
    min_args: 1,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::void("rt_os_makedirs", &[P_I64, P_I8]),
};

/// os.rmdir function - remove a directory
pub static OS_RMDIR: StdlibFunctionDef = StdlibFunctionDef {
    name: "rmdir",
    runtime_name: "rt_os_rmdir",
    params: &[ParamDef::required("path", TypeSpec::Str)],
    return_type: TypeSpec::None,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::void("rt_os_rmdir", &[P_I64]),
};

/// os.rename function - rename or move file/directory
pub static OS_RENAME: StdlibFunctionDef = StdlibFunctionDef {
    name: "rename",
    runtime_name: "rt_os_rename",
    params: &[
        ParamDef::required("src", TypeSpec::Str),
        ParamDef::required("dst", TypeSpec::Str),
    ],
    return_type: TypeSpec::None,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::void("rt_os_rename", &[P_I64, P_I64]),
};

/// os.replace function - replace file/directory
pub static OS_REPLACE: StdlibFunctionDef = StdlibFunctionDef {
    name: "replace",
    runtime_name: "rt_os_replace",
    params: &[
        ParamDef::required("src", TypeSpec::Str),
        ParamDef::required("dst", TypeSpec::Str),
    ],
    return_type: TypeSpec::None,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::void("rt_os_replace", &[P_I64, P_I64]),
};

/// os.getenv function - get environment variable
/// default parameter is optional (will be null pointer if not provided)
pub static OS_GETENV: StdlibFunctionDef = StdlibFunctionDef {
    name: "getenv",
    runtime_name: "rt_os_getenv",
    params: &[
        ParamDef::required("key", TypeSpec::Str),
        ParamDef::optional("default", TypeSpec::Str),
    ],
    return_type: TypeSpec::Optional(&TYPE_STR),
    min_args: 1,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_os_getenv", &[P_I64, P_I64], Some(R_I64), false),
};

/// os module definition
pub static OS_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "os",
    functions: &[
        OS_REMOVE,
        OS_GETCWD,
        OS_CHDIR,
        OS_LISTDIR,
        OS_MKDIR,
        OS_MAKEDIRS,
        OS_RMDIR,
        OS_RENAME,
        OS_REPLACE,
        OS_GETENV,
    ],
    attrs: &[OS_ENVIRON, OS_NAME],
    constants: &[],
    classes: &[],
    submodules: &[&OS_PATH_MODULE],
};

// ============= os.path submodule =============

/// os.path.join function - variadic args collected to list
/// Requires at least one path argument (CPython raises TypeError with 0 args)
pub static OS_PATH_JOIN: StdlibFunctionDef = StdlibFunctionDef {
    name: "join",
    runtime_name: "rt_os_path_join",
    params: &[ParamDef::variadic("paths", TypeSpec::Str)],
    return_type: TypeSpec::Str,
    min_args: 1,
    max_args: usize::MAX,
    hints: LoweringHints::VARIADIC_TO_LIST,
    // variadic_to_list: lowering collects args into a list, so codegen receives a single I64 (list ptr)
    codegen: RuntimeFuncDef::new("rt_os_path_join", &[P_I64], Some(R_I64), false),
};

/// os.path.exists function
pub static OS_PATH_EXISTS: StdlibFunctionDef = StdlibFunctionDef {
    name: "exists",
    runtime_name: "rt_os_path_exists",
    params: &[ParamDef::required("path", TypeSpec::Str)],
    return_type: TypeSpec::Bool,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_os_path_exists", &[P_I64], Some(R_I8), false),
};

/// os.path.abspath function - get absolute path
pub static OS_PATH_ABSPATH: StdlibFunctionDef = StdlibFunctionDef {
    name: "abspath",
    runtime_name: "rt_os_path_abspath",
    params: &[ParamDef::required("path", TypeSpec::Str)],
    return_type: TypeSpec::Str,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_os_path_abspath", &[P_I64], Some(R_I64), false),
};

/// os.path.isdir function - check if path is directory
pub static OS_PATH_ISDIR: StdlibFunctionDef = StdlibFunctionDef {
    name: "isdir",
    runtime_name: "rt_os_path_isdir",
    params: &[ParamDef::required("path", TypeSpec::Str)],
    return_type: TypeSpec::Bool,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_os_path_isdir", &[P_I64], Some(R_I8), false),
};

/// os.path.isfile function - check if path is file
pub static OS_PATH_ISFILE: StdlibFunctionDef = StdlibFunctionDef {
    name: "isfile",
    runtime_name: "rt_os_path_isfile",
    params: &[ParamDef::required("path", TypeSpec::Str)],
    return_type: TypeSpec::Bool,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_os_path_isfile", &[P_I64], Some(R_I8), false),
};

/// os.path.basename function - get file name
pub static OS_PATH_BASENAME: StdlibFunctionDef = StdlibFunctionDef {
    name: "basename",
    runtime_name: "rt_os_path_basename",
    params: &[ParamDef::required("path", TypeSpec::Str)],
    return_type: TypeSpec::Str,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_os_path_basename", &[P_I64], Some(R_I64), false),
};

/// os.path.dirname function - get parent directory
pub static OS_PATH_DIRNAME: StdlibFunctionDef = StdlibFunctionDef {
    name: "dirname",
    runtime_name: "rt_os_path_dirname",
    params: &[ParamDef::required("path", TypeSpec::Str)],
    return_type: TypeSpec::Str,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_os_path_dirname", &[P_I64], Some(R_I64), false),
};

/// os.path.split function - split path into (dirname, basename)
pub static OS_PATH_SPLIT: StdlibFunctionDef = StdlibFunctionDef {
    name: "split",
    runtime_name: "rt_os_path_split",
    params: &[ParamDef::required("path", TypeSpec::Str)],
    return_type: TypeSpec::Tuple(&TYPE_STR),
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_os_path_split", &[P_I64], Some(R_I64), false),
};

/// os.path submodule definition
pub static OS_PATH_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "os.path",
    functions: &[
        OS_PATH_JOIN,
        OS_PATH_EXISTS,
        OS_PATH_ABSPATH,
        OS_PATH_ISDIR,
        OS_PATH_ISFILE,
        OS_PATH_BASENAME,
        OS_PATH_DIRNAME,
        OS_PATH_SPLIT,
    ],
    attrs: &[],
    constants: &[],
    classes: &[],
    submodules: &[],
};
