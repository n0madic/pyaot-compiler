# Test file for itertools.chain / itertools.islice
#
# Regression: itertools.chain used to route through the generic stdlib-call path
# with a one-param descriptor that mismatched the runtime ABI
# (rt_chain_new(iters_list, num_iters)). It silently dropped every argument past
# the first and passed the first iterable directly where a list-of-iterators was
# expected, so the runtime dereferenced a null `data` pointer -> SIGSEGV. chain
# now folds its variadic iterables into one list (VARIADIC_TO_LIST) and the
# runtime iter()-wraps each element lazily.
#
# islice had the sibling bug: the iterable was passed un-iter()-wrapped (raw list
# dereferenced as an iterator -> SIGSEGV) and the one-numeric-arg form was mapped
# to `start` instead of `stop`. It now has dedicated lowering that iter()-wraps
# the iterable and resolves start/stop/step from the argument count.

import itertools
from itertools import chain, islice

# ── chain over two lists ──
r1 = list(itertools.chain([1, 2], [3, 4]))
print(r1)
assert r1 == [1, 2, 3, 4], "chain of two lists"

# ── from-import form ──
r2 = list(chain([1, 2], [3, 4]))
print(r2)
assert r2 == [1, 2, 3, 4], "chain via from-import"

# ── single iterable ──
print(list(itertools.chain([1, 2, 3])))

# ── three iterables ──
print(list(itertools.chain([1], [2], [3])))

# ── empty chain() ──
print(list(itertools.chain()))
assert list(itertools.chain()) == [], "empty chain"


# ── chain over generators (the case that crashed in test_generators.py) ──
def count_up(base: int):
    yield base
    yield base + 1
    yield base + 2


r_gen = list(itertools.chain(count_up(0), count_up(10)))
print(r_gen)
assert r_gen == [0, 1, 2, 10, 11, 12], "chain of two generators"


# ── send(None)-primed generators chained together ──
def echo_once():
    received = yield 0
    yield received


def chain_two_gens() -> None:
    out: list[int] = []
    for v in itertools.chain(count_up(1), count_up(4)):
        out.append(v)
    print(out)


chain_two_gens()

# ── mixed iterable kinds: list, generator, tuple, range, str ──
mixed = list(itertools.chain([1, 2], count_up(100), (7, 8), range(3)))
print(mixed)
assert mixed == [1, 2, 100, 101, 102, 7, 8, 0, 1, 2], "chain of mixed kinds"

# ── chain over strings yields characters ──
print(list(itertools.chain("ab", "cd")))

# ── chain consumed by a for-loop with accumulation ──
acc: list[int] = []
for v in itertools.chain([1, 2], [3, 4, 5]):
    acc.append(v * v)
print(acc)
assert acc == [1, 4, 9, 16, 25], "for-loop over chain"

# ── chain consumed by tuple() and sum() ──
print(tuple(itertools.chain([1, 2], [3])))
print(sum(itertools.chain([10, 20], [30])))

# ── chain in a comprehension ──
doubled = [x * 2 for x in itertools.chain([1, 2], [3])]
print(doubled)
assert doubled == [2, 4, 6], "comprehension over chain"

# ── nested chain ──
nested = list(itertools.chain(itertools.chain([1], [2]), [3, 4]))
print(nested)
assert nested == [1, 2, 3, 4], "nested chain"


# ── islice: stop-only form (start defaults to 0) ──
print(list(itertools.islice([0, 1, 2, 3, 4, 5], 3)))
assert list(itertools.islice([0, 1, 2, 3, 4, 5], 3)) == [0, 1, 2], "islice stop"

# ── islice: start + stop ──
print(list(itertools.islice([0, 1, 2, 3, 4, 5], 1, 4)))
assert list(islice([0, 1, 2, 3, 4, 5], 1, 4)) == [1, 2, 3], "islice start/stop"

# ── islice: start + stop + step ──
print(list(itertools.islice([0, 1, 2, 3, 4, 5], 0, 6, 2)))
assert list(islice([0, 1, 2, 3, 4, 5], 0, 6, 2)) == [0, 2, 4], "islice step"

# ── islice over a string / range / generator ──
print(list(itertools.islice("ABCDEFG", 2, 6, 2)))
print(list(itertools.islice(range(100), 5)))


def squares():
    i = 0
    while i < 10:
        yield i * i
        i = i + 1


print(list(itertools.islice(squares(), 2, 8, 3)))

# ── islice does not over-consume an unbounded source ──
def naturals():
    i = 0
    while True:
        yield i
        i = i + 1


print(list(itertools.islice(naturals(), 4)))

# ── islice composed over chain ──
print(list(itertools.islice(itertools.chain([0, 1, 2], [3, 4, 5]), 2, 5)))

# ── islice stop past the end clamps to the source length ──
print(list(itertools.islice([1, 2, 3], 10)))

# ── islice consumed in a for-loop ──
isl_acc: list[int] = []
for v in itertools.islice([10, 20, 30, 40, 50], 1, 4):
    isl_acc.append(v)
print(isl_acc)
assert isl_acc == [20, 30, 40], "for-loop over islice"

# ── islice argument validation (CPython ValueError parity) ──
try:
    list(itertools.islice([1, 2, 3, 4], -1))
    print("no error")
except ValueError as e:
    print(e)

try:
    list(itertools.islice([1, 2, 3, 4], -2, 3))
    print("no error")
except ValueError as e:
    print(e)

try:
    list(itertools.islice([1, 2, 3, 4], 0, 4, 0))
    print("no error")
except ValueError as e:
    print(e)

print("All itertools tests passed!")
