//! Runtime object type definitions registry
//!
//! This module provides a declarative registry for runtime object types,
//! following the same pattern as stdlib function definitions. Each object type
//! is defined once with metadata about its fields, display format, and type tag.
//!
//! ## Design Goals
//!
//! 1. **Single Source of Truth**: All object type metadata in one place
//! 2. **Declarative**: Object types are described, not implemented
//! 3. **Metadata-driven**: Lowering and codegen use metadata, not hardcoded logic
//! 4. **DRY**: Eliminates hardcoded match statements across multiple crates
//!
//! ## Usage
//!
//! ```text
//! use pyaot_stdlib_defs::object_types::{lookup_object_type, lookup_object_field};
//!
//! // Look up an object type by TypeTagKind
//! if let Some(obj_def) = lookup_object_type(TypeTagKind::CompletedProcess) {
//!     println!("Display format: {}", obj_def.display_format);
//!
//!     // Look up a field by name
//!     if let Some(field) = obj_def.get_field("returncode") {
//!         println!("Runtime getter: {}", field.runtime_getter);
//!     }
//! }
//! ```

use crate::modules::collections;
use crate::modules::hashlib;
use crate::modules::io;
use crate::modules::re;
use crate::modules::urllib;
use crate::types::{StdlibMethodDef, TypeSpec};
use pyaot_core_defs::runtime_func_def::RT_STRUCT_TIME_GET_FIELD;
use pyaot_core_defs::RuntimeFuncDef;
use pyaot_core_defs::TypeTagKind;

/// Field definition for a runtime object type
///
/// Describes a single field/attribute of an object type, including
/// its name, type, and the runtime function to call for accessing it.
#[derive(Debug, Clone, Copy)]
pub struct ObjectFieldDef {
    /// Field name as it appears in Python (e.g., "returncode", "tm_year")
    pub name: &'static str,
    /// Runtime getter function name (e.g., "rt_struct_time_get_field")
    pub runtime_getter: &'static str,
    /// Field type
    pub field_type: TypeSpec,
    /// Codegen descriptor for the generic RuntimeFunc::Call handler
    pub codegen: RuntimeFuncDef,
    /// Optional constant field index passed as extra argument to a generic getter.
    /// When `Some(i)`, lowering emits `Constant::Int(i)` as the second argument
    /// alongside the object pointer.
    pub field_index: Option<i64>,
}

/// Display format specification for an object type
///
/// Describes how to format an object for printing/repr.
/// Can be either a static string or a format that includes field values.
#[derive(Debug, Clone, Copy)]
pub enum DisplayFormat {
    /// Simple static format (e.g., "<match object>")
    Static(&'static str),
    /// Format with a single field value (e.g., "CompletedProcess(returncode={})")
    /// The str is the format template, the usize is the field index to use
    WithField(&'static str, usize),
    /// Complex format - delegate to custom runtime function
    Custom(&'static str),
}

/// Object type definition
///
/// Defines a runtime object type with its fields, methods, and metadata.
/// This is the single source of truth for how object types behave.
#[derive(Debug, Clone, Copy)]
pub struct ObjectTypeDef {
    /// The type tag for this object (from core-defs)
    pub type_tag: TypeTagKind,
    /// Object type name (e.g., "CompletedProcess")
    pub name: &'static str,
    /// Fields/attributes of this object
    pub fields: &'static [ObjectFieldDef],
    /// Methods available on this object
    pub methods: &'static [&'static StdlibMethodDef],
    /// How to display this object in print/repr
    pub display_format: DisplayFormat,
}

impl ObjectTypeDef {
    /// Get a field by name
    pub const fn get_field(&self, name: &str) -> Option<&'static ObjectFieldDef> {
        let mut i = 0;
        while i < self.fields.len() {
            if const_str_eq(self.fields[i].name, name) {
                return Some(&self.fields[i]);
            }
            i += 1;
        }
        None
    }

    /// Get a method by name
    pub const fn get_method(&self, name: &str) -> Option<&'static StdlibMethodDef> {
        let mut i = 0;
        while i < self.methods.len() {
            if const_str_eq(self.methods[i].name, name) {
                return Some(self.methods[i]);
            }
            i += 1;
        }
        None
    }
}

use crate::types::const_str_eq;

use crate::types::{TYPE_DICT_STR_STR, TYPE_LIST_STR, TYPE_OPTIONAL_STR};

// =============================================================================
// CompletedProcess object (subprocess module)
// =============================================================================

static COMPLETED_PROCESS_FIELDS: &[ObjectFieldDef] = &[
    ObjectFieldDef {
        name: "args",
        runtime_getter: "rt_completed_process_get_args",
        field_type: TYPE_LIST_STR,
        codegen: RuntimeFuncDef::unary_to_i64("rt_completed_process_get_args"),
        field_index: None,
    },
    ObjectFieldDef {
        name: "returncode",
        runtime_getter: "rt_completed_process_get_returncode",
        field_type: TypeSpec::Int,
        codegen: RuntimeFuncDef::unary_to_i64("rt_completed_process_get_returncode"),
        field_index: None,
    },
    ObjectFieldDef {
        name: "stdout",
        runtime_getter: "rt_completed_process_get_stdout",
        field_type: TYPE_OPTIONAL_STR,
        codegen: RuntimeFuncDef::unary_to_i64("rt_completed_process_get_stdout"),
        field_index: None,
    },
    ObjectFieldDef {
        name: "stderr",
        runtime_getter: "rt_completed_process_get_stderr",
        field_type: TYPE_OPTIONAL_STR,
        codegen: RuntimeFuncDef::unary_to_i64("rt_completed_process_get_stderr"),
        field_index: None,
    },
];

pub static COMPLETED_PROCESS: ObjectTypeDef = ObjectTypeDef {
    type_tag: TypeTagKind::CompletedProcess,
    name: "CompletedProcess",
    fields: COMPLETED_PROCESS_FIELDS,
    methods: &[], // No methods, only field accessors
    display_format: DisplayFormat::WithField("CompletedProcess(returncode={})", 1),
};

// =============================================================================
// StructTime object (time module)
// =============================================================================

static STRUCT_TIME_FIELDS: &[ObjectFieldDef] = &[
    ObjectFieldDef {
        name: "tm_year",
        runtime_getter: "rt_struct_time_get_field",
        field_type: TypeSpec::Int,
        codegen: RT_STRUCT_TIME_GET_FIELD,
        field_index: Some(0),
    },
    ObjectFieldDef {
        name: "tm_mon",
        runtime_getter: "rt_struct_time_get_field",
        field_type: TypeSpec::Int,
        codegen: RT_STRUCT_TIME_GET_FIELD,
        field_index: Some(1),
    },
    ObjectFieldDef {
        name: "tm_mday",
        runtime_getter: "rt_struct_time_get_field",
        field_type: TypeSpec::Int,
        codegen: RT_STRUCT_TIME_GET_FIELD,
        field_index: Some(2),
    },
    ObjectFieldDef {
        name: "tm_hour",
        runtime_getter: "rt_struct_time_get_field",
        field_type: TypeSpec::Int,
        codegen: RT_STRUCT_TIME_GET_FIELD,
        field_index: Some(3),
    },
    ObjectFieldDef {
        name: "tm_min",
        runtime_getter: "rt_struct_time_get_field",
        field_type: TypeSpec::Int,
        codegen: RT_STRUCT_TIME_GET_FIELD,
        field_index: Some(4),
    },
    ObjectFieldDef {
        name: "tm_sec",
        runtime_getter: "rt_struct_time_get_field",
        field_type: TypeSpec::Int,
        codegen: RT_STRUCT_TIME_GET_FIELD,
        field_index: Some(5),
    },
    ObjectFieldDef {
        name: "tm_wday",
        runtime_getter: "rt_struct_time_get_field",
        field_type: TypeSpec::Int,
        codegen: RT_STRUCT_TIME_GET_FIELD,
        field_index: Some(6),
    },
    ObjectFieldDef {
        name: "tm_yday",
        runtime_getter: "rt_struct_time_get_field",
        field_type: TypeSpec::Int,
        codegen: RT_STRUCT_TIME_GET_FIELD,
        field_index: Some(7),
    },
    ObjectFieldDef {
        name: "tm_isdst",
        runtime_getter: "rt_struct_time_get_field",
        field_type: TypeSpec::Int,
        codegen: RT_STRUCT_TIME_GET_FIELD,
        field_index: Some(8),
    },
];

pub static STRUCT_TIME: ObjectTypeDef = ObjectTypeDef {
    type_tag: TypeTagKind::StructTime,
    name: "struct_time",
    fields: STRUCT_TIME_FIELDS,
    methods: &[], // No methods, only field accessors
    display_format: DisplayFormat::Custom("rt_struct_time_repr"),
};

// =============================================================================
// Match object (re module)
// =============================================================================

// Match has methods, not field getters, so fields are empty
static MATCH_FIELDS: &[ObjectFieldDef] = &[];

// Match object methods from re module
static MATCH_METHODS: &[&StdlibMethodDef] = &[
    &re::MATCH_GROUP,
    &re::MATCH_START,
    &re::MATCH_END,
    &re::MATCH_GROUPS,
    &re::MATCH_SPAN,
];

pub static MATCH: ObjectTypeDef = ObjectTypeDef {
    type_tag: TypeTagKind::Match,
    name: "Match",
    fields: MATCH_FIELDS,
    methods: MATCH_METHODS,
    display_format: DisplayFormat::Static("<match object>"),
};

// =============================================================================
// File object (built-in)
// =============================================================================

// File has methods, not field getters, so fields are empty
static FILE_FIELDS: &[ObjectFieldDef] = &[];

pub static FILE: ObjectTypeDef = ObjectTypeDef {
    type_tag: TypeTagKind::File,
    name: "File",
    fields: FILE_FIELDS,
    methods: &[], // File methods use separate dispatch (complex I/O semantics)
    display_format: DisplayFormat::Custom("rt_file_repr"),
};

// =============================================================================
// ParseResult object (urllib.parse module)
// =============================================================================

static PARSE_RESULT_FIELDS: &[ObjectFieldDef] = &[
    ObjectFieldDef {
        name: "scheme",
        runtime_getter: "rt_parse_result_get_scheme",
        field_type: TypeSpec::Str,
        codegen: RuntimeFuncDef::unary_to_i64("rt_parse_result_get_scheme"),
        field_index: None,
    },
    ObjectFieldDef {
        name: "netloc",
        runtime_getter: "rt_parse_result_get_netloc",
        field_type: TypeSpec::Str,
        codegen: RuntimeFuncDef::unary_to_i64("rt_parse_result_get_netloc"),
        field_index: None,
    },
    ObjectFieldDef {
        name: "path",
        runtime_getter: "rt_parse_result_get_path",
        field_type: TypeSpec::Str,
        codegen: RuntimeFuncDef::unary_to_i64("rt_parse_result_get_path"),
        field_index: None,
    },
    ObjectFieldDef {
        name: "params",
        runtime_getter: "rt_parse_result_get_params",
        field_type: TypeSpec::Str,
        codegen: RuntimeFuncDef::unary_to_i64("rt_parse_result_get_params"),
        field_index: None,
    },
    ObjectFieldDef {
        name: "query",
        runtime_getter: "rt_parse_result_get_query",
        field_type: TypeSpec::Str,
        codegen: RuntimeFuncDef::unary_to_i64("rt_parse_result_get_query"),
        field_index: None,
    },
    ObjectFieldDef {
        name: "fragment",
        runtime_getter: "rt_parse_result_get_fragment",
        field_type: TypeSpec::Str,
        codegen: RuntimeFuncDef::unary_to_i64("rt_parse_result_get_fragment"),
        field_index: None,
    },
];

// ParseResult object methods from urllib module
static PARSE_RESULT_METHODS: &[&StdlibMethodDef] = &[&urllib::PARSE_RESULT_GETURL];

pub static PARSE_RESULT: ObjectTypeDef = ObjectTypeDef {
    type_tag: TypeTagKind::ParseResult,
    name: "ParseResult",
    fields: PARSE_RESULT_FIELDS,
    methods: PARSE_RESULT_METHODS,
    display_format: DisplayFormat::Custom("rt_parse_result_repr"),
};

// =============================================================================
// HttpResponse object (urllib.request module)
// =============================================================================

// Standard `http.client.HTTPResponse` fields (`.status`, `.url`, `.headers`)
// plus pyaot-specific conveniences that mirror the pip `requests.Response`
// surface (`.status_code`, `.ok`, `.content`, `.text`). The extensions make
// pyaot's bundled `site-packages/requests` a drop-in for the real pip
// `requests` library: user code writing `resp.status_code` / `resp.text` /
// `resp.json()` works against either package without changes. On plain
// CPython without pip-installed requests, `urlopen()` still returns a
// standard HTTPResponse (no extension) — users should prefer `.status` /
// `.read()` for CPython-urllib-only portability.
static HTTP_RESPONSE_FIELDS: &[ObjectFieldDef] = &[
    ObjectFieldDef {
        name: "status",
        runtime_getter: "rt_http_response_get_status",
        field_type: TypeSpec::Int,
        codegen: RuntimeFuncDef::unary_to_i64("rt_http_response_get_status"),
        field_index: None,
    },
    // `.status_code` — alias matching the pip `requests` library.
    ObjectFieldDef {
        name: "status_code",
        runtime_getter: "rt_http_response_get_status",
        field_type: TypeSpec::Int,
        codegen: RuntimeFuncDef::unary_to_i64("rt_http_response_get_status"),
        field_index: None,
    },
    ObjectFieldDef {
        name: "url",
        runtime_getter: "rt_http_response_get_url",
        field_type: TypeSpec::Str,
        codegen: RuntimeFuncDef::unary_to_i64("rt_http_response_get_url"),
        field_index: None,
    },
    ObjectFieldDef {
        name: "headers",
        runtime_getter: "rt_http_response_get_headers",
        field_type: TYPE_DICT_STR_STR,
        codegen: RuntimeFuncDef::unary_to_i64("rt_http_response_get_headers"),
        field_index: None,
    },
    // `.ok` — 200 <= status < 300 (requests-library convention).
    ObjectFieldDef {
        name: "ok",
        runtime_getter: "rt_http_response_get_ok",
        field_type: TypeSpec::Bool,
        codegen: RuntimeFuncDef::unary_to_i8("rt_http_response_get_ok"),
        field_index: None,
    },
    // `.content` — raw response body as bytes.
    ObjectFieldDef {
        name: "content",
        runtime_getter: "rt_http_response_read",
        field_type: TypeSpec::Bytes,
        codegen: RuntimeFuncDef::unary_to_i64("rt_http_response_read"),
        field_index: None,
    },
    // `.text` — response body decoded as UTF-8 str.
    ObjectFieldDef {
        name: "text",
        runtime_getter: "rt_http_response_get_text",
        field_type: TypeSpec::Str,
        codegen: RuntimeFuncDef::unary_to_i64("rt_http_response_get_text"),
        field_index: None,
    },
];

// HttpResponse object methods — CPython-standard urllib methods plus the
// requests-library-compatible `.json()` parser.
static HTTP_RESPONSE_METHODS: &[&StdlibMethodDef] = &[
    &urllib::HTTP_RESPONSE_READ,
    &urllib::HTTP_RESPONSE_GETURL,
    &urllib::HTTP_RESPONSE_GETCODE,
    &urllib::HTTP_RESPONSE_JSON,
];

pub static HTTP_RESPONSE: ObjectTypeDef = ObjectTypeDef {
    type_tag: TypeTagKind::HttpResponse,
    name: "HTTPResponse",
    fields: HTTP_RESPONSE_FIELDS,
    methods: HTTP_RESPONSE_METHODS,
    display_format: DisplayFormat::Custom("rt_http_response_repr"),
};

// =============================================================================
// Request object (urllib.request module)
// =============================================================================

static REQUEST_FIELDS: &[ObjectFieldDef] = &[
    // `full_url` — CPython-compatible alias for the URL.
    ObjectFieldDef {
        name: "full_url",
        runtime_getter: "rt_request_get_url",
        field_type: TypeSpec::Str,
        codegen: RuntimeFuncDef::unary_to_i64("rt_request_get_url"),
        field_index: None,
    },
    // `data` — request body bytes (or None if unset).
    ObjectFieldDef {
        name: "data",
        runtime_getter: "rt_request_get_data",
        field_type: TypeSpec::Bytes,
        codegen: RuntimeFuncDef::unary_to_i64("rt_request_get_data"),
        field_index: None,
    },
    ObjectFieldDef {
        name: "headers",
        runtime_getter: "rt_request_get_headers",
        field_type: TYPE_DICT_STR_STR,
        codegen: RuntimeFuncDef::unary_to_i64("rt_request_get_headers"),
        field_index: None,
    },
    ObjectFieldDef {
        name: "method",
        runtime_getter: "rt_request_get_method",
        field_type: TypeSpec::Str,
        codegen: RuntimeFuncDef::unary_to_i64("rt_request_get_method"),
        field_index: None,
    },
];

static REQUEST_METHODS: &[&StdlibMethodDef] = &[];

pub static REQUEST: ObjectTypeDef = ObjectTypeDef {
    type_tag: TypeTagKind::Request,
    name: "Request",
    fields: REQUEST_FIELDS,
    methods: REQUEST_METHODS,
    display_format: DisplayFormat::Static("<urllib.request.Request object>"),
};

// =============================================================================
// Hash object (hashlib module)
// =============================================================================

static HASH_FIELDS: &[ObjectFieldDef] = &[];

static HASH_METHODS: &[&StdlibMethodDef] = &[&hashlib::HASH_HEXDIGEST, &hashlib::HASH_DIGEST];

pub static HASH: ObjectTypeDef = ObjectTypeDef {
    type_tag: TypeTagKind::Hash,
    name: "HASH",
    fields: HASH_FIELDS,
    methods: HASH_METHODS,
    display_format: DisplayFormat::Static("<hashlib.HASH object>"),
};

// =============================================================================
// StringIO object (io module)
// =============================================================================

static STRINGIO_FIELDS: &[ObjectFieldDef] = &[];

static STRINGIO_METHODS: &[&StdlibMethodDef] = &[
    &io::STRINGIO_WRITE,
    &io::STRINGIO_READ,
    &io::STRINGIO_READLINE,
    &io::STRINGIO_GETVALUE,
    &io::STRINGIO_SEEK,
    &io::STRINGIO_TELL,
    &io::STRINGIO_CLOSE,
    &io::STRINGIO_TRUNCATE,
];

pub static STRINGIO: ObjectTypeDef = ObjectTypeDef {
    type_tag: TypeTagKind::StringIO,
    name: "StringIO",
    fields: STRINGIO_FIELDS,
    methods: STRINGIO_METHODS,
    display_format: DisplayFormat::Static("<_io.StringIO object>"),
};

// =============================================================================
// BytesIO object (io module)
// =============================================================================

static BYTESIO_FIELDS: &[ObjectFieldDef] = &[];

static BYTESIO_METHODS: &[&StdlibMethodDef] = &[
    &io::BYTESIO_WRITE,
    &io::BYTESIO_READ,
    &io::BYTESIO_GETVALUE,
    &io::BYTESIO_SEEK,
    &io::BYTESIO_TELL,
    &io::BYTESIO_CLOSE,
];

pub static BYTESIO: ObjectTypeDef = ObjectTypeDef {
    type_tag: TypeTagKind::BytesIO,
    name: "BytesIO",
    fields: BYTESIO_FIELDS,
    methods: BYTESIO_METHODS,
    display_format: DisplayFormat::Static("<_io.BytesIO object>"),
};

// =============================================================================
// Registry
// =============================================================================

// =============================================================================
// Counter object (collections module)
// =============================================================================

static COUNTER_FIELDS: &[ObjectFieldDef] = &[];

static COUNTER_METHODS: &[&StdlibMethodDef] = &[
    &collections::COUNTER_MOST_COMMON,
    &collections::COUNTER_TOTAL,
    &collections::COUNTER_UPDATE,
    &collections::COUNTER_SUBTRACT,
];

pub static COUNTER: ObjectTypeDef = ObjectTypeDef {
    type_tag: TypeTagKind::Counter,
    name: "Counter",
    fields: COUNTER_FIELDS,
    methods: COUNTER_METHODS,
    display_format: DisplayFormat::Custom("rt_counter_repr"),
};

// =============================================================================
// Deque object (collections module)
// =============================================================================

static DEQUE_FIELDS: &[ObjectFieldDef] = &[];

static DEQUE_METHODS: &[&StdlibMethodDef] = &[
    &collections::DEQUE_APPEND,
    &collections::DEQUE_APPENDLEFT,
    &collections::DEQUE_POP,
    &collections::DEQUE_POPLEFT,
    &collections::DEQUE_EXTEND,
    &collections::DEQUE_EXTENDLEFT,
    &collections::DEQUE_ROTATE,
    &collections::DEQUE_CLEAR,
    &collections::DEQUE_REVERSE,
    &collections::DEQUE_COPY,
    &collections::DEQUE_COUNT,
];

pub static DEQUE: ObjectTypeDef = ObjectTypeDef {
    type_tag: TypeTagKind::Deque,
    name: "deque",
    fields: DEQUE_FIELDS,
    methods: DEQUE_METHODS,
    display_format: DisplayFormat::Custom("rt_deque_repr"),
};

/// All defined object types
pub static ALL_OBJECT_TYPES: &[&ObjectTypeDef] = &[
    &COMPLETED_PROCESS,
    &STRUCT_TIME,
    &MATCH,
    &FILE,
    &PARSE_RESULT,
    &HTTP_RESPONSE,
    &REQUEST,
    &HASH,
    &STRINGIO,
    &BYTESIO,
    &COUNTER,
    &DEQUE,
];

/// Look up an object type definition by TypeTagKind
pub fn lookup_object_type(type_tag: TypeTagKind) -> Option<&'static ObjectTypeDef> {
    ALL_OBJECT_TYPES
        .iter()
        .find(|def| def.type_tag == type_tag)
        .copied()
}

/// Look up an object field by type tag and field name
pub fn lookup_object_field(
    type_tag: TypeTagKind,
    field_name: &str,
) -> Option<&'static ObjectFieldDef> {
    lookup_object_type(type_tag).and_then(|def| def.get_field(field_name))
}

/// Look up an object method by type tag and method name
pub fn lookup_object_method(
    type_tag: TypeTagKind,
    method_name: &str,
) -> Option<&'static StdlibMethodDef> {
    lookup_object_type(type_tag).and_then(|def| def.get_method(method_name))
}

/// Look up an object type definition by type name.
/// Type names: "struct_time", "CompletedProcess", "Match", "File"
pub fn lookup_object_type_by_name(type_name: &str) -> Option<&'static ObjectTypeDef> {
    ALL_OBJECT_TYPES
        .iter()
        .find(|def| def.name == type_name)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lookup_object_type() {
        let obj = lookup_object_type(TypeTagKind::CompletedProcess);
        assert!(obj.is_some());
        assert_eq!(obj.unwrap().name, "CompletedProcess");

        let obj = lookup_object_type(TypeTagKind::StructTime);
        assert!(obj.is_some());
        assert_eq!(obj.unwrap().name, "struct_time");
    }

    #[test]
    fn test_lookup_object_field() {
        let field = lookup_object_field(TypeTagKind::CompletedProcess, "returncode");
        assert!(field.is_some());
        assert_eq!(
            field.unwrap().runtime_getter,
            "rt_completed_process_get_returncode"
        );

        let field = lookup_object_field(TypeTagKind::StructTime, "tm_year");
        assert!(field.is_some());
        assert_eq!(field.unwrap().runtime_getter, "rt_struct_time_get_field");
        assert_eq!(field.unwrap().field_index, Some(0));

        let field = lookup_object_field(TypeTagKind::CompletedProcess, "nonexistent");
        assert!(field.is_none());
    }

    #[test]
    fn test_get_field() {
        let field = COMPLETED_PROCESS.get_field("args");
        assert!(field.is_some());
        assert_eq!(field.unwrap().name, "args");

        let field = STRUCT_TIME.get_field("tm_mon");
        assert!(field.is_some());
        assert_eq!(field.unwrap().name, "tm_mon");
    }

    #[test]
    fn test_all_fields_unique() {
        for obj_def in ALL_OBJECT_TYPES {
            let mut seen = std::collections::HashSet::new();
            for field in obj_def.fields {
                assert!(
                    seen.insert(field.name),
                    "Duplicate field '{}' in {}",
                    field.name,
                    obj_def.name
                );
            }
        }
    }

    #[test]
    fn test_all_tag_kinds_unique() {
        let mut seen = std::collections::HashSet::new();
        for obj_def in ALL_OBJECT_TYPES {
            assert!(
                seen.insert(obj_def.type_tag),
                "Duplicate type tag {:?} for {}",
                obj_def.type_tag,
                obj_def.name
            );
        }
    }

    #[test]
    fn test_get_method() {
        // Match object has methods
        let method = MATCH.get_method("group");
        assert!(method.is_some());
        assert_eq!(method.unwrap().name, "group");
        assert_eq!(method.unwrap().runtime_name, "rt_match_group");

        let method = MATCH.get_method("start");
        assert!(method.is_some());
        assert_eq!(method.unwrap().name, "start");

        let method = MATCH.get_method("nonexistent");
        assert!(method.is_none());

        // CompletedProcess has no methods
        let method = COMPLETED_PROCESS.get_method("anything");
        assert!(method.is_none());
    }

    #[test]
    fn test_lookup_object_method() {
        let method = lookup_object_method(TypeTagKind::Match, "group");
        assert!(method.is_some());
        assert_eq!(method.unwrap().runtime_name, "rt_match_group");

        let method = lookup_object_method(TypeTagKind::Match, "end");
        assert!(method.is_some());
        assert_eq!(method.unwrap().runtime_name, "rt_match_end");

        let method = lookup_object_method(TypeTagKind::Match, "groups");
        assert!(method.is_some());
        assert_eq!(method.unwrap().runtime_name, "rt_match_groups");

        let method = lookup_object_method(TypeTagKind::Match, "span");
        assert!(method.is_some());
        assert_eq!(method.unwrap().runtime_name, "rt_match_span");

        // Non-existent method
        let method = lookup_object_method(TypeTagKind::Match, "invalid");
        assert!(method.is_none());

        // Type without methods
        let method = lookup_object_method(TypeTagKind::CompletedProcess, "any");
        assert!(method.is_none());
    }

    #[test]
    fn test_all_methods_unique() {
        for obj_def in ALL_OBJECT_TYPES {
            let mut seen = std::collections::HashSet::new();
            for method in obj_def.methods {
                assert!(
                    seen.insert(method.name),
                    "Duplicate method '{}' in {}",
                    method.name,
                    obj_def.name
                );
            }
        }
    }
}
