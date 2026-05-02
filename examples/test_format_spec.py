# §F.4 Format Specification Idiom Suite
# Tests PEP 3101 format spec mini-language via f-strings and format().
# Every assertion must match CPython output exactly.

# ---------------------------------------------------------------------------
# Integer type characters
# ---------------------------------------------------------------------------
assert f"{42:d}" == "42"
assert f"{42:b}" == "101010"
assert f"{42:o}" == "52"
assert f"{42:x}" == "2a"
assert f"{42:X}" == "2A"
assert f"{42:#x}" == "0x2a"
assert f"{42:#b}" == "0b101010"
assert f"{42:#o}" == "0o52"
assert f"{255:x}" == "ff"
assert f"{255:X}" == "FF"
assert f"{0:d}" == "0"
assert f"{-42:d}" == "-42"

print("integer type chars: OK")

# ---------------------------------------------------------------------------
# Width and alignment
# ---------------------------------------------------------------------------
assert f"{42:5}" == "   42"
assert f"{42:<5}" == "42   "
assert f"{42:>5}" == "   42"
assert f"{42:^5}" == " 42  "
assert f"{'hi':>10}" == "        hi"
assert f"{'hi':<10}" == "hi        "
assert f"{'hi':^10}" == "    hi    "
assert f"{'a':*^7}" == "***a***"

print("width and alignment: OK")

# ---------------------------------------------------------------------------
# Sign
# ---------------------------------------------------------------------------
assert f"{42:+d}" == "+42"
assert f"{-42:+d}" == "-42"
assert f"{42: d}" == " 42"
assert f"{-42: d}" == "-42"

print("sign: OK")

# ---------------------------------------------------------------------------
# Zero-padding
# ---------------------------------------------------------------------------
assert f"{42:05d}" == "00042"
assert f"{-42:05d}" == "-0042"
assert f"{42:08b}" == "00101010"

print("zero-padding: OK")

# ---------------------------------------------------------------------------
# Grouping separators
# ---------------------------------------------------------------------------
assert f"{1234567:,}" == "1,234,567"
assert f"{1234567:_}" == "1_234_567"
assert f"{1234567.89:,.2f}" == "1,234,567.89"
assert f"{1000000:,d}" == "1,000,000"

print("grouping separators: OK")

# ---------------------------------------------------------------------------
# Float type characters
# ---------------------------------------------------------------------------
assert f"{3.14:.2f}" == "3.14"
assert f"{0.0001234:.2e}" == "1.23e-04"
assert f"{0.25:.1%}" == "25.0%"
assert f"{1.0:.0f}" == "1"
assert f"{3.14159:.4f}" == "3.1416"
assert f"{1234.5:10.2f}" == "   1234.50"
assert f"{0.0:.4f}" == "0.0000"

print("float type chars: OK")

# ---------------------------------------------------------------------------
# String truncation (precision)
# ---------------------------------------------------------------------------
assert f"{'abcdef':.3}" == "abc"
assert f"{'hello':10}" == "hello     "
assert f"{'hello':>10}" == "     hello"

print("string truncation: OK")

# ---------------------------------------------------------------------------
# Bool formatting (treated as int subclass for numeric specs)
# ---------------------------------------------------------------------------
assert f"{True:5}" == " True"
assert f"{False:5}" == "False"
assert f"{True:d}" == "1"
assert f"{False:d}" == "0"

print("bool formatting: OK")

# ---------------------------------------------------------------------------
# §F.2 regression: non-literal expression must get correct alignment
# ---------------------------------------------------------------------------
def fmt_int_var(x: int) -> str:
    return f"{x:5d}"

r1 = fmt_int_var(2)
assert r1 == "    2", "§F.2 regression: expected '    2' got " + repr(r1)

r2 = fmt_int_var(1000)
assert r2 == " 1000", "§F.2 regression: expected ' 1000' got " + repr(r2)

r3 = fmt_int_var(-3)
assert r3 == "   -3", "§F.2 regression: expected '   -3' got " + repr(r3)

def fmt_float_var(x: float) -> str:
    return f"{x:.3f}"

r4 = fmt_float_var(3.14159)
assert r4 == "3.142", "§F.2 float var: expected '3.142' got " + repr(r4)

r5 = fmt_float_var(0.0)
assert r5 == "0.000", "§F.2 float var: expected '0.000' got " + repr(r5)

print("§F.2 variable expression: OK")

# ---------------------------------------------------------------------------
# Nested (dynamic) format specs
# ---------------------------------------------------------------------------
n = 4
r6 = f"{3.14159:.{n}f}"
assert r6 == "3.1416", "nested spec: expected '3.1416' got " + repr(r6)

width = 8
r7 = f"{42:{width}d}"
assert r7 == "      42", "nested width: expected '      42' got " + repr(r7)

print("dynamic (nested) format specs: OK")

# ---------------------------------------------------------------------------
# format() builtin — parallel with f-strings
# ---------------------------------------------------------------------------
assert format(42, "5d") == "   42"
assert format(3.14, ".2f") == "3.14"
assert format("hello", ">10") == "     hello"
assert format(True, "d") == "1"
assert format(255, "x") == "ff"
assert format(1234567, ",") == "1,234,567"

print("format() builtin: OK")

# ---------------------------------------------------------------------------
# .format() method — positional and keyword
# ---------------------------------------------------------------------------
assert "Hello {:>10}!".format("world") == "Hello      world!"
assert "{0:5d} + {1:5d}".format(1, 2) == "    1 +     2"
assert "{name:>10}".format(name="Alice") == "     Alice"
assert "{:.4f}".format(3.14159) == "3.1416"

print(".format() method: OK")

# ---------------------------------------------------------------------------
# User-class __format__
# ---------------------------------------------------------------------------
class Point:
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

    def __format__(self, spec: str) -> str:
        if spec == "polar":
            return "Point(" + str(self.x) + "," + str(self.y) + ")"
        return "(" + str(self.x) + ", " + str(self.y) + ")"

p = Point(3, 4)
r_polar = f"{p:polar}"
assert r_polar == "Point(3,4)", "user __format__: got " + repr(r_polar)
r_default = f"{p}"
assert r_default == "(3, 4)", "user __format__ default: got " + repr(r_default)
r_fmt = format(p, "polar")
assert r_fmt == "Point(3,4)", "format() user class: got " + repr(r_fmt)

print("user __format__: OK")

print("All format spec tests passed!")
