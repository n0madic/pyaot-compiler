# Conditional-import test support module (test_import.py SECTION 9). This module
# is imported ONLY from inside an `if` branch, so its module `<init>` (which sets
# the FACTOR global) must be emitted in-position and run only because the branch
# is taken — the conditional-`<init>` path.
FACTOR: int = 3


def scale(n: int) -> int:
    return n * FACTOR
