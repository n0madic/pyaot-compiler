# Phase 4A — indexed read / write: list, dict, tuple, str, bytes; negative
# indices; subscript assignment.

# List read + write.
xs = [10, 20, 30, 40]
print(xs[0])
print(xs[3])
print(xs[-1])
print(xs[-2])
xs[0] = 99
xs[-1] = 77
print(xs)

# Dict read + write (string keys, int keys).
d = {"a": 1, "b": 2}
print(d["a"])
d["c"] = 3
d["a"] = 100
print(d["a"])
print(d["c"])
print(len(d))

counts = {1: "one", 2: "two"}
print(counts[2])
counts[3] = "three"
print(counts[3])

# Tuple read (immutable).
t = (5, 6, 7)
print(t[0])
print(t[-1])
print(t[1])

# String indexing (codepoint-aware, negatives).
s = "python"
print(s[0])
print(s[-1])
print(s[2])

# Bytes indexing (returns int values).
b = b"ABC"
print(b[0])
print(b[1])
print(b[-1])

# Indexing with a computed (variable) index.
i = 2
print(xs[i])

# Nested subscript write.
grid = [[1, 2], [3, 4]]
grid[0][1] = 20
print(grid)
print(grid[0][1])
