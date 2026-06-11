# Runtime ShadowFrame audit — zip over FRESH-element sources. A string /
# enumerate / map / generator source ALLOCATES each element inside the inner
# next(); the zip nexts must root already-obtained items across the remaining
# inner nexts and the result-tuple allocation, or a gc_stress collection frees
# them (use-after-free). List sources never caught this: their elements stay
# reachable through the source list.


# ── zip(2) over two strings: both items are fresh StrObj chars ──
for a, b in zip("abc", "xyz"):
    print(a, b)

# ── zip(2): fresh item1 (string) against an allocating second source ──
def gen_words():
    for i in range(3):
        yield "w" + str(i)


for c, w in zip("pqr", gen_words()):
    print(c, w)

# (zip of 3+ iterables is not in the frontend yet — the runtime's zip3/zipN
# nexts carry the same rooting defensively.)

# ── zip over enumerate (its elements are fresh tuples) ──
for pair, ch in zip(enumerate("mn"), "uv"):
    print(pair[0], pair[1], ch)

# ── tuple()/list() drain a zip of fresh elements through one more alloc ──
print(list(zip("ab", "cd")))
print(tuple(zip("gh", "ij")))

# (reduce with a heap accumulator would exercise the same hazard, but the
# legacy rt_reduce ABI only supports int/bool accumulators today — the
# tagged variant carries the rooting defensively.)

print("p9 zip fresh elems passed!")
