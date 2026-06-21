//! sys module definition
//!
//! Provides access to system-specific parameters and functions.

use crate::types::{
    ConstValue, LoweringHints, ParamDef, StdlibAttrDef, StdlibConstDef, StdlibFunctionDef,
    StdlibModuleDef, TypeSpec, TYPE_STR,
};
#[allow(unused_imports)]
use pyaot_core_defs::runtime_func_def::{P_F64, P_I64, P_I8, R_F64, R_I64, R_I8};
use pyaot_core_defs::RuntimeFuncDef;

// `sys.platform` is the host platform string (CPython's values, not Rust's
// `std::env::consts::OS`). pyaot compiles natively, so host == target and the
// build-time `cfg` reflects the produced binary's platform.
#[cfg(target_os = "macos")]
const PLATFORM: &str = "darwin";
#[cfg(target_os = "linux")]
const PLATFORM: &str = "linux";
#[cfg(target_os = "windows")]
const PLATFORM: &str = "win32";
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
const PLATFORM: &str = "unknown";

#[cfg(target_endian = "little")]
const BYTEORDER: &str = "little";
#[cfg(target_endian = "big")]
const BYTEORDER: &str = "big";

/// sys.argv attribute
pub static SYS_ARGV: StdlibAttrDef = StdlibAttrDef {
    name: "argv",
    runtime_getter: "rt_sys_get_argv",
    ty: TypeSpec::List(&TYPE_STR),
    writable: false,
    codegen: RuntimeFuncDef::new("rt_sys_get_argv", &[], Some(R_I64), false),
};

/// sys.path attribute — module search path.
///
/// Initialised at process start from:
///   1. Directory of the executable (for code next to the installed binary).
///   2. Current working directory.
///   3. Each `:`-separated entry of `PYTHONPATH`, if set.
///
/// Returns the SAME `ListObj` across calls, so mutations like
/// `sys.path.append("...")` persist for the lifetime of the process.
/// The list has no effect on module resolution (that happens at compile
/// time), but matches CPython's surface so portability/diagnostic code
/// that reads `sys.path` keeps working.
pub static SYS_PATH: StdlibAttrDef = StdlibAttrDef {
    name: "path",
    runtime_getter: "rt_sys_get_path",
    ty: TypeSpec::List(&TYPE_STR),
    writable: false,
    codegen: RuntimeFuncDef::new("rt_sys_get_path", &[], Some(R_I64), false),
};

/// sys.exit function
pub static SYS_EXIT: StdlibFunctionDef = StdlibFunctionDef {
    name: "exit",
    runtime_name: "rt_sys_exit",
    params: &[ParamDef::optional_with_default(
        "code",
        TypeSpec::Int,
        ConstValue::Int(0),
    )],
    return_type: TypeSpec::None, // Never returns, but type is None
    min_args: 0,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX, // Takes Int directly
    codegen: RuntimeFuncDef::void("rt_sys_exit", &[P_I64]),
};

/// sys.intern function
pub static SYS_INTERN: StdlibFunctionDef = StdlibFunctionDef {
    name: "intern",
    runtime_name: "rt_sys_intern",
    params: &[ParamDef::required("string", TypeSpec::Str)],
    return_type: TypeSpec::Str,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX, // Takes Str directly
    codegen: RuntimeFuncDef::new("rt_sys_intern", &[P_I64], Some(R_I64), false),
};

/// sys.platform — host platform identifier (e.g. "darwin", "linux", "win32").
pub static SYS_PLATFORM: StdlibConstDef = StdlibConstDef {
    name: "platform",
    value: ConstValue::Str(PLATFORM),
    ty: TypeSpec::Str,
};

/// sys.maxsize — largest `Py_ssize_t`; `2**63 - 1` on a 64-bit platform.
pub static SYS_MAXSIZE: StdlibConstDef = StdlibConstDef {
    name: "maxsize",
    value: ConstValue::Int(i64::MAX),
    ty: TypeSpec::Int,
};

/// sys.maxunicode — largest Unicode code point (`0x10FFFF`).
pub static SYS_MAXUNICODE: StdlibConstDef = StdlibConstDef {
    name: "maxunicode",
    value: ConstValue::Int(0x10FFFF),
    ty: TypeSpec::Int,
};

/// sys.byteorder — native byte order ("little" / "big").
pub static SYS_BYTEORDER: StdlibConstDef = StdlibConstDef {
    name: "byteorder",
    value: ConstValue::Str(BYTEORDER),
    ty: TypeSpec::Str,
};

/// sys module definition
pub static SYS_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "sys",
    functions: &[SYS_EXIT, SYS_INTERN],
    attrs: &[SYS_ARGV, SYS_PATH],
    constants: &[SYS_PLATFORM, SYS_MAXSIZE, SYS_MAXUNICODE, SYS_BYTEORDER],
    classes: &[],
    exceptions: &[],
    submodules: &[],
};
