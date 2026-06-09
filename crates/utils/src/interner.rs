//! String interning for efficient string storage and comparison

use indexmap::IndexSet;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InternedString(u32);

impl InternedString {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for InternedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InternedString({})", self.0)
    }
}

#[derive(Debug, Default)]
pub struct StringInterner {
    strings: IndexSet<String>,
}

impl StringInterner {
    pub fn new() -> Self {
        Self {
            strings: IndexSet::new(),
        }
    }

    pub fn intern(&mut self, s: &str) -> InternedString {
        let (idx, _) = self.strings.insert_full(s.to_string());
        InternedString(idx as u32)
    }

    pub fn get(&self, id: InternedString) -> Option<&str> {
        self.strings.get_index(id.index()).map(|s| s.as_str())
    }

    pub fn resolve(&self, id: InternedString) -> &str {
        self.get(id).expect("invalid interned string")
    }

    /// Look up a string that may already be interned, without mutating
    pub fn lookup(&self, s: &str) -> Option<InternedString> {
        self.strings
            .get_index_of(s)
            .map(|idx| InternedString(idx as u32))
    }

    /// Get the number of interned strings
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }

    /// Iterate over all interned strings and their IDs
    pub fn iter(&self) -> impl Iterator<Item = (InternedString, &str)> {
        self.strings
            .iter()
            .enumerate()
            .map(|(idx, s)| (InternedString(idx as u32), s.as_str()))
    }
}
