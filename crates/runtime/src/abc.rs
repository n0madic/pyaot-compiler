//! ABC (Abstract Base Classes) module runtime functions

use crate::object::Obj;

/// abc.abstractmethod(funcobj) -> funcobj
///
/// A no-op decorator that returns its argument unchanged.
/// The actual abstract method handling is done at compile time during decorator parsing.
/// This function exists only to allow `from abc import abstractmethod` imports.
#[no_mangle]
pub extern "C" fn rt_abc_abstractmethod(funcobj: *mut Obj) -> *mut Obj {
    // Simply return the function object unchanged
    funcobj
}
