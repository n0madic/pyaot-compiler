//! `urllib.error` module — hosts stdlib-provided exception classes.
//!
//! `HTTPError` and `URLError` are declared as `StdlibExceptionClass`, each
//! with a reserved `class_id` in the stdlib-exception range. They are *not*
//! in Python's `builtins` namespace, so code must explicitly import them
//! (`from urllib.error import HTTPError`) before raising/catching — exactly
//! matching CPython semantics. At runtime they use the same class_id-based
//! catch machinery as user-defined Exception subclasses, with their parent
//! `OSError` registered so `except OSError:` captures them.

use crate::types::{StdlibExceptionClass, StdlibModuleDef};
use pyaot_core_defs::{BuiltinExceptionKind, BUILTIN_EXCEPTION_COUNT};

/// `urllib.error.URLError` — raised on lower-level request failures
/// (connection refused, DNS resolution errors, etc.). Subclass of OSError.
pub static URL_ERROR: StdlibExceptionClass = StdlibExceptionClass {
    name: "URLError",
    class_id: BUILTIN_EXCEPTION_COUNT, // first reserved stdlib slot
    parent: BuiltinExceptionKind::OSError,
    module: "urllib.error",
};

/// `urllib.error.HTTPError` — raised by CPython's `urlopen` on 4xx/5xx.
/// Subclass of URLError (which itself subclasses OSError). We model
/// directly as subclass of OSError for now; full URLError-as-parent can be
/// added once we allow stdlib exceptions as parents.
pub static HTTP_ERROR: StdlibExceptionClass = StdlibExceptionClass {
    name: "HTTPError",
    class_id: BUILTIN_EXCEPTION_COUNT + 1,
    parent: BuiltinExceptionKind::OSError,
    module: "urllib.error",
};

/// `urllib.error` module — surfaces HTTPError and URLError for import.
pub static URLLIB_ERROR_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "urllib.error",
    functions: &[],
    attrs: &[],
    constants: &[],
    classes: &[],
    exceptions: &[&URL_ERROR, &HTTP_ERROR],
    submodules: &[],
};
