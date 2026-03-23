/// Timsort implementation for Python lists
///
/// Timsort is a hybrid stable sorting algorithm derived from merge sort and insertion sort,
/// designed to perform well on many kinds of real-world data. It was designed by Tim Peters
/// in 2002 for use in the Python programming language.
///
/// Key features:
/// - Stable sort (preserves relative order of equal elements)
/// - O(n log n) worst-case time complexity
/// - O(n) best-case time complexity (for already sorted data)
/// - Adaptive: takes advantage of existing order in the data
use std::cmp::Ordering;
use std::ptr;

/// Minimum size of a run for insertion sort
/// Empirically determined to be optimal for most datasets
const MIN_RUN: usize = 32;

/// Maximum merge stack size (sufficient for any reasonable input size)
const MAX_MERGE_STACK: usize = 85;

/// A run is a sorted sequence in the array
#[derive(Debug, Clone, Copy)]
struct Run {
    start: usize,
    len: usize,
}

/// Sort a slice of integers in-place using Timsort
pub fn timsort_int(data: &mut [i64]) {
    let len = data.len();
    if len < 2 {
        return;
    }

    // For small arrays, use insertion sort directly
    if len < MIN_RUN {
        insertion_sort_int(data, 0, len);
        return;
    }

    // Find runs and push them onto stack
    let mut runs = Vec::with_capacity(MAX_MERGE_STACK);
    let min_run = compute_min_run(len);

    let mut start = 0;
    while start < len {
        let mut run_len = count_run_int(data, start);

        // If run is too small, extend it with insertion sort
        if run_len < min_run {
            let force = min_run.min(len - start);
            insertion_sort_int(data, start, start + force);
            run_len = force;
        }

        runs.push(Run {
            start,
            len: run_len,
        });
        start += run_len;

        // Merge runs to maintain invariants
        merge_collapse_int(data, &mut runs);
    }

    // Final merges
    merge_force_collapse_int(data, &mut runs);
}

/// Sort a slice of floats in-place using Timsort
#[allow(dead_code)]
pub fn timsort_float(data: &mut [f64]) {
    let len = data.len();
    if len < 2 {
        return;
    }

    // For small arrays, use insertion sort directly
    if len < MIN_RUN {
        insertion_sort_float(data, 0, len);
        return;
    }

    // Find runs and push them onto stack
    let mut runs = Vec::with_capacity(MAX_MERGE_STACK);
    let min_run = compute_min_run(len);

    let mut start = 0;
    while start < len {
        let mut run_len = count_run_float(data, start);

        // If run is too small, extend it with insertion sort
        if run_len < min_run {
            let force = min_run.min(len - start);
            insertion_sort_float(data, start, start + force);
            run_len = force;
        }

        runs.push(Run {
            start,
            len: run_len,
        });
        start += run_len;

        // Merge runs to maintain invariants
        merge_collapse_float(data, &mut runs);
    }

    // Final merges
    merge_force_collapse_float(data, &mut runs);
}

/// Sort with a comparison function
pub fn timsort_with_cmp<T: Clone, F>(data: &mut [T], mut cmp: F)
where
    F: FnMut(&T, &T) -> Ordering,
{
    let len = data.len();
    if len < 2 {
        return;
    }

    // For small arrays, use insertion sort directly
    if len < MIN_RUN {
        insertion_sort_with_cmp(data, 0, len, &mut cmp);
        return;
    }

    // Find runs and push them onto stack
    let mut runs = Vec::with_capacity(MAX_MERGE_STACK);
    let min_run = compute_min_run(len);

    let mut start = 0;
    while start < len {
        let mut run_len = count_run_with_cmp(data, start, &mut cmp);

        // If run is too small, extend it with insertion sort
        if run_len < min_run {
            let force = min_run.min(len - start);
            insertion_sort_with_cmp(data, start, start + force, &mut cmp);
            run_len = force;
        }

        runs.push(Run {
            start,
            len: run_len,
        });
        start += run_len;

        // Merge runs to maintain invariants
        merge_collapse_with_cmp(data, &mut runs, &mut cmp);
    }

    // Final merges
    merge_force_collapse_with_cmp(data, &mut runs, &mut cmp);
}

/// Compute the minimum run length for the given array size
/// Returns a value in range [32, 64] such that N/min_run is close to a power of 2
fn compute_min_run(n: usize) -> usize {
    let mut r = 0;
    let mut n = n;
    while n >= MIN_RUN {
        r |= n & 1;
        n >>= 1;
    }
    n + r
}

/// Count the length of the next run, starting at `start`
/// Returns the length of an ascending or descending run
fn count_run_int(data: &mut [i64], start: usize) -> usize {
    let len = data.len();
    if start + 1 >= len {
        return len - start;
    }

    let mut end = start + 1;

    // Check if descending
    if data[end] < data[start] {
        while end < len && data[end] < data[end - 1] {
            end += 1;
        }
        // Reverse the descending run
        reverse_int(data, start, end);
    } else {
        // Ascending run
        while end < len && data[end] >= data[end - 1] {
            end += 1;
        }
    }

    end - start
}

fn count_run_float(data: &mut [f64], start: usize) -> usize {
    let len = data.len();
    if start + 1 >= len {
        return len - start;
    }

    let mut end = start + 1;

    // Check if descending
    if data[end] < data[start] {
        while end < len && data[end] < data[end - 1] {
            end += 1;
        }
        // Reverse the descending run
        reverse_float(data, start, end);
    } else {
        // Ascending run
        while end < len && data[end] >= data[end - 1] {
            end += 1;
        }
    }

    end - start
}

fn count_run_with_cmp<T, F>(data: &mut [T], start: usize, cmp: &mut F) -> usize
where
    F: FnMut(&T, &T) -> Ordering,
{
    let len = data.len();
    if start + 1 >= len {
        return len - start;
    }

    let mut end = start + 1;

    // Check if descending
    if cmp(&data[end], &data[start]) == Ordering::Less {
        while end < len && cmp(&data[end], &data[end - 1]) == Ordering::Less {
            end += 1;
        }
        // Reverse the descending run
        data[start..end].reverse();
    } else {
        // Ascending run
        while end < len && cmp(&data[end], &data[end - 1]) != Ordering::Less {
            end += 1;
        }
    }

    end - start
}

/// Reverse a portion of the array
fn reverse_int(data: &mut [i64], start: usize, end: usize) {
    let mut i = start;
    let mut j = end - 1;
    while i < j {
        data.swap(i, j);
        i += 1;
        j -= 1;
    }
}

fn reverse_float(data: &mut [f64], start: usize, end: usize) {
    let mut i = start;
    let mut j = end - 1;
    while i < j {
        data.swap(i, j);
        i += 1;
        j -= 1;
    }
}

/// Insertion sort for a range [start, end)
fn insertion_sort_int(data: &mut [i64], start: usize, end: usize) {
    for i in (start + 1)..end {
        let key = data[i];
        let mut j = i;
        while j > start && data[j - 1] > key {
            data[j] = data[j - 1];
            j -= 1;
        }
        data[j] = key;
    }
}

fn insertion_sort_float(data: &mut [f64], start: usize, end: usize) {
    for i in (start + 1)..end {
        let key = data[i];
        let mut j = i;
        while j > start && data[j - 1] > key {
            data[j] = data[j - 1];
            j -= 1;
        }
        data[j] = key;
    }
}

fn insertion_sort_with_cmp<T, F>(data: &mut [T], start: usize, end: usize, cmp: &mut F)
where
    F: FnMut(&T, &T) -> Ordering,
{
    for i in (start + 1)..end {
        let mut j = i;
        while j > start && cmp(&data[j - 1], &data[j]) == Ordering::Greater {
            data.swap(j - 1, j);
            j -= 1;
        }
    }
}

/// Maintain the merge invariants by merging runs
fn merge_collapse_int(data: &mut [i64], runs: &mut Vec<Run>) {
    while runs.len() > 1 {
        let n = runs.len();

        // Invariant 1: runs[n-2].len > runs[n-1].len
        // Invariant 2: runs[n-3].len > runs[n-2].len + runs[n-1].len
        if n >= 3 && runs[n - 3].len <= runs[n - 2].len + runs[n - 1].len {
            if runs[n - 3].len < runs[n - 1].len {
                merge_at_int(data, runs, n - 3);
            } else {
                merge_at_int(data, runs, n - 2);
            }
        } else if runs[n - 2].len <= runs[n - 1].len {
            merge_at_int(data, runs, n - 2);
        } else {
            break;
        }
    }
}

fn merge_collapse_float(data: &mut [f64], runs: &mut Vec<Run>) {
    while runs.len() > 1 {
        let n = runs.len();

        if n >= 3 && runs[n - 3].len <= runs[n - 2].len + runs[n - 1].len {
            if runs[n - 3].len < runs[n - 1].len {
                merge_at_float(data, runs, n - 3);
            } else {
                merge_at_float(data, runs, n - 2);
            }
        } else if runs[n - 2].len <= runs[n - 1].len {
            merge_at_float(data, runs, n - 2);
        } else {
            break;
        }
    }
}

fn merge_collapse_with_cmp<T: Clone, F>(data: &mut [T], runs: &mut Vec<Run>, cmp: &mut F)
where
    F: FnMut(&T, &T) -> Ordering,
{
    while runs.len() > 1 {
        let n = runs.len();

        if n >= 3 && runs[n - 3].len <= runs[n - 2].len + runs[n - 1].len {
            if runs[n - 3].len < runs[n - 1].len {
                merge_at_with_cmp(data, runs, n - 3, cmp);
            } else {
                merge_at_with_cmp(data, runs, n - 2, cmp);
            }
        } else if runs[n - 2].len <= runs[n - 1].len {
            merge_at_with_cmp(data, runs, n - 2, cmp);
        } else {
            break;
        }
    }
}

/// Merge all remaining runs
fn merge_force_collapse_int(data: &mut [i64], runs: &mut Vec<Run>) {
    while runs.len() > 1 {
        let n = runs.len();
        if n >= 3 && runs[n - 3].len < runs[n - 1].len {
            merge_at_int(data, runs, n - 3);
        } else {
            merge_at_int(data, runs, n - 2);
        }
    }
}

fn merge_force_collapse_float(data: &mut [f64], runs: &mut Vec<Run>) {
    while runs.len() > 1 {
        let n = runs.len();
        if n >= 3 && runs[n - 3].len < runs[n - 1].len {
            merge_at_float(data, runs, n - 3);
        } else {
            merge_at_float(data, runs, n - 2);
        }
    }
}

fn merge_force_collapse_with_cmp<T: Clone, F>(data: &mut [T], runs: &mut Vec<Run>, cmp: &mut F)
where
    F: FnMut(&T, &T) -> Ordering,
{
    while runs.len() > 1 {
        let n = runs.len();
        if n >= 3 && runs[n - 3].len < runs[n - 1].len {
            merge_at_with_cmp(data, runs, n - 3, cmp);
        } else {
            merge_at_with_cmp(data, runs, n - 2, cmp);
        }
    }
}

/// Merge the run at index `i` with the next run
fn merge_at_int(data: &mut [i64], runs: &mut Vec<Run>, i: usize) {
    let run1 = runs[i];
    let run2 = runs[i + 1];

    merge_int(
        data,
        run1.start,
        run1.start + run1.len,
        run1.start + run1.len + run2.len,
    );

    runs[i] = Run {
        start: run1.start,
        len: run1.len + run2.len,
    };
    runs.remove(i + 1);
}

fn merge_at_float(data: &mut [f64], runs: &mut Vec<Run>, i: usize) {
    let run1 = runs[i];
    let run2 = runs[i + 1];

    merge_float(
        data,
        run1.start,
        run1.start + run1.len,
        run1.start + run1.len + run2.len,
    );

    runs[i] = Run {
        start: run1.start,
        len: run1.len + run2.len,
    };
    runs.remove(i + 1);
}

fn merge_at_with_cmp<T: Clone, F>(data: &mut [T], runs: &mut Vec<Run>, i: usize, cmp: &mut F)
where
    F: FnMut(&T, &T) -> Ordering,
{
    let run1 = runs[i];
    let run2 = runs[i + 1];

    merge_with_cmp(
        data,
        run1.start,
        run1.start + run1.len,
        run1.start + run1.len + run2.len,
        cmp,
    );

    runs[i] = Run {
        start: run1.start,
        len: run1.len + run2.len,
    };
    runs.remove(i + 1);
}

/// Merge two sorted runs [start, mid) and [mid, end)
fn merge_int(data: &mut [i64], start: usize, mid: usize, end: usize) {
    let len1 = mid - start;
    let len2 = end - mid;

    if len1 == 0 || len2 == 0 {
        return;
    }

    // Copy first run to temporary buffer
    let mut temp = Vec::with_capacity(len1);
    unsafe {
        ptr::copy_nonoverlapping(data.as_ptr().add(start), temp.as_mut_ptr(), len1);
        temp.set_len(len1);
    }

    let mut i = 0; // Index into temp
    let mut j = mid; // Index into second run
    let mut k = start; // Index into output

    // Merge
    while i < len1 && j < end {
        if temp[i] <= data[j] {
            data[k] = temp[i];
            i += 1;
        } else {
            data[k] = data[j];
            j += 1;
        }
        k += 1;
    }

    // Copy remaining elements from temp
    while i < len1 {
        data[k] = temp[i];
        i += 1;
        k += 1;
    }
}

fn merge_float(data: &mut [f64], start: usize, mid: usize, end: usize) {
    let len1 = mid - start;
    let len2 = end - mid;

    if len1 == 0 || len2 == 0 {
        return;
    }

    // Copy first run to temporary buffer
    let mut temp = Vec::with_capacity(len1);
    unsafe {
        ptr::copy_nonoverlapping(data.as_ptr().add(start), temp.as_mut_ptr(), len1);
        temp.set_len(len1);
    }

    let mut i = 0; // Index into temp
    let mut j = mid; // Index into second run
    let mut k = start; // Index into output

    // Merge
    while i < len1 && j < end {
        if temp[i] <= data[j] {
            data[k] = temp[i];
            i += 1;
        } else {
            data[k] = data[j];
            j += 1;
        }
        k += 1;
    }

    // Copy remaining elements from temp
    while i < len1 {
        data[k] = temp[i];
        i += 1;
        k += 1;
    }
}

fn merge_with_cmp<T: Clone, F>(data: &mut [T], start: usize, mid: usize, end: usize, cmp: &mut F)
where
    F: FnMut(&T, &T) -> Ordering,
{
    let len1 = mid - start;
    let len2 = end - mid;

    if len1 == 0 || len2 == 0 {
        return;
    }

    // Copy both halves into temporary buffers to avoid O(n) shifts per element
    let left: Vec<T> = data[start..mid].to_vec();
    let right: Vec<T> = data[mid..end].to_vec();

    let mut i = 0; // index into left
    let mut j = 0; // index into right
    let mut k = start; // index into data (output)

    while i < left.len() && j < right.len() {
        // Use <= (not <) to preserve stability: left elements come first on equal keys
        if cmp(&left[i], &right[j]) != Ordering::Greater {
            data[k] = left[i].clone();
            i += 1;
        } else {
            data[k] = right[j].clone();
            j += 1;
        }
        k += 1;
    }

    while i < left.len() {
        data[k] = left[i].clone();
        i += 1;
        k += 1;
    }

    while j < right.len() {
        data[k] = right[j].clone();
        j += 1;
        k += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timsort_int_empty() {
        let mut data: [i64; 0] = [];
        timsort_int(&mut data);
        let expected: [i64; 0] = [];
        assert_eq!(data, expected);
    }

    #[test]
    fn test_timsort_int_single() {
        let mut data = [42];
        timsort_int(&mut data);
        assert_eq!(data, [42]);
    }

    #[test]
    fn test_timsort_int_sorted() {
        let mut data = [1, 2, 3, 4, 5];
        timsort_int(&mut data);
        assert_eq!(data, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_timsort_int_reverse() {
        let mut data = [5, 4, 3, 2, 1];
        timsort_int(&mut data);
        assert_eq!(data, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_timsort_int_random() {
        let mut data = [3, 7, 1, 9, 2, 5, 8, 4, 6];
        timsort_int(&mut data);
        assert_eq!(data, [1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn test_timsort_float() {
        let mut data = [3.5, 1.2, 4.8, 2.1, 5.9];
        timsort_float(&mut data);
        assert_eq!(data, [1.2, 2.1, 3.5, 4.8, 5.9]);
    }
}
