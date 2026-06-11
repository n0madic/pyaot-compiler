# Phase 8H D3 (extended) — checked Dyn->Raw unbox coverage beyond the basic
# p8h_checked_unbox.py: Optional/None flows, container-element Dyn sources,
# dict.get misses, raw-i64 stdlib params (math.gcd/comb/factorial/perm), bool
# promotion through Dyn, str rejection at a raw-i64 boundary, and chained Dyn
# producers. Small ints only: rt_unbox_int is a 64-bit unbox, BigInt-through-
# raw-i64 is out of scope.

import math


# ── 1. Optional (Union[float, None]) into a raw-f64 param ──
def opt(flag):
    if flag:
        return 2.25
    return None


print(math.sqrt(opt(True)))
try:
    print(math.sqrt(opt(False)))
except TypeError:
    print("TypeError caught: None into sqrt")


# ── 2. Dyn from a mixed-list element into raw-f64 (int/bool promote) ──
mixed = [2.25, 16, True]
print(math.sqrt(mixed[0]))
print(math.sqrt(mixed[1]))
print(math.sqrt(mixed[2]))
print(math.floor(mixed[0]))

# ── 3. Dyn from dict.get into raw-f64; a miss (None) raises TypeError ──
table = {"a": 6.25}
print(math.sqrt(table.get("a")))
try:
    print(math.sqrt(table.get("b")))
except TypeError:
    print("TypeError caught: dict.get miss")


# ── 4. Dyn into raw-i64 params (math.gcd / comb / factorial / perm) ──
def pick_int(flag):
    if flag:
        return 12
    return 8


print(math.gcd(pick_int(True), 18))
print(math.comb(pick_int(False), 2))

dyn_ints = [12, 5, "nope"]
print(math.gcd(dyn_ints[0], 18))
print(math.comb(dyn_ints[1], 2))
print(math.factorial(dyn_ints[1]))
print(math.perm(dyn_ints[1], 2))


# ── 5. bool through a Dyn return into raw-i64 (CPython: bool is an int) ──
def maybe_bool(flag):
    if flag:
        return True
    return 6


print(math.gcd(maybe_bool(True), 8))
print(math.gcd(maybe_bool(False), 8))

# ── 6. str through Dyn into raw-i64 raises TypeError (caught, no crash) ──
try:
    print(math.gcd(dyn_ints[2], 4))
except TypeError:
    print("TypeError caught: str into gcd")


# ── 7. Chained Dyn producers into raw-f64 ──
def wrap(v):
    return v


print(math.sqrt(wrap(opt(True))))

# ── 8. Contrast: statically proven args keep the unchecked fast path ──
print(math.gcd(12, 18))
print(math.sqrt(2.25))

print("p8h checked unbox 2 passed!")
