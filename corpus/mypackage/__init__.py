# mypackage/__init__.py — exercises a relative import from a submodule
# (`from .utils import double`) during package initialization.
from .utils import double

greet: str = "Hello from mypackage"


def helper() -> int:
    return 42


def get_doubled_value() -> int:
    # Uses the relative-imported `double` from .utils.
    return double(21)
