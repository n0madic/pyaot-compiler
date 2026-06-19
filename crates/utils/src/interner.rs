//! String interning for efficient string storage and comparison.
//!
//! Entries are stored as raw byte blobs, not `String`, so a `bytes` literal that
//! is not valid UTF-8 (`b"\xff"`) can be interned alongside UTF-8 strings in one
//! id space. A string and a byte literal with identical bytes dedup to the same
//! id (harmless — the MIR `Const` variant, not the id, decides `str` vs `bytes`).
//! [`StringInterner::resolve`] keeps the `&str` contract for the (overwhelming)
//! UTF-8 callers; [`StringInterner::resolve_bytes`] is the raw view used by the
//! `bytes`-literal lowering.

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
    blobs: IndexSet<Box<[u8]>>,
}

impl StringInterner {
    pub fn new() -> Self {
        Self {
            blobs: IndexSet::new(),
        }
    }

    pub fn intern(&mut self, s: &str) -> InternedString {
        self.intern_bytes(s.as_bytes())
    }

    /// Intern raw bytes — a `bytes` literal that may not be valid UTF-8. Returns
    /// the same id as [`Self::intern`] for an identical byte sequence.
    pub fn intern_bytes(&mut self, b: &[u8]) -> InternedString {
        let (idx, _) = self.blobs.insert_full(Box::from(b));
        InternedString(idx as u32)
    }

    pub fn get(&self, id: InternedString) -> Option<&str> {
        self.blobs
            .get_index(id.index())
            .and_then(|b| std::str::from_utf8(b).ok())
    }

    pub fn resolve(&self, id: InternedString) -> &str {
        std::str::from_utf8(self.resolve_bytes(id)).expect("interned string is not valid UTF-8")
    }

    /// The raw bytes of an interned blob — valid for any id (UTF-8 string or a
    /// non-UTF-8 `bytes` literal). The `bytes`-literal lowering reads back here.
    pub fn resolve_bytes(&self, id: InternedString) -> &[u8] {
        self.blobs
            .get_index(id.index())
            .map(|b| &**b)
            .expect("invalid interned string")
    }

    /// Look up a string that may already be interned, without mutating.
    pub fn lookup(&self, s: &str) -> Option<InternedString> {
        self.blobs
            .get_index_of(s.as_bytes())
            .map(|idx| InternedString(idx as u32))
    }

    /// Get the number of interned blobs.
    pub fn len(&self) -> usize {
        self.blobs.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.blobs.is_empty()
    }

    /// Iterate over all interned blobs that are valid UTF-8 and their IDs
    /// (non-UTF-8 `bytes` literals are skipped — this view is `&str`).
    pub fn iter(&self) -> impl Iterator<Item = (InternedString, &str)> {
        self.blobs.iter().enumerate().filter_map(|(idx, b)| {
            std::str::from_utf8(b)
                .ok()
                .map(|s| (InternedString(idx as u32), s))
        })
    }
}
