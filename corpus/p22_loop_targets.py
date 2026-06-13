# Backlog §4 — finish "Unpacking & loop targets": attribute/subscript `for`-loop
# targets, and a non-literal/computed `range()` step in a `for`-loop.
#
# Two routing changes share this probe:
#  (A) `bind_for_target` now delegates supported shapes to `assign_to_target`, so
#      an attribute (`for obj.attr in …` → SetAttr) or subscript (`for lst[i] in …`
#      → SetItem) leaf is bound each iteration — exactly the path nested
#      destructuring uses (DRY, no new HIR/typeck surface).
#  (B) `lower_for` takes the Phase-3c raw-i64 fast path ONLY for a simple-`Name`
#      target with a compile-time-literal step; anything else (computed/variable
#      step, attr/subscript target) takes the general iterator path, which drives
#      the runtime `RangeIter` (correct direction at runtime + a `step == 0`
#      `ValueError` guard).
#
# The §4 trap (PITFALLS B): a non-literal/negative step must NOT collapse a loop
# to `sum(...) == 0`. Probed below with a negative variable step.
#
# `==` asserts are the spec (Principle 9); prints feed the differential harness.


# ===== Attribute as a `for`-target =====
class Cell:
    v: int

    def __init__(self) -> None:
        self.v = 0


c = Cell()
for c.v in [10, 20, 30]:
    pass
assert c.v == 30, "attribute for-target (list) keeps the last bound value"
print(c.v)  # 30

# Attribute for-target over a range (general path: simple step, attr target).
for c.v in range(3):
    pass
assert c.v == 2, "attribute for-target (range) keeps the last bound value"
print(c.v)  # 2


# ===== Subscript as a `for`-target =====
a: list[int] = [0, 0, 0]
for a[0] in [1, 2, 3]:
    pass
assert a[0] == 3, "subscript for-target keeps the last bound value"
print(a[0])  # 3

# Dict-subscript for-target with a runtime key.
d: dict[str, int] = {"k": 0}
k = "k"
for d[k] in [7, 8, 9]:
    pass
assert d[k] == 9, "dict-subscript for-target keeps the last bound value"
print(d["k"])  # 9


# ===== Non-literal / computed `range()` step (general path) =====
# Negative VARIABLE step (the §4 trap): must descend, NOT collapse to empty.
step = -1
descending: list[int] = []
for i in range(10, 0, step):
    descending.append(i)
assert descending == [10, 9, 8, 7, 6, 5, 4, 3, 2, 1], "negative variable step descends"
assert sum(range(10, 0, step)) == 55, "negative variable step: sum is 55, not 0 (§4 trap)"
print(sum(range(10, 0, step)))  # 55

# Computed positive step (a BinOp, not a literal → general path).
computed: list[int] = []
for i in range(0, 10, 1 + 1):
    computed.append(i)
assert computed == [0, 2, 4, 6, 8], "computed positive step"
print(computed)  # [0, 2, 4, 6, 8]

# `-(-1)` is a nested unary, NOT recognized as an int literal → general path.
# Step is +1, so range(10, 0, +1) is EMPTY (start > stop, ascending).
empty_dir: list[int] = []
for i in range(10, 0, -(-1)):
    empty_dir.append(i)
assert empty_dir == [], "range(10, 0, +1) is empty (ascending past stop)"
assert sum(range(10, 0, -(-1))) == 0, "genuinely-empty range sums to 0"
print(len(empty_dir))  # 0

# Direction cases through the value form (already correct, regression-guard).
pos_var = 2
assert list(range(0, 5, pos_var)) == [0, 2, 4], "positive variable step ascends"
neg_var = -2
assert list(range(5, 0, neg_var)) == [5, 3, 1], "negative variable step descends"
empty_var = 3
assert list(range(0, 0, empty_var)) == [], "empty start==stop range"
print(list(range(0, 5, pos_var)))  # [0, 2, 4]
print(list(range(5, 0, neg_var)))  # [5, 3, 1]


# ===== `step == 0` → ValueError (runtime contract fix) =====
# For-loop general path: building the range iterator raises eagerly.
zero = 0
caught_loop = False
try:
    for i in range(0, 5, zero):
        pass
except ValueError as e:
    caught_loop = True
    assert "must not be zero" in str(e), "step==0 message"
assert caught_loop, "for-loop range(_, _, 0) raises ValueError"
print(caught_loop)  # True

# Value form: list(range(0, 5, 0)) raises at construction.
caught_value = False
try:
    _ = list(range(0, 5, 0))
except ValueError as e:
    caught_value = True
    assert "must not be zero" in str(e), "step==0 value-form message"
assert caught_value, "list(range(0, 5, 0)) raises ValueError"
print(caught_value)  # True


# ===== Interaction probes =====
# Attribute for-target accumulation inside a method.
class Accumulator:
    cur: int
    total: int

    def __init__(self) -> None:
        self.cur = 0
        self.total = 0

    def run(self, xs: list[int]) -> int:
        for self.cur in xs:
            self.total = self.total + self.cur
        return self.total


acc = Accumulator()
assert acc.run([1, 2, 3, 4]) == 10, "attr for-target accumulates in a method"
assert acc.cur == 4, "attr for-target leaves the last element bound"
print(acc.run([5, 6]))  # 21  (10 + 5 + 6)

# Subscript for-target writing into a list, then reading back.
buf: list[int] = [0, 0]
idx = 1
for buf[idx] in [100, 200, 300]:
    pass
assert buf == [0, 300], "subscript for-target writes the indexed slot only"
print(buf)  # [0, 300]

# Variable-step range inside a function.
def step_sum(start: int, stop: int, st: int) -> int:
    acc2 = 0
    for v in range(start, stop, st):
        acc2 = acc2 + v
    return acc2


assert step_sum(0, 10, 2) == 20, "variable-step range in a function (ascending)"
assert step_sum(10, 0, -2) == 30, "variable-step range in a function (descending)"
print(step_sum(0, 10, 2))   # 20
print(step_sum(10, 0, -2))  # 30

# Tuple-unpack target still works (no regression from the delegation change).
pairs: list[tuple[int, int]] = [(1, 10), (2, 20), (3, 30)]
unpack_sum = 0
for ka, vb in pairs:
    unpack_sum = unpack_sum + ka + vb
assert unpack_sum == 66, "tuple-unpack for-target unaffected"
print(unpack_sum)  # 66

print("Loop-target & range-step tests passed!")
