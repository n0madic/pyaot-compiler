//! Standard library module definitions
//!
//! Each submodule defines a stdlib module with its functions, attributes, and constants.

pub mod abc;
pub mod base64_mod;
pub mod copy;
pub mod functools;
pub mod hashlib;
pub mod io;
pub mod itertools;
pub mod json;
pub mod math;
pub mod os;
pub mod random;
pub mod re;
pub mod string;
pub mod subprocess;
pub mod sys;
pub mod time;
pub mod urllib;

use crate::types::StdlibModuleDef;

/// All registered stdlib modules
pub static ALL_MODULES: &[&StdlibModuleDef] = &[
    &abc::ABC_MODULE,
    &sys::SYS_MODULE,
    &os::OS_MODULE,
    &os::OS_PATH_MODULE,
    &re::RE_MODULE,
    &json::JSON_MODULE,
    &math::MATH_MODULE,
    &time::TIME_MODULE,
    &subprocess::SUBPROCESS_MODULE,
    &urllib::URLLIB_MODULE,
    &urllib::URLLIB_PARSE_MODULE,
    &urllib::URLLIB_REQUEST_MODULE,
    &string::STRING_MODULE,
    &random::RANDOM_MODULE,
    &hashlib::HASHLIB_MODULE,
    &base64_mod::BASE64_MODULE,
    &copy::COPY_MODULE,
    &functools::FUNCTOOLS_MODULE,
    &itertools::ITERTOOLS_MODULE,
    &io::IO_MODULE,
];

/// Get a module definition by name (supports dotted names like "os.path")
pub fn get_module(name: &str) -> Option<&'static StdlibModuleDef> {
    ALL_MODULES
        .iter()
        .find(|module| module.name == name)
        .copied()
}

/// Check if a module name is a known stdlib module
pub fn is_stdlib_module(name: &str) -> bool {
    // Check full name
    if get_module(name).is_some() {
        return true;
    }
    // Check if it's a root module - derive from ALL_MODULES (DRY)
    let root = name.split('.').next().unwrap_or(name);
    ALL_MODULES
        .iter()
        .any(|m| m.name.split('.').next().unwrap_or(m.name) == root)
}

/// Get the root module name from a potentially dotted path
pub fn get_root_module(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}
