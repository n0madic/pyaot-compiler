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
pub(crate) const fn fnv1a(s: &[u8]) -> u64 {
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
pub(super) const FNV_MATMUL: u64 = fnv1a(b"__matmul__");
pub(super) const FNV_RMATMUL: u64 = fnv1a(b"__rmatmul__");
pub(super) const FNV_TRUEDIV: u64 = fnv1a(b"__truediv__");
pub(super) const FNV_RTRUEDIV: u64 = fnv1a(b"__rtruediv__");
pub(super) const FNV_FLOORDIV: u64 = fnv1a(b"__floordiv__");
pub(super) const FNV_RFLOORDIV: u64 = fnv1a(b"__rfloordiv__");
pub(super) const FNV_MOD: u64 = fnv1a(b"__mod__");
pub(super) const FNV_RMOD: u64 = fnv1a(b"__rmod__");
pub(super) const FNV_POW: u64 = fnv1a(b"__pow__");
pub(super) const FNV_RPOW: u64 = fnv1a(b"__rpow__");
pub(super) const FNV_NEG: u64 = fnv1a(b"__neg__");
pub(super) const FNV_POS: u64 = fnv1a(b"__pos__");
pub(super) const FNV_INVERT: u64 = fnv1a(b"__invert__");
pub(super) const FNV_ABS: u64 = fnv1a(b"__abs__");
pub(super) const FNV_INT: u64 = fnv1a(b"__int__");
pub(super) const FNV_FLOAT: u64 = fnv1a(b"__float__");
pub(super) const FNV_REPR: u64 = fnv1a(b"__repr__");
pub(super) const FNV_STR: u64 = fnv1a(b"__str__");
pub(super) const FNV_FORMAT: u64 = fnv1a(b"__format__");
pub(super) const FNV_LT: u64 = fnv1a(b"__lt__");
pub(super) const FNV_HASH: u64 = fnv1a(b"__hash__");
pub(super) const FNV_ITER: u64 = fnv1a(b"__iter__");
pub(super) const FNV_BOOL: u64 = fnv1a(b"__bool__");
pub(super) const FNV_LEN: u64 = fnv1a(b"__len__");
pub(super) const FNV_AND: u64 = fnv1a(b"__and__");
pub(super) const FNV_RAND: u64 = fnv1a(b"__rand__");
pub(super) const FNV_OR: u64 = fnv1a(b"__or__");
pub(super) const FNV_ROR: u64 = fnv1a(b"__ror__");
pub(super) const FNV_XOR: u64 = fnv1a(b"__xor__");
pub(super) const FNV_RXOR: u64 = fnv1a(b"__rxor__");
pub(super) const FNV_LSHIFT: u64 = fnv1a(b"__lshift__");
pub(super) const FNV_RLSHIFT: u64 = fnv1a(b"__rlshift__");
pub(super) const FNV_RSHIFT: u64 = fnv1a(b"__rshift__");
pub(super) const FNV_RRSHIFT: u64 = fnv1a(b"__rrshift__");

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

    // CPython §3.3.8 subclass-first reflected rule: if both operands are
    // instances and `type(b)` is a PROPER subclass of `type(a)` whose reflected
    // method is OVERRIDDEN (differs from `a`'s reflected slot), try
    // `b.__rop__(a)` BEFORE `a.__op__(b)` — the derived class handles the op.
    if is_instance(a) && is_instance(b) {
        let class_a = (*(a as *const InstanceObj)).class_id;
        let class_b = (*(b as *const InstanceObj)).class_id;
        if class_a != class_b && crate::vtable::rt_class_inherits_from(class_b, class_a) != 0 {
            let refl_b = lookup_dunder_func(class_b, reflected_hash);
            let refl_a = lookup_dunder_func(class_a, reflected_hash);
            if !refl_b.is_null() && refl_b != refl_a {
                let f: DunderFn = std::mem::transmute(refl_b);
                let result = f(b, va);
                if !is_not_implemented(result) {
                    return Some(result.0 as *mut Obj);
                }
            }
        }
    }

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

/// Try to dispatch a unary-op dunder for class instances. Returns
/// `Some(result)` when a dunder is registered (regardless of whether it
/// returns NotImplemented — unary dunders rarely do, so we surface the
/// returned Value verbatim). Returns `None` when the dunder isn't
/// registered, so the caller can fall back to primitive negation.
#[inline]
pub(super) unsafe fn try_class_unary_dunder(a: *mut Obj, dunder_hash: u64) -> Option<*mut Obj> {
    if !is_instance(a) {
        return None;
    }
    let class_id = (*(a as *const InstanceObj)).class_id;
    let func_ptr = lookup_dunder_func(class_id, dunder_hash);
    if func_ptr.is_null() {
        return None;
    }
    type UnaryDunderFn = unsafe extern "C" fn(*mut Obj) -> Value;
    let f: UnaryDunderFn = std::mem::transmute(func_ptr);
    let result = f(a);
    Some(result.0 as *mut Obj)
}

/// Dispatch `__int__` for `int(obj)` when `obj` is a class instance.
/// Returns the boxed dunder result (a tagged `Value`, typically an Int);
/// `None` when the instance has no `__int__`. Mirrors `rt_obj_neg`'s use of
/// `try_class_unary_dunder` so `int()`/`float()` follow CPython's protocol.
///
/// # Safety
/// `obj` must be a valid object pointer (the caller verifies `is_ptr` and the
/// Instance type tag before calling).
pub unsafe fn try_int_dunder(obj: *mut Obj) -> Option<*mut Obj> {
    try_class_unary_dunder(obj, FNV_INT)
}

/// Dispatch `iter(obj)` / `for x in obj` for a class instance via its
/// `__iter__` dunder (lazy user-class iterator protocol). Returns the boxed
/// `__iter__()` result (typically the iterator instance, or `self` for a
/// self-iterator); `None` when the instance defines no `__iter__` (the caller
/// then raises `TypeError: object is not iterable`).
///
/// # Safety
/// `obj` must be a valid object pointer; this fn re-checks the `Instance` tag.
pub unsafe fn try_iter_dunder(obj: *mut Obj) -> Option<*mut Obj> {
    try_class_unary_dunder(obj, FNV_ITER)
}

/// Evaluate truthiness of a class instance per CPython's protocol: `__bool__`
/// if defined (returns bool), else `__len__() != 0` if defined, else `True`.
/// Returns `None` when the instance defines neither dunder (the caller defaults
/// to truthy). Used by `rt_is_truthy`'s Instance arm so `bool(inst)` / `if inst`
/// / `not inst` honour a user `__bool__`/`__len__`.
///
/// Both dunders return through the Tagged baseline (the registered dunder
/// pointer's return ABI is a tagged `Value` — the "dunder-return-Tagged" policy,
/// the same ABI `try_int_dunder` reads for `__int__`): `__bool__ -> bool` boxes
/// to a tagged bool, `__len__ -> int` to a tagged int. `rt_is_truthy` then
/// interprets the tagged result correctly (the bool directly, the int as
/// `!= 0`).
///
/// # Safety
/// `obj` must be a valid object pointer; this fn re-checks the `Instance` tag.
pub(super) unsafe fn try_bool_dunder(obj: *mut Obj) -> Option<bool> {
    if !is_instance(obj) {
        return None;
    }
    let class_id = (*(obj as *const InstanceObj)).class_id;
    type DunderFn = unsafe extern "C" fn(*mut Obj) -> Value;
    // CPython priority: `__bool__` first, then `__len__() != 0`.
    for &hash in &[FNV_BOOL, FNV_LEN] {
        let func_ptr = lookup_dunder_func(class_id, hash);
        if !func_ptr.is_null() {
            let f: DunderFn = std::mem::transmute(func_ptr);
            return Some(crate::ops::rt_is_truthy(f(obj).0 as *mut Obj) != 0);
        }
    }
    None
}

/// Dispatch `__hash__` for `hash(obj)` when `obj` is a class instance. Returns
/// the boxed dunder result (a tagged int `Value` as `*mut Obj`); `None` when the
/// instance has no `__hash__` (the caller then raises `TypeError: unhashable
/// type`, matching CPython for a class that suppresses hashing).
///
/// # Safety
/// See [`try_int_dunder`].
pub unsafe fn try_hash_dunder(obj: *mut Obj) -> Option<*mut Obj> {
    try_class_unary_dunder(obj, FNV_HASH)
}

/// Dispatch `__float__` for `float(obj)` when `obj` is a class instance.
/// Returns the boxed dunder result (typically a boxed `FloatObj` pointer);
/// `None` when the instance has no `__float__`.
///
/// # Safety
/// See [`try_int_dunder`].
pub unsafe fn try_float_dunder(obj: *mut Obj) -> Option<*mut Obj> {
    try_class_unary_dunder(obj, FNV_FLOAT)
}

/// Dispatch `__repr__` for a class instance encountered while rendering a
/// container's repr (e.g. `print([P(1)])` / `str({P(1)})`). Returns the
/// resulting `StrObj` pointer, or `None` when the instance's class defines
/// no `__repr__` (the caller then falls back to the default object repr).
///
/// Mirrors CPython: a container's repr uses each element's `repr()`
/// (`type(elem).__repr__`), NOT `__str__`. The top-level `print(instance)`
/// path dispatches `__str__`/`__repr__` at lowering time, but container
/// elements are rendered by the runtime, which has no static class type —
/// so it must dispatch through `DUNDER_FUNC_REGISTRY` here.
///
/// # Safety
/// `obj` must be a valid object pointer (the caller is rendering it as a
/// container element; this fn re-checks the `Instance` type tag).
pub unsafe fn try_repr_dunder(obj: *mut Obj) -> Option<*mut Obj> {
    try_class_unary_dunder(obj, FNV_REPR)
}

/// Dispatch `str(self)` for a class instance via its `__str__`, falling back to
/// `__repr__` (CPython: `str()` defaults to `__repr__`). Used by `rt_format`'s
/// empty-spec path when an instance has no `__format__` — `object.__format__`
/// with an empty spec returns `str(self)`. Returns `None` when the instance
/// defines neither (the caller then renders the default object repr).
///
/// # Safety
/// `obj` must be a valid object pointer; this fn re-checks the `Instance` tag.
pub unsafe fn try_str_dunder(obj: *mut Obj) -> Option<*mut Obj> {
    try_class_unary_dunder(obj, FNV_STR).or_else(|| try_class_unary_dunder(obj, FNV_REPR))
}

/// Dispatch `abs(self)` for a class instance via its `__abs__` dunder (§6 —
/// `abs(UnaryNum(-5))`). Returns `None` when the instance defines no `__abs__`
/// (the caller then raises `TypeError`, CPython's behavior).
///
/// # Safety
/// `obj` must be a valid object pointer; this fn re-checks the `Instance` tag.
pub unsafe fn try_abs_dunder(obj: *mut Obj) -> Option<*mut Obj> {
    try_class_unary_dunder(obj, FNV_ABS)
}

/// Dispatch `value.__format__(spec)` for a class instance (`f"{p:spec}"` ≡
/// `format(p, "spec")` ≡ `type(p).__format__(p, "spec")`). `spec` is a `StrObj`
/// pointer (an empty string for `f"{p}"`). Returns the dunder's `str` result, or
/// `None` when the instance's class defines no `__format__` (the caller then
/// emulates `object.__format__`). The reflected slot is unused — only the
/// receiver is ever a class instance here.
///
/// # Safety
/// `obj` must be a valid object pointer; `spec` a valid `StrObj` pointer. This
/// fn re-checks the `Instance` tag on `obj`.
pub unsafe fn try_format_dunder(obj: *mut Obj, spec: *mut Obj) -> Option<*mut Obj> {
    try_class_dunder(obj, spec, FNV_FORMAT, 0)
}

/// Order two class instances for sorting via their `__lt__` dunder. CPython
/// sorts using `<` only: a truthy `a.__lt__(b)` → `Less`; else a truthy
/// `b.__lt__(a)` → `Greater`; else `Equal`. Returns `None` when the
/// instances' class defines no `__lt__` (the caller then falls back to a
/// stable address ordering, matching the prior behaviour).
///
/// The runtime sort comparator (`compare_list_elements`) needs this because
/// it has no static class type — `min`/`max` over class elements dispatch
/// `__lt__` at lowering time, but `sorted`/`list.sort` compare elements in
/// the runtime.
///
/// **Calling convention:** unlike the arithmetic dunders (which return a
/// tagged `Value`), a comparison dunder `__lt__ -> bool` returns a **raw
/// `i8`** (0/1) — the registry stores the method's native ABI, and a
/// `bool`-typed return is not boxed. So this dispatches through an
/// `i8`-returning fn pointer, not the `Value`-returning [`DunderFn`].
///
/// # Safety
/// `a` and `b` must be valid object pointers; this fn re-checks the
/// `Instance` type tag on both.
pub unsafe fn try_instance_lt_ordering(a: *mut Obj, b: *mut Obj) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    type LtFn = unsafe extern "C" fn(*mut Obj, Value) -> i8;

    if !is_instance(a) || !is_instance(b) {
        return None;
    }
    let class_a = (*(a as *const InstanceObj)).class_id;
    let lt_a = lookup_dunder_func(class_a, FNV_LT);
    if lt_a.is_null() {
        return None;
    }
    let fa: LtFn = std::mem::transmute(lt_a);
    if fa(a, Value(b as u64)) != 0 {
        return Some(Ordering::Less);
    }
    let class_b = (*(b as *const InstanceObj)).class_id;
    let lt_b = lookup_dunder_func(class_b, FNV_LT);
    if !lt_b.is_null() {
        let fb: LtFn = std::mem::transmute(lt_b);
        if fb(b, Value(a as u64)) != 0 {
            return Some(Ordering::Greater);
        }
    }
    Some(Ordering::Equal)
}
