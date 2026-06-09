# Phase 4 — cross-feature integration: containers + iteration + comprehensions +
# methods + nested structures + iteration builtins, all composed together.
def process(data) -> dict:
    result = {}
    for key, values in data:
        total = sum(values)
        result[key] = total
    return result

records = [("a", [1, 2, 3]), ("b", [10, 20]), ("c", [100])]
summed = process(records)
print(sorted(summed.items()))

# Comprehension + methods + builtins.
matrix = [[i * j for j in range(1, 4)] for i in range(1, 4)]
print(matrix)
flat = []
for row in matrix:
    flat.extend(row)
print(flat)
print(sum(flat))
print(max(flat))
print(min(flat))
print(sorted(set(flat)))

# Iteration builtins composed.
names = ["alice", "bob", "carol"]
indexed = {i: n for i, n in enumerate(names)}
print(sorted(indexed.items()))
print([len(n) for n in names])
zipped = list(zip(names, [len(n) for n in names]))
print(zipped)

# Nested containers + subscript write + methods.
grid = [[0, 0], [0, 0]]
grid[0][1] = 5
grid[1][0] = 7
print(grid)
for r in grid:
    r.append(99)
print(grid)

# Membership + filters.
primes = [2, 3, 5, 7, 11]
print([p for p in range(15) if p in primes])
print(6 in primes)

# bytes + tuple.
b = bytes([72, 105])
print(b)
print(b[0])
t = (1, 2, 3)
print(t + (4, 5))
print(sum(t))
