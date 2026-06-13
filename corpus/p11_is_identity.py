"""Backlog §2 — `is` / `is not` against non-`None` operands.

Identity is defined as bit-identity (PLAN §2 trap): bool/None singletons,
same-object, and distinct-object cases all match CPython. Integer and string
identity *caching* is implementation-defined and deliberately NOT exercised
here (equal fixnums are bit-identical under this model, equal heap strings are
not). `type(x) is T` waits on the `type()` builtin and is out of scope.
"""


# ── bool singletons ─────────────────────────────────────────────────────────
print(True is True)
print(False is False)
print(True is False)
print(True is not False)
print(True is not True)

flag = True
other = False
print(flag is True)
print(flag is False)
print(other is False)
print(flag is not False)


# ── class-instance identity ─────────────────────────────────────────────────
class Box:
    def __init__(self, v: int) -> None:
        self.v = v


a = Box(1)
b = Box(1)
c = a
print(a is b)        # distinct objects
print(a is a)        # same object
print(a is c)        # alias of the same object
print(a is not b)
print(c is not a)
print(a is None)     # the dedicated None path still works alongside


# ── container identity (distinct literals are distinct objects) ──────────────
l1 = [1, 2, 3]
l2 = [1, 2, 3]
l3 = l1
print(l1 is l2)
print(l1 is l3)
print(l1 is not l2)

d1 = {"k": 1}
d2 = d1
print(d1 is d2)


# ── interaction probes: identity inside guards, with and/or/not ─────────────
if a is c and flag is True:
    print("guard-and ok")
if not (a is b):
    print("guard-not ok")

i = 0
while a is c and i < 2:
    print("loop", i)
    i += 1


# ── identity as a stored/returned bool value ────────────────────────────────
same = a is c
print(same)
print(l1 is l2 or a is c)
