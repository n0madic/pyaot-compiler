# Class instantiation + method dispatch.
# Mixes direct (monomorphic) and polymorphic (vtable) calls — Phase 1's
# WPA should collapse some of these; Phase 2's tagged values will change
# the per-call cost. Keeping both paths isolates regressions.

class Point:
    x: float
    y: float

    def __init__(self, x: float, y: float) -> None:
        self.x = x
        self.y = y

    def norm(self) -> float:
        return self.x * self.x + self.y * self.y


class Shape:
    def area(self) -> float:
        return 0.0


class Circle(Shape):
    r: float

    def __init__(self, r: float) -> None:
        self.r = r

    def area(self) -> float:
        return 3.14159 * self.r * self.r


class Square(Shape):
    s: float

    def __init__(self, s: float) -> None:
        self.s = s

    def area(self) -> float:
        return self.s * self.s


def main() -> None:
    n: int = 200_000

    # Monomorphic: only Point.norm() is dispatched.
    total_mono: float = 0.0
    for i in range(n):
        p: Point = Point(float(i), float(i) + 1.0)
        total_mono = total_mono + p.norm()

    # Polymorphic: alternates Circle / Square through a Shape handle.
    total_poly: float = 0.0
    for i in range(n):
        s: Shape = Circle(float(i)) if i % 2 == 0 else Square(float(i))
        total_poly = total_poly + s.area()

    print("classes:", total_mono, total_poly)


if __name__ == "__main__":
    main()
