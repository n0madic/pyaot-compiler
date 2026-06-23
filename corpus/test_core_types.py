# Consolidated test file for core types and operators

from typing import Callable

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

# =============================================================================
# Area E §E.6 — local variable type rebinding
# =============================================================================
# Cross-site type unification for locals via numeric tower + post-loop
# rebind heuristic (§A.6 #3). Walks the function body before lowering and
# merges every `Bind`/`ForBind` observation; the MIR local is then sized
# for the unified type and each RHS is coerced through the tower.

# Numeric promotion on a local via AugAssign inside a function.
def _e6_f1() -> float:
    x = 0
    x += 0.5
    x += 0.25
    return x

assert abs(_e6_f1() - 0.75) < 1e-9, "E.6 augassign int->float in function"

# Bool absorbed by Int on AugAssign in function.
def _e6_f2() -> int:
    flag = False
    flag += 1
    flag += 1
    return flag

assert _e6_f2() == 2, "E.6 augassign bool->int in function"

# Post-loop rebind (§A.6 #3 — previously failed with "unknown attribute").
class _E6Wrapper:
    def __init__(self) -> None:
        self.x = 0

for _e6_a, _e6_b in [(1, 2), (3, 4)]:
    pass
_e6_c = _E6Wrapper()
_e6_c.x = 99
assert _e6_c.x == 99, "E.6 post-loop rebind to class instance"

print("Local variable type rebinding (§E.6): PASS")

# =============================================================================
# FOLDED: p2_expr.py — scalar expressions / operators (was print-based)
# =============================================================================
def _fold_p2_expr() -> None:
    assert 1 + 2 == 3
    assert 10 - 3 == 7
    assert 4 * 5 == 20
    assert 7 / 2 == 3.5
    assert 7 // 2 == 3
    assert -7 // 2 == -4
    assert 7 % 3 == 1
    assert -7 % 2 == 1
    assert 2 ** 10 == 1024
    assert 3.0 + 1.5 == 4.5
    assert 10.0 / 4 == 2.5
    assert 1 + 2 * 3 - 4 == 3
    assert 5 & 3 == 1
    assert 5 | 2 == 7
    assert 5 ^ 1 == 4
    assert 1 << 4 == 16
    assert 256 >> 2 == 64
    assert ~5 == -6
    assert -(3 + 4) == -7
    assert (not True) == False
    assert (not 0) == True
    assert (1 < 2) == True
    assert (2 < 1) == False
    assert (1 < 2 < 3) == True
    assert (1 < 2 > 3) == False
    assert (3 == 3) == True
    assert (3 != 4) == True
    assert (2 <= 2) == True
    assert (5 >= 6) == False
    assert (True and False) == False
    assert (True or False) == True
    assert (0 or 7) == 7
    assert (5 and 3) == 3
    assert (1 if True else 2) == 1
    assert (1 if False else 2) == 2
    assert ("yes" if 3 > 2 else "no") == "yes"
    assert abs(-5) == 5
    assert abs(5) == 5
    assert int(3.9) == 3
    assert float(7) == 7.0
    assert str(42) == "42"
    assert bool(0) == False
    assert bool(1) == True
    assert len("hello") == 5
    assert 10 % 3 == 1
    assert 10 // 3 == 3
    assert (2 + 3 == 5) == True
    assert 1 + 2 * 3 - 4 // 2 + 5 % 3 == 7


_fold_p2_expr()

# =============================================================================
# FOLDED: p2_scalars_print.py — scalar literals + print formatting
# =============================================================================
def _fold_p2_scalars() -> None:
    # Value-level scalar checks (print formatting is exercised by the target's
    # "Print functionality" section; here we assert the underlying values).
    assert "hello" == "hello"
    assert 3.14 == 3.14
    assert 2.0 == 2.0
    assert -2.5 == -2.5
    assert True == True
    assert False == False
    assert None is None
    assert -5 == -5
    assert 1000000 == 1000000
    assert 0 == 0
    # sep / multi-value print formatting, reconstructed as string composition.
    assert ", ".join(["comma", "sep"]) == "comma, sep"
    assert " = ".join(["mixed", str(42)]) == "mixed = 42"


_fold_p2_scalars()

# =============================================================================
# FOLDED: p2_bignum.py — arbitrary-precision integer arithmetic (heap BigInt)
# =============================================================================
def _fold_p2_bignum_factorial(n: int) -> int:
    result = 1
    for i in range(1, n + 1):
        result = result * i
    return result


def _fold_p2_bignum_fact_rec(n: int) -> int:
    if n <= 1:
        return 1
    return n * _fold_p2_bignum_fact_rec(n - 1)


def _fold_p2_bignum() -> None:
    assert 2 ** 100 == 1267650600228229401496703205376
    assert 2 ** 64 == 18446744073709551616
    assert 10 ** 30 == 1000000000000000000000000000000

    assert _fold_p2_bignum_factorial(30) == 265252859812191058636308480000000
    assert _fold_p2_bignum_factorial(20) == 2432902008176640000
    assert _fold_p2_bignum_factorial(10) == 3628800
    assert _fold_p2_bignum_fact_rec(25) == 15511210043330985984000000

    big = 2 ** 100
    assert big + 1 == 1267650600228229401496703205377
    assert big * 2 == 2535301200456458802993406410752
    assert big - big == 0
    assert big // 1000000 == 1267650600228229401496703
    assert big % 7 == 2
    assert (big > 0) == True
    assert (big == 2 ** 100) == True
    assert (2 ** 100 < 2 ** 101) == True
    assert -(2 ** 70) == -1180591620717411303424
    assert 2 ** 100 // 2 ** 100 == 1
    assert (10 ** 20) - (10 ** 20) == 0
    xb = 2 ** 100
    assert xb & 1 == 0
    assert xb | 1 == 1267650600228229401496703205377
    assert xb ^ 1 == 1267650600228229401496703205377
    assert xb >> 4 == 79228162514264337593543950336
    assert xb << 4 == 20282409603651670423947251286016
    assert ((xb << 4) >> 4 == xb) == True
    assert 2 ** 100 & 1 == 0
    assert (2 ** 100 + 1) & 255 == 1
    assert xb & 0 == 0


_fold_p2_bignum()

# =============================================================================
# FOLDED: p3_numeric.py — unboxed float arithmetic + literal-bounded raw-int
# loops (Phase 3 numeric specialization). Loop shapes preserved verbatim.
# =============================================================================
def _fold_p3_numeric_poly(a: float) -> float:
    return a * a + 2.0 * a + 1.0


def _fold_p3_numeric_tri(n: int) -> int:
    # Non-literal range bound: the cursor stays tagged inside this function.
    total = 0
    for i in range(n + 1):
        total = total + i
    return total


def _fold_p3_numeric() -> None:
    # Unboxed float accumulation (raw fadd, no boxing / GC traffic).
    acc = 0.0
    for i in range(10):
        acc = acc + 0.5
    assert acc == 5.0

    # Raw float * and - (fmul / fsub); tagged true-division stays exact.
    x = 2.5
    y = x * x - 1.0
    assert y == 5.25
    assert 7.0 / 2.0 == 3.5
    assert 10 / 4 == 2.5

    # Literal-bounded raw-int loop; cursor runs raw, accumulator stays tagged.
    s = 0
    for k in range(1, 11):
        s = s + k
    assert s == 55

    # Negative literal step.
    t = 0
    for d in range(20, 0, -3):
        t = t + d
    assert t == 77

    # Loop variable used in body arithmetic; n and n*n run on the raw path.
    sq = 0
    for n in range(1, 6):
        sq = sq + n * n
    assert sq == 55

    # Mixed int/float stays tagged but correct (runtime promotes).
    assert 3 + 1.5 == 4.5
    assert 2 * 2.0 == 4.0

    assert _fold_p3_numeric_poly(3.0) == 16.0
    assert _fold_p3_numeric_tri(100) == 5050


_fold_p3_numeric()

# =============================================================================
# FOLDED: p3c_raw_int_loops.py — raw-int loop specialization (interval proof).
# Loop / recursion shapes and bounds preserved verbatim to drive specialization;
# only the printed values become asserts.
# =============================================================================
def _fold_p3c_raw_int_loops() -> None:
    # 1. Narrowed induction variable + derived expressions: i, i*3, i*3 % 7,
    #    i*3 // 7 are provably in +-2^48, so each runs raw. Each sub-expression
    #    is accumulated independently across the full loop (same shapes/bound as
    #    the original per-iteration print).
    s_i = 0
    s_i3 = 0
    s_mod = 0
    s_div = 0
    for i in range(50):
        s_i = s_i + i
        s_i3 = s_i3 + i * 3
        s_mod = s_mod + i * 3 % 7
        s_div = s_div + i * 3 // 7
    assert s_i == 1225
    assert s_i3 == 3675
    assert s_mod == 147
    assert s_div == 504

    # 2. Floor semantics with NEGATIVE operands (srem/sdiv truncate toward zero;
    #    codegen must floor toward -inf).
    assert (-7) // 2 == -4
    assert (-7) % 2 == 1
    assert 7 // 2 == 3
    assert 7 % 2 == 1
    assert (-1) // 3 == -1
    assert (-1) % 3 == 2
    assert (-13) // 4 == -4
    assert (-13) % 4 == 3

    # A bounded NEGATIVE-step loop whose induction variable goes negative and
    # feeds raw % / //; the floor correction runs on live raw operands. Each
    # sub-expression is accumulated independently (preserves the loop shape).
    s_k = 0
    s_kmod = 0
    s_kdiv = 0
    for k in range(5, -6, -1):
        s_k = s_k + k
        s_kmod = s_kmod + k % 3
        s_kdiv = s_kdiv + k // 3
    assert s_k == 0
    assert s_kmod == 12
    assert s_kdiv == -4

    # 3. The proof must REFUSE: x doubles 60 times -> 2**60 exceeds the +-2^48
    #    bound, so x stays tagged and promotes to a heap bignum.
    x = 1
    for _ in range(60):
        x = x * 2
    assert x == 1152921504606846976

    # 4. An unboundable while (collatz-shaped): n escapes any static bound, so it
    #    stays tagged and its % / // stay bignum-safe on the tagged baseline.
    n = 27
    steps = 0
    while n != 1:
        if n % 2 == 0:
            n = n // 2
        else:
            n = 3 * n + 1
        steps = steps + 1
    assert steps == 111

    # 5. The induction variable read AFTER the loop (its final value survives).
    last = -1
    for j in range(10):
        last = j
    assert last == 9

    # 6. A small accumulator stays tagged while the index runs raw.
    total = 0
    for m in range(100):
        total = total + (m * 7 % 13)
    assert total == 590


_fold_p3c_raw_int_loops()

# =============================================================================
# FOLDED: p3c_interproc_raw.py — interprocedural raw-int specialization.
# Call-graph shapes (direct edges, address-taken, recursion, try seam) preserved
# verbatim; only the printed accumulations become asserts.
# =============================================================================
def _fold_p3c_safe_div(a: int, b: int) -> int:
    return a // b


def _fold_p3c_dbl(n: int) -> int:
    return n + n


def _fold_p3c_apply_fn(f: Callable[[int], int], x: int) -> int:
    return f(x)


def _fold_p3c_mix(a: int, b: int) -> int:
    return a * 2 + b


def _fold_p3c_countdown(n: int) -> int:
    if n <= 0:
        return 0
    return _fold_p3c_countdown(n - 1) + n


def _fold_p3c_add3(a: int, b: int, c: int) -> int:
    return a + b + c


def _fold_p3c_run() -> int:
    s = 0
    try:
        for i in range(500):
            s = (s + _fold_p3c_add3(i, i * 2, 1)) % 100003
    except ValueError:
        s = -1
    return s


def _fold_p3c_interproc_raw() -> None:
    # (a) bounded args across a direct call edge -> params/return go raw.
    acc = 0
    for i in range(200):
        acc = (acc + _fold_p3c_safe_div(1000, i % 13 + 1)) % 100003
    assert acc == 49923

    # (b) address-taken `dbl` (passed as a value) stays tagged; the indirect
    #     call hands it a tagged bignum.
    assert _fold_p3c_apply_fn(_fold_p3c_dbl, 10 ** 30) == 2000000000000000000000000000000
    assert _fold_p3c_dbl(21) == 42

    # (c) per-position: `a` bounded (raw), `b` unbounded at the 2nd site (stays
    #     tagged); the mixed bignum result must be exact.
    assert _fold_p3c_mix(3, 100) == 106
    assert _fold_p3c_mix(7, 10 ** 40) == 10000000000000000000000000000000000000014

    # (d) recursive bounded function — the interproc fixpoint must terminate.
    assert _fold_p3c_countdown(50) == 1275

    # (e) raw-param/raw-return function called inside a try (Tail trampoline seam).
    assert _fold_p3c_run() == 74741


_fold_p3c_interproc_raw()

# =============================================================================
# FOLDED: p4_literals.py — container literals (list/tuple/set/dict/bytes),
# nesting, heterogeneous elements, annotated-empty bootstrap.
# =============================================================================
def _fold_p4_literals() -> None:
    nums = [1, 2, 3, 4]
    assert nums == [1, 2, 3, 4]
    assert len(nums) == 4

    pair = (10, 20)
    assert pair == (10, 20)

    uniq = {1, 2, 3, 2, 1}
    assert len(uniq) == 3

    table = {"one": 1, "two": 2, "three": 3}
    assert table == {"one": 1, "two": 2, "three": 3}
    assert len(table) == 3

    raw = b"bytes!"
    assert raw == b"bytes!"
    assert len(raw) == 6

    # Heterogeneous (tagged) elements.
    mixed = [1, "two", 3.5, True]
    assert mixed == [1, "two", 3.5, True]
    assert mixed[1] == "two"

    # Nested literals.
    grid = [[1, 2, 3], [4, 5, 6]]
    assert grid == [[1, 2, 3], [4, 5, 6]]
    assert grid[1] == [4, 5, 6]
    assert grid[0][2] == 3

    records = {"a": [1, 2], "b": [3, 4]}
    assert records["b"] == [3, 4]

    # Empty-container bootstrap (annotation seeds element type).
    acc: list[int] = []
    assert acc == []
    assert len(acc) == 0

    lookup: dict[str, int] = {}
    assert lookup == {}
    assert len(lookup) == 0

    # Non-annotated empty list stays correct (tagged elements).
    blank = []
    assert blank == []
    assert len(blank) == 0

    # A single-element tuple keeps its shape.
    solo = (42,)
    assert solo == (42,)
    assert len(solo) == 1


_fold_p4_literals()

# =============================================================================
# FOLDED: p4_operators.py — container operators: + / * (concat / repeat),
# == / !=, ordering, and membership (in / not in).
# =============================================================================
def _fold_p4_operators() -> None:
    a = [1, 2]
    b = [3, 4]
    assert a + b == [1, 2, 3, 4]
    assert a * 3 == [1, 2, 1, 2, 1, 2]
    assert [0] * 5 == [0, 0, 0, 0, 0]
    assert len(a + b) == 4

    t = (1, 2) + (3, 4)
    assert t == (1, 2, 3, 4)
    assert len(t) == 4

    assert b"ab" + b"cd" == b"abcd"
    assert b"xy" * 3 == b"xyxyxy"

    # Equality (structural) across container kinds.
    assert ([1, 2, 3] == [1, 2, 3]) == True
    assert ([1, 2] == [1, 2, 3]) == False
    assert ([1, 2] != [3, 4]) == True
    assert ((1, 2) == (1, 2)) == True
    assert ({1, 2, 3} == {3, 2, 1}) == True
    assert ({"a": 1} == {"a": 1}) == True
    assert (b"abc" == b"abc") == True
    assert (b"abc" == b"abd") == False

    # Ordering on lists and tuples (lexicographic).
    assert ([1, 2, 3] < [1, 2, 4]) == True
    assert ([1, 2] < [1, 2, 3]) == True
    assert ([2] > [1, 9, 9]) == True
    assert ((1, 2) <= (1, 2)) == True
    assert ((1, 3) >= (1, 2)) == True

    # Membership.
    xs = [10, 20, 30]
    assert (20 in xs) == True
    assert (25 in xs) == False
    assert (25 not in xs) == True
    d = {"k": 1}
    assert ("k" in d) == True
    assert ("z" in d) == False
    st = {1, 2, 3}
    assert (2 in st) == True
    assert (9 not in st) == True
    assert (3 in (1, 2, 3)) == True
    assert ("y" in "python") == True
    assert ("q" in "python") == False

    # Operators feeding into expressions.
    total = len([1, 2] + [3, 4, 5])
    assert total == 5


_fold_p4_operators()

# =============================================================================
# FOLDED: p4_subscript.py — indexed read / write: list / dict / tuple / str /
# bytes; negative indices; subscript assignment.
# =============================================================================
def _fold_p4_subscript() -> None:
    # List read + write.
    xs = [10, 20, 30, 40]
    assert xs[0] == 10
    assert xs[3] == 40
    assert xs[-1] == 40
    assert xs[-2] == 30
    xs[0] = 99
    xs[-1] = 77
    assert xs == [99, 20, 30, 77]

    # Dict read + write (string keys, int keys).
    d = {"a": 1, "b": 2}
    assert d["a"] == 1
    d["c"] = 3
    d["a"] = 100
    assert d["a"] == 100
    assert d["c"] == 3
    assert len(d) == 3

    counts = {1: "one", 2: "two"}
    assert counts[2] == "two"
    counts[3] = "three"
    assert counts[3] == "three"

    # Tuple read (immutable).
    t = (5, 6, 7)
    assert t[0] == 5
    assert t[-1] == 7
    assert t[1] == 6

    # String indexing (codepoint-aware, negatives).
    s = "python"
    assert s[0] == "p"
    assert s[-1] == "n"
    assert s[2] == "t"

    # Bytes indexing (returns int values).
    bb = b"ABC"
    assert bb[0] == 65
    assert bb[1] == 66
    assert bb[-1] == 67

    # Indexing with a computed (variable) index.
    i = 2
    assert xs[i] == 30

    # Nested subscript write.
    grid = [[1, 2], [3, 4]]
    grid[0][1] = 20
    assert grid == [[1, 20], [3, 4]]
    assert grid[0][1] == 20


_fold_p4_subscript()

# =============================================================================
# FOLDED: p16_numeric_tower_float.py — int->float tower at the return + local
# seams. DIVERGENCE-SAFE: annotation is a contract (pyaot keeps the int as
# float, CPython keeps the raw int), so every probe asserts numerically with `==`
# and never on a repr that differs between runtimes.
# =============================================================================
def _p16_ret_int_zero() -> float:
    return 0


def _p16_ret_int_seven() -> float:
    return 7


def _p16_ret_bool(b: bool) -> float:
    return b


def _p16_local_from_int() -> float:
    y: float = 5
    assert y == 5.0
    return y + 0.0


def _p16_mixed(flag: bool):
    if flag:
        return 1.5
    return 0


def _p16_use_mixed() -> float:
    a: float = _p16_mixed(True)
    b: float = _p16_mixed(False)
    assert a == 1.5
    assert b == 0.0
    return a + b


def _p16_big_pow() -> float:
    return 2 ** 62


def _p16_one() -> float:
    return 1


def _p16_sum_floats() -> float:
    xs = [_p16_one(), _p16_one(), 0.5]
    return sum(xs)


def _fold_p16_numeric_tower_float() -> None:
    # return: int / bool through `-> float`.
    assert _p16_ret_int_zero() == 0.0
    assert _p16_ret_int_seven() == 7.0
    assert _p16_ret_bool(True) == 1.0
    assert _p16_ret_bool(False) == 0.0
    # Float-forced: adding a float makes the result a float on BOTH sides.
    assert _p16_ret_int_zero() + 0.5 == 0.5
    assert _p16_ret_int_seven() + 0.5 == 7.5
    assert _p16_ret_bool(False) + 0.5 == 0.5
    assert _p16_ret_bool(True) + 0.5 == 1.5

    # annotated `: float` local from an int.
    assert _p16_local_from_int() == 5.0

    # unannotated mixed return (inferred Dyn) bound to a `: float` local.
    assert _p16_use_mixed() == 1.5

    # bignum arm: a heap BigInt through `-> float` (exact power of two).
    assert _p16_big_pow() == 4611686018427387904.0

    # `-> float` int returns feeding `sum` over a float list.
    assert _p16_sum_floats() == 2.5


_fold_p16_numeric_tower_float()

# =============================================================================
# FOLDED: p44_numeric_tower_seams.py — int->float tower at the param / global /
# field seams. DIVERGENCE-SAFE (same annotation-as-contract rationale as p16);
# all probes assert numerically with `==`. Classes hoisted to module level
# (frontend forbids classes nested in a function).
# =============================================================================
def _p44_poly(a: float) -> float:
    return a * 2.0


class _p44_Scaler:
    base: float

    def __init__(self, base: float) -> None:
        self.base = base

    def scaled(self, factor: float) -> float:
        return self.base * factor


class _p44_Counter:
    value: float

    def __init__(self) -> None:
        self.value = 0.0

    def set_from_int(self, n: int) -> None:
        self.value = n


_p44_g: float = 0.0


def _p44_read_g() -> float:
    return _p44_g + 0.0


def _p44_set_g(n: int) -> None:
    global _p44_g
    _p44_g = n


def _p44_take_float(x: float) -> float:
    return x


def _p44_scale_by_ten(factor: float) -> float:
    return factor * 10.0


def _fold_p44_numeric_tower_seams() -> None:
    # free-fn `float` parameter: int + bool args.
    assert _p44_poly(3) == 6.0
    assert _p44_poly(True) == 2.0
    assert _p44_poly(False) == 0.0
    assert _p44_poly(3) + 0.5 == 6.5
    assert _p44_poly(True) == 2.0

    # method `float` parameter: positional + keyword.
    s = _p44_Scaler(2)
    assert s.base == 2.0
    assert s.scaled(3) == 6.0
    assert s.scaled(factor=4) == 8.0
    assert s.scaled(3) + 0.5 == 6.5
    assert s.scaled(factor=4) + 0.5 == 8.5

    # `float` FIELD written from an int (store-side SetField box).
    c = _p44_Counter()
    assert c.value == 0.0
    c.set_from_int(5)
    assert c.value == 5.0
    assert c.value + 0.5 == 5.5

    # `float` GLOBAL written from an int.
    assert _p44_read_g() == 0.0
    _p44_set_g(5)
    assert _p44_g == 5.0
    assert _p44_read_g() == 5.0
    assert _p44_read_g() + 0.5 == 5.5

    # bignum arm: a heap BigInt through a `float` parameter (exact power of two).
    assert _p44_take_float(2 ** 62) == 4611686018427387904.0

    # int->float global feeding a float-param free function.
    _p44_set_g(3)
    assert _p44_scale_by_ten(_p44_g) == 30.0


_fold_p44_numeric_tower_seams()


# ===== SECTION: left-shift of a large fixnum promotes to bignum (no silent wrap) =====
def _review_shift_bignum():
    assert ((1 << 60) - 1) << 4 == 18446744073709551600, "<< must not silently wrap"
    assert (1 << 59) << 5 == 18446744073709551616
    assert (2**100) >> 50 == 2**50, "shift of a bignum"


_review_shift_bignum()


# ===== SECTION: `x in y` evaluates the LEFT operand first =====
_review_in_order: list[str] = []


def _review_in_left() -> int:
    _review_in_order.append("left")
    return 1


def _review_in_right() -> list[int]:
    _review_in_order.append("right")
    return [1, 2, 3]


_review_in_present = _review_in_left() in _review_in_right()
assert _review_in_present is True
assert _review_in_order == ["left", "right"], "`in` evaluates left before right"


print("All core types and operator tests passed!")
