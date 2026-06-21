# Test match statement (pattern matching)

# ===== SECTION: Basic Literal Matching =====

def test_match_int_literal():
    x: int = 1
    result: int = 0
    match x:
        case 1:
            result = 10
        case 2:
            result = 20
        case _:
            result = -1
    assert result == 10

test_match_int_literal()

def test_match_int_literal_second():
    x: int = 2
    result: int = 0
    match x:
        case 1:
            result = 10
        case 2:
            result = 20
        case _:
            result = -1
    assert result == 20

test_match_int_literal_second()

def test_match_int_literal_default():
    x: int = 999
    result: int = 0
    match x:
        case 1:
            result = 10
        case 2:
            result = 20
        case _:
            result = -1
    assert result == -1

test_match_int_literal_default()

def test_match_str_literal():
    x: str = "hello"
    result: int = 0
    match x:
        case "hello":
            result = 1
        case "world":
            result = 2
        case _:
            result = -1
    assert result == 1

test_match_str_literal()

def test_match_str_literal_second():
    x: str = "world"
    result: int = 0
    match x:
        case "hello":
            result = 1
        case "world":
            result = 2
        case _:
            result = -1
    assert result == 2

test_match_str_literal_second()

# ===== SECTION: Singleton Patterns =====

def test_match_true():
    x: bool = True
    result: int = 0
    match x:
        case True:
            result = 1
        case False:
            result = 2
    assert result == 1

test_match_true()

def test_match_false():
    x: bool = False
    result: int = 0
    match x:
        case True:
            result = 1
        case False:
            result = 2
    assert result == 2

test_match_false()

# ===== SECTION: Capture Patterns (as) =====

def test_match_capture():
    x: int = 42
    result: int = 0
    match x:
        case y:
            result = y
    assert result == 42

test_match_capture()

def test_match_capture_with_literal_first():
    x: int = 42
    result: int = 0
    match x:
        case 1:
            result = 1
        case y:
            result = y + 100
    assert result == 142

test_match_capture_with_literal_first()

def test_match_capture_literal_matches():
    x: int = 1
    result: int = 0
    match x:
        case 1:
            result = 1
        case y:
            result = y + 100
    assert result == 1

test_match_capture_literal_matches()

# ===== SECTION: Or Patterns =====

def test_match_or_pattern():
    x: int = 2
    result: int = 0
    match x:
        case 1 | 2 | 3:
            result = 10
        case _:
            result = -1
    assert result == 10

test_match_or_pattern()

def test_match_or_pattern_first():
    x: int = 1
    result: int = 0
    match x:
        case 1 | 2 | 3:
            result = 10
        case _:
            result = -1
    assert result == 10

test_match_or_pattern_first()

def test_match_or_pattern_third():
    x: int = 3
    result: int = 0
    match x:
        case 1 | 2 | 3:
            result = 10
        case _:
            result = -1
    assert result == 10

test_match_or_pattern_third()

def test_match_or_pattern_no_match():
    x: int = 99
    result: int = 0
    match x:
        case 1 | 2 | 3:
            result = 10
        case _:
            result = -1
    assert result == -1

test_match_or_pattern_no_match()

# ===== SECTION: Guard Clauses =====

def test_match_guard_true():
    x: int = 5
    result: int = 0
    match x:
        case n if n > 3:
            result = 1
        case _:
            result = -1
    assert result == 1

test_match_guard_true()

def test_match_guard_false():
    x: int = 2
    result: int = 0
    match x:
        case n if n > 3:
            result = 1
        case _:
            result = -1
    assert result == -1

test_match_guard_false()

def test_match_guard_with_literal():
    x: int = 10
    result: int = 0
    match x:
        case 10 if x > 5:
            result = 1
        case 10:
            result = 2
        case _:
            result = -1
    assert result == 1

test_match_guard_with_literal()

def test_match_guard_literal_guard_false():
    x: int = 3
    result: int = 0
    match x:
        case 3 if x > 5:
            result = 1
        case 3:
            result = 2
        case _:
            result = -1
    assert result == 2

test_match_guard_literal_guard_false()

# ===== SECTION: Sequence Patterns =====

def test_match_list_exact():
    x: list[int] = [1, 2]
    result: int = 0
    match x:
        case [a, b]:
            result = a + b
        case _:
            result = -1
    assert result == 3

test_match_list_exact()

def test_match_list_wrong_length():
    x: list[int] = [1, 2, 3]
    result: int = 0
    match x:
        case [a, b]:
            result = a + b
        case _:
            result = -1
    assert result == -1

test_match_list_wrong_length()

def test_match_list_with_values():
    x: list[int] = [1, 2, 3]
    result: int = 0
    match x:
        case [1, second, 3]:
            result = second
        case _:
            result = -1
    assert result == 2

test_match_list_with_values()

def test_match_list_value_no_match():
    x: list[int] = [1, 2, 4]
    result: int = 0
    match x:
        case [1, second, 3]:
            result = second
        case _:
            result = -1
    assert result == -1

test_match_list_value_no_match()

# ===== SECTION: Starred Patterns =====

def test_match_starred_empty():
    x: list[int] = [1, 2]
    result: int = 0
    match x:
        case [first, *rest]:
            result = first + len(rest)
    assert result == 2  # 1 + len([2]) = 2

test_match_starred_empty()

def test_match_starred_multiple():
    x: list[int] = [1, 2, 3, 4, 5]
    result: int = 0
    match x:
        case [first, *middle, last]:
            result = first + last + len(middle)
    assert result == 9  # 1 + 5 + len([2, 3, 4]) = 9

test_match_starred_multiple()

def test_match_starred_at_start():
    x: list[int] = [1, 2, 3, 4, 5]
    result: int = 0
    match x:
        case [*init, last]:
            result = len(init) + last
    assert result == 9  # len([1, 2, 3, 4]) + 5 = 9

test_match_starred_at_start()

# ===== SECTION: Multiple Cases =====

def test_match_multi_case():
    x: int = 42
    result: str = ""
    match x:
        case 0:
            result = "zero"
        case 1:
            result = "one"
        case 42:
            result = "answer"
        case _:
            result = "other"
    assert result == "answer"

test_match_multi_case()

# ===== SECTION: Nested Expressions =====

def test_match_expression_subject():
    result: int = 0
    match 1 + 1:
        case 2:
            result = 1
        case _:
            result = -1
    assert result == 1

test_match_expression_subject()

def test_match_function_subject():
    def get_val() -> int:
        return 42

    result: int = 0
    match get_val():
        case 42:
            result = 1
        case _:
            result = -1
    assert result == 1

test_match_function_subject()

# ===== SECTION: Complex Guards =====

def test_match_complex_guard():
    x: int = 10
    y: int = 5
    result: int = 0
    match x:
        case n if n > 0 and y < 10:
            result = n + y
        case _:
            result = -1
    assert result == 15

test_match_complex_guard()

# Issue #1: sequence pattern short-circuit (was crashing on out-of-bounds)
def test_match_sequence_short_subject():
    x: list[int] = [1]
    result: int = 0
    match x:
        case [a, b, c]:
            result = 1
        case _:
            result = -1
    assert result == -1

test_match_sequence_short_subject()

def test_match_sequence_empty_subject():
    x: list[int] = []
    result: int = 0
    match x:
        case [a]:
            result = 1
        case _:
            result = -1
    assert result == -1

test_match_sequence_empty_subject()

def test_match_sequence_multiple_fallthrough():
    x: list[int] = [5]
    result: int = 0
    match x:
        case [a, b, c]:
            result = 3
        case [a, b]:
            result = 2
        case [a]:
            result = 1
        case _:
            result = -1
    assert result == 1

test_match_sequence_multiple_fallthrough()

# Issue #3: or-pattern second alternative
def test_match_or_pattern_second_alt():
    x: int = 2
    result: int = 0
    match x:
        case 1 | 2 | 3:
            result = x
        case _:
            result = -1
    assert result == 2

test_match_or_pattern_second_alt()

# Issue #4: mapping pattern key containment (control flow correctness)
def test_match_mapping_key_missing():
    d: dict[str, str] = {"a": "hello"}
    result: str = ""
    match d:
        case {"b": v}:
            result = v
        case {"a": v}:
            result = v
        case _:
            result = "none"
    assert result == "hello"

test_match_mapping_key_missing()

def test_match_mapping_multiple_keys():
    d: dict[str, str] = {"x": "ten", "y": "twenty"}
    result: str = ""
    match d:
        case {"a": v1, "b": v2}:
            result = v1
        case {"x": v1, "y": v2}:
            result = v1
        case _:
            result = "none"
    assert result == "ten"

test_match_mapping_multiple_keys()

# Issue #10: singleton pattern matching
def test_match_singleton_bool():
    x: bool = True
    result: int = 0
    match x:
        case True:
            result = 1
        case False:
            result = 2
    assert result == 1

test_match_singleton_bool()

def test_match_singleton_false():
    x: bool = False
    result: int = 0
    match x:
        case True:
            result = 1
        case False:
            result = 2
    assert result == 2

test_match_singleton_false()

# ===== SECTION: Mapping Patterns with **rest =====

# Basic: **rest excludes matched keys
d_rest1: dict[str, int] = {"a": 1, "b": 2, "c": 3}
match d_rest1:
    case {"a": val_a, **rest1}:
        pass
assert val_a == 1, "matched value wrong"
assert "a" not in rest1, "'a' should be excluded from rest"
assert rest1["b"] == 2 and rest1["c"] == 3, "wrong rest values"
assert len(rest1) == 2, "wrong rest length"

# Multiple matched keys
d_rest2: dict[str, int] = {"x": 10, "y": 20, "z": 30}
match d_rest2:
    case {"x": vx, "y": vy, **rest2}:
        pass
assert vx == 10 and vy == 20
assert len(rest2) == 1 and rest2["z"] == 30, "wrong rest"

# All keys matched — rest is empty
d_rest3: dict[str, int] = {"a": 1}
match d_rest3:
    case {"a": va, **rest3}:
        pass
assert len(rest3) == 0, "expected empty rest"

# Original dict is not modified (copy semantics)
d_rest4: dict[str, int] = {"a": 1, "b": 2}
match d_rest4:
    case {"a": _, **rest4}:
        pass
assert len(d_rest4) == 2 and d_rest4["a"] == 1, "original modified"

print("Mapping pattern **rest tests passed!")

# ===== SECTION: Class Patterns =====

class Point:
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

# Basic class pattern: capture all keyword attrs
def test_match_class_basic():
    p: Point = Point(1, 2)
    result: int = 0
    match p:
        case Point(x=a, y=b):
            result = a + b
    assert result == 3

test_match_class_basic()

# Class pattern with literal value check
def test_match_class_literal_match():
    p: Point = Point(0, 5)
    result: int = 0
    match p:
        case Point(x=0, y=val):
            result = val
        case _:
            result = -1
    assert result == 5

test_match_class_literal_match()

# Class pattern literal check - no match, falls through
def test_match_class_literal_no_match():
    p: Point = Point(3, 5)
    result: int = 0
    match p:
        case Point(x=0, y=val):
            result = val
        case Point(x=a, y=b):
            result = a + b
    assert result == 8

test_match_class_literal_no_match()

# Class pattern isinstance-only (no attr checks)
def test_match_class_isinstance_only():
    p: Point = Point(1, 2)
    result: int = 0
    match p:
        case Point():
            result = 1
        case _:
            result = -1
    assert result == 1

test_match_class_isinstance_only()

# Class pattern with wildcard fallthrough
def test_match_class_fallthrough():
    p: Point = Point(7, 8)
    result: str = ""
    match p:
        case Point(x=0, y=0):
            result = "origin"
        case Point(x=a, y=b):
            result = "point"
        case _:
            result = "other"
    assert result == "point"

test_match_class_fallthrough()

# Multiple class types - isinstance discriminates correctly
class Color:
    def __init__(self, r: int, g: int, b: int):
        self.r = r
        self.g = g
        self.b = b

def test_match_class_multiple_types():
    p: Point = Point(1, 2)
    result: str = ""
    match p:
        case Color(r=r, g=g, b=b):
            result = "color"
        case Point(x=x, y=y):
            result = "point"
        case _:
            result = "other"
    assert result == "point"

test_match_class_multiple_types()

def test_match_class_multiple_types_color():
    c: Color = Color(255, 0, 128)
    result: str = ""
    match c:
        case Point(x=x, y=y):
            result = "point"
        case Color(r=r, g=g, b=b):
            result = "color"
        case _:
            result = "other"
    assert result == "color"

test_match_class_multiple_types_color()

# Guard with class pattern
def test_match_class_guard():
    p: Point = Point(5, 10)
    result: int = 0
    match p:
        case Point(x=x, y=y) if x > 3:
            result = x + y
        case Point(x=x, y=y):
            result = x
    assert result == 15

test_match_class_guard()

# Guard false - falls to next case
def test_match_class_guard_false():
    p: Point = Point(1, 10)
    result: int = 0
    match p:
        case Point(x=x, y=y) if x > 3:
            result = x + y
        case Point(x=x, y=y):
            result = x
    assert result == 1

test_match_class_guard_false()

# Class pattern matching origin point
def test_match_class_origin():
    p: Point = Point(0, 0)
    result: str = ""
    match p:
        case Point(x=0, y=0):
            result = "origin"
        case Point(x=0, y=y):
            result = "y-axis"
        case Point(x=x, y=0):
            result = "x-axis"
        case Point(x=x, y=y):
            result = "general"
    assert result == "origin"

test_match_class_origin()

def test_match_class_y_axis():
    p: Point = Point(0, 5)
    result: str = ""
    match p:
        case Point(x=0, y=0):
            result = "origin"
        case Point(x=0, y=y):
            result = "y-axis"
        case Point(x=x, y=0):
            result = "x-axis"
        case Point(x=x, y=y):
            result = "general"
    assert result == "y-axis"

test_match_class_y_axis()

def test_match_class_x_axis():
    p: Point = Point(3, 0)
    result: str = ""
    match p:
        case Point(x=0, y=0):
            result = "origin"
        case Point(x=0, y=y):
            result = "y-axis"
        case Point(x=x, y=0):
            result = "x-axis"
        case Point(x=x, y=y):
            result = "general"
    assert result == "x-axis"

test_match_class_x_axis()

def test_match_class_general():
    p: Point = Point(3, 7)
    result: str = ""
    match p:
        case Point(x=0, y=0):
            result = "origin"
        case Point(x=0, y=y):
            result = "y-axis"
        case Point(x=x, y=0):
            result = "x-axis"
        case Point(x=x, y=y):
            result = "general"
    assert result == "general"

test_match_class_general()

print("Class pattern tests passed!")

# ===== SECTION: Class Patterns with Inheritance =====

class Shape:
    def __init__(self, name: str):
        self.name = name

class Circle(Shape):
    def __init__(self, name: str, radius: int):
        self.name = name
        self.radius = radius

# Subclass matches parent pattern (isinstance semantics)
def test_match_class_inheritance():
    c: Circle = Circle("circle", 5)
    result: str = ""
    match c:
        case Shape(name=n):
            result = n
        case _:
            result = "other"
    assert result == "circle"

test_match_class_inheritance()

# More specific pattern first
def test_match_class_inheritance_specific_first():
    c: Circle = Circle("mycirc", 10)
    result: str = ""
    match c:
        case Circle(name=n, radius=r):
            result = "circle"
        case Shape(name=n):
            result = "shape"
        case _:
            result = "other"
    assert result == "circle"

test_match_class_inheritance_specific_first()

print("Class pattern inheritance tests passed!")

# ===== SECTION: Folded from p7_match.py (structural match desugar) =====

# User class used in class-patterns must live at module level (cannot nest).
class _p7m_Point:
    x: int
    y: int

    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y


def _fold_or_capture():
    # ── or-patterns WITH capture: every alternative binds the same name(s);
    # the bind on whichever alternative matched survives the merge. ──

    # sequence alternatives of different lengths, same capture
    def seq_first(v: list[int]) -> int:
        match v:
            case [x] | [x, _]:
                return x
            case _:
                return -1

    assert seq_first([5]) == 5
    assert seq_first([7, 8]) == 7
    assert seq_first([]) == -1

    # tuple alternatives, bind the non-zero element
    def tuple_other(v: tuple[int, int]) -> int:
        match v:
            case (0, y) | (y, 0):
                return y
            case _:
                return -99

    assert tuple_other((0, 5)) == 5
    assert tuple_other((9, 0)) == 9
    assert tuple_other((1, 2)) == -99

    # three alternatives, all bind x
    def three(v: tuple[int, int]) -> int:
        match v:
            case (1, x) | (2, x) | (3, x):
                return x
            case _:
                return -1

    assert three((1, 11)) == 11
    assert three((2, 22)) == 22
    assert three((3, 33)) == 33
    assert three((4, 44)) == -1

    # mapping alternatives, same capture name
    def mapping_v(d: dict[str, int]) -> int:
        match d:
            case {"a": v} | {"b": v}:
                return v
            case _:
                return 0

    assert mapping_v({"a": 1}) == 1
    assert mapping_v({"b": 2}) == 2
    assert mapping_v({"c": 3}) == 0

    # keyword class patterns, literal discriminant + shared capture
    def class_v(p: _p7m_Point) -> int:
        match p:
            case _p7m_Point(x=0, y=v) | _p7m_Point(y=0, x=v):
                return v
            case _:
                return -1

    assert class_v(_p7m_Point(0, 7)) == 7   # x==0 → v = y
    assert class_v(_p7m_Point(9, 0)) == 9   # y==0 → v = x
    assert class_v(_p7m_Point(2, 3)) == -1

    # guard reads the OR-bound capture
    def guarded(v: list[int]) -> int:
        match v:
            case [a] | [a, _] if a > 0:
                return a
            case _:
                return -1

    assert guarded([5]) == 5
    assert guarded([7, 8]) == 7
    assert guarded([-3]) == -1   # guard fails on the bound `a`

    # `(... | ...) as name`: the OR binds nothing, the `as` binds the whole match
    def as_form(v: list[int]) -> int:
        match v:
            case ([1, 2] | [3, 4]) as pair:
                return len(pair)
            case _:
                return 0

    assert as_form([1, 2]) == 2
    assert as_form([3, 4]) == 2
    assert as_form([5, 6]) == 0


_fold_or_capture()


def _fold_p7_match():
    # ── literals + default ──
    def kind(n: int) -> str:
        match n:
            case 0:
                return "zero"
            case 1:
                return "one"
            case _:
                return "many"

    assert kind(0) == "zero"
    assert kind(1) == "one"
    assert kind(5) == "many"

    # ── string literals ──
    def color(s: str) -> int:
        match s:
            case "red":
                return 1
            case "green":
                return 2
            case _:
                return 0

    assert color("red") == 1
    assert color("green") == 2
    assert color("blue") == 0

    # ── singletons ──
    def truthy(b: bool) -> str:
        match b:
            case True:
                return "yes"
            case False:
                return "no"

    assert truthy(True) == "yes"
    assert truthy(False) == "no"

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

    assert bucket(0) == "zero"
    assert bucket(-3) == "neg"
    assert bucket(7) == "small"
    assert bucket(42) == "big:42"

    # ── or-patterns (capture-free) on strings ──
    def vowel(c: str) -> bool:
        match c:
            case "a" | "e" | "i" | "o" | "u":
                return True
            case _:
                return False

    assert vowel("a") == True
    assert vowel("z") == False

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

    assert shape([]) == "empty"
    assert shape([5]) == "one:5"
    assert shape([2, 3]) == "two:5"
    assert shape([1, 2, 3, 4]) == "many:1:3"

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

    assert tail_sum([10, 1, 2, 3]) == 6
    assert tail_sum([]) == -1

    # ── mapping patterns (literal value inside mapping) ──
    def role(d: dict[str, str]) -> str:
        match d:
            case {"role": "admin"}:
                return "admin"
            case {"role": r}:
                return "role:" + r
            case _:
                return "none"

    assert role({"role": "admin"}) == "admin"
    assert role({"role": "user"}) == "role:user"
    assert role({"name": "x"}) == "none"

    # ── mapping with **rest (copy semantics, original untouched) ──
    def split(d: dict[str, int]) -> int:
        match d:
            case {"a": a, **rest}:
                return a + len(rest)
            case _:
                return -1

    base = {"a": 10, "b": 2, "c": 3}
    assert split(base) == 12
    assert len(base) == 3
    assert split({"b": 1}) == -1

    # ── class patterns (keyword-only) ──
    def describe(p: _p7m_Point) -> str:
        match p:
            case _p7m_Point(x=0, y=0):
                return "origin"
            case _p7m_Point(x=0, y=yy):
                return "on-y:" + str(yy)
            case _p7m_Point(x=xx, y=0):
                return "on-x:" + str(xx)
            case _p7m_Point(x=a, y=b):
                return "at:" + str(a) + "," + str(b)

    assert describe(_p7m_Point(0, 0)) == "origin"
    assert describe(_p7m_Point(0, 5)) == "on-y:5"
    assert describe(_p7m_Point(3, 0)) == "on-x:3"
    assert describe(_p7m_Point(2, 4)) == "at:2,4"

    # ── nested literal inside sequence ──
    def pair_kind(p: list[int]) -> str:
        match p:
            case [0, y]:
                return "zero-first:" + str(y)
            case [x, 0]:
                return "zero-second:" + str(x)
            case _:
                return "other"

    assert pair_kind([0, 9]) == "zero-first:9"
    assert pair_kind([7, 0]) == "zero-second:7"
    assert pair_kind([1, 2]) == "other"


_fold_p7_match()

# ── match as a statement (no return), capture leaks like CPython ──
match [1, 2, 3]:
    case [a, *bs]:
        leak = a + len(bs)
    case _:
        leak = -1
assert leak == 3

print("p7_match fold tests passed!")

print("All match tests passed!")
