//! Central definition of built-in function kinds for first-class function values.
//!
//! This module provides a single source of truth for built-in functions that can be
//! used as first-class values (passed to map, filter, sorted, assigned to variables).
//!
//! # Usage
//!
//! ```
//! use pyaot_core_defs::{BuiltinFunctionKind, BUILTIN_FUNCTION_COUNT};
//!
//! // Use the enum directly
//! let builtin = BuiltinFunctionKind::Len;
//! assert_eq!(builtin.id(), 0);
//! assert_eq!(builtin.name(), "len");
//!
//! // Lookup by name
//! assert_eq!(BuiltinFunctionKind::from_name("len"), Some(BuiltinFunctionKind::Len));
//! ```
//!
//! # Adding New Builtins
//!
//! To add a new first-class builtin, add an entry to `define_builtins!` macro invocation
//! below. Both the compiler and runtime will pick up the change automatically.

#![forbid(unsafe_code)]

/// Macro that defines the canonical builtin function list and generates:
/// - `BuiltinFunctionKind` enum with all builtin variants
/// - Lookup functions for IDs and names
/// - `BUILTIN_FUNCTION_COUNT` constant
///
/// This ensures compile-time generation with zero runtime overhead.
macro_rules! define_builtins {
    ($($variant:ident = $id:expr => $name:literal),* $(,)?) => {
        /// Built-in function kind enum - single source of truth for first-class builtins.
        /// These are functions that can be used as values (passed to map, filter, etc.)
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum BuiltinFunctionKind {
            $($variant = $id,)*
        }

        impl BuiltinFunctionKind {
            /// Get the numeric ID (0-N) for this builtin.
            /// Used as index into the runtime function pointer table.
            #[inline]
            pub const fn id(self) -> u8 {
                self as u8
            }

            /// Get the Python name for this builtin.
            #[inline]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)*
                }
            }

            /// Create from a numeric ID.
            /// Returns `None` if the ID is invalid.
            #[inline]
            pub const fn from_id(id: u8) -> Option<Self> {
                match id {
                    $($id => Some(Self::$variant),)*
                    _ => None,
                }
            }

            /// Create from a Python builtin name.
            /// Returns `None` if the name is not a first-class builtin.
            #[inline]
            pub fn from_name(name: &str) -> Option<Self> {
                match name {
                    $($name => Some(Self::$variant),)*
                    _ => None,
                }
            }

            /// Array of all first-class builtin function kinds.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];
        }

        impl std::fmt::Display for BuiltinFunctionKind {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.name())
            }
        }

        /// Number of first-class builtin function types.
        /// Used to size the runtime function pointer table.
        pub const BUILTIN_FUNCTION_COUNT: usize = {
            let mut count = 0usize;
            $(let _ = $id; count += 1;)*
            count
        };
    };
}

// =============================================================================
// SINGLE SOURCE OF TRUTH FOR FIRST-CLASS BUILTIN FUNCTIONS
// =============================================================================
//
// This is the canonical definition used by both the compiler and runtime.
// Adding a new builtin here automatically makes it available everywhere.
//
// Note: Only include builtins that can work as first-class values.
// Special builtins like print, range, input are NOT included because they
// have special calling conventions or require special handling.

define_builtins! {
    Len = 0 => "len",
    Str = 1 => "str",
    Int = 2 => "int",
    Float = 3 => "float",
    Bool = 4 => "bool",
    Abs = 5 => "abs",
    Hash = 6 => "hash",
    Ord = 7 => "ord",
    Chr = 8 => "chr",
    Repr = 9 => "repr",
    Type = 10 => "type",
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_ids_unique() {
        let mut seen = std::collections::HashSet::new();
        for builtin in BuiltinFunctionKind::ALL {
            assert!(
                seen.insert(builtin.id()),
                "Duplicate ID: {} for {}",
                builtin.id(),
                builtin.name()
            );
        }
    }

    #[test]
    fn test_ids_are_sequential() {
        for (i, builtin) in BuiltinFunctionKind::ALL.iter().enumerate() {
            assert_eq!(
                builtin.id() as usize,
                i,
                "ID {} for '{}' should be at index {}",
                builtin.id(),
                builtin.name(),
                i
            );
        }
    }

    #[test]
    fn test_count_matches() {
        assert_eq!(BuiltinFunctionKind::ALL.len(), BUILTIN_FUNCTION_COUNT);
    }

    #[test]
    fn test_from_name_lookup() {
        assert_eq!(
            BuiltinFunctionKind::from_name("len"),
            Some(BuiltinFunctionKind::Len)
        );
        assert_eq!(
            BuiltinFunctionKind::from_name("str"),
            Some(BuiltinFunctionKind::Str)
        );
        assert_eq!(
            BuiltinFunctionKind::from_name("int"),
            Some(BuiltinFunctionKind::Int)
        );
        assert_eq!(
            BuiltinFunctionKind::from_name("float"),
            Some(BuiltinFunctionKind::Float)
        );
        assert_eq!(
            BuiltinFunctionKind::from_name("bool"),
            Some(BuiltinFunctionKind::Bool)
        );
        assert_eq!(
            BuiltinFunctionKind::from_name("abs"),
            Some(BuiltinFunctionKind::Abs)
        );
        assert_eq!(
            BuiltinFunctionKind::from_name("hash"),
            Some(BuiltinFunctionKind::Hash)
        );
        assert_eq!(
            BuiltinFunctionKind::from_name("ord"),
            Some(BuiltinFunctionKind::Ord)
        );
        assert_eq!(
            BuiltinFunctionKind::from_name("chr"),
            Some(BuiltinFunctionKind::Chr)
        );
        assert_eq!(
            BuiltinFunctionKind::from_name("repr"),
            Some(BuiltinFunctionKind::Repr)
        );
        assert_eq!(
            BuiltinFunctionKind::from_name("type"),
            Some(BuiltinFunctionKind::Type)
        );
        assert_eq!(BuiltinFunctionKind::from_name("print"), None);
        assert_eq!(BuiltinFunctionKind::from_name("range"), None);
    }

    #[test]
    fn test_from_id_lookup() {
        assert_eq!(
            BuiltinFunctionKind::from_id(0),
            Some(BuiltinFunctionKind::Len)
        );
        assert_eq!(
            BuiltinFunctionKind::from_id(10),
            Some(BuiltinFunctionKind::Type)
        );
        assert_eq!(BuiltinFunctionKind::from_id(11), None);
        assert_eq!(BuiltinFunctionKind::from_id(255), None);
    }

    #[test]
    fn test_round_trip() {
        for builtin in BuiltinFunctionKind::ALL {
            // name -> from_name -> name should be identity
            let name = builtin.name();
            let from_name = BuiltinFunctionKind::from_name(name)
                .expect("builtin name must have a valid lookup");
            assert_eq!(from_name, *builtin);

            // id -> from_id -> id should be identity
            let id = builtin.id();
            let from_id =
                BuiltinFunctionKind::from_id(id).expect("builtin id must have a valid lookup");
            assert_eq!(from_id, *builtin);
        }
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", BuiltinFunctionKind::Len), "len");
        assert_eq!(format!("{}", BuiltinFunctionKind::Str), "str");
        assert_eq!(format!("{}", BuiltinFunctionKind::Type), "type");
    }
}
