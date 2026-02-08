# mypackage/math/__init__.py
# Math subpackage with relative imports

# Relative import from parent package
from .. import greet

PI: float = 3.14159
E: float = 2.71828

def square(x: int) -> int:
    return x * x

def get_parent_greeting() -> str:
    # Use the imported variable from parent package
    return greet
