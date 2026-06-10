# shapes/__init__.py — the canonical package facade: re-export the public names
# from a submodule so callers write `from shapes import Circle`, not
# `from shapes.circle import Circle`. Exercises re-export of a class, a function,
# and a constant through the package surface.
from .circle import Circle, unit_area, PI

VERSION: str = "1.0"
