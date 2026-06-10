from typing import Callable


# ── plain lambdas (no capture) ──
double = lambda x: x * 2
print(double(21))

add = lambda a, b: a + b
print(add(3, 4))


# ── lambda capturing a module-level binding ──
base = 100
shift = lambda d: base + d
print(shift(1))


# ── lambdas passed to user higher-order functions ──
def apply(f: Callable[[int], int], x: int) -> int:
    return f(x)


print(apply(lambda y: y + 1, 41))
print(apply(double, 5))


def apply_twice(f: Callable[[int], int], x: int) -> int:
    return f(f(x))


print(apply_twice(lambda n: n * 3, 2))


# ── a HOF returning a lambda (capture of both params) ──
def compose(f: Callable[[int], int], g: Callable[[int], int]) -> Callable[[int], int]:
    return lambda x: f(g(x))


inc = lambda v: v + 1
print(compose(double, inc)(10))
print(compose(inc, double)(10))


# ── lambda capturing the loop variable inside a function (late binding) ──
def lambda_rows() -> list[Callable[[int], int]]:
    rows: list[Callable[[int], int]] = []
    for k in range(3):
        rows.append(lambda x: x + k)
    return rows


rows = lambda_rows()
print(rows[0](100), rows[1](100), rows[2](100))


# ── conditional lambda selection ──
def pick(flag: bool) -> Callable[[int], int]:
    if flag:
        return lambda n: n - 1
    return lambda n: n + 1


print(pick(True)(10), pick(False)(10))


# ── lambdas over strings ──
shout = lambda s: s + "!"
print(shout("hello"))
