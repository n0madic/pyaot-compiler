//! Live dict views: `dict.keys()` / `dict.values()` / `dict.items()`.
//!
//! **Runtime contract (added with the dict-view objects):** the three view
//! kinds are distinct heap tags (`TypeTagKind::DictKeys` / `DictValues` /
//! `DictItems`) sharing one layout, [`DictViewObj`] — a header plus a single
//! pointer to the viewed dict-family object. A view holds no snapshot; every
//! operation reads the dict's CURRENT contents, so a mutation of the dict
//! after `d.keys()` is visible through the view (CPython semantics).
//!
//! Entry points (all `Value`-ABI):
//! * `rt_dict_keys_view` / `rt_dict_values_view` / `rt_dict_items_view` —
//!   construct a view (the compiled `ContainerOp::DictKeys/Values/Items`).
//!
//! The generic tag-dispatched runtime seams route a view here: `len`
//! ([`view_len`]), truthiness, iteration ([`view_iter`] — keys reuse the
//! size-guarded dict key iterator, values/items walk the live dict through
//! `IteratorKind::DictValues`/`DictItems`), `in` ([`view_contains`]), `==` /
//! ordering ([`try_view_eq`] / [`try_view_cmp`]), the set operators on the
//! set-like keys/items views ([`try_view_setop`], always returning a fresh
//! `set`), `isdisjoint` ([`view_isdisjoint`]), and repr
//! (`conversions::dict_view_repr_string`).
//!
//! The internal list builders `rt_dict_keys`/`rt_dict_values`/`rt_dict_items`
//! (and their `Value`-ABI symbols) are unchanged — the runtime itself and the
//! stdlib `Counter.keys()` descriptors still use those list snapshots.

use crate::gc::{self, gc_pop, gc_push, ShadowFrame};
use crate::object::{DictObj, DictViewObj, IteratorKind, IteratorObj, Obj, TypeTagKind};
use pyaot_core_defs::Value;

/// Is `tag` one of the three dict-view tags?
#[inline]
pub(crate) fn is_view_tag(tag: TypeTagKind) -> bool {
    matches!(
        tag,
        TypeTagKind::DictKeys | TypeTagKind::DictValues | TypeTagKind::DictItems
    )
}

/// Is `tag` a set-like view (keys / items — CPython's `dict_keys` and
/// `dict_items` implement the `Set` ABC; `dict_values` does not)?
#[inline]
fn is_setlike_view_tag(tag: TypeTagKind) -> bool {
    matches!(tag, TypeTagKind::DictKeys | TypeTagKind::DictItems)
}

/// The heap tag of `obj`, or `None` for null / a tagged immediate.
///
/// # Safety
/// `obj` must be null, a tagged immediate, or a valid object pointer.
#[inline]
unsafe fn heap_tag(obj: *mut Obj) -> Option<TypeTagKind> {
    if obj.is_null() || !Value(obj as u64).is_ptr() {
        None
    } else {
        Some((*obj).type_tag())
    }
}

/// The viewed dict of a view object.
///
/// # Safety
/// `view` must be a live `DictViewObj`.
#[inline]
unsafe fn view_dict(view: *mut Obj) -> *mut Obj {
    (*(view as *mut DictViewObj)).dict
}

/// Allocate a view of `dict` with the given view `tag`.
fn make_view(dict: *mut Obj, tag: TypeTagKind) -> *mut Obj {
    // Root the dict across the allocation: the caller's dict may be an
    // unrooted temporary (`make_dict().keys()`).
    let mut roots: [*mut Obj; 1] = [dict];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    // SAFETY: `frame` lives on this stack frame until the matching gc_pop.
    unsafe { gc_push(&mut frame) };
    let obj = gc::gc_alloc(std::mem::size_of::<DictViewObj>(), tag as u8);
    gc_pop();
    unsafe {
        crate::debug_assert_dict_family!(roots[0], "dict view");
        (*(obj as *mut DictViewObj)).dict = roots[0];
    }
    obj
}

#[export_name = "rt_dict_keys_view"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_keys_view(dict: Value) -> Value {
    Value::from_ptr(make_view(dict.unwrap_ptr(), TypeTagKind::DictKeys))
}

#[export_name = "rt_dict_values_view"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_values_view(dict: Value) -> Value {
    Value::from_ptr(make_view(dict.unwrap_ptr(), TypeTagKind::DictValues))
}

#[export_name = "rt_dict_items_view"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_items_view(dict: Value) -> Value {
    Value::from_ptr(make_view(dict.unwrap_ptr(), TypeTagKind::DictItems))
}

/// Construct a view by tag (the gradual `rt_obj_method` `keys`/`values`/
/// `items` path).
pub(crate) fn dict_view_new(dict: *mut Obj, tag: TypeTagKind) -> *mut Obj {
    make_view(dict, tag)
}

/// `len(view)` — the live dict's entry count.
///
/// # Safety
/// `view` must be a live `DictViewObj`.
pub(crate) unsafe fn view_len(view: *mut Obj) -> i64 {
    (*(view_dict(view) as *mut DictObj)).len as i64
}

/// `iter(view)`.
///
/// # Safety
/// `view` must be a live `DictViewObj`.
pub(crate) unsafe fn view_iter(view: *mut Obj) -> *mut Obj {
    let tag = (*view).type_tag();
    let dict = view_dict(view);
    let kind = match tag {
        // Keys iterate exactly like the dict itself (size-guarded).
        TypeTagKind::DictKeys => return crate::iterator::rt_iter_dict(dict),
        TypeTagKind::DictValues => IteratorKind::DictValues,
        _ => IteratorKind::DictItems,
    };
    let mut roots: [*mut Obj; 1] = [dict];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    gc_push(&mut frame);
    let obj = gc::gc_alloc(
        std::mem::size_of::<IteratorObj>(),
        TypeTagKind::Iterator as u8,
    );
    gc_pop();
    let dict = roots[0];
    let iter = obj as *mut IteratorObj;
    (*iter).kind = kind as u8;
    (*iter).exhausted = false;
    (*iter).reversed = false;
    (*iter).source = dict;
    (*iter).index = 0;
    (*iter).range_stop = 0;
    (*iter).range_step = 0;
    (*iter).size_guard = dict;
    (*iter).expected_len = (*(dict as *mut DictObj)).len as i64;
    obj
}

/// One step of a live values/items iterator (`IteratorKind::DictValues` /
/// `DictItems`). Returns `None` when exhausted (the caller sets the flag and
/// applies its raise-or-sentinel protocol).
///
/// # Safety
/// `iter` must be a live `IteratorObj` of one of those two kinds.
pub(crate) unsafe fn iter_next_entry(iter: *mut IteratorObj, items: bool) -> Option<*mut Obj> {
    let dict = (*iter).source as *mut DictObj;
    // CPython parity: a size change since iteration began raises.
    if (*dict).len as i64 != (*iter).expected_len {
        crate::raise_exc!(
            crate::exceptions::ExceptionType::RuntimeError,
            "dictionary changed size during iteration"
        );
    }
    loop {
        let idx = (*iter).index as usize;
        if idx >= (*dict).entries_len {
            return None;
        }
        (*iter).index += 1;
        let entry = (*dict).entries.add(idx);
        if (*entry).key.0 == 0 {
            continue; // deleted slot
        }
        if !items {
            return Some((*entry).value.0 as *mut Obj);
        }
        // The tuple allocation may collect, but the key/value stay reachable
        // through the dict (rooted by the iterator), and the entries array is
        // not touched by a collection — re-read the entry after allocating.
        let tuple = crate::tuple::rt_make_tuple(2);
        let dict = (*iter).source as *mut DictObj;
        let entry = (*dict).entries.add(idx);
        crate::tuple::rt_tuple_set(tuple, 0, (*entry).key.0 as *mut Obj);
        crate::tuple::rt_tuple_set(tuple, 1, (*entry).value.0 as *mut Obj);
        return Some(tuple);
    }
}

/// `elem in view`.
///
/// # Safety
/// `view` must be a live `DictViewObj`.
pub(crate) unsafe fn view_contains(view: *mut Obj, elem: *mut Obj) -> i8 {
    let tag = (*view).type_tag();
    let dict = view_dict(view);
    match tag {
        TypeTagKind::DictKeys => crate::dict::rt_dict_contains(dict, elem),
        TypeTagKind::DictValues => {
            // Linear value-equality scan over the live entries. `rt_obj_eq`
            // may run a user `__eq__`, so re-read the dict each step and keep
            // both the view and the probe rooted.
            let mut roots: [*mut Obj; 2] = [view, elem];
            let mut frame = ShadowFrame {
                prev: std::ptr::null_mut(),
                nroots: 2,
                roots: roots.as_mut_ptr(),
            };
            gc_push(&mut frame);
            let mut i = 0usize;
            let mut found = 0;
            loop {
                let d = view_dict(roots[0]) as *mut DictObj;
                if i >= (*d).entries_len {
                    break;
                }
                let entry = (*d).entries.add(i);
                i += 1;
                if (*entry).key.0 == 0 {
                    continue;
                }
                if crate::ops::rt_obj_eq((*entry).value.0 as *mut Obj, roots[1]) != 0 {
                    found = 1;
                    break;
                }
            }
            gc_pop();
            found
        }
        _ => {
            // `(k, v) in d.items()` — a 2-tuple whose key is present and whose
            // value compares equal. Anything else is simply not contained.
            if heap_tag(elem) != Some(TypeTagKind::Tuple) {
                return 0;
            }
            let t = elem as *mut crate::object::TupleObj;
            if (*t).len != 2 {
                return 0;
            }
            let key = (*(*t).data.as_ptr()).0 as *mut Obj;
            let val = (*(*t).data.as_ptr().add(1)).0 as *mut Obj;
            if crate::dict::rt_dict_contains(dict, key) == 0 {
                return 0;
            }
            let stored = crate::dict::rt_dict_get(dict, key);
            crate::ops::rt_obj_eq(stored, val)
        }
    }
}

/// Collect any iterable into a fresh set (raising CPython's TypeError for a
/// non-iterable immediate). A dict view is collected through its own
/// iterator.
unsafe fn to_set(obj: *mut Obj) -> *mut Obj {
    if heap_tag(obj).is_none() {
        crate::raise_exc!(
            crate::exceptions::ExceptionType::TypeError,
            "'{}' object is not iterable",
            immediate_name(obj)
        );
    }
    crate::set::set_from_iterable(obj)
}

/// The Python type name of a null / tagged immediate operand.
fn immediate_name(obj: *mut Obj) -> &'static str {
    let v = Value(obj as u64);
    match v.primitive_type() {
        Some(t) => t.type_name(),
        None => "NoneType",
    }
}

/// The four set operators a set-like view supports.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewSetOp {
    And,
    Or,
    Sub,
    Xor,
}

/// `a <op> b` where at least one operand is a set-like (keys / items) view:
/// CPython's `dictviews_{and,or,sub,xor}` — the result is always a fresh
/// `set`, and the non-view operand may be any iterable. `&` puts the view on
/// the left (CPython swaps the operands); the others compute
/// `set(a) <op> b`. Returns `None` when neither operand is a set-like view
/// (the caller falls through to its normal dispatch).
///
/// # Safety
/// `a` and `b` must be null, tagged immediates, or valid object pointers.
pub(crate) unsafe fn try_view_setop(a: *mut Obj, b: *mut Obj, op: ViewSetOp) -> Option<*mut Obj> {
    let av = heap_tag(a).is_some_and(is_setlike_view_tag);
    let bv = heap_tag(b).is_some_and(is_setlike_view_tag);
    if !av && !bv {
        return None;
    }
    let (l, r) = if op == ViewSetOp::And && !av {
        (b, a)
    } else {
        (a, b)
    };
    let mut roots: [*mut Obj; 4] = [l, r, std::ptr::null_mut(), std::ptr::null_mut()];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 4,
        roots: roots.as_mut_ptr(),
    };
    gc_push(&mut frame);
    roots[2] = to_set(roots[0]);
    roots[3] = if crate::set::is_set_family(roots[1]) {
        roots[1]
    } else {
        to_set(roots[1])
    };
    let result = match op {
        ViewSetOp::And => crate::set::rt_set_intersection(roots[2], roots[3]),
        ViewSetOp::Or => crate::set::rt_set_union(roots[2], roots[3]),
        ViewSetOp::Sub => crate::set::rt_set_difference(roots[2], roots[3]),
        ViewSetOp::Xor => crate::set::rt_set_symmetric_difference(roots[2], roots[3]),
    };
    gc_pop();
    Some(result)
}

/// Is `tag` an operand a set-like view compares against (CPython: a
/// `set` / `frozenset` / keys view / items view)?
fn is_setlike_operand(tag: Option<TypeTagKind>) -> bool {
    matches!(
        tag,
        Some(
            TypeTagKind::Set
                | TypeTagKind::FrozenSet
                | TypeTagKind::DictKeys
                | TypeTagKind::DictItems
        )
    )
}

/// The element count of a set-like operand.
unsafe fn setlike_len(obj: *mut Obj) -> i64 {
    if is_view_tag((*obj).type_tag()) {
        view_len(obj)
    } else {
        crate::set::rt_set_len(obj)
    }
}

/// Is every element of `a` contained in `b`? (Both set-like operands.)
unsafe fn all_contained_in(a: *mut Obj, b: *mut Obj) -> bool {
    let mut roots: [*mut Obj; 3] = [a, b, std::ptr::null_mut()];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 3,
        roots: roots.as_mut_ptr(),
    };
    gc_push(&mut frame);
    roots[2] = crate::iterator::rt_iter_value_dyn(roots[0]);
    let mut ok = true;
    loop {
        let elem = crate::iterator::rt_iter_next_no_exc(roots[2]);
        if crate::iterator::rt_iter_is_exhausted(roots[2]) != 0 {
            break;
        }
        if crate::ops::rt_obj_contains(roots[1], elem) == 0 {
            ok = false;
            break;
        }
    }
    gc_pop();
    ok
}

/// `a == b` when either operand is a dict view: a values view compares by
/// identity; a keys/items view equals another set-like operand with the same
/// elements, and nothing else. `None` when neither operand is a view.
///
/// # Safety
/// `a` and `b` must be valid heap object pointers.
pub(crate) unsafe fn try_view_eq(a: *mut Obj, b: *mut Obj) -> Option<i8> {
    let (ta, tb) = ((*a).type_tag(), (*b).type_tag());
    if !is_view_tag(ta) && !is_view_tag(tb) {
        return None;
    }
    if ta == TypeTagKind::DictValues || tb == TypeTagKind::DictValues {
        return Some((a == b) as i8);
    }
    if !is_setlike_operand(Some(ta)) || !is_setlike_operand(Some(tb)) {
        return Some(0);
    }
    if setlike_len(a) != setlike_len(b) {
        return Some(0);
    }
    Some(all_contained_in(a, b) as i8)
}

/// `a < b` / `<=` / `>` / `>=` (`op_tag` 0..=3, the `rt_obj_cmp` encoding)
/// when either operand is a set-like view: subset / superset tests. A
/// comparison against a non-set-like operand (or with a values view) raises
/// CPython's TypeError. `None` when neither operand is a view.
///
/// # Safety
/// `a` and `b` must be null, tagged immediates, or valid object pointers.
pub(crate) unsafe fn try_view_cmp(a: *mut Obj, b: *mut Obj, op_tag: u8) -> Option<i8> {
    let (ta, tb) = (heap_tag(a), heap_tag(b));
    if !ta.is_some_and(is_view_tag) && !tb.is_some_and(is_view_tag) {
        return None;
    }
    if !is_setlike_operand(ta) || !is_setlike_operand(tb) {
        let sym = ["<", "<=", ">", ">="][(op_tag as usize).min(3)];
        let name = |t: Option<TypeTagKind>, o: *mut Obj| match t {
            Some(t) => crate::ops::comparison_type_name(t),
            None => immediate_name(o),
        };
        crate::raise_exc!(
            crate::exceptions::ExceptionType::TypeError,
            "'{}' not supported between instances of '{}' and '{}'",
            sym,
            name(ta, a),
            name(tb, b)
        );
    }
    let (la, lb) = (setlike_len(a), setlike_len(b));
    let r = match op_tag {
        0 => la < lb && all_contained_in(a, b),
        1 => la <= lb && all_contained_in(a, b),
        2 => la > lb && all_contained_in(b, a),
        _ => la >= lb && all_contained_in(b, a),
    };
    Some(r as i8)
}

/// `view.isdisjoint(other)` — True iff no element of the iterable `other` is
/// in the (set-like) view.
///
/// # Safety
/// `view` must be a live keys/items `DictViewObj`.
pub(crate) unsafe fn view_isdisjoint(view: *mut Obj, other: *mut Obj) -> i8 {
    if heap_tag(other).is_none() {
        crate::raise_exc!(
            crate::exceptions::ExceptionType::TypeError,
            "'{}' object is not iterable",
            immediate_name(other)
        );
    }
    let mut roots: [*mut Obj; 3] = [view, other, std::ptr::null_mut()];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 3,
        roots: roots.as_mut_ptr(),
    };
    gc_push(&mut frame);
    roots[2] = crate::iterator::rt_iter_value_dyn(roots[1]);
    let mut disjoint = 1;
    loop {
        let elem = crate::iterator::rt_iter_next_no_exc(roots[2]);
        if crate::iterator::rt_iter_is_exhausted(roots[2]) != 0 {
            break;
        }
        if view_contains(roots[0], elem) != 0 {
            disjoint = 0;
            break;
        }
    }
    gc_pop();
    disjoint
}
