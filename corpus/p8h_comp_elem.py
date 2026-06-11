# Phase 8H D1 — container element types inferred from pushes.
# Comprehension results and append/add/insert/extend-built containers carry
# precise element types, so downstream numeric code stays specialized.

# list comprehension of floats: elements usable as floats directly
xs = [i * 0.5 for i in range(5)]
total = 0.0
for x in xs:
    total = total + x
print(total)
print(xs[2] * 2.0)

# list comprehension of ints
sq = [i * i for i in range(6)]
print(sq[3] + 10)

# nested comprehension
grid = [[i * j for j in range(3)] for i in range(3)]
print(grid[2][2] + 1)

# set comprehension
evens = {i * 2 for i in range(4)}
print(len(evens))
print(6 in evens)

# dict comprehension
d = {i: i * 1.5 for i in range(4)}
print(d[3] + 0.5)

# append-built list
acc = []
for i in range(4):
    acc.append(i * 0.25)
print(acc[3] * 4.0)

# extend-built list
more = []
more.extend([1, 2, 3])
more.extend([4, 5])
print(more[4] * 3)

# insert
ins = []
ins.insert(0, 1.5)
ins.insert(0, 2.5)
print(ins[0] + ins[1])

# set add
s = set()
s.add(10)
s.add(20)
print(20 in s)

# setitem element constraint
fixed = [0.0, 0.0, 0.0]
fixed[1] = 3.25
print(fixed[1] * 2.0)

# string elements
words = [w + "!" for w in ["a", "b"]]
print(words[0] + words[1])

print("p8h comp elem passed!")
