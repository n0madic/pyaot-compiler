# Raw (unboxed) arithmetic fast paths: float `/`,
# comparisons and negation stay unboxed, raw int `+ - *` are inline
# overflow-checked machine ops. Every result must match CPython exactly —
# NaN comparisons, signed zeros, ZeroDivisionError, bignum promotion.
import math


def fdiv(a: float, b: float) -> float:
    return a / b


def fneg(a: float) -> float:
    return -a


def fpos(a: float) -> float:
    return +a


def flt(a: float, b: float) -> bool:
    return a < b


def fle(a: float, b: float) -> bool:
    return a <= b


def fgt(a: float, b: float) -> bool:
    return a > b


def fge(a: float, b: float) -> bool:
    return a >= b


def feq(a: float, b: float) -> bool:
    return a == b


def fne(a: float, b: float) -> bool:
    return a != b


def mixed(a: float) -> float:
    # float op int literal: the literal converts exactly.
    return a * 2 + 1 - a / 4


def mixed_cmp(a: float) -> bool:
    return a > 3 and a < 9007199254740992


def test_float_div() -> None:
    assert fdiv(7.0, 2.0) == 3.5
    assert fdiv(-1.0, 4.0) == -0.25
    assert math.isinf(fdiv(1e308, 1e-308))
    assert math.isnan(fdiv(math.nan, 2.0))
    assert math.isnan(fdiv(1.0, math.nan))
    for zero in (0.0, -0.0):
        try:
            fdiv(1.0, zero)
            assert False, "expected ZeroDivisionError"
        except ZeroDivisionError as e:
            assert str(e) == "division by zero"
    try:
        x: float = 2.5
        y = x / 0
        assert False, y
    except ZeroDivisionError as e:
        assert str(e) == "division by zero"
    # -0.0 results keep their sign.
    assert math.copysign(1.0, fdiv(-0.0, 5.0)) == -1.0
    assert math.copysign(1.0, fdiv(0.0, -5.0)) == -1.0


def test_float_neg() -> None:
    assert fneg(1.5) == -1.5
    assert math.copysign(1.0, fneg(0.0)) == -1.0
    assert math.copysign(1.0, fneg(-0.0)) == 1.0
    assert math.isnan(fneg(math.nan))
    assert fneg(math.inf) == -math.inf
    assert fpos(-2.0) == -2.0
    assert math.copysign(1.0, fpos(-0.0)) == -1.0


def test_float_compare() -> None:
    nan = math.nan
    assert flt(1.0, 2.0) and not flt(2.0, 1.0) and not flt(1.0, 1.0)
    assert fle(1.0, 1.0) and fge(1.0, 1.0)
    assert fgt(2.0, 1.0) and not fgt(1.0, 1.0)
    assert feq(0.0, -0.0) and not fne(0.0, -0.0)
    # NaN: every ordered comparison and `==` are False, `!=` is True.
    assert not flt(nan, 1.0) and not fle(nan, 1.0)
    assert not fgt(nan, 1.0) and not fge(nan, 1.0)
    assert not feq(nan, nan) and fne(nan, nan)
    assert not flt(1.0, nan) and fne(1.0, nan)
    assert flt(-math.inf, math.inf)


def test_mixed_literals() -> None:
    assert mixed(4.0) == 8.0
    assert mixed(-0.5) == -0.5 * 2 + 1 + 0.125
    assert mixed_cmp(4.0) and not mixed_cmp(3.0) and not mixed_cmp(math.nan)


def test_raw_int_loops() -> None:
    # Range-proven cursors: `+ - *` on Raw(I64) are inline machine ops.
    total = 0
    for i in range(1000):
        total += i * 3 - 1
    assert total == 1497500
    acc = 0
    for i in range(-50, 50):
        for j in range(10):
            acc += i * j - j
    assert acc == -6750
    # Unbounded accumulators stay tagged and promote to bignum exactly.
    big = 1
    for k in range(1, 80):
        big = big * 3 + k
    assert big > 2**100
    n = 9223372036854775807
    assert n + 1 == 9223372036854775808
    assert -n - 2 == -9223372036854775809
    assert n * n == 85070591730234615847396907784232501249


test_float_div()
test_float_neg()
test_float_compare()
test_mixed_literals()
test_raw_int_loops()
print("test_raw_fastpaths passed!")
