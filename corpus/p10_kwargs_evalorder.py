# Phase 10 — argument evaluation order with keyword arguments (PLAN §1 trap).
# CPython evaluates call arguments left-to-right AS WRITTEN; reordering keyword
# values into positional slots permutes side effects. Every trace() below must
# print in written order, not parameter order.


def trace(label: str, val: int) -> int:
    print("eval", label)
    return val


def f(a: int, b: int, c: int) -> None:
    print("f", a, b, c)


def g(a: int, b: int = 10, c: int = 20) -> None:
    print("g", a, b, c)


def h(a: int, b: int = 1, *args: int, **kw: int) -> None:
    out = ""
    for k in sorted(kw.keys()):
        out += k + "=" + str(kw[k]) + ";"
    print("h", a, b, len(args), out)


def main() -> None:
    # Keywords written out of parameter order: must evaluate a, c, b.
    f(trace("f.a", 1), c=trace("f.c", 3), b=trace("f.b", 2))
    # All keywords, fully reversed.
    f(c=trace("f2.c", 30), b=trace("f2.b", 20), a=trace("f2.a", 10))
    # Defaults skipped in the middle: written order is a then c.
    g(trace("g.a", 1), c=trace("g.c", 99))
    # Keyword for an earlier param evaluated after a later one.
    g(b=trace("g2.b", 5), a=trace("g2.a", 4))
    # **kwargs leftovers interleaved with a fixed-param keyword.
    h(trace("h.a", 1), z=trace("h.z", 26), b=trace("h.b", 2), y=trace("h.y", 25))


main()
