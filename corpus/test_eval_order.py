# Evaluation order, scoping and statement semantics (KNOWN_BUGS Phase 4).
import sys

log: list[str] = []


def f() -> int:
    log.append("f")
    return 1


def g():
    log.append("g")
    return 0


def two(a, b):
    return (a, b)


def k() -> str:
    log.append("k")
    return "a"


def v() -> int:
    log.append("v")
    return 1


class Acc:
    def __init__(self) -> None:
        self.items: list[int] = []

    def __iadd__(self, x: int) -> "Acc":
        self.items.append(x)
        return self


class CM:
    def __enter__(self):
        return self

    def __exit__(self, t, val, tb):
        log.append("exit " + str(t is not None))
        return True


class Bad:
    def __str__(self) -> str:
        raise RuntimeError("boom")


class Pt:
    def __init__(self, a: int, b: int = 2, *, c: str = "c") -> None:
        self.s = str(a) + "-" + str(b) + "-" + c


class SubPt(Pt):
    pass


def classify(x) -> str:
    match x:
        case [a, b]:
            return "seq2"
        case _:
            return "other"


def test_left_to_right() -> None:
    del log[:]
    assert f() + (g() or 2) == 3
    assert log == ["f", "g"]
    x = 1
    assert x + (x := 5) == 6
    y = 1
    assert two(y, (y := 7)) == (1, 7)
    del log[:]
    d: dict[str, int] = {}
    d[k()] = v()
    assert log == ["v", "k"]


def test_inplace() -> None:
    a = [1]
    b = a
    a += [2]
    assert b == [1, 2]
    m = [1, 2]
    n = m
    m *= 2
    assert n == [1, 2, 1, 2]
    t = (1,)
    t2 = t
    t += (2,)
    assert t == (1, 2) and t2 == (1,)
    acc = Acc()
    alias = acc
    acc += 3
    assert alias.items == [3] and acc is alias
    s = "ab"
    s += "c"
    assert s == "abc"


def test_unbound() -> None:
    caught = ""
    try:
        print(z)
    except UnboundLocalError as e:
        caught = str(e)
    assert caught == "cannot access local variable 'z' where it is not associated with a value"
    if len(caught) > 1000:
        z = 1
    e = 5
    try:
        raise ValueError("v")
    except ValueError as e:
        pass
    unbound = False
    try:
        print(e)
    except UnboundLocalError:
        unbound = True
    assert unbound
    w = 5
    try:
        pass
    except ValueError as w:
        pass
    assert w == 5


def test_closures_and_genexprs() -> None:
    kk = 3
    assert sum(kk * i for i in range(3)) == 9
    gen = (kk + i for i in range(2))
    assert next(gen) == 3 and next(gen) == 4
    fs = [lambda: i for i in range(3)]
    assert [h() for h in fs] == [2, 2, 2]


def test_patterns_and_unpack() -> None:
    assert classify("ab") == "other"
    assert classify({"a": 1, "b": 2}) == "other"
    assert classify([1, 2]) == "seq2"
    assert classify((1, 2)) == "seq2"
    first, *rest = [1, *[2, 3]]
    assert first == 1 and rest == [2, 3]
    p, q = (*[1], 2)
    assert (p, q) == (1, 2)


def test_with_and_print() -> None:
    del log[:]
    with CM():
        raise ValueError("x")
    assert log == ["exit True"]
    try:
        print(Bad(), file=sys.stderr)
    except RuntimeError:
        pass


def test_keyword_construction() -> None:
    assert Pt(b=5, a=1).s == "1-5-c"
    assert Pt(1, c="z").s == "1-2-z"
    assert SubPt(a=7).s == "7-2-c"


test_left_to_right()
test_keyword_construction()
test_inplace()
test_unbound()
test_closures_and_genexprs()
test_patterns_and_unpack()
test_with_and_print()
print("test_eval_order passed!")
