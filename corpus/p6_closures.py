from typing import Callable


# ── nested def reading an enclosing local (basic capture) ──
def twice(x: int) -> int:
    def inner(y: int) -> int:
        return y * 2
    return inner(x)


print(twice(21))


# ── returned closures: independent cells per activation ──
def make_adder(n: int) -> Callable[[int], int]:
    def add(x: int) -> int:
        return x + n
    return add


add5 = make_adder(5)
add7 = make_adder(7)
print(add5(1), add7(1))
print(add5(add7(0)))


# ── outer rebinding is visible through the shared cell (late binding) ──
def make_getter() -> int:
    x = 10

    def get() -> int:
        return x

    first = get()
    x = 20
    return first * 1000 + get()


print(make_getter())


# ── recursion via self-capture ──
def make_fact() -> Callable[[int], int]:
    def fact(n: int) -> int:
        if n <= 1:
            return 1
        return n * fact(n - 1)
    return fact


fact = make_fact()
print(fact(6))


# ── transitive (two-level) capture bubbling ──
def outer(a: int) -> int:
    def mid(b: int) -> int:
        def inner(c: int) -> int:
            return a + b + c
        return inner(b * 10)
    return mid(a * 10)


print(outer(3))


# ── stored closures: a list of function values, called by index ──
fs: list[Callable[[], int]] = []


def make_const(k: int) -> Callable[[], int]:
    def const() -> int:
        return k
    return const


for i in range(3):
    fs.append(make_const(i * 11))
print(fs[0](), fs[1](), fs[2]())


# ── classic late-binding pitfall: all loop closures see the FINAL i ──
def make_loop_closures() -> list[Callable[[], int]]:
    out: list[Callable[[], int]] = []
    for i in range(3):
        def f() -> int:
            return i
        out.append(f)
    return out


loop_fs = make_loop_closures()
print(loop_fs[0](), loop_fs[1](), loop_fs[2]())


# ── multiple captures of mixed types ──
def describe(name: str, base: int) -> Callable[[int], str]:
    def fmt(extra: int) -> str:
        return name + ": " + str(base + extra)
    return fmt


f = describe("total", 100)
print(f(11))
print(f(22))


# ── a top-level function used as a value (thunk) ──
def square(x: int) -> int:
    return x * x


def apply(g: Callable[[int], int], x: int) -> int:
    return g(x)


print(apply(square, 4))
print(apply(make_adder(3), 4))
