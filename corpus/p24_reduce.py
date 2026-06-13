# `functools.reduce(function, iterable[, initial])` — a higher-order builtin.
#
# Desugared in the frontend to a compiled accumulator loop calling
# `function(acc, elem)` each iteration (mirroring sum/min/max/all/any), NOT the
# raw-ABI `rt_reduce` callback path (the PITFALLS A4 anti-pattern). The reduction
# callable rides the ordinary indirect-call machinery — a lambda, a capturing
# lambda, or a named def alike — so its args/result stay on the uniform tagged
# ABI. Without `initial` the accumulator seeds from the first element (an empty
# iterable raises `TypeError`); with one, it seeds from `initial` (an empty
# iterable returns it unchanged).
#
# `==` asserts are the spec (Principle 9); prints feed the differential harness.

from functools import reduce


# ===== Basic: sum / product over a list, no initial =====
assert reduce(lambda a, b: a + b, [1, 2, 3, 4, 5]) == 15
assert reduce(lambda a, b: a * b, [1, 2, 3, 4]) == 24
print(reduce(lambda a, b: a + b, [1, 2, 3, 4, 5]))  # 15


# ===== With an initial value =====
assert reduce(lambda a, b: a + b, [1, 2, 3, 4, 5], 100) == 115
assert reduce(lambda a, b: a + b, [], 42) == 42  # empty + initial → initial
assert reduce(lambda a, b: a + b, [7], 3) == 10  # single elem + initial → f(3, 7)
print(reduce(lambda a, b: a + b, [1, 2, 3], 100))  # 106


# ===== Single element, no initial → returns the element (func never called) =====
assert reduce(lambda a, b: a + b, [99]) == 99
print(reduce(lambda a, b: a + b, [99]))  # 99


# ===== Named function as the callable =====
def add(a: int, b: int) -> int:
    return a + b


assert reduce(add, [10, 20, 30]) == 60
assert reduce(add, [10, 20, 30], 5) == 65
print(reduce(add, [10, 20, 30]))  # 60


# ===== Capturing lambda (free variable) =====
offset: int = 10
assert reduce(lambda a, b: a + b + offset, [1, 2, 3]) == (1 + 2 + offset) + 3 + offset
print(reduce(lambda a, b: a + b + offset, [1, 2, 3]))  # 26


# ===== Over a range and over a tuple =====
assert reduce(lambda a, b: a + b, range(1, 5)) == 10  # 1+2+3+4
assert reduce(lambda a, b: a + b, (10, 20, 30)) == 60
print(reduce(lambda a, b: a + b, range(1, 5)))  # 10


# ===== String accumulator (heap-typed acc) =====
assert reduce(lambda a, b: a + b, ["a", "b", "c", "d"]) == "abcd"
assert reduce(lambda a, b: a + b, ["x", "y"], "init-") == "init-xy"
print(reduce(lambda a, b: a + b, ["a", "b", "c", "d"]))  # abcd


# ===== List accumulator built across iterations (heap acc, GC-rooted) =====
# A named callable with annotated list params keeps `acc + [..]` a statically
# typed list-concat (dynamic `list + list` through gradual params is a separate
# unrelated limitation). Exercises a heap accumulator surviving across the
# reduction loop's inner allocations.
def append_doubled(acc: list[int], x: int) -> list[int]:
    return acc + [x * 2]


doubled = reduce(append_doubled, [1, 2, 3], [])
assert doubled == [2, 4, 6]
print(doubled)  # [2, 4, 6]


# ===== reduce() inside a function, over a parameter =====
def fold_sum(xs: list[int]) -> int:
    return reduce(lambda a, b: a + b, xs, 0)


assert fold_sum([1, 2, 3, 4]) == 10
assert fold_sum([]) == 0
print(fold_sum([5, 5, 5]))  # 15


# ===== Empty iterable with no initial → TypeError =====
caught = False
try:
    reduce(lambda a, b: a + b, [])
except TypeError as e:
    caught = True
    assert "empty" in str(e)
assert caught, "reduce() of empty iterable with no initial value must raise TypeError"
print(caught)  # True


# ===== Left-fold order matters (subtraction is not commutative) =====
# (((10 - 1) - 2) - 3) == 4
assert reduce(lambda a, b: a - b, [10, 1, 2, 3]) == 4
print(reduce(lambda a, b: a - b, [10, 1, 2, 3]))  # 4


print("reduce() tests passed!")
