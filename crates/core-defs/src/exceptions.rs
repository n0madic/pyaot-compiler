//! Central definition of built-in exception types.
//!
//! This module provides a single source of truth for all built-in exception types,
//! their tags (0-27), and lookup functions. The `define_exceptions!` macro generates
//! the `BuiltinExceptionKind` enum and all lookup functions at compile time.
//!
//! # Usage
//!
//! ```
//! use pyaot_core_defs::{BuiltinExceptionKind, exception_name_to_tag, is_builtin_exception_name};
//!
//! // Use the enum directly
//! let exc = BuiltinExceptionKind::ValueError;
//! assert_eq!(exc.tag(), 3);
//! assert_eq!(exc.name(), "ValueError");
//!
//! // Lookup by name
//! assert_eq!(BuiltinExceptionKind::from_name("ValueError"), Some(BuiltinExceptionKind::ValueError));
//!
//! // Legacy functions still work
//! assert_eq!(exception_name_to_tag("ValueError"), Some(3));
//! ```
//!
//! # Adding New Exceptions
//!
//! To add a new built-in exception, simply add a new entry to `define_exceptions!`
//! macro invocation below. Everything else is generated automatically by the macro.
//! Both the `types` and `runtime` crates will pick up the change automatically.

#![forbid(unsafe_code)]

/// Built-in exception metadata (for backward compatibility).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinException {
    /// Numeric tag (0-N) matching runtime ExceptionType enum discriminant
    pub tag: u8,
    /// Exception type name as it appears in Python code
    pub name: &'static str,
}

/// Macro that defines the canonical exception list and generates:
/// - `BuiltinExceptionKind` enum with all exception variants
/// - `BUILTIN_EXCEPTIONS` constant array
/// - Lookup functions for tags and names
///
/// This ensures compile-time generation with zero runtime overhead.
macro_rules! define_exceptions {
    ($($variant:ident = $tag:expr => $name:literal),* $(,)?) => {
        /// Built-in exception kind enum - single source of truth.
        /// Use this enum instead of separate Type/Builtin variants.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum BuiltinExceptionKind {
            $($variant = $tag,)*
        }

        impl BuiltinExceptionKind {
            /// Get the numeric tag (0-N) for this exception kind.
            #[inline]
            pub const fn tag(self) -> u8 {
                self as u8
            }

            /// Get the Python name for this exception kind.
            #[inline]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)*
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

            /// Create from a Python exception name.
            /// Returns `None` if the name is not a built-in exception.
            #[inline]
            pub fn from_name(name: &str) -> Option<Self> {
                match name {
                    $($name => Some(Self::$variant),)*
                    _ => None,
                }
            }

            /// Array of all built-in exception kinds.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];
        }

        impl std::fmt::Display for BuiltinExceptionKind {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.name())
            }
        }

        /// Canonical list of all built-in exceptions (for backward compatibility).
        pub const BUILTIN_EXCEPTIONS: &[BuiltinException] = &[
            $(BuiltinException { tag: $tag, name: $name }),*
        ];

        /// Number of built-in exception types.
        /// First user-defined exception class ID should be >= this value.
        pub const BUILTIN_EXCEPTION_COUNT: u8 = {
            let mut count = 0u8;
            $(let _ = $tag; count += 1;)*
            count
        };

        /// Lookup exception type tag by name (for backward compatibility).
        #[inline]
        pub fn exception_name_to_tag(name: &str) -> Option<u8> {
            BuiltinExceptionKind::from_name(name).map(|k| k.tag())
        }

        /// Lookup exception name by tag (for backward compatibility).
        #[inline]
        pub fn exception_tag_to_name(tag: u8) -> Option<&'static str> {
            BuiltinExceptionKind::from_tag(tag).map(|k| k.name())
        }

        /// Check if a name is a built-in exception type.
        #[inline]
        pub fn is_builtin_exception_name(name: &str) -> bool {
            BuiltinExceptionKind::from_name(name).is_some()
        }
    };
}

// =============================================================================
// SINGLE SOURCE OF TRUTH FOR BUILT-IN EXCEPTIONS
// =============================================================================
//
// This is the canonical definition used by both the compiler (types crate)
// and runtime crate. Adding a new exception here automatically makes it
// available everywhere.

define_exceptions! {
    Exception = 0 => "Exception",
    AssertionError = 1 => "AssertionError",
    IndexError = 2 => "IndexError",
    ValueError = 3 => "ValueError",
    StopIteration = 4 => "StopIteration",
    TypeError = 5 => "TypeError",
    RuntimeError = 6 => "RuntimeError",
    GeneratorExit = 7 => "GeneratorExit",
    KeyError = 8 => "KeyError",
    AttributeError = 9 => "AttributeError",
    IOError = 10 => "IOError",
    ZeroDivisionError = 11 => "ZeroDivisionError",
    OverflowError = 12 => "OverflowError",
    MemoryError = 13 => "MemoryError",
    NameError = 14 => "NameError",
    NotImplementedError = 15 => "NotImplementedError",
    FileNotFoundError = 16 => "FileNotFoundError",
    PermissionError = 17 => "PermissionError",
    RecursionError = 18 => "RecursionError",
    EOFError = 19 => "EOFError",
    SystemExit = 20 => "SystemExit",
    KeyboardInterrupt = 21 => "KeyboardInterrupt",
    FileExistsError = 22 => "FileExistsError",
    ImportError = 23 => "ImportError",
    OSError = 24 => "OSError",
    ConnectionError = 25 => "ConnectionError",
    TimeoutError = 26 => "TimeoutError",
    SyntaxError = 27 => "SyntaxError",
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_tags_unique() {
        let mut seen = std::collections::HashSet::new();
        for exc in BUILTIN_EXCEPTIONS {
            assert!(
                seen.insert(exc.tag),
                "Duplicate tag: {} for {}",
                exc.tag,
                exc.name
            );
        }
    }

    #[test]
    fn test_tags_are_sequential() {
        for (i, exc) in BUILTIN_EXCEPTIONS.iter().enumerate() {
            assert_eq!(
                exc.tag as usize, i,
                "Tag {} for '{}' should be at index {}",
                exc.tag, exc.name, i
            );
        }
    }

    #[test]
    fn test_count_matches() {
        assert_eq!(BUILTIN_EXCEPTIONS.len(), BUILTIN_EXCEPTION_COUNT as usize);
    }

    #[test]
    fn test_name_to_tag_lookup() {
        assert_eq!(exception_name_to_tag("Exception"), Some(0));
        assert_eq!(exception_name_to_tag("AssertionError"), Some(1));
        assert_eq!(exception_name_to_tag("IndexError"), Some(2));
        assert_eq!(exception_name_to_tag("ValueError"), Some(3));
        assert_eq!(exception_name_to_tag("StopIteration"), Some(4));
        assert_eq!(exception_name_to_tag("TypeError"), Some(5));
        assert_eq!(exception_name_to_tag("RuntimeError"), Some(6));
        assert_eq!(exception_name_to_tag("GeneratorExit"), Some(7));
        assert_eq!(exception_name_to_tag("KeyError"), Some(8));
        assert_eq!(exception_name_to_tag("AttributeError"), Some(9));
        assert_eq!(exception_name_to_tag("IOError"), Some(10));
        assert_eq!(exception_name_to_tag("ZeroDivisionError"), Some(11));
        assert_eq!(exception_name_to_tag("OverflowError"), Some(12));
        assert_eq!(exception_name_to_tag("MemoryError"), Some(13));
        assert_eq!(exception_name_to_tag("NameError"), Some(14));
        assert_eq!(exception_name_to_tag("NotImplementedError"), Some(15));
        assert_eq!(exception_name_to_tag("FileNotFoundError"), Some(16));
        assert_eq!(exception_name_to_tag("PermissionError"), Some(17));
        assert_eq!(exception_name_to_tag("RecursionError"), Some(18));
        assert_eq!(exception_name_to_tag("EOFError"), Some(19));
        assert_eq!(exception_name_to_tag("SystemExit"), Some(20));
        assert_eq!(exception_name_to_tag("KeyboardInterrupt"), Some(21));
        assert_eq!(exception_name_to_tag("FileExistsError"), Some(22));
        assert_eq!(exception_name_to_tag("ImportError"), Some(23));
        assert_eq!(exception_name_to_tag("OSError"), Some(24));
        assert_eq!(exception_name_to_tag("ConnectionError"), Some(25));
        assert_eq!(exception_name_to_tag("TimeoutError"), Some(26));
        assert_eq!(exception_name_to_tag("SyntaxError"), Some(27));
        assert_eq!(exception_name_to_tag("NotARealException"), None);
    }

    #[test]
    fn test_tag_to_name_lookup() {
        assert_eq!(exception_tag_to_name(0), Some("Exception"));
        assert_eq!(exception_tag_to_name(1), Some("AssertionError"));
        assert_eq!(exception_tag_to_name(12), Some("OverflowError"));
        assert_eq!(exception_tag_to_name(13), Some("MemoryError"));
        assert_eq!(exception_tag_to_name(26), Some("TimeoutError"));
        assert_eq!(exception_tag_to_name(27), Some("SyntaxError"));
        assert_eq!(exception_tag_to_name(28), None);
        assert_eq!(exception_tag_to_name(255), None);
    }

    #[test]
    fn test_is_builtin_exception_name() {
        assert!(is_builtin_exception_name("Exception"));
        assert!(is_builtin_exception_name("ValueError"));
        assert!(is_builtin_exception_name("OverflowError"));
        assert!(!is_builtin_exception_name("MyCustomError"));
        assert!(!is_builtin_exception_name(""));
    }

    #[test]
    fn test_round_trip() {
        for exc in BUILTIN_EXCEPTIONS {
            // name -> tag -> name should be identity
            let tag =
                exception_name_to_tag(exc.name).expect("exception name must have a valid tag");
            assert_eq!(tag, exc.tag);
            let name = exception_tag_to_name(tag).expect("exception tag must have a valid name");
            assert_eq!(name, exc.name);

            // Should also be recognized as builtin
            assert!(is_builtin_exception_name(exc.name));
        }
    }

    #[test]
    fn test_builtin_exception_kind_enum() {
        // Test enum creation and methods
        let exc = BuiltinExceptionKind::ValueError;
        assert_eq!(exc.tag(), 3);
        assert_eq!(exc.name(), "ValueError");

        // Test from_tag
        assert_eq!(
            BuiltinExceptionKind::from_tag(3),
            Some(BuiltinExceptionKind::ValueError)
        );
        assert_eq!(BuiltinExceptionKind::from_tag(255), None);

        // Test from_name
        assert_eq!(
            BuiltinExceptionKind::from_name("ValueError"),
            Some(BuiltinExceptionKind::ValueError)
        );
        assert_eq!(BuiltinExceptionKind::from_name("NotAnException"), None);

        // Test ALL array
        assert_eq!(
            BuiltinExceptionKind::ALL.len(),
            BUILTIN_EXCEPTION_COUNT as usize
        );
        assert_eq!(
            BuiltinExceptionKind::ALL[0],
            BuiltinExceptionKind::Exception
        );
        assert_eq!(
            BuiltinExceptionKind::ALL[3],
            BuiltinExceptionKind::ValueError
        );
    }

    #[test]
    fn test_builtin_exception_kind_display() {
        assert_eq!(
            format!("{}", BuiltinExceptionKind::ValueError),
            "ValueError"
        );
        assert_eq!(format!("{}", BuiltinExceptionKind::Exception), "Exception");
    }
}
