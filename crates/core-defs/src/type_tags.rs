//! Central definition of runtime type tags.
//!
//! This module provides a single source of truth for all runtime type tags,
//! their numeric values (0-18), and lookup functions. The `define_type_tags!` macro
//! generates the `TypeTagKind` enum and all lookup functions at compile time.
//!
//! # Usage
//!
//! ```
//! use pyaot_core_defs::{TypeTagKind, type_tag_to_name};
//!
//! // Use the enum directly
//! let tag = TypeTagKind::List;
//! assert_eq!(tag.tag(), 5);
//! assert_eq!(tag.name(), "List");
//! assert_eq!(tag.type_class(), "<class 'list'>");
//! assert_eq!(tag.type_name(), "list");
//!
//! // Lookup by tag
//! assert_eq!(TypeTagKind::from_tag(5), Some(TypeTagKind::List));
//!
//! // Legacy function
//! assert_eq!(type_tag_to_name(5), Some("List"));
//! ```
//!
//! # Adding New Type Tags
//!
//! To add a new type tag, simply add a new entry to `define_type_tags!`
//! macro invocation below. Everything else is generated automatically by the macro.
//! Both the `types` and `runtime` crates will pick up the change automatically.

#![forbid(unsafe_code)]

/// Macro that defines the canonical type tag list and generates:
/// - `TypeTagKind` enum with all type tag variants
/// - `TYPE_TAG_COUNT` constant
/// - Lookup functions for tags and names
///
/// Each entry has: variant = tag_number => "debug_name" => "type_class" => "type_name"
/// - debug_name: Internal name (e.g., "StructTime")
/// - type_class: Python type() result (e.g., "<class 'time.struct_time'>")
/// - type_name: Short Python type name (e.g., "time.struct_time")
///
/// This ensures compile-time generation with zero runtime overhead.
macro_rules! define_type_tags {
    ($($variant:ident = $tag:expr => $name:literal => $type_class:literal => $type_name:literal),* $(,)?) => {
        /// Runtime type tag kind enum - single source of truth.
        /// These tags identify the type of heap objects at runtime.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum TypeTagKind {
            $($variant = $tag,)*
        }

        impl TypeTagKind {
            /// Get the numeric tag (0-N) for this type.
            #[inline]
            pub const fn tag(self) -> u8 {
                self as u8
            }

            /// Get the internal debug name for this type tag (e.g., "StructTime").
            #[inline]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)*
                }
            }

            /// Get the Python type() class string (e.g., "<class 'time.struct_time'>").
            /// Used by the `type()` builtin function.
            #[inline]
            pub const fn type_class(self) -> &'static str {
                match self {
                    $(Self::$variant => $type_class,)*
                }
            }

            /// Get the short Python type name (e.g., "time.struct_time").
            /// Used for error messages and repr output.
            #[inline]
            pub const fn type_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $type_name,)*
                }
            }

            /// Create from a numeric tag.
            /// Returns `None` if the tag is invalid.
            #[inline]
            pub const fn from_tag(tag: u8) -> Option<Self> {
                match tag {
                    $($tag => Some(Self::$variant),)*
                    _ => None,
                }
            }

            /// Create from a type name.
            /// Returns `None` if the name is not a valid type tag.
            #[inline]
            pub fn from_name(name: &str) -> Option<Self> {
                match name {
                    $($name => Some(Self::$variant),)*
                    _ => None,
                }
            }

            /// Array of all type tag kinds.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];
        }

        impl std::fmt::Display for TypeTagKind {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.name())
            }
        }

        /// Number of type tags.
        pub const TYPE_TAG_COUNT: u8 = {
            let mut count = 0u8;
            $(let _ = $tag; count += 1;)*
            count
        };

        /// Lookup type tag name by tag value.
        #[inline]
        pub fn type_tag_to_name(tag: u8) -> Option<&'static str> {
            TypeTagKind::from_tag(tag).map(|k| k.name())
        }

        /// Check if a name is a valid type tag name.
        #[inline]
        pub fn is_type_tag_name(name: &str) -> bool {
            TypeTagKind::from_name(name).is_some()
        }
    };
}

// =============================================================================
// SINGLE SOURCE OF TRUTH FOR RUNTIME TYPE TAGS
// =============================================================================
//
// This is the canonical definition used by both the compiler (types crate)
// and runtime crate. Adding a new type tag here automatically makes it
// available everywhere.
//
// Format: Variant = tag => "debug_name" => "type_class" => "type_name"

define_type_tags! {
    Int = 0 => "Int" => "<class 'int'>" => "int",
    Float = 1 => "Float" => "<class 'float'>" => "float",
    Bool = 2 => "Bool" => "<class 'bool'>" => "bool",
    Str = 3 => "Str" => "<class 'str'>" => "str",
    None = 4 => "None" => "<class 'NoneType'>" => "NoneType",
    List = 5 => "List" => "<class 'list'>" => "list",
    Tuple = 6 => "Tuple" => "<class 'tuple'>" => "tuple",
    Dict = 7 => "Dict" => "<class 'dict'>" => "dict",
    Instance = 8 => "Instance" => "<class 'object'>" => "object",
    Iterator = 9 => "Iterator" => "<class 'iterator'>" => "iterator",
    Set = 10 => "Set" => "<class 'set'>" => "set",
    Bytes = 11 => "Bytes" => "<class 'bytes'>" => "bytes",
    Cell = 12 => "Cell" => "<class 'cell'>" => "cell",
    Generator = 13 => "Generator" => "<class 'generator'>" => "generator",
    Match = 14 => "Match" => "<class 're.Match'>" => "re.Match",
    File = 15 => "File" => "<class '_io.TextIOWrapper'>" => "TextIOWrapper",
    StringBuilder = 16 => "StringBuilder" => "<class 'StringBuilder'>" => "StringBuilder",
    StructTime = 17 => "StructTime" => "<class 'time.struct_time'>" => "time.struct_time",
    CompletedProcess = 18 => "CompletedProcess" => "<class 'subprocess.CompletedProcess'>" => "subprocess.CompletedProcess",
    ParseResult = 19 => "ParseResult" => "<class 'urllib.parse.ParseResult'>" => "urllib.parse.ParseResult",
    HttpResponse = 20 => "HttpResponse" => "<class 'http.client.HTTPResponse'>" => "http.client.HTTPResponse",
    Hash = 21 => "Hash" => "<class 'hashlib.HASH'>" => "hashlib.HASH",
    StringIO = 22 => "StringIO" => "<class '_io.StringIO'>" => "io.StringIO",
    BytesIO = 23 => "BytesIO" => "<class '_io.BytesIO'>" => "io.BytesIO",
    DefaultDict = 24 => "DefaultDict" => "<class 'collections.defaultdict'>" => "collections.defaultdict",
    Counter = 25 => "Counter" => "<class 'collections.Counter'>" => "collections.Counter",
    Deque = 26 => "Deque" => "<class 'collections.deque'>" => "collections.deque",
    Request = 27 => "Request" => "<class 'urllib.request.Request'>" => "urllib.request.Request",
    NotImplemented = 28 => "NotImplemented" => "<class 'NotImplementedType'>" => "NotImplementedType",
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_tags_unique() {
        let mut seen = std::collections::HashSet::new();
        for kind in TypeTagKind::ALL {
            assert!(
                seen.insert(kind.tag()),
                "Duplicate tag: {} for {}",
                kind.tag(),
                kind.name()
            );
        }
    }

    #[test]
    fn test_tags_are_sequential() {
        for (i, kind) in TypeTagKind::ALL.iter().enumerate() {
            assert_eq!(
                kind.tag() as usize,
                i,
                "Tag {} for '{}' should be at index {}",
                kind.tag(),
                kind.name(),
                i
            );
        }
    }

    #[test]
    fn test_count_matches() {
        assert_eq!(TypeTagKind::ALL.len(), TYPE_TAG_COUNT as usize);
    }

    #[test]
    fn test_from_tag_lookup() {
        assert_eq!(TypeTagKind::from_tag(0), Some(TypeTagKind::Int));
        assert_eq!(TypeTagKind::from_tag(3), Some(TypeTagKind::Str));
        assert_eq!(TypeTagKind::from_tag(5), Some(TypeTagKind::List));
        assert_eq!(TypeTagKind::from_tag(15), Some(TypeTagKind::File));
        assert_eq!(TypeTagKind::from_tag(16), Some(TypeTagKind::StringBuilder));
        assert_eq!(TypeTagKind::from_tag(17), Some(TypeTagKind::StructTime));
        assert_eq!(
            TypeTagKind::from_tag(18),
            Some(TypeTagKind::CompletedProcess)
        );
        assert_eq!(TypeTagKind::from_tag(19), Some(TypeTagKind::ParseResult));
        assert_eq!(TypeTagKind::from_tag(20), Some(TypeTagKind::HttpResponse));
        assert_eq!(TypeTagKind::from_tag(21), Some(TypeTagKind::Hash));
        assert_eq!(TypeTagKind::from_tag(22), Some(TypeTagKind::StringIO));
        assert_eq!(TypeTagKind::from_tag(23), Some(TypeTagKind::BytesIO));
        assert_eq!(TypeTagKind::from_tag(24), Some(TypeTagKind::DefaultDict));
        assert_eq!(TypeTagKind::from_tag(25), Some(TypeTagKind::Counter));
        assert_eq!(TypeTagKind::from_tag(26), Some(TypeTagKind::Deque));
        assert_eq!(TypeTagKind::from_tag(27), Some(TypeTagKind::Request));
        assert_eq!(TypeTagKind::from_tag(28), Some(TypeTagKind::NotImplemented));
        assert_eq!(TypeTagKind::from_tag(29), None);
        assert_eq!(TypeTagKind::from_tag(255), None);
    }

    #[test]
    fn test_from_name_lookup() {
        assert_eq!(TypeTagKind::from_name("Int"), Some(TypeTagKind::Int));
        assert_eq!(TypeTagKind::from_name("List"), Some(TypeTagKind::List));
        assert_eq!(TypeTagKind::from_name("File"), Some(TypeTagKind::File));
        assert_eq!(TypeTagKind::from_name("NotAType"), None);
    }

    #[test]
    fn test_tag_to_name() {
        assert_eq!(type_tag_to_name(0), Some("Int"));
        assert_eq!(type_tag_to_name(5), Some("List"));
        assert_eq!(type_tag_to_name(15), Some("File"));
        assert_eq!(type_tag_to_name(255), None);
    }

    #[test]
    fn test_round_trip() {
        for kind in TypeTagKind::ALL {
            let tag = kind.tag();
            let recovered =
                TypeTagKind::from_tag(tag).expect("type tag must have a valid recovery");
            assert_eq!(*kind, recovered);

            let name = kind.name();
            let by_name = TypeTagKind::from_name(name).expect("type tag name must be valid");
            assert_eq!(*kind, by_name);
        }
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", TypeTagKind::Int), "Int");
        assert_eq!(format!("{}", TypeTagKind::List), "List");
        assert_eq!(format!("{}", TypeTagKind::Generator), "Generator");
    }

    #[test]
    fn test_type_class() {
        assert_eq!(TypeTagKind::Int.type_class(), "<class 'int'>");
        assert_eq!(TypeTagKind::Float.type_class(), "<class 'float'>");
        assert_eq!(TypeTagKind::Bool.type_class(), "<class 'bool'>");
        assert_eq!(TypeTagKind::Str.type_class(), "<class 'str'>");
        assert_eq!(TypeTagKind::None.type_class(), "<class 'NoneType'>");
        assert_eq!(TypeTagKind::List.type_class(), "<class 'list'>");
        assert_eq!(TypeTagKind::Tuple.type_class(), "<class 'tuple'>");
        assert_eq!(TypeTagKind::Dict.type_class(), "<class 'dict'>");
        assert_eq!(TypeTagKind::Set.type_class(), "<class 'set'>");
        assert_eq!(TypeTagKind::Bytes.type_class(), "<class 'bytes'>");
        assert_eq!(TypeTagKind::Iterator.type_class(), "<class 'iterator'>");
        assert_eq!(TypeTagKind::Generator.type_class(), "<class 'generator'>");
        assert_eq!(TypeTagKind::Match.type_class(), "<class 're.Match'>");
        assert_eq!(
            TypeTagKind::File.type_class(),
            "<class '_io.TextIOWrapper'>"
        );
        assert_eq!(
            TypeTagKind::StructTime.type_class(),
            "<class 'time.struct_time'>"
        );
        assert_eq!(
            TypeTagKind::CompletedProcess.type_class(),
            "<class 'subprocess.CompletedProcess'>"
        );
    }

    #[test]
    fn test_type_name() {
        assert_eq!(TypeTagKind::Int.type_name(), "int");
        assert_eq!(TypeTagKind::Float.type_name(), "float");
        assert_eq!(TypeTagKind::Bool.type_name(), "bool");
        assert_eq!(TypeTagKind::Str.type_name(), "str");
        assert_eq!(TypeTagKind::None.type_name(), "NoneType");
        assert_eq!(TypeTagKind::List.type_name(), "list");
        assert_eq!(TypeTagKind::Tuple.type_name(), "tuple");
        assert_eq!(TypeTagKind::Dict.type_name(), "dict");
        assert_eq!(TypeTagKind::Set.type_name(), "set");
        assert_eq!(TypeTagKind::Bytes.type_name(), "bytes");
        assert_eq!(TypeTagKind::Instance.type_name(), "object");
        assert_eq!(TypeTagKind::Iterator.type_name(), "iterator");
        assert_eq!(TypeTagKind::Generator.type_name(), "generator");
        assert_eq!(TypeTagKind::Match.type_name(), "re.Match");
        assert_eq!(TypeTagKind::File.type_name(), "TextIOWrapper");
        assert_eq!(TypeTagKind::StructTime.type_name(), "time.struct_time");
        assert_eq!(
            TypeTagKind::CompletedProcess.type_name(),
            "subprocess.CompletedProcess"
        );
    }

    #[test]
    fn test_all_type_classes_start_with_class() {
        for kind in TypeTagKind::ALL {
            assert!(
                kind.type_class().starts_with("<class '"),
                "type_class for {} should start with \"<class '\", got: {}",
                kind.name(),
                kind.type_class()
            );
            assert!(
                kind.type_class().ends_with("'>"),
                "type_class for {} should end with \"'>\", got: {}",
                kind.name(),
                kind.type_class()
            );
        }
    }
}
