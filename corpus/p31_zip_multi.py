# Multi-iterable `zip` (3+ iterables), §12.
#
# The runtime already had `rt_zip3_new` / `rt_zipn_new` + the Zip3/ZipN iterator
# objects (kind-dispatched `rt_iter_next`); only the front-half was wired for the
# 2-iterable form. Now `zip(a, b, c, …)` (N≥3) lowers to a fresh runtime list of
# the N `iter()`-wrapped sources + `rt_zipn_new(list, count)` (one new
# `ContainerOp::ZipN`, ABI `[Val, Idx]`), and typeck infers the result element as
# a fixed-arity `tuple[…]` of one type per iterable — so `list(zip(xs, ys, zs))`
# types as `list[tuple[X, Y, Z]]` and assigns into an annotated container slot.
# The dedicated 2-iterable `rt_zip_new` path is unchanged.
#
# `==` asserts are the spec (Principle 9); prints feed the differential harness.


# ===== zip of 3 lists into an annotated list[tuple[...]] slot =====
z3_a: list[int] = [1, 2, 3]
z3_b: list[str] = ["a", "b", "c"]
z3_c: list[float] = [1.0, 2.0, 3.0]
z3: list[tuple[int, str, float]] = list(zip(z3_a, z3_b, z3_c))
assert len(z3) == 3
assert z3[0] == (1, "a", 1.0)
assert z3[1] == (2, "b", 2.0)
assert z3[2] == (3, "c", 3.0)
print(z3)


# ===== shortest iterable wins (different lengths) =====
s_a: list[int] = [1, 2]
s_b: list[int] = [10, 20, 30]
s_c: list[int] = [100, 200, 300, 400]
s: list[tuple[int, int, int]] = list(zip(s_a, s_b, s_c))
assert len(s) == 2
assert s == [(1, 10, 100), (2, 20, 200)]
print(s)


# ===== 4-iterable form (ZipN with count=4) =====
q4 = list(zip([1, 2], [3, 4], [5, 6], [7, 8]))
assert q4 == [(1, 3, 5, 7), (2, 4, 6, 8)]
print(q4)


# ===== 5 iterables, mixed element types =====
m = list(zip([1, 2], ["x", "y"], [1.5, 2.5], [True, False], ["p", "q"]))
assert m[0] == (1, "x", 1.5, True, "p")
assert m[1] == (2, "y", 2.5, False, "q")
print(m)


# ===== direct iteration of a 3-zip (tuple unpacking in the for-target) =====
total = 0
joined = ""
for i, name, f in zip(z3_a, z3_b, z3_c):
    total += i
    joined += name
    assert isinstance(f, float)
assert total == 6
assert joined == "abc"
print(total, joined)


# ===== 2-iterable form still works (unchanged rt_zip_new path) =====
two: list[tuple[int, str]] = list(zip(z3_a, z3_b))
assert two == [(1, "a"), (2, "b"), (3, "c")]
print(two)


# ===== cross with already-green features (sum over zipped products) =====
xs: list[int] = [1, 2, 3]
ys: list[int] = [4, 5, 6]
dot = 0
for a, b in zip(xs, ys):
    dot += a * b
assert dot == 32  # 1*4 + 2*5 + 3*6
print(dot)

print("All multi-zip tests passed!")
