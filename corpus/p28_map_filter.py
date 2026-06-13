# `map(func, iterable)` and `filter(func, iterable)` — the next higher-order
# builtins after `reduce`, §5.
#
# Both are desugared in the frontend to an EAGER compiled loop that calls the
# callback per element through the ordinary uniform-tagged indirect-call
# machinery, materializes the results into a `list`, and wraps it in an iterator
# so `for`/`list`/`next`/`sum` consume it:
#
#     map(f, xs)        ~= iter([f(x) for x in xs])
#     filter(f, xs)     ~= iter([x for x in xs if f(x)])
#     filter(None, xs)  ~= iter([x for x in xs if x])   (element truthiness)
#
# This deliberately AVOIDS the runtime `rt_map_new` / `rt_filter_new` /
# `IteratorKind::Map/Filter` lazy-iterator HOF machinery (the PITFALLS A4
# anti-pattern — a parallel calling convention with hand-encoded captures, marker
# bits, and an `i8` predicate ABI). The callback rides the same tagged `Call` a
# compiled lambda/closure does, and builtin callbacks (`map(str, …)` /
# `map(len, …)`) resolve through the normal `Symbol`-dispatch with no extra code.
# `func` is staged ONCE (CPython single function evaluation). Eager-vs-lazy
# side-effect timing is observationally invisible on this finite, pure corpus
# (the `lower_sum`/`reduce` materialization precedent). Only the single-iterable
# form is supported; multi-iterable `map` needs `zip` (§12, out of scope).
#
# `==` asserts are the spec (Principle 9); prints feed the differential harness.


# ===== map: named def, consumed via a for-loop =====
def triple(n: int) -> int:
    return n * 3


out: list[int] = []
for v in map(triple, [1, 2, 3, 4]):
    out.append(v)
assert out == [3, 6, 9, 12]
print(out)  # [3, 6, 9, 12]


# ===== map: consumed via repeated next() + StopIteration =====
mi = map(lambda x: x + 100, [1, 2])
assert next(mi) == 101
assert next(mi) == 102
caught = False
try:
    next(mi)
except StopIteration:
    caught = True
assert caught, "an exhausted map iterator raises StopIteration"
print(caught)  # True


# ===== map: plain lambda =====
assert list(map(lambda x: x * x, [1, 2, 3, 4])) == [1, 4, 9, 16]
print(list(map(lambda x: x * x, [1, 2, 3, 4])))  # [1, 4, 9, 16]


# ===== map: capturing lambda (single + multiple free variables) =====
factor: int = 3
assert list(map(lambda x: x * factor, [1, 2, 3])) == [3, 6, 9]
print(list(map(lambda x: x * factor, [1, 2, 3])))  # [3, 6, 9]

a: int = 2
b: int = 10
assert list(map(lambda x: x * a + b, [1, 2, 3])) == [12, 14, 16]
print(list(map(lambda x: x * a + b, [1, 2, 3])))  # [12, 14, 16]


# ===== map: builtin callbacks (the Symbol-dispatch cases) =====
assert list(map(str, [1, 2, 3])) == ["1", "2", "3"]
assert list(map(int, ["10", "20", "30"])) == [10, 20, 30]
assert list(map(len, ["a", "bb", "ccc"])) == [1, 2, 3]
assert list(map(abs, [-5, 3, -2])) == [5, 3, 2]
assert list(map(lambda x: str(x), [7, 8])) == ["7", "8"]
print(list(map(str, [1, 2, 3])))  # ['1', '2', '3']
print(list(map(len, ["a", "bb", "ccc"])))  # [1, 2, 3]
print(list(map(abs, [-5, 3, -2])))  # [5, 3, 2]


# ===== map: over string elements =====
assert list(map(lambda s: s + "!", ["a", "b", "c"])) == ["a!", "b!", "c!"]
print(list(map(lambda s: s + "!", ["a", "b", "c"])))  # ['a!', 'b!', 'c!']


# ===== filter: named predicate =====
def is_even(n: int) -> bool:
    return n % 2 == 0


assert list(filter(is_even, [1, 2, 3, 4, 5, 6])) == [2, 4, 6]
print(list(filter(is_even, [1, 2, 3, 4, 5, 6])))  # [2, 4, 6]


# ===== filter: consumed via next() =====
fi = filter(is_even, [1, 2, 3, 4])
assert next(fi) == 2
assert next(fi) == 4
print("filter next ok")


# ===== filter(None, xs): element truthiness across kinds =====
assert list(filter(None, [0, 1, 2, 0, 3])) == [1, 2, 3]
assert list(filter(None, ["", "a", "", "b"])) == ["a", "b"]
assert list(filter(None, [[], [1], [], [2, 3]])) == [[1], [2, 3]]
assert list(filter(None, [True, False, True, False])) == [True, True]
print(list(filter(None, [0, 1, 2, 0, 3])))  # [1, 2, 3]
print(list(filter(None, ["", "a", "", "b"])))  # ['a', 'b']
print(list(filter(None, [[], [1], [], [2, 3]])))  # [[1], [2, 3]]

# filter(None) over a sequence with `None` (the Optional case): None is falsy.
opt = [1, None, 2, None, 3]
assert list(filter(None, opt)) == [1, 2, 3]
print(list(filter(None, opt)))  # [1, 2, 3]

# All-falsy → empty; all-truthy → unchanged.
assert list(filter(None, [0, 0, False, ""])) == []
assert list(filter(None, [1, 2, 3])) == [1, 2, 3]
print(list(filter(None, [0, 0, False, ""])))  # []

# next() over a filter(None, …) iterator.
fn = filter(None, [0, 5, 0, 7])
assert next(fn) == 5
assert next(fn) == 7
print("filter(None) next ok")


# ===== nesting: map(f, filter(g, xs)) =====
def is_pos(n: int) -> bool:
    return n > 0


assert list(map(lambda x: x * 2, filter(is_pos, [-1, 2, -3, 4]))) == [4, 8]
print(list(map(lambda x: x * 2, filter(is_pos, [-1, 2, -3, 4]))))  # [4, 8]


# ===== list(map(...)) / list(filter(...)) with a closure + `: list[int]` =====
mult: int = 5
squares: list[int] = list(map(lambda x: x * mult, [1, 2, 3]))
assert squares == [5, 10, 15]
print(squares)  # [5, 10, 15]

threshold: int = 3
big: list[int] = list(filter(lambda x: x > threshold, [1, 2, 3, 4, 5]))
assert big == [4, 5]
print(big)  # [4, 5]


# ===== sum(map(...)) + for-accumulate over filter(...) =====
assert sum(map(lambda x: x * x, [1, 2, 3, 4])) == 30
print(sum(map(lambda x: x * x, [1, 2, 3, 4])))  # 30

total: int = 0
for v in filter(is_even, [1, 2, 3, 4, 5, 6, 7, 8]):
    total += v
assert total == 20  # 2 + 4 + 6 + 8
print(total)  # 20


# ===== single-evaluation guard: the callback is called once per element =====
_map_calls: int = 0


def _count_map(x: int) -> int:
    global _map_calls
    _map_calls += 1
    return x * 10


mapped = list(map(_count_map, [1, 2, 3, 4]))
assert mapped == [10, 20, 30, 40]
assert _map_calls == 4  # exactly one call per element
print(_map_calls)  # 4

_filt_calls: int = 0


def _count_pred(x: int) -> bool:
    global _filt_calls
    _filt_calls += 1
    return x > 2


kept = list(filter(_count_pred, [1, 2, 3, 4, 5, 6]))
assert kept == [3, 4, 5, 6]
assert _filt_calls == 6  # the predicate runs once per element
print(_filt_calls)  # 6


# ===== shadowing guard: a local `map` binding wins over the builtin =====
def shadow_map() -> int:
    # A local `map` shadows the builtin; the call dispatches the user binding.
    map = lambda items: sum(items)
    return map([10, 20, 30])


assert shadow_map() == 60
print(shadow_map())  # 60


print("map() / filter() tests passed!")
