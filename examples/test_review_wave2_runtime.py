"""Repro for Wave 2 runtime fixes (whole-project review).

- str/bytes split(None, maxsplit) and rsplit(None, maxsplit) must keep the
  remainder (with interior whitespace preserved) once the split budget is hit,
  instead of dropping middle words (string/split_join.rs #10/#11, bytes twins).
  All four whitespace paths now share slice_utils::whitespace_field_ranges.
- file.write must write the entire buffer (write_all) and not silently drop the
  tail on a short write (file.rs).

(subprocess large-output drain is verified separately — it spawns an external
process so it is not part of this diffed example.)
"""

import os


def test_split_maxsplit() -> None:
    print("a b c".split(None, 1))
    print("a b c".rsplit(None, 1))
    print("a b c".split(None, 0))
    print("  a  b  c  ".split(None, 1))
    print("  a  b  c  ".rsplit(None, 1))
    print("a b c d".split(None, 2))
    print("a b c d".rsplit(None, 2))
    print("a-b-c-d".split("-", 1))
    print("a-b-c-d".rsplit("-", 1))


def test_bytes_split_maxsplit() -> None:
    print(b"a b c".split(None, 1))
    print(b"a b c".rsplit(None, 1))
    print(b"x y z w".split(None, 2))
    print(b"x y z w".rsplit(None, 2))
    print(b"a-b-c-d".split(b"-", 1))
    print(b"a-b-c-d".rsplit(b"-", 1))


def test_file_large_write() -> None:
    path = "/tmp/pyaot_review_wave2_bigwrite.txt"
    big = "X" * 200000
    with open(path, "w") as f:
        n = f.write(big)
    print(n)
    with open(path) as f:
        content = f.read()
    print(len(content))
    os.remove(path)


def main() -> None:
    test_split_maxsplit()
    test_bytes_split_maxsplit()
    test_file_large_write()


main()
