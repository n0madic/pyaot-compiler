"""Phase 5 — GC soak for class instances (PITFALLS A5/B12/B15).

Instance fields are uniform tagged `Value` slots traced via `Value::is_ptr()` —
no per-field side-table. This program keeps a class-instance graph (instances
holding instances in Tagged fields) live across thousands of allocating calls,
forcing many collections in between. If a field slot is not traced, an inner
instance is freed underfoot and the output diverges from CPython (or crashes).
"""


class Inner:
    def __init__(self, n: int):
        self.n = n

    def get(self) -> int:
        return self.n


class Outer:
    def __init__(self, inner: Inner, tag: int):
        self.inner = inner
        self.tag = tag

    def total(self) -> int:
        # Reaches the inner instance through a Tagged field — it must be alive.
        return self.inner.get() + self.tag


# A survivor instance graph that must outlive every loop below.
survivor = Outer(Inner(7), 100)

# Build a list of 5000 Outer instances; each holds an Inner reachable ONLY through
# its Tagged `inner` field. Every Inner()/Outer()/append allocates, forcing GC
# while `outers` and `survivor` stay rooted.
outers: list[Outer] = []
for i in range(5000):
    inner = Inner(i)
    outer = Outer(inner, i * 2)
    outers.append(outer)

# Traverse: each `o.total()` dereferences the Tagged `inner` field of an instance
# that has survived thousands of intervening collections.
s = 0
for o in outers:
    s = s + o.total()
print(len(outers))
print(s)

# A bignum accumulator re-boxed every iteration while the instance graph is live.
big = 0
for o in outers:
    big = big + o.inner.get() * 1000000000000000000
print(big)

# Mutate fields in a loop (each iteration allocates a fresh Inner that replaces
# the old one — the replaced instances become garbage mid-loop).
for o in outers:
    o.inner = Inner(o.tag)
acc = 0
for o in outers:
    acc = acc + o.inner.get()
print(acc)

# The survivor graph is still intact after all that allocation.
print(survivor.total())
print(survivor.inner.get())
