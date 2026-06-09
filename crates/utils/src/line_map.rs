//! Byte offset to line/column mapping for source-level debug information.

/// Maps byte offsets in source text to 1-based line and column numbers.
///
/// Built by scanning for newline characters. Lookups use binary search
/// for O(log n) performance.
#[derive(Debug, Clone)]
pub struct LineMap {
    /// Byte offset of the start of each line (always starts with 0).
    line_starts: Vec<u32>,
}

impl LineMap {
    /// Build a line map from source text.
    pub fn new(source: &str) -> Self {
        let mut line_starts = vec![0u32];
        for (i, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push((i + 1) as u32);
            }
        }
        Self { line_starts }
    }

    /// Convert a byte offset to a 1-based line number.
    pub fn line_number(&self, byte_offset: u32) -> u32 {
        match self.line_starts.binary_search(&byte_offset) {
            Ok(idx) => idx as u32 + 1,
            Err(idx) => idx as u32, // idx is the line containing this offset
        }
    }

    /// Convert a byte offset to (line, column), both 1-based.
    pub fn line_col(&self, byte_offset: u32) -> (u32, u32) {
        let line_idx = match self.line_starts.binary_search(&byte_offset) {
            Ok(idx) => idx,
            Err(idx) => idx - 1,
        };
        let line = line_idx as u32 + 1;
        let col = byte_offset - self.line_starts[line_idx] + 1;
        (line, col)
    }

    /// Return the total number of lines.
    pub fn line_count(&self) -> u32 {
        self.line_starts.len() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_source() {
        let map = LineMap::new("");
        assert_eq!(map.line_count(), 1);
        assert_eq!(map.line_number(0), 1);
        assert_eq!(map.line_col(0), (1, 1));
    }

    #[test]
    fn single_line() {
        let map = LineMap::new("hello");
        assert_eq!(map.line_count(), 1);
        assert_eq!(map.line_number(0), 1);
        assert_eq!(map.line_number(4), 1);
        assert_eq!(map.line_col(0), (1, 1));
        assert_eq!(map.line_col(4), (1, 5));
    }

    #[test]
    fn multi_line() {
        // "abc\ndef\nghi"
        //  0123 4567 890
        let map = LineMap::new("abc\ndef\nghi");
        assert_eq!(map.line_count(), 3);

        // Line 1: bytes 0..3
        assert_eq!(map.line_number(0), 1);
        assert_eq!(map.line_number(2), 1);
        assert_eq!(map.line_col(0), (1, 1));
        assert_eq!(map.line_col(2), (1, 3));

        // Line 2: bytes 4..7
        assert_eq!(map.line_number(4), 2);
        assert_eq!(map.line_number(6), 2);
        assert_eq!(map.line_col(4), (2, 1));
        assert_eq!(map.line_col(6), (2, 3));

        // Line 3: bytes 8..10
        assert_eq!(map.line_number(8), 3);
        assert_eq!(map.line_number(10), 3);
        assert_eq!(map.line_col(8), (3, 1));
        assert_eq!(map.line_col(10), (3, 3));
    }

    #[test]
    fn at_newline_boundary() {
        // "a\nb"
        //  01 23
        let map = LineMap::new("a\nb");
        assert_eq!(map.line_number(0), 1); // 'a'
        assert_eq!(map.line_number(1), 1); // '\n' belongs to line 1
        assert_eq!(map.line_number(2), 2); // 'b'
    }

    #[test]
    fn trailing_newline() {
        // "abc\n"
        //  0123 4
        let map = LineMap::new("abc\n");
        assert_eq!(map.line_count(), 2);
        assert_eq!(map.line_number(0), 1);
        assert_eq!(map.line_number(3), 1); // '\n'
        assert_eq!(map.line_number(4), 2); // empty last line
    }

    #[test]
    fn empty_lines() {
        // "\n\n"
        //  0 1 2
        let map = LineMap::new("\n\n");
        assert_eq!(map.line_count(), 3);
        assert_eq!(map.line_number(0), 1);
        assert_eq!(map.line_number(1), 2);
        assert_eq!(map.line_number(2), 3);
    }

    #[test]
    fn python_source() {
        // Line 1: "def add(a: int, b: int) -> int:\n"  bytes 0..31, \n at 31
        // Line 2: "    return a + b\n"                  bytes 32..48, \n at 48
        // Line 3: "\n"                                  byte 49, \n at 49
        // Line 4: "x: int = add(3, 4)\n"               bytes 50..68, \n at 68
        // Line 5: ""                                    byte 69
        let source = "def add(a: int, b: int) -> int:\n    return a + b\n\nx: int = add(3, 4)\n";
        let map = LineMap::new(source);
        assert_eq!(map.line_count(), 5);
        assert_eq!(map.line_number(0), 1); // 'd' in 'def'
        assert_eq!(map.line_number(32), 2); // ' ' in '    return'
        assert_eq!(map.line_number(49), 3); // '\n' (empty line)
        assert_eq!(map.line_number(50), 4); // 'x' in 'x: int'
    }
}
