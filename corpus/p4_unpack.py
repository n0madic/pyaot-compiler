# Phase 4B — fixed-tuple unpacking in assignment and for-loop targets.

# Literal-sequence unpacking.
a, b = 1, 2
print(a)
print(b)

# Swap (all RHS staged before binding).
a, b = b, a
print(a)
print(b)

# Unpack from a tuple value.
point = (3, 4)
x, y = point
print(x)
print(y)

# Unpack from a list value.
first, second, third = [10, 20, 30]
print(first)
print(second)
print(third)

# Unpack in a for-loop over a list of pairs.
pairs = [(1, "one"), (2, "two"), (3, "three")]
for num, word in pairs:
    print(num)
    print(word)

# Unpack three-element rows.
rows = [(1, 2, 3), (4, 5, 6)]
for p, q, r in rows:
    print(p + q + r)

# Nested data: dict built then iterated as items would need .items() (4C); here
# iterate the keys and index for the value.
table = {"a": 1, "b": 2}
for key in table:
    print(key)
    print(table[key])

# Unpacking feeds further computation.
lo, hi = 2, 8
print(hi - lo)
