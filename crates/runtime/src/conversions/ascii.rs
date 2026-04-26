//! ascii() conversion functions for Python runtime

use crate::object::{
    BytesObj, DictObj, ListObj, Obj, ObjHeader, SetObj, StrObj, TupleObj, TypeTagKind,
};
use crate::string::rt_make_str;

/// Helper to convert an object to its ASCII representation string
pub(super) unsafe fn obj_to_ascii_string(obj: *mut Obj) -> String {
    if obj.is_null() {
        return "None".to_string();
    }

    let v = pyaot_core_defs::Value(obj as u64);
    if v.is_none() {
        return "None".to_string();
    }
    if v.is_int() {
        return v.unwrap_int().to_string();
    }
    if v.is_bool() {
        return if v.unwrap_bool() { "True" } else { "False" }.to_string();
    }

    let header = obj as *mut ObjHeader;
    match (*header).type_tag {
        TypeTagKind::Str => {
            // Get the repr form with quotes
            let src = obj as *mut StrObj;
            let len = (*src).len;
            let data = (*src).data.as_ptr();
            let bytes = std::slice::from_raw_parts(data, len);

            let mut s = String::with_capacity(len + 2);
            s.push('\'');
            if let Ok(text) = std::str::from_utf8(bytes) {
                for c in text.chars() {
                    match c {
                        '\n' => s.push_str("\\n"),
                        '\r' => s.push_str("\\r"),
                        '\t' => s.push_str("\\t"),
                        '\\' => s.push_str("\\\\"),
                        '\'' => s.push_str("\\'"),
                        _ => {
                            let cp = c as u32;
                            if cp < 128 {
                                s.push(c);
                            } else if cp <= 0xFF {
                                s.push_str(&format!("\\x{:02x}", cp));
                            } else if cp <= 0xFFFF {
                                s.push_str(&format!("\\u{:04x}", cp));
                            } else {
                                s.push_str(&format!("\\U{:08x}", cp));
                            }
                        }
                    }
                }
            }
            s.push('\'');
            s
        }
        TypeTagKind::List => {
            let list = obj as *mut ListObj;
            let len = (*list).len;

            let mut s = String::from("[");
            for i in 0..len {
                if i > 0 {
                    s.push_str(", ");
                }
                let elem = (*(*list).data.add(i)).0 as *mut crate::object::Obj;
                s.push_str(&obj_to_ascii_string(elem));
            }
            s.push(']');
            s
        }
        TypeTagKind::Tuple => {
            let tuple = obj as *mut TupleObj;
            let len = (*tuple).len;
            let data = (*tuple).data.as_ptr();

            let mut s = String::from("(");
            for i in 0..len {
                if i > 0 {
                    s.push_str(", ");
                }
                let elem = *data.add(i);
                s.push_str(&obj_to_ascii_string(elem.0 as *mut Obj));
            }
            if len == 1 {
                s.push(',');
            }
            s.push(')');
            s
        }
        TypeTagKind::Dict => {
            let dict = obj as *mut DictObj;
            let entries_len = (*dict).entries_len;
            let entries = (*dict).entries;

            let mut s = String::from("{");
            let mut first = true;
            for i in 0..entries_len {
                let entry = entries.add(i);
                let key = (*entry).key;
                if key.0 != 0 {
                    if !first {
                        s.push_str(", ");
                    }
                    first = false;
                    s.push_str(&obj_to_ascii_string(key.0 as *mut Obj));
                    s.push_str(": ");
                    s.push_str(&obj_to_ascii_string((*entry).value.0 as *mut Obj));
                }
            }
            s.push('}');
            s
        }
        TypeTagKind::Set => {
            let set = obj as *mut SetObj;
            let len = (*set).len;
            if len == 0 {
                return "set()".to_string();
            }
            let capacity = (*set).capacity;
            let entries = (*set).entries;
            let mut s = String::from("{");
            let mut first = true;
            for i in 0..capacity {
                let entry = entries.add(i);
                let elem = (*entry).elem;
                if elem.0 != 0 && elem != crate::object::TOMBSTONE {
                    if !first {
                        s.push_str(", ");
                    }
                    first = false;
                    s.push_str(&obj_to_ascii_string(elem.0 as *mut Obj));
                }
            }
            s.push('}');
            s
        }
        TypeTagKind::Bytes => {
            // Bytes ascii() is identical to repr() — all bytes are already ASCII-safe
            let src = obj as *mut BytesObj;
            let len = (*src).len;
            let data = (*src).data.as_ptr();
            let mut s = String::with_capacity(len + 3);
            s.push_str("b'");
            for i in 0..len {
                let b = *data.add(i);
                if (0x20..0x7f).contains(&b) && b != b'\'' && b != b'\\' {
                    s.push(b as char);
                } else {
                    s.push_str(&format!("\\x{:02x}", b));
                }
            }
            s.push('\'');
            s
        }
        // For non-string primitive types, delegate to repr (they don't contain non-ASCII)
        _ => super::to_str::obj_to_repr_string(obj),
    }
}

/// ascii() for collections (list, tuple, dict, set), str, bytes, and generic objects — runtime type-dispatched
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_ascii_collection(obj: *mut Obj) -> *mut Obj {
    let s = unsafe { obj_to_ascii_string(obj) };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
