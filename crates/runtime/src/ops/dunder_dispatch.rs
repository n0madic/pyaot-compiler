//! Runtime dispatch of class binary-op dunders for Union arithmetic.
//!
//! When a `rt_obj_*` arithmetic helper receives a class instance as one of
//! its operands, primitive numeric extraction would either return zero
//! placeholders (see `extract_numeric_pair`) or raise a misleading
//! TypeError. CPython instead consults the operands' forward / reflected
//! dunders (`__add__` / `__radd__`, `__mul__` / `__rmul__`, etc.). This
//! module mirrors that protocol at runtime so polymorphic-`other`
//! parameters in user-defined dunders (e.g. `def __radd__(self, other)`
//! where the type planner widens `other` to `Union[Self, int, float, bool]`)
//! work correctly when called with another class instance.

use crate::object::{InstanceObj, Obj, TypeTagKind};
use crate::vtable::lookup_dunder_func;
use pyaot_core_defs::Value;

/// Compute FNV-1a hash at compile time. Must match
/// `pyaot_utils::fnv1a_hash` so runtime lookup matches compiler-registered
/// `METHOD_NAME_REGISTRY` entries.
const fn fnv1a(s: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    let mut i = 0;
    while i < s.len() {
        hash ^= s[i] as u64;
        hash = hash.wrapping_mul(0x100000001b3);
        i += 1;
    }
    hash
}

pub(super) const FNV_ADD: u64 = fnv1a(b"__add__");
pub(super) const FNV_RADD: u64 = fnv1a(b"__radd__");
pub(super) const FNV_SUB: u64 = fnv1a(b"__sub__");
pub(super) const FNV_RSUB: u64 = fnv1a(b"__rsub__");
pub(super) const FNV_MUL: u64 = fnv1a(b"__mul__");
pub(super) const FNV_RMUL: u64 = fnv1a(b"__rmul__");
pub(super) const FNV_TRUEDIV: u64 = fnv1a(b"__truediv__");
pub(super) const FNV_RTRUEDIV: u64 = fnv1a(b"__rtruediv__");
pub(super) const FNV_FLOORDIV: u64 = fnv1a(b"__floordiv__");
pub(super) const FNV_RFLOORDIV: u64 = fnv1a(b"__rfloordiv__");
pub(super) const FNV_MOD: u64 = fnv1a(b"__mod__");
pub(super) const FNV_RMOD: u64 = fnv1a(b"__rmod__");
pub(super) const FNV_POW: u64 = fnv1a(b"__pow__");
pub(super) const FNV_RPOW: u64 = fnv1a(b"__rpow__");

/// Uniform calling convention for all binary-op dunders. Every dunder is
/// called as `(self_obj, other_value) -> Value`. The `Value` return slot
/// can hold a tagged primitive (Int/Bool/None), a heap pointer (Class
/// instance, Float, Str, ...), or the `NotImplemented` singleton —
/// because all three fit in 64 bits and use the same return register on
/// the System V ABI. The actual Python return type only affects how the
/// caller interprets the bits, not the calling convention.
type DunderFn = unsafe extern "C" fn(*mut Obj, Value) -> Value;

#[inline]
unsafe fn is_instance(p: *mut Obj) -> bool {
    let v = Value(p as u64);
    if !v.is_ptr() || p.is_null() {
        return false;
    }
    (*p).type_tag() == TypeTagKind::Instance
}

#[inline]
unsafe fn is_not_implemented(v: Value) -> bool {
    if !v.is_ptr() {
        return false;
    }
    let p: *mut Obj = v.0 as *mut Obj;
    !p.is_null() && (*p).type_tag() == TypeTagKind::NotImplemented
}

/// Returns true if either operand is a class instance — caller should
/// route through `try_class_dunder` before attempting primitive numeric
/// dispatch.
#[inline]
pub(super) unsafe fn either_is_instance(a: *mut Obj, b: *mut Obj) -> bool {
    is_instance(a) || is_instance(b)
}

/// Try to dispatch a binary-op dunder for class instances following
/// CPython's protocol: forward dunder on `a` first, reflected dunder on
/// `b` as fallback (or only path if `a` isn't an instance).
///
/// Returns `Some(result)` when a dunder exists and returned a non-`NotImplemented`
/// value. Returns `None` when neither dunder is registered or both
/// returned `NotImplemented` — the caller is then responsible for
/// raising an appropriate `TypeError`.
#[inline]
pub(super) unsafe fn try_class_dunder(
    a: *mut Obj,
    b: *mut Obj,
    forward_hash: u64,
    reflected_hash: u64,
) -> Option<*mut Obj> {
    let va = Value(a as u64);
    let vb = Value(b as u64);

    // Forward: a.__op__(b)
    if is_instance(a) {
        let class_id = (*(a as *const InstanceObj)).class_id;
        let func_ptr = lookup_dunder_func(class_id, forward_hash);
        if !func_ptr.is_null() {
            let f: DunderFn = std::mem::transmute(func_ptr);
            let result = f(a, vb);
            if !is_not_implemented(result) {
                return Some(result.0 as *mut Obj);
            }
        }
    }

    // Reflected: b.__rop__(a)
    if is_instance(b) {
        let class_id = (*(b as *const InstanceObj)).class_id;
        let func_ptr = lookup_dunder_func(class_id, reflected_hash);
        if !func_ptr.is_null() {
            let f: DunderFn = std::mem::transmute(func_ptr);
            let result = f(b, va);
            if !is_not_implemented(result) {
                return Some(result.0 as *mut Obj);
            }
        }
    }

    None
}
