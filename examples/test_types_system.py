# Consolidated test file for type system

from typing import List, Dict, Set, Tuple, TypeAlias, Literal, TypeVar, Protocol

# ===== SECTION: typing module imports (List, Dict, Set, Tuple, Optional, Union) =====

# Test List (typing)
nums_typing: List[int] = [1, 2, 3]
assert nums_typing[0] == 1, "nums_typing[0] should equal 1"
assert nums_typing[1] == 2, "nums_typing[1] should equal 2"
assert len(nums_typing) == 3, "len(nums_typing) should equal 3"

# Test Dict (typing)
d_typing: Dict[str, int] = {"a": 1, "b": 2}
assert d_typing["a"] == 1, "d_typing[\"a\"] should equal 1"
assert d_typing["b"] == 2, "d_typing[\"b\"] should equal 2"

# Test Set (typing)
s_typing: Set[int] = {1, 2, 3}
assert 1 in s_typing, "1 should be in s_typing"
assert 4 not in s_typing, "4 not should be in s_typing"

# Test Tuple (typing)
t_typing: Tuple[int, int, int] = (1, 2, 3)
assert t_typing[0] == 1, "t_typing[0] should equal 1"
assert t_typing[1] == 2, "t_typing[1] should equal 2"
assert t_typing[2] == 3, "t_typing[2] should equal 3"

# ===== SECTION: PEP 585 generics (list[T], dict[K,V]) =====

# Test list (built-in)
nums_builtin: list[int] = [10, 20, 30]
assert nums_builtin[0] == 10, "nums_builtin[0] should equal 10"
assert nums_builtin[1] == 20, "nums_builtin[1] should equal 20"
assert len(nums_builtin) == 3, "len(nums_builtin) should equal 3"

# Test dict (built-in)
d_builtin: dict[str, int] = {"x": 100, "y": 200}
assert d_builtin["x"] == 100, "d_builtin[\"x\"] should equal 100"
assert d_builtin["y"] == 200, "d_builtin[\"y\"] should equal 200"

# Test set (built-in)
s_builtin: set[int] = {10, 20, 30}
assert 10 in s_builtin, "10 should be in s_builtin"
assert 40 not in s_builtin, "40 not should be in s_builtin"

# Test tuple (built-in)
t_builtin: tuple[int, int, int] = (10, 20, 30)
assert t_builtin[0] == 10, "t_builtin[0] should equal 10"
assert t_builtin[1] == 20, "t_builtin[1] should equal 20"
assert t_builtin[2] == 30, "t_builtin[2] should equal 30"

# Nested types
nested_typing: Dict[str, List[int]] = {"key": [1, 2, 3]}
assert nested_typing["key"][0] == 1, "nested_typing[\"key\"][0] should equal 1"
assert nested_typing["key"][2] == 3, "nested_typing[\"key\"][2] should equal 3"

nested_builtin: dict[str, list[int]] = {"data": [4, 5, 6]}
assert nested_builtin["data"][0] == 4, "nested_builtin[\"data\"][0] should equal 4"
assert nested_builtin["data"][2] == 6, "nested_builtin[\"data\"][2] should equal 6"

# Mixed styles
mixed1: List[tuple[int, int]] = [(1, 2), (3, 4)]
assert mixed1[0][0] == 1, "mixed1[0][0] should equal 1"
assert mixed1[1][1] == 4, "mixed1[1][1] should equal 4"

complex_nested: dict[str, List[tuple[int, int]]] = {"pairs": [(1, 2), (3, 4)]}
assert complex_nested["pairs"][0][0] == 1, "complex_nested[\"pairs\"][0][0] should equal 1"
assert complex_nested["pairs"][1][1] == 4, "complex_nested[\"pairs\"][1][1] should equal 4"

# ===== SECTION: Union types =====

# PEP 604 union types (int | str)
val1: int | str = 42
val2: int | str = "hello"

# Union with None
maybe_int: int | None = 5
maybe_none: int | None = None
maybe_value: int | None = 42

# Multi-type Union
multi_none: int | str | None = None
multi_str: int | str | None = "test"

# List with Union elements
union_items: list[int | str] = [1, "two", 3]

# Dict with Union values
union_data: dict[str, int | str] = {"a": 1, "b": "two"}

# ===== SECTION: Union operations (print, comparison, str conversion) =====

# Print tests
print(maybe_int)  # -> 5
print(maybe_none)  # -> None

multi: int | str | None = "hello"
print(multi)  # -> hello

float_or_none: float | None = 3.14
print(float_or_none)  # -> 3.14

# Comparison tests
a: int | None = 42
b: int | None = 42
assert a == b, "a should equal b"

a = None
b = None
assert a == b, "a should equal b"

x_union: int | str = 42
y_union: int | str = "42"
assert x_union != y_union, "x_union should not equal y_union"  # different types should not be equal

c: int | None = 10
d: int | None = 20
assert c != d, "c should not equal d"

# Mixed comparisons (Union with non-Union)
val: int | None = 42
assert val == 42, "val should equal 42"

val = None
assert val == None, "val should equal None"

# String Union comparisons
s1: str | None = "test"
s2: str | None = "test"
assert s1 == s2, "s1 should equal s2"

s3: str | None = "different"
assert s1 != s3, "s1 should not equal s3"

# str() conversion tests
int_union: int | None = 123
su = str(int_union)
assert su == "123", "su should equal \"123\""

none_union: int | None = None
su = str(none_union)
assert su == "None", "su should equal \"None\""

float_union: float | None = 2.5
su = str(float_union)
assert su == "2.5", "su should equal \"2.5\""

bool_union: bool | None = False
su = str(bool_union)
assert su == "False", "su should equal \"False\""

# ===== SECTION: Union ordering comparisons (<, >, <=, >=) =====

# Same-type int comparisons through Union
union_int1: int | str = 10
union_int2: int | str = 20
assert union_int1 < union_int2, "10 < 20"
assert union_int2 > union_int1, "20 > 10"
assert union_int1 <= union_int1, "10 <= 10"
assert union_int1 >= union_int1, "10 >= 10"
assert union_int1 <= union_int2, "10 <= 20"
assert union_int2 >= union_int1, "20 >= 10"
assert not (union_int1 > union_int2), "not (10 > 20)"
assert not (union_int2 < union_int1), "not (20 < 10)"

# Float comparisons through Union
union_float1: int | float = 3.14
union_float2: int | float = 2.71
assert union_float2 < union_float1, "2.71 < 3.14"
assert union_float1 > union_float2, "3.14 > 2.71"
assert union_float1 >= union_float1, "3.14 >= 3.14"
assert union_float2 <= union_float2, "2.71 <= 2.71"

# Mixed int/float comparisons (int promoted to float)
union_mixed1: int | float = 5
union_mixed2: int | float = 5.5
assert union_mixed1 < union_mixed2, "5 < 5.5"
assert union_mixed2 > union_mixed1, "5.5 > 5"

union_mixed3: int | float = 5
union_mixed4: int | float = 5.0
assert union_mixed3 <= union_mixed4, "5 <= 5.0"
assert union_mixed3 >= union_mixed4, "5 >= 5.0"

# String lexicographic comparison through Union
union_str1: int | str = "apple"
union_str2: int | str = "banana"
assert union_str1 < union_str2, "apple < banana"
assert union_str2 > union_str1, "banana > apple"
assert union_str1 <= union_str1, "apple <= apple"
assert union_str2 >= union_str2, "banana >= banana"

# String comparison edge cases
union_str3: str | None = "a"
union_str4: str | None = "aa"
assert union_str3 < union_str4, "a < aa"
assert union_str4 > union_str3, "aa > a"

# Boolean comparisons through Union (False=0, True=1)
union_bool1: int | bool = False
union_bool2: int | bool = True
assert union_bool1 < union_bool2, "False < True"
assert union_bool2 > union_bool1, "True > False"
assert union_bool1 <= union_bool1, "False <= False"
assert union_bool2 >= union_bool2, "True >= True"

# Edge case: same value
union_same: int | str = 42
assert union_same <= union_same, "42 <= 42"
assert union_same >= union_same, "42 >= 42"
assert not (union_same < union_same), "not (42 < 42)"
assert not (union_same > union_same), "not (42 > 42)"

# Negative numbers
union_neg1: int | float = -10
union_neg2: int | float = 5
assert union_neg1 < union_neg2, "-10 < 5"
assert union_neg2 > union_neg1, "5 > -10"

# Float edge cases
union_fl1: float | None = 0.0
union_fl2: float | None = 0.1
assert union_fl1 < union_fl2, "0.0 < 0.1"
assert union_fl2 > union_fl1, "0.1 > 0.0"

# ===== SECTION: Type narrowing after isinstance() =====

def test_basic_int_narrowing() -> None:
    x: int | str = 42
    if isinstance(x, int):
        result = x + 10
        assert result == 52, "result should equal 52"
    else:
        assert False, "False should be True"

def test_basic_str_narrowing() -> None:
    x: int | str = "hello"
    if isinstance(x, str):
        result = x.upper()
        assert result == "HELLO", "result should equal \"HELLO\""
    else:
        assert False, "False should be True"

def test_else_branch_narrowing() -> None:
    x: int | str = "world"
    if isinstance(x, int):
        assert False, "False should be True"
    else:
        result = x.lower()
        assert result == "world", "result should equal \"world\""

def test_three_type_union() -> None:
    x: int | str | None = 42
    if isinstance(x, int):
        result = x + 8
        assert result == 50, "result should equal 50"
    elif isinstance(x, str):
        assert False, "False should be True"
    else:
        assert False, "False should be True"

def test_three_type_union_str() -> None:
    x: int | str | None = "test"
    if isinstance(x, int):
        assert False, "False should be True"
    elif isinstance(x, str):
        assert x.upper() == "TEST", "x.upper() should equal \"TEST\""
    else:
        assert False, "False should be True"

def test_negation() -> None:
    x: int | str = "negated"
    if not isinstance(x, int):
        assert x.upper() == "NEGATED", "x.upper() should equal \"NEGATED\""
    else:
        assert False, "False should be True"

# ===== SECTION: Type inference for locals =====

# Integer inference
inf_x = 5
assert inf_x == 5, "inf_x should equal 5"

# Float inference
inf_y = 3.14
inf_z = inf_y + 1.0
assert inf_z > 4.0, "inf_z should be greater than 4.0"
assert inf_z < 5.0, "inf_z should be less than 5.0"

# Float arithmetic
inf_a = 2.5
inf_b = 1.5
inf_c = inf_a + inf_b
assert inf_c == 4.0, "inf_c should equal 4.0"

# Boolean inference
inf_flag = True
assert inf_flag, "inf_flag should be True"

# String inference
inf_name = "hello"
assert len(inf_name) == 5, "len(inf_name) should equal 5"

# List inference
inf_nums = [1, 2, 3]
assert inf_nums[0] == 1, "inf_nums[0] should equal 1"

# Tuple inference
inf_point = (10, 20)
assert inf_point[0] == 10, "inf_point[0] should equal 10"

# Reassignment (same type)
counter = 0
counter = counter + 1
assert counter == 1, "counter should equal 1"

# Mixed: some with annotation, some without
annotated: int = 42
assert annotated == 42, "annotated should equal 42"
w = 100
assert annotated + w == 142, "annotated + w should equal 142"

# Empty list REQUIRES annotation
empty: list[int] = []
assert len(empty) == 0, "len(empty) should equal 0"

# List with inferred type, then use methods
numbers = [10, 20, 30]
numbers.append(40)
assert len(numbers) == 4, "len(numbers) should equal 4"
assert numbers[3] == 40, "numbers[3] should equal 40"

# Using inferred variables in function calls
def double(n: int) -> int:
    return n * 2

infer_val = 7
infer_result = double(infer_val)
assert infer_result == 14, "infer_result should equal 14"

# ===== SECTION: Tuple unpacking =====

# Basic tuple unpacking
unpack_a, unpack_b = (1, 2)
assert unpack_a == 1, "unpack_a should equal 1"
assert unpack_b == 2, "unpack_b should equal 2"

# Tuple unpacking with more elements
unpack_x, unpack_y, unpack_z = (10, 20, 30)
assert unpack_x == 10, "unpack_x should equal 10"
assert unpack_y == 20, "unpack_y should equal 20"
assert unpack_z == 30, "unpack_z should equal 30"

# Unpacking without parentheses (implicit tuple)
unpack_p, unpack_q = 100, 200
assert unpack_p == 100, "unpack_p should equal 100"
assert unpack_q == 200, "unpack_q should equal 200"

# Swap values (parallel assignment)
unpack_m, unpack_n = 5, 10
unpack_m, unpack_n = unpack_n, unpack_m
assert unpack_m == 10, "unpack_m should equal 10"
assert unpack_n == 5, "unpack_n should equal 5"

# Unpack from a tuple variable
my_tuple: tuple[int, int, int] = (7, 8, 9)
t1, t2, t3 = my_tuple
assert t1 == 7, "t1 should equal 7"
assert t2 == 8, "t2 should equal 8"
assert t3 == 9, "t3 should equal 9"

# Unpack from a list
list_vals: list[int] = [100, 200, 300]
l1, l2, l3 = list_vals
assert l1 == 100, "l1 should equal 100"
assert l2 == 200, "l2 should equal 200"
assert l3 == 300, "l3 should equal 300"

# Unpack mixed types from tuple
name, age = ("Alice", 30)
assert age == 30, "age should equal 30"

# Unpacking in a function
def get_pair() -> tuple[int, int]:
    return (42, 84)

r1, r2 = get_pair()
assert r1 == 42, "r1 should equal 42"
assert r2 == 84, "r2 should equal 84"

# ===== SECTION: Starred unpacking (*rest) =====

# Basic case: a, *rest = [1, 2, 3, 4]
star_a, *star_rest = [1, 2, 3, 4]
assert star_a == 1, "star_a should equal 1"
assert star_rest == [2, 3, 4], "star_rest should equal [2, 3, 4]"

# Starred at the beginning: *rest, last = [1, 2, 3, 4]
*star_rest2, star_last = [1, 2, 3, 4]
assert star_rest2 == [1, 2, 3], "star_rest2 should equal [1, 2, 3]"
assert star_last == 4, "star_last should equal 4"

# Starred in the middle: first, *middle, last = [1, 2, 3, 4, 5]
star_first, *star_middle, star_last2 = [1, 2, 3, 4, 5]
assert star_first == 1, "star_first should equal 1"
assert star_middle == [2, 3, 4], "star_middle should equal [2, 3, 4]"
assert star_last2 == 5, "star_last2 should equal 5"

# Edge case: empty starred portion
star_x, *star_empty, star_y = [1, 2]
assert star_x == 1, "star_x should equal 1"
assert star_empty == [], "star_empty should equal []"
assert star_y == 2, "star_y should equal 2"

# Edge case: only starred
*star_all_items, = [1, 2, 3]
assert star_all_items == [1, 2, 3], "star_all_items should equal [1, 2, 3]"

# Edge case: single element before star
star_a2, *star_rest3 = [10]
assert star_a2 == 10, "star_a2 should equal 10"
assert star_rest3 == [], "star_rest3 should equal []"

# Works with tuples (starred always returns list)
star_t1, *star_t_rest = (10, 20, 30)
assert star_t1 == 10, "star_t1 should equal 10"
assert star_t_rest == [20, 30], "star_t_rest should equal [20, 30]"

# Multiple elements before and after star
star_p1, star_p2, *star_p_mid, star_p_last1, star_p_last2 = [1, 2, 3, 4, 5, 6, 7]
assert star_p1 == 1, "star_p1 should equal 1"
assert star_p2 == 2, "star_p2 should equal 2"
assert star_p_mid == [3, 4, 5], "star_p_mid should equal [3, 4, 5]"
assert star_p_last1 == 6, "star_p_last1 should equal 6"
assert star_p_last2 == 7, "star_p_last2 should equal 7"

# ===== SECTION: Union Is/IsNot operators =====

# Is with Union - same object identity
union_obj1: int | str = "test"
union_obj2: int | str = union_obj1
assert union_obj1 is union_obj2, "same object identity"
assert not (union_obj1 is not union_obj2), "same object: is not should be False"

# IsNot with Union - different objects
union_obj3: int | str = "different"
assert union_obj1 is not union_obj3, "different objects"
assert not (union_obj1 is union_obj3), "different objects: is should be False"

# Is with None and Union
maybe_val: int | None = None
assert maybe_val is None, "None identity check"

maybe_val2: int | None = 42
assert maybe_val2 is not None, "non-None identity check"

# Is with int Union
int_union1: int | str = 42
int_union2: int | str = 42  # Note: small ints may be the same object or not depending on boxing
# We don't test identity for primitives since boxing may vary

# ===== SECTION: Union In/NotIn operators =====

# In with Union container (list)
union_list: list[int] | None = [1, 2, 3]
if union_list is not None:
    # Need to check after narrowing for now
    pass

# Dict containment with Union container
union_dict: dict[str, int] | None = {"a": 1, "b": 2}
if union_dict is not None:
    pass

# Set containment with Union container
union_set: set[int] | None = {1, 2, 3}
if union_set is not None:
    pass

# String containment with Union container
union_str: str | None = "hello world"
if union_str is not None:
    pass

# Run type narrowing tests
test_basic_int_narrowing()
test_basic_str_narrowing()
test_else_branch_narrowing()
test_three_type_union()
test_three_type_union_str()
test_negation()

# ===== SECTION: Type narrowing with 'or' conditions =====

def test_or_else_narrowing() -> None:
    """Test that else-branch narrows when 'or' is false."""
    x: int | str | None = None
    if isinstance(x, int) or isinstance(x, str):
        assert False, "Should not reach here when x is None"
    else:
        # Both are false -> x is NOT int AND NOT str -> x is None
        assert x is None, "x should be narrowed to None"

def test_or_different_vars() -> None:
    """Test 'or' narrowing with different variables."""
    x: int | str = "hello"
    y: int | float = 3.14
    if isinstance(x, int) or isinstance(y, int):
        assert False, "Should not reach here"
    else:
        # Both false -> x is str AND y is float
        assert x.upper() == "HELLO", "x should be narrowed to str"
        assert y > 3.0, "y should be narrowed to float"

def test_not_or_then_narrowing() -> None:
    """Test that then-branch narrows when 'not (a or b)' is true."""
    x: int | str = "world"
    y: int | None = None
    if not (isinstance(x, int) or isinstance(y, int)):
        # Both are false -> x is NOT int (str) AND y is NOT int (None)
        assert x.lower() == "world", "x should be narrowed to str"
        assert y is None, "y should be narrowed to None"
    else:
        assert False, "Should not reach here"

def test_or_same_var_triple_union() -> None:
    """Test 'or' narrowing excludes multiple types from same var."""
    x: int | str | None = None
    if isinstance(x, int) or isinstance(x, str):
        assert False, "Should not reach here when x is None"
    else:
        assert x is None, "x should be narrowed to None"

def test_not_and_else_narrowing() -> None:
    """Test that else-branch narrows when 'not (a and b)' is false."""
    x: int | str = 42
    y: int | float = 10
    if not (isinstance(x, int) and isinstance(y, int)):
        assert False, "Should not reach here when both are int"
    else:
        # not (a and b) is false -> a and b is true
        result = x + y
        assert result == 52, "Both should be narrowed to int"

# Run the 'or' narrowing tests
test_or_else_narrowing()
test_or_different_vars()
test_not_or_then_narrowing()
test_or_same_var_triple_union()
test_not_and_else_narrowing()

# ===== SECTION: Bool is subtype of Int (Python semantics) =====
# In Python, bool is a subtype of int: isinstance(True, int) == True
# This means True and False can be used wherever an int is expected.

def int_increment(x: int) -> int:
    return x + 1

# Bool arguments should be accepted by functions expecting int
assert int_increment(True) == 2, "int_increment(True) should equal 2"
assert int_increment(False) == 1, "int_increment(False) should equal 1"

def int_double(x: int) -> int:
    return x * 2

assert int_double(True) == 2, "int_double(True) should equal 2"
assert int_double(False) == 0, "int_double(False) should equal 0"

# Bool in arithmetic expressions (already works, but good to verify)
bool_as_int_result: int = True + True
assert bool_as_int_result == 2, "True + True should equal 2"

bool_as_int_result2: int = True * 5
assert bool_as_int_result2 == 5, "True * 5 should equal 5"

bool_as_int_result3: int = False * 100
assert bool_as_int_result3 == 0, "False * 100 should equal 0"

# Bool subtraction and other arithmetic
bool_sub_result: int = True - False
assert bool_sub_result == 1, "True - False should equal 1"

bool_floor_div: int = True // True
assert bool_floor_div == 1, "True // True should equal 1"

# ===== SECTION: is None / is not None narrowing =====

def test_is_none_narrowing_basic() -> None:
    """Test basic is None narrowing."""
    x: int | None = None
    if x is None:
        # then-branch: x is None
        assert x is None, "x should be None in then-branch"
    else:
        # else-branch: x is not None (narrowed to int)
        result = x + 1
        assert False, "Should not reach else-branch when x is None"

def test_is_not_none_narrowing_basic() -> None:
    """Test basic is not None narrowing."""
    x: int | None = 42
    if x is not None:
        # then-branch: x is not None (narrowed to int)
        result = x + 10
        assert result == 52, "x should be narrowed to int in then-branch"
    else:
        # else-branch: x is None
        assert False, "Should not reach else-branch when x is 42"

def test_is_none_with_str_union() -> None:
    """Test is None narrowing with str | None."""
    s: str | None = "hello"
    if s is not None:
        assert s.upper() == "HELLO", "s should be narrowed to str"
    else:
        assert False, "Should not reach else-branch"

def test_is_none_triple_union() -> None:
    """Test is None narrowing with int | str | None."""
    x: int | str | None = None
    if x is None:
        assert x is None, "x is None"
    else:
        # x is narrowed to int | str
        assert False, "Should not reach else-branch when x is None"

def test_is_not_none_triple_union() -> None:
    """Test is not None narrowing with int | str | None."""
    x: int | str | None = 42
    if x is not None:
        # x is narrowed to int | str
        # We can't further narrow without isinstance, but we know it's not None
        pass
    else:
        assert False, "Should not reach else-branch when x is 42"

def test_not_is_none_negation() -> None:
    """Test not (x is None) - should be equivalent to x is not None."""
    x: int | None = 42
    if not (x is None):
        result = x + 10
        assert result == 52, "x should be narrowed to int"
    else:
        assert False, "Should not reach else-branch"

def test_not_is_not_none_negation() -> None:
    """Test not (x is not None) - should be equivalent to x is None."""
    x: int | None = None
    if not (x is not None):
        assert x is None, "x should be None"
    else:
        assert False, "Should not reach else-branch"

def test_is_none_with_isinstance_chain() -> None:
    """Test is not None followed by isinstance for further narrowing."""
    x: int | str | None = 42
    if x is not None:
        # Now x is int | str
        if isinstance(x, int):
            result = x + 8
            assert result == 50, "x should be narrowed to int"
        else:
            assert False, "Should not reach str branch"
    else:
        assert False, "Should not reach None branch"

# Run is None narrowing tests
test_is_none_narrowing_basic()
test_is_not_none_narrowing_basic()
test_is_none_with_str_union()
test_is_none_triple_union()
test_is_not_none_triple_union()
test_not_is_none_negation()
test_not_is_not_none_negation()
test_is_none_with_isinstance_chain()

# ===== SECTION: Truthiness narrowing for Optional =====

def test_truthiness_if_x_then_not_none() -> None:
    """Test: if x: narrows x to exclude None in then-branch."""
    x: int | None = 42
    if x:
        # x is truthy, so x is not None (narrowed to int)
        result = x + 10
        assert result == 52, "x should be narrowed to int in then-branch"
    else:
        assert False, "Should not reach else-branch when x is 42"

def test_truthiness_if_not_x_else_not_none() -> None:
    """Test: if not x: else-branch narrows x to exclude None."""
    x: int | None = 42
    if not x:
        assert False, "Should not reach then-branch when x is 42"
    else:
        # x is truthy, so x is not None (narrowed to int)
        result = x + 10
        assert result == 52, "x should be narrowed to int in else-branch"

def test_truthiness_none_is_falsy() -> None:
    """Test: None value takes else branch."""
    x: int | None = None
    if x:
        assert False, "None should be falsy"
    else:
        # x is falsy (could be None or 0), but we know it's None here
        pass

def test_truthiness_zero_is_falsy() -> None:
    """Test: 0 (int) is falsy but still int type."""
    x: int | None = 0
    if x:
        assert False, "0 should be falsy"
    else:
        # Note: we can't narrow to None here because 0 is also falsy
        # This is correct behavior - we don't narrow in the else branch
        pass

def test_truthiness_str_none_union() -> None:
    """Test truthiness narrowing with str | None."""
    s: str | None = "hello"
    if s:
        # s is truthy, so s is not None (narrowed to str)
        result = s.upper()
        assert result == "HELLO", "s should be narrowed to str"
    else:
        assert False, "Should not reach else-branch when s is 'hello'"

def test_truthiness_empty_str_is_falsy() -> None:
    """Test: empty string is falsy."""
    s: str | None = ""
    if s:
        assert False, "Empty string should be falsy"
    else:
        # Note: we can't narrow to None here because "" is also falsy
        pass

def test_truthiness_combined_with_isinstance() -> None:
    """Test truthiness narrowing combined with isinstance."""
    x: int | str | None = 42
    if x:
        # x is truthy, so x is not None (narrowed to int | str)
        if isinstance(x, int):
            result = x + 8
            assert result == 50, "x should be narrowed to int"
        else:
            assert False, "Should not reach str branch"
    else:
        assert False, "Should not reach None branch"

def test_truthiness_while_loop() -> None:
    """Test truthiness narrowing in while loop conditions."""
    x: int | None = 3
    count = 0
    while x:
        # x is narrowed to int in loop body
        count = count + 1
        x = x - 1  # Reassign works correctly with narrowed Union
    assert count == 3, "Should have looped 3 times"

# Run truthiness narrowing tests
test_truthiness_if_x_then_not_none()
test_truthiness_if_not_x_else_not_none()
test_truthiness_none_is_falsy()
test_truthiness_zero_is_falsy()
test_truthiness_str_none_union()
test_truthiness_empty_str_is_falsy()
test_truthiness_combined_with_isinstance()
test_truthiness_while_loop()

# ===== SECTION: TypeAlias (PEP 613) =====

# TypeAlias with annotation style
IntList: TypeAlias = list[int]
ta_nums: IntList = [1, 2, 3]
assert len(ta_nums) == 3, "TypeAlias IntList should work as list[int]"
assert ta_nums[0] == 1, "TypeAlias IntList element access"

StrDict: TypeAlias = dict[str, int]
ta_dict: StrDict = {"a": 1, "b": 2}
assert ta_dict["a"] == 1, "TypeAlias StrDict should work as dict[str, int]"
assert len(ta_dict) == 2, "TypeAlias StrDict length"

# Nested type alias
NestedAlias: TypeAlias = list[dict[str, int]]
ta_nested: NestedAlias = [{"x": 10}, {"y": 20}]
assert len(ta_nested) == 2, "Nested TypeAlias should work"
assert ta_nested[0]["x"] == 10, "Nested TypeAlias element access"

print("TypeAlias tests passed!")

# ===== SECTION: PEP 695 type statement =====

type IntSet = set[int]
ta_set: IntSet = {1, 2, 3}
assert 1 in ta_set, "PEP 695 type alias should work as set[int]"
assert len(ta_set) == 3, "PEP 695 type alias set length"

type OptStr = str | None
ta_opt1: OptStr = "hello"
ta_opt2: OptStr = None
assert ta_opt1 == "hello", "PEP 695 union alias with value"
assert ta_opt2 is None, "PEP 695 union alias with None"

print("PEP 695 type statement tests passed!")

# ===== SECTION: Literal types =====

lit_mode: Literal["r", "w"] = "r"
assert lit_mode == "r", "Literal[str, str] should accept string value"

lit_code: Literal[0, 1, 2] = 1
assert lit_code == 1, "Literal[int, int, int] should accept int value"

lit_flag: Literal[True] = True
assert lit_flag == True, "Literal[bool] should accept bool value"

lit_neg: Literal[-1, 0, 1] = -1
assert lit_neg == -1, "Literal with negative int"

lit_none: Literal[None] = None
assert lit_none is None, "Literal[None] should accept None"

print("Literal type tests passed!")

# ===== SECTION: TypeVar =====

T = TypeVar('T')

# TypeVar with int — the identity function accepts the annotation
def tv_identity_int(x: T) -> T:
    return x

tv_int_result: int = tv_identity_int(42)
assert tv_int_result == 42, "TypeVar identity with int"

# TypeVar with str — separate function since AOT compiles one specialization
def tv_identity_str(x: T) -> T:
    return x

tv_str_result: str = tv_identity_str("hello")
assert tv_str_result == "hello", "TypeVar identity with str"

# TypeVar with constraints — annotation is accepted (resolves to Union[int, float])
Num = TypeVar('Num', int, float)
tv_constrained_val: Num = 42
assert tv_constrained_val == 42, "TypeVar with constraints (annotation accepted)"

# TypeVar with bound — accepted as the bound type
Comparable = TypeVar('Comparable', bound=int)

def tv_max_val(a: Comparable, b: Comparable) -> Comparable:
    if a > b:
        return a
    return b

assert tv_max_val(3, 7) == 7, "TypeVar with bound"
assert tv_max_val(10, 2) == 10, "TypeVar with bound (reverse)"

print("TypeVar tests passed!")

# ===== SECTION: Protocol (structural subtyping) =====

# Protocol class definition is accepted — compile-time structural type
class Drawable(Protocol):
    def draw(self) -> str: ...

class Circle:
    def draw(self) -> str:
        return "circle"

class Square:
    def draw(self) -> str:
        return "square"

# Protocol-typed parameter accepts concrete class instances
def proto_render(shape: Drawable) -> str:
    return shape.draw()

proto_c = Circle()
proto_s = Square()
assert proto_render(proto_c) == "circle", "Protocol accepts Circle"
assert proto_render(proto_s) == "square", "Protocol accepts Square"

# Protocol can also be used as a variable type annotation
proto_shape: Drawable = Circle()
assert proto_shape.draw() == "circle", "Protocol variable annotation"

# Protocol with vtable layout mismatch: Square2 has __init__ + field, so draw is at different slot
class Sizable(Protocol):
    def size(self) -> int: ...

class MyBox:
    count: int
    def __init__(self, n: int) -> None:
        self.count = n
    def size(self) -> int:
        return self.count

def proto_get_size(obj: Sizable) -> int:
    return obj.size()

proto_box = MyBox(5)
assert proto_get_size(proto_box) == 5, "Protocol with different vtable layout"

# isinstance structural checks
assert isinstance(proto_box, Sizable) == True, "isinstance: box satisfies Sizable"
assert isinstance(proto_c, Sizable) == False, "isinstance: Circle lacks size()"
assert isinstance(42, Sizable) == False, "isinstance: int does not satisfy Sizable"
assert isinstance("hi", Sizable) == False, "isinstance: str does not satisfy Sizable"

# Empty Protocol: every object satisfies it
class AnyProto(Protocol):
    pass

assert isinstance(proto_box, AnyProto) == True, "isinstance: empty Protocol satisfied by instance"
assert isinstance(42, AnyProto) == True, "isinstance: empty Protocol satisfied by int"
assert isinstance("hi", AnyProto) == True, "isinstance: empty Protocol satisfied by str"

# Tuple-of-types containing a Protocol
assert isinstance(proto_box, (int, Sizable)) == True, "isinstance: tuple-of-types with Protocol (True)"
assert isinstance(proto_c, (int, Sizable)) == False, "isinstance: tuple-of-types with Protocol (False)"

# Addable Protocol: class with __add__ satisfies it (annotation + isinstance)
class Addable(Protocol):
    def __add__(self, other: int) -> int: ...

class Counter:
    def __init__(self, n: int) -> None:
        self.n = n
    def __add__(self, other: int) -> int:
        return self.n + other

class NoAddable:
    pass

proto_counter = Counter(10)

# Concrete usage (not through Protocol interface): __add__ dispatches directly
assert proto_counter.__add__(5) == 15, "Counter.__add__ works directly"
assert isinstance(proto_counter, Addable) == True, "isinstance: Counter satisfies Addable"
assert isinstance(NoAddable(), Addable) == False, "isinstance: NoAddable lacks __add__"
assert isinstance(42, Addable) == False, "isinstance: int does not satisfy Addable"

# Negative case (compile-time diagnostic): uncomment to verify
# class EmptyClass:
#     pass
# def accepts_sized(s: Sizable) -> int:  # diagnostic: type 'EmptyClass' does not satisfy protocol 'Sizable': missing method 'size'
#     return s.size()
# accepts_sized(EmptyClass())

print("Protocol tests passed!")

# ===== SECTION: Union function parameters and arithmetic =====

from typing import Union

def union_pass(x: Union[int, str]) -> Union[int, str]:
    return x

assert union_pass(5) == 5, "Union[int, str] param with int"
assert union_pass("hi") == "hi", "Union[int, str] param with str"

# Union arithmetic on variables
union_x: int | float = 5
union_y: int | float = union_x + union_x
assert union_y == 10, "Union int+int arithmetic"

union_z: int | float = 2.5
union_w: int | float = union_z + union_z
assert union_w == 5.0, "Union float+float arithmetic"

# Union function with arithmetic
def union_double(x: Union[int, float]) -> Union[int, float]:
    return x + x

assert union_double(7) == 14, "Union function int arithmetic"
assert union_double(1.5) == 3.0, "Union function float arithmetic"

print("Union function param/arithmetic tests passed!")

# Union return type with primitive returns. The Return-terminator codegen
# must box int/bool/float operands so callers see well-formed `Value` bits
# instead of raw scalars — pre-fix this would SEGV when downstream
# `rt_print_obj` reads raw int 42 (low 3 bits 0b010, no valid tag) and
# falls to the heap-pointer dispatch arm.

def union_return_int_or_str(b: bool):
    if b:
        return 42
    return "hello"

union_int_branch = union_return_int_or_str(True)
assert union_int_branch == 42, "Union return: int branch"
union_str_branch = union_return_int_or_str(False)
assert union_str_branch == "hello", "Union return: str branch"

def union_return_bool_or_str(b: bool):
    if b:
        return True
    return "false-branch"

union_bool_branch = union_return_bool_or_str(True)
assert union_bool_branch == True, "Union return: bool branch"

def union_return_float_or_str(b: bool):
    if b:
        return 1.5
    return "small"

union_float_branch = union_return_float_or_str(True)
assert union_float_branch == 1.5, "Union return: float branch"
union_str_branch2 = union_return_float_or_str(False)
assert union_str_branch2 == "small", "Union return: str branch (mixed with float)"

# Numeric-tower promotion at Return: when a function returns either a Float
# or an Int (e.g. `return 1.5` / `return 0`), type inference promotes the
# function's return type to `Float` (`int ⊂ float`) and emits the function
# signature as `f64`. The Int return branch's operand is a raw `i64` —
# without the (I64, F64) and (I8, F64) coercion arms in the Return
# terminator codegen, Cranelift's verifier rejects the function with
# "result has type i64, must match function signature of f64". The
# function-typed local annotation here uses `-> float` so the call result
# is unambiguously `Float` and the assignment storage path is uniform.
def numeric_tower_return(b: bool) -> float:
    if b:
        return 1.5
    return 0

ntr_float_result: float = numeric_tower_return(True)
assert ntr_float_result == 1.5, "Numeric-tower return: float branch"
ntr_int_result: float = numeric_tower_return(False)
assert ntr_int_result == 0.0, "Numeric-tower return: int branch promoted to float"

def numeric_tower_return_bool(b: bool) -> float:
    if b:
        return 1.5
    return True

ntrb_float_result: float = numeric_tower_return_bool(True)
assert ntrb_float_result == 1.5, "Numeric-tower return: float branch (bool variant)"
ntrb_bool_result: float = numeric_tower_return_bool(False)
assert ntrb_bool_result == 1.0, "Numeric-tower return: bool branch promoted to float"

# Numeric-tower promotion via `join_return_types` for *unannotated*
# functions. Pre-fix: `def f(b: bool): return 1.5 if b else 0` was inferred
# as `Union[int, float]` (because `join_return_types` used
# `Type::normalize_union` which doesn't promote), prescan stored
# `Union[int, float]` as `x`'s var_type, the assignment routed through
# Ptr storage, and the raw F64 return was mis-stored as a tagged pointer
# — SEGV at the next reader. Post-fix `join_return_types` uses
# `Type::unify_field_type` (numeric tower), so the inferred return is
# `Float`, prescan stores `Float`, and F64 storage is uniform end-to-end.

def unannotated_mixed_return(b: bool):
    if b:
        return 1.5
    return 0

unann_float_branch: float = unannotated_mixed_return(True)
assert unann_float_branch == 1.5, "Unannotated mixed return: float branch"
unann_int_branch: float = unannotated_mixed_return(False)
assert unann_int_branch == 0.0, "Unannotated mixed return: int branch promoted"

# Same pattern bound to an unannotated local — exercises the prescan
# var_type path that used to SEGV. With pyaot's numeric-tower promotion,
# `unann_y`'s value is `0.0` (Float storage); CPython would return raw
# `0`. Don't print the values directly to avoid CPython differential
# noise — the assertions cover correctness via `==` (0.0 == 0).
unann_x = unannotated_mixed_return(True)
assert unann_x == 1.5, "Unannotated mixed return: bound to unannotated local"
unann_y = unannotated_mixed_return(False)
assert unann_y == 0.0, "Unannotated mixed return: int branch via unannotated local"
assert unann_y == 0, "Unannotated mixed return: numeric equality across types"

# Bool + Int → Int promotion (`bool ⊂ int`).
def unannotated_bool_or_int(b: bool):
    if b:
        return 1
    return False

unann_bi_int: int = unannotated_bool_or_int(True)
assert unann_bi_int == 1, "Unannotated bool|int return: int branch"
unann_bi_bool: int = unannotated_bool_or_int(False)
assert unann_bi_bool == 0, "Unannotated bool|int return: bool branch promoted to int"

print("Numeric-tower unannotated-return tests passed!")

# Exercise through `print` so `rt_print_obj` decodes the tagged bits.
print(union_return_int_or_str(True))
print(union_return_int_or_str(False))
print(union_return_bool_or_str(True))
print(union_return_float_or_str(True))
print(union_return_float_or_str(False))

print("Union return primitive-boxing tests passed!")

print("All type system tests passed!")
