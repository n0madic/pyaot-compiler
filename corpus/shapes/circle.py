# shapes/circle.py — leaf submodule defining the public Circle type.
PI: float = 3.14159


class Circle:
    def __init__(self, radius: float) -> None:
        self.radius = radius

    def area(self) -> float:
        return PI * self.radius * self.radius


def unit_area() -> float:
    return Circle(1.0).area()
