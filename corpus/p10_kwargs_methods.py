# Phase 10 — keyword arguments on method calls: user classes (defaults,
# written-order side effects, virtual dispatch, super(), classmethod /
# staticmethod, **kwargs leftovers) plus the full list.sort matrix.


def trace(label: str, val: int) -> int:
    print("eval", label)
    return val


class Greeter:
    def __init__(self, name: str):
        self.name = name

    def greet(self, greeting: str = "Hello", punct: str = "!") -> str:
        return greeting + ", " + self.name + punct

    def add(self, a: int, b: int = 10, c: int = 100) -> int:
        return a + b * 2 + c * 3

    def collect(self, first: int, **extra: int) -> str:
        out = str(first)
        for k in sorted(extra.keys()):
            out += ";" + k + "=" + str(extra[k])
        return out

    @staticmethod
    def shout(word: str, times: int = 2) -> str:
        return word * times

    @classmethod
    def tag(cls, label: str = "g") -> str:
        return "<" + label + ">"


# Overrides declare IDENTICAL parameter names and defaults — the documented
# precondition for call-site keyword/default adaptation on virtual calls
# (differing defaults across overrides are a loud compile error).
class Base:
    def describe(self, prefix: str = "p", n: int = 1) -> str:
        return prefix + ":" + str(n)


class Derived(Base):
    def describe(self, prefix: str = "p", n: int = 1) -> str:
        return "[" + prefix + ":" + str(n) + "]"


class Child(Base):
    def describe(self, prefix: str = "p", n: int = 1) -> str:
        inner = super().describe(n=99, prefix=prefix)
        return "{" + inner + "}"


def main() -> None:
    g = Greeter("Ada")

    # ── defaults / keyword permutations ──
    print(g.greet())
    print(g.greet(punct="?"))
    print(g.greet(greeting="Hi"))
    print(g.greet("Hey", punct="."))
    print(g.greet(punct="!!", greeting="Yo"))
    print(g.add(1))
    print(g.add(1, c=2))
    print(g.add(1, b=5))
    print(g.add(c=1, a=2, b=3))

    # ── written-order side effects across reordered keywords ──
    print(g.add(trace("a", 1), c=trace("c", 2), b=trace("b", 3)))

    # ── **kwargs leftovers on a method ──
    print(g.collect(1, z=26, b=2))
    print(g.collect(first=5, y=25))

    # ── staticmethod / classmethod with keywords ──
    print(Greeter.shout("ha", times=3))
    print(Greeter.shout(word="oi"))
    print(g.shout("eh", times=1))
    print(Greeter.tag(label="x"))
    print(Greeter.tag())

    # ── virtual dispatch with keywords (override has its own defaults) ──
    objs = [Base(), Derived(), Child()]
    for o in objs:
        print(o.describe(n=5))
    for o in objs:
        print(o.describe(prefix="kw"))

    # ── super().m(kw=) ──
    print(Child().describe())

    # ── list.sort matrix ──
    xs = [3, 1, 2]
    xs.sort()
    print(xs)
    xs.sort(reverse=True)
    print(xs)
    xs.sort(reverse=False)
    print(xs)
    xs.sort(key=lambda v: -v)
    print(xs)
    xs.sort(key=None, reverse=True)
    print(xs)

    ws = ["bbb", "a", "cc"]
    ws.sort(key=len)
    print(ws)
    ws.sort(key=len, reverse=True)
    print(ws)

    # stability under key sort: equal keys keep order, reverse does not flip them
    pairs = [(2, "a"), (1, "b"), (2, "c"), (1, "d")]
    pairs.sort(key=lambda p: p[0])
    print(pairs)
    pairs = [(2, "a"), (1, "b"), (2, "c"), (1, "d")]
    pairs.sort(key=lambda p: p[0], reverse=True)
    print(pairs)

    # key closure capture + side-effect order of sort kwargs
    factor = -1
    nums = [5, 1, 4]
    nums.sort(key=lambda v: v * factor, reverse=trace("rv", 0) == 1)
    print(nums)


main()
