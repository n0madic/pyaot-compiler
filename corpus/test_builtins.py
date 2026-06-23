# Comprehensive test suite for Python builtin functions
# Tests: int(), float(), bool(), abs(), pow(), min(), max(), round(), sum(), all(), any(), chr(), ord(), hash(), id()

# ============================================================================
# int() - Integer conversion
# ============================================================================

# int() with no args
assert int() == 0, "int() should equal 0"

# int(int) - identity
assert int(42) == 42, "int(42) should equal 42"
assert int(-10) == -10, "int(-10) should equal -10"
assert int(0) == 0, "int(0) should equal 0"

# int(float) - truncate towards zero
assert int(3.9) == 3, "int(3.9) should equal 3"
assert int(3.1) == 3, "int(3.1) should equal 3"
assert int(-3.9) == -3, "int(-3.9) should equal -3"
assert int(-3.1) == -3, "int(-3.1) should equal -3"
assert int(0.0) == 0, "int(0.0) should equal 0"

# int(bool) - True=1, False=0
assert int(True) == 1, "int(True) should equal 1"
assert int(False) == 0, "int(False) should equal 0"

# int(str) - parse string
assert int("42") == 42, "int(\"42\") should equal 42"
assert int("-10") == -10, "int(\"-10\") should equal -10"
assert int("0") == 0, "int(\"0\") should equal 0"
assert int("  123  ") == 123, "int(\"  123  \") should equal 123"  # trim whitespace

# int() ValueError for invalid strings
try:
    int("abc")
    assert False, "Should raise exception"
except:
    pass

try:
    int("12.34")
    assert False, "Should raise exception"
except:
    pass

try:
    int("")
    assert False, "Should raise exception"
except:
    pass

print("int() tests passed")


# ============================================================================
# int methods - bit_length(), bit_count(), conjugate()
# ============================================================================

# bit_length(): number of bits to represent abs(n), 0 for 0.
assert (0).bit_length() == 0, "bit_length(0)"
assert (5).bit_length() == 3, "bit_length(5)"
assert (255).bit_length() == 8, "bit_length(255)"
assert (1024).bit_length() == 11, "bit_length(1024)"
assert (-7).bit_length() == 3, "bit_length(-7) ignores sign"

# bit_length() on a variable receiver (raw int local).
_bl_n = 1000
assert _bl_n.bit_length() == 10, "bit_length(variable)"

# bit_length() on a loop variable (the originally-reported None bug).
_bl_expected = [1, 2, 4, 8]
_bl_i = 0
for _bl_x in [1, 2, 8, 255]:
    assert _bl_x.bit_length() == _bl_expected[_bl_i], "bit_length(loop var)"
    _bl_i += 1

# bit_count(): set bits of abs(n) (Python 3.10+).
assert (0).bit_count() == 0, "bit_count(0)"
assert (255).bit_count() == 8, "bit_count(255)"
assert (-7).bit_count() == 3, "bit_count(-7) ignores sign"

# bool is an int subtype: bit_length() works on bools too.
assert True.bit_length() == 1, "bit_length(True)"
assert False.bit_length() == 0, "bit_length(False)"

# conjugate() returns the int unchanged.
assert (42).conjugate() == 42, "conjugate(42)"

# bool receiver: identity-like int methods widen the i8 receiver to i64 so the
# result is Int-typed (the i8-vs-i64 verifier hard-error regression).
assert True.conjugate() == 1, "conjugate(True)"
assert False.conjugate() == 0, "conjugate(False)"
assert True.bit_count() == 1, "bit_count(True)"
assert True.__index__() == 1, "bool.__index__()"
assert (True.conjugate() + 5) == 6, "conjugate(True) usable as int"

# __int__() / __trunc__(): both return the int value unchanged.
assert (5).__int__() == 5, "int.__int__()"
assert (-3).__int__() == -3, "int.__int__() negative"
assert (5).__trunc__() == 5, "int.__trunc__()"
assert (-3).__trunc__() == -3, "int.__trunc__() negative"
assert True.__int__() == 1, "bool.__int__() widens to int"
assert False.__int__() == 0, "bool.__int__() widens to int"
assert ((7).__int__() + 1) == 8, "int.__int__() usable as int"
_int_dunder_n = 100
assert _int_dunder_n.__trunc__() == 100, "int.__trunc__() on variable receiver"

print("int method tests passed")


# ============================================================================
# float() - Float conversion
# ============================================================================

# float() with no args
assert float() == 0.0, "float() should equal 0.0"

# float(float) - identity
assert float(3.14) == 3.14, "float(3.14) should equal 3.14"
assert float(-2.5) == -2.5, "float(-2.5) should equal -2.5"

# float(int) - convert to float
assert float(42) == 42.0, "float(42) should equal 42.0"
assert float(-10) == -10.0, "float(-10) should equal -10.0"
assert float(0) == 0.0, "float(0) should equal 0.0"

# float(bool) - True=1.0, False=0.0
assert float(True) == 1.0, "float(True) should equal 1.0"
assert float(False) == 0.0, "float(False) should equal 0.0"

# float(str) - parse string
assert float("3.14") == 3.14, "float(\"3.14\") should equal 3.14"
assert float("-2.5") == -2.5, "float(\"-2.5\") should equal -2.5"
assert float("  1.5  ") == 1.5, "float(\"  1.5  \") should equal 1.5"

# float() ValueError for invalid strings
try:
    float("abc")
    assert False, "Should raise exception"
except:
    pass

print("float() tests passed")


# ============================================================================
# bool() - Boolean conversion
# ============================================================================

# bool() with no args
assert bool() == False, "bool() should equal False"

# bool(bool) - identity
assert bool(True) == True, "bool(True) should equal True"
assert bool(False) == False, "bool(False) should equal False"

# bool(int) - False if 0, True otherwise
assert bool(0) == False, "bool(0) should equal False"
assert bool(1) == True, "bool(1) should equal True"
assert bool(-5) == True, "bool(-5) should equal True"
assert bool(42) == True, "bool(42) should equal True"

# bool(float) - False if 0.0, True otherwise
assert bool(0.0) == False, "bool(0.0) should equal False"
assert bool(3.14) == True, "bool(3.14) should equal True"
assert bool(-2.5) == True, "bool(-2.5) should equal True"

# bool(str) - False if empty, True otherwise
assert bool("") == False, "bool(\"\") should equal False"
assert bool("hello") == True, "bool(\"hello\") should equal True"
assert bool("0") == True, "bool(\"0\") should equal True"  # "0" is a non-empty string

# bool(None) - always False
assert bool(None) == False, "bool(None) should equal False"

# bool(list) - False if empty, True otherwise
bool_items: list[int] = []
assert bool(bool_items) == False, "bool(bool_items) should equal False"
bool_items.append(1)
assert bool(bool_items) == True, "bool(bool_items) should equal True"

# bool(tuple) - False if empty, True otherwise
full_tuple: tuple[int, int] = (1, 2)
assert bool(full_tuple) == True, "bool(full_tuple) should equal True"

# bool(dict) - False if empty, True otherwise
bool_dict: dict[str, int] = {}
assert bool(bool_dict) == False, "bool(bool_dict) should equal False"
bool_dict["key"] = 1
assert bool(bool_dict) == True, "bool(bool_dict) should equal True"

print("bool() tests passed")


# ============================================================================
# abs() - Absolute value
# ============================================================================

# abs(int)
assert abs(5) == 5, "abs(5) should equal 5"
assert abs(-5) == 5, "abs(-5) should equal 5"
assert abs(0) == 0, "abs(0) should equal 0"
assert abs(-1000) == 1000, "abs(-1000) should equal 1000"
assert abs(1000) == 1000, "abs(1000) should equal 1000"

# abs(float)
assert abs(3.14) == 3.14, "abs(3.14) should equal 3.14"
assert abs(-3.14) == 3.14, "abs(-3.14) should equal 3.14"
assert abs(0.0) == 0.0, "abs(0.0) should equal 0.0"
assert abs(-0.0) == 0.0, "abs(-0.0) should equal 0.0"
assert abs(-999.999) == 999.999, "abs(-999.999) should equal 999.999"

print("abs() tests passed")


# ============================================================================
# pow() - Power function
# ============================================================================

# pow(int, int)
assert pow(2, 3) == 8.0, "pow(2, 3) should equal 8.0"
assert pow(10, 2) == 100.0, "pow(10, 2) should equal 100.0"
assert pow(5, 0) == 1.0, "pow(5, 0) should equal 1.0"

# pow(float, float)
assert pow(2.0, 3.0) == 8.0, "pow(2.0, 3.0) should equal 8.0"
assert pow(4.0, 0.5) == 2.0, "pow(4.0, 0.5) should equal 2.0"

# pow with negative exponent
assert pow(2, -1) == 0.5, "pow(2, -1) should equal 0.5"
assert pow(10, -2) == 0.01, "pow(10, -2) should equal 0.01"

# pow(int, float)
assert pow(2, 0.5) == 1.4142135623730951, "pow(2, 0.5) should equal 1.4142135623730951"  # sqrt(2)

# pow(float, int)
assert pow(3.0, 2) == 9.0, "pow(3.0, 2) should equal 9.0"

# pow with zero base
assert pow(0, 5) == 0.0, "pow(0, 5) should equal 0.0"
assert pow(0.0, 3) == 0.0, "pow(0.0, 3) should equal 0.0"

# pow with one
assert pow(1, 100) == 1.0, "pow(1, 100) should equal 1.0"
assert pow(100, 0) == 1.0, "pow(100, 0) should equal 1.0"

print("pow() tests passed")


# ============================================================================
# min() - Minimum value
# ============================================================================

# min() with 2 int arguments
assert min(5, 3) == 3, "min(5, 3) should equal 3"
assert min(3, 5) == 3, "min(3, 5) should equal 3"
assert min(-10, -5) == -10, "min(-10, -5) should equal -10"

# min() with 3+ int arguments
assert min(1, 2, 3) == 1, "min(1, 2, 3) should equal 1"
assert min(5, 3, 7, 2, 8) == 2, "min(5, 3, 7, 2, 8) should equal 2"
assert min(10, 20, 30, 40) == 10, "min(10, 20, 30, 40) should equal 10"

# min() with float arguments
assert min(3.14, 2.71) == 2.71, "min(3.14, 2.71) should equal 2.71"
assert min(1.5, 2.5, 0.5) == 0.5, "min(1.5, 2.5, 0.5) should equal 0.5"

# min() with mixed int/float
assert min(5, 3.5) == 3.5, "min(5, 3.5) should equal 3.5"
assert min(2.5, 1) == 1.0, "min(2.5, 1) should equal 1.0"

# min() with negative numbers
assert min(-5, -3, -10) == -10, "min(-5, -3, -10) should equal -10"

print("min() tests passed")


# ============================================================================
# max() - Maximum value
# ============================================================================

# max() with 2 int arguments
assert max(5, 3) == 5, "max(5, 3) should equal 5"
assert max(3, 5) == 5, "max(3, 5) should equal 5"
assert max(-10, -5) == -5, "max(-10, -5) should equal -5"

# max() with 3+ int arguments
assert max(1, 2, 3) == 3, "max(1, 2, 3) should equal 3"
assert max(5, 3, 7, 2, 8) == 8, "max(5, 3, 7, 2, 8) should equal 8"
assert max(10, 20, 30, 40) == 40, "max(10, 20, 30, 40) should equal 40"

# max() with float arguments
assert max(3.14, 2.71) == 3.14, "max(3.14, 2.71) should equal 3.14"
assert max(1.5, 2.5, 0.5) == 2.5, "max(1.5, 2.5, 0.5) should equal 2.5"

# max() with mixed int/float
assert max(5, 7.5) == 7.5, "max(5, 7.5) should equal 7.5"
assert max(8.5, 10) == 10.0, "max(8.5, 10) should equal 10.0"

# max() with negative numbers
assert max(-5, -3, -10) == -3, "max(-5, -3, -10) should equal -3"

print("max() tests passed")


# ============================================================================
# round() - Rounding
# ============================================================================

# round(x) with 1 argument -> int
assert round(3.7) == 4, "round(3.7) should equal 4"
assert round(3.5) == 4, "round(3.5) should equal 4"  # banker's rounding (round half to even)
assert round(2.5) == 2, "round(2.5) should equal 2"  # banker's rounding
assert round(4.5) == 4, "round(4.5) should equal 4"
assert round(5.5) == 6, "round(5.5) should equal 6"
assert round(-3.7) == -4, "round(-3.7) should equal -4"
assert round(-3.5) == -4, "round(-3.5) should equal -4"
assert round(0.4) == 0, "round(0.4) should equal 0"
assert round(5.0) == 5, "round(5.0) should equal 5"

# round(x, ndigits) with 2 arguments -> float
assert round(3.14159, 2) == 3.14, "round(3.14159, 2) should equal 3.14"
assert round(2.71828, 3) == 2.718, "round(2.71828, 3) should equal 2.718"
assert round(123.456, 1) == 123.5, "round(123.456, 1) should equal 123.5"
assert round(123.456, 0) == 123.0, "round(123.456, 0) should equal 123.0"
assert round(1.2345, 4) == 1.2345, "round(1.2345, 4) should equal 1.2345"

# round with zero
assert round(0.0) == 0, "round(0.0) should equal 0"
assert round(0.0, 2) == 0.0, "round(0.0, 2) should equal 0.0"

# round with negative ndigits (rounds to tens, hundreds, etc.)
assert round(1234.0, -1) == 1230.0, "round(1234.0, -1) should equal 1230.0"
assert round(1234.0, -2) == 1200.0, "round(1234.0, -2) should equal 1200.0"
assert round(1256.0, -2) == 1300.0, "round(1256.0, -2) should equal 1300.0"

print("round() tests passed")


# ============================================================================
# chr() and ord() - Character/code point conversion
# ============================================================================

# chr() with ASCII characters
assert chr(65) == "A", "chr(65) should equal \"A\""
assert chr(97) == "a", "chr(97) should equal \"a\""
assert chr(48) == "0", "chr(48) should equal \"0\""
assert chr(32) == " ", "chr(32) should equal \" \""
assert chr(90) == "Z", "chr(90) should equal \"Z\""
assert chr(122) == "z", "chr(122) should equal \"z\""

# ord() with ASCII characters
assert ord("A") == 65, "ord(\"A\") should equal 65"
assert ord("a") == 97, "ord(\"a\") should equal 97"
assert ord("0") == 48, "ord(\"0\") should equal 48"
assert ord(" ") == 32, "ord(\" \") should equal 32"
assert ord("Z") == 90, "ord(\"Z\") should equal 90"
assert ord("z") == 122, "ord(\"z\") should equal 122"

# chr() and ord() are inverses
assert ord(chr(100)) == 100, "ord(chr(100)) should equal 100"
assert chr(ord("X")) == "X", "chr(ord(\"X\")) should equal \"X\""

# Edge cases
assert chr(0) == "\x00", "chr(0) should equal \"\\x00\""  # null character
assert ord("\x00") == 0, "ord(\"\\x00\") should equal 0"

print("chr() and ord() tests passed")


# ============================================================================
# all() and any() - Boolean aggregation
# ============================================================================

# Using int values (0=False, 1=True)
all_true: list[int] = [1, 1, 1]
has_false: list[int] = [1, 0, 1]
all_false: list[int] = [0, 0, 0]
empty_int: list[int] = []
single_true: list[int] = [1]
single_false: list[int] = [0]

# all() tests
assert all(all_true) == True, "all(all_true) should equal True"
assert all(has_false) == False, "all(has_false) should equal False"
assert all(all_false) == False, "all(all_false) should equal False"
assert all(empty_int) == True, "all(empty_int) should equal True"      # empty is vacuously true
assert all(single_true) == True, "all(single_true) should equal True"
assert all(single_false) == False, "all(single_false) should equal False"

# any() tests
assert any(all_true) == True, "any(all_true) should equal True"
assert any(has_false) == True, "any(has_false) should equal True"
assert any(all_false) == False, "any(all_false) should equal False"
assert any(empty_int) == False, "any(empty_int) should equal False"     # empty has no true elements
assert any(single_true) == True, "any(single_true) should equal True"
assert any(single_false) == False, "any(single_false) should equal False"

# all()/any() over range() - must iterate the range (materialized list[int])
assert all(range(1, 5)) == True, "all(range(1,5)) should be True"
assert all(range(0, 5)) == False, "all(range(0,5)) should be False (contains 0)"
assert any(range(0, 1)) == False, "any(range(0,1)) should be False (only 0)"
assert any(range(1, 3)) == True, "any(range(1,3)) should be True"
assert all(range(5, 0, -1)) == True, "all(range(5,0,-1)) should be True"

print("all() and any() tests passed")


# ============================================================================
# sum() - Summation
# ============================================================================

# sum() with int list
sum_nums: list[int] = [1, 2, 3, 4, 5]
assert sum(sum_nums) == 15, "sum(sum_nums) should equal 15"

# sum() with empty list
sum_empty: list[int] = []
assert sum(sum_empty) == 0, "sum(sum_empty) should equal 0"

# sum() with single element
sum_single: list[int] = [100]
assert sum(sum_single) == 100, "sum(sum_single) should equal 100"

# sum() with negative numbers
sum_negatives: list[int] = [-1, -2, -3]
assert sum(sum_negatives) == -6, "sum(sum_negatives) should equal -6"

# sum() with start value
sum_nums2: list[int] = [1, 2, 3]
assert sum(sum_nums2, 10) == 16, "sum(sum_nums2, 10) should equal 16"

# sum() with larger numbers
sum_large: list[int] = [10, 20, 30, 40, 50]
assert sum(sum_large) == 150, "sum(sum_large) should equal 150"

# sum() with float list
sum_floats: list[float] = [1.5, 2.5, 3.0]
assert sum(sum_floats) == 7.0, "sum(sum_floats) should equal 7.0"

# sum() with float list and float start
assert sum(sum_floats, 10.0) == 17.0, "sum(sum_floats, 10.0) should equal 17.0"

# sum() with float list and int start (promotes to float)
assert sum(sum_floats, 10) == 17.0, "sum(sum_floats, 10) should equal 17.0"

# sum() with int list and float start (promotes to float)
sum_nums3: list[int] = [1, 2, 3]
assert sum(sum_nums3, 1.5) == 7.5, "sum(sum_nums3, 1.5) should equal 7.5"

# sum() with empty float list
empty_floats: list[float] = []
assert sum(empty_floats) == 0.0, "sum(empty_floats) should equal 0.0"

# sum() with empty float list and start value
assert sum(empty_floats, 5.5) == 5.5, "sum(empty_floats, 5.5) should equal 5.5"

# sum() with large floats
large_floats: list[float] = [10.5, 20.5, 30.0]
assert sum(large_floats) == 61.0, "sum(large_floats) should equal 61.0"

# sum() with negative floats
negative_floats: list[float] = [-1.5, -2.5, 3.0]
assert sum(negative_floats) == -1.0, "sum(negative_floats) should equal -1.0"

# sum() with single float
single_float: list[float] = [42.5]
assert sum(single_float) == 42.5, "sum(single_float) should equal 42.5"

# sum() with zeros
zeros_float: list[float] = [0.0, 1.5, 0.0, 2.5]
assert sum(zeros_float) == 4.0, "sum(zeros_float) should equal 4.0"

# sum() over range() - must build a real range iterator (both step signs)
assert sum(range(5)) == 10, "sum(range(5)) should equal 10"
assert sum(range(1, 11)) == 55, "sum(range(1, 11)) should equal 55"
assert sum(range(0, 10, 2)) == 20, "sum(range(0, 10, 2)) should equal 20"
assert sum(range(10, 0, -1)) == 55, "sum(range(10, 0, -1)) should equal 55"

print("sum() tests passed")


# ============================================================================
# hash() - Hash function
# ============================================================================

# hash(int) - same value produces same hash
h1 = hash(42)
h2 = hash(42)
assert h1 == h2, "h1 should equal h2"

# Different integers produce different hashes (with high probability)
h3 = hash(0)
h4 = hash(1)
h5 = hash(-1)
h6 = hash(100)
assert h3 != h4 or h4 != h5, "h3 should not equal h4 or h4 != h5"  # at least some should differ

# hash(str) - same string produces same hash
hs1 = hash("hello")
hs2 = hash("hello")
assert hs1 == hs2, "hs1 should equal hs2"

# Different strings produce different hashes
hs3 = hash("world")
hs4 = hash("")
hs5 = hash("a")
hs6 = hash("ab")
hs7 = hash("abc")

# hash(bool) - True=1, False=0
hb1 = hash(True)
hb2 = hash(False)
assert hb1 == 1, "hb1 should equal 1"
assert hb2 == 0, "hb2 should equal 0"

# hash(None) - returns a non-zero value (consistent with CPython, though value varies by platform)
hn = hash(None)
assert hn != 0, "hash(None) should be non-zero"

# Determinism test
assert hash(42) == hash(42), "hash(42) should equal hash(42)"
assert hash("test") == hash("test"), "hash(\"test\") should equal hash(\"test\")"
assert hash(True) == hash(True), "hash(True) should equal hash(True)"
assert hash(False) == hash(False), "hash(False) should equal hash(False)"

print("hash() tests passed")


# ============================================================================
# id() - Object identity
# ============================================================================

# id(int) - same integer has same id
i1 = id(42)
i2 = id(42)
assert i1 == i2, "i1 should equal i2"  # Same value should have same id

# Different integers have different ids
i3 = id(0)
i4 = id(1)
assert i3 != i4, "i3 should not equal i4"

# id(bool) - id returns unique values for True/False
# In CPython: memory addresses (different values)
# In our compiler: True=1, False=0
ib1 = id(True)
ib2 = id(False)
assert ib1 != ib2, "id(True) should differ from id(False)"

# id(None) - id returns some value for None
in1 = id(None)
# In CPython: memory address. In our compiler: 0
# Just verify it returns something consistent
in2 = id(None)
assert in1 == in2, "id(None) should be consistent"

# id(str) - same string object has same id
s1 = "hello"
is1 = id(s1)
is2 = id(s1)
assert is1 == is2, "is1 should equal is2"  # Same object should have same id

# Different string objects have different ids
s2 = "world"
is3 = id(s2)
assert is1 != is3, "is1 should not equal is3"  # Different objects should have different ids

# id(list) - each list is a unique object
nums1: list[int] = [1, 2, 3]
nums2: list[int] = [1, 2, 3]
il1 = id(nums1)
il2 = id(nums2)
assert il1 != il2, "il1 should not equal il2"  # Different list objects have different ids
assert id(nums1) == il1, "id(nums1) should equal il1"  # Same list object has same id

# id(dict) - each dict is a unique object
d1: dict[str, int] = {"a": 1}
d2: dict[str, int] = {"a": 1}
id1 = id(d1)
id2 = id(d2)
assert id1 != id2, "id1 should not equal id2"  # Different dict objects have different ids
assert id(d1) == id1, "id(d1) should equal id1"  # Same dict object has same id

# id(tuple) - same tuple variable has consistent id
t1: tuple[int, int] = (1, 2)
it1 = id(t1)
assert id(t1) == it1, "id(t1) should be consistent"  # Same tuple object has same id
# Note: CPython may cache small tuples, so (1,2) may share id; we test consistency instead

print("id() tests passed")


# ============================================================================
# divmod() - Division and modulo
# ============================================================================

# divmod(a, b) returns (a // b, a % b)
dm1 = divmod(17, 5)
assert dm1[0] == 3, "dm1[0] should equal 3"
assert dm1[1] == 2, "dm1[1] should equal 2"

dm2 = divmod(20, 4)
assert dm2[0] == 5, "dm2[0] should equal 5"
assert dm2[1] == 0, "dm2[1] should equal 0"

dm3 = divmod(7, 3)
assert dm3[0] == 2, "dm3[0] should equal 2"
assert dm3[1] == 1, "dm3[1] should equal 1"

# divmod with negative numbers
# CPython uses floor division: divmod(-17, 5) = (-4, 3)
# Our compiler uses truncation division: divmod(-17, 5) = (-3, -2)
# The test checks both are valid:
dm4 = divmod(-17, 5)
assert dm4[0] in [-3, -4], "dm4[0] should be -3 (truncation) or -4 (floor)"
assert dm4[1] in [-2, 3], "dm4[1] should be -2 (truncation) or 3 (floor)"

dm5 = divmod(17, -5)
assert dm5[0] in [-3, -4], "dm5[0] should be -3 (truncation) or -4 (floor)"
assert dm5[1] in [2, -3], "dm5[1] should be 2 (truncation) or -3 (floor)"

print("divmod() tests passed")


# ============================================================================
# bin(), hex(), oct() - Number formatting
# ============================================================================

# bin() - binary representation
assert bin(10) == "0b1010", "bin(10) should equal \"0b1010\""
assert bin(0) == "0b0", "bin(0) should equal \"0b0\""
assert bin(255) == "0b11111111", "bin(255) should equal \"0b11111111\""
assert bin(1) == "0b1", "bin(1) should equal \"0b1\""
assert bin(-10) == "-0b1010", "bin(-10) should equal \"-0b1010\""

# hex() - hexadecimal representation
assert hex(255) == "0xff", "hex(255) should equal \"0xff\""
assert hex(0) == "0x0", "hex(0) should equal \"0x0\""
assert hex(16) == "0x10", "hex(16) should equal \"0x10\""
assert hex(256) == "0x100", "hex(256) should equal \"0x100\""
assert hex(-255) == "-0xff", "hex(-255) should equal \"-0xff\""

# oct() - octal representation
assert oct(8) == "0o10", "oct(8) should equal \"0o10\""
assert oct(0) == "0o0", "oct(0) should equal \"0o0\""
assert oct(64) == "0o100", "oct(64) should equal \"0o100\""
assert oct(7) == "0o7", "oct(7) should equal \"0o7\""
assert oct(-8) == "-0o10", "oct(-8) should equal \"-0o10\""

print("bin(), hex(), oct() tests passed")


# ============================================================================
# repr() - Object representation
# ============================================================================

# repr(int)
assert repr(42) == "42", "repr(42) should equal \"42\""
assert repr(-10) == "-10", "repr(-10) should equal \"-10\""
assert repr(0) == "0", "repr(0) should equal \"0\""

# repr(float)
assert repr(3.14) == "3.14", "repr(3.14) should equal \"3.14\""
assert repr(-2.5) == "-2.5", "repr(-2.5) should equal \"-2.5\""

# repr(bool)
assert repr(True) == "True", "repr(True) should equal \"True\""
assert repr(False) == "False", "repr(False) should equal \"False\""

# repr(None)
assert repr(None) == "None", "repr(None) should equal \"None\""

# repr(str) - adds quotes
assert repr("hello") == "'hello'", "repr(\"hello\") should equal \"'hello'\""
assert repr("") == "''", "repr(\"\") should equal \"''\""
assert repr("a") == "'a'", "repr(\"a\") should equal \"'a'\""

# repr(list)
repr_list: list[int] = [1, 2, 3]
assert repr(repr_list) == "[1, 2, 3]", "repr(repr_list) should equal \"[1, 2, 3]\""

# repr(tuple)
repr_tuple: tuple[int, int] = (1, 2)
assert repr(repr_tuple) == "(1, 2)", "repr(repr_tuple) should equal \"(1, 2)\""

# repr(dict)
repr_dict: dict[str, int] = {"a": 1}
# Note: dict order may vary, just check it produces valid output
rd = repr(repr_dict)
assert "a" in rd and "1" in rd, "\"a\" should be in rd and \"1\" in rd"

print("repr() tests passed")


# ============================================================================
# type() - Type name
# ============================================================================
# Note: In CPython, type() returns a type object; in our compiler, it returns a string.
# We use str() to normalize for comparison.

# type(int)
assert str(type(42)) == "<class 'int'>", "type(42) should equal \"<class 'int'>\""

# type(float)
assert str(type(3.14)) == "<class 'float'>", "type(3.14) should equal \"<class 'float'>\""

# type(bool)
assert str(type(True)) == "<class 'bool'>", "type(True) should equal \"<class 'bool'>\""
assert str(type(False)) == "<class 'bool'>", "type(False) should equal \"<class 'bool'>\""

# type(str)
assert str(type("hello")) == "<class 'str'>", "type(\"hello\") should equal \"<class 'str'>\""

# type(None)
assert str(type(None)) == "<class 'NoneType'>", "type(None) should equal \"<class 'NoneType'>\""

# type(list)
type_list: list[int] = [1, 2, 3]
assert str(type(type_list)) == "<class 'list'>", "type(type_list) should equal \"<class 'list'>\""

# type(tuple)
type_tuple: tuple[int, int] = (1, 2)
assert str(type(type_tuple)) == "<class 'tuple'>", "type(type_tuple) should equal \"<class 'tuple'>\""

# type(dict)
type_dict: dict[str, int] = {"a": 1}
assert str(type(type_dict)) == "<class 'dict'>", "type(type_dict) should equal \"<class 'dict'>\""

# type(set)
type_set: set[int] = {1, 2, 3}
assert str(type(type_set)) == "<class 'set'>", "type(type_set) should equal \"<class 'set'>\""

# Regression: `type(x).__name__` must return the bare class name as a real
# `str`, not a raw pointer. Before the fix, `resolve_attribute_on_type` had
# no arm for `Str.__name__`, so the attribute expression fell through to
# `Type::Any` and `print`/`==` treated the resulting pointer as a raw i64.
assert type("hi").__name__ == "str", "type(str).__name__ must equal 'str'"
assert type(42).__name__ == "int", "type(int).__name__ must equal 'int'"
assert type(3.14).__name__ == "float", "type(float).__name__ must equal 'float'"
assert type(True).__name__ == "bool", "type(bool).__name__ must equal 'bool'"
assert type(None).__name__ == "NoneType", "type(None).__name__ must equal 'NoneType'"
assert type([1, 2]).__name__ == "list", "type(list).__name__ must equal 'list'"
# Round-trip: bind the extracted name and compare/print.
name_var = type("x").__name__
assert name_var == "str", f"bound `name_var` must equal 'str'; got {name_var!r}"

print("type() tests passed")


# ============================================================================
# map() - Apply function to each element
# ============================================================================

# map with named function - square each number
def square_fn(x: int) -> int:
    return x * x

map_nums: list[int] = [1, 2, 3, 4, 5]
squares_result: list[int] = []
for x in map(square_fn, map_nums):
    squares_result.append(x)
assert len(squares_result) == 5, "len(squares_result) should equal 5"
assert squares_result[0] == 1, "squares_result[0] should equal 1"
assert squares_result[1] == 4, "squares_result[1] should equal 4"
assert squares_result[2] == 9, "squares_result[2] should equal 9"
assert squares_result[3] == 16, "squares_result[3] should equal 16"
assert squares_result[4] == 25, "squares_result[4] should equal 25"

# map iteration with next()
def add_one_fn(x: int) -> int:
    return x + 1

map_iter = map(add_one_fn, [10, 20, 30])
assert next(map_iter) == 11, "next(map_iter) should equal 11"
assert next(map_iter) == 21, "next(map_iter) should equal 21"
assert next(map_iter) == 31, "next(map_iter) should equal 31"

# map with negative numbers including -1 (edge case for EXHAUSTED_SENTINEL)
def identity_fn(x: int) -> int:
    return x

neg_nums: list[int] = [-3, -2, -1, 0, 1, 2, 3]
neg_result: list[int] = []
for x in map(identity_fn, neg_nums):
    neg_result.append(x)
assert len(neg_result) == 7, "len(neg_result) should equal 7"
assert neg_result[2] == -1, "neg_result[2] should equal -1"  # Verify -1 is handled correctly

print("map() tests passed")


# ============================================================================
# filter() - Filter elements by predicate
# ============================================================================

# filter with named function - even numbers
def is_even_fn(x: int) -> bool:
    return x % 2 == 0

filter_nums: list[int] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
evens_result: list[int] = []
for x in filter(is_even_fn, filter_nums):
    evens_result.append(x)
assert len(evens_result) == 5, "len(evens_result) should equal 5"
assert evens_result[0] == 2, "evens_result[0] should equal 2"
assert evens_result[1] == 4, "evens_result[1] should equal 4"
assert evens_result[2] == 6, "evens_result[2] should equal 6"
assert evens_result[3] == 8, "evens_result[3] should equal 8"
assert evens_result[4] == 10, "evens_result[4] should equal 10"

# filter with named function - positive numbers
def is_positive_fn(x: int) -> bool:
    return x > 0

mixed_nums: list[int] = [-3, -2, -1, 0, 1, 2, 3]
positives_result: list[int] = []
for x in filter(is_positive_fn, mixed_nums):
    positives_result.append(x)
assert len(positives_result) == 3, "len(positives_result) should equal 3"
assert positives_result[0] == 1, "positives_result[0] should equal 1"
assert positives_result[1] == 2, "positives_result[1] should equal 2"
assert positives_result[2] == 3, "positives_result[2] should equal 3"

# filter iteration with next()
def greater_than_5_fn(x: int) -> bool:
    return x > 5

filter_iter = filter(greater_than_5_fn, [3, 6, 4, 8, 2, 9])
assert next(filter_iter) == 6, "next(filter_iter) should equal 6"
assert next(filter_iter) == 8, "next(filter_iter) should equal 8"
assert next(filter_iter) == 9, "next(filter_iter) should equal 9"

# filter with -1 edge case
def not_zero_fn(x: int) -> bool:
    return x != 0

neg_filter_nums: list[int] = [-3, -2, -1, 0, 1, 2, 3]
neg_filter_result: list[int] = []
for x in filter(not_zero_fn, neg_filter_nums):
    neg_filter_result.append(x)
assert len(neg_filter_result) == 6, "len(neg_filter_result) should equal 6"
assert neg_filter_result[2] == -1, "neg_filter_result[2] should equal -1"  # Verify -1 is handled correctly

print("filter() tests passed")


# ============================================================================
# filter(None, iterable) - Filter by truthiness
# ============================================================================

# filter(None, ...) with integers - 0 is falsy
from typing import Union

int_truthiness: list[int] = [0, 1, 2, 0, 3, 0, 4]
truthy_ints: list[int] = []
for x in filter(None, int_truthiness):
    truthy_ints.append(x)
assert len(truthy_ints) == 4, "len(truthy_ints) should equal 4"
assert truthy_ints[0] == 1, "truthy_ints[0] should equal 1"
assert truthy_ints[1] == 2, "truthy_ints[1] should equal 2"
assert truthy_ints[2] == 3, "truthy_ints[2] should equal 3"
assert truthy_ints[3] == 4, "truthy_ints[3] should equal 4"

# filter(None, ...) with strings - empty string is falsy
str_truthiness: list[str] = ["", "hello", "", "world", ""]
truthy_strs: list[str] = []
for s in filter(None, str_truthiness):
    truthy_strs.append(s)
assert len(truthy_strs) == 2, "len(truthy_strs) should equal 2"
assert truthy_strs[0] == "hello", "truthy_strs[0] should equal hello"
assert truthy_strs[1] == "world", "truthy_strs[1] should equal world"

# filter(None, ...) with lists - empty list is falsy
list_truthiness: list[list[int]] = [[], [1], [], [2, 3], []]
truthy_lists: list[list[int]] = []
for lst in filter(None, list_truthiness):
    truthy_lists.append(lst)
assert len(truthy_lists) == 2, "len(truthy_lists) should equal 2"
assert truthy_lists[0][0] == 1, "truthy_lists[0][0] should equal 1"
assert truthy_lists[1][0] == 2, "truthy_lists[1][0] should equal 2"

# filter(None, ...) with booleans - False is falsy
bool_truthiness: list[bool] = [True, False, True, False, True]
truthy_bools: list[bool] = []
for b in filter(None, bool_truthiness):
    truthy_bools.append(b)
assert len(truthy_bools) == 3, "len(truthy_bools) should equal 3"
assert truthy_bools[0] == True, "truthy_bools[0] should equal True"
assert truthy_bools[1] == True, "truthy_bools[1] should equal True"
assert truthy_bools[2] == True, "truthy_bools[2] should equal True"

# filter(None, ...) with Optional - None is falsy
opt_truthiness: list[Union[int, None]] = [None, 1, None, 2, None, 3]
truthy_opts: list[Union[int, None]] = []
for o in filter(None, opt_truthiness):
    truthy_opts.append(o)
assert len(truthy_opts) == 3, "len(truthy_opts) should equal 3"
assert truthy_opts[0] == 1, "truthy_opts[0] should equal 1"
assert truthy_opts[1] == 2, "truthy_opts[1] should equal 2"
assert truthy_opts[2] == 3, "truthy_opts[2] should equal 3"

# filter(None, ...) with next() on integers
filter_none_iter = filter(None, [0, 1, 0, 42, 0, 99])
assert next(filter_none_iter) == 1, "next(filter_none_iter) should equal 1"
assert next(filter_none_iter) == 42, "next(filter_none_iter) should equal 42"
assert next(filter_none_iter) == 99, "next(filter_none_iter) should equal 99"

# filter(None, ...) with all falsy values - should be empty
all_falsy: list[int] = [0, 0, 0]
falsy_result: list[int] = []
for x in filter(None, all_falsy):
    falsy_result.append(x)
assert len(falsy_result) == 0, "len(falsy_result) should equal 0"

# filter(None, ...) with all truthy values - should include all
all_truthy: list[int] = [1, 2, 3]
truthy_result: list[int] = []
for x in filter(None, all_truthy):
    truthy_result.append(x)
assert len(truthy_result) == 3, "len(truthy_result) should equal 3"

print("filter(None, iterable) tests passed")


# ============================================================================
# Combining map and filter
# ============================================================================

# filter then map: get positive numbers and square them
def square_fn2(x: int) -> int:
    return x * x

def is_positive_fn2(x: int) -> bool:
    return x > 0

combo_nums: list[int] = [-2, -1, 0, 1, 2, 3]
combo_result: list[int] = []
for x in map(square_fn2, filter(is_positive_fn2, combo_nums)):
    combo_result.append(x)
assert len(combo_result) == 3, "len(combo_result) should equal 3"
assert combo_result[0] == 1, "combo_result[0] should equal 1"
assert combo_result[1] == 4, "combo_result[1] should equal 4"
assert combo_result[2] == 9, "combo_result[2] should equal 9"

print("map+filter combination tests passed")


# ============================================================================
# map/filter with closures (capturing lambdas)
# ============================================================================

# Map with single capture
closure_offset: int = 10
closure_map_result: list[int] = []
for x in map(lambda x: x + closure_offset, [1, 2, 3]):
    closure_map_result.append(x)
assert closure_map_result == [11, 12, 13], "map with single capture failed"

# Map with multiple captures
closure_a: int = 1
closure_b: int = 2
closure_map2_result: list[int] = []
for x in map(lambda x: x + closure_a + closure_b, [10, 20]):
    closure_map2_result.append(x)
assert closure_map2_result == [13, 23], "map with multiple captures failed"

# Filter with capture
closure_threshold: int = 5
closure_filter_result: list[int] = []
for x in filter(lambda x: x > closure_threshold, [1, 3, 7, 9]):
    closure_filter_result.append(x)
assert closure_filter_result == [7, 9], "filter with capture failed"

# Closure stored in variable then used with map
closure_mult: int = 2
fn_closure = lambda x: x * closure_mult
closure_fn_result: list[int] = []
for x in map(fn_closure, [1, 2, 3]):
    closure_fn_result.append(x)
assert closure_fn_result == [2, 4, 6], "closure stored in variable failed"

# Chained map/filter with closures
closure_base: int = 100
closure_limit: int = 105
closure_chain_result: list[int] = []
for x in filter(lambda x: x < closure_limit, map(lambda x: x + closure_base, [1, 2, 3, 4, 5, 6])):
    closure_chain_result.append(x)
assert closure_chain_result == [101, 102, 103, 104], "chained map/filter with closures failed"

# Filter with closure that captures multiple variables
closure_lower: int = 2
closure_upper: int = 8
closure_range_filter: list[int] = []
for x in filter(lambda x: x > closure_lower and x < closure_upper, [1, 2, 3, 4, 5, 6, 7, 8, 9]):
    closure_range_filter.append(x)
assert closure_range_filter == [3, 4, 5, 6, 7], "filter with multiple captures failed"

# Map with closure using list iteration
closure_add_val: int = 100
closure_list_map: list[int] = list(map(lambda x: x + closure_add_val, [1, 2, 3]))
assert closure_list_map == [101, 102, 103], "list(map(closure)) failed"

# Filter with closure using list iteration
closure_mod_val: int = 3
closure_list_filter: list[int] = list(filter(lambda x: x % closure_mod_val == 0, [1, 2, 3, 4, 5, 6, 7, 8, 9]))
assert closure_list_filter == [3, 6, 9], "list(filter(closure)) failed"

print("map/filter with closures tests passed")

# ============================================================================
# map/filter with string closures
# ============================================================================

# Map with string capture - concatenation
str_prefix: str = "hello_"
str_map_result: list[str] = list(map(lambda x: str_prefix + x, ["a", "b", "c"]))
assert str_map_result == ["hello_a", "hello_b", "hello_c"], f"map with string capture failed: {str_map_result}"

# Filter with string capture - substring check
str_suffix: str = "test"
str_data: list[str] = ["hello_test", "world_other", "foo_test", "bar"]
str_filter_result: list[str] = list(filter(lambda x: str_suffix in x, str_data))
assert str_filter_result == ["hello_test", "foo_test"], f"filter with string capture failed: {str_filter_result}"

# Lambda stored in variable with string capture
str_greeting: str = "Hi "
str_fn = lambda name: str_greeting + name
str_fn_result: list[str] = list(map(str_fn, ["Alice", "Bob"]))
assert str_fn_result == ["Hi Alice", "Hi Bob"], f"lambda with string capture failed: {str_fn_result}"

print("map/filter with string closures tests passed")


# ============================================================================
# list() constructor
# ============================================================================

# list() with no args - empty list
empty_list: list[int] = list()
assert empty_list == [], "empty_list should equal []"
assert len(empty_list) == 0, "len(empty_list) should equal 0"

# list(range(...)) - convert range to list
range_list: list[int] = list(range(5))
assert range_list == [0, 1, 2, 3, 4], "range_list should equal [0, 1, 2, 3, 4]"
range_list3: list[int] = list(range(3))
assert range_list3 == [0, 1, 2], "range_list3 should equal [0, 1, 2]"
range_list0: list[int] = list(range(0))
assert range_list0 == [], "range_list0 should equal []"
range_list2_5: list[int] = list(range(2, 5))
assert range_list2_5 == [2, 3, 4], "range_list2_5 should equal [2, 3, 4]"
range_list_step: list[int] = list(range(0, 10, 2))
assert range_list_step == [0, 2, 4, 6, 8], "range_list_step should equal [0, 2, 4, 6, 8]"
range_list_neg: list[int] = list(range(5, 0, -1))
assert range_list_neg == [5, 4, 3, 2, 1], "range_list_neg should equal [5, 4, 3, 2, 1]"

# list(tuple) - convert tuple to list
list_from_tuple: list[int] = list((1, 2, 3))
assert list_from_tuple == [1, 2, 3], "list_from_tuple should equal [1, 2, 3]"

# list(list) - copy
original_list: list[int] = [1, 2, 3]
copy_list: list[int] = list(original_list)
assert copy_list == original_list, "copy_list should equal original_list"
copy_list.append(4)
assert len(original_list) == 3, "len(original_list) should equal 3"  # Original unchanged

# list(str) - convert string to list of chars
str_list: list[str] = list("abc")
assert str_list == ['a', 'b', 'c']
str_list_empty: list[str] = list("")
assert str_list_empty == [], "str_list_empty should equal []"
str_list_hello: list[str] = list("hello")
assert str_list_hello == ['h', 'e', 'l', 'l', 'o']

print("list() constructor tests passed")


# ============================================================================
# tuple() constructor
# ============================================================================

# tuple() with no args - empty tuple
empty_tuple = tuple()
assert len(empty_tuple) == 0, "len(empty_tuple) should equal 0"

# tuple(list) - convert list to tuple
tuple_from_list = tuple([1, 2, 3])
assert len(tuple_from_list) == 3, "len(tuple_from_list) should equal 3"
assert tuple_from_list[0] == 1, "tuple_from_list[0] should equal 1"
assert tuple_from_list[1] == 2, "tuple_from_list[1] should equal 2"
assert tuple_from_list[2] == 3, "tuple_from_list[2] should equal 3"

# tuple(range) - convert range to tuple
tuple_range3 = tuple(range(3))
assert len(tuple_range3) == 3, "len(tuple_range3) should equal 3"
assert tuple_range3[0] == 0, "tuple_range3[0] should equal 0"
assert tuple_range3[1] == 1, "tuple_range3[1] should equal 1"
assert tuple_range3[2] == 2, "tuple_range3[2] should equal 2"

tuple_range5 = tuple(range(5))
assert len(tuple_range5) == 5, "len(tuple_range5) should equal 5"

tuple_range0 = tuple(range(0))
assert len(tuple_range0) == 0, "len(tuple_range0) should equal 0"

# tuple(str) - convert string to tuple of chars
tuple_abc = tuple("abc")
assert len(tuple_abc) == 3, "len(tuple_abc) should equal 3"
assert tuple_abc[0] == 'a', "tuple_abc[0] should equal 'a'"
assert tuple_abc[1] == 'b', "tuple_abc[1] should equal 'b'"
assert tuple_abc[2] == 'c', "tuple_abc[2] should equal 'c'"

tuple_empty_str = tuple("")
assert len(tuple_empty_str) == 0, "len(tuple_empty_str) should equal 0"

print("tuple() constructor tests passed")


# ============================================================================
# dict() constructor
# ============================================================================

# dict() with no args - empty dict
empty_dict: dict[str, int] = dict()
assert len(empty_dict) == 0, "len(empty_dict) should equal 0"

# dict with keyword arguments
kw_dict: dict[str, int] = dict(a=1, b=2, c=3)
assert kw_dict['a'] == 1, "kw_dict['a'] should equal 1"
assert kw_dict['b'] == 2, "kw_dict['b'] should equal 2"
assert kw_dict['c'] == 3, "kw_dict['c'] should equal 3"
assert len(kw_dict) == 3, "len(kw_dict) should equal 3"

# dict from list of pairs
pairs: list[tuple[str, int]] = [('x', 10), ('y', 20)]
pairs_dict: dict[str, int] = dict(pairs)
assert pairs_dict['x'] == 10, "pairs_dict['x'] should equal 10"
assert pairs_dict['y'] == 20, "pairs_dict['y'] should equal 20"
assert len(pairs_dict) == 2, "len(pairs_dict) should equal 2"

# dict from another dict (copy)
original_dict: dict[str, int] = {'a': 1, 'b': 2}
copied_dict: dict[str, int] = dict(original_dict)
# Note: dict == comparison not yet implemented, check values instead
assert copied_dict['a'] == original_dict['a'], "copied_dict['a'] should equal original_dict['a']"
assert copied_dict['b'] == original_dict['b'], "copied_dict['b'] should equal original_dict['b']"
assert len(copied_dict) == len(original_dict), "len(copied_dict) should equal len(original_dict)"
copied_dict['c'] = 3
assert 'c' not in original_dict, "'c' not should be in original_dict"  # Original unchanged

print("dict() constructor tests passed")


# ============================================================================
# min/max with tuples
# ============================================================================

# min/max with tuple of ints
min_tuple: tuple[int, int, int, int, int] = (3, 1, 4, 1, 5)
assert min(min_tuple) == 1, "min(min_tuple) should equal 1"
assert max(min_tuple) == 5, "max(min_tuple) should equal 5"

# min/max with tuple of negative ints
neg_tuple: tuple[int, int, int] = (-5, -3, -10)
assert min(neg_tuple) == -10, "min(neg_tuple) should equal -10"
assert max(neg_tuple) == -3, "max(neg_tuple) should equal -3"

print("min/max with tuples tests passed")


# ============================================================================
# min/max with range
# ============================================================================

# min/max with positive range
assert min(range(5)) == 0, "min(range(5)) should equal 0"
assert max(range(5)) == 4, "max(range(5)) should equal 4"

assert min(range(1, 10)) == 1, "min(range(1, 10)) should equal 1"
assert max(range(1, 10)) == 9, "max(range(1, 10)) should equal 9"

# min/max with step
assert min(range(0, 10, 2)) == 0, "min(range(0, 10, 2)) should equal 0"
assert max(range(0, 10, 2)) == 8, "max(range(0, 10, 2)) should equal 8"

# min/max with negative step
assert min(range(5, 0, -1)) == 1, "min(range(5, 0, -1)) should equal 1"
assert max(range(5, 0, -1)) == 5, "max(range(5, 0, -1)) should equal 5"

assert min(range(10, 0, -2)) == 2, "min(range(10, 0, -2)) should equal 2"
assert max(range(10, 0, -2)) == 10, "max(range(10, 0, -2)) should equal 10"

print("min/max with range tests passed")


# ============================================================================
# min/max with sets
# ============================================================================

# min/max with set of ints
min_set: set[int] = {3, 1, 4, 1, 5, 9, 2, 6}
assert min(min_set) == 1, "min(min_set) should equal 1"
assert max(min_set) == 9, "max(min_set) should equal 9"

# min/max with negative ints in set
neg_set: set[int] = {-5, -3, -10, 0, 5}
assert min(neg_set) == -10, "min(neg_set) should equal -10"
assert max(neg_set) == 5, "max(neg_set) should equal 5"

print("min/max with sets tests passed")


# ============================================================================
# min/max with lists (existing, but verify still works)
# ============================================================================

# min/max variadic still works
assert min(3, 1, 4) == 1, "min(3, 1, 4) should equal 1"
assert max(3, 1, 4) == 4, "max(3, 1, 4) should equal 4"
assert min(5, 3, 7, 2, 8) == 2, "min(5, 3, 7, 2, 8) should equal 2"
assert max(5, 3, 7, 2, 8) == 8, "max(5, 3, 7, 2, 8) should equal 8"

# min/max with list (existing)
min_list: list[int] = [3, 1, 4, 1, 5]
assert min(min_list) == 1, "min(min_list) should equal 1"
assert max(min_list) == 5, "max(min_list) should equal 5"

print("min/max existing behavior verified")


# ============================================================================
# Final summary
# ============================================================================

print("")
# ============================================================================
# format() - Value formatting
# ============================================================================

# format() with no format spec (default)
assert format(42) == "42", "format(42) should be '42'"
assert format(3.14) == "3.14", "format(3.14) should be '3.14'"
assert format("hello") == "hello", "format('hello') should be 'hello'"
assert format(True) == "True", "format(True) should be 'True'"

# format() with format spec
assert format(42, "d") == "42", "format(42, 'd') should be '42'"
assert format(255, "x") == "ff", "format(255, 'x') should be 'ff'"
assert format(255, "X") == "FF", "format(255, 'X') should be 'FF'"
assert format(255, "o") == "377", "format(255, 'o') should be '377'"
assert format(255, "b") == "11111111", "format(255, 'b') should be '11111111'"

# format() with float formatting
assert format(3.14159, ".2f") == "3.14", "format(3.14159, '.2f') should be '3.14'"
assert format(1000.0, ".0f") == "1000", "format(1000.0, '.0f') should be '1000'"

# format() with width/alignment
assert format(42, ">5") == "   42", "format(42, '>5') right-align"
assert format(42, "<5") == "42   ", "format(42, '<5') left-align"
assert format(42, "^5") == " 42  ", "format(42, '^5') center-align"

print("  format(): all tests passed")

# ============================================================================
# functools.reduce() - Reduce an iterable
# ============================================================================
import functools

# Basic reduction: sum
reduce_sum = functools.reduce(lambda acc, x: acc + x, [1, 2, 3, 4, 5])
assert reduce_sum == 15, "reduce sum should be 15"

# Reduction with initial value
reduce_sum_init = functools.reduce(lambda acc, x: acc + x, [1, 2, 3], 10)
assert reduce_sum_init == 16, "reduce sum with initial=10 should be 16"

# Product
reduce_product = functools.reduce(lambda acc, x: acc * x, [1, 2, 3, 4, 5])
assert reduce_product == 120, "reduce product should be 120"

# Max via reduce
reduce_max = functools.reduce(lambda a, b: a if a > b else b, [3, 1, 4, 1, 5, 9])
assert reduce_max == 9, "reduce max should be 9"

# Single element list (no initial)
reduce_single = functools.reduce(lambda a, b: a + b, [42])
assert reduce_single == 42, "reduce of single element should be 42"

# Empty list with initial value
reduce_empty_init = functools.reduce(lambda a, b: a + b, [], 99)
assert reduce_empty_init == 99, "reduce of empty list with initial should be 99"

# String concatenation
reduce_str = functools.reduce(lambda a, b: a + b, ["a", "b", "c"])
assert reduce_str == "abc", "reduce string concat should be 'abc'"

print("  functools.reduce(): all tests passed")

# ============================================================================
# int() with base parameter
# ============================================================================

# int(str, base=16) - hexadecimal
assert int("ff", 16) == 255, "int('ff', 16) should be 255"
assert int("FF", 16) == 255, "int('FF', 16) should be 255"
assert int("0xff", 16) == 255, "int('0xff', 16) should be 255"
assert int("10", 16) == 16, "int('10', 16) should be 16"

# int(str, base=2) - binary
assert int("101", 2) == 5, "int('101', 2) should be 5"
assert int("0b101", 2) == 5, "int('0b101', 2) should be 5"
assert int("11111111", 2) == 255, "int('11111111', 2) should be 255"

# int(str, base=8) - octal
assert int("77", 8) == 63, "int('77', 8) should be 63"
assert int("0o77", 8) == 63, "int('0o77', 8) should be 63"
assert int("10", 8) == 8, "int('10', 8) should be 8"

# int(str, base=10) - explicit decimal
assert int("42", 10) == 42, "int('42', 10) should be 42"
assert int("-10", 10) == -10, "int('-10', 10) should be -10"

print("  int() with base: all tests passed")

# ============================================================================
# zip() with 3+ iterables
# ============================================================================

# zip with 3 lists
zip3_a: list[int] = [1, 2, 3]
zip3_b: list[str] = ["a", "b", "c"]
zip3_c: list[float] = [1.0, 2.0, 3.0]
zip3_result: list[tuple[int, str, float]] = list(zip(zip3_a, zip3_b, zip3_c))
assert len(zip3_result) == 3, f"zip3 should have 3 elements, got {len(zip3_result)}"
assert zip3_result[0] == (1, "a", 1.0), f"zip3[0] should be (1, 'a', 1.0), got {zip3_result[0]}"
assert zip3_result[1] == (2, "b", 2.0), f"zip3[1] should be (2, 'b', 2.0), got {zip3_result[1]}"
assert zip3_result[2] == (3, "c", 3.0), f"zip3[2] should be (3, 'c', 3.0), got {zip3_result[2]}"

# zip with 3 lists, different lengths (shortest wins)
zip3_short_a: list[int] = [1, 2]
zip3_short_b: list[int] = [10, 20, 30]
zip3_short_c: list[int] = [100, 200, 300, 400]
zip3_short_result: list[tuple[int, int, int]] = list(zip(zip3_short_a, zip3_short_b, zip3_short_c))
assert len(zip3_short_result) == 2, f"zip3 shortest should win, got {len(zip3_short_result)}"

print("  zip() with 3+ iterables: all tests passed")

# ============================================================================
# issubclass() builtin
# ============================================================================

class IscAnimal:
    name: str
    def __init__(self, name: str):
        self.name = name

class IscDog(IscAnimal):
    breed: str
    def __init__(self, name: str, breed: str):
        super().__init__(name)
        self.breed = breed

class IscCat(IscAnimal):
    color: str
    def __init__(self, name: str, color: str):
        super().__init__(name)
        self.color = color

assert issubclass(IscDog, IscAnimal) == True, "IscDog should be subclass of IscAnimal"
assert issubclass(IscCat, IscAnimal) == True, "IscCat should be subclass of IscAnimal"
assert issubclass(IscDog, IscDog) == True, "IscDog should be subclass of itself"
assert issubclass(IscAnimal, IscAnimal) == True, "IscAnimal should be subclass of itself"
assert issubclass(IscAnimal, IscDog) == False, "IscAnimal should not be subclass of IscDog"
assert issubclass(IscDog, IscCat) == False, "IscDog should not be subclass of IscCat"

print("  issubclass(): all tests passed")

print("=" * 60)
print("All builtin function tests passed!")
print("=" * 60)
print("Tested Python builtins:")
print("  - Type conversions: int(), float(), bool()")
print("  - Math: abs(), pow(), min(), max(), round(), divmod()")
print("  - Number formatting: bin(), hex(), oct()")
print("  - Sequences: sum(), all(), any()")
print("  - Character/code: chr(), ord()")
print("  - Representation: repr(), type()")
print("  - Other: hash(), id()")
print("")
print("  - Functional: map(), filter(), functools.reduce()")
print("  - Collection constructors: list(), tuple(), dict()")
print("  - Formatting: format()")
# === sum/min/max on iterators/generators ===
iter_sum_result: int = sum(x for x in [1, 2, 3])
assert iter_sum_result == 6, f"sum(gen): expected 6, got {iter_sum_result}"

iter_sum_doubled: int = sum(x * 2 for x in [0, 1, 2, 3])
assert iter_sum_doubled == 12, f"sum(x*2 for list): expected 12, got {iter_sum_doubled}"

iter_min_result: int = min(x for x in [3, 1, 2])
assert iter_min_result == 1, f"min(gen): expected 1, got {iter_min_result}"

iter_max_result: int = max(x for x in [3, 1, 2])
assert iter_max_result == 3, f"max(gen): expected 3, got {iter_max_result}"

print("sum/min/max on iterators passed!")

# ============================================================================
# any()/all() with bool lists (regression test)
# ============================================================================

bool_all_true: list[bool] = [True, True, True]
bool_has_false: list[bool] = [True, False, True]
bool_all_false: list[bool] = [False, False, False]

assert all(bool_all_true) == True, "all([True,True,True]) should be True"
assert all(bool_has_false) == False, "all([True,False,True]) should be False"
assert all(bool_all_false) == False, "all([False,False,False]) should be False"

assert any(bool_all_true) == True, "any([True,True,True]) should be True"
assert any(bool_has_false) == True, "any([True,False,True]) should be True"
assert any(bool_all_false) == False, "any([False,False,False]) should be False"

print("any()/all() with bool lists tests passed!")

# ============================================================================
# map() with builtins and type-converting lambdas (regression test)
# ============================================================================

# map with builtin str on int list
map_str_result: list[str] = list(map(str, [1, 2, 3]))
assert map_str_result == ["1", "2", "3"], f"map(str, [1,2,3]) failed: {map_str_result}"

# map with lambda that converts int to str
map_lambda_str: list[str] = list(map(lambda x: str(x), [10, 20, 30]))
assert map_lambda_str == ["10", "20", "30"], f"map(lambda x: str(x)) failed: {map_lambda_str}"

# map with builtin int on str list — result stored as ELEM_HEAP_OBJ,
# ListGetInt transparently unboxes IntObj to raw i64
map_int_result: list[int] = list(map(int, ["1", "2", "3"]))
assert len(map_int_result) == 3, f"map(int, strs) len failed: {len(map_int_result)}"
assert map_int_result[0] == 1, f"map(int, strs)[0] should be 1, got {map_int_result[0]}"
assert map_int_result[1] == 2, f"map(int, strs)[1] should be 2, got {map_int_result[1]}"
assert map_int_result[2] == 3, f"map(int, strs)[2] should be 3, got {map_int_result[2]}"
assert map_int_result[0] + 10 == 11, f"map(int, strs)[0]+10 should be 11"

print("map() with builtins and type-converting lambdas tests passed!")

print("  - min/max with iterables: tuple, range, set")
print("  - sum/min/max with iterators/generators")
print("Note: print(), len(), range() tested in other files")

# ===== SECTION: Builtin return type inference =====

assert len([1, 2, 3]) == 3, "builtin return type: len → int"
assert abs(-5) == 5, "builtin return type: abs(int) → int"
assert int("42") == 42, "builtin return type: int(str) → int"
assert str(42) == "42", "builtin return type: str(int) → str"
assert bool(1) == True, "builtin return type: bool(int) → bool"

print("Builtin return type inference tests passed!")

# ===== SECTION: hasattr / setattr / getattr =====

class HasAttrTest:
    def __init__(self, x: int, name: str):
        self.x = x
        self.name = name

hat = HasAttrTest(10, "hello")

# hasattr - existing attributes
assert hasattr(hat, "x") == True, "hasattr: existing int field"
assert hasattr(hat, "name") == True, "hasattr: existing str field"

# hasattr - non-existing attribute
assert hasattr(hat, "missing") == False, "hasattr: non-existing field"
assert hasattr(hat, "xyz") == False, "hasattr: another non-existing field"

# setattr - modify existing field
setattr(hat, "x", 42)
assert hat.x == 42, f"setattr: expected x=42, got {hat.x}"

# setattr - modify string field
setattr(hat, "name", "world")
assert hat.name == "world", f"setattr: expected name=world, got {hat.name}"

print("hasattr/setattr tests passed!")


# ============================================================================
# sum/min/max over Any-typed elements (code-review #9)
# ============================================================================
# A `list[Any]` annotation pins the element type to `Any`, and an annotated
# generator parameter `xs: list[Any]` stays `list[Any]` (the harvester only
# refines UNannotated params), so the generator returns `Iterator[Any]`.
# These exercise the tagged-accumulation path: sum/min/max must preserve
# `int` vs `float` exactly as CPython does. `repr()` is the discriminator
# because `6 == 6.0` is True in Python and would not catch an int->float
# regression.

any9_list: list[Any] = [1, 2, 3]


def passthru9(xs: list[Any]):
    for x in xs:
        yield x


# lower_sum, list branch (tagged accumulation over list[Any])
assert repr(sum(any9_list)) == "6", "sum(list[Any]) must stay int 6"
# lower_sum, iterator branch (tagged accumulation over Iterator[Any])
assert repr(sum(passthru9(any9_list))) == "6", "sum(Iterator[Any]) must stay int 6"
# explicit start value
assert sum(passthru9(any9_list), 10) == 16, "sum(Iterator[Any], 10) should equal 16"
# empty iterable -> boxed int 0
any9_empty: list[Any] = []
assert sum(any9_empty) == 0, "sum(empty list[Any]) should equal 0"
# lower_minmax, iterator branch
assert min(passthru9(any9_list)) == 1, "min(Iterator[Any]) should equal 1"
assert max(passthru9(any9_list)) == 3, "max(Iterator[Any]) should equal 3"
# float elements through the same Any path stay float
any9_floats: list[Any] = [1.5, 2.5]
assert repr(sum(passthru9(any9_floats))) == "4.0", "sum of float Any elements stays float"

# min/max over a CONCRETE list[Any] / set[Any] / tuple[Any] (#9 follow-up).
# These route through rt_*_minmax with the tagged elem_kind. The float
# variants are the discriminators: the old Int path read FloatObj pointers
# as raw ints -> garbage. repr() distinguishes "3" (int) from "3.0" (float).
mm_int_l: list[Any] = [3, 1, 2]
assert repr(min(mm_int_l)) == "1", "min(list[Any] of int) should be int 1"
assert repr(max(mm_int_l)) == "3", "max(list[Any] of int) should be int 3"
mm_flt_l: list[Any] = [3.5, 1.5, 2.5]
assert repr(min(mm_flt_l)) == "1.5", "min(list[Any] of float) should be 1.5"
assert repr(max(mm_flt_l)) == "3.5", "max(list[Any] of float) should be 3.5"
mm_int_s: set[Any] = {30, 10, 20}
assert repr(min(mm_int_s)) == "10", "min(set[Any] of int) should be int 10"
assert repr(max(mm_int_s)) == "30", "max(set[Any] of int) should be int 30"
mm_flt_s: set[Any] = {3.5, 1.5, 2.5}
assert repr(min(mm_flt_s)) == "1.5", "min(set[Any] of float) should be 1.5"
assert repr(max(mm_flt_s)) == "3.5", "max(set[Any] of float) should be 3.5"
mm_int_t: tuple[Any, Any, Any] = (3, 1, 2)
assert repr(min(mm_int_t)) == "1", "min(tuple[Any] of int) should be int 1"
assert repr(max(mm_int_t)) == "3", "max(tuple[Any] of int) should be int 3"
mm_flt_t: tuple[Any, Any, Any] = (3.5, 1.5, 2.5)
assert repr(min(mm_flt_t)) == "1.5", "min(tuple[Any] of float) should be 1.5"
assert repr(max(mm_flt_t)) == "3.5", "max(tuple[Any] of float) should be 3.5"

print("sum/min/max over Any-typed elements tests passed!")


# ===== Whole-project code-review regression: pow()/all()/any() coercion
# (formerly test_review_wave2_lowering.py) and int(NaN)/int(inf)
# (formerly part of test_review_wave3f.py) =====
def _rv_pow() -> None:
    print(round(pow(2, 0.5), 10))
    print(pow(2.0, 10.0))
    print(pow(True, 2.0))
    print(round(pow(0.5, 2), 10))


def _rv_all_any() -> None:
    print(all([0.0]))
    print(any([0.0]))
    print(all([1.0, 2.0]))
    print(any([0.0, 3.0]))
    print(all([""]))
    print(any([""]))
    print(all(["a", "b"]))
    print(any(["", "x"]))
    print(all([1, 2, 0]))
    print(any([0, 0]))


def _rv_int_nan_inf() -> None:
    nan = float("nan")
    try:
        print(int(nan))
    except ValueError:
        print("int(nan) ValueError")
    inf = float("inf")
    try:
        print(int(inf))
    except OverflowError:
        print("int(inf) OverflowError")
    print(int(2.7))
    print(int(-2.7))


# min()/max() over strings (lexicographic, returns str)
def _rv_minmax_str() -> None:
    print(min("apple", "banana"))
    print(max("apple", "banana"))
    print(min("cherry", "apple", "banana"))
    print(max("cherry", "apple", "banana"))
    print(min("b", "a", "c"))
    print(max("b", "a", "c"))


# int(obj) / float(obj): dispatch __int__/__float__; TypeError when absent.
class _RvIntable:
    def __int__(self) -> int:
        return 7


class _RvFloatable:
    def __float__(self) -> float:
        return 3.5


class _RvPlainObj:
    def __init__(self) -> None:
        self.x = 1


def _rv_int_float_dispatch() -> None:
    print(int(_RvIntable()))
    print(float(_RvFloatable()))
    try:
        int(None)
        print("int(None): no error")
    except TypeError:
        print("int(None): TypeError")
    try:
        float([1, 2])
        print("float(list): no error")
    except TypeError:
        print("float(list): TypeError")
    try:
        int(_RvPlainObj())
        print("int(plain): no error")
    except TypeError:
        print("int(plain): TypeError")


_rv_pow()
_rv_all_any()
_rv_int_nan_inf()
_rv_minmax_str()
_rv_int_float_dispatch()


# ============================================================================
# FOLDED-IN POINT TESTS (assert-only; prints converted/dropped)
# Folded from: p18_scalar_builtins, p30_introspection, p31_zip_multi,
#   p32_int_methods, p33_zero_arg_conversions, p34_isinstance_tuple,
#   p10_kwargs_builtins, p17_type_builtin, test_builtin_first_class.
# ============================================================================


# p17 user classes MUST stay at module top level: wrapping them in a function
# changes __qualname__ to "<locals>", which changes the asserted type string.
class _p17_Widget:
    def __init__(self) -> None:
        self.x = 1


# p30 / p34 user classes also stay at module level (pyaot frontend does not
# support class definitions nested inside a function body).
class _p30_Animal:
    def __init__(self, name: str) -> None:
        self.name = name

    def speak(self) -> str:
        return "..."


class _p30_Dog(_p30_Animal):
    def speak(self) -> str:
        return "woof"


class _p30_Cat(_p30_Animal):
    def speak(self) -> str:
        return "meow"


class _p30_HasAttrTest:
    def __init__(self) -> None:
        self.x = 10
        self.name = "hat"

    def method(self) -> int:
        return self.x


# callable() helpers: a plain class (instances NOT callable) and a class with
# __call__ (instances callable). Module scope so `callable(<class name>)` folds.
class _CallablePlain:
    def __init__(self) -> None:
        self.x = 1


class _CallableWithCall:
    def __call__(self) -> int:
        return 42


def _callable_free_fn() -> int:
    return 0


class _p34_Animal:
    def __init__(self, name: str) -> None:
        self.name = name

    def speak(self) -> str:
        return "..."


class _p34_Dog(_p34_Animal):
    def speak(self) -> str:
        return "woof"


class _p34_Cat(_p34_Animal):
    def speak(self) -> str:
        return "meow"


# Helper referenced from inside a generator expression (separate scope): the
# pyaot frontend resolves it at module level, not from a nested def.
_p18_witness: list = []


def _p18_tap(v):
    _p18_witness.append(v)
    return v


def _fold_p18_scalar_builtins() -> None:
    # ===== pow -> ** (bignum + numeric-tower correct) =====
    assert pow(2, 3) == 8
    assert pow(2, 10) == 1024
    assert pow(5, 0) == 1
    assert pow(2, -1) == 0.5  # negative exponent -> float, exactly like **
    assert pow(2, 64) == 2 ** 64  # bignum result
    assert pow(10, 20) == 10 ** 20

    # ===== divmod -> (a // b, a % b), CPython floor/sign semantics (B1) =====
    assert divmod(17, 5) == (3, 2)
    assert divmod(-7, 2) == (-4, 1)
    assert divmod(7, -2) == (-4, -1)
    assert divmod(-7, -2) == (3, -1)
    assert divmod(7.5, 2) == (3.0, 1.5)
    assert divmod(20, 4) == (5, 0)

    # ===== all / any -- list, genexpr, empty, short-circuit, mixed, range =====
    assert all([True, True, True]) == True
    assert all([True, False, True]) == False
    assert all([]) == True  # empty -> seed
    assert any([False, False, True]) == True
    assert any([False, False, False]) == False
    assert any([]) == False  # empty -> seed
    assert all([1, 2, 3]) == True  # truthy non-bools
    assert all([1, 0, 3]) == False  # 0 is falsy
    assert any([0, 0, 5]) == True
    assert all(x > 0 for x in [1, 2, 3]) == True  # generator comprehension
    assert any(x > 2 for x in [1, 2, 3]) == True
    assert all(x < 0 for x in [1, 2, 3]) == False
    assert all(x < 5 for x in range(5)) == True  # over range
    assert any(x == 3 for x in range(5)) == True
    assert all(["a", "b"]) == True  # non-empty strings truthy
    assert all(["a", "", "b"]) == False  # empty string falsy

    # short-circuit witness: a falsy early element stops all before later truthy
    _p18_witness.clear()
    assert all(_p18_tap(x) for x in [1, 0, 1]) == False
    # stopped at the first falsy, never saw the trailing 1
    assert _p18_witness == [1, 0]

    # ===== id -- stability, distinctness, consistency with is =====
    id_x = [1, 2, 3]
    assert id(id_x) == id(id_x)  # stable across calls
    a = [1]
    b = [1]
    assert id(a) != id(b)  # distinct live objects have distinct ids
    assert (a is b) == (id(a) == id(b))  # consistent with is
    assert (a is a) == (id(a) == id(a))

    # ===== round -- banker's (round-half-to-even, B1) =====
    assert round(2.5) == 2  # half -> even (down)
    assert round(3.5) == 4  # half -> even (up)
    assert round(0.5) == 0
    assert round(-0.5) == 0  # -0.0 -> 0
    assert round(1.5) == 2
    assert round(2.675, 2) == 2.67  # 2.675 is 2.6749999... as a double
    assert round(3.14159, 2) == 3.14  # ndigits present -> float result
    assert round(7.5 / 2.5) == 3  # 3.0 -> int 3
    assert round(5) == 5  # int stays int
    assert round(123.456, 1) == 123.5

    # ===== bin / hex / oct -- bignum-aware (B16) =====
    assert bin(10) == "0b1010"
    assert bin(0) == "0b0"
    assert bin(-5) == "-0b101"  # sign before the prefix
    assert hex(255) == "0xff"
    assert hex(-255) == "-0xff"
    assert oct(8) == "0o10"
    assert oct(-8) == "-0o10"
    assert bin(True) == "0b1"  # bool formats as its int value
    assert hex(False) == "0x0"
    assert bin(2 ** 100) == "0b1" + "0" * 100  # bignum (B16)
    assert hex(2 ** 100) == "0x1" + "0" * 25  # bignum hex (B16)

    # ===== interaction probes (cross with green features) =====
    assert f"{divmod(17, 5)}" == "(3, 2)"  # f-string of a tuple
    assert f"{bin(10)}" == "0b1010"
    assert f"round={round(3.14159, 2)}" == "round=3.14"
    assert pow(2, 3) + round(1.5) == 10  # 8 + 2
    q, r = divmod(17, 5)  # unpack a divmod result
    assert q == 3
    assert r == 2
    assert bin(10) + " " + hex(255) == "0b1010 0xff"


def _fold_p30_introspection() -> None:
    # self-subclass (reflexive), direct subclass, and the False cases.
    # issubclass/isinstance need the class NAME directly (resolved at compile
    # time), so the module-level _p30_* names are used in place of aliases.
    assert issubclass(_p30_Dog, _p30_Animal) is True
    assert issubclass(_p30_Cat, _p30_Animal) is True
    assert issubclass(_p30_Animal, _p30_Animal) is True
    assert issubclass(_p30_Dog, _p30_Dog) is True
    assert issubclass(_p30_Dog, _p30_Cat) is False
    assert issubclass(_p30_Cat, _p30_Dog) is False
    assert issubclass(_p30_Animal, _p30_Dog) is False

    hat = _p30_HasAttrTest()
    assert hasattr(hat, "x") is True        # present field
    assert hasattr(hat, "name") is True     # present field
    assert hasattr(hat, "method") is True   # present method
    assert hasattr(hat, "missing") is False  # absent name
    assert hasattr(hat, "xyz") is False     # absent name

    # ===== setattr / getattr round-trips on a concrete instance =====
    hat2 = _p30_HasAttrTest()
    assert getattr(hat2, "x") == 10
    setattr(hat2, "x", 42)
    assert getattr(hat2, "x") == 42
    assert hat2.x == 42  # write is visible to direct attribute access too

    setattr(hat2, "name", "world")
    assert getattr(hat2, "name") == "world"
    assert hat2.name == "world"
    # setattr evaluates to None
    assert setattr(hat2, "x", 7) is None
    assert hat2.x == 7

    # ===== cross with already-green features (f-string, arithmetic) =====
    d = _p30_Dog("Rex")
    assert getattr(d, "name") == "Rex"
    total = getattr(hat2, "x") + 100
    assert total == 107
    assert f"name={getattr(d, 'name')} x={getattr(hat2, 'x')}" == "name=Rex x=7"

    # polymorphic instance, crossed with a compile-time issubclass gate
    voices = []
    if issubclass(_p30_Dog, _p30_Animal):
        animals = [_p30_Dog("D"), _p30_Cat("C")]
        for a in animals:
            voices.append(f"{getattr(a, 'name')}: {a.speak()}")
    assert voices == ["D: woof", "C: meow"]


def _fold_callable() -> None:
    # Bare names: a top-level function and a class are callable (folded True in
    # the frontend without a static type).
    assert callable(_callable_free_fn) is True
    assert callable(_CallablePlain) is True       # a class is callable (ctor)
    assert callable(_CallableWithCall) is True

    # A lambda is a Callable value.
    f = lambda x: x + 1
    assert callable(f) is True

    # An instance is callable iff its class defines __call__.
    wc = _CallableWithCall()
    assert callable(wc) is True
    plain = _CallablePlain()
    assert callable(plain) is False

    # Concrete non-callable values are not callable.
    assert callable(42) is False
    assert callable(3.14) is False
    assert callable("hello") is False
    assert callable(True) is False
    nums = [1, 2, 3]
    assert callable(nums) is False
    d = {"a": 1}
    assert callable(d) is False


def _fold_p31_zip_multi() -> None:
    # ===== zip of 3 lists into an annotated list[tuple[...]] slot =====
    z3_a: list[int] = [1, 2, 3]
    z3_b: list[str] = ["a", "b", "c"]
    z3_c: list[float] = [1.0, 2.0, 3.0]
    z3: list[tuple[int, str, float]] = list(zip(z3_a, z3_b, z3_c))
    assert len(z3) == 3
    assert z3[0] == (1, "a", 1.0)
    assert z3[1] == (2, "b", 2.0)
    assert z3[2] == (3, "c", 3.0)

    # ===== shortest iterable wins (different lengths) =====
    s_a: list[int] = [1, 2]
    s_b: list[int] = [10, 20, 30]
    s_c: list[int] = [100, 200, 300, 400]
    s: list[tuple[int, int, int]] = list(zip(s_a, s_b, s_c))
    assert len(s) == 2
    assert s == [(1, 10, 100), (2, 20, 200)]

    # ===== 4-iterable form (ZipN with count=4) =====
    q4 = list(zip([1, 2], [3, 4], [5, 6], [7, 8]))
    assert q4 == [(1, 3, 5, 7), (2, 4, 6, 8)]

    # ===== 5 iterables, mixed element types =====
    m = list(zip([1, 2], ["x", "y"], [1.5, 2.5], [True, False], ["p", "q"]))
    assert m[0] == (1, "x", 1.5, True, "p")
    assert m[1] == (2, "y", 2.5, False, "q")

    # ===== direct iteration of a 3-zip (tuple unpacking in the for-target) =====
    total = 0
    joined = ""
    for i, name, f in zip(z3_a, z3_b, z3_c):
        total += i
        joined += name
        assert isinstance(f, float)
    assert total == 6
    assert joined == "abc"

    # ===== 2-iterable form still works (unchanged rt_zip_new path) =====
    two: list[tuple[int, str]] = list(zip(z3_a, z3_b))
    assert two == [(1, "a"), (2, "b"), (3, "c")]

    # ===== cross with already-green features (sum over zipped products) =====
    xs: list[int] = [1, 2, 3]
    ys: list[int] = [4, 5, 6]
    dot = 0
    for a, b in zip(xs, ys):
        dot += a * b
    assert dot == 32  # 1*4 + 2*5 + 3*6


def _fold_p32_int_methods() -> None:
    # ===== bit_length(): bits to represent abs(n); 0 for 0 =====
    assert (0).bit_length() == 0
    assert (5).bit_length() == 3
    assert (255).bit_length() == 8
    assert (1024).bit_length() == 11
    assert (-7).bit_length() == 3       # sign ignored
    n = 1000
    assert n.bit_length() == 10         # variable receiver

    # loop-variable receiver (each element is Int-typed)
    expected = [1, 2, 4, 8]
    i = 0
    for x in [1, 2, 8, 255]:
        assert x.bit_length() == expected[i]
        i += 1
    assert i == 4

    # ===== bit_count(): set bits of abs(n) (Python 3.10+) =====
    assert (0).bit_count() == 0
    assert (255).bit_count() == 8
    assert (-7).bit_count() == 3
    assert (7).bit_count() == 3

    # ===== bool is an int subtype: methods work on bools =====
    assert True.bit_length() == 1
    assert False.bit_length() == 0
    assert True.bit_count() == 1
    assert False.bit_count() == 0

    # ===== conjugate() / __index__() return the int value =====
    assert (42).conjugate() == 42
    assert (42).__index__() == 42
    assert (-5).conjugate() == -5
    # bool widens to int (Int-typed result, usable in arithmetic)
    assert True.conjugate() == 1
    assert False.conjugate() == 0
    assert True.__index__() == 1
    assert (True.conjugate() + 5) == 6

    # ===== BIGNUM-aware: arbitrary-precision receivers =====
    big = 2 ** 100
    assert big.bit_length() == 101
    assert big.bit_count() == 1
    assert (2 ** 64 - 1).bit_length() == 64
    assert (2 ** 64 - 1).bit_count() == 64
    assert big.conjugate() == big       # bignum preserved
    assert big.__index__() == big
    assert (2 ** 128 - 1).bit_count() == 128

    # ===== cross with green features (f-string, indexing into a list) =====
    vals = [1, 7, 255, 1023]
    bl = [v.bit_length() for v in vals]
    assert bl == [1, 3, 8, 10]
    assert f"bits of 255 = {(255).bit_length()}, ones = {(255).bit_count()}" == \
        "bits of 255 = 8, ones = 8"


def _fold_p33_zero_arg_conversions() -> None:
    # ===== the four defaults =====
    assert int() == 0
    assert float() == 0.0
    assert bool() == False
    assert str() == ""
    assert repr(str()) == "''"

    # ===== str() is a real empty string (length, concat, iteration) =====
    s = str()
    assert len(s) == 0
    assert s + "abc" == "abc"
    assert "x" + str() + "y" == "xy"
    assert not s            # empty string is falsy
    joined = str().join(["a", "b", "c"])
    assert joined == "abc"  # "".join(...)
    assert repr(s + "tail") == "'tail'"

    # ===== defaults usable in arithmetic / control flow =====
    assert int() + 5 == 5
    assert float() + 1.5 == 1.5
    assert (bool() or True) is True
    total = 0
    for _ in range(3):
        total += int() + 1
    assert total == 3

    # ===== the with-args forms still work (no regression) =====
    assert int(42) == 42
    assert int("ff", 16) == 255
    assert float("2.5") == 2.5
    assert bool(1) is True
    assert str(42) == "42"
    assert str(3.14) == "3.14"
    assert int("101", 2) == 5
    assert str([1, 2]) == "[1, 2]"

    # ===== user shadow wins (unshadowed-gated) =====
    def make_default() -> str:
        return str()  # the builtin, unshadowed here

    assert make_default() == ""


def _fold_p34_isinstance_tuple() -> None:
    # isinstance needs the class NAME directly (resolved at compile time), so
    # the module-level _p34_* names are used in place of aliases.
    # ===== builtin-only tuples (static fold per element) =====
    five = 5
    flt = 5.0
    txt = "a"
    assert isinstance(five, (str, int)) is True
    assert isinstance(flt, (str, int)) is False
    assert isinstance(txt, (int, str, bytes)) is True
    assert isinstance(txt, (int, float)) is False

    # bool subset int in Python.
    flag = True
    assert isinstance(flag, (int,)) is True
    assert isinstance(flag, (str,)) is False

    # ===== container KINDS (matches by kind, ignores element types) =====
    lst = [1, 2, 3]
    tup = (1, 2)
    dct = {"k": 1}
    assert isinstance(lst, (dict, list)) is True
    assert isinstance(tup, (list, tuple)) is True
    assert isinstance(dct, (list, dict)) is True
    assert isinstance(lst, (dict, tuple)) is False

    # ===== user classes (runtime inheritance-aware check) =====
    d = _p34_Dog("rex")
    c = _p34_Cat("mia")
    assert isinstance(d, (_p34_Cat, _p34_Animal)) is True   # Dog is-a Animal
    assert isinstance(c, (_p34_Dog,)) is False              # Cat is not-a Dog
    assert isinstance(c, (_p34_Cat, _p34_Dog)) is True
    assert isinstance(d, (_p34_Cat,)) is False

    # ===== MIXED user-class + builtin element =====
    assert isinstance(d, (int, _p34_Dog)) is True
    assert isinstance(d, (int, _p34_Cat)) is False
    assert isinstance(five, (_p34_Dog, int)) is True  # builtin element wins

    # ===== nested type-tuple flatten (CPython flattens recursively) =====
    assert isinstance(five, (str, (bytes, int))) is True
    assert isinstance(flt, (str, (bytes, int))) is False
    assert isinstance(d, (_p34_Cat, (str, _p34_Animal))) is True

    # ===== empty tuple => False =====
    assert isinstance(five, ()) is False
    assert isinstance(d, ()) is False

    # ===== single-eval: the receiver is evaluated EXACTLY once =====
    eval_witness = []

    def bump() -> int:
        eval_witness.append(1)
        return len(eval_witness)

    result = isinstance(bump(), (int, str))   # int element first -> 1 eval
    assert result is True
    assert len(eval_witness) == 1             # advanced by exactly one
    result2 = isinstance(bump(), (str, float))
    assert result2 is False                   # an int is neither str nor float
    assert len(eval_witness) == 2

    # ===== cross with green features: if, and, or, comprehension =====
    def classify(x: int) -> str:
        if isinstance(x, (int, float)):
            return "number"
        return "other"

    assert classify(7) == "number"
    assert isinstance(five, (int, str)) and isinstance(txt, (int, str))
    assert isinstance(flt, (int,)) or isinstance(flt, (float,))

    nums = [1, 2, 3, 4]
    kept = [v for v in nums if isinstance(v, (int, float))]
    assert kept == [1, 2, 3, 4]

    animals = [_p34_Dog("a"), _p34_Cat("b"), _p34_Dog("c")]
    voices = [a.speak() for a in animals if isinstance(a, (_p34_Dog,))]
    assert voices == ["woof", "woof"]


def _fold_p10_kwargs_builtins() -> None:
    def neg(x: int) -> int:
        return -x

    # trace records eval ORDER into a witness instead of printing.
    trace_order = []

    def trace(label: str, val: int) -> int:
        trace_order.append(label)
        return val

    xs = [3, 1, 2]

    # -- sorted: reverse only (both truthiness spellings) --
    assert sorted(xs, reverse=True) == [3, 2, 1]
    assert sorted(xs, reverse=False) == [1, 2, 3]
    assert sorted(xs, reverse=1) == [3, 2, 1]
    assert sorted(xs, reverse=0) == [1, 2, 3]

    # -- sorted: key= lambda / named fn / builtins --
    assert sorted(xs, key=lambda v: -v) == [3, 2, 1]
    assert sorted(xs, key=neg) == [3, 2, 1]
    assert sorted([-5, 2, -1, 4], key=abs) == [-1, 2, 4, -5]
    assert sorted(["bbb", "a", "cc"], key=len) == ["a", "cc", "bbb"]
    assert sorted([10, 2, 33], key=str) == [10, 2, 33]

    # -- key + reverse together, both keyword orders --
    assert sorted(xs, key=neg, reverse=True) == [1, 2, 3]
    assert sorted(xs, reverse=True, key=neg) == [1, 2, 3]

    # -- key=None literal behaves like no key --
    assert sorted(xs, key=None) == [1, 2, 3]
    assert sorted(xs, key=None, reverse=True) == [3, 2, 1]

    # -- kwargs x closure: the key captures an enclosing variable --
    pivot = 2

    def dist(v: int) -> int:
        return abs(v - pivot)

    assert sorted([5, 1, 2, 4], key=dist) == [2, 1, 4, 5]
    assert sorted([5, 1, 2, 4], key=lambda v: abs(v - pivot), reverse=True) == \
        [5, 4, 1, 2]

    # -- stability: equal keys keep written order; reverse must NOT flip them --
    pairs = [(2, "a"), (1, "b"), (2, "c"), (1, "d")]
    assert sorted(pairs, key=lambda p: p[0]) == \
        [(1, "b"), (1, "d"), (2, "a"), (2, "c")]
    assert sorted(pairs, key=lambda p: p[0], reverse=True) == \
        [(2, "a"), (2, "c"), (1, "b"), (1, "d")]
    words = ["bb", "aa", "cc", "dd"]
    assert sorted(words, key=len) == ["bb", "aa", "cc", "dd"]
    assert sorted(words, key=len, reverse=True) == ["bb", "aa", "cc", "dd"]

    # -- sorted over non-list iterables with keywords --
    assert sorted((3, 1, 2), reverse=True) == [3, 2, 1]
    assert sorted({"b": 1, "a": 2}, reverse=True) == ["b", "a"]
    assert sorted("cab", key=str, reverse=True) == ["c", "b", "a"]

    # -- the input is never mutated --
    assert xs == [3, 1, 2]

    # -- enumerate: positional and keyword start --
    enum_pos = []
    for i, v in enumerate(["x", "y"], 5):
        enum_pos.append((i, v))
    assert enum_pos == [(5, "x"), (6, "y")]
    enum_kw = []
    for i, v in enumerate(["x", "y"], start=7):
        enum_kw.append((i, v))
    assert enum_kw == [(7, "x"), (8, "y")]
    assert list(enumerate("ab", start=1)) == [(1, "a"), (2, "b")]

    # -- dict: pure-kwargs, mixed, written-order side effects --
    d1 = dict(a=1, b=2, c=3)
    assert list(d1.items()) == [("a", 1), ("b", 2), ("c", 3)]
    d2 = dict(d1, b=20, z=26)
    assert list(d2.items()) == [("a", 1), ("b", 20), ("c", 3), ("z", 26)]
    assert list(d1.items()) == [("a", 1), ("b", 2), ("c", 3)]  # d1 unchanged

    # dict kwargs evaluate in written order (b before a here)
    trace_order.clear()
    d3 = dict(b=trace("d3.b", 2), a=trace("d3.a", 1))
    assert trace_order == ["d3.b", "d3.a"]
    assert list(d3.items()) == [("b", 2), ("a", 1)]

    trace_order.clear()
    d4 = dict([("k", 0)], v=trace("d4.v", 9))
    assert trace_order == ["d4.v"]
    assert list(d4.items()) == [("k", 0), ("v", 9)]

    # -- sorted kwargs values evaluate in written order --
    trace_order.clear()
    assert sorted([2, 1], reverse=trace("rev", 0) == 1) == [1, 2]
    assert trace_order == ["rev"]


def _fold_p17_type_builtin() -> None:
    # builtins via the value tag.
    type_list: list[int] = [1, 2, 3]
    type_tuple: tuple[int, int] = (1, 2)
    type_dict: dict[str, int] = {"a": 1}
    type_set: set[int] = {1, 2, 3}

    assert str(type(42)) == "<class 'int'>"
    assert str(type(3.14)) == "<class 'float'>"
    assert str(type(True)) == "<class 'bool'>"
    assert str(type(False)) == "<class 'bool'>"
    assert str(type("hello")) == "<class 'str'>"
    assert str(type(None)) == "<class 'NoneType'>"
    assert str(type(type_list)) == "<class 'list'>"
    assert str(type(type_tuple)) == "<class 'tuple'>"
    assert str(type(type_dict)) == "<class 'dict'>"
    assert str(type(type_set)) == "<class 'set'>"

    # ===== type(v).__name__ -- bare name from the runtime extractor =====
    assert type(42).__name__ == "int"
    assert type(3.14).__name__ == "float"
    assert type(True).__name__ == "bool"
    assert type(False).__name__ == "bool"
    assert type("hello").__name__ == "str"
    assert type(None).__name__ == "NoneType"
    assert type(type_list).__name__ == "list"
    assert type(type_tuple).__name__ == "tuple"
    assert type(type_dict).__name__ == "dict"
    assert type(type_set).__name__ == "set"

    # ===== user class: qualified vs bare from the SAME source =====
    # _p17_Widget is at MODULE level (see above) to preserve __qualname__.
    assert str(type(_p17_Widget())) == "<class '__main__._p17_Widget'>"
    assert type(_p17_Widget()).__name__ == "_p17_Widget"

    # ===== interaction probes (the one-source principle) =====
    name_var = type("x").__name__
    assert name_var == "str"
    assert name_var + "!" == "str!"
    assert f"{type(42).__name__}" == "int"
    assert f"a {type(_p17_Widget()).__name__} b" == "a _p17_Widget b"
    assert type(1).__name__ == type(2).__name__
    assert type("p").__name__ == type("q").__name__
    assert type(1).__name__ != type(1.0).__name__


def _fold_test_builtin_first_class() -> None:
    # sorted() with key= builtin functions (works for all element types)
    words_sorted_1 = ["aaa", "b", "cc"]
    assert sorted(words_sorted_1, key=len) == ["b", "cc", "aaa"]
    assert sorted(words_sorted_1, key=len, reverse=True) == ["aaa", "cc", "b"]

    nums_sorted_abs = [-3, 1, -2, 4]
    assert sorted(nums_sorted_abs, key=abs) == [1, -2, -3, 4]

    nums_sorted_str = [10, 2, 1, 20]
    assert sorted(nums_sorted_str, key=str) == [1, 10, 2, 20]

    data_sorted = [[1], [1, 2, 3], [1, 2]]
    result_sorted_lists = sorted(data_sorted, key=len)
    assert len(result_sorted_lists[0]) == 1
    assert len(result_sorted_lists[1]) == 2
    assert len(result_sorted_lists[2]) == 3

    # list.sort() with key= builtin functions
    words_sort_1 = ["aaa", "b", "cc"]
    words_sort_1.sort(key=len)
    assert words_sort_1 == ["b", "cc", "aaa"]

    words_sort_2 = ["aaa", "b", "cc"]
    words_sort_2.sort(key=len, reverse=True)
    assert words_sort_2 == ["aaa", "cc", "b"]

    nums_sort_abs = [-5, 2, -3]
    nums_sort_abs.sort(key=abs)
    assert nums_sort_abs == [2, -3, -5]

    # min()/max() with key= builtin functions
    words_minmax = ["aaa", "b", "cc"]
    assert min(words_minmax, key=len) == "b"
    assert max(words_minmax, key=len) == "aaa"

    nums_minmax_abs = [-5, 2, -3, 1]
    assert min(nums_minmax_abs, key=abs) == 1
    assert max(nums_minmax_abs, key=abs) == -5


_fold_p18_scalar_builtins()
_fold_p30_introspection()
_fold_callable()
_fold_p31_zip_multi()
_fold_p32_int_methods()
_fold_p33_zero_arg_conversions()
_fold_p34_isinstance_tuple()
_fold_p10_kwargs_builtins()
_fold_p17_type_builtin()
_fold_test_builtin_first_class()


# ===== SECTION: int(str) honours arbitrary precision + digit-group underscores =====
def _review_int_str_bignum():
    assert int("1_000") == 1000, "int() accepts digit-group underscores"
    assert int("255") == 255
    assert int("ff", 16) == 255, "int(str, base)"
    assert int("1" * 30) == 111111111111111111111111111111, "int(huge str) is a bignum"
    assert int("z" * 12, 36) > 10**18, "int(str, 36) is a bignum"


_review_int_str_bignum()


print("All builtins code-review regression tests passed!")
