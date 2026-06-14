# Backlog §1 — mutable / computed parameter defaults on top-level functions.
#
# Python evaluates a parameter default ONCE, at `def`-execution time; a mutable
# default is the SAME object reused across calls (the famous "mutable default
# gotcha"), and a computed default (`5 + 5`, an expression over module globals)
# is evaluated once at def time. The compiler realizes both with a synthetic
# GC-rooted global slot: set once at the def's module-init position, read
# (shared) at every defaulted call.

# ── Mutable list default: shared across calls (aliasing trap) ──
def append_to_list(x: int, lst: list[int] = []) -> list[int]:
    lst.append(x)
    return lst

r1: list[int] = append_to_list(1)
assert len(r1) == 1, "first call has 1 element"
assert r1[0] == 1

r2: list[int] = append_to_list(2)
assert len(r2) == 2, "second call shares the list"
assert r2[0] == 1
assert r2[1] == 2

r3: list[int] = append_to_list(3)
assert len(r3) == 3
assert r3[2] == 3

# All three results are the SAME shared list object.
assert r1 == r2, "results are the same list"
assert r2 == r3
assert r1 is r2, "identity: one shared default object"
assert r2 is r3

# An explicit argument does NOT touch the shared default.
fresh: list[int] = [100]
r4: list[int] = append_to_list(4, fresh)
assert len(r4) == 2
assert r4[0] == 100
assert r4[1] == 4
assert r4 is fresh, "explicit arg is used, not the default"

# The default list still grows on the next defaulted call.
r5: list[int] = append_to_list(5)
assert len(r5) == 4
assert r5[3] == 5
assert r5 is r1, "still the same shared default"
print("mutable list default aliasing passed")

# ── Mutable dict default: shared across calls ──
def collect(k: str, v: int, d: dict[str, int] = {}) -> dict[str, int]:
    d[k] = v
    return d

d1 = collect("a", 1)
d2 = collect("b", 2)
assert len(d2) == 2
assert d1 is d2
assert d1["a"] == 1
assert d1["b"] == 2
print("mutable dict default aliasing passed")

# ── Computed default: an arithmetic expression, evaluated once ──
def expr_defaults(*, name: str = "default", count: int = 5 + 5) -> str:
    return name + ":" + str(count)

assert expr_defaults() == "default:10"
assert expr_defaults(name="custom") == "custom:10"
assert expr_defaults(count=20) == "default:20"
print("computed default passed")

# ── Computed default over a module global, evaluated in module scope at def time ──
BASE: int = 100

def scaled(x: int, factor: int = BASE * 2) -> int:
    return x + factor

assert scaled(1) == 201
assert scaled(1, 0) == 1
assert scaled(5) == 205
print("module-global computed default passed")

# ── A non-empty list default starts from its initial elements (once) ──
def accumulate(x: int, acc: list[int] = [0]) -> list[int]:
    acc.append(x)
    return acc

a1 = accumulate(1)
assert a1 == [0, 1]
a2 = accumulate(2)
assert a2 == [0, 1, 2]
assert a1 is a2
print("seeded list default passed")

# ── Literal defaults are unchanged (regression): per-call fresh, not shared ──
def with_literals(a: int, b: int = 10, c: str = "z", d: bool = True) -> str:
    return str(a) + str(b) + c + str(d)

assert with_literals(1) == "110zTrue"
assert with_literals(1, 2) == "12zTrue"
assert with_literals(1, 2, "q") == "12qTrue"
assert with_literals(1, 2, "q", False) == "12qFalse"
print("literal defaults regression passed")

print("All mutable/computed default tests passed")
