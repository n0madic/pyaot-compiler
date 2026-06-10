# ── a simple counting generator driven by a for-loop ──
def count_up(n):
    i = 0
    while i < n:
        yield i
        i = i + 1


for x in count_up(5):
    print(x)


# ── a generator transforming an input iterable ──
def squares(xs):
    for v in xs:
        yield v * v


for s in squares([1, 2, 3, 4]):
    print(s)


# ── a generator with multiple distinct yield points ──
def three():
    yield "a"
    yield "b"
    yield "c"


for t in three():
    print(t)


# ── yield from delegates to a sub-iterable ──
def chained():
    yield 0
    yield from [1, 2, 3]
    yield from squares([2, 3])
    yield 99


for v in chained():
    print(v)


# ── materializing a generator with list() ──
def evens(limit):
    i = 0
    while i < limit:
        if i % 2 == 0:
            yield i
        i = i + 1


print(list(evens(10)))


# ── next() drives a generator explicitly ──
def naturals():
    i = 1
    while True:
        yield i
        i = i + 1


nat = naturals()
print(next(nat), next(nat), next(nat))


# ── sum over a generator ──
def first_n(n):
    i = 0
    while i < n:
        yield i + 1
        i = i + 1


print(sum(first_n(5)))


# ── a generator parameterized by several args ──
def ramp(start, stop, step):
    cur = start
    while cur < stop:
        yield cur
        cur = cur + step


for v in ramp(0, 10, 3):
    print(v)
