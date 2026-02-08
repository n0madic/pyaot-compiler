# mypackage/math/ops.py
# Math operations with relative imports

# Relative import from parent's sibling (mypackage/utils.py)
from ..utils import double
# Relative import of variable from parent package (mypackage/math/__init__.py)
from . import PI

def add(a: int, b: int) -> int:
    return a + b

def multiply(a: int, b: int) -> int:
    return a * b

def subtract(a: int, b: int) -> int:
    return a - b

def doubled_ten() -> int:
    # Use the imported double function from parent's utils
    return double(10)

def get_doubled(x: int) -> int:
    return double(x)

def get_pi() -> float:
    # Use the imported PI variable from parent package
    return PI
