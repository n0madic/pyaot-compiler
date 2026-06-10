# Phase 7E gate — structural match (frontend desugar to if/elif).

# ── literals + default ──
def kind(n: int) -> str:
    match n:
        case 0:
            return "zero"
        case 1:
            return "one"
        case _:
            return "many"

print(kind(0), kind(1), kind(5))

# ── string literals ──
def color(s: str) -> int:
    match s:
        case "red":
            return 1
        case "green":
            return 2
        case _:
            return 0

print(color("red"), color("green"), color("blue"))

# ── singletons ──
def truthy(b: bool) -> str:
    match b:
        case True:
            return "yes"
        case False:
            return "no"

print(truthy(True), truthy(False))

# ── capture pattern + guard ──
def bucket(n: int) -> str:
    match n:
        case 0:
            return "zero"
        case x if x < 0:
            return "neg"
        case x if x < 10:
            return "small"
        case x:
            return "big:" + str(x)

print(bucket(0), bucket(-3), bucket(7), bucket(42))

# ── or-patterns (capture-free) ──
def vowel(c: str) -> bool:
    match c:
        case "a" | "e" | "i" | "o" | "u":
            return True
        case _:
            return False

print(vowel("a"), vowel("z"))

# ── sequence patterns on a list subject ──
def shape(items: list[int]) -> str:
    match items:
        case []:
            return "empty"
        case [x]:
            return "one:" + str(x)
        case [x, y]:
            return "two:" + str(x + y)
        case [first, *rest]:
            return "many:" + str(first) + ":" + str(len(rest))

print(shape([]), shape([5]), shape([2, 3]), shape([1, 2, 3, 4]))

# ── star capture keeps the tail as a list ──
def tail_sum(items: list[int]) -> int:
    match items:
        case [_, *rest]:
            total = 0
            for v in rest:
                total = total + v
            return total
        case _:
            return -1

print(tail_sum([10, 1, 2, 3]), tail_sum([]))

# ── mapping patterns ──
def role(d: dict[str, str]) -> str:
    match d:
        case {"role": "admin"}:
            return "admin"
        case {"role": r}:
            return "role:" + r
        case _:
            return "none"

print(role({"role": "admin"}), role({"role": "user"}), role({"name": "x"}))

# ── mapping with **rest (copy semantics, original untouched) ──
def split(d: dict[str, int]) -> int:
    match d:
        case {"a": a, **rest}:
            return a + len(rest)
        case _:
            return -1

base = {"a": 10, "b": 2, "c": 3}
print(split(base), len(base), split({"b": 1}))

# ── class patterns (keyword-only) ──
class Point:
    x: int
    y: int

    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

def describe(p: Point) -> str:
    match p:
        case Point(x=0, y=0):
            return "origin"
        case Point(x=0, y=yy):
            return "on-y:" + str(yy)
        case Point(x=xx, y=0):
            return "on-x:" + str(xx)
        case Point(x=a, y=b):
            return "at:" + str(a) + "," + str(b)

print(describe(Point(0, 0)))
print(describe(Point(0, 5)))
print(describe(Point(3, 0)))
print(describe(Point(2, 4)))

# ── nested literal inside sequence ──
def pair_kind(p: list[int]) -> str:
    match p:
        case [0, y]:
            return "zero-first:" + str(y)
        case [x, 0]:
            return "zero-second:" + str(x)
        case _:
            return "other"

print(pair_kind([0, 9]), pair_kind([7, 0]), pair_kind([1, 2]))

# ── match as a statement (no return), capture leaks like CPython ──
match [1, 2, 3]:
    case [a, *bs]:
        leak = a + len(bs)
    case _:
        leak = -1
print("leak:", leak)

print("p7_match done")
