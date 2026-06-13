# Walrus / named expression `:=` (PEP 572, §2).
#
# `(target := value)` evaluates `value` once, binds it to `target` (a bare name)
# in the CONTAINING scope, and evaluates to the assigned value. Lowered in the
# frontend (`lower_named_expr`) through the ordinary write/read place machinery
# (local / captured cell / promoted module-global), so a name bound in an
# `if`/`while`/comprehension test is visible afterward, exactly as CPython.
#
# Also regression-guards `rt_obj_pos`: unary `+` on a bool now yields an int
# (`+True == 1`), mirroring unary `-` (`-True == -1`) which already promoted.
#
# `==` asserts are the spec (Principle 9); prints feed the differential harness.


# ===== Basic walrus in an if condition (name visible after the if) =====
nums: list[int] = [1, 2, 3, 4, 5]
if (n := len(nums)) > 3:
    assert n == 5
assert n == 5  # binds in the enclosing scope
print(n)  # 5

# Nested sub-expression.
x: int = 10
if (doubled := x * 2) > 15:
    assert doubled == 20
print(doubled)  # 20


# ===== Walrus in a while condition (re-evaluated each iteration) =====
data: list[int] = [3, 2, 1, 0]
idx: int = 0
collected: list[int] = []
while (val := data[idx]) > 0:
    collected.append(val)
    idx += 1
assert collected == [3, 2, 1]
assert val == 0  # the falsy value that ended the loop is still bound
print(collected)  # [3, 2, 1]
print(val)        # 0


# ===== Walrus inside a function-local scope =====
def big_sum(xs: list[int]) -> int:
    total = 0
    i = 0
    while i < len(xs):
        if (d := xs[i] * 2) > 4:
            total += d
        i += 1
    return total


assert big_sum([1, 2, 3, 4]) == 6 + 8
print(big_sum([1, 2, 3, 4]))  # 14


# ===== Walrus in a comprehension filter (leaks to the enclosing scope) =====
src: list[int] = [1, 2, 3, 4, 5]
squares_gt5: list[int] = [y for v in src if (y := v * v) > 5]
assert squares_gt5 == [9, 16, 25]
assert y == 25  # PEP 572: the comprehension walrus binds in the containing scope
print(squares_gt5)  # [9, 16, 25]
print(y)            # 25


# ===== Walrus as a sub-expression value =====
z = (w := 42) + 1
assert z == 43 and w == 42
print(z, w)  # 43 42

# Nested walrus.
if (a := (b := 5) + 1) == 6:
    assert a == 6 and b == 5
print(a, b)  # 6 5

# Walrus in a ternary.
t = (m := 7) if True else 0
assert t == 7 and m == 7
print(t, m)  # 7 7


# ===== Module-level walrus read inside a function (promoted to global) =====
if (config := 100) > 50:
    pass


def read_config() -> int:
    return config + 1


assert read_config() == 101
print(read_config())  # 101


# ===== Walrus reused/overwritten across a loop =====
acc: int = 0
for k in [1, 2, 3]:
    acc += (sq := k * k)
assert sq == 9 and acc == 14
print(sq, acc)  # 9 14


# ===== Regression: unary `+`/`-` on a bool yields an int =====
assert (+True) == 1 and isinstance(+True, int)
assert (+False) == 0
assert (-True) == -1
assert (-False) == 0
print(+True, +False, -True, -False)  # 1 0 -1 0


print("walrus + unary-plus-bool tests passed!")
