//! Central definition of built-in exception types.
//!
//! This module provides a single source of truth for all built-in exception types,
//! their tags (0-28), and lookup functions. The `define_exceptions!` macro generates
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
    ($($variant:ident = $tag:expr => $name:literal : $parent:ident),* $(,)?) => {
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

            /// The direct builtin parent class (CPython's exception tree).
            /// `None` only for `BaseException`, the root.
            #[inline]
            #[allow(unreachable_patterns)]
            pub const fn parent(self) -> Option<Self> {
                match self {
                    Self::BaseException => None,
                    $(Self::$variant => Some(Self::$parent),)*
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
                    // Legacy aliases: in Python 3 both names ARE `OSError`
                    // (the same class object), not subclasses.
                    "IOError" | "EnvironmentError" => Some(Self::OSError),
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

/// Number of contiguous class IDs reserved after `BUILTIN_EXCEPTION_COUNT`
/// for stdlib-provided exception classes (e.g. `urllib.error.HTTPError`,
/// `re.error`, `socket.error`). Each stdlib module declares a
/// `StdlibExceptionClass` with a class_id in `BUILTIN_EXCEPTION_COUNT ..
/// BUILTIN_EXCEPTION_COUNT + RESERVED_STDLIB_EXCEPTION_SLOTS`. User-defined
/// classes start at FIRST_USER_CLASS_ID and never overlap stdlib or built-in tags.
/// The u8 class-id space is shared with user classes, so the reserve is kept
/// small (only `urllib.error` uses two slots today).
pub const RESERVED_STDLIB_EXCEPTION_SLOTS: u8 = 8;

/// Number of contiguous class IDs reserved after the stdlib exception slots
/// for built-in generic container types (`list`, `dict`, `set`, `tuple`,
/// `tuple[T, ...]`, `deque`, `defaultdict`). These IDs occupy
/// `[BUILTIN_EXCEPTION_COUNT + RESERVED_STDLIB_EXCEPTION_SLOTS,
///   BUILTIN_EXCEPTION_COUNT + RESERVED_STDLIB_EXCEPTION_SLOTS + RESERVED_BUILTIN_TYPE_SLOTS)`.
/// The constants themselves live in `crates/types/src/builtin_classes.rs`
/// where `ClassId` is available.
pub const RESERVED_BUILTIN_TYPE_SLOTS: u8 = 7;

/// First `class_id` available for user-defined Python classes. Ranges:
/// - `[0, BUILTIN_EXCEPTION_COUNT)` — built-in exceptions
/// - `[BUILTIN_EXCEPTION_COUNT, BUILTIN_EXCEPTION_COUNT + RESERVED_STDLIB_EXCEPTION_SLOTS)` — stdlib exceptions
/// - `[BUILTIN_EXCEPTION_COUNT + RESERVED_STDLIB_EXCEPTION_SLOTS, FIRST_USER_CLASS_ID)` — builtin generic types
/// - `[FIRST_USER_CLASS_ID, ...)` — user-defined classes
pub const FIRST_USER_CLASS_ID: u8 =
    BUILTIN_EXCEPTION_COUNT + RESERVED_STDLIB_EXCEPTION_SLOTS + RESERVED_BUILTIN_TYPE_SLOTS;

// =============================================================================
// SINGLE SOURCE OF TRUTH FOR BUILT-IN EXCEPTIONS
// =============================================================================
//
// This is the canonical definition used by both the compiler (types crate)
// and runtime crate. Adding a new exception here automatically makes it
// available everywhere.

define_exceptions! {
    // `Variant = tag => "Name" : Parent` — the parent column is the direct
    // builtin base (CPython's exception tree); `BaseException` names itself
    // (it is the root: `parent()` returns `None` for it). The runtime class
    // registry is seeded from this column (`rt_init_builtin_exception_classes`),
    // so `except OSError:` catches `FileNotFoundError`, `except LookupError:`
    // catches `KeyError`, and so on. Tags are append-only.
    Exception = 0 => "Exception" : BaseException,
    AssertionError = 1 => "AssertionError" : Exception,
    IndexError = 2 => "IndexError" : LookupError,
    ValueError = 3 => "ValueError" : Exception,
    StopIteration = 4 => "StopIteration" : Exception,
    TypeError = 5 => "TypeError" : Exception,
    RuntimeError = 6 => "RuntimeError" : Exception,
    GeneratorExit = 7 => "GeneratorExit" : BaseException,
    KeyError = 8 => "KeyError" : LookupError,
    AttributeError = 9 => "AttributeError" : Exception,
    // Tag 10 was `IOError`, which in Python 3 is merely an alias of `OSError`
    // (`from_name` maps `IOError`/`EnvironmentError` to `OSError`).
    ArithmeticError = 10 => "ArithmeticError" : Exception,
    ZeroDivisionError = 11 => "ZeroDivisionError" : ArithmeticError,
    OverflowError = 12 => "OverflowError" : ArithmeticError,
    MemoryError = 13 => "MemoryError" : Exception,
    NameError = 14 => "NameError" : Exception,
    NotImplementedError = 15 => "NotImplementedError" : RuntimeError,
    FileNotFoundError = 16 => "FileNotFoundError" : OSError,
    PermissionError = 17 => "PermissionError" : OSError,
    RecursionError = 18 => "RecursionError" : RuntimeError,
    EOFError = 19 => "EOFError" : Exception,
    SystemExit = 20 => "SystemExit" : BaseException,
    KeyboardInterrupt = 21 => "KeyboardInterrupt" : BaseException,
    FileExistsError = 22 => "FileExistsError" : OSError,
    ImportError = 23 => "ImportError" : Exception,
    OSError = 24 => "OSError" : Exception,
    ConnectionError = 25 => "ConnectionError" : OSError,
    TimeoutError = 26 => "TimeoutError" : OSError,
    SyntaxError = 27 => "SyntaxError" : Exception,
    BaseException = 28 => "BaseException" : BaseException,
    // Raised by the `del name` unbound read-guard (`rt_check_bound`).
    UnboundLocalError = 29 => "UnboundLocalError" : NameError,
    // §9 — encode/decode error hierarchy: `LookupError` is raised for an
    // unknown encoding name; `Unicode{En,De}codeError` when bytes / codepoints
    // fall outside the target codec.
    LookupError = 30 => "LookupError" : Exception,
    UnicodeError = 31 => "UnicodeError" : ValueError,
    UnicodeEncodeError = 32 => "UnicodeEncodeError" : UnicodeError,
    UnicodeDecodeError = 33 => "UnicodeDecodeError" : UnicodeError,
    // The rest of CPython's concrete builtin exception tree.
    FloatingPointError = 34 => "FloatingPointError" : ArithmeticError,
    BufferError = 35 => "BufferError" : Exception,
    ModuleNotFoundError = 36 => "ModuleNotFoundError" : ImportError,
    ReferenceError = 37 => "ReferenceError" : Exception,
    SystemError = 38 => "SystemError" : Exception,
    StopAsyncIteration = 39 => "StopAsyncIteration" : Exception,
    IndentationError = 40 => "IndentationError" : SyntaxError,
    TabError = 41 => "TabError" : IndentationError,
    UnicodeTranslateError = 42 => "UnicodeTranslateError" : UnicodeError,
    BlockingIOError = 43 => "BlockingIOError" : OSError,
    ChildProcessError = 44 => "ChildProcessError" : OSError,
    BrokenPipeError = 45 => "BrokenPipeError" : ConnectionError,
    ConnectionAbortedError = 46 => "ConnectionAbortedError" : ConnectionError,
    ConnectionRefusedError = 47 => "ConnectionRefusedError" : ConnectionError,
    ConnectionResetError = 48 => "ConnectionResetError" : ConnectionError,
    InterruptedError = 49 => "InterruptedError" : OSError,
    IsADirectoryError = 50 => "IsADirectoryError" : OSError,
    NotADirectoryError = 51 => "NotADirectoryError" : OSError,
    ProcessLookupError = 52 => "ProcessLookupError" : OSError,
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
        assert_eq!(exception_name_to_tag("IOError"), Some(24));
        assert_eq!(exception_name_to_tag("EnvironmentError"), Some(24));
        assert_eq!(exception_name_to_tag("ArithmeticError"), Some(10));
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
        assert_eq!(exception_tag_to_name(28), Some("BaseException"));
        assert_eq!(exception_tag_to_name(29), Some("UnboundLocalError"));
        assert_eq!(exception_tag_to_name(30), Some("LookupError"));
        assert_eq!(exception_tag_to_name(31), Some("UnicodeError"));
        assert_eq!(exception_tag_to_name(32), Some("UnicodeEncodeError"));
        assert_eq!(exception_tag_to_name(33), Some("UnicodeDecodeError"));
        assert_eq!(exception_tag_to_name(52), Some("ProcessLookupError"));
        assert_eq!(exception_tag_to_name(53), None);
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
