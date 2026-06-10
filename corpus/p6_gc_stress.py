from typing import Callable


# Soak the collector with closures (env tuples), cells (shared mutable state),
# and generators (tagged slot arrays) all live across many allocations.


# ── many independent counter closures, each holding a live cell ──
def make_counter(start: int) -> Callable[[], int]:
    n = start

    def step() -> int:
        nonlocal n
        n = n + 1
        return n

    return step


counters: list[Callable[[], int]] = []
for i in range(200):
    counters.append(make_counter(i * 1000))

# Drive them repeatedly, allocating lists each round to force collections.
checksum = 0
for _ in range(50):
    junk: list[int] = []
    for c in counters:
        junk.append(c())
    checksum = checksum + junk[0] + junk[len(junk) - 1]
print(checksum)


# ── generators holding live locals across many resumes ──
def windows(data: list, size: int):
    i = 0
    while i + size <= len(data):
        acc = 0
        j = 0
        while j < size:
            acc = acc + data[i + j]
            j = j + 1
        yield acc
        i = i + 1


data: list[int] = []
for i in range(500):
    data.append(i)

total = 0
for w in windows(data, 4):
    total = total + w
    # allocate garbage every iteration
    tmp: list[int] = [w, w, w]
    total = total + tmp[0]
print(total)


# ── closures capturing a shared cell, mutated through a generator drive ──
def make_pair() -> Callable[[int], int]:
    state = 0

    def bump(d: int) -> int:
        nonlocal state
        state = state + d
        return state

    return bump


bumpers: list[Callable[[int], int]] = []
for i in range(100):
    bumpers.append(make_pair())

acc = 0
for r in range(30):
    box: list[int] = []
    for b in bumpers:
        box.append(b(r))
    acc = acc + box[0]
print(acc)


# ── closures feeding a generator: build many counters, drive them, then a
#    generator streams the pre-computed results (closures + cells + a generator
#    soak, with the closure calls outside the generator's tagged slots) ──
def value_stream(values: list):
    for v in values:
        yield v * 2
        yield v + 1


def build_values(count: int) -> list:
    fns: list[Callable[[], int]] = []
    for k in range(count):
        fns.append(make_counter(k))
    out: list[int] = []
    for f in fns:
        out.append(f())
    return out


stream_total = 0
for v in value_stream(build_values(300)):
    stream_total = stream_total + v
    spill: list[int] = [v, v + 1]
    stream_total = stream_total + spill[0]
print(stream_total)
