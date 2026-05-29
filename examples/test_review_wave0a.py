"""Repro for Wave 0a memory-safety / correctness fixes (whole-project review).

Covers the functionally-observable fixes:
- StringIO/BytesIO seek-past-end gap must be NUL-filled (issue #6)
- bytes strip / join produce correct results (rooting fixes are exercised
  more thoroughly under gc_stress_test, but correctness is checked here too)
"""

import io


def stringio_gap() -> None:
    s = io.StringIO("ab")
    s.seek(5)
    s.write("X")
    print(repr(s.getvalue()))
    print(len(s.getvalue()))


def bytesio_gap() -> None:
    b = io.BytesIO(b"ab")
    b.seek(5)
    b.write(b"X")
    print(b.getvalue())
    print(len(b.getvalue()))


def bytes_ops() -> None:
    print(b"  hello world  ".strip())
    print(b",".join([b"a", b"bb", b"ccc"]))
    print(b"ab" * 3)
    print(b"ab" + b"cd")


def main() -> None:
    stringio_gap()
    bytesio_gap()
    bytes_ops()


main()
