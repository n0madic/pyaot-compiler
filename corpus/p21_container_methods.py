# §9 container methods (ContainerMethod path). Wires three receiver families
# whose runtime impls already existed but were not dispatched:
#   tuple : index / count           (value-comparing queries → int, B13)
#   set   : issubset / issuperset / isdisjoint (→ bool, B13)
#           intersection_update / difference_update / symmetric_difference_update
#           (mutate in place → None)
#   dict  : popitem                 (a fresh (k, v) 2-tuple, LIFO, B5)
#
# MECHANISM: dispatch keys on the receiver `SemTy` via `MethodRecv` (NEVER the
# method name alone — the §10 trap: `index`/`count`/`update` are shared names),
# routed to a new `ContainerOp` per method (hir op + codegen `d()` decl + lowering
# `MethodRecv` arm + typeck `method_ty`). Comparisons ride a proven `Raw(I8)`;
# updates mutate in place (None result); the `popitem` 2-tuple stays `Tagged`
# (GC-rootable) and typed `Dyn`, so `k, v = d.popitem()` unpacks through the
# gradual seam — the same approach as `str.partition`.
#
# The `==` / ValueError / KeyError assertions are the spec (Principle 9). Set
# contents are printed via `sorted(list(s))` to keep output deterministic.

# ===== tuple.index / .count (incl. duplicates, not-found → ValueError) =====
assert (1, 2, 3, 2).index(2) == 1
assert (1, 2, 3, 2).count(2) == 2
assert (5, 5, 5).count(5) == 3
assert (1, 2, 1, 3).index(1) == 0
assert ("a", "b", "c").index("c") == 2
assert ("x", "y", "x").count("x") == 2
# bound to a variable (a homogeneous fixed-arity tuple)
nums = (10, 20, 30, 20, 10)
assert nums.count(20) == 2
assert nums.index(30) == 2
# not found → ValueError
miss_caught = False
try:
    (1, 2, 3).index(99)
except ValueError:
    miss_caught = True
assert miss_caught
print("tuple:", (1, 2, 3, 2).index(2), (1, 2, 3, 2).count(2), "miss", miss_caught)

# ===== set.issubset / .issuperset / .isdisjoint (true + false cases) =====
assert {1, 2}.issubset({1, 2, 3}) == True
assert {1, 4}.issubset({1, 2, 3}) == False
assert {1, 2, 3}.issuperset({1, 2}) == True
assert {1, 2}.issuperset({1, 4}) == False
assert {1, 2}.isdisjoint({3, 4}) == True
assert {1, 2}.isdisjoint({2, 3}) == False
# empty-set edge: subset of anything, disjoint from anything
assert set().issubset({1, 2}) == True
assert {1, 2}.isdisjoint(set()) == True
print(
    "set cmp:",
    {1, 2}.issubset({1, 2, 3}),
    {1, 2, 3}.issuperset({1, 2}),
    {1, 2}.isdisjoint({3, 4}),
)

# ===== set.intersection_update (mutates in place → None) =====
s1 = {1, 2, 3, 4}
r1 = s1.intersection_update({2, 3, 5})
assert r1 is None
assert sorted(list(s1)) == [2, 3]
assert len(s1) == 2

# ===== set.difference_update =====
s2 = {1, 2, 3, 4}
s2.difference_update({2, 4})
assert sorted(list(s2)) == [1, 3]

# ===== set.symmetric_difference_update =====
s3 = {1, 2, 3}
s3.symmetric_difference_update({2, 3, 4})
assert sorted(list(s3)) == [1, 4]
print("set update:", sorted(list(s1)), sorted(list(s2)), sorted(list(s3)))

# ===== set.symmetric_difference (new-set algebra, distinct from *_update) =====
sa = {1, 2, 3}
sb = {2, 3, 4}
sd = sa.symmetric_difference(sb)
assert sorted(list(sd)) == [1, 4]
assert sorted(list(sa)) == [1, 2, 3]  # operands unchanged
assert sorted(list(sb)) == [2, 3, 4]
print("symdiff:", sorted(list(sd)))

# ===== list.remove (mutates in place → None; ValueError on miss) =====
li = [10, 20, 30, 20]
ret = li.remove(20)  # removes the FIRST occurrence
assert ret is None
assert li == [10, 30, 20]
li_str = ["a", "b", "c"]
li_str.remove("b")
assert li_str == ["a", "c"]
rm_miss = False
try:
    [1, 2, 3].remove(99)
except ValueError:
    rm_miss = True
assert rm_miss
print("list.remove:", li, li_str, "miss", rm_miss)

# ===== dict.popitem (LIFO, matches CPython 3.7+; empty → KeyError) =====
d = {"a": 1, "b": 2, "c": 3}
k, v = d.popitem()
assert k == "c" and v == 3
assert sorted(list(d.keys())) == ["a", "b"]
assert len(d) == 2
# the result is a real 2-tuple (subscriptable through the gradual seam)
d2 = {"x": 10, "y": 20}
item = d2.popitem()
assert item[0] == "y" and item[1] == 20
# empty dict → KeyError
d3 = {"only": 42}
ok, ov = d3.popitem()
assert ok == "only" and ov == 42
empty_caught = False
try:
    d3.popitem()
except KeyError:
    empty_caught = True
assert empty_caught
print("popitem:", k, v, "then", item[0], item[1], "empty", empty_caught)

# ===== interaction probes (cross green features) =====
# tuple.count in a comprehension over a runtime sequence
src = (1, 1, 2, 3, 3, 3)
counts = [src.count(x) for x in (1, 2, 3)]
assert counts == [2, 1, 3]
# set update then len, inside a flow that also reads membership
pool = {1, 2, 3, 4, 5}
pool.intersection_update({2, 4, 6, 8})
assert len(pool) == 2
assert (2 in pool) == True and (6 in pool) == False
# tuple.index feeding a subscript
labels = ("red", "green", "blue")
picked = ["red", "green", "blue"][labels.index("green")]
assert picked == "green"
print("interaction:", counts, len(pool), picked)

print("All container-method tests passed!")
