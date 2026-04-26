//! JSON module runtime support
//!
//! Provides:
//! - json.dumps(obj) -> str (serialize Python object to JSON string)
//! - json.loads(s) -> obj (deserialize JSON string to Python object)
//! - json.dump(obj, fp) -> None (serialize Python object to JSON file)
//! - json.load(fp) -> obj (deserialize JSON object from JSON file)

use crate::object::{DictObj, FloatObj, ListObj, Obj, TupleObj, TypeTagKind};
use crate::utils::make_str_from_rust;
use pyaot_core_defs::Value as RuntimeValue;
use serde_json::{Map, Number, Value};

/// Convert a list/tuple element to JSON, dispatching on Value tag
unsafe fn elem_to_json_value(elem: *mut Obj) -> Value {
    let v = RuntimeValue(elem as u64);
    if v.is_int() {
        Value::Number(Number::from(v.unwrap_int()))
    } else if v.is_bool() {
        Value::Bool(v.unwrap_bool())
    } else if v.is_none() {
        Value::Null
    } else {
        obj_to_json_value(elem)
    }
}

/// Convert a dict value pointer to JSON
/// Dict values may be Value-tagged primitives or heap object pointers.
unsafe fn maybe_raw_to_json_value(ptr: *mut Obj) -> Value {
    obj_to_json_value(ptr)
}

/// Convert a Python runtime object to a serde_json::Value
unsafe fn obj_to_json_value(obj: *mut Obj) -> Value {
    if obj.is_null() {
        return Value::Null;
    }

    // Check Value-tagged primitives before heap pointer dereference.
    let rv = RuntimeValue(obj as u64);
    if rv.is_int() {
        return Value::Number(Number::from(rv.unwrap_int()));
    }
    if rv.is_bool() {
        return Value::Bool(rv.unwrap_bool());
    }
    if rv.is_none() {
        return Value::Null;
    }

    match (*obj).header.type_tag {
        TypeTagKind::None => Value::Null,
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
            let list_obj = obj as *mut ListObj;
            let len = (*list_obj).len;
            let mut arr = Vec::with_capacity(len);
            for i in 0..len {
                let elem = (*(*list_obj).data.add(i)).0 as *mut Obj;
                arr.push(elem_to_json_value(elem));
            }
            Value::Array(arr)
        }
        TypeTagKind::Tuple => {
            let tuple_obj = obj as *const TupleObj;
            let len = (*tuple_obj).len;
            let mut arr = Vec::with_capacity(len);
            for i in 0..len {
                let elem = *(*tuple_obj).data.as_ptr().add(i);
                arr.push(elem_to_json_value(elem.0 as *mut Obj));
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
                if key.0 == 0 {
                    continue;
                }
                let key_ptr = key.0 as *mut Obj;
                // Keys must be strings for JSON
                if (*key_ptr).header.type_tag != TypeTagKind::Str {
                    crate::utils::raise_value_error(
                        "TypeError: keys must be strings for JSON serialization",
                    );
                }
                let key_str = crate::utils::extract_str_unchecked(key_ptr);
                let value = maybe_raw_to_json_value((*entry).value.0 as *mut Obj);
                map.insert(key_str, value);
            }
            Value::Object(map)
        }
        _ => {
            crate::utils::raise_value_error("TypeError: Object is not JSON serializable");
        }
    }
}

/// Maximum JSON nesting depth (matches serde_json default of 128 plus headroom)
const MAX_JSON_DEPTH: u32 = 200;

/// Convert a serde_json::Value to a Python runtime object.
///
/// `depth` tracks the current recursion depth to prevent stack overflow on
/// deeply-nested JSON inputs. The public entry point passes `depth = 0`.
unsafe fn json_value_to_obj(value: &Value, depth: u32) -> *mut Obj {
    if depth >= MAX_JSON_DEPTH {
        crate::utils::raise_value_error("ValueError: Exceeded maximum JSON nesting depth");
    }

    match value {
        Value::Null => crate::object::none_obj(),
        Value::Bool(b) => pyaot_core_defs::Value::from_bool(*b).0 as *mut crate::object::Obj,
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                pyaot_core_defs::Value::from_int(i).0 as *mut crate::object::Obj
            } else if let Some(f) = n.as_f64() {
                crate::boxing::rt_box_float(f)
            } else {
                // Fallback: try as f64
                crate::boxing::rt_box_float(0.0)
            }
        }
        Value::String(s) => make_str_from_rust(s),
        Value::Array(arr) => {
            let list = crate::list::rt_make_list(arr.len() as i64);
            // Root the list so GC triggered by recursive allocs does not collect it.
            let mut roots: [*mut Obj; 1] = [list];
            let mut frame = crate::gc::ShadowFrame {
                prev: std::ptr::null_mut(),
                nroots: 1,
                roots: roots.as_mut_ptr(),
            };
            crate::gc::gc_push(&mut frame);
            for item in arr {
                let obj = json_value_to_obj(item, depth + 1);
                crate::list::rt_list_push(roots[0], obj);
            }
            crate::gc::gc_pop();
            roots[0]
        }
        Value::Object(map) => {
            let dict = crate::dict::rt_make_dict(map.len() as i64);
            // Root dict so GC triggered by key/value allocs does not collect it.
            // Also root key_obj across the value alloc (index 1 used as scratch).
            let mut roots: [*mut Obj; 2] = [dict, std::ptr::null_mut()];
            let mut frame = crate::gc::ShadowFrame {
                prev: std::ptr::null_mut(),
                nroots: 2,
                roots: roots.as_mut_ptr(),
            };
            crate::gc::gc_push(&mut frame);
            for (key, val) in map {
                let key_obj = make_str_from_rust(key);
                roots[1] = key_obj; // root key across the recursive value alloc
                let val_obj = json_value_to_obj(val, depth + 1);
                crate::dict::rt_dict_set(roots[0], roots[1], val_obj);
                roots[1] = std::ptr::null_mut();
            }
            crate::gc::gc_pop();
            roots[0]
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
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "ValueError: {}",
                e
            );
        }
    }
}

/// Add CPython-compatible spaces to JSON output
/// Converts {"key":value} to {"key": value} and [1,2,3] to [1, 2, 3]
fn add_json_spaces(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    let mut in_string = false;
    let mut escape = false;

    for c in s.chars() {
        if in_string {
            if escape {
                // This character is escaped, skip escape tracking
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
        }

        result.push(c);

        // Add space after comma or colon when not inside a string
        if !in_string && !escape && (c == ',' || c == ':') {
            result.push(' ');
        }
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
        Ok(value) => json_value_to_obj(&value, 0),
        Err(e) => {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "json.decoder.JSONDecodeError: {}",
                e
            );
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
            let formatted = add_json_spaces(&s);
            let handle = &mut *(*file_obj).handle;
            if let Err(e) = handle.write_all(formatted.as_bytes()) {
                raise_exc!(
                    crate::exceptions::ExceptionType::ValueError,
                    "IOError: {}",
                    e
                );
            }
        }
        Err(e) => {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "ValueError: {}",
                e
            );
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
        raise_exc!(
            crate::exceptions::ExceptionType::ValueError,
            "IOError: {}",
            e
        );
    }

    match serde_json::from_str::<Value>(&content) {
        Ok(value) => json_value_to_obj(&value, 0),
        Err(e) => {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "json.decoder.JSONDecodeError: {}",
                e
            );
        }
    }
}
