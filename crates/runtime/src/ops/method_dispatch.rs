//! Gradual-completeness method dispatch for a `Dyn`/`Union`-typed receiver.
//!
//! `rt_obj_method` is the runtime analogue of CPython's `type(obj).method`
//! resolution: a `recv.method(args)` call whose receiver type the front-half
//! could not pin to a concrete shape lowers to ONE call here, and we decide by
//! the receiver's runtime tag.
//!
//! * A **container** receiver (`list` / `dict` / `set` / `deque`, plus the
//!   `DefaultDict` / `Counter` dict-family aliases) routes to the EXISTING
//!   typed `rt_list_*` / `rt_dict_*` / `rt_set_*` / `rt_deque_*` family — the
//!   same functions the statically-typed `ContainerMethod` path calls (Phase
//!   A). Positional arguments ride a `tuple[Tagged]` (`args_tuple`); we unpack
//!   exactly the slots a given method needs.
//! * An **`Instance`** receiver routes through the method's uniform thunk
//!   (`METHOD_UNIFORM_REGISTRY`), the fixed `(self, __args__, __kwargs__)`
//!   ABI — so an arbitrary user method works on any `Dyn` receiver (Phase B).
//! * Anything else (an immediate, or a tag with no such method) raises
//!   `AttributeError`, matching CPython's failure mode.

use crate::object::{Obj, TypeTagKind};
use crate::ops::comparison::type_name;
use crate::ops::dunder_dispatch::fnv1a;
use crate::vtable::lookup_method_uniform;
use pyaot_core_defs::Value;

// ── FNV-1a hashes of the dispatched method names (must match the compiler's
//    `pyaot_utils::fnv1a_hash`, which lowering uses to materialize `name_hash`). ──

// list
const H_APPEND: u64 = fnv1a(b"append");
const H_EXTEND: u64 = fnv1a(b"extend");
const H_POP: u64 = fnv1a(b"pop");
const H_INSERT: u64 = fnv1a(b"insert");
const H_REMOVE: u64 = fnv1a(b"remove");
const H_CLEAR: u64 = fnv1a(b"clear");
const H_COPY: u64 = fnv1a(b"copy");
const H_REVERSE: u64 = fnv1a(b"reverse");
const H_INDEX: u64 = fnv1a(b"index");
const H_COUNT: u64 = fnv1a(b"count");
const H_SORT: u64 = fnv1a(b"sort");
// dict
const H_GET: u64 = fnv1a(b"get");
const H_SETDEFAULT: u64 = fnv1a(b"setdefault");
const H_KEYS: u64 = fnv1a(b"keys");
const H_VALUES: u64 = fnv1a(b"values");
const H_ITEMS: u64 = fnv1a(b"items");
const H_UPDATE: u64 = fnv1a(b"update");
// set
const H_ADD: u64 = fnv1a(b"add");
const H_DISCARD: u64 = fnv1a(b"discard");
// deque
const H_APPENDLEFT: u64 = fnv1a(b"appendleft");
const H_POPLEFT: u64 = fnv1a(b"popleft");
const H_EXTENDLEFT: u64 = fnv1a(b"extendleft");
const H_ROTATE: u64 = fnv1a(b"rotate");
// int / bool
const H_BIT_LENGTH: u64 = fnv1a(b"bit_length");
const H_BIT_COUNT: u64 = fnv1a(b"bit_count");
const H_CONJUGATE: u64 = fnv1a(b"conjugate");
const H_DUNDER_INDEX: u64 = fnv1a(b"__index__");

/// The uniform method-thunk ABI: `(self, __args__, __kwargs__) -> Value`. Fixed
/// regardless of the method's source arity / parameter reprs, so a
/// `transmute`-and-call is sound (unlike `rt_vtable_lookup_by_name`, whose ptr
/// is the method's *native* ABI).
type UniformThunk = unsafe extern "C" fn(Value, Value, Value) -> Value;

/// Number of positional arguments packed into `args_tuple`.
#[inline]
unsafe fn argc(args: *mut Obj) -> i64 {
    crate::tuple::rt_tuple_len(args)
}

/// Positional argument `i` as a tagged `Value` (raw tuple-slot bits).
#[inline]
unsafe fn arg(args: *mut Obj, i: i64) -> Value {
    Value(crate::tuple::rt_tuple_get(args, i) as u64)
}

/// Unbox a tagged-int (or bool) argument to a machine `i64` index. A non-int
/// argument yields 0 — the gradual contract treats an out-of-domain index
/// leniently rather than mis-reading pointer bits.
#[inline]
fn as_index(v: Value) -> i64 {
    if v.is_int() {
        v.unwrap_int()
    } else if v.is_bool() {
        v.unwrap_bool() as i64
    } else {
        0
    }
}

/// Raw tagged bits of a `Value` as the `*mut Obj` the typed runtime fns expect
/// (the runtime carries every `Value` through the `*mut Obj` ABI; a tagged
/// immediate survives as its bits — see `rt_dict_get_abi`).
#[inline]
fn bits(v: Value) -> *mut Obj {
    v.0 as *mut Obj
}

/// `AttributeError` for a receiver tag that has no such method. The attribute
/// name is only available as its FNV hash here, so the message names the type;
/// the corpus is byte-exact on stdout, and a resolvable method never reaches
/// this arm.
unsafe fn raise_no_attr(tname: &str) -> ! {
    crate::raise_exc!(
        crate::exceptions::ExceptionType::AttributeError,
        "'{}' object has no attribute (gradual method dispatch)",
        tname
    )
}

/// Dispatch `recv.method(args, kwargs)` by the receiver's runtime tag.
///
/// `name_hash` is the RAW FNV-1a hash of the method name; `args_tuple` is a
/// `tuple[Tagged]` of the positional args; `kwargs` is a `dict[str, Tagged]`
/// or the null sentinel. The result rides the tagged baseline (the caller
/// GC-roots it).
#[no_mangle]
pub extern "C" fn rt_obj_method(
    recv: Value,
    name_hash: i64,
    args_tuple: Value,
    kwargs: Value,
) -> Value {
    let h = name_hash as u64;
    let at = args_tuple.0 as *mut Obj;
    // An immediate int/bool receiver (`(5).bit_length()`, `True.bit_count()`):
    // dispatch by Value BEFORE the pointer guard (an immediate is not a ptr).
    if recv.is_int() || recv.is_bool() {
        return unsafe { int_method(recv, h, at) };
    }
    let recv_ptr = recv.0 as *mut Obj;
    // A null pointer / `None` has no dispatchable method.
    if !recv.is_ptr() || recv_ptr.is_null() {
        let tname = match recv.primitive_type() {
            Some(t) => t.type_name(),
            None => "NoneType",
        };
        unsafe { raise_no_attr(tname) }
    }
    unsafe {
        let tag = (*recv_ptr).type_tag();
        match tag {
            TypeTagKind::List => list_method(recv_ptr, h, at, kwargs, tag),
            TypeTagKind::Dict | TypeTagKind::DefaultDict | TypeTagKind::Counter => {
                dict_method(recv_ptr, h, at, tag)
            }
            TypeTagKind::Set => set_method(recv_ptr, h, at, tag),
            TypeTagKind::Deque => deque_method(recv_ptr, h, at, tag),
            TypeTagKind::Tuple => tuple_method(recv_ptr, h, at, tag),
            // A heap bignum int (`rt_int_*` are bignum-aware and take the Value).
            TypeTagKind::BigInt => int_method(recv, h, at),
            TypeTagKind::Instance => instance_method(recv, name_hash, args_tuple, kwargs),
            other => raise_no_attr(type_name(other)),
        }
    }
}

/// `tuple` value-comparing queries on a `Dyn` receiver (§9 sibling). `index`
/// raises `ValueError` on a miss (CPython); `count` returns 0.
unsafe fn tuple_method(recv: *mut Obj, h: u64, at: *mut Obj, tag: TypeTagKind) -> Value {
    let n = argc(at);
    match h {
        H_INDEX if n == 1 => Value::from_int(crate::tuple::rt_tuple_index(recv, bits(arg(at, 0)))),
        H_COUNT if n == 1 => Value::from_int(crate::tuple::rt_tuple_count(recv, bits(arg(at, 0)))),
        _ => raise_no_attr(type_name(tag)),
    }
}

/// `int` / `bool` methods on a `Dyn` receiver (§9): `bit_length`/`bit_count`
/// (bignum-aware counts), `conjugate`/`__index__` (the int value itself; a bool
/// widens to int). `recv` is the tagged int Value (fixnum, bool, or heap
/// bignum) — the `rt_int_*` helpers accept all three.
unsafe fn int_method(recv: Value, h: u64, at: *mut Obj) -> Value {
    let n = argc(at);
    match h {
        H_BIT_LENGTH if n == 0 => Value::from_int(crate::math_ops::rt_int_bit_length(recv)),
        H_BIT_COUNT if n == 0 => Value::from_int(crate::math_ops::rt_int_bit_count(recv)),
        H_CONJUGATE if n == 0 => crate::math_ops::rt_int_index(recv),
        H_DUNDER_INDEX if n == 0 => crate::math_ops::rt_int_index(recv),
        _ => raise_no_attr("int"),
    }
}

/// Read a boolean keyword argument from the `kwargs` dict (the null sentinel or
/// a `dict[str, Tagged]`). Absent / null ⇒ `false`. Used by the `Dyn`-receiver
/// `list.sort(reverse=…)` form, whose keyword the container branch must honor.
unsafe fn kwarg_truthy(kwargs: Value, key: &[u8]) -> bool {
    if !kwargs.is_ptr() {
        return false;
    }
    let d = kwargs.0 as *mut Obj;
    if d.is_null() {
        return false;
    }
    let key_obj = crate::string::rt_make_str(key.as_ptr(), key.len());
    let v = crate::dict::rt_dict_get(d, key_obj);
    if v.is_null() {
        return false;
    }
    crate::ops::rt_is_truthy(v) != 0
}

unsafe fn list_method(
    recv: *mut Obj,
    h: u64,
    at: *mut Obj,
    kwargs: Value,
    tag: TypeTagKind,
) -> Value {
    let n = argc(at);
    match h {
        H_APPEND if n == 1 => {
            crate::list::rt_list_append(recv, bits(arg(at, 0)));
            Value::NONE
        }
        H_EXTEND if n == 1 => {
            crate::list::rt_list_extend(recv, bits(arg(at, 0)));
            Value::NONE
        }
        H_POP => {
            let idx = if n >= 1 { as_index(arg(at, 0)) } else { -1 };
            Value(crate::list::rt_list_pop(recv, idx) as u64)
        }
        H_INSERT if n == 2 => {
            crate::list::rt_list_insert(recv, as_index(arg(at, 0)), bits(arg(at, 1)));
            Value::NONE
        }
        H_REMOVE if n == 1 => {
            crate::list::rt_list_remove(recv, bits(arg(at, 0)));
            Value::NONE
        }
        H_CLEAR if n == 0 => {
            crate::list::rt_list_clear(recv);
            Value::NONE
        }
        H_COPY if n == 0 => Value(crate::list::rt_list_copy(recv) as u64),
        H_REVERSE if n == 0 => {
            crate::list::rt_list_reverse(recv);
            Value::NONE
        }
        H_INDEX if n == 1 => Value::from_int(crate::list::rt_list_index(recv, bits(arg(at, 0)))),
        H_COUNT if n == 1 => Value::from_int(crate::list::rt_list_count(recv, bits(arg(at, 0)))),
        H_SORT if n == 0 => {
            // No-key form. `key=` on a `Dyn` receiver is already handled upstream
            // by the frontend `sort` desugar (type-blind `ListSortByKeys`); only
            // a `reverse=`-only call reaches here, so honor that keyword.
            let reverse = kwarg_truthy(kwargs, b"reverse") as i8;
            crate::list::rt_list_sort(recv, reverse);
            Value::NONE
        }
        _ => raise_no_attr(type_name(tag)),
    }
}

unsafe fn dict_method(recv: *mut Obj, h: u64, at: *mut Obj, tag: TypeTagKind) -> Value {
    let n = argc(at);
    match h {
        H_GET if n == 1 => Value(crate::dict::rt_dict_get_default(
            recv,
            bits(arg(at, 0)),
            Value::NONE.0 as *mut Obj,
        ) as u64),
        H_GET if n == 2 => {
            Value(crate::dict::rt_dict_get_default(recv, bits(arg(at, 0)), bits(arg(at, 1))) as u64)
        }
        H_SETDEFAULT if n == 1 => Value(crate::dict::rt_dict_setdefault(
            recv,
            bits(arg(at, 0)),
            Value::NONE.0 as *mut Obj,
        ) as u64),
        H_SETDEFAULT if n == 2 => {
            Value(crate::dict::rt_dict_setdefault(recv, bits(arg(at, 0)), bits(arg(at, 1))) as u64)
        }
        H_POP if n == 1 => {
            let r = crate::dict::rt_dict_pop(recv, bits(arg(at, 0)));
            if r.is_null() {
                crate::raise_exc!(
                    crate::exceptions::ExceptionType::KeyError,
                    "pop(): key not found"
                );
            }
            Value(r as u64)
        }
        H_POP if n == 2 => {
            let r = crate::dict::rt_dict_pop(recv, bits(arg(at, 0)));
            if r.is_null() {
                arg(at, 1)
            } else {
                Value(r as u64)
            }
        }
        H_KEYS if n == 0 => Value(crate::dict::rt_dict_keys(recv) as u64),
        H_VALUES if n == 0 => Value(crate::dict::rt_dict_values(recv) as u64),
        H_ITEMS if n == 0 => Value(crate::dict::rt_dict_items(recv) as u64),
        H_UPDATE if n == 1 => {
            crate::dict::rt_dict_update(recv, bits(arg(at, 0)));
            Value::NONE
        }
        H_CLEAR if n == 0 => {
            crate::dict::rt_dict_clear(recv);
            Value::NONE
        }
        H_COPY if n == 0 => Value(crate::dict::rt_dict_copy(recv) as u64),
        _ => raise_no_attr(type_name(tag)),
    }
}

unsafe fn set_method(recv: *mut Obj, h: u64, at: *mut Obj, tag: TypeTagKind) -> Value {
    let n = argc(at);
    match h {
        H_ADD if n == 1 => {
            crate::set::rt_set_add(recv, bits(arg(at, 0)));
            Value::NONE
        }
        H_DISCARD if n == 1 => {
            crate::set::rt_set_discard(recv, bits(arg(at, 0)));
            Value::NONE
        }
        H_REMOVE if n == 1 => {
            crate::set::rt_set_remove(recv, bits(arg(at, 0)));
            Value::NONE
        }
        H_UPDATE if n == 1 => {
            crate::set::rt_set_update(recv, bits(arg(at, 0)));
            Value::NONE
        }
        H_CLEAR if n == 0 => {
            crate::set::rt_set_clear(recv);
            Value::NONE
        }
        H_COPY if n == 0 => Value(crate::set::rt_set_copy(recv) as u64),
        H_POP if n == 0 => Value(crate::set::rt_set_pop(recv) as u64),
        _ => raise_no_attr(type_name(tag)),
    }
}

unsafe fn deque_method(recv: *mut Obj, h: u64, at: *mut Obj, tag: TypeTagKind) -> Value {
    let n = argc(at);
    match h {
        H_APPEND if n == 1 => {
            crate::deque::rt_deque_append(recv, bits(arg(at, 0)));
            Value::NONE
        }
        H_APPENDLEFT if n == 1 => {
            crate::deque::rt_deque_appendleft(recv, bits(arg(at, 0)));
            Value::NONE
        }
        H_POP if n == 0 => Value(crate::deque::rt_deque_pop(recv) as u64),
        H_POPLEFT if n == 0 => Value(crate::deque::rt_deque_popleft(recv) as u64),
        H_EXTEND if n == 1 => {
            crate::deque::rt_deque_extend(recv, bits(arg(at, 0)));
            Value::NONE
        }
        H_EXTENDLEFT if n == 1 => {
            crate::deque::rt_deque_extendleft(recv, bits(arg(at, 0)));
            Value::NONE
        }
        H_ROTATE => {
            let k = if n >= 1 { as_index(arg(at, 0)) } else { 1 };
            crate::deque::rt_deque_rotate(recv, k);
            Value::NONE
        }
        H_CLEAR if n == 0 => {
            crate::deque::rt_deque_clear(recv);
            Value::NONE
        }
        H_COPY if n == 0 => Value(crate::deque::rt_deque_copy(recv) as u64),
        H_REVERSE if n == 0 => {
            crate::deque::rt_deque_reverse(recv);
            Value::NONE
        }
        H_COUNT if n == 1 => Value::from_int(crate::deque::rt_deque_count(recv, bits(arg(at, 0)))),
        H_INDEX if n == 1 => Value::from_int(crate::deque::rt_deque_index(recv, bits(arg(at, 0)))),
        H_INSERT if n == 2 => {
            crate::deque::rt_deque_insert(recv, as_index(arg(at, 0)), bits(arg(at, 1)));
            Value::NONE
        }
        H_REMOVE if n == 1 => {
            crate::deque::rt_deque_remove(recv, bits(arg(at, 0)));
            Value::NONE
        }
        _ => raise_no_attr(type_name(tag)),
    }
}

/// Dispatch an arbitrary user method on an `Instance` receiver through its
/// uniform thunk. The thunk binds `__args__` / `__kwargs__` to the method's
/// parameters at run time (defaults, `*args`, the checked float/bool unbox) and
/// makes ONE direct call to the native method, coercing `self` parent-first for
/// an inherited method. A missing thunk ⇒ `AttributeError`.
unsafe fn instance_method(recv: Value, name_hash: i64, args_tuple: Value, kwargs: Value) -> Value {
    let class_id = (*(recv.0 as *const crate::object::InstanceObj)).class_id;
    let ptr = lookup_method_uniform(class_id, name_hash as u64);
    if ptr.is_null() {
        raise_no_attr("instance");
    }
    let thunk: UniformThunk = std::mem::transmute(ptr);
    thunk(recv, args_tuple, kwargs)
}
