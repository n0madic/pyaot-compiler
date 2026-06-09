# Phase 4A — container literals: list / tuple / set / dict / bytes, including
# nesting, heterogeneous elements, and annotated-empty bootstrap (PITFALLS B4).

# Flat literals.
nums = [1, 2, 3, 4]
print(nums)
print(len(nums))

pair = (10, 20)
print(pair)

uniq = {1, 2, 3, 2, 1}
print(len(uniq))

table = {"one": 1, "two": 2, "three": 3}
print(table)
print(len(table))

raw = b"bytes!"
print(raw)
print(len(raw))

# Heterogeneous (tagged) elements.
mixed = [1, "two", 3.5, True]
print(mixed)
print(mixed[1])

# Nested literals.
grid = [[1, 2, 3], [4, 5, 6]]
print(grid)
print(grid[1])
print(grid[0][2])

records = {"a": [1, 2], "b": [3, 4]}
print(records["b"])

# Empty-container bootstrap: the annotation seeds the element type before any
# store, so these compile to tagged-element heap containers (not a heap default).
acc: list[int] = []
print(acc)
print(len(acc))

lookup: dict[str, int] = {}
print(lookup)
print(len(lookup))

# Non-annotated empty list stays correct (tagged elements).
blank = []
print(blank)
print(len(blank))

# A single-element tuple keeps its shape.
solo = (42,)
print(solo)
print(len(solo))
