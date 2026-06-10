# ── a basic generator expression consumed by a for-loop ──
xs = [1, 2, 3, 4, 5]
for v in (x * 2 for x in xs):
    print(v)


# ── genexpr materialized with list() ──
print(list(n * n for n in xs))


# ── genexpr with a filter ──
print(list(x for x in xs if x % 2 == 1))


# ── sum over a genexpr ──
print(sum(x for x in range(10)))


# ── a nested genexpr (inner clause over a literal) ──
pairs = ((a, b) for a in [1, 2] for b in [10, 20])
for p in pairs:
    print(p)


# ── genexpr over a string ──
print(list(c for c in "abc"))


# ── a genexpr driven by next() ──
g = (i + 1 for i in range(3))
print(next(g), next(g), next(g))
