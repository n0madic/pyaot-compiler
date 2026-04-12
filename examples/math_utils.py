# math_utils.py
PI: float = 3.14159
E: float = 2.71828
NAME: str = "math_utils"

def add(a: int, b: int) -> int:
    return a + b

def multiply(x: int, y: int) -> int:
    return x * y

class Point:
    x: int
    y: int

    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

    def sum(self) -> int:
        return self.x + self.y

    def describe(self) -> str:
        return "Point(" + str(self.x) + "," + str(self.y) + ")"


def origin() -> Point:
    return Point(0, 0)


def point_at(x: int, y: int) -> Point:
    return Point(x, y)
