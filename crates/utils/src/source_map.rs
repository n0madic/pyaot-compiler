//! Program-wide source map: gives every module of a multi-file program a
//! disjoint range of byte offsets, so a bare [`crate::Span`] identifies its file.
//!
//! The entry script occupies offsets `0..len`; each further module is placed
//! after the previous one with a one-byte gap (so an end-of-file span of one
//! module is never mistaken for the start of the next). The frontend parses a
//! module at its base offset, so every span it produces — and every span any
//! later phase copies from the HIR — is already global; diagnostics map a span
//! back to its file with [`SourceMap::file_at`] and rebase it by
//! [`SourceFile::base`].

/// One source file of the program.
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// Display name (the path diagnostics and tracebacks show).
    pub name: String,
    /// The file's full source text.
    pub source: String,
    /// Global offset of the file's first byte.
    pub base: u32,
}

impl SourceFile {
    /// One past the file's last global offset.
    pub fn end(&self) -> u32 {
        self.base + self.source.len() as u32
    }
}

/// The ordered set of [`SourceFile`]s, with disjoint offset ranges.
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// The base offset the next [`SourceMap::add`] will assign.
    pub fn next_base(&self) -> u32 {
        self.files.last().map_or(0, |f| f.end() + 1)
    }

    /// Register a file and return its base offset.
    pub fn add(&mut self, name: impl Into<String>, source: impl Into<String>) -> u32 {
        let base = self.next_base();
        self.insert(SourceFile {
            name: name.into(),
            source: source.into(),
            base,
        });
        base
    }

    /// Register a file whose base offset was assigned elsewhere (e.g. by the
    /// frontend via [`SourceMap::next_base`] bookkeeping of its own).
    pub fn insert(&mut self, file: SourceFile) {
        let pos = self.files.partition_point(|f| f.base < file.base);
        self.files.insert(pos, file);
    }

    /// The file containing global offset `offset` (its end offset included,
    /// for spans that point just past the last byte).
    pub fn file_at(&self, offset: u32) -> Option<&SourceFile> {
        let idx = self.files.partition_point(|f| f.base <= offset);
        let file = self.files.get(idx.checked_sub(1)?)?;
        (offset <= file.end()).then_some(file)
    }

    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_get_disjoint_ranges() {
        let mut map = SourceMap::new();
        assert_eq!(map.add("main.py", "abc"), 0);
        assert_eq!(map.add("mod.py", "xy"), 4);
        assert_eq!(map.file_at(0).unwrap().name, "main.py");
        assert_eq!(map.file_at(3).unwrap().name, "main.py");
        assert_eq!(map.file_at(4).unwrap().name, "mod.py");
        assert_eq!(map.file_at(6).unwrap().name, "mod.py");
        assert!(map.file_at(7).is_none());
        assert_eq!(map.next_base(), 7);
    }

    #[test]
    fn insert_keeps_order() {
        let mut map = SourceMap::new();
        map.insert(SourceFile {
            name: "b".into(),
            source: "zz".into(),
            base: 10,
        });
        map.insert(SourceFile {
            name: "a".into(),
            source: "abc".into(),
            base: 0,
        });
        assert_eq!(map.file_at(1).unwrap().name, "a");
        assert_eq!(map.file_at(11).unwrap().name, "b");
        assert!(map.file_at(5).is_none());
    }
}
