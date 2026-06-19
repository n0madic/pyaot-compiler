# Class decorators (§5) — side-effecting decorators that return the class
# unchanged, plus the parameterized factory form. The "class value" the
# decorator receives is the class-id int (the same convention as a
# `@classmethod`'s `cls` and `object.__new__(cls)`); the class name stays bound
# to its class id, so `C(...)` still constructs. A decorator that returns a
# DIFFERENT class, or stores the class as a value, is out of scope.

_marks: list = []
_count: list = [0]


# A plain side-effecting decorator: runs for effect, returns the class.
def register(cls: int) -> int:
    _marks.append(cls)
    _count[0] = _count[0] + 1
    return cls


# A parameterized factory decorator `@label("name")`.
def label(name: str):
    def deco(cls: int) -> int:
        _marks.append(cls)
        return cls

    return deco


@register
class Widget:
    def __init__(self, n: int):
        self.n = n

    def doubled(self) -> int:
        return self.n * 2


@label("gadget")
class Gadget:
    def __init__(self, m: int):
        self.m = m

    def tripled(self) -> int:
        return self.m * 3


# Stacked: both decorators run (innermost first), the class is unchanged.
@register
@label("both")
class Both:
    def __init__(self, k: int):
        self.k = k


# (a) The side effects ran.
assert _count[0] == 2, f"register ran twice: {_count[0]}"
assert len(_marks) == 4, f"four decorator applications: {len(_marks)}"

# (b) The decorated classes still construct and behave normally.
w = Widget(5)
assert w.doubled() == 10, f"Widget.doubled: {w.doubled()}"
assert w.n == 5, f"Widget.n: {w.n}"

g = Gadget(7)
assert g.tripled() == 21, f"Gadget.tripled: {g.tripled()}"

b = Both(9)
assert b.k == 9, f"Both.k: {b.k}"

# The decorator received the class id (an int) — markers are the same id twice
# for the stacked class, distinct ids for the others.
assert _marks[0] != _marks[1], "Widget vs Gadget have distinct ids"

print("class decorators: PASS")
