# Phase 8H D3 — checked Dyn->Raw unbox at stdlib raw-ABI boundaries.
# Gradual (Dyn) and numeric arguments reach math.* raw-f64 params without
# annotations; a genuinely wrong tag raises TypeError instead of SEGV.

import math


def pick(flag):
    if flag:
        return 2.25
    return 4.0


# Dyn argument (unannotated function return) into a raw-f64 param
v = pick(True)
print(math.sqrt(v))
print(math.floor(pick(False)))

# int / bool arguments into a raw-f64 param (CPython promotes)
print(math.sqrt(16))
print(math.sqrt(True))
print(math.exp(0))
print(math.log(1))

# int literal still works, floats keep the fast path
print(math.sqrt(2.25))
print(math.pow(2, 10))

# a wrong tag raises TypeError (caught, not a crash)
def bad(flag):
    if flag:
        return "not a number"
    return 1.0


try:
    print(math.sqrt(bad(True)))
except TypeError:
    print("TypeError caught")

print("p8h checked unbox passed!")
