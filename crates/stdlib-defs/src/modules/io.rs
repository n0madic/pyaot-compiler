//! io module definition
//!
//! Provides in-memory I/O objects: StringIO and BytesIO.

use crate::types::{
    ConstValue, LoweringHints, ParamDef, StdlibFunctionDef, StdlibMethodDef, StdlibModuleDef,
    TypeSpec,
};

/// io.StringIO(initial?) constructor
pub static IO_STRINGIO: StdlibFunctionDef = StdlibFunctionDef {
    name: "StringIO",
    runtime_name: "rt_stringio_new",
    params: &[ParamDef::optional("initial", TypeSpec::Str)],
    return_type: TypeSpec::StringIO,
    min_args: 0,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
};

/// io.BytesIO(initial?) constructor
pub static IO_BYTESIO: StdlibFunctionDef = StdlibFunctionDef {
    name: "BytesIO",
    runtime_name: "rt_bytesio_new",
    params: &[ParamDef::optional("initial", TypeSpec::Bytes)],
    return_type: TypeSpec::BytesIO,
    min_args: 0,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
};

// StringIO methods
pub static STRINGIO_WRITE: StdlibMethodDef = StdlibMethodDef {
    name: "write",
    runtime_name: "rt_stringio_write",
    params: &[ParamDef::required("s", TypeSpec::Str)],
    return_type: TypeSpec::Int,
    min_args: 1,
    max_args: 1,
};

pub static STRINGIO_READ: StdlibMethodDef = StdlibMethodDef {
    name: "read",
    runtime_name: "rt_stringio_read",
    params: &[ParamDef::optional_with_default(
        "size",
        TypeSpec::Int,
        ConstValue::Int(-1),
    )],
    return_type: TypeSpec::Str,
    min_args: 0,
    max_args: 1,
};

pub static STRINGIO_READLINE: StdlibMethodDef = StdlibMethodDef {
    name: "readline",
    runtime_name: "rt_stringio_readline",
    params: &[],
    return_type: TypeSpec::Str,
    min_args: 0,
    max_args: 0,
};

pub static STRINGIO_GETVALUE: StdlibMethodDef = StdlibMethodDef {
    name: "getvalue",
    runtime_name: "rt_stringio_getvalue",
    params: &[],
    return_type: TypeSpec::Str,
    min_args: 0,
    max_args: 0,
};

pub static STRINGIO_SEEK: StdlibMethodDef = StdlibMethodDef {
    name: "seek",
    runtime_name: "rt_stringio_seek",
    params: &[ParamDef::required("pos", TypeSpec::Int)],
    return_type: TypeSpec::Int,
    min_args: 1,
    max_args: 1,
};

pub static STRINGIO_TELL: StdlibMethodDef = StdlibMethodDef {
    name: "tell",
    runtime_name: "rt_stringio_tell",
    params: &[],
    return_type: TypeSpec::Int,
    min_args: 0,
    max_args: 0,
};

pub static STRINGIO_CLOSE: StdlibMethodDef = StdlibMethodDef {
    name: "close",
    runtime_name: "rt_stringio_close",
    params: &[],
    return_type: TypeSpec::None,
    min_args: 0,
    max_args: 0,
};

pub static STRINGIO_TRUNCATE: StdlibMethodDef = StdlibMethodDef {
    name: "truncate",
    runtime_name: "rt_stringio_truncate",
    params: &[ParamDef::optional_with_default(
        "size",
        TypeSpec::Int,
        ConstValue::Int(-1),
    )],
    return_type: TypeSpec::Int,
    min_args: 0,
    max_args: 1,
};

// BytesIO methods
pub static BYTESIO_WRITE: StdlibMethodDef = StdlibMethodDef {
    name: "write",
    runtime_name: "rt_bytesio_write",
    params: &[ParamDef::required("b", TypeSpec::Bytes)],
    return_type: TypeSpec::Int,
    min_args: 1,
    max_args: 1,
};

pub static BYTESIO_READ: StdlibMethodDef = StdlibMethodDef {
    name: "read",
    runtime_name: "rt_bytesio_read",
    params: &[ParamDef::optional_with_default(
        "size",
        TypeSpec::Int,
        ConstValue::Int(-1),
    )],
    return_type: TypeSpec::Bytes,
    min_args: 0,
    max_args: 1,
};

pub static BYTESIO_GETVALUE: StdlibMethodDef = StdlibMethodDef {
    name: "getvalue",
    runtime_name: "rt_bytesio_getvalue",
    params: &[],
    return_type: TypeSpec::Bytes,
    min_args: 0,
    max_args: 0,
};

pub static BYTESIO_SEEK: StdlibMethodDef = StdlibMethodDef {
    name: "seek",
    runtime_name: "rt_bytesio_seek",
    params: &[ParamDef::required("pos", TypeSpec::Int)],
    return_type: TypeSpec::Int,
    min_args: 1,
    max_args: 1,
};

pub static BYTESIO_TELL: StdlibMethodDef = StdlibMethodDef {
    name: "tell",
    runtime_name: "rt_bytesio_tell",
    params: &[],
    return_type: TypeSpec::Int,
    min_args: 0,
    max_args: 0,
};

pub static BYTESIO_CLOSE: StdlibMethodDef = StdlibMethodDef {
    name: "close",
    runtime_name: "rt_bytesio_close",
    params: &[],
    return_type: TypeSpec::None,
    min_args: 0,
    max_args: 0,
};

/// io module definition
pub static IO_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "io",
    functions: &[IO_STRINGIO, IO_BYTESIO],
    attrs: &[],
    constants: &[],
    classes: &[],
    submodules: &[],
};
