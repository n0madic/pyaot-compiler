# Phase 4C — eager list / dict / set comprehensions: nested `for` clauses, `if`
# filters, heap-element comps, and comprehensions feeding further computation.

# Simple list comprehension.
squares = [x * x for x in range(7)]
print(squares)

# Filtered.
evens = [x for x in range(15) if x % 2 == 0]
print(evens)

# Multiple filters.
both = [x for x in range(30) if x % 2 == 0 if x % 3 == 0]
print(both)

# Mapped from a list.
doubled = [n * 2 for n in [10, 20, 30]]
print(doubled)

# Nested `for` clauses (cartesian product).
grid = [(i, j) for i in range(3) for j in range(2)]
print(grid)

# Comprehension over a string.
chars = [c for c in "hello"]
print(chars)

# Heap-element comprehension (a list of lists).
rows = [[y for y in range(x)] for x in range(5)]
print(rows)

# Comprehension with a function call in the element.
words = ["a", "bb", "ccc", "dddd"]
lengths = [len(w) for w in words]
print(lengths)

# Set comprehension (dedups).
mods = {x % 4 for x in range(20)}
print(len(mods))

# Dict comprehension.
table = {x: x * x for x in range(6)}
print(table)
print(table[4])

# Dict comprehension with a filter.
big = {k: k * 10 for k in range(10) if k > 5}
print(big)

# Comprehension result fed into a reduce.
total = sum([x for x in range(10) if x % 2 == 1])
print(total)

# Comprehension over another comprehension's result.
base = [x + 1 for x in range(5)]
scaled = [b * 3 for b in base]
print(scaled)

# Membership against a comprehension result.
allowed = [n * n for n in range(5)]
print(9 in allowed)
print(10 in allowed)
