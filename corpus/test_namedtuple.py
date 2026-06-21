# Differential corpus: collections.namedtuple desugared into a synthesized class
# with positional fields (frontend-only). Every check is an assert so a wrong
# result fails on BOTH pyaot and CPython; the only print is the final marker.
#
# Covers: construction, .field / [index] / len access, repr (!r byte-exact),
# __eq__ (same-class + cross-type), iteration (for/list/tuple/sum), membership
# (in), `*p` spread, tuple-unpacking, the list/tuple/string field specs, the
# qualified form, single field, and nested namedtuple fields.
#
# Out of scope (NOT asserted — they diverge by design): equality against a real
# tuple (`p == (1, 2)`), negative indices / slices, the `_make`/`_asdict`/
# `_replace`/`_fields` API, `defaults=`/`rename=`.

from collections import namedtuple
import collections

# ===== SECTION: basics — fields, repr, indexing, len =====

Point = namedtuple("Point", ["x", "y"])
p = Point(1, 2)
# Attribute access.
assert p.x == 1
assert p.y == 2
# Positional indexing (synthesized __getitem__).
assert p[0] == 1
assert p[1] == 2
# len (synthesized __len__).
assert len(p) == 2
# Byte-exact repr uses the typename and `!r`.
assert repr(p) == "Point(x=1, y=2)", repr(p)
# str() (and print()) falls back to __repr__.
assert str(p) == "Point(x=1, y=2)", str(p)

# ===== SECTION: __eq__ — same-class and cross-type =====

assert Point(1, 2) == Point(1, 2)
assert not (Point(1, 2) == Point(1, 3))
assert Point(1, 2) != Point(3, 4)
assert not (Point(1, 2) != Point(1, 2))
# Cross-type comparison is False (the isinstance guard fails), never raises.
assert not (Point(1, 2) == "not a point")
assert Point(1, 2) != 42

# ===== SECTION: iteration — for / list / tuple / sum =====

acc = []
for v in p:
    acc.append(v)
assert acc == [1, 2]
assert list(p) == [1, 2]
assert tuple(p) == (1, 2)
assert sum(p) == 3

# ===== SECTION: membership and `*p` spread =====

assert 1 in p
assert 2 in p
assert 99 not in p


def add2(a, b):
    return a + b


assert add2(*p) == 3

# ===== SECTION: tuple-unpacking (len + getitem) =====

a, b = p
assert a == 1
assert b == 2

# ===== SECTION: field-spec forms — list / space-string / comma-string =====

Color = namedtuple("Color", "r g b")
c = Color(255, 128, 0)
assert c.r == 255
assert c[2] == 0
assert len(c) == 3
assert repr(c) == "Color(r=255, g=128, b=0)", repr(c)
r, g, bl = c
assert (r, g, bl) == (255, 128, 0)

Rec = namedtuple("Rec", "name, age")
rec = Rec("Bob", 30)
assert rec.name == "Bob"
assert rec.age == 30
# str field reprs with quotes (!r).
assert repr(rec) == "Rec(name='Bob', age=30)", repr(rec)

# ===== SECTION: qualified collections.namedtuple form =====

Q = collections.namedtuple("Q", ["v"])
q = Q(42)
assert q.v == 42
assert repr(q) == "Q(v=42)", repr(q)
assert Q(42) == Q(42)
assert Q(42) != Q(43)

# ===== SECTION: single field =====

One = namedtuple("One", ["only"])
o = One(7)
assert o.only == 7
assert o[0] == 7
assert len(o) == 1
assert list(o) == [7]
assert o == One(7)
assert o != One(8)

# ===== SECTION: mixed-type fields + nested namedtuple field =====

Item = namedtuple("Item", ["label", "qty", "price"])
it = Item("apple", 3, 1.5)
assert it.label == "apple"
assert it.price == 1.5
assert repr(it) == "Item(label='apple', qty=3, price=1.5)", repr(it)
assert it == Item("apple", 3, 1.5)
assert it != Item("apple", 4, 1.5)

Line = namedtuple("Line", ["start", "end"])
ln = Line(Point(0, 0), Point(3, 4))
assert ln.start == Point(0, 0)
assert ln.end.y == 4
# Nested namedtuple repr recurses through the field's __repr__.
assert repr(ln) == "Line(start=Point(x=0, y=0), end=Point(x=3, y=4))", repr(ln)
assert ln == Line(Point(0, 0), Point(3, 4))
assert ln != Line(Point(0, 0), Point(3, 5))

print("All namedtuple tests passed!")
