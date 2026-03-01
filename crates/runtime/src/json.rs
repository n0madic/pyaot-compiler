//! JSON module runtime support
//!
//! Provides:
//! - json.dumps(obj) -> str (serialize Python object to JSON string)
//! - json.loads(s) -> obj (deserialize JSON string to Python object)
//! - json.dump(obj, fp) -> None (serialize Python object to JSON file)
//! - json.load(fp) -> obj (deserialize JSON object from JSON file)

use crate::object::{
    BoolObj, DictObj, FloatObj, IntObj, ListObj, Obj, TupleObj, TypeTagKind, ELEM_HEAP_OBJ,
    ELEM_RAW_BOOL, ELEM_RAW_INT,
};
use crate::utils::make_str_from_rust;
use serde_json::{Map, Number, Value};

/// Convert a list/tuple element to JSON, respecting elem_tag
unsafe fn elem_to_json_value(elem: *mut Obj, elem_tag: u8) -> Value {
    match elem_tag {
        ELEM_RAW_INT => Value::Number(Number::from(elem as i64)),
        ELEM_RAW_BOOL => Value::Bool((elem as i64) != 0),
        _ => obj_to_json_value(elem), // ELEM_HEAP_OBJ
    }
}

/// Convert a dict value pointer to JSON
/// Dict values may be raw primitives (stored as pointer-sized integers) or heap objects
unsafe fn maybe_raw_to_json_value(ptr: *mut Obj) -> Value {
    if ptr.is_null() {
        return Value::Null;
    }
    if crate::utils::is_heap_obj(ptr) {
        obj_to_json_value(ptr)
    } else {
        // Raw integer value stored as pointer
        Value::Number(Number::from(ptr as i64))
    }
}

/// Convert a Python runtime object to a serde_json::Value
unsafe fn obj_to_json_value(obj: *mut Obj) -> Value {
    if obj.is_null() {
        return Value::Null;
    }

    match (*obj).header.type_tag {
        TypeTagKind::None => Value::Null,
        TypeTagKind::Bool => {
            let bool_obj = obj as *const BoolObj;
            Value::Bool((*bool_obj).value)
        }
        TypeTagKind::Int => {
            let int_obj = obj as *const IntObj;
            Value::Number(Number::from((*int_obj).value))
        }
        TypeTagKind::Float => {
            let float_obj = obj as *const FloatObj;
            let val = (*float_obj).value;
            match Number::from_f64(val) {
                Some(n) => Value::Number(n),
                None => {
                    // NaN and Infinity are not valid JSON
                    if val.is_nan() {
                        crate::utils::raise_value_error("ValueError: NaN is not valid JSON");
                    } else {
                        crate::utils::raise_value_error("ValueError: Infinity is not valid JSON");
                    }
                }
            }
        }
        TypeTagKind::Str => {
            let s = crate::utils::extract_str_unchecked(obj);
            Value::String(s)
        }
        TypeTagKind::List => {
            let list_obj = obj as *const ListObj;
            let len = (*list_obj).len;
            let elem_tag = (*list_obj).elem_tag;
            let mut arr = Vec::with_capacity(len);
            for i in 0..len {
                let elem = *(*list_obj).data.add(i);
                arr.push(elem_to_json_value(elem, elem_tag));
            }
            Value::Array(arr)
        }
        TypeTagKind::Tuple => {
            let tuple_obj = obj as *const TupleObj;
            let len = (*tuple_obj).len;
            let elem_tag = (*tuple_obj).elem_tag;
            let mut arr = Vec::with_capacity(len);
            for i in 0..len {
                let elem = *(*tuple_obj).data.as_ptr().add(i);
                arr.push(elem_to_json_value(elem, elem_tag));
            }
            Value::Array(arr)
        }
        TypeTagKind::Dict => {
            let dict_obj = obj as *const DictObj;
            let entries_len = (*dict_obj).entries_len;
            let mut map = Map::new();
            for i in 0..entries_len {
                let entry = (*dict_obj).entries.add(i);
                let key = (*entry).key;
                if key.is_null() {
                    continue;
                }
                // Keys must be strings for JSON
                if (*key).header.type_tag != TypeTagKind::Str {
                    crate::utils::raise_value_error(
                        "TypeError: keys must be strings for JSON serialization",
                    );
                }
                let key_str = crate::utils::extract_str_unchecked(key);
                let value = maybe_raw_to_json_value((*entry).value);
                map.insert(key_str, value);
            }
            Value::Object(map)
        }
        _ => {
            crate::utils::raise_value_error("TypeError: Object is not JSON serializable");
        }
    }
}

/// Convert a serde_json::Value to a Python runtime object
unsafe fn json_value_to_obj(value: &Value) -> *mut Obj {
    match value {
        Value::Null => crate::object::none_obj(),
        Value::Bool(b) => crate::boxing::rt_box_bool(if *b { 1 } else { 0 }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                crate::boxing::rt_box_int(i)
            } else if let Some(f) = n.as_f64() {
                crate::boxing::rt_box_float(f)
            } else {
                // Fallback: try as f64
                crate::boxing::rt_box_float(0.0)
            }
        }
        Value::String(s) => make_str_from_rust(s),
        Value::Array(arr) => {
            let list = crate::list::rt_make_list(arr.len() as i64, ELEM_HEAP_OBJ);
            for item in arr {
                let obj = json_value_to_obj(item);
                crate::list::rt_list_push(list, obj);
            }
            list
        }
        Value::Object(map) => {
            let dict = crate::dict::rt_make_dict(map.len() as i64);
            for (key, val) in map {
                let key_obj = make_str_from_rust(key);
                let val_obj = json_value_to_obj(val);
                crate::dict::rt_dict_set(dict, key_obj, val_obj);
            }
            dict
        }
    }
}

/// Serialize a Python object to a JSON string
///
/// # Safety
/// `obj` must be a valid pointer to a Python runtime object (or null for None).
#[no_mangle]
pub unsafe extern "C" fn rt_json_dumps(obj: *mut Obj) -> *mut Obj {
    let value = obj_to_json_value(obj);
    match serde_json::to_string(&value) {
        Ok(s) => {
            // CPython uses ", " for item separator and ": " for key separator
            // We need to add spaces after commas and colons
            // Note: This simple approach only works for JSON without strings containing , or :
            // but serde_json's compact output is predictable enough for this to work
            let formatted = add_json_spaces(&s);
            make_str_from_rust(&formatted)
        }
        Err(e) => {
            let msg = format!("ValueError: {}", e);
            crate::utils::raise_value_error(&msg);
        }
    }
}

/// Add CPython-compatible spaces to JSON output
/// Converts {"key":value} to {"key": value} and [1,2,3] to [1, 2, 3]
fn add_json_spaces(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    let mut in_string = false;
    let mut prev_char = '\0';

    for c in s.chars() {
        // Track string boundaries (handle escaped quotes)
        if c == '"' && prev_char != '\\' {
            in_string = !in_string;
        }

        result.push(c);

        // Add space after comma or colon when not inside a string
        if !in_string && (c == ',' || c == ':') {
            result.push(' ');
        }

        prev_char = c;
    }

    result
}

/// Deserialize a JSON string to a Python object
///
/// # Safety
/// `s` must be a valid pointer to a StrObj containing valid JSON.
#[no_mangle]
pub unsafe extern "C" fn rt_json_loads(s: *mut Obj) -> *mut Obj {
    if s.is_null() {
        crate::utils::raise_value_error("TypeError: the JSON object must be str, not NoneType");
    }

    if (*s).header.type_tag != TypeTagKind::Str {
        crate::utils::raise_value_error("TypeError: the JSON object must be str");
    }

    let json_str = crate::utils::extract_str_unchecked(s);
    match serde_json::from_str::<Value>(&json_str) {
        Ok(value) => json_value_to_obj(&value),
        Err(e) => {
            let msg = format!("json.decoder.JSONDecodeError: {}", e);
            crate::utils::raise_value_error(&msg);
        }
    }
}

/// Serialize a Python object to a JSON file
///
/// # Safety
/// `obj` must be a valid pointer to a Python runtime object.
/// `fp` must be a valid pointer to a FileObj opened for writing.
#[no_mangle]
pub unsafe extern "C" fn rt_json_dump(obj: *mut Obj, fp: *mut Obj) {
    use crate::object::FileObj;
    use std::io::Write;

    if fp.is_null() {
        crate::utils::raise_value_error("TypeError: expected file object, got None");
    }

    if (*fp).header.type_tag != TypeTagKind::File {
        crate::utils::raise_value_error("TypeError: expected file object");
    }

    let file_obj = fp as *mut FileObj;
    if (*file_obj).closed || (*file_obj).handle.is_null() {
        crate::utils::raise_value_error("ValueError: I/O operation on closed file");
    }

    let value = obj_to_json_value(obj);
    match serde_json::to_string(&value) {
        Ok(s) => {
            let handle = &mut *(*file_obj).handle;
            if let Err(e) = handle.write_all(s.as_bytes()) {
                let msg = format!("IOError: {}", e);
                crate::utils::raise_value_error(&msg);
            }
        }
        Err(e) => {
            let msg = format!("ValueError: {}", e);
            crate::utils::raise_value_error(&msg);
        }
    }
}

/// Deserialize a Python object from a JSON file
///
/// # Safety
/// `fp` must be a valid pointer to a FileObj opened for reading.
#[no_mangle]
pub unsafe extern "C" fn rt_json_load(fp: *mut Obj) -> *mut Obj {
    use crate::object::FileObj;
    use std::io::Read;

    if fp.is_null() {
        crate::utils::raise_value_error("TypeError: expected file object, got None");
    }

    if (*fp).header.type_tag != TypeTagKind::File {
        crate::utils::raise_value_error("TypeError: expected file object");
    }

    let file_obj = fp as *mut FileObj;
    if (*file_obj).closed || (*file_obj).handle.is_null() {
        crate::utils::raise_value_error("ValueError: I/O operation on closed file");
    }

    let handle = &mut *(*file_obj).handle;
    let mut content = String::new();
    if let Err(e) = handle.read_to_string(&mut content) {
        let msg = format!("IOError: {}", e);
        crate::utils::raise_value_error(&msg);
    }

    match serde_json::from_str::<Value>(&content) {
        Ok(value) => json_value_to_obj(&value),
        Err(e) => {
            let msg = format!("json.decoder.JSONDecodeError: {}", e);
            crate::utils::raise_value_error(&msg);
        }
    }
}
