"""Repro for Wave 3f builtins/runtime fixes (whole-project review).

- int(float('nan')) -> ValueError, int(float('inf')) -> OverflowError, instead
  of the silent saturating cast (0 / i64::MAX) (runtime/builtins.rs).
- os.path normalization collapses a leading `/..` to `/` (runtime/os.rs
  normalize_path, exercised via abspath of an absolute path).
- re.sub keeps a literal `$` in the replacement instead of treating it as a
  capture-group reference (runtime/re.rs).

(The cross-module `len()` uninitialized-result fix is a safety fix without a
clean positive diff. int()/float()/getattr error paths for unconvertible /
missing attributes are documented simplifications needing a runtime dispatch
and are left as known gaps.)
"""

from os.path import abspath


def test_int_nan_inf() -> None:
    nan = float("nan")
    try:
        print(int(nan))
    except ValueError:
        print("int(nan) ValueError")
    inf = float("inf")
    try:
        print(int(inf))
    except OverflowError:
        print("int(inf) OverflowError")
    print(int(2.7))
    print(int(-2.7))


def test_abspath_leading_parent() -> None:
    print(abspath("/../foo"))
    print(abspath("/.."))
    print(abspath("/../../a/b"))


def test_re_sub_literal_dollar() -> None:
    import re

    print(re.sub(r"(\d+)", r"$\1", "abc123"))
    print(re.sub(r"\d+", "[$]", "x5y"))
    print(re.sub(r"(\w)(\w)", r"\2\1", "ab"))


def main() -> None:
    test_int_nan_inf()
    test_abspath_leading_parent()
    test_re_sub_literal_dollar()


main()
