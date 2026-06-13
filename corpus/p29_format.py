# §5/§9/§13 — format() / str.format() / !a-ascii() focused suite.
# Every assertion must match CPython output exactly (differential gate).

# ---------------------------------------------------------------------------
# str.format(): auto-indexed positional fields
# ---------------------------------------------------------------------------
assert "{}".format("only") == "only"
assert "{} + {} = {}".format(1, 2, 3) == "1 + 2 = 3"
assert "Hello {}!".format("world") == "Hello world!"
assert "{} and {}".format("a", "b") == "a and b"

print("auto positional: OK")

# ---------------------------------------------------------------------------
# str.format(): explicit (manual) positional indices, including reuse + reorder
# ---------------------------------------------------------------------------
assert "{0}".format("x") == "x"
assert "{0} {1} {0}".format("a", "b") == "a b a"
assert "{2}{1}{0}".format("c", "b", "a") == "abc"
assert "{1} + {0} = {2}".format(1, 2, 3) == "2 + 1 = 3"

print("manual positional: OK")

# ---------------------------------------------------------------------------
# str.format(): keyword fields, and mixed positional + keyword
# ---------------------------------------------------------------------------
assert "{name}".format(name="Alice") == "Alice"
assert "{x} and {y} and {x}".format(x="foo", y="bar") == "foo and bar and foo"
assert "{0} and {name}".format("first", name="second") == "first and second"
assert "{0}: {item}, {1}: {value}".format("key", "val", item="apple", value=42) == "key: apple, val: 42"

print("keyword + mixed: OK")

# ---------------------------------------------------------------------------
# str.format(): brace escapes and zero placeholders
# ---------------------------------------------------------------------------
assert "No placeholders".format() == "No placeholders"
assert "".format() == ""
assert "Use {{}} for placeholders".format() == "Use {} for placeholders"
assert "{{{}}}".format(42) == "{42}"
assert "{{literal}}".format() == "{literal}"

print("escapes: OK")

# ---------------------------------------------------------------------------
# str.format(): static format specs inside fields (auto / indexed / keyword)
# ---------------------------------------------------------------------------
assert "Hello {:>10}!".format("world") == "Hello      world!"
assert "{0:5d} + {1:5d}".format(1, 2) == "    1 +     2"
assert "{name:>10}".format(name="Alice") == "     Alice"
assert "{:.4f}".format(3.14159) == "3.1416"
assert "{:*^7}".format("a") == "***a***"
assert "{0:>5}{1:<5}".format("x", "y") == "    xy    "
assert "{val:08.2f}".format(val=3.14159) == "00003.14"
assert "{:x}".format(255) == "ff"
assert "{:,}".format(1234567) == "1,234,567"

print("format specs: OK")

# ---------------------------------------------------------------------------
# str.format(): conversion flags inside fields
# ---------------------------------------------------------------------------
assert "{!r}".format("hi") == "'hi'"
assert "{0!r}".format("hi") == "'hi'"
assert "{name!r}".format(name="hi") == "'hi'"
assert "{!s}".format(42) == "42"
assert "{!r} and {!s}".format("a", "b") == "'a' and b"

print("format conversions: OK")

# ---------------------------------------------------------------------------
# format() builtin (single + spec)
# ---------------------------------------------------------------------------
assert format(42) == "42"
assert format(3.14) == "3.14"
assert format("hi") == "hi"
assert format(42, "5d") == "   42"
assert format(3.14159, ".2f") == "3.14"
assert format("hello", ">10") == "     hello"
assert format(255, "#x") == "0xff"
assert format(True, "d") == "1"
assert format(7, "05") == "00007"

print("format() builtin: OK")

# ---------------------------------------------------------------------------
# ascii() builtin and the f-string `!a` conversion (ASCII content)
# ---------------------------------------------------------------------------
assert ascii("hello") == "'hello'"
assert ascii(42) == "42"
assert ascii(3.14) == "3.14"
assert ascii(True) == "True"
assert ascii(None) == "None"
assert ascii([1, 2, 3]) == "[1, 2, 3]"

a_str = "world"
assert f"{a_str!a}" == "'world'"
a_int = 42
assert f"{a_int!a}" == "42"
assert "{!a}".format("z") == "'z'"

print("ascii / !a: OK")

# ---------------------------------------------------------------------------
# Dynamic (nested) f-string format specs reuse the same engine
# ---------------------------------------------------------------------------
prec = 3
assert f"{3.14159:.{prec}f}" == "3.142"
wid = 6
assert f"{42:{wid}d}" == "    42"
assert f"{'hi':>{wid}}" == "    hi"

print("dynamic specs: OK")

# ---------------------------------------------------------------------------
# User-class __format__ via f-string, format() and bare {p}
# ---------------------------------------------------------------------------
class Temperature:
    def __init__(self, celsius: float):
        self.celsius = celsius

    def __format__(self, spec: str) -> str:
        if spec == "F":
            return str(self.celsius * 9 / 5 + 32) + "F"
        return str(self.celsius) + "C"

t = Temperature(100.0)
assert f"{t}" == "100.0C"
assert f"{t:F}" == "212.0F"
assert format(t, "F") == "212.0F"
assert "{}".format(t) == "100.0C"
assert "{0:F}".format(t) == "212.0F"


class Labelled:
    def __init__(self, label: str):
        self.label = label

    def __str__(self) -> str:
        return "label:" + self.label


# A class with __str__ but no __format__: empty spec routes to __str__.
lab = Labelled("hi")
assert f"{lab}" == "label:hi"
assert "{}".format(lab) == "label:hi"

print("user __format__: OK")

print("All p29 format tests passed!")
