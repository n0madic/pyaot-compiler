# Phase 4D — focused container methods on statically-known receivers.
# (Set iteration order is implementation-defined, so set results are compared via
# sorted()/len(), never printed directly.)

# ── list methods ──
xs = []
for i in range(5):
    xs.append(i * i)
print(xs)

print(xs.pop())
print(xs)

xs.insert(0, 99)
print(xs)

xs.extend([7, 8])
print(xs)

print(xs.index(99))
print(xs.count(4))

dup = [1, 2, 2, 3, 2]
print(dup.count(2))

ys = xs.copy()
ys.reverse()
print(ys)
print(xs)  # copy did not mutate the original

zs = [5, 3, 8, 1, 9, 2]
zs.sort()
print(zs)

popped = zs.pop(0)
print(popped)
print(zs)

zs.clear()
print(zs)
print(len(zs))

# ── dict methods ──
d = {"a": 1, "b": 2, "c": 3}
print(d.get("b"))
print(d.get("missing"))
print(d.get("missing", -1))
print(sorted(d.keys()))
print(sorted(d.values()))
print(sorted(d.items()))

print(d.pop("a"))
print(sorted(d.keys()))

d.setdefault("z", 99)
print(d.get("z"))
d.setdefault("b", 999)  # already present; unchanged
print(d.get("b"))

d.update({"x": 10, "y": 20})
print(d.get("x"))
print(d.get("y"))

dcopy = d.copy()
dcopy.clear()
print(len(dcopy))
print(len(d))

# ── set methods ──
s = set()
s.add(1)
s.add(2)
s.add(2)
s.add(3)
print(len(s))
s.discard(1)
s.discard(99)  # absent — no error
s.remove(2)
print(len(s))

a = {1, 2, 3, 4}
b = {3, 4, 5, 6}
print(sorted(a.union(b)))
print(sorted(a.intersection(b)))
print(sorted(a.difference(b)))

c = a.copy()
c.add(100)
print(len(a))
print(len(c))

# ── methods feeding loops / comprehensions ──
acc = []
for k in sorted(d.keys()):
    acc.append(d.get(k))
print(acc)

bucket = {}
for w in ["apple", "banana", "apple", "cherry", "banana", "apple"]:
    bucket[w] = bucket.get(w, 0) + 1
print(sorted(bucket.items()))
