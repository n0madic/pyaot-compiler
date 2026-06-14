# `collections.Counter` (§10) — a dict subclass for counting hashable elements.
#
# Pure front-half WIRING over the pre-existing runtime (`counter.rs` + the shared
# `DictObj` layout under `TypeTagKind::Counter`), plus the runtime additions a
# differential-correct Counter needs: `rt_counter_get` (missing key → 0, not
# KeyError), a CPython-faithful `Counter({...})` repr in most-common order, and
# the dict-family seam (len / `in` / iteration / keys-values-items accept the
# Counter tag). Construction picks `rt_make_counter_empty` vs
# `rt_make_counter_from_iter` by arity; the result is typed `RuntimeObject(Counter)`
# so the `.most_common()/.total()/.update()/.subtract()` methods dispatch.
#
# Out of scope (documented): `Counter(mapping)` / `Counter(**kwargs)` (would count
# keys, not honor mapped counts), Counter arithmetic (`c1 + c2`), and `.elements()`.
#
# `==`/`assert` are the spec (Principle 9); `print` feeds the differential harness.


from collections import Counter


# ===== construction + repr (most-common order, CPython 3.x) =====
c = Counter("aabbbc")          # a:2, b:3, c:1
print(c)                       # Counter({'b': 3, 'a': 2, 'c': 1})
print(Counter([1, 1, 2, 3, 3, 3]))   # Counter({3: 3, 1: 2, 2: 1})
print(Counter(("x", "y", "x")))      # Counter({'x': 2, 'y': 1})
print(Counter())               # Counter()
print(Counter(ch for ch in "hello"))  # Counter({'l': 2, 'h': 1, 'e': 1, 'o': 1})

# str() / repr() / f-string go through the same most-common repr.
assert str(c) == "Counter({'b': 3, 'a': 2, 'c': 1})"
assert repr(c) == "Counter({'b': 3, 'a': 2, 'c': 1})"
print(f"{c}")                  # Counter({'b': 3, 'a': 2, 'c': 1})


# ===== subscript read: present, and MISSING → 0 (no KeyError) =====
assert c["a"] == 2
assert c["b"] == 3
assert c["x"] == 0             # missing key → 0
before = len(c)
_ = c["zzz"]                   # reading a missing key must NOT insert it
assert len(c) == before
print(c["a"], c["b"], c["x"])  # 2 3 0


# ===== subscript write + augmented assignment (missing starts at 0) =====
c["a"] += 1                    # 2 -> 3
c["z"] += 5                    # missing -> 0 -> 5 (inserts 'z')
c["c"] = 10                    # direct set
print(c["a"], c["z"], c["c"])  # 3 5 10


# ===== len / membership =====
print(len(c))                  # 4  (a, b, c, z)
print("a" in c, "x" in c)      # True False
c["zero"] = 0                  # a key with count 0 is still a member
assert "zero" in c
assert "x" not in c
print("zero" in c)             # True


# ===== iteration (keys), sorted, list =====
print(sorted(c))               # ['a', 'b', 'c', 'z', 'zero']
print(sorted(list(c)))         # ['a', 'b', 'c', 'z', 'zero']
total_keys = 0
for _k in c:
    total_keys += 1
assert total_keys == len(c)


# ===== keys() / values() / items() =====
print(sorted(c.keys()))                       # ['a', 'b', 'c', 'z', 'zero']
print(sorted(c.values()))                     # [0, 3, 3, 5, 10]
print(sorted(c.items()))                      # [('a', 3), ('b', 3), ('c', 10), ('z', 5), ('zero', 0)]
# comprehension over items (cross with a green feature).
doubled = {k: v * 2 for k, v in c.items()}
assert doubled["z"] == 10
print(sorted(doubled.items()))


# ===== most_common / total =====
c2 = Counter("aabbbcccc")      # a:2, b:3, c:4
print(c2.most_common(2))       # [('c', 4), ('b', 3)]
print(c2.most_common())        # [('c', 4), ('b', 3), ('a', 2)]
print(c2.total())              # 9
assert c2.total() == 9
# most_common(0) and most_common(negative) are empty (only the no-arg form
# returns all) — the runtime's i64::MIN sentinel keeps them distinct.
print(c2.most_common(0))       # []
print(c2.most_common(-1))      # []
print(c2.most_common(100))     # all 3 (n >= len)
assert c2.most_common(0) == []
# a tie keeps insertion order (stable sort), matching CPython.
print(Counter("abab").most_common())   # [('a', 2), ('b', 2)]


# ===== update / subtract (any iterable; negative counts allowed) =====
d = Counter()
d.update("aax")
print(d["a"], d["x"])          # 2 1
d.update(["a", "y", "y"])
print(d["a"], d["y"])          # 3 2
d.subtract("aaa")              # a: 3 - 3 = 0
print(d["a"])                  # 0
neg = Counter("ab")            # a:1, b:1
neg.subtract("aabb")           # a:1-2=-1, b:1-2=-1
print(neg["a"], neg["b"])      # -1 -1
print(neg)                     # Counter({'a': -1, 'b': -1})


# ===== truthiness =====
print(bool(Counter()))         # False
print(bool(Counter("a")))      # True
if Counter("x"):
    print("non-empty is truthy")   # non-empty is truthy
assert not Counter()


# ===== Counter through annotated functions (param + return) =====
def total_count(counter: Counter) -> int:
    return counter.total()


def char_freq(text: str) -> Counter:
    return Counter(text)


assert total_count(Counter("hello")) == 5
freq = char_freq("mississippi")   # m:1, i:4, s:4, p:2
print(freq.most_common(1))        # [('i', 4)]  (i before s — insertion order tie-break)
print(freq["s"], freq["m"], freq["q"])   # 4 1 0


# ===== a typical word-frequency use =====
words = "the quick brown fox the lazy dog the".split()
wc = Counter(words)
print(wc.most_common(1))          # [('the', 3)]
print(wc["the"], wc["fox"], wc["cat"])   # 3 1 0

print("p35 counter OK")
