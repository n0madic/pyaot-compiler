# Phase 8A — package re-export: a package `__init__.py` that publishes names it
# imported from a submodule (`from .circle import Circle, unit_area, PI`). The
# canonical Python package facade. Both access styles must resolve.

# ── Style 1: `from package import <re-exported names>`. ──
from shapes import Circle, unit_area, PI

c: Circle = Circle(2.0)
area: float = c.area()
assert area > 12.56, "area should be ~12.566"
assert area < 12.57, "area should be ~12.566"
print(area)

u: float = unit_area()
assert u > 3.14, "unit_area should be ~3.14159"
assert u < 3.15, "unit_area should be ~3.14159"
print(u)

assert PI > 3.14, "PI re-export should equal 3.14159"
print(PI)

# ── Style 2: `import package; package.<re-exported name>`. ──
import shapes

c2: shapes.Circle = shapes.Circle(3.0)
area2: float = c2.area()
assert area2 > 28.27, "area2 should be ~28.274"
assert area2 < 28.28, "area2 should be ~28.274"
print(area2)

print(shapes.unit_area())
print(shapes.VERSION)

print("All re-export tests passed!")
