# Consolidated test file for core types and operators

# ===== SECTION: Print functionality =====

# Print single integer
print(42)

# Print single float
print(3.14)

# Print single bool
print(True)
print(False)

# Print None
print(None)

# Print string
print("Hello, World!")

# Print multiple values of same type
print(1, 2, 3)

# Print multiple values of different types
print(42, 3.14, True, "hello")

# Print expression results
x_print: int = 10
y_print: int = 20
print(x_print + y_print)

# Print in a loop
for i in range(3):
    print(i)

# Nested print (result of expression with function call)
def add_for_print(a: int, b: int) -> int:
    return a + b

print(add_for_print(5, 7))

# ===== SECTION: Arithmetic operators =====

def add(a: int, b: int) -> int:
    return a + b

def multiply(x: int, y: int) -> int:
    return x * y

# Test arithmetic operations
result1: int = add(10, 20)
assert result1 == 30, "add(10, 20) should be 30"

result2: int = multiply(5, 6)
assert result2 == 30, "multiply(5, 6) should be 30"

total: int = result1 + result2
assert total == 60, "30 + 30 should be 60"

# Integer division and modulo
int_a: int = 15
int_b: int = 4
assert int_a // int_b == 3, "15 // 4 should be 3"
assert int_a % int_b == 3, "15 % 4 should be 3"
assert 17 // 5 == 3, "17 // 5 should be 3"
assert 17 % 5 == 2, "17 % 5 should be 2"

# Float operations
float_x: float = 7.5
float_y: float = 2.5
assert float_x + float_y == 10.0, "7.5 + 2.5 should be 10.0"
assert float_x - float_y == 5.0, "7.5 - 2.5 should be 5.0"
assert float_x * float_y == 18.75, "7.5 * 2.5 should be 18.75"
assert float_x / float_y == 3.0, "7.5 / 2.5 should be 3.0"
assert round(float_x / float_y) == 3, "round(7.5 / 2.5) should be 3"

# String multiplication
str_s: str = "Hello"
assert str_s * 3 == "HelloHelloHello", "str * 3 should repeat string"
assert "ab" * 2 == "abab", "'ab' * 2 should be 'abab'"

# String multiplication edge cases
assert "x" * 0 == "", "str * 0 should be empty"
assert "hello" * 0 == "", "multi-char str * 0 should be empty"
assert "x" * -1 == "", "str * negative should be empty"

# ===== SECTION: Comparison operators =====

x: int = 15
y: int = 10

assert x > y, "15 > 10 should be True"
assert not (x == y), "15 == 10 should be False"
assert y <= x, "10 <= 15 should be True"
assert x >= y, "15 >= 10 should be True"
assert x != y, "15 != 10 should be True"

# ===== SECTION: Identity operators (is/is not) =====

# Test 'is' with heap types (lists)
list1: list[int] = [1, 2, 3]
list2: list[int] = [1, 2, 3]
list3: list[int] = list1

assert list1 is list3, "list1 is list3 should be True (same object)"
assert list1 is not list2, "list1 is not list2 should be True (different objects)"
assert not (list1 is list2), "list1 is list2 should be False"
assert not (list1 is not list3), "list1 is not list3 should be False"

# Test 'is' with strings
str1: str = "hello"
str2: str = "hello"
str3: str = str1

assert str1 is str3, "str1 is str3 should be True (same object)"
# Note: String interning is not implemented, so identical string literals may be different objects

# Test 'is' with None
none1: None = None
none2: None = None
assert none1 is none2, "None is None should be True"
assert none1 is not list1, "None is not list should be True"

# Test 'is not' with dictionaries
dict1: dict[str, int] = {"a": 1}
dict2: dict[str, int] = {"a": 1}
dict3: dict[str, int] = dict1

assert dict1 is dict3, "dict1 is dict3 should be True (same object)"
assert dict1 is not dict2, "dict1 is not dict2 should be True (different objects)"

# ===== SECTION: Logical operators =====

z: int = 5

assert x > y and y > z, "15 > 10 and 10 > 5 should be True"
assert x > y or y < z, "15 > 10 or 10 < 5 should be True (first part true)"
assert not (x < y), "not (15 < 10) should be True"

# Complex boolean expressions
assert x > 10 and y < 20, "15 > 10 and 10 < 20 should be True"
assert x > 10 and y < 20 or z == 5, "complex expression should be True"

# Short-circuit evaluation: and/or return values, not just booleans
# Test 'and': returns right if left is truthy, else returns left
sc_and_1: int = 5 and 10
assert sc_and_1 == 10, "5 and 10 should return 10"

sc_and_2: int = 0 and 10
assert sc_and_2 == 0, "0 and 10 should return 0"

sc_and_3: int = 42 and 99
assert sc_and_3 == 99, "42 and 99 should return 99"

# Test 'or': returns left if left is truthy, else returns right
sc_or_1: int = 5 or 10
assert sc_or_1 == 5, "5 or 10 should return 5"

sc_or_2: int = 0 or 10
assert sc_or_2 == 10, "0 or 10 should return 10"

sc_or_3: int = 42 or 99
assert sc_or_3 == 42, "42 or 99 should return 42"

# Test with strings (important for method dispatch)
sc_str_or: str = "" or "default"
assert sc_str_or == "default", "empty string or 'default' should return 'default'"

sc_str_and: str = "hello" and "world"
assert sc_str_and == "world", "'hello' and 'world' should return 'world'"

# Test method calls on short-circuit results
sc_method: str = ("" or "hello").upper()
assert sc_method == "HELLO", "method call on short-circuit result should work"

# Test with None
sc_none_or_1: int = 0 or 42
assert sc_none_or_1 == 42, "None or 42 should return 42"

sc_none_and_1: int = 42 and 0
assert sc_none_and_1 == 0, "42 and None should return None"

# Unary negation
a: int = 42
neg_a: int = -a
assert neg_a == -42, "-42 should equal -42"
assert neg_a + a == 0, "-42 + 42 should be 0"

# Double negation
b: int = 5
neg_b: int = -b
pos_b: int = -neg_b
assert pos_b == b, "-(-5) should equal 5"

# Not operator with boolean values
flag1: bool = True
flag2: bool = False

assert not (not flag1), "not not True should be True"
assert not flag2, "not False should be True"
assert flag1 or flag2, "True or False should be True"
assert flag1 and not flag2, "True and not False should be True"

# ===== SECTION: Power operator =====

# Integer power operations
pow_x: int = 2 ** 3
assert pow_x == 8, "pow_x should equal 8"

pow_y: int = 3 ** 4
assert pow_y == 81, "pow_y should equal 81"

pow_z: int = 5 ** 0
assert pow_z == 1, "pow_z should equal 1"

pow_w: int = 2 ** 10
assert pow_w == 1024, "pow_w should equal 1024"

# Negative base
pow_a: int = (-2) ** 3
assert pow_a == -8, "pow_a should equal -8"

pow_b: int = (-2) ** 4
assert pow_b == 16, "pow_b should equal 16"

# Float power operations
f1: float = 2.0 ** 3.0
assert f1 == 8.0, "f1 should equal 8.0"

f2: float = 4.0 ** 0.5
assert f2 == 2.0, "f2 should equal 2.0"

f3: float = 2.5 ** 2.0
assert f3 == 6.25, "f3 should equal 6.25"

# Mixed int/float (should use float)
m1: float = 2 ** 3.0
assert m1 == 8.0, "m1 should equal 8.0"

m2: float = 2.0 ** 3
assert m2 == 8.0, "m2 should equal 8.0"

# Edge cases
e1: int = 1 ** 100
assert e1 == 1, "e1 should equal 1"

e2: int = 0 ** 5
assert e2 == 0, "e2 should equal 0"

e3: int = 10 ** 1
assert e3 == 10, "e3 should equal 10"

# Larger values
big: int = 2 ** 20
assert big == 1048576, "big should equal 1048576"

# Additional power tests from test_operators.py
assert 2 ** 3 == 8, "2 ** 3 should be 8"
assert 3 ** 2 == 9, "3 ** 2 should be 9"
assert 2 ** 0 == 1, "2 ** 0 should be 1"

# ===== SECTION: Bitwise operators =====

# Test bitwise AND (&)
bit_a: int = 0b1100  # 12
bit_b: int = 0b1010  # 10
bit_result: int = bit_a & bit_b
assert bit_result == 0b1000, "bitwise AND failed"  # 8
assert (15 & 7) == 7, "bitwise AND with literals failed"

# Test bitwise OR (|)
bit_result = bit_a | bit_b
assert bit_result == 0b1110, "bitwise OR failed"  # 14
assert (8 | 4) == 12, "bitwise OR with literals failed"

# Test bitwise XOR (^)
bit_result = bit_a ^ bit_b
assert bit_result == 0b0110, "bitwise XOR failed"  # 6
assert (12 ^ 10) == 6, "bitwise XOR with literals failed"

# Test left shift (<<)
bit_x: int = 1
bit_result = bit_x << 4
assert bit_result == 16, "left shift failed"
assert (3 << 2) == 12, "left shift with literals failed"

# Test right shift (>>)
bit_y: int = 16
bit_result = bit_y >> 2
assert bit_result == 4, "right shift failed"
assert (32 >> 3) == 4, "right shift with literals failed"

# Test bitwise NOT (~)
bit_z: int = 0
bit_result = ~bit_z
assert bit_result == -1, "bitwise NOT of 0 failed"

bit_result = ~(-1)
assert bit_result == 0, "bitwise NOT of -1 failed"

n: int = 5  # 0b0101
bit_result = ~n
assert bit_result == -6, "bitwise NOT of 5 failed"  # ~5 = -6 in two's complement

# Test combined operations
# (a & b) | (a ^ b) should equal (a | b)
combined: int = (bit_a & bit_b) | (bit_a ^ bit_b)
assert combined == (bit_a | bit_b), "combined bitwise operations failed"

# Test chained shifts
val: int = 1
val = val << 3  # 8
val = val << 1  # 16
assert val == 16, "chained left shifts failed"

val = val >> 2  # 4
assert val == 4, "right shift after left shifts failed"

# Test with negative numbers
neg: int = -8
bit_result = neg >> 2
assert bit_result == -2, "arithmetic right shift of negative failed"  # -8 / 4 = -2 (sign-extended)

# Test masking patterns (common use case)
byte: int = 255  # 0xFF
mask: int = 0x0F  # lower 4 bits
lower: int = byte & mask
assert lower == 15, "lower nibble extraction failed"

upper: int = (byte >> 4) & mask
assert upper == 15, "upper nibble extraction failed"

# Test bit setting/clearing patterns
flags: int = 0
flags = flags | (1 << 0)  # set bit 0
assert flags == 1, "set bit 0 failed"
flags = flags | (1 << 2)  # set bit 2
assert flags == 5, "set bit 2 failed"  # 0b101 = 5

# Check bit 1 is not set
bit1: int = (flags >> 1) & 1
assert bit1 == 0, "bit 1 should not be set"

# Check bit 2 is set
bit2: int = (flags >> 2) & 1
assert bit2 == 1, "bit 2 should be set"

# Clear bit 0 using AND with inverted mask
flags = flags & ~(1 << 0)
assert flags == 4, "clear bit 0 failed"  # 0b100 = 4

# ===== SECTION: Chained comparisons =====

# Basic chained comparison: 1 < x < 10
chain_x: int = 5
assert 1 < chain_x < 10, "1 < 5 < 10 should be True"
assert not (10 < chain_x < 20), "10 < 5 < 20 should be False"

# Chained comparison with variables
chain_a: int = 1
chain_b: int = 2
chain_c: int = 3
assert chain_a < chain_b < chain_c, "1 < 2 < 3 should be True"
assert not (chain_c < chain_b < chain_a), "3 < 2 < 1 should be False"

# Equality chains: a == b == c
eq_a: int = 5
eq_b: int = 5
eq_c: int = 5
assert eq_a == eq_b == eq_c, "5 == 5 == 5 should be True"

eq_d: int = 6
assert not (eq_a == eq_b == eq_d), "5 == 5 == 6 should be False"

# Mixed operators: 1 < 2 <= 2 < 3
assert 1 < 2 <= 2 < 3, "1 < 2 <= 2 < 3 should be True"
assert 1 <= 1 < 2 <= 2, "1 <= 1 < 2 <= 2 should be True"

# Four-way chains
assert 1 < 2 < 3 < 4, "1 < 2 < 3 < 4 should be True"
assert not (1 < 2 < 3 < 2), "1 < 2 < 3 < 2 should be False"

# Float comparisons in chains
f_a: float = 1.0
f_b: float = 2.5
f_c: float = 5.0
assert f_a < f_b < f_c, "1.0 < 2.5 < 5.0 should be True"
assert 0.5 < f_a < f_b, "0.5 < 1.0 < 2.5 should be True"

# Edge cases: boundary conditions
assert 0 <= 0 < 1, "0 <= 0 < 1 should be True (boundary)"
assert not (0 < 0 < 1), "0 < 0 < 1 should be False (strict inequality)"
assert 1 <= 1 <= 1, "1 <= 1 <= 1 should be True (all equal)"

# Mixed int and float in chains
assert 1 < 2.5 < 4, "1 < 2.5 < 4 should be True (mixed types)"

# Short-circuit verification: side effects should only happen if previous comparison is true
# Using a counter function to track evaluations
chain_call_count: int = 0

def chain_increment_and_return(val: int) -> int:
    global chain_call_count
    chain_call_count = chain_call_count + 1
    return val

# When first comparison fails, middle operand should not be evaluated again
chain_call_count = 0
chain_result: bool = 10 < chain_increment_and_return(5) < 3  # 10 < 5 is False, short-circuit
assert chain_call_count == 1, "Middle operand should be evaluated once"
assert not chain_result, "10 < 5 < 3 should be False"

# When first comparison succeeds, middle operand is used in both comparisons
# but should only be evaluated once
chain_call_count = 0
chain_result = 1 < chain_increment_and_return(5) < 10  # Both comparisons evaluated
assert chain_call_count == 1, "Middle operand should be evaluated only once"
assert chain_result, "1 < 5 < 10 should be True"

# Greater-than chains
assert 10 > 5 > 1, "10 > 5 > 1 should be True"
assert not (1 > 5 > 10), "1 > 5 > 10 should be False"

# Not-equal chains
assert 1 != 2 != 3, "1 != 2 != 3 should be True"
assert not (1 != 2 != 2), "1 != 2 != 2 should be False"

# Complex chain with multiple function calls
chain_call_count = 0
# This chain: f(1) < f(2) < f(3)
# Each function should be called exactly once
chain_result = chain_increment_and_return(1) < chain_increment_and_return(2) < chain_increment_and_return(3)
assert chain_call_count == 3, "Each operand should be evaluated once"
assert chain_result, "1 < 2 < 3 should be True"

# ===== SECTION: Bool + float mixed arithmetic (regression test) =====

assert True + 0.5 == 1.5, "True + 0.5 should equal 1.5"
assert False + 0.5 == 0.5, "False + 0.5 should equal 0.5"
assert True * 2.0 == 2.0, "True * 2.0 should equal 2.0"
assert 0.5 + True == 1.5, "0.5 + True should equal 1.5"
assert 1.0 - True == 0.0, "1.0 - True should equal 0.0"
assert True / 2.0 == 0.5, "True / 2.0 should equal 0.5"
assert 10.0 * False == 0.0, "10.0 * False should equal 0.0"

print("Bool + float mixed arithmetic tests passed!")

# ===== SECTION: Float scientific notation formatting (regression test) =====

assert str(1e308) == "1e+308", f"str(1e308) should be '1e+308', got '{str(1e308)}'"
assert str(1e-308) == "1e-308", f"str(1e-308) should be '1e-308', got '{str(1e-308)}'"
assert str(1e20) == "1e+20", f"str(1e20) should be '1e+20', got '{str(1e20)}'"
assert str(1e-10) == "1e-10", f"str(1e-10) should be '1e-10', got '{str(1e-10)}'"
# Values below the threshold should still use decimal notation
assert str(1e15) == "1000000000000000.0", f"str(1e15) should be decimal, got '{str(1e15)}'"
assert str(1.5) == "1.5", f"str(1.5) should be '1.5', got '{str(1.5)}'"
assert str(0.0) == "0.0", f"str(0.0) should be '0.0', got '{str(0.0)}'"

print("Float scientific notation formatting tests passed!")

# ===== SECTION: Bidirectional type propagation for empty containers =====

_bidir_nums: list[int] = []
_bidir_nums.append(1)
_bidir_nums.append(2)
assert len(_bidir_nums) == 2, "bidirectional: empty list with type hint"
assert _bidir_nums[0] == 1, "bidirectional: list[int] elem access"

_bidir_d: dict[str, int] = {}
_bidir_d["a"] = 1
assert _bidir_d["a"] == 1, "bidirectional: empty dict with type hint"

print("Bidirectional type propagation tests passed!")

print("All core types and operator tests passed!")
