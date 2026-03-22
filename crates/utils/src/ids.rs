//! ID types for various entities in the compiler

use std::fmt;

/// Offset applied to generator function IDs to derive resume function IDs.
/// Resume func_id = original func_id + RESUME_FUNC_ID_OFFSET.
/// Used in lowering (to create) and codegen (to decode).
pub const RESUME_FUNC_ID_OFFSET: u32 = 10000;

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(pub u32);

        impl $name {
            pub fn new(idx: u32) -> Self {
                Self(idx)
            }

            pub fn index(self) -> usize {
                self.0 as usize
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl From<u32> for $name {
            fn from(idx: u32) -> Self {
                Self(idx)
            }
        }

        impl From<usize> for $name {
            fn from(idx: usize) -> Self {
                Self(idx as u32)
            }
        }
    };
}

define_id!(FuncId);
define_id!(TypeId);
define_id!(VarId);
define_id!(BlockId);
define_id!(LocalId);
define_id!(SymbolId);
define_id!(ClassId);
