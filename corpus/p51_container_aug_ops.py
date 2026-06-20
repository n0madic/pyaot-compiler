# §9 container augmented ops + `dict.fromkeys` class form.
#
# (1) `dict.fromkeys(keys[, value])` CLASS form — bare `dict` in receiver
#     position used to lower to an unresolved `Dyn`, so the instance-form
#     `fromkeys` dispatch never fired. It now desugars to a `MethodCall` on a
#     throwaway empty-dict receiver (typed `dict` ⇒ the existing Fromkeys path).
#     CPython shares ONE value object across all keys — the aliasing witness
#     below proves pyaot does too.
# (2) Augmented set ops `&=` / `-=` / `^=` are now in-place (`IAnd`/`ISub`/`IXor`
#     → `rt_set_*_update`, returning the same object), like the existing `|=`.
#     The desugar `s = s & o` would rebind to a NEW object; an alias would not
#     see the mutation. Each op below keeps an alias and asserts it observes the
#     change — the divergence being closed.

# ===== dict.fromkeys class form =====
d = dict.fromkeys(["a", "b", "c"], 0)
print(sorted(d.items()))
e = dict.fromkeys(["x", "y"])
print(sorted(e.keys()))
print(e["x"] is None)
# Snapshot any iterable (range / tuple / generator) via the iterator protocol.
print(sorted(dict.fromkeys(range(3), 9).items()))
print(sorted(dict.fromkeys((1, 2), "v").items()))
# Value aliasing: one shared value object across all keys (CPython semantics).
shared = dict.fromkeys([1, 2, 3], [])
shared[1].append("z")
print(shared[2])
print(shared[1] is shared[2])
print(shared[2] is shared[3])

# ===== set &= with an alias witness =====
s = {1, 2, 3, 4}
t = s
s &= {2, 3, 5}
assert t is s, "&= must mutate in place"
assert sorted(s) == [2, 3], "&= result"
print("iand", sorted(t))

# ===== set -= with an alias witness =====
a = {1, 2, 3, 4}
b = a
a -= {2, 4}
assert b is a, "-= must mutate in place"
assert sorted(a) == [1, 3], "-= result"
print("isub", sorted(b))

# ===== set ^= with an alias witness =====
c = {1, 2, 3}
g = c
c ^= {2, 3, 4}
assert g is c, "^= must mutate in place"
assert sorted(c) == [1, 4], "^= result"
print("ixor", sorted(g))

# ===== |= regression (already in-place) =====
p = {1, 2}
q = p
p |= {3, 4}
assert q is p, "|= must mutate in place"
print("ior", sorted(q))

# ===== numeric augmented ops still produce new values (not set ops) =====
n = 10
n -= 3
print("num -=", n)
x = 6
x &= 3
print("num &=", x)
y = 6
y ^= 3
print("num ^=", y)
z = 12
z |= 1
print("num |=", z)

# ===== aug ops on attribute / subscript set targets =====
holder = {"s": {1, 2, 3}}
holder["s"] &= {2, 3, 4}
print("subscript &=", sorted(holder["s"]))


class Box:
    def __init__(self):
        self.s = {1, 2, 3, 4}


box = Box()
alias = box.s
box.s -= {1, 2}
print("attr -=", sorted(box.s), alias is box.s)

print("container aug-op + dict.fromkeys tests passed!")
