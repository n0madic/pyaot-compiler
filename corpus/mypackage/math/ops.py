# mypackage/math/ops.py — exercises `from ..utils import double` (a relative
# import reaching a sibling subpackage) and `from . import PI` (a name from the
# current package).
from ..utils import double
from . import PI


def add(a: int, b: int) -> int:
    return a + b


def multiply(a: int, b: int) -> int:
    return a * b


def doubled_ten() -> int:
    return double(10)


def get_doubled(x: int) -> int:
    return double(x)


def get_pi() -> float:
    return PI
