"""Repro for Wave 2 lowering fixes (whole-project review).

- pow() must coerce Int/Bool/tagged operands to f64 correctly rather than
  blindly emitting IntToFloat (which hard-errors the verifier on a Bool and
  reinterprets tagged pointer bits) — math/arithmetic.rs #5.
- all()/any() truthiness must be correct for float/str/heap element lists,
  not "tagged pointer != 0" — predicates.rs #4.

(The Class-dunder unary / `in` fixes are error-path improvements verified
separately: they now raise TypeError instead of SIGSEGV / silently returning
False, but the CPython error text differs so they are not diffed here.)
"""


def test_pow() -> None:
    print(round(pow(2, 0.5), 10))
    print(pow(2.0, 10.0))
    print(pow(True, 2.0))
    print(round(pow(0.5, 2), 10))


def test_all_any_float() -> None:
    print(all([0.0]))
    print(any([0.0]))
    print(all([1.0, 2.0]))
    print(any([0.0, 3.0]))


def test_all_any_str() -> None:
    print(all([""]))
    print(any([""]))
    print(all(["a", "b"]))
    print(any(["", "x"]))


def test_all_any_int() -> None:
    print(all([1, 2, 0]))
    print(any([0, 0]))
    print(all([1, 2, 3]))


def main() -> None:
    test_pow()
    test_all_any_float()
    test_all_any_str()
    test_all_any_int()


main()
