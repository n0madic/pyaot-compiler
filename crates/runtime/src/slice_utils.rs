//! Shared utilities for slice index normalization
//!
//! This module provides common functions for normalizing slice indices across
//! all collection types (list, tuple, string, bytes). The normalization handles:
//! - Sentinel values (i64::MIN for default start, i64::MAX for default end)
//! - Negative indices (counted from the end)
//! - Clamping to valid bounds

/// Normalize a slice index for positive step (or step=1 implicit).
///
/// # Arguments
/// * `idx` - The raw index value (may be negative or sentinel)
/// * `len` - The length of the sequence
/// * `is_start` - True if this is the start index, false if end index
///
/// # Sentinel values
/// * `i64::MIN` - Default start (returns 0)
/// * `i64::MAX` - Default end (returns len)
///
/// # Returns
/// A normalized index in the range [0, len]
#[inline]
pub fn normalize_index_positive_step(idx: i64, len: i64, is_start: bool) -> i64 {
    if is_start && idx == i64::MIN {
        0 // Default start for positive step
    } else if !is_start && idx == i64::MAX {
        len // Default end for positive step
    } else if idx < 0 {
        (len + idx).max(0)
    } else {
        idx.min(len)
    }
}

/// Normalize start and end indices for a slice with positive step.
///
/// Convenience function that normalizes both indices at once.
#[inline]
pub fn normalize_slice_positive(start: i64, end: i64, len: i64) -> (i64, i64) {
    let start = normalize_index_positive_step(start, len, true);
    let end = normalize_index_positive_step(end, len, false);
    (start, end)
}

/// ASCII whitespace used by `str`/`bytes` whitespace splitting.
#[inline]
fn is_split_whitespace(c: u8) -> bool {
    c == b' ' || c == b'\t' || c == b'\n' || c == b'\r'
}

/// Compute the `(start, end)` byte ranges of the whitespace-delimited fields
/// produced by `split(None, maxsplit)` (`reverse = false`) or
/// `rsplit(None, maxsplit)` (`reverse = true`), in left-to-right order.
///
/// Shared by `str` and `bytes` (the only type-specific part is allocating the
/// field objects from each range). CPython semantics, including the previously
/// broken maxsplit case:
/// - runs of consecutive ASCII whitespace are a single separator;
/// - leading/trailing whitespace produces no empty fields;
/// - once `maxsplit` splits have been made, the rest of the string is a single
///   field with its *interior* whitespace preserved — for `split` the trailing
///   whitespace is kept (`"a b c ".split(None, 1) == ["a", "b c "]`), for
///   `rsplit` the leading whitespace is kept
///   (`"  a b c".rsplit(None, 1) == ["  a b", "c"]`).
///
/// `maxsplit < 0` means unlimited.
pub fn whitespace_field_ranges(data: &[u8], maxsplit: i64, reverse: bool) -> Vec<(usize, usize)> {
    let max = if maxsplit < 0 { i64::MAX } else { maxsplit };
    let n = data.len();
    let mut ranges: Vec<(usize, usize)> = Vec::new();

    if !reverse {
        let mut i = 0usize;
        let mut splits = 0i64;
        loop {
            while i < n && is_split_whitespace(data[i]) {
                i += 1;
            }
            if i >= n {
                break;
            }
            if splits >= max {
                // Remainder field — interior + trailing whitespace preserved.
                ranges.push((i, n));
                break;
            }
            let start = i;
            while i < n && !is_split_whitespace(data[i]) {
                i += 1;
            }
            ranges.push((start, i));
            splits += 1;
        }
    } else {
        let mut j = n;
        let mut splits = 0i64;
        loop {
            while j > 0 && is_split_whitespace(data[j - 1]) {
                j -= 1;
            }
            if j == 0 {
                break;
            }
            if splits >= max {
                // Remainder field — interior + leading whitespace preserved.
                ranges.push((0, j));
                break;
            }
            let end = j;
            while j > 0 && !is_split_whitespace(data[j - 1]) {
                j -= 1;
            }
            ranges.push((j, end));
            splits += 1;
        }
        ranges.reverse();
    }
    ranges
}

/// Normalize a slice index for negative step.
///
/// # Arguments
/// * `idx` - The raw index value (may be negative or sentinel)
/// * `len` - The length of the sequence
/// * `is_start` - True if this is the start index, false if end index
///
/// # Sentinel values
/// * `i64::MIN` - Default start (returns len-1)
/// * `i64::MAX` - Default end (returns -1, i.e., before index 0)
///
/// # Returns
/// A normalized index. For start: [-1, len-1], for end: [-1, len]
#[inline]
pub fn normalize_index_negative_step(idx: i64, len: i64, is_start: bool) -> i64 {
    if is_start {
        if idx == i64::MIN {
            len - 1 // Default start for negative step
        } else if idx < 0 {
            (len + idx).max(-1)
        } else {
            idx.min(len - 1)
        }
    } else {
        // end index
        if idx == i64::MAX {
            -1 // Default end for negative step (before index 0)
        } else if idx < 0 {
            (len + idx).max(-1)
        } else {
            idx.min(len)
        }
    }
}

/// Normalize start and end indices for a slice based on step direction.
///
/// This is the main entry point for slice normalization.
///
/// # Arguments
/// * `start` - The raw start index (may be i64::MIN sentinel)
/// * `end` - The raw end index (may be i64::MAX sentinel)
/// * `len` - The length of the sequence
/// * `step` - The step value (positive or negative)
///
/// # Returns
/// A tuple of (normalized_start, normalized_end)
#[inline]
pub fn normalize_slice_indices(start: i64, end: i64, len: i64, step: i64) -> (i64, i64) {
    if step > 0 {
        normalize_slice_positive(start, end, len)
    } else {
        let s = normalize_index_negative_step(start, len, true);
        let e = normalize_index_negative_step(end, len, false);
        (s, e)
    }
}

/// Calculate the length of a slice result.
///
/// # Arguments
/// * `start` - Normalized start index
/// * `end` - Normalized end index
///
/// # Returns
/// The number of elements in the slice (0 if start >= end)
#[inline]
pub fn slice_length(start: i64, end: i64) -> usize {
    if end > start {
        (end - start) as usize
    } else {
        0
    }
}

/// Collect indices for a stepped slice.
///
/// # Arguments
/// * `start` - Normalized start index
/// * `end` - Normalized end index
/// * `step` - The step value (must not be 0)
///
/// # Returns
/// A vector of indices to include in the slice
pub fn collect_step_indices(start: i64, end: i64, step: i64) -> Vec<usize> {
    let mut indices = Vec::new();
    if step > 0 {
        let mut i = start;
        while i < end {
            indices.push(i as usize);
            i += step;
        }
    } else {
        let mut i = start;
        while i > end {
            indices.push(i as usize);
            i += step; // step is negative
        }
    }
    indices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_positive_step() {
        // Normal cases
        assert_eq!(normalize_slice_positive(0, 5, 10), (0, 5));
        assert_eq!(normalize_slice_positive(2, 8, 10), (2, 8));

        // Negative indices
        assert_eq!(normalize_slice_positive(-3, -1, 10), (7, 9));
        assert_eq!(normalize_slice_positive(-5, 10, 10), (5, 10));

        // Sentinel values
        assert_eq!(normalize_slice_positive(i64::MIN, i64::MAX, 10), (0, 10));
        assert_eq!(normalize_slice_positive(i64::MIN, 5, 10), (0, 5));
        assert_eq!(normalize_slice_positive(2, i64::MAX, 10), (2, 10));

        // Clamping
        assert_eq!(normalize_slice_positive(-100, 100, 10), (0, 10));
        assert_eq!(normalize_slice_positive(15, 20, 10), (10, 10));
    }

    #[test]
    fn test_normalize_negative_step() {
        let len = 10i64;

        // Default start for negative step is len-1
        assert_eq!(normalize_index_negative_step(i64::MIN, len, true), 9);
        // Default end for negative step is -1
        assert_eq!(normalize_index_negative_step(i64::MAX, len, false), -1);

        // Normal indices
        assert_eq!(normalize_index_negative_step(5, len, true), 5);
        assert_eq!(normalize_index_negative_step(5, len, false), 5);

        // Negative indices
        assert_eq!(normalize_index_negative_step(-1, len, true), 9);
        assert_eq!(normalize_index_negative_step(-1, len, false), 9);
    }

    #[test]
    fn test_slice_length() {
        assert_eq!(slice_length(0, 5), 5);
        assert_eq!(slice_length(3, 7), 4);
        assert_eq!(slice_length(5, 5), 0);
        assert_eq!(slice_length(7, 3), 0);
    }

    #[test]
    fn test_collect_step_indices() {
        // Positive step
        assert_eq!(collect_step_indices(0, 10, 2), vec![0, 2, 4, 6, 8]);
        assert_eq!(collect_step_indices(1, 10, 3), vec![1, 4, 7]);

        // Negative step
        assert_eq!(
            collect_step_indices(9, -1, -1),
            vec![9, 8, 7, 6, 5, 4, 3, 2, 1, 0]
        );
        assert_eq!(collect_step_indices(9, -1, -2), vec![9, 7, 5, 3, 1]);
    }
}
