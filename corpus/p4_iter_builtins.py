# Phase 4C — iteration builtins: enumerate / zip / sorted / reversed / min / max /
# sum, and the list / dict / set / tuple / bytes constructors over iterables.
# Exercised over int AND str elements (PITFALLS B13: min/max compare by value).

# enumerate.
for i, name in enumerate(["alpha", "beta", "gamma"]):
    print(i)
    print(name)

# enumerate with a start.
for idx, ch in enumerate("xyz", 1):
    print(idx)
    print(ch)

# zip.
for a, b in zip([1, 2, 3], ["one", "two", "three"]):
    print(a)
    print(b)

# sorted over an int list and a str list.
print(sorted([5, 2, 8, 1, 9]))
print(sorted(["banana", "apple", "cherry"]))
print(sorted({3, 1, 2}))

# reversed.
print(list(reversed([1, 2, 3, 4])))
for c in reversed("abc"):
    print(c)

# min / max over int lists.
print(min([7, 3, 9, 1]))
print(max([7, 3, 9, 1]))

# min / max over str lists (value comparison, not pointer order).
print(min(["pear", "fig", "kiwi"]))
print(max(["pear", "fig", "kiwi"]))

# min / max with multiple positional args.
print(min(4, 9, 2, 7))
print(max(4, 9, 2, 7))

# sum over ints and floats.
print(sum([10, 20, 30]))
print(sum([1.5, 2.5, 3.0]))
print(sum(range(5)))
print(sum([1, 2, 3], 100))

# Constructors over iterables.
print(list(range(4)))
print(list("abc"))
print(tuple([9, 8, 7]))
# Set iteration order is implementation-defined, so compare via sorted/len.
print(sorted(set([1, 1, 2, 2, 3])))
print(len(set("mississippi")))
print(dict([("x", 1), ("y", 2)]))
print(bytes([72, 73]))

# Builtins composed.
print(sorted([len(w) for w in ["aa", "b", "cccc", "ddd"]]))
print(max([x * x for x in range(5)]))
print(sum(sorted([3, 1, 2])))
pairs = list(zip([1, 2], [3, 4]))
print(pairs)
