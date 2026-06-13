# Backlog §4 — nested destructuring (`a, (b, c) = …`).
#
# Nested sequence targets in all three contexts: assignment, `for`-loop targets,
# and comprehension / generator-expression targets. `assign_to_target` recurses
# into a Tuple/List target through `lower_unpack_subscript` — each nested element
# is staged and re-subscripted positionally, so deeper nesting and the for/comp
# paths fall out of the same machinery. Flat and starred forms already worked.
#
# Covers: literal RHS, runtime RHS (variable / call, forcing the subscript path),
# mixed tuple/list, deep nesting, nested attribute/subscript leaves, nested +
# starred (assignment only — a starred leaf inside a comprehension is a separate,
# unrelated gap), and interaction probes crossing nesting with comprehension
# element-type inference + `sum()` and with `*seq` spread (§13).


# ===== Assignment, literal RHS =====
a1, (b1, c1) = 1, (2, 3)
print(a1, b1, c1)                         # 1 2 3

# Deep nesting (3 levels).
x1, (y1, (z1, w1)) = 10, (20, (30, 40))
print(x1, y1, z1, w1)                     # 10 20 30 40

# Multiple nested groups.
(m1, m2), (m3, m4) = (1, 2), (3, 4)
print(m1, m2, m3, m4)                     # 1 2 3 4

# Mixed tuple/list.
g1, [h1, i1] = 1, [2, 3]
print(g1, h1, i1)                         # 1 2 3

# Single-element parenthesized target.
(p1,) = (9,)
print(p1)                                 # 9


# ===== Assignment, runtime RHS (forces the subscript path) =====
# A Name RHS is not a literal sequence, so the outer unpack stages-then-subscripts.
pair: tuple[int, tuple[int, int]] = (1, (2, 3))
a2, (b2, c2) = pair
print(a2, b2, c2)                         # 1 2 3


def make_nested() -> tuple[int, tuple[int, int]]:
    return (1, (2, 3))


# A call RHS likewise drives the subscript path.
a3, (b3, c3) = make_nested()
print(a3, b3, c3)                         # 1 2 3


# ===== Nested attribute / subscript leaves =====
class Box:
    x: int
    y: int

    def __init__(self) -> None:
        self.x = 0
        self.y = 0


box = Box()
slot: list[int] = [0, 0]
a4, (box.x, slot[0]) = 1, (2, 3)
print(a4, box.x, slot[0])                 # 1 2 3


# ===== Nested + starred (assignment context only) =====
a5, (b5, c5) = 1, (2, 3)
print(a5, b5, c5)                         # 1 2 3

s1, (s2, *s3, s4) = 1, (2, 3, 4, 5)
print(s1, s2, s3, s4)                     # 1 2 [3, 4] 5

t1, (*t2, t3) = 1, [2, 3, 4]
print(t1, t2, t3)                         # 1 [2, 3] 4


# ===== For-loop nested target =====
for fa, (fb, fc) in [(1, (2, 3)), (4, (5, 6))]:
    print(fa, fb, fc)                     # 1 2 3 / 4 5 6

for fg, [fh, fi] in [(1, [2, 3]), (4, [5, 6])]:
    print(fg, fh, fi)                     # 1 2 3 / 4 5 6


# ===== Comprehensions with nested targets =====
lc = [a + b + c for a, (b, c) in [(1, (2, 3)), (4, (5, 6))]]
print(lc)                                 # [6, 15]

dc = {a: b + c for a, (b, c) in [(1, (2, 3)), (4, (5, 6))]}
print(dc)                                 # {1: 5, 4: 11}

sc = sorted({a + b + c for a, (b, c) in [(1, (2, 3)), (4, (5, 6))]})
print(sc)                                 # [6, 15]

# Multi-clause: outer clause binds a row, inner clause unpacks nested tuples.
mc = [a + b for row in [[(1, (2, 3))], [(4, (5, 6))]] for a, (b, c) in row]
print(mc)                                 # [3, 9]


# ===== Generator expression with a nested target =====
gs = sum(a * b + c for a, (b, c) in [(1, (2, 3)), (4, (5, 6))])
print(gs)                                 # (1*2+3) + (4*5+6) = 31


# ===== Interaction probes (cross nesting with green features) =====
# Nested-target comprehension feeding sum() (element-type inference + sum, §D1/D2).
total = sum(x for x in [a + b + c for a, (b, c) in [(1, (2, 3)), (4, (5, 6))]])
print(total)                              # 6 + 15 = 21


def add3(p: int, q: int, r: int) -> int:
    return p + q + r


# A triple built from a nested unpack, spread into a fixed-arity callee (§13).
ia, (ib, ic) = 1, (2, 3)
triple = (ia, ib, ic)
print(add3(*triple))                      # 6

print("Nested destructuring tests passed!")
