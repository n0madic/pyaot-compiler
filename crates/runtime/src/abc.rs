//! ABC (Abstract Base Classes) module runtime functions

use crate::object::Obj;
use pyaot_core_defs::Value;

/// abc.abstractmethod(funcobj) -> funcobj
///
/// A no-op decorator that returns its argument unchanged.
/// The actual abstract method handling is done at compile time during decorator parsing.
/// This function exists only to allow `from abc import abstractmethod` imports.
pub fn rt_abc_abstractmethod(funcobj: *mut Obj) -> *mut Obj {
    // Simply return the function object unchanged
    funcobj
}
#[export_name = "rt_abc_abstractmethod"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_abc_abstractmethod_abi(funcobj: Value) -> Value {
    Value::from_ptr(rt_abc_abstractmethod(funcobj.unwrap_ptr()))
}
