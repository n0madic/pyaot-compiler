# Phase 10 — keyword arguments on builtins: sorted(key=, reverse=),
# enumerate(start=), dict(a=1) / dict(pos, a=1). Includes the mandatory
# kwargs × closure interaction probe and sort-stability checks.


def neg(x: int) -> int:
    return -x


def trace(label: str, val: int) -> int:
    print("eval", label)
    return val


def main() -> None:
    xs = [3, 1, 2]

    # ── sorted: reverse only (both truthiness spellings) ──
    print(sorted(xs, reverse=True))
    print(sorted(xs, reverse=False))
    print(sorted(xs, reverse=1))
    print(sorted(xs, reverse=0))

    # ── sorted: key= lambda / named fn / builtins ──
    print(sorted(xs, key=lambda v: -v))
    print(sorted(xs, key=neg))
    print(sorted([-5, 2, -1, 4], key=abs))
    print(sorted(["bbb", "a", "cc"], key=len))
    print(sorted([10, 2, 33], key=str))

    # ── key + reverse together, both keyword orders ──
    print(sorted(xs, key=neg, reverse=True))
    print(sorted(xs, reverse=True, key=neg))

    # ── key=None literal behaves like no key ──
    print(sorted(xs, key=None))
    print(sorted(xs, key=None, reverse=True))

    # ── kwargs × closure: the key captures an enclosing variable ──
    pivot = 2

    def dist(v: int) -> int:
        return abs(v - pivot)

    print(sorted([5, 1, 2, 4], key=dist))
    print(sorted([5, 1, 2, 4], key=lambda v: abs(v - pivot), reverse=True))

    # ── stability: equal keys keep written order; reverse must NOT flip them ──
    pairs = [(2, "a"), (1, "b"), (2, "c"), (1, "d")]
    print(sorted(pairs, key=lambda p: p[0]))
    print(sorted(pairs, key=lambda p: p[0], reverse=True))
    words = ["bb", "aa", "cc", "dd"]
    print(sorted(words, key=len))
    print(sorted(words, key=len, reverse=True))

    # ── sorted over non-list iterables with keywords ──
    print(sorted((3, 1, 2), reverse=True))
    print(sorted({"b": 1, "a": 2}, reverse=True))
    print(sorted("cab", key=str, reverse=True))

    # ── the input is never mutated ──
    print(xs)

    # ── enumerate: positional and keyword start ──
    for i, v in enumerate(["x", "y"], 5):
        print(i, v)
    for i, v in enumerate(["x", "y"], start=7):
        print(i, v)
    print(list(enumerate("ab", start=1)))

    # ── dict: pure-kwargs, mixed, written-order side effects ──
    d1 = dict(a=1, b=2, c=3)
    print(d1)
    d2 = dict(d1, b=20, z=26)
    print(d2)
    print(d1)
    d3 = dict(b=trace("d3.b", 2), a=trace("d3.a", 1))
    print(d3)
    d4 = dict([("k", 0)], v=trace("d4.v", 9))
    print(d4)

    # ── sorted kwargs values evaluate in written order ──
    print(sorted([2, 1], reverse=trace("rev", 0) == 1))


main()
