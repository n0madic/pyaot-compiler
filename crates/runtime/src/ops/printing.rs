//! Print operations for Python runtime

use crate::object::{
    BigIntObj, DequeObj, DictObj, FloatObj, ListObj, Obj, SetObj, StrObj, TupleObj, TypeTagKind,
};
use crate::print::{rt_print_bytes_obj, rt_print_str_obj};
use pyaot_core_defs::Value;

// === Print functions for print() builtin ===

/// Print an integer value (no newline)
#[no_mangle]
pub extern "C" fn rt_print_int_value(value: i64) {
    rt_emit!("{}", value);
}

/// Print a float value (no newline)
#[no_mangle]
pub extern "C" fn rt_print_float_value(value: f64) {
    rt_emit!("{}", crate::utils::format_float_python(value));
}

/// Print a boolean value (no newline)
#[no_mangle]
pub extern "C" fn rt_print_bool_value(value: bool) {
    rt_emit!("{}", if value { "True" } else { "False" });
}

/// Print None value (no newline)
#[no_mangle]
pub extern "C" fn rt_print_none_value() {
    rt_emit!("None");
}

/// Print a C string (no newline)
/// str_ptr is a pointer to a null-terminated C string
///
/// # Safety
/// `str_ptr` must be null or a valid pointer to a null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn rt_print_str_value(str_ptr: *const i8) {
    if !str_ptr.is_null() {
        let c_str = std::ffi::CStr::from_ptr(str_ptr);
        rt_emit!("{}", c_str.to_string_lossy());
    }
}

/// Print newline and flush stdout
#[no_mangle]
pub extern "C" fn rt_print_newline() {
    rt_emit!("\n");
}

/// Print default separator (space)
#[no_mangle]
pub extern "C" fn rt_print_sep() {
    rt_emit!(" ");
}

/// Flush stdout (useful after print without newline)
#[no_mangle]
pub extern "C" fn rt_flush_stdout() {
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

/// Flush the CURRENT print target — `print(..., flush=True)`. Routes by the
/// sticky global target so a `flush=True` on a `file=sys.stderr` line flushes
/// stderr (a no-op in practice — Rust's stderr is unbuffered — but kept honest)
/// and a default/`file=sys.stdout` line flushes the buffered stdout.
#[no_mangle]
pub extern "C" fn rt_print_flush() {
    use std::io::Write;
    if crate::print::is_stderr_target() {
        let _ = std::io::stderr().flush();
    } else {
        let _ = std::io::stdout().flush();
    }
}

// === Runtime type dispatch printing operations for Union types ===

/// Print a single element from a container, dispatching on Value tag
unsafe fn print_elem_repr(elem: *mut Obj) {
    print_obj_repr(elem);
}

/// Print Python repr of any boxed object (strings get quotes).
///
/// NOTE: This function intentionally duplicates much of `rt_print_obj` below.
/// The key difference is that `print_obj_repr` renders strings in repr mode
/// (with single quotes and escape sequences), while `rt_print_obj` renders
/// strings in str mode (raw content, no quotes). All other type arms are
/// identical. We keep them separate rather than adding a `repr: bool` parameter
/// to avoid branching overhead on a hot path that is called recursively for
/// every element in nested containers.
unsafe fn print_obj_repr(obj: *mut Obj) {
    if obj.is_null() {
        rt_emit!("None");
        return;
    }
    // Check Value-tagged primitives before heap pointer dereference.
    let v = Value(obj as u64);
    if v.is_int() {
        rt_emit!("{}", v.unwrap_int());
        return;
    }
    if v.is_bool() {
        rt_emit!("{}", if v.unwrap_bool() { "True" } else { "False" });
        return;
    }
    if v.is_none() {
        rt_emit!("None");
        return;
    }
    match (*obj).type_tag() {
        // Int/Bool handled above via Value tag checks; these arms are unreachable
        // for correctly-tagged values but required for exhaustiveness.
        TypeTagKind::Int => rt_emit!("{}", Value(obj as u64).unwrap_int()),
        TypeTagKind::Bool => {
            rt_emit!(
                "{}",
                if Value(obj as u64).unwrap_bool() {
                    "True"
                } else {
                    "False"
                }
            )
        }
        TypeTagKind::Float => {
            let float_obj = obj as *mut FloatObj;
            rt_emit!("{}", crate::utils::format_float_python((*float_obj).value));
        }
        TypeTagKind::BigInt => rt_emit!("{}", (*(obj as *mut BigIntObj)).value),
        TypeTagKind::Str => {
            // In repr mode, strings get single quotes with proper escaping
            let str_obj = obj as *mut StrObj;
            let len = (*str_obj).len;
            let data = (*str_obj).data.as_ptr();
            let bytes = std::slice::from_raw_parts(data, len);
            if let Ok(text) = std::str::from_utf8(bytes) {
                let mut s = String::with_capacity(len + 2);
                s.push('\'');
                crate::conversions::repr_escape_into(&mut s, text);
                s.push('\'');
                rt_emit!("{}", s);
            }
        }
        TypeTagKind::None => rt_emit!("None"),
        TypeTagKind::List => print_list_repr(obj),
        TypeTagKind::Tuple => print_tuple_repr(obj),
        TypeTagKind::Dict => print_dict_repr(obj),
        TypeTagKind::Set => print_set_repr(obj),
        TypeTagKind::FrozenSet => print_frozenset_repr(obj),
        TypeTagKind::Bytes => rt_print_bytes_obj(obj),
        TypeTagKind::ByteArray => rt_emit!("{}", crate::bytearray::bytearray_repr_string(obj)),
        TypeTagKind::Instance => print_instance_repr(obj),
        TypeTagKind::Iterator => rt_emit!("<iterator>"),
        TypeTagKind::Cell => rt_emit!("<cell>"),
        // For these types, use type_name() from core-defs (single source of truth)
        TypeTagKind::Generator => rt_emit!(
            "<{} object at {:p}>",
            TypeTagKind::Generator.type_name(),
            obj
        ),
        TypeTagKind::Match => rt_emit!("<{} object at {:p}>", TypeTagKind::Match.type_name(), obj),
        TypeTagKind::File => rt_emit!("<{} at {:p}>", TypeTagKind::File.type_name(), obj),
        TypeTagKind::StringBuilder => {
            rt_emit!("<{} at {:p}>", TypeTagKind::StringBuilder.type_name(), obj)
        }
        TypeTagKind::StructTime => rt_emit!("<{} at {:p}>", TypeTagKind::StructTime.type_name(), obj),
        TypeTagKind::CompletedProcess => rt_emit!(
            "<{} at {:p}>",
            TypeTagKind::CompletedProcess.type_name(),
            obj
        ),
        TypeTagKind::ParseResult => {
            rt_emit!("<{} at {:p}>", TypeTagKind::ParseResult.type_name(), obj)
        }
        TypeTagKind::HttpResponse => {
            rt_emit!("<{} at {:p}>", TypeTagKind::HttpResponse.type_name(), obj)
        }
        TypeTagKind::Hash => rt_emit!("<{} object>", TypeTagKind::Hash.type_name()),
        TypeTagKind::StringIO => rt_emit!("<_io.StringIO object>"),
        TypeTagKind::BytesIO => rt_emit!("<_io.BytesIO object>"),
        TypeTagKind::DefaultDict => print_dict_repr(obj), // Same repr as dict
        TypeTagKind::Counter => {
            rt_emit!("{}", crate::conversions::counter_repr_string(obj))
        }
        TypeTagKind::Deque => print_deque_repr(obj),
        TypeTagKind::Request => rt_emit!("<{} at {:p}>", TypeTagKind::Request.type_name(), obj),
        TypeTagKind::NotImplemented => rt_emit!("NotImplemented"),
        TypeTagKind::Closure => rt_emit!("<{} at {:p}>", TypeTagKind::Closure.type_name(), obj),
    }
}

/// Render a class instance encountered as a container element. CPython
/// renders container elements with `repr()`, so dispatch the user
/// `__repr__` via `DUNDER_FUNC_REGISTRY` (the top-level `print(instance)`
/// path resolves this at lowering time, but container elements are rendered
/// by the runtime and have no static class type). Falls back to the default
/// object repr when the class defines no `__repr__`.
unsafe fn print_instance_repr(obj: *mut Obj) {
    if let Some(str_obj) = crate::ops::try_repr_dunder(obj) {
        if !str_obj.is_null() {
            let s = str_obj as *mut StrObj;
            let len = (*s).len;
            let bytes = std::slice::from_raw_parts((*s).data.as_ptr(), len);
            if let Ok(text) = std::str::from_utf8(bytes) {
                rt_emit!("{}", text);
            }
            return;
        }
    }
    // No user `__repr__`: CPython's default `<__main__.Cls object at 0x..>`.
    rt_emit!("{}", crate::instance::instance_default_repr(obj));
}

/// Print list repr: [elem, elem, ...]
unsafe fn print_list_repr(obj: *mut Obj) {
    let list = obj as *mut ListObj;
    let len = (*list).len;
    let data = (*list).data;

    rt_emit!("[");
    for i in 0..len {
        if i > 0 {
            rt_emit!(", ");
        }
        let elem = (*data.add(i)).0 as *mut Obj;
        print_elem_repr(elem);
    }
    rt_emit!("]");
}

/// Print tuple repr: (elem, elem, ...) with trailing comma for single-element
unsafe fn print_tuple_repr(obj: *mut Obj) {
    let tuple = obj as *mut TupleObj;
    let len = (*tuple).len;
    let data = (*tuple).data.as_ptr();

    rt_emit!("(");
    for i in 0..len {
        if i > 0 {
            rt_emit!(", ");
        }
        let elem = (*data.add(i)).0 as *mut Obj;
        print_elem_repr(elem);
    }
    if len == 1 {
        rt_emit!(",");
    }
    rt_emit!(")");
}

/// Print deque repr: `deque([elem, ...])` or `deque([...], maxlen=N)`.
/// Walks the ring buffer in logical left-to-right order. In CPython
/// `str(deque) == repr(deque)`, so both print paths route here.
unsafe fn print_deque_repr(obj: *mut Obj) {
    let d = obj as *mut DequeObj;
    let len = (*d).len;
    let cap = (*d).capacity;
    let head = (*d).head;
    let maxlen = (*d).maxlen;

    rt_emit!("deque([");
    for i in 0..len {
        if i > 0 {
            rt_emit!(", ");
        }
        let ring_idx = (head + i) % cap;
        let elem = (*(*d).data.add(ring_idx)).0 as *mut Obj;
        print_elem_repr(elem);
    }
    rt_emit!("]");
    if maxlen >= 0 {
        rt_emit!(", maxlen={}", maxlen);
    }
    rt_emit!(")");
}

/// Print a value that may be a heap object or a Value-tagged primitive.
unsafe fn print_maybe_raw_repr(ptr: *mut Obj) {
    print_obj_repr(ptr);
}

/// Print dict repr: {key: value, ...}
/// Dict keys are always boxed, but values may be raw primitives
unsafe fn print_dict_repr(obj: *mut Obj) {
    let dict = obj as *mut DictObj;
    let entries_len = (*dict).entries_len;
    let entries = (*dict).entries;

    rt_emit!("{{");
    let mut first = true;
    for i in 0..entries_len {
        let entry = entries.add(i);
        let key = (*entry).key;
        if key.0 != 0 {
            if !first {
                rt_emit!(", ");
            }
            first = false;
            print_obj_repr(key.0 as *mut Obj);
            rt_emit!(": ");
            print_maybe_raw_repr((*entry).value.0 as *mut Obj);
        }
    }
    rt_emit!("}}");
}

/// Print set repr: {elem, ...} or set() for empty
/// Set elements are always boxed heap objects
unsafe fn print_set_repr(obj: *mut Obj) {
    let set = obj as *mut SetObj;
    let len = (*set).len;
    let capacity = (*set).capacity;
    let entries = (*set).entries;
    use crate::object::TOMBSTONE;

    if len == 0 {
        rt_emit!("set()");
        return;
    }

    rt_emit!("{{");
    let mut first = true;
    for i in 0..capacity {
        let entry = entries.add(i);
        let elem = (*entry).elem;
        if elem.0 != 0 && elem != TOMBSTONE {
            if !first {
                rt_emit!(", ");
            }
            first = false;
            print_obj_repr(elem.0 as *mut Obj);
        }
    }
    rt_emit!("}}");
}

/// Print frozenset repr: `frozenset({elem, ...})` or `frozenset()` for empty
/// (distinct from `set`'s `{…}` / `set()`). Shares `SetObj` layout.
unsafe fn print_frozenset_repr(obj: *mut Obj) {
    let set = obj as *mut SetObj;
    let len = (*set).len;
    let capacity = (*set).capacity;
    let entries = (*set).entries;
    use crate::object::TOMBSTONE;

    if len == 0 {
        rt_emit!("frozenset()");
        return;
    }

    rt_emit!("frozenset({{");
    let mut first = true;
    for i in 0..capacity {
        let entry = entries.add(i);
        let elem = (*entry).elem;
        if elem.0 != 0 && elem != TOMBSTONE {
            if !first {
                rt_emit!(", ");
            }
            first = false;
            print_obj_repr(elem.0 as *mut Obj);
        }
    }
    rt_emit!("}})");
}

/// Print any heap object with runtime type dispatch
/// Used for Union types where the actual type is determined at runtime
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_print_obj(obj: *mut Obj) {
    if obj.is_null() {
        rt_emit!("None");
        return;
    }
    // Check Value-tagged primitives before heap pointer dereference.
    let v = Value(obj as u64);
    if v.is_int() {
        rt_emit!("{}", v.unwrap_int());
        return;
    }
    if v.is_bool() {
        rt_emit!("{}", if v.unwrap_bool() { "True" } else { "False" });
        return;
    }
    if v.is_none() {
        rt_emit!("None");
        return;
    }
    unsafe {
        match (*obj).type_tag() {
            // Int/Bool handled above via Value tag checks; arms required for exhaustiveness.
            TypeTagKind::Int => rt_emit!("{}", Value(obj as u64).unwrap_int()),
            TypeTagKind::Bool => {
                rt_emit!(
                    "{}",
                    if Value(obj as u64).unwrap_bool() {
                        "True"
                    } else {
                        "False"
                    }
                )
            }
            TypeTagKind::Float => {
                let float_obj = obj as *mut FloatObj;
                rt_emit!("{}", crate::utils::format_float_python((*float_obj).value));
            }
            TypeTagKind::BigInt => rt_emit!("{}", (*(obj as *mut BigIntObj)).value),
            TypeTagKind::Str => rt_print_str_obj(obj),
            TypeTagKind::Bytes => rt_print_bytes_obj(obj),
            TypeTagKind::None => rt_emit!("None"),
            TypeTagKind::List => print_list_repr(obj),
            TypeTagKind::Tuple => print_tuple_repr(obj),
            TypeTagKind::Dict => print_dict_repr(obj),
            TypeTagKind::Set => print_set_repr(obj),
            TypeTagKind::FrozenSet => print_frozenset_repr(obj),
            TypeTagKind::ByteArray => {
                rt_emit!("{}", crate::bytearray::bytearray_repr_string(obj))
            }
            TypeTagKind::Instance => {
                // CPython's `print(instance)` renders `str(instance)`: dispatch
                // the user `__str__` (falling back to `__repr__`) via the dunder
                // registry. A `Dyn`/`Union` instance reaches here — the static
                // class path is devirtualized at lowering; without this it would
                // show only the default object repr (e.g. a value from a
                // `Union[T, NotImplementedT]` dunder return, §4).
                match crate::ops::try_str_dunder(obj) {
                    Some(str_obj) if !str_obj.is_null() => rt_print_str_obj(str_obj),
                    _ => rt_emit!("{}", crate::instance::instance_default_repr(obj)),
                }
            }
            TypeTagKind::Iterator => rt_emit!("<iterator>"),
            TypeTagKind::Cell => rt_emit!("<cell>"),
            // For these types, use type_name() from core-defs (single source of truth)
            TypeTagKind::Generator => rt_emit!(
                "<{} object at {:p}>",
                TypeTagKind::Generator.type_name(),
                obj
            ),
            TypeTagKind::Match => {
                rt_emit!("<{} object at {:p}>", TypeTagKind::Match.type_name(), obj)
            }
            TypeTagKind::File => rt_emit!("<{} at {:p}>", TypeTagKind::File.type_name(), obj),
            TypeTagKind::StringBuilder => {
                rt_emit!("<{} at {:p}>", TypeTagKind::StringBuilder.type_name(), obj)
            }
            TypeTagKind::StructTime => {
                rt_emit!("<{} at {:p}>", TypeTagKind::StructTime.type_name(), obj)
            }
            TypeTagKind::CompletedProcess => rt_emit!(
                "<{} at {:p}>",
                TypeTagKind::CompletedProcess.type_name(),
                obj
            ),
            TypeTagKind::ParseResult => {
                rt_emit!("<{} at {:p}>", TypeTagKind::ParseResult.type_name(), obj)
            }
            TypeTagKind::HttpResponse => {
                rt_emit!("<{} at {:p}>", TypeTagKind::HttpResponse.type_name(), obj)
            }
            TypeTagKind::Hash => rt_emit!("<{} object>", TypeTagKind::Hash.type_name()),
            TypeTagKind::StringIO => rt_emit!("<_io.StringIO object>"),
            TypeTagKind::BytesIO => rt_emit!("<_io.BytesIO object>"),
            TypeTagKind::DefaultDict => print_dict_repr(obj),
            TypeTagKind::Counter => {
                rt_emit!("{}", crate::conversions::counter_repr_string(obj))
            }
            TypeTagKind::Deque => print_deque_repr(obj),
            TypeTagKind::Request => {
                rt_emit!("<{} at {:p}>", TypeTagKind::Request.type_name(), obj)
            }
            TypeTagKind::NotImplemented => rt_emit!("NotImplemented"),
            TypeTagKind::Closure => {
                rt_emit!("<{} at {:p}>", TypeTagKind::Closure.type_name(), obj)
            }
        }
    }
}
#[export_name = "rt_print_obj"]
pub extern "C" fn rt_print_obj_abi(obj: Value) {
    rt_print_obj(obj.unwrap_ptr())
}
