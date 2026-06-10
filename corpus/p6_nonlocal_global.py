from typing import Callable


# ── counter closure: `nonlocal` rebinding through the shared cell ──
def make_counter() -> Callable[[], int]:
    n = 0

    def inc() -> int:
        nonlocal n
        n = n + 1
        return n

    return inc


c1 = make_counter()
c2 = make_counter()
print(c1(), c1(), c1())
print(c2())


# ── augmented nonlocal assignment ──
def make_acc(start: int) -> Callable[[int], int]:
    total = start

    def add(x: int) -> int:
        nonlocal total
        total += x
        return total

    return add


acc = make_acc(100)
print(acc(1), acc(2), acc(3))


# ── two-level bubbling: the innermost writes the outermost's cell ──
def level0() -> int:
    v = 1

    def level1() -> int:
        def level2() -> None:
            nonlocal v
            v = v * 10

        level2()
        level2()
        return v

    return level1()


print(level0())


# ── two closures sharing one cell (reader + writer) ──
def make_pair() -> int:
    state = 5

    def read() -> int:
        return state

    def bump() -> None:
        nonlocal state
        state = state + 1

    bump()
    bump()
    return read()


print(make_pair())


# ── global counter mutated from functions ──
count = 0


def bump_global() -> None:
    global count
    count = count + 1


def show_count() -> int:
    return count


bump_global()
bump_global()
bump_global()
print(count, show_count())


# ── module global read (no declaration) inside functions ──
scale = 7


def scaled(x: int) -> int:
    return x * scale


print(scaled(6))
scale = 9
print(scaled(6))


# ── the late-binding list: every closure sees the final loop value ──
fs: list[Callable[[], int]] = []
for i in range(3):
    fs.append(lambda: i)
print([f() for f in fs] == [2, 2, 2])
