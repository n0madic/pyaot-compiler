# Phase 4B — general `for` over any iterable via the iterator protocol; the
# `range(...)` fast path is preserved; for-else / break / continue all work.

# Iterate every container kind.
for x in [1, 2, 3]:
    print(x)

for c in "abc":
    print(c)

for v in (10, 20, 30):
    print(v)

for k in {"a": 1, "b": 2}:
    print(k)

# A set (order is insertion-based in this runtime; keep it single-valued style).
seen = 0
for e in {7}:
    seen = seen + e
print(seen)

# bytes iterate to ints.
btotal = 0
for byte in b"ABC":
    btotal = btotal + byte
print(btotal)

# Accumulate over a list.
total = 0
for n in [5, 10, 15, 20]:
    total = total + n
print(total)

# Nested loops over a nested list.
grid = [[1, 2, 3], [4, 5, 6]]
for row in grid:
    rowsum = 0
    for cell in row:
        rowsum = rowsum + cell
    print(rowsum)

# for-else: runs the else on normal completion.
for i in [1, 2, 3]:
    print(i)
else:
    print("complete")

# break skips the else.
for j in [1, 2, 3, 4, 5]:
    if j == 3:
        break
    print(j)
else:
    print("not printed")

# continue.
for m in [1, 2, 3, 4]:
    if m % 2 == 0:
        continue
    print(m)

# range fast path still compiles and runs.
acc = 0
for r in range(1, 6):
    acc = acc + r
print(acc)

# Iterate a variable that holds a list.
data = [100, 200]
for d in data:
    print(d)

# Membership inside a loop.
allowed = [2, 4, 6]
for q in [1, 2, 3, 4]:
    if q in allowed:
        print(q)
