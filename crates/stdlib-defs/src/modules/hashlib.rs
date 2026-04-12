//! hashlib module definition
//!
//! Provides cryptographic hash functions (md5, sha1, sha256).

use crate::types::{
    LoweringHints, ParamDef, StdlibClassDef, StdlibFunctionDef, StdlibMethodDef, StdlibModuleDef,
    TypeSpec,
};
#[allow(unused_imports)]
use pyaot_core_defs::runtime_func_def::{P_F64, P_I64, P_I8, R_F64, R_I64, R_I8};
use pyaot_core_defs::RuntimeFuncDef;

/// hashlib.md5(data) -> Hash object
pub static HASHLIB_MD5: StdlibFunctionDef = StdlibFunctionDef {
    name: "md5",
    runtime_name: "rt_hashlib_md5",
    params: &[ParamDef::required("data", TypeSpec::Bytes)],
    return_type: TypeSpec::Hash,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
    codegen: RuntimeFuncDef::new("rt_hashlib_md5", &[P_I64], Some(R_I64), false),
};

/// hashlib.sha1(data) -> Hash object
pub static HASHLIB_SHA1: StdlibFunctionDef = StdlibFunctionDef {
    name: "sha1",
    runtime_name: "rt_hashlib_sha1",
    params: &[ParamDef::required("data", TypeSpec::Bytes)],
    return_type: TypeSpec::Hash,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
    codegen: RuntimeFuncDef::new("rt_hashlib_sha1", &[P_I64], Some(R_I64), false),
};

/// hashlib.sha256(data) -> Hash object
pub static HASHLIB_SHA256: StdlibFunctionDef = StdlibFunctionDef {
    name: "sha256",
    runtime_name: "rt_hashlib_sha256",
    params: &[ParamDef::required("data", TypeSpec::Bytes)],
    return_type: TypeSpec::Hash,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
    codegen: RuntimeFuncDef::new("rt_hashlib_sha256", &[P_I64], Some(R_I64), false),
};

/// Hash.hexdigest() method
pub static HASH_HEXDIGEST: StdlibMethodDef = StdlibMethodDef {
    name: "hexdigest",
    runtime_name: "rt_hash_hexdigest",
    params: &[],
    return_type: TypeSpec::Str,
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::new("rt_hash_hexdigest", &[P_I64], Some(R_I64), false),
};

/// Hash.digest() method
pub static HASH_DIGEST: StdlibMethodDef = StdlibMethodDef {
    name: "digest",
    runtime_name: "rt_hash_digest",
    params: &[],
    return_type: TypeSpec::Bytes,
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::new("rt_hash_digest", &[P_I64], Some(R_I64), false),
};

/// Hash class definition
pub static HASH_CLASS: StdlibClassDef = StdlibClassDef {
    name: "Hash",
    methods: &[HASH_HEXDIGEST, HASH_DIGEST],
    type_spec: Some(TypeSpec::Hash),
};

/// hashlib module definition
pub static HASHLIB_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "hashlib",
    functions: &[HASHLIB_MD5, HASHLIB_SHA1, HASHLIB_SHA256],
    attrs: &[],
    constants: &[],
    classes: &[HASH_CLASS],
    exceptions: &[],
    submodules: &[],
};
