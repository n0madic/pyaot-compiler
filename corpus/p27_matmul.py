# Matrix-multiply operator `@` / `__matmul__` (PEP 465), §2.
#
# There is no built-in numeric `@`, so `a @ b` dispatches the user
# `__matmul__` / `__rmatmul__` dunder at runtime (exactly like `+`/`*` route to
# `rt_obj_add`/`rt_obj_mul`): the frontend lowers `@` to `BinOp::MatMul`, codegen
# emits the tagged `rt_obj_matmul`, and the runtime dispatches the dunder (or
# raises `TypeError`). typeck types `a @ b` as the `__matmul__` declared return,
# so attribute access on a matrix-product result resolves statically. `@=` falls
# back to `__matmul__` (the same convention `+=` uses for `__add__` — in-place
# `__imatmul__` is a separate, pre-existing gap).
#
# `==` asserts are the spec (Principle 9); prints feed the differential harness.


# ===== __matmul__ returning a scalar (dot product) =====
class Vec:
    def __init__(self, x: int, y: int) -> None:
        self.x = x
        self.y = y

    def __matmul__(self, other: "Vec") -> int:
        return self.x * other.x + self.y * other.y


v1 = Vec(1, 2)
v2 = Vec(3, 4)
assert (v1 @ v2) == 11  # 1*3 + 2*4
assert (v2 @ v1) == 11
assert (Vec(0, 0) @ v1) == 0
print(v1 @ v2)  # 11


# ===== __matmul__ returning an instance (dunder return type drives attr access) =====
class Mat:
    def __init__(self, v: int) -> None:
        self.v = v

    def __matmul__(self, other: "Mat") -> "Mat":
        return Mat(self.v * other.v)


product = Mat(3) @ Mat(5)
assert product.v == 15  # the result is statically a `Mat`, so `.v` type-checks
print(product.v)  # 15

# Chained matmul (left-associative).
chained = Mat(2) @ Mat(3) @ Mat(4)
assert chained.v == 24
print(chained.v)  # 24


# ===== __rmatmul__ : the left operand's type has no __matmul__ =====
class Scaled:
    def __init__(self, k: int) -> None:
        self.k = k

    def __rmatmul__(self, other: int) -> int:
        return other + self.k


assert (10 @ Scaled(5)) == 15  # int has no __matmul__ → Scaled.__rmatmul__(10)
print(10 @ Scaled(5))  # 15


# ===== `@=` augmented assignment (falls back to __matmul__) =====
acc = Mat(2)
acc @= Mat(7)
assert acc.v == 14
print(acc.v)  # 14


# ===== matmul inside a function / over a loop =====
def total_dot(pairs: list[tuple[Vec, Vec]]) -> int:
    total = 0
    for a, b in pairs:
        total += a @ b
    return total


assert total_dot([(Vec(1, 1), Vec(2, 2)), (Vec(3, 0), Vec(1, 5))]) == 4 + 3
print(total_dot([(Vec(1, 1), Vec(2, 2)), (Vec(3, 0), Vec(1, 5))]))  # 7


# ===== TypeError on unsupported operands (no __matmul__/__rmatmul__) =====
caught_num = False
try:
    _ = 3 @ 4
except TypeError:
    caught_num = True
assert caught_num
print(caught_num)  # True

caught_obj = False


class NoMat:
    def __init__(self) -> None:
        self.q = 1


try:
    _ = NoMat() @ NoMat()
except TypeError:
    caught_obj = True
assert caught_obj
print(caught_obj)  # True


print("matmul (@ / __matmul__) tests passed!")
