# Zero-argument type-conversion builtins `int()` / `float()` / `bool()` / `str()`.
#
# CPython gives each a default: `int() == 0`, `float() == 0.0`, `bool() == False`,
# `str() == ""`. The unary `rt_builtin_*` take one argument, so a no-arg call must
# NOT reach them (that builds an arity-mismatched, invalid Cranelift call). They
# fold here: `int`/`float`/`bool` to a default constant in lowering; `str()` to a
# `""` literal in the FRONTEND (interning lives there — lowering's interner is
# immutable). The one/two-arg forms are unaffected and keep their runtime paths.
#
# `==` asserts are the spec (Principle 9); prints feed the differential harness.


# ===== the four defaults =====
assert int() == 0
assert float() == 0.0
assert bool() == False
assert str() == ""
print(int(), float(), bool(), repr(str()))  # 0 0.0 False ''


# ===== str() is a real empty string (length, concat, iteration) =====
s = str()
assert len(s) == 0
assert s + "abc" == "abc"
assert "x" + str() + "y" == "xy"
assert not s            # empty string is falsy
joined = str().join(["a", "b", "c"])
assert joined == "abc"  # "".join(...)
print(repr(s + "tail"))  # 'tail'


# ===== defaults usable in arithmetic / control flow =====
assert int() + 5 == 5
assert float() + 1.5 == 1.5
assert (bool() or True) is True
total = 0
for _ in range(3):
    total += int() + 1
assert total == 3
print(total)            # 3


# ===== the with-args forms still work (no regression) =====
assert int(42) == 42
assert int("ff", 16) == 255
assert float("2.5") == 2.5
assert bool(1) is True
assert str(42) == "42"
assert str(3.14) == "3.14"
print(int("101", 2), str([1, 2]))  # 5 [1, 2]


# ===== user shadow wins (unshadowed-gated) =====
def make_default() -> str:
    return str()  # the builtin, unshadowed here


assert make_default() == ""
print(repr(make_default()))  # ''

print("All zero-arg conversion tests passed!")
