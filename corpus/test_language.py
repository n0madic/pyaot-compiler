"""Consolidated language-feature parity tests (real-script gaps)."""

import os

# ── Module-level classes (pyaot cannot nest classes in functions) ──


# from p8e_language: tuple () parameter default + __slots__
class _Node:
    __slots__ = ('data', 'kids')

    def __init__(self, data, kids=()):
        self.data = data
        self.kids = kids

    def child_count(self):
        return len(self.kids)


# from p8e_language: return-type inference — unannotated dunder / method chains
class _Value:
    def __init__(self, data):
        self.data = data

    def __add__(self, other):
        o = other if isinstance(other, _Value) else _Value(other)
        return _Value(self.data + o.data)

    def __sub__(self, other):
        o = other if isinstance(other, _Value) else _Value(other)
        return _Value(self.data - o.data)

    def __mul__(self, other):
        o = other if isinstance(other, _Value) else _Value(other)
        return _Value(self.data * o.data)

    def relu(self):
        return _Value(self.data if self.data > 0.0 else 0.0)

    def squared(self):
        return self * self


def _make(x):
    return _Value(x)


# from p8e_language: str() / repr() honour user __str__ / __repr__
class _Labelled:
    def __init__(self, n):
        self.n = n

    def __repr__(self):
        return "repr<" + str(self.n) + ">"

    def __str__(self):
        return "str<" + str(self.n) + ">"


class _ReprOnly:
    def __init__(self, n):
        self.n = n

    def __repr__(self):
        return "RO(" + str(self.n) + ")"


# ── Module-level lambda-with-defaults (specifically supported by pyaot) ──
# from p8h_lang #9
scale = lambda x, k=2: x * k
add3 = lambda a, b=10, c=100: a + b + c
double = lambda v: v * 2


def _p8e_language():
    # ── f-string format specs ──
    step = 42
    total = 1000
    loss = 3.14159265
    acc = 0.8765
    assert f"step {step+1:4d} / {total:4d} | loss {loss:.4f}" == \
        "step   43 / 1000 | loss 3.1416"
    assert f"acc={acc*100:.1f}% [{step:05d}] {'ok':>6} {loss:8.3f}" == \
        "acc=87.6% [00042]     ok    3.142"

    # ── slicing: list / str / tuple, negative, stepped, open-ended ──
    xs = [0, 1, 2, 3, 4, 5, 6, 7]
    assert xs[2:5] == [2, 3, 4]
    assert xs[:3] == [0, 1, 2]
    assert xs[5:] == [5, 6, 7]
    assert xs[-3:] == [5, 6, 7]
    assert xs[:-2] == [0, 1, 2, 3, 4, 5]
    assert xs[1:7:2] == [1, 3, 5]
    assert xs[::2] == [0, 2, 4, 6]
    assert xs[::-1] == [7, 6, 5, 4, 3, 2, 1, 0]
    hs, head_dim = 2, 3
    assert xs[hs:hs + head_dim] == [2, 3, 4]
    s = "abcdefgh"
    assert s[2:5] == "cde"
    assert s[::-1] == "hgfedcba"
    assert s[:4] == "abcd"
    t = (10, 20, 30, 40, 50)
    assert t[1:4] == (20, 30, 40)
    rows = [[1, 2, 3, 4], [5, 6, 7, 8]]
    assert [r[1:3] for r in rows] == [[2, 3], [6, 7]]

    # ── str.join / list.index / str.strip ──
    words = ["emma", "olivia", "ava"]
    assert "".join(words) == "emmaoliviaava"
    assert "-".join(words) == "emma-olivia-ava"
    assert ", ".join(words) == "emma, olivia, ava"
    chars = ['a', 'b', 'c', 'd']
    assert chars.index('c') == 2
    line = "  hello world  \n"
    assert "[" + line.strip() + "]" == "[hello world]"
    assert "".join([str(n) for n in [1, 2, 3]]) == "123"

    # ── tuple () parameter default + __slots__ ──
    a = _Node(10)
    b = _Node(20, (a,))
    assert a.data == 10
    assert a.child_count() == 0
    assert b.data == 20
    assert b.child_count() == 1

    # ── return-type inference: unannotated dunder / method chains stay typed ──
    logits: list[_Value] = [_Value(-1.0), _Value(2.0), _Value(3.0)]
    max_val = 2.0
    relu_data = []
    for v in logits:
        rv = (v - max_val).relu()         # method on a dunder result (inferred Value)
        relu_data.append(f"{rv.data:.4f}")
    assert relu_data == ["0.0000", "0.0000", "1.0000"]
    assert _make(5.0).squared().data == 25.0  # method chain across an unannotated func

    acc2: _Value = logits[0]
    for i in range(1, 3):
        acc2 = acc2 + logits[i]
    assert acc2.data == 4.0

    # ── comprehension loop variables do not leak into the enclosing scope ──
    i = 99
    squares = [i * i for i in range(4)]
    assert squares == [0, 1, 4, 9]
    assert i == 99                        # i stays 99 (comprehension has its own scope)
    pairs = [(j, k) for j in range(2) for k in range(2)]
    j = "outer-j"
    both = [j + str(k) for j, k in [("a", 1), ("b", 2)]]
    assert pairs == [(0, 0), (0, 1), (1, 0), (1, 1)]
    assert both == ['a1', 'b2']
    assert j == "outer-j"                 # j stays "outer-j"

    # ── str() / repr() honour user __str__ / __repr__ (and __repr__ fallback) ──
    lab = _Labelled(1)
    assert str(lab) == "str<1>"
    assert repr(lab) == "repr<1>"
    assert f"{lab}" == "str<1>"
    ro = _ReprOnly(2)
    assert str(ro) == "RO(2)"             # str falls back to __repr__
    assert repr(ro) == "RO(2)"
    assert f"{ro!r}" == "RO(2)"
    assert f"{lab} | {ro!r}" == "str<1> | RO(2)"


def _p8h_lang():
    # ── #9: module-level lambda with defaults ──
    assert scale(10) == 20
    assert scale(10, 3) == 30
    assert scale(10, k=5) == 50

    assert add3(1) == 111
    assert add3(1, 2) == 103
    assert add3(1, 2, 3) == 6
    assert add3(1, c=7) == 18

    # A no-defaults lambda keeps the closure path.
    assert double(21) == 42

    # ── #15b: iterate a File stored in a variable ──
    path = "/tmp/pyaot_lang_test.txt"
    with open(path, "w") as w:
        w.write("alpha\nbeta\ngamma\n")
    f = open(path)
    read_lines = []
    for line in f:
        read_lines.append(line.strip())
    f.close()
    assert read_lines == ["alpha", "beta", "gamma"]

    # The syntactic form still works through the same lowering path.
    line_lengths = []
    for line in open(path):
        line_lengths.append(len(line))
    assert line_lengths == [6, 5, 6]
    os.remove(path)

    # ── #14a: os.environ writes are visible to subsequent reads ──
    os.environ["P8H_TEST_VAR"] = "p8h-value"
    assert os.getenv("P8H_TEST_VAR") == "p8h-value"
    assert os.environ["P8H_TEST_VAR"] == "p8h-value"
    assert os.environ.get("P8H_TEST_VAR") == "p8h-value"
    v = os.getenv("P8H_MISSING_VAR")
    assert v is None


def _p8h_comp_elem():
    # list comprehension of floats: elements usable as floats directly
    xs = [i * 0.5 for i in range(5)]
    total = 0.0
    for x in xs:
        total = total + x
    assert total == 5.0
    assert xs[2] * 2.0 == 2.0

    # list comprehension of ints
    sq = [i * i for i in range(6)]
    assert sq[3] + 10 == 19

    # nested comprehension
    grid = [[i * j for j in range(3)] for i in range(3)]
    assert grid[2][2] + 1 == 5

    # set comprehension
    evens = {i * 2 for i in range(4)}
    assert len(evens) == 4
    assert (6 in evens) is True

    # dict comprehension
    d = {i: i * 1.5 for i in range(4)}
    assert d[3] + 0.5 == 5.0

    # append-built list
    acc = []
    for i in range(4):
        acc.append(i * 0.25)
    assert acc[3] * 4.0 == 3.0

    # extend-built list
    more = []
    more.extend([1, 2, 3])
    more.extend([4, 5])
    assert more[4] * 3 == 15

    # insert
    ins = []
    ins.insert(0, 1.5)
    ins.insert(0, 2.5)
    assert ins[0] + ins[1] == 4.0

    # set add
    s = set()
    s.add(10)
    s.add(20)
    assert (20 in s) is True

    # setitem element constraint
    fixed = [0.0, 0.0, 0.0]
    fixed[1] = 3.25
    assert fixed[1] * 2.0 == 6.5

    # string elements
    words = [w + "!" for w in ["a", "b"]]
    assert words[0] + words[1] == "a!b!"


def _p8h_sum():
    assert sum([1, 2, 3]) == 6
    assert sum([0.5, 1.5, 2.0]) == 4.0
    assert sum([1, 2], 10) == 13
    assert sum([0.25, 0.25], 1.0) == 1.5
    assert sum(range(10)) == 45

    # result feeds typed numeric code without annotations
    # (binary-exact fractions: CPython >= 3.12 uses Neumaier compensated
    # summation for floats, our expansion is a naive left fold)
    total = sum([0.125, 0.25, 0.5])
    assert total * 2.0 == 1.75
    half = sum([1, 2, 3, 4]) / 2
    assert half == 5.0

    # generator-expression argument (materialized as a list comprehension)
    assert sum(i * i for i in range(5)) == 30
    assert sum(x * 0.5 for x in [1.0, 2.0, 3.0]) == 3.0
    assert sum(i for i in range(10) if i % 2 == 0) == 20

    # nested sums
    assert sum([sum([1, 2]), sum([3, 4])]) == 10

    # bools count as ints
    assert sum([True, True, False]) == 2

    # empty list with int elements
    assert sum([0, 0]) + sum([1]) == 1


_p8e_language()
_p8h_lang()
_p8h_comp_elem()
_p8h_sum()

print("All language feature tests passed!")
