x = 5
print(x)
x += 3
print(x)
x = y = 10
print(x, y)
a: int = 7
print(a)
b: float = 2.5
print(b)

if x > 5:
    print("big")
else:
    print("small")

if x < 0:
    print("neg")
elif x == 13:
    print("thirteen")
else:
    print("other")

i = 0
while i < 5:
    print(i)
    i += 1

total = 0
for n in range(1, 6):
    total += n
print(total)

for j in range(10):
    if j == 3:
        break
    print(j)

for k in range(5):
    if k % 2 == 0:
        continue
    print(k)

for m in range(3):
    print(m)
else:
    print("for-else done")

w = 0
while w < 3:
    w += 1
else:
    print("while-else done")

assert x == 10
assert 1 < 2
print("asserts passed")

for d in range(10, 0, -2):
    print(d)

s = 0
for q in range(100):
    s += q
print(s)
