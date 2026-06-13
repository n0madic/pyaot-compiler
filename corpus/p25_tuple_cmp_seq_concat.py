# Lexicographic tuple ordering in min/max/sorted + dynamic sequence concatenation.
#
# (A) Tuple ordering: `rt_obj_cmp` (the min/max fold path) and
#     `sorted`/`compare_list_elements` now dispatch a `Tuple` operand to the
#     lexicographic `tuple_cmp_ordering` (CPython `(1, 2) < (1, 3)`), instead of
#     raising `TypeError` (min/max) or comparing by pointer address (sorted).
#     Recurses element-wise, so nested tuples order correctly too.
# (B) Dynamic sequence concatenation: `rt_obj_add` (the gradual `+` path — two
#     `Dyn` operands, e.g. inside an untyped-param function) now handles
#     `list + list`, `tuple + tuple`, and `bytes + bytes` via the existing
#     `rt_list_concat`/`rt_tuple_concat`/`rt_bytes_concat`. Statically-typed
#     concatenation already worked; this closes the dynamic path.
#
# `==` asserts are the spec (Principle 9); prints feed the differential harness.


# ===== (A) min/max over tuple-yielding gen-exprs (the §G.4 path) =====
assert max((v, i) for i, v in enumerate([3, 1, 4, 1, 5])) == (5, 4)
assert min((v, i) for i, v in enumerate([3, 1, 4, 1, 5])) == (1, 1)
print(max((v, i) for i, v in enumerate([3, 1, 4, 1, 5])))  # (5, 4)
print(min((v, i) for i, v in enumerate([3, 1, 4, 1, 5])))  # (1, 1)

# Tie-breaking: strict </> keeps the first-seen best on tie.
assert min((v, i) for i, v in enumerate([3, 1, 1, 4])) == (1, 1)
assert max((v, i) for i, v in enumerate([4, 1, 4, 1])) == (4, 2)

# 3-tuple lexicographic compare.
_data = [(1, 2, 3), (1, 2, 4), (1, 1, 9)]
assert min((a, b, c) for a, b, c in _data) == (1, 1, 9)
assert max((a, b, c) for a, b, c in _data) == (1, 2, 4)
print(min(_data))  # (1, 1, 9)
print(max(_data))  # (1, 2, 4)


# ===== min/max over a direct list of tuples =====
pairs: list[tuple[int, str]] = [(2, "b"), (1, "z"), (1, "a")]
assert min(pairs) == (1, "a")
assert max(pairs) == (2, "b")
print(min(pairs))  # (1, 'a')
print(max(pairs))  # (2, 'b')


# ===== sorted() over a list of tuples (lexicographic, not pointer order) =====
unsorted: list[tuple[int, int]] = [(3, 1), (1, 2), (1, 1), (2, 5)]
assert sorted(unsorted) == [(1, 1), (1, 2), (2, 5), (3, 1)]
print(sorted(unsorted))  # [(1, 1), (1, 2), (2, 5), (3, 1)]

# sorted reverse.
assert sorted(unsorted, reverse=True) == [(3, 1), (2, 5), (1, 2), (1, 1)]

# Mixed-type tuple elements (int then str) sort lexicographically.
mixed: list[tuple[int, str]] = [(2, "x"), (1, "b"), (1, "a")]
assert sorted(mixed) == [(1, "a"), (1, "b"), (2, "x")]
print(sorted(mixed))  # [(1, 'a'), (1, 'b'), (2, 'x')]


# ===== Nested tuples order recursively =====
nested: list[tuple[int, tuple[int, int]]] = [(1, (2, 3)), (1, (1, 9)), (0, (9, 9))]
assert sorted(nested) == [(0, (9, 9)), (1, (1, 9)), (1, (2, 3))]
assert min(nested) == (0, (9, 9))
assert max(nested) == (1, (2, 3))
print(sorted(nested))  # [(0, (9, 9)), (1, (1, 9)), (1, (2, 3))]


# ===== Direct tuple comparison operators =====
assert (1, 2) < (1, 3)
assert (1, 2) < (2, 0)
assert (1, 2, 3) > (1, 2)
assert (1, 2) <= (1, 2)
assert not ((1, 2) < (1, 2))
print((1, 2) < (1, 3))  # True


# ===== (B) Dynamic sequence concatenation through gradual `+` =====
# Untyped-param function → `+` lowers to the dynamic `rt_obj_add` path.
def cat(a, b):
    return a + b


assert cat([1, 2], [3, 4]) == [1, 2, 3, 4]
assert cat((1, 2), (3, 4)) == (1, 2, 3, 4)
assert cat(b"ab", b"cd") == b"abcd"
assert cat("ab", "cd") == "abcd"  # str path already worked — regression guard
assert cat(2, 3) == 5  # numeric path — regression guard
print(cat([1, 2], [3, 4]))  # [1, 2, 3, 4]
print(cat((1, 2), (3, 4)))  # (1, 2, 3, 4)
print(cat(b"ab", b"cd"))    # b'abcd'

# Empty-operand concatenation.
assert cat([], [1]) == [1]
assert cat([1], []) == [1]
assert cat((), (1,)) == (1,)
print(cat([], [1]))  # [1]

# Chained dynamic concatenation (fresh allocations each step — GC exercise).
acc_list = cat(cat([1], [2]), cat([3], [4]))
assert acc_list == [1, 2, 3, 4]
print(acc_list)  # [1, 2, 3, 4]


# ===== Mismatched sequence concatenation raises TypeError =====
# (CPython and pyaot word the message differently, so only the type is probed.)
caught = False
try:
    _ = cat([1], (2,))
except TypeError:
    caught = True
assert caught, "list + tuple must raise TypeError"
print(caught)  # True


print("tuple-cmp + sequence-concat tests passed!")
