"""Repro for Wave 3e lowering fixes (whole-project review).

- Unary -/+ on a bool yields int in CPython (-True == -1, +True == 1), not bool
  (operators/unary_ops.rs + constraint_solver/solve.rs).
- for-loop range step direction is detected by peeking through UnOp(Neg):
  `-1` (UnOp(Neg, Int(1))) is negative and `-(-1)` is positive (double
  negation) — statements/iter_protocol.rs.

(The VarId-undercount and generator-element fallback fixes are safety/typing
hardening without a clean positive diff; they are covered by the full suite.)
"""


def test_unary_bool() -> None:
    print(-True)
    print(+True)
    print(-False)
    x = True
    print(-x + 5)
    y = False
    print(+y)


def test_range_step() -> None:
    total = 0
    for i in range(10, 0, -1):
        total += i
    print(total)

    desc: list[int] = []
    for i in range(5, 0, -1):
        desc.append(i)
    print(desc)

    # `-(-1)` is +1 (double negation): range(10, 0, 1) is empty.
    total2 = 0
    for i in range(10, 0, -(-1)):
        total2 += i
    print(total2)


def main() -> None:
    test_unary_bool()
    test_range_step()


main()
