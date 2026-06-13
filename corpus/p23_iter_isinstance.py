# Standalone `iter()` builtin + `isinstance()` against container builtins.
#
# (A) `iter(iterable)` builds a runtime iterator object via the same
#     `ContainerOp::Iter` → `rt_iter_value` the for-loop drives (so a File
#     iterable would route through `rt_file_readlines` in lowering too); `next(it)`
#     consumes it via the raising `rt_iter_next` (StopIteration on exhaustion). The
#     2-arg sentinel form `iter(callable, sentinel)` is out of scope (compile error,
#     not probed here). Wired next to the existing `next` builtin (recognized by
#     name; shadowing not supported, same as `sum`/`set`/`next`).
# (B) `isinstance(x, list|dict|set|tuple)` — the builtin-isinstance static fold now
#     matches container targets by KIND (element types are irrelevant to
#     isinstance), alongside the existing `str|int|float|bool|bytes`.
#
# `==` asserts are the spec (Principle 9); prints feed the differential harness.


# ===== (A) iter() / next() over each iterable kind =====
nums: list[int] = [10, 20, 30]
it = iter(nums)
assert next(it) == 10
assert next(it) == 20
assert next(it) == 30
print("list iter ok")

# StopIteration on exhaustion.
caught = False
try:
    next(it)
except StopIteration:
    caught = True
assert caught, "exhausted list iterator raises StopIteration"
print(caught)  # True

# Tuple.
ti = iter((1, 2))
assert next(ti) == 1
assert next(ti) == 2
print("tuple iter ok")

# String.
si = iter("AB")
assert next(si) == "A"
assert next(si) == "B"
print("str iter ok")

# Range.
ri = iter(range(3))
assert next(ri) == 0
assert next(ri) == 1
assert next(ri) == 2
print("range iter ok")

# Dict (iterates keys, insertion order).
d: dict[str, int] = {"a": 1, "b": 2}
di = iter(d)
assert next(di) == "a"
assert next(di) == "b"
print("dict iter ok")

# iter() of a freshly-built list, then consume fully via a manual loop.
acc = 0
manual = iter([1, 2, 3, 4])
manual_done = False
while not manual_done:
    try:
        acc = acc + next(manual)
    except StopIteration:
        manual_done = True
assert acc == 10
print(acc)  # 10


# ===== (B) isinstance() against container builtins =====
li: list[int] = [1, 2, 3]
tu: tuple[int, int] = (1, 2)
dd: dict[str, int] = {"x": 1}
ss: set[int] = {1, 2, 3}

assert isinstance(li, list)
assert isinstance(tu, tuple)
assert isinstance(dd, dict)
assert isinstance(ss, set)
print("positive isinstance ok")

# Cross-kind: each value is NOT an instance of the other container kinds.
assert not isinstance(li, tuple)
assert not isinstance(li, dict)
assert not isinstance(li, set)
assert not isinstance(tu, list)
assert not isinstance(dd, list)
assert not isinstance(ss, list)
print("cross-kind isinstance ok")

# Container vs primitive and vice versa.
assert not isinstance(li, int)
assert not isinstance(42, list)
assert not isinstance("hi", tuple)
print("container/primitive isinstance ok")

# Element types are irrelevant to isinstance — a list[str] is still a list.
words: list[str] = ["a", "b"]
assert isinstance(words, list)
mixed_tuple: tuple[int, str] = (1, "x")
assert isinstance(mixed_tuple, tuple)
print("element-type-agnostic isinstance ok")

# The original test_iteration.py usage: a `*rest` from a starred unpack is a list.
for _first, *rest in [(1, 2, 3), (10, 20, 30)]:
    assert isinstance(rest, list)
print("starred-rest isinstance ok")

# isinstance still works for the primitive builtins (regression-guard) + bool ⊂ int.
assert isinstance(5, int)
assert isinstance(True, int)
assert isinstance(3.0, float)
assert isinstance("s", str)
assert isinstance(b"b", bytes)
print("primitive isinstance ok")

print("iter() + container-isinstance tests passed!")
