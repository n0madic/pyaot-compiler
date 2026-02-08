# mypackage/__init__.py
# Package initialization file with relative imports

# Relative import from sibling module
from .utils import double

greet: str = "Hello from mypackage"

def helper() -> int:
    return 42

def get_doubled_value() -> int:
    # Use the imported function from .utils
    return double(21)
