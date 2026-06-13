# Backlog §1 — `*seq` spread into a non-`*args` callee.
#
# Spreading an iterable's elements as positional arguments: into a fixed-arity
# callee (`f(*xs)`), mixed with plain positionals (`f(1, *xs, 4)`), multiple
# spreads (`f(*a, *b, c)`), through defaults, into a `*args` callee, and into a
# decorated function. A list/tuple LITERAL spread flattens at compile time; a
# runtime sequence (variable / call result / comprehension / deque) materializes
# an argv list, length-checks it, and binds each parameter by position.
#
# Interaction probes (Principle: cross each new feature with a green one):
# comprehension-as-source, spread inside a loop, and left-to-right call-argument
# evaluation order combined with side-effecting plain args and spread sources.

from typing import Callable


# ===== Fixed-arity, no defaults =====
def f3(a: int, b: int, c: int) -> int:
    return a + b + c


# Compile-time literal spread (list and tuple).
print(f3(*[1, 2, 3]))          # 6
print(f3(*(7, 8, 9)))          # 24

# Runtime variable spread (list and tuple).
nums_list: list[int] = [10, 20, 30]
nums_tuple: tuple[int, int, int] = (4, 5, 6)
print(f3(*nums_list))          # 60
print(f3(*nums_tuple))         # 15


# Function-result spread.
def make3() -> tuple[int, int, int]:
    return (100, 200, 300)


print(f3(*make3()))            # 600


# ===== Mixed plain + spread =====
def f4(a: int, b: int, c: int, d: int) -> int:
    return a * 1000 + b * 100 + c * 10 + d


print(f4(1, *[2, 3], 4))       # 1234 (literal)
mid: list[int] = [2, 3]
print(f4(1, *mid, 4))          # 1234 (runtime)

first: int = 9
rest_pair: tuple[int, int] = (8, 7)
print(f3(first, *rest_pair))   # 24


# ===== Multiple spreads =====
print(f4(*[1, 2], *[3, 4]))    # 1234 (literal)
a2: tuple[int, int] = (1, 2)
b2: tuple[int, int] = (3, 4)


def f5(a: int, b: int, c: int, d: int, e: int) -> int:
    return a + b + c + d + e


print(f5(*a2, *b2, 5))         # 15


# ===== Empty spread =====
empty: list[int] = []
print(f3(1, *empty, 2, 3))     # 6 (runtime empty contributes nothing)
print(f3(*[], 1, 2, 3))        # 6 (literal empty)


# ===== str parameters (gradual heap — Dyn admitted directly) =====
def cat3(a: str, b: str, c: str) -> str:
    return a + b + c


print(cat3(*["x", "y", "z"]))  # xyz
words: list[str] = ["Hello", " ", "World"]
print(cat3(*words))            # Hello World


# ===== float parameters (Raw(F64) — laundered through a typed slot) =====
def addf(a: float, b: float) -> float:
    return a + b


floats: list[float] = [1.5, 2.5]
print(addf(*floats))           # 4.0
print(addf(*[10.25, 0.75]))    # 11.0


# ===== bool parameters (Raw(I8) — laundered through a typed slot) =====
def andb(a: bool, b: bool) -> bool:
    return a and b


bools: list[bool] = [True, False]
print(andb(*bools))            # False
flags: tuple[bool, bool] = (True, True)
print(andb(*flags))            # True


# ===== Defaults filled from a short spread =====
def with_def(a: int, b: int, c: int = 100) -> int:
    return a + b + c


print(with_def(*[1, 2]))       # 103 (c defaults)
print(with_def(*[1, 2, 3]))    # 6   (c supplied)
two: list[int] = [1, 2]
three: list[int] = [1, 2, 3]
print(with_def(*two))          # 103
print(with_def(*three))        # 6


def multi_def(a: int, b: int = 10, c: int = 20) -> int:
    return a + b + c


print(multi_def(*[5]))         # 35
print(multi_def(*[5, 15]))     # 40
print(multi_def(*[5, 15, 25])) # 45


def greet(name: str, greeting: str = "Hello", punct: str = "!") -> str:
    return greeting + " " + name + punct


only_name: tuple[str] = ("World",)
print(greet(*only_name))       # Hello World!
print(greet(*["Sun", "Hi"]))   # Hi Sun!


# ===== *args callee =====
def va(a: int, *rest: int) -> int:
    total: int = a
    for x in rest:
        total += x
    return total


print(va(*[1, 2, 3]))          # 6  (a=1, rest=(2,3))
print(va(10, *[4, 5]))         # 19 (a=10, rest=(4,5))
print(va(*[42]))               # 42 (a=42, rest=())


def va2(a: int, b: int, *rest: int) -> int:
    total: int = a + b
    for x in rest:
        total += x
    return total


print(va2(*[10, 20, 30, 40]))  # 100
print(va2(*[5, 15]))           # 20 (rest empty)
lead: list[int] = [1, 2]
print(va2(1, 2, *[3, 4, 5]))   # 18


# ===== Interaction: comprehension as the spread source =====
print(f3(*[x * 2 for x in range(3)]))  # 6 (0, 2, 4)


# ===== Interaction: spread inside a loop =====
def f1(x: int) -> int:
    return x * x


loop_total: int = 0
for i in range(4):
    one: list[int] = [i]
    loop_total += f1(*one)
print(loop_total)              # 0 + 1 + 4 + 9 = 14


# ===== Interaction: left-to-right evaluation order =====
order_log: list[int] = []


def rec(n: int) -> int:
    order_log.append(n)
    return n


def src() -> list[int]:
    order_log.append(99)
    return [7, 8]


def take4(a: int, b: int, c: int, d: int) -> int:
    return a * 1000 + b * 100 + c * 10 + d


r: int = take4(rec(1), *src(), rec(2))
print(order_log)               # [1, 99, 2]
print(r)                       # 1782


# ===== Decorated function (its slot is a (*args, **kwargs) wrapper) =====
def logged(func: Callable[..., int]) -> Callable[..., int]:
    def wrapper(*args: int, **kwargs: int) -> int:
        return func(*args, **kwargs)
    return wrapper


@logged
def add2(x: int, y: int) -> int:
    return x + y


pair: list[int] = [10, 20]
print(add2(*pair))             # 30 (runtime spread)
print(add2(*[3, 4]))           # 7  (literal spread)
print(add2(1, *[6]))           # 7  (mixed)

print("All spread tests passed!")
