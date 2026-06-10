# mypackage/math/__init__.py — exercises `from .. import <var>` (importing a
# name from the parent package during nested-package initialization).
from .. import greet

PI: float = 3.14159


def square(x: int) -> int:
    return x * x


def get_parent_greeting() -> str:
    # Returns the parent package's `greet`, relative-imported above.
    return greet
