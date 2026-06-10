"""Phase 5A — single classes: fields, methods, construction, class annotations.

Only field VALUES are printed (never a bare instance — the default repr carries a
non-deterministic address; that path is gated from 5C via __str__/__repr__).
"""


class Widget:
    def __init__(self, w: int, h: int):
        self.w = w
        self.h = h

    def area(self) -> int:
        return self.w * self.h

    def scale(self, factor: int):
        self.w = self.w * factor
        self.h = self.h * factor


class Counter:
    def __init__(self):
        self.count = 0

    def increment(self):
        self.count = self.count + 1

    def add(self, n: int):
        self.count += n

    def value(self) -> int:
        return self.count


class Point:
    def __init__(self, x: float, y: float):
        self.x = x
        self.y = y

    def shifted(self, dx: float, dy: float) -> float:
        return self.x + dx + self.y + dy


def make_widget(w: int, h: int) -> Widget:
    return Widget(w, h)


def total_area(a: Widget, b: Widget) -> int:
    return a.area() + b.area()


w = Widget(3, 4)
print(w.area())
print(w.w)
print(w.h)
w.scale(2)
print(w.area())
print(w.w, w.h)

c = Counter()
c.increment()
c.increment()
c.add(5)
print(c.value())

w2 = make_widget(10, 2)
print(w2.area())
print(total_area(w, w2))

p = Point(1.5, 2.5)
print(p.shifted(0.5, 0.5))
print(p.x, p.y)
