"""Phase 5C — arithmetic / comparison / conversion dunders on a concrete class.

Output is deterministic: instances are rendered via __repr__ (registered, so the
runtime's default-repr path dispatches it), never the address-bearing default repr.
"""


class Vector:
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

    def __add__(self, other: Vector) -> Vector:
        return Vector(self.x + other.x, self.y + other.y)

    def __sub__(self, other: Vector) -> Vector:
        return Vector(self.x - other.x, self.y - other.y)

    def __mul__(self, scalar: int) -> Vector:
        return Vector(self.x * scalar, self.y * scalar)

    def __neg__(self) -> Vector:
        return Vector(-self.x, -self.y)

    def __eq__(self, other: Vector) -> bool:
        return self.x == other.x and self.y == other.y

    def __repr__(self) -> str:
        return "Vector(" + str(self.x) + ", " + str(self.y) + ")"


a = Vector(1, 2)
b = Vector(3, 4)

# Arithmetic — routes through rt_obj_add/sub/mul/neg (registered dunders).
print(a + b)
print(a - b)
print(a * 3)
print(-a)

# Comparison — compiler-routed (rt_obj_eq does not dispatch class __eq__).
print(a == b)
print(a == Vector(1, 2))
print(a != b)
print(a != Vector(1, 2))

# Results are precisely typed (Vector), so field access works on them.
c = a + b
print(c.x)
print(c.y)
d = a * 2
print(d.x)
print(d.y)

# __repr__ directly, and via print().
print(a)
print(b)
