# Differential corpus: @dataclass synthesis of __init__/__repr__/__eq__ from
# field annotations (frontend-only desugaring). Every check is an assert so a
# wrong result fails on BOTH pyaot and CPython; the only print is the final
# marker, so stdout is one line compared byte-for-byte against CPython.
#
# Scope: bare @dataclass / @dataclass() / @dataclasses.dataclass forms, annotated
# fields and literal defaults, ClassVar exclusion, missing-only dunder synthesis.

from dataclasses import dataclass
import dataclasses
from typing import ClassVar

# ===== SECTION: basic dataclass — int/str/float/bool fields =====


@dataclass
class Basic:
    n: int
    name: str
    ratio: float
    flag: bool


b = Basic(1, "hi", 1.5, True)
# Field access.
assert b.n == 1
assert b.name == "hi"
assert b.ratio == 1.5
assert b.flag is True
# Byte-exact repr: !r quotes the str field, leaves the rest bare.
assert repr(b) == "Basic(n=1, name='hi', ratio=1.5, flag=True)", repr(b)
# str() (and hence print()) falls back to __repr__.
assert str(b) == "Basic(n=1, name='hi', ratio=1.5, flag=True)", str(b)

# ===== SECTION: __eq__ — same-class equal/unequal, cross-type, __ne__ =====


@dataclass
class Pair:
    x: int
    y: int


p1 = Pair(1, 2)
p2 = Pair(1, 2)
p3 = Pair(3, 4)
# Equal values → equal; differing values → not equal.
assert p1 == p2
assert not (p1 == p3)
# __ne__ is auto-derived from __eq__.
assert p1 != p3
assert not (p1 != p2)
# Cross-type comparison returns False (the isinstance guard fails), never raises.
assert not (p1 == Basic(1, "hi", 1.5, True))
assert p1 != Basic(1, "hi", 1.5, True)
assert p1 != "not a pair"
assert p1 != 42

# ===== SECTION: literal defaults + partial construction =====


@dataclass
class WithDefaults:
    a: int
    b: int = 0
    c: str = "x"
    d: float = -1.5
    e: int = -7


# All defaults applied.
wd1 = WithDefaults(5)
assert wd1.a == 5
assert wd1.b == 0
assert wd1.c == "x"
assert wd1.d == -1.5
assert wd1.e == -7
assert repr(wd1) == "WithDefaults(a=5, b=0, c='x', d=-1.5, e=-7)", repr(wd1)
# Partial override.
wd2 = WithDefaults(5, 6, "y")
assert wd2.b == 6
assert wd2.c == "y"
assert wd2.d == -1.5
assert repr(wd2) == "WithDefaults(a=5, b=6, c='y', d=-1.5, e=-7)", repr(wd2)
# Equality respects defaults.
assert WithDefaults(5) == WithDefaults(5, 0, "x", -1.5, -7)
assert WithDefaults(5) != WithDefaults(5, 1)

# ===== SECTION: zero-field dataclass =====


@dataclass
class Empty:
    pass


e1 = Empty()
e2 = Empty()
assert repr(e1) == "Empty()", repr(e1)
# Any two instances of a fieldless dataclass are equal.
assert e1 == e2
assert not (e1 != e2)
assert e1 != Pair(1, 2)

# ===== SECTION: nested repr — list field and dataclass field =====


@dataclass
class Node:
    label: str
    value: int


@dataclass
class Tree:
    root: Node
    items: list


t = Tree(Node("r", 9), [10, 20, 30])
assert t.root == Node("r", 9)
# Nested dataclass repr recurses; list repr is the standard CPython form.
assert repr(t) == "Tree(root=Node(label='r', value=9), items=[10, 20, 30])", repr(t)
# Structural equality through the nested dataclass field.
assert t == Tree(Node("r", 9), [10, 20, 30])
assert t != Tree(Node("r", 8), [10, 20, 30])

# ===== SECTION: ClassVar stays a class attribute, not a field =====


@dataclass
class Config:
    name: str
    version: int = 1
    KIND: ClassVar[str] = "config"


cfg = Config("main")
assert cfg.name == "main"
assert cfg.version == 1
# ClassVar is reachable as a class attribute and is NOT part of init/repr/eq.
assert cfg.KIND == "config"
assert Config.KIND == "config"
assert repr(cfg) == "Config(name='main', version=1)", repr(cfg)
# Two configs differing only by (immutable) class var are equal.
assert Config("main") == Config("main", 1)

# ===== SECTION: qualified @dataclasses.dataclass / @dataclass() forms =====


@dataclasses.dataclass
class Qualified:
    a: int


@dataclasses.dataclass()
class QualifiedCalled:
    a: int


@dataclass()
class Called:
    a: int


assert repr(Qualified(3)) == "Qualified(a=3)", repr(Qualified(3))
assert repr(QualifiedCalled(4)) == "QualifiedCalled(a=4)", repr(QualifiedCalled(4))
assert repr(Called(5)) == "Called(a=5)", repr(Called(5))
assert Qualified(3) == Qualified(3)
assert Called(5) != Called(6)

# ===== SECTION: user-defined dunders are preserved, not overwritten =====


@dataclass
class CustomRepr:
    x: int

    def __repr__(self) -> str:
        return "CUSTOM<" + str(self.x) + ">"


cr = CustomRepr(7)
# The user's __repr__ wins; __init__/__eq__ are still synthesized.
assert repr(cr) == "CUSTOM<7>", repr(cr)
assert cr.x == 7
assert cr == CustomRepr(7)
assert cr != CustomRepr(8)


@dataclass
class CustomEq:
    x: int

    def __eq__(self, other) -> bool:
        # Compare on parity of x rather than equality (clearly distinguishable
        # from the synthesized __eq__).
        if isinstance(other, CustomEq):
            return (self.x % 2) == (other.x % 2)
        return False


# The user's __eq__ wins; __init__/__repr__ are still synthesized.
assert CustomEq(2) == CustomEq(4)
assert CustomEq(2) != CustomEq(3)
assert repr(CustomEq(2)) == "CustomEq(x=2)", repr(CustomEq(2))

print("All dataclass tests passed!")
