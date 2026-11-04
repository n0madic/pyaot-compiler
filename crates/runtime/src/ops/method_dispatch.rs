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
// dict views
const H_ISDISJOINT: u64 = fnv1a(b"isdisjoint");
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
// str (the gradual sibling of `lower_str_method`; `index`/`count`/`copy` reuse
// the shared hashes above). Mirrors the typed `rt_str_*` surface.
const H_UPPER: u64 = fnv1a(b"upper");
const H_LOWER: u64 = fnv1a(b"lower");
const H_TITLE: u64 = fnv1a(b"title");
const H_CAPITALIZE: u64 = fnv1a(b"capitalize");
const H_SWAPCASE: u64 = fnv1a(b"swapcase");
const H_STRIP: u64 = fnv1a(b"strip");
const H_LSTRIP: u64 = fnv1a(b"lstrip");
const H_RSTRIP: u64 = fnv1a(b"rstrip");
const H_STARTSWITH: u64 = fnv1a(b"startswith");
const H_ENDSWITH: u64 = fnv1a(b"endswith");
const H_FIND: u64 = fnv1a(b"find");
const H_RFIND: u64 = fnv1a(b"rfind");
const H_RINDEX: u64 = fnv1a(b"rindex");
const H_REPLACE: u64 = fnv1a(b"replace");
const H_SPLIT: u64 = fnv1a(b"split");
const H_RSPLIT: u64 = fnv1a(b"rsplit");
const H_JOIN: u64 = fnv1a(b"join");
const H_ZFILL: u64 = fnv1a(b"zfill");
const H_CENTER: u64 = fnv1a(b"center");
const H_LJUST: u64 = fnv1a(b"ljust");
const H_RJUST: u64 = fnv1a(b"rjust");
const H_ENCODE: u64 = fnv1a(b"encode");
const H_ISDIGIT: u64 = fnv1a(b"isdigit");
const H_ISALPHA: u64 = fnv1a(b"isalpha");
const H_ISALNUM: u64 = fnv1a(b"isalnum");
const H_ISSPACE: u64 = fnv1a(b"isspace");
const H_ISUPPER: u64 = fnv1a(b"isupper");
const H_ISLOWER: u64 = fnv1a(b"islower");
const H_ISASCII: u64 = fnv1a(b"isascii");
const H_ISDECIMAL: u64 = fnv1a(b"isdecimal");
const H_ISNUMERIC: u64 = fnv1a(b"isnumeric");
// generator / in-memory stream / file
const H_SEND: u64 = fnv1a(b"send");
const H_THROW: u64 = fnv1a(b"throw");
const H_CLOSE: u64 = fnv1a(b"close");
const H_READ: u64 = fnv1a(b"read");
const H_READLINE: u64 = fnv1a(b"readline");
const H_READLINES: u64 = fnv1a(b"readlines");
const H_WRITE: u64 = fnv1a(b"write");
const H_GETVALUE: u64 = fnv1a(b"getvalue");
const H_SEEK: u64 = fnv1a(b"seek");
const H_TELL: u64 = fnv1a(b"tell");
const H_TRUNCATE: u64 = fnv1a(b"truncate");
const H_FLUSH: u64 = fnv1a(b"flush");

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
                dict_method(recv_ptr, h, at, kwargs, tag)
            }
            TypeTagKind::Set => set_method(recv_ptr, h, at, tag),
            TypeTagKind::Deque => deque_method(recv_ptr, h, at, tag),
            TypeTagKind::DictKeys | TypeTagKind::DictValues | TypeTagKind::DictItems => {
                dict_view_method(recv_ptr, h, at, tag)
            }
            TypeTagKind::Tuple => tuple_method(recv_ptr, h, at, tag),
            // A `str` receiver: the gradual sibling of the typed `lower_str_method`
            // path (e.g. `data.encode()` where `data` is a `Dyn`/`Union`-typed
            // parameter the front-half could not pin to `str`).
            TypeTagKind::Str => str_method(recv_ptr, h, at, tag),
            // A heap bignum int (`rt_int_*` are bignum-aware and take the Value).
            TypeTagKind::BigInt => int_method(recv, h, at),
            TypeTagKind::Instance => instance_method(recv, name_hash, args_tuple, kwargs),
            // A gradual generator (`g.send(v)` / `g.close()` where `g` is
            // `Dyn`) and the stream objects that share those method names
            // (`StringIO`/`BytesIO`/file `.close()`): each goes to its own
            // runtime family, so a name never picks the wrong object kind.
            TypeTagKind::Generator => generator_method(recv_ptr, h, at),
            TypeTagKind::StringIO => stringio_method(recv_ptr, h, at),
            TypeTagKind::BytesIO => bytesio_method(recv_ptr, h, at),
            TypeTagKind::File => file_method(recv_ptr, h, at),
            other => raise_no_attr(type_name(other)),
        }
    }
}

/// Generator methods on a `Dyn` receiver — the gradual sibling of the typed
/// `GenOp::Send` / `GenOp::Close` lowering, through the same runtime entry
/// points. `throw()` is not supported by the compiled resume protocol.
unsafe fn generator_method(recv: *mut Obj, h: u64, at: *mut Obj) -> Value {
    let n = argc(at);
    match h {
        H_SEND if n == 1 => Value(crate::generator::rt_generator_send(recv, arg(at, 0).0 as i64) as u64),
        H_CLOSE if n == 0 => {
            crate::generator::rt_generator_close(recv);
            Value::NONE
        }
        H_THROW => crate::raise_exc!(
            crate::exceptions::ExceptionType::NotImplementedError,
            "generator.throw() is not supported"
        ),
        _ => raise_no_attr("generator"),
    }
}

/// An optional size/position argument: absent → `default`.
#[inline]
unsafe fn opt_index(at: *mut Obj, n: i64, default: i64) -> i64 {
    if n >= 1 {
        let v = arg(at, 0);
        if v.is_none() {
            default
        } else {
            as_index(v)
        }
    } else {
        default
    }
}

/// `io.StringIO` methods on a `Dyn` receiver (same `rt_stringio_*` family as
/// the typed stdlib-method path).
unsafe fn stringio_method(recv: *mut Obj, h: u64, at: *mut Obj) -> Value {
    use crate::stringio as sio;
    let n = argc(at);
    match h {
        H_WRITE if n == 1 => Value::from_int(sio::rt_stringio_write(recv, bits(arg(at, 0)))),
        H_READ if n <= 1 => Value::from_ptr(sio::rt_stringio_read(recv, opt_index(at, n, -1))),
        H_READLINE if n == 0 => Value::from_ptr(sio::rt_stringio_readline(recv)),
        H_GETVALUE if n == 0 => Value::from_ptr(sio::rt_stringio_getvalue(recv)),
        H_SEEK if n == 1 => Value::from_int(sio::rt_stringio_seek(recv, as_index(arg(at, 0)))),
        H_TELL if n == 0 => Value::from_int(sio::rt_stringio_tell(recv)),
        H_TRUNCATE if n <= 1 => Value::from_int(sio::rt_stringio_truncate(recv, opt_index(at, n, -1))),
        H_CLOSE if n == 0 => {
            sio::rt_stringio_close(recv);
            Value::NONE
        }
        _ => raise_no_attr("_io.StringIO"),
    }
}

/// `io.BytesIO` methods on a `Dyn` receiver.
unsafe fn bytesio_method(recv: *mut Obj, h: u64, at: *mut Obj) -> Value {
    use crate::stringio as sio;
    let n = argc(at);
    match h {
        H_WRITE if n == 1 => Value::from_int(sio::rt_bytesio_write(recv, bits(arg(at, 0)))),
        H_READ if n <= 1 => Value::from_ptr(sio::rt_bytesio_read(recv, opt_index(at, n, -1))),
        H_GETVALUE if n == 0 => Value::from_ptr(sio::rt_bytesio_getvalue(recv)),
        H_SEEK if n == 1 => Value::from_int(sio::rt_bytesio_seek(recv, as_index(arg(at, 0)))),
        H_TELL if n == 0 => Value::from_int(sio::rt_bytesio_tell(recv)),
        H_CLOSE if n == 0 => {
            sio::rt_bytesio_close(recv);
            Value::NONE
        }
        _ => raise_no_attr("_io.BytesIO"),
    }
}

/// File-object methods on a `Dyn` receiver (same `rt_file_*` family as the
/// typed `lower_file_method` path).
unsafe fn file_method(recv: *mut Obj, h: u64, at: *mut Obj) -> Value {
    use crate::file as f;
    let n = argc(at);
    match h {
        H_READ if n == 0 => Value::from_ptr(f::rt_file_read(recv)),
        H_READ if n == 1 => Value::from_ptr(f::rt_file_read_n(recv, opt_index(at, n, -1))),
        H_READLINE if n == 0 => Value::from_ptr(f::rt_file_readline(recv)),
        H_READLINES if n == 0 => Value::from_ptr(f::rt_file_readlines(recv)),
        H_WRITE if n == 1 => Value::from_int(f::rt_file_write(recv, bits(arg(at, 0)))),
        H_FLUSH if n == 0 => {
            f::rt_file_flush(recv);
            Value::NONE
        }
        H_CLOSE if n == 0 => {
            f::rt_file_close(recv);
            Value::NONE
        }
        _ => raise_no_attr("_io.TextIOWrapper"),
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

/// `str` methods on a `Dyn`/`Union` receiver — the gradual-completeness sibling
/// of the typed `lower_str_method` path. Routes to the SAME `rt_str_*` family
/// the statically-typed path calls; positional args ride the `tuple[Tagged]`,
/// the raw-i64 slots (`start`/`end`/`count`/`maxsplit`/`width`) are unboxed from
/// their tagged ints with the same defaults `lower_str_method` uses, and an
/// absent optional object arg (`chars`/`sep`/`fillchar`/`encoding`/`errors`)
/// passes the null sentinel. `find`/`rfind`/`index`/`rindex` ride the shared
/// `rt_str_search` (op_tag 0/1/2/3 — `index`/`rindex` raise `ValueError` on a
/// miss). Method names not wired here (the rarer `splitlines`/`partition`/
/// `removeprefix`/`expandtabs` surface) fall through to `AttributeError`, the
/// same incremental policy the container arms follow.
unsafe fn str_method(recv: *mut Obj, h: u64, at: *mut Obj, tag: TypeTagKind) -> Value {
    use crate::string as s;
    let n = argc(at);
    // Optional object arg `i` as a ptr (null when absent). The `arg` read is
    // unsafe; a closure does not inherit the enclosing `unsafe fn` context.
    let opt = |i: i64| -> *mut Obj {
        if i < n {
            bits(unsafe { arg(at, i) })
        } else {
            std::ptr::null_mut()
        }
    };
    let to_bool = |b: i8| if b != 0 { Value::TRUE } else { Value::FALSE };
    match h {
        // 0-arg, str → str.
        H_UPPER if n == 0 => Value::from_ptr(s::rt_str_upper(recv)),
        H_LOWER if n == 0 => Value::from_ptr(s::rt_str_lower(recv)),
        H_TITLE if n == 0 => Value::from_ptr(s::rt_str_title(recv)),
        H_CAPITALIZE if n == 0 => Value::from_ptr(s::rt_str_capitalize(recv)),
        H_SWAPCASE if n == 0 => Value::from_ptr(s::rt_str_swapcase(recv)),
        H_STRIP if n == 0 => Value::from_ptr(s::rt_str_strip(recv)),
        // Optional `chars` (null = whitespace).
        H_LSTRIP if n <= 1 => Value::from_ptr(s::rt_str_lstrip(recv, opt(0))),
        H_RSTRIP if n <= 1 => Value::from_ptr(s::rt_str_rstrip(recv, opt(0))),
        // 1 arg, str → bool.
        H_STARTSWITH if n == 1 => to_bool(s::rt_str_startswith(recv, bits(arg(at, 0)))),
        H_ENDSWITH if n == 1 => to_bool(s::rt_str_endswith(recv, bits(arg(at, 0)))),
        // Search family — shared `rt_str_search` with the method's op_tag, the
        // optional `start`/`end` riding raw i64 (defaults 0 / i64::MAX). `index`/
        // `rindex` raise `ValueError` on a miss; `find`/`rfind` return -1.
        H_FIND | H_RFIND | H_INDEX | H_RINDEX if n >= 1 => {
            let start = if n >= 2 { as_index(arg(at, 1)) } else { 0 };
            let end = if n >= 3 { as_index(arg(at, 2)) } else { i64::MAX };
            let op_tag = match h {
                H_FIND => 0,
                H_RFIND => 1,
                H_INDEX => 2,
                _ => 3,
            };
            Value::from_int(s::rt_str_search(recv, bits(arg(at, 0)), start, end, op_tag))
        }
        H_COUNT if n == 1 => Value::from_int(s::rt_str_count(recv, bits(arg(at, 0)))),
        // `replace(old, new[, count])` — `count` raw i64, absent → -1 (replace all).
        H_REPLACE if n >= 2 => {
            let count = if n >= 3 { as_index(arg(at, 2)) } else { -1 };
            Value::from_ptr(s::rt_str_replace(recv, bits(arg(at, 0)), bits(arg(at, 1)), count))
        }
        // `split`/`rsplit([sep[, maxsplit]])` — absent sep = whitespace (null),
        // absent maxsplit = -1 (unlimited).
        H_SPLIT if n <= 2 => {
            let maxsplit = if n >= 2 { as_index(arg(at, 1)) } else { -1 };
            Value::from_ptr(s::rt_str_split(recv, opt(0), maxsplit))
        }
        H_RSPLIT if n <= 2 => {
            let maxsplit = if n >= 2 { as_index(arg(at, 1)) } else { -1 };
            Value::from_ptr(s::rt_str_rsplit(recv, opt(0), maxsplit))
        }
        // `sep.join(iterable)`.
        H_JOIN if n == 1 => Value::from_ptr(s::rt_str_join(recv, bits(arg(at, 0)))),
        // Alignment — `width` raw i64; `fillchar` optional (null = space).
        H_ZFILL if n == 1 => Value::from_ptr(s::rt_str_zfill(recv, as_index(arg(at, 0)))),
        H_CENTER if (1..=2).contains(&n) => {
            Value::from_ptr(s::rt_str_center(recv, as_index(arg(at, 0)), opt(1)))
        }
        H_LJUST if (1..=2).contains(&n) => {
            Value::from_ptr(s::rt_str_ljust(recv, as_index(arg(at, 0)), opt(1)))
        }
        H_RJUST if (1..=2).contains(&n) => {
            Value::from_ptr(s::rt_str_rjust(recv, as_index(arg(at, 0)), opt(1)))
        }
        // `encode([encoding[, errors]])` → bytes (null args = utf-8 / strict).
        H_ENCODE if n <= 2 => Value::from_ptr(s::rt_str_encode(recv, opt(0), opt(1))),
        // Codepoint predicates → bool.
        H_ISDIGIT if n == 0 => to_bool(s::rt_str_isdigit(recv)),
        H_ISALPHA if n == 0 => to_bool(s::rt_str_isalpha(recv)),
        H_ISALNUM if n == 0 => to_bool(s::rt_str_isalnum(recv)),
        H_ISSPACE if n == 0 => to_bool(s::rt_str_isspace(recv)),
        H_ISUPPER if n == 0 => to_bool(s::rt_str_isupper(recv)),
        H_ISLOWER if n == 0 => to_bool(s::rt_str_islower(recv)),
        H_ISASCII if n == 0 => to_bool(s::rt_str_isascii(recv)),
        H_ISDECIMAL if n == 0 => to_bool(s::rt_str_isdecimal(recv)),
        H_ISNUMERIC if n == 0 => to_bool(s::rt_str_isnumeric(recv)),
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

unsafe fn dict_method(recv: *mut Obj, h: u64, at: *mut Obj, kwargs: Value, tag: TypeTagKind) -> Value {
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
        // Live views (the same objects the typed `ContainerOp::DictKeys/...`
        // path builds), not list snapshots.
        H_KEYS if n == 0 => Value(crate::dict_view::dict_view_new(recv, TypeTagKind::DictKeys) as u64),
        H_VALUES if n == 0 => {
            Value(crate::dict_view::dict_view_new(recv, TypeTagKind::DictValues) as u64)
        }
        H_ITEMS if n == 0 => Value(crate::dict_view::dict_view_new(recv, TypeTagKind::DictItems) as u64),
        // `dict.update(E, **F)` (plain dict only): merge an optional positional
        // mapping, then the keyword dict (a string-keyed mapping — CPython treats
        // `update(**F)` as merging `F`). Counter.update has add-semantics, so its
        // keyword form is left out (falls through to the positional-only arm /
        // raise below).
        H_UPDATE if tag == TypeTagKind::Dict && n <= 1 => {
            if n == 1 {
                crate::dict::rt_dict_update(recv, bits(arg(at, 0)));
            }
            if kwargs.is_ptr() {
                crate::dict::rt_dict_update(recv, kwargs.0 as *mut Obj);
            }
            Value::NONE
        }
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

/// Dict-view methods: `isdisjoint` on the set-like keys / items views (the
/// typed view receiver routes here too — a view has no static method table).
unsafe fn dict_view_method(recv: *mut Obj, h: u64, at: *mut Obj, tag: TypeTagKind) -> Value {
    let n = argc(at);
    match h {
        H_ISDISJOINT if n == 1 && tag != TypeTagKind::DictValues => Value::from_bool(
            crate::dict_view::view_isdisjoint(recv, bits(arg(at, 0))) != 0,
        ),
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
