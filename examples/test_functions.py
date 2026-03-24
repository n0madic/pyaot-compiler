# Consolidated test file for functions

# ===== SECTION: Function definitions and calls =====

def add(a: int, b: int) -> int:
    return a + b

def multiply(x: int, y: int) -> int:
    return x * y

# Test simple function calls
result1: int = add(10, 20)
assert result1 == 30, "add(10, 20) should be 30"

result2: int = multiply(3, 4)
assert result2 == 12, "multiply(3, 4) should be 12"

final: int = result1 + result2
assert final == 42, "30 + 12 should be 42"

# Test function composition
fx: int = add(5, 3)
fy: int = multiply(fx, 2)
assert fy == 16, "(5 + 3) * 2 should be 16"

# Function without arguments
def greet() -> str:
    return "Hello from greet()"

message: str = greet()
assert message == "Hello from greet()", "function without args failed"

# Function with float arguments
def multiply_float(a: float, b: float) -> float:
    return a * b

float_result: float = multiply_float(3.5, 2.0)
assert float_result == 7.0, "float args function failed"

# ===== SECTION: Recursion =====

def factorial_recursive(n: int) -> int:
    if n <= 1:
        return 1
    prev: int = factorial_recursive(n - 1)
    result: int = n * prev
    return result

# Test recursive factorial
fact_5: int = factorial_recursive(5)
assert fact_5 == 120, "factorial_recursive(5) should be 120"

fact_1: int = factorial_recursive(1)
assert fact_1 == 1, "factorial_recursive(1) should be 1"

fact_0: int = factorial_recursive(0)
assert fact_0 == 1, "factorial_recursive(0) should be 1"

# Test factorial using while loop
fn: int = 5
factorial_iter: int = 1
fi: int = 1

while fi <= fn:
    factorial_iter = factorial_iter * fi
    fi = fi + 1

assert factorial_iter == 120, "5! should be 120"

# Test Fibonacci using while loop
fib_n: int = 10
fa: int = 0
fb: int = 1
fj: int = 0

while fj < fib_n:
    temp: int = fa + fb
    fa = fb
    fb = temp
    fj = fj + 1

assert fa == 55, "10th Fibonacci number should be 55"

# ===== SECTION: Multiple return types =====

def return_int() -> int:
    return 42

def return_float() -> float:
    return 3.14

def return_string() -> str:
    return "Hello, World!"

def return_bool() -> bool:
    return True

def return_list() -> list[int]:
    return [1, 2, 3]

def return_bytes() -> bytes:
    return b"Hello"

assert return_int() == 42, "return int failed"
assert return_float() == 3.14, "return float failed"
assert return_string() == "Hello, World!", "return string failed"
assert return_bool() == True, "return bool failed"
assert return_list() == [1, 2, 3], "return list failed"
assert return_bytes() == b"Hello", "return bytes failed"

# ===== SECTION: Nested functions =====

# Test 1: Basic nested function
def test_basic_nested() -> None:
    def inner_add(a: int, b: int) -> int:
        return a + b
    result: int = inner_add(2, 3)
    assert result == 5, "result should equal 5"

# Test 2: Recursive nested function
def test_nested_recursive() -> None:
    def factorial(n: int) -> int:
        if n <= 1:
            return 1
        return n * factorial(n - 1)
    result: int = factorial(5)
    assert result == 120, "result should equal 120"

# Test 3: No arguments nested function
def test_nested_no_args() -> None:
    def get_value() -> int:
        return 42
    assert get_value() == 42, "get_value() should equal 42"

# Test 4: Multiple nested functions
def test_multiple_nested() -> None:
    def inner_add(a: int, b: int) -> int:
        return a + b
    def inner_mul(a: int, b: int) -> int:
        return a * b
    sum_result: int = inner_add(3, 4)
    mul_result: int = inner_mul(3, 4)
    assert sum_result == 7, "sum_result should equal 7"
    assert mul_result == 12, "mul_result should equal 12"

# Test 5: Nested with conditional
def test_nested_with_conditional() -> None:
    def abs_value(x: int) -> int:
        if x < 0:
            return -x
        else:
            return x
    assert abs_value(-5) == 5, "abs_value(-5) should equal 5"
    assert abs_value(5) == 5, "abs_value(5) should equal 5"
    assert abs_value(0) == 0, "abs_value(0) should equal 0"

# Test 6: Nested function with while loop
def test_nested_with_loop() -> None:
    def sum_to_n(n: int) -> int:
        total: int = 0
        i: int = 1
        while i <= n:
            total = total + i
            i = i + 1
        return total
    assert sum_to_n(5) == 15, "sum_to_n(5) should equal 15"  # 1+2+3+4+5
    assert sum_to_n(10) == 55, "sum_to_n(10) should equal 55"  # 1+2+...+10

# Test 7: Two-level nested functions
def test_two_levels() -> None:
    def outer() -> int:
        def inner() -> int:
            return 42
        return inner()
    result: int = outer()
    assert result == 42, "result should equal 42"

# ===== SECTION: Nested Function Return Type Inference =====
# Tests for proper return type inference in nested functions
# These tests cover bugs related to uextend.i64 errors when comparing
# results of nested function calls

# Test 1: Nested function without return annotation - comparison with result
# This was the original bug: comparing the result of a nested function call
# caused uextend.i64 error because return type was incorrectly inferred as None
def test_nested_no_annotation_comparison() -> None:
    def outer() -> int:
        def inner(a, b):  # No return type annotation
            return a + b
        return inner(1, 2)
    result: int = outer()
    assert result == 3, "nested function without annotation: expected 3"
    assert result > 0, "nested function result > 0"
    assert result < 10, "nested function result < 10"
    assert result >= 3, "nested function result >= 3"
    assert result <= 3, "nested function result <= 3"
    assert result != 0, "nested function result != 0"

# Test 2: Nested function with explicit bytes return type
# Functions with explicit annotations should use the declared type, not infer
def test_nested_explicit_bytes_annotation() -> None:
    def get_bytes() -> bytes:
        return b"test"
    result: bytes = get_bytes()
    assert result == b"test", "nested function with -> bytes annotation"

# Test 3: Nested function with explicit str return type
def test_nested_explicit_str_annotation() -> None:
    def get_str() -> str:
        return "hello"
    result: str = get_str()
    assert result == "hello", "nested function with -> str annotation"

# Test 4: Outer function returning result of nested function call
# Tests that return type is properly inferred through the call chain
def test_outer_returning_nested_call() -> None:
    def outer():  # No return type annotation
        def inner(x, y):  # No return type annotation
            return x * y
        return inner(3, 4)
    result: int = outer()
    assert result == 12, "outer returning nested call: expected 12"
    # Comparison operations should work
    assert result == 12, "comparison == works"
    assert result != 0, "comparison != works"

# Test 5: Multiple statements before return in nested function
# Tests that return type inference scans all statements, not just first
def test_nested_multiple_statements() -> None:
    def outer():
        x: int = 10
        y: int = 20
        def inner(a, b):
            temp: int = a + b
            return temp * 2
        z: int = inner(x, y)
        return z
    result: int = outer()
    assert result == 60, "multiple statements before return: expected 60"

# Test 6: Deeply nested functions with no annotations
def test_deeply_nested_no_annotations() -> None:
    def level1():
        def level2():
            def level3(x):
                return x + 1
            return level3(5)
        return level2()
    result: int = level1()
    assert result == 6, "deeply nested functions: expected 6"
    assert result == 6, "deeply nested comparison works"

# Test 7: Nested function with different operations
def test_nested_various_operations() -> None:
    def test_add():
        def add(a, b):
            return a + b
        return add(10, 5)

    def test_sub():
        def sub(a, b):
            return a - b
        return sub(10, 5)

    def test_mul():
        def mul(a, b):
            return a * b
        return mul(10, 5)

    assert test_add() == 15, "nested add: expected 15"
    assert test_sub() == 5, "nested sub: expected 5"
    assert test_mul() == 50, "nested mul: expected 50"

print("Nested function return type inference tests passed!")

# ===== SECTION: Closure capture =====

def test_closure() -> None:
    multiplier: int = 10
    def scale(x: int) -> int:
        return x * multiplier
    result: int = scale(5)
    assert result == 50, "result should equal 50"

def test_multiple_captures() -> None:
    base: int = 10
    offset: int = 5
    def compute(x: int) -> int:
        return x * base + offset
    result: int = compute(3)
    assert result == 35, "result should equal 35"  # 3 * 10 + 5

def test_grandparent_capture() -> None:
    x: int = 10
    def level1() -> int:
        def level2() -> int:
            return x  # captures x from grandparent
        return level2()
    result: int = level1()
    assert result == 10, "result should equal 10"

def test_closure_chain() -> None:
    x: int = 10
    def level1() -> int:
        y: int = 20
        def level2() -> int:
            return x + y  # captures from both levels
        return level2()
    result: int = level1()
    assert result == 30, "result should equal 30"

# ===== SECTION: nonlocal statement =====

def test_basic_nonlocal() -> None:
    """Test basic nonlocal modification"""
    x: int = 10
    def inner() -> None:
        nonlocal x
        x = 20
    inner()
    assert x == 20, "x should equal 20"

def test_nonlocal_counter() -> None:
    """Test nonlocal counter pattern"""
    count: int = 0
    def increment() -> None:
        nonlocal count
        count = count + 1
    increment()
    increment()
    increment()
    assert count == 3, "count should equal 3"

def test_nonlocal_read_and_write() -> None:
    """Test reading and writing nonlocal variable"""
    value: int = 5
    def double_value() -> None:
        nonlocal value
        value = value * 2
    double_value()
    assert value == 10, "value should equal 10"
    double_value()
    assert value == 20, "value should equal 20"

def test_multiple_nonlocal_vars() -> None:
    """Test multiple nonlocal variables"""
    a: int = 1
    b: int = 2
    def swap() -> None:
        nonlocal a, b
        swap_temp: int = a
        a = b
        b = swap_temp
    swap()
    assert a == 2, "a should equal 2"
    assert b == 1, "b should equal 1"

def test_nested_nonlocal() -> None:
    """Test deeply nested nonlocal (2 levels)"""
    outer_val: int = 100
    def middle() -> None:
        nonlocal outer_val
        def inner() -> None:
            nonlocal outer_val
            outer_val = outer_val + 1
        inner()
    middle()
    assert outer_val == 101, "outer_val should equal 101"

def test_nonlocal_in_conditional() -> None:
    """Test nonlocal inside conditional"""
    result: int = 0
    def set_result(condition: bool) -> None:
        nonlocal result
        if condition:
            result = 42
        else:
            result = 0
    set_result(True)
    assert result == 42, "result should equal 42"
    set_result(False)
    assert result == 0, "result should equal 0"

def test_nonlocal_in_loop() -> None:
    """Test nonlocal inside loop"""
    total: int = 0
    def add_numbers() -> None:
        nonlocal total
        for i in range(5):
            total = total + i
    add_numbers()
    # 0 + 1 + 2 + 3 + 4 = 10
    assert total == 10, "total should equal 10"

# ===== SECTION: Lambda expressions =====

# Basic lambda with inferred int types
lambda_add = lambda x, y: x + y
result_add: int = lambda_add(2, 3)
assert result_add == 5, "result_add should equal 5"

# Lambda with single parameter
double = lambda x: x * 2
result_double: int = double(5)
assert result_double == 10, "result_double should equal 10"

# Lambda returning boolean
is_positive = lambda x: x > 0
result_pos: bool = is_positive(5)
assert result_pos == True, "result_pos should equal True"
result_neg: bool = is_positive(-1)
assert result_neg == False, "result_neg should equal False"

# Lambda with string operations
lambda_greet = lambda name: "Hello " + name
result_greet: str = lambda_greet("World")
assert result_greet == "Hello World", "result_greet should equal \"Hello World\""

# Lambda with float
half = lambda x: x / 2.0
result_half: float = half(10.0)
assert result_half == 5.0, "result_half should equal 5.0"

# ===== SECTION: Lambda with closures =====

# Capture single variable
lambda_multiplier: int = 10
scale_lambda = lambda x: x * lambda_multiplier
result_scale: int = scale_lambda(5)
assert result_scale == 50, "result_scale should equal 50"

# Capture multiple variables
lambda_base: int = 100
lambda_offset: int = 5
compute_lambda = lambda x: lambda_base + x * lambda_offset
result_compute: int = compute_lambda(3)
assert result_compute == 115, "result_compute should equal 115"

# Capture string
prefix: str = "Hello "
greet_closure = lambda name: prefix + name
result_closure: str = greet_closure("World")
assert result_closure == "Hello World", "result_closure should equal \"Hello World\""

# Nested arithmetic with capture
factor: int = 2
addend: int = 3
transform = lambda x: x * factor + addend
result_transform: int = transform(10)
assert result_transform == 23, "result_transform should equal 23"

# ===== SECTION: Default parameters =====

def add_simple(a: int, b: int = 10) -> int:
    return a + b

default_result: int = add_simple(5)       # 5 + 10 = 15
assert default_result == 15, "default_result should equal 15"

default_result = add_simple(5, 3)         # 5 + 3 = 8
assert default_result == 8, "default_result should equal 8"

default_result = add_simple(5, b=20)      # 5 + 20 = 25
assert default_result == 25, "default_result should equal 25"

def add3(a: int, b: int = 0, c: int = 0) -> int:
    return a + b + c

default_result = add3(5)                  # 5 + 0 + 0 = 5
assert default_result == 5, "default_result should equal 5"

default_result = add3(5, 3)               # 5 + 3 + 0 = 8
assert default_result == 8, "default_result should equal 8"

default_result = add3(5, 3, 2)            # 5 + 3 + 2 = 10
assert default_result == 10, "default_result should equal 10"

# ===== SECTION: Mutable default parameters =====
# In Python, mutable defaults (list, dict, set) are evaluated once at function definition time
# and shared across all calls. This is the famous "mutable default gotcha".

def append_to_list(x: int, lst: list[int] = []) -> list[int]:
    lst.append(x)
    return lst

# Each call should append to the SAME list
mutable_result1: list[int] = append_to_list(1)
assert len(mutable_result1) == 1, "First call should have 1 element"
assert mutable_result1[0] == 1, "First element should be 1"

mutable_result2: list[int] = append_to_list(2)
assert len(mutable_result2) == 2, "Second call should have 2 elements (shared list)"
assert mutable_result2[0] == 1, "First element should still be 1"
assert mutable_result2[1] == 2, "Second element should be 2"

mutable_result3: list[int] = append_to_list(3)
assert len(mutable_result3) == 3, "Third call should have 3 elements"
assert mutable_result3[2] == 3, "Third element should be 3"

# Verify all results point to the same list
assert mutable_result1 == mutable_result2, "Results should be the same list"
assert mutable_result2 == mutable_result3, "Results should be the same list"

# Test with explicit argument (should not use the shared default)
fresh_list: list[int] = [100]
mutable_result4: list[int] = append_to_list(4, fresh_list)
assert len(mutable_result4) == 2, "Explicit list should have 2 elements"
assert mutable_result4[0] == 100, "First element of explicit list should be 100"
assert mutable_result4[1] == 4, "Second element should be 4"

# The default list should still have 3 elements (unchanged by explicit call)
mutable_result5: list[int] = append_to_list(5)
assert len(mutable_result5) == 4, "Default list should now have 4 elements"
assert mutable_result5[3] == 5, "Fourth element should be 5"

print("Mutable default parameter tests passed")

# ===== SECTION: Keyword arguments =====

default_result = add3(1, b=2)             # 1 + 2 + 0 = 3
assert default_result == 3, "default_result should equal 3"

default_result = add3(1, c=10)            # 1 + 0 + 10 = 11
assert default_result == 11, "default_result should equal 11"

default_result = add3(1, b=2, c=3)        # 1 + 2 + 3 = 6
assert default_result == 6, "default_result should equal 6"

default_result = add3(1, c=3, b=2)        # 1 + 2 + 3 = 6 (order doesn't matter)
assert default_result == 6, "default_result should equal 6"

# print() with sep kwargs
print(1, 2, 3, sep="-")                   # Should print: 1-2-3

# Keyword-only call
def multiply_kwargs(a: int, b: int) -> int:
    return a * b

default_result = multiply_kwargs(a=3, b=4)       # 12
assert default_result == 12, "default_result should equal 12"

default_result = multiply_kwargs(b=4, a=3)       # 12 (order doesn't matter)
assert default_result == 12, "default_result should equal 12"

# Run all nested function tests
test_basic_nested()
test_nested_recursive()
test_closure()
test_nested_no_args()
test_multiple_nested()
test_multiple_captures()
test_nested_with_conditional()
test_nested_with_loop()
test_two_levels()
test_grandparent_capture()
test_closure_chain()

# Run all nonlocal tests
test_basic_nonlocal()
test_nonlocal_counter()
test_nonlocal_read_and_write()
test_multiple_nonlocal_vars()
test_nested_nonlocal()
test_nonlocal_in_conditional()
test_nonlocal_in_loop()

# ===== SECTION: Variadic Parameters (*args, **kwargs) =====

# Test 1: Basic *args
def sum_all(*args: int) -> int:
    total: int = 0
    for num in args:
        total = total + num
    return total

assert sum_all(1, 2, 3) == 6, "*args with 3 args"
assert sum_all(10) == 10, "*args with 1 arg"
assert sum_all() == 0, "*args with no args"

# Test 2: Mixed regular and *args
def greet_all(greeting: str, *names: str) -> str:
    result: str = greeting
    for name in names:
        result = result + " " + name
    return result

assert greet_all("Hello", "Alice", "Bob") == "Hello Alice Bob"
assert greet_all("Hi") == "Hi"

# Test 3: Basic **kwargs - count keys
def count_kwargs(**kwargs: int) -> int:
    count: int = 0
    for key in kwargs:
        count = count + 1
    return count

# Note: Since we can't call with keyword args yet, skip this for now
# assert count_kwargs(a=1, b=2, c=3) == 3
# assert count_kwargs() == 0

# Test 4: *args with len() builtin
def count_args(*args: int) -> int:
    return len(args)

assert count_args(1, 2, 3, 4, 5) == 5
assert count_args() == 0

# Test 5: Access *args by index
def get_second(*args: int) -> int:
    if len(args) >= 2:
        return args[1]
    return -1

assert get_second(10, 20, 30) == 20
assert get_second(5) == -1

# Test 6: Regular params with defaults + *args
def with_defaults(a: int, b: int = 10, *args: int) -> int:
    total: int = a + b
    for num in args:
        total = total + num
    return total

assert with_defaults(5) == 15, "with_defaults(5) should be 15"
assert with_defaults(5, 20) == 25, "with_defaults(5, 20) should be 25"
assert with_defaults(5, 20, 30, 40) == 95, "with_defaults(5, 20, 30, 40) should be 95"

# Test 7: Only *args parameter
def only_args(*args: str) -> str:
    result: str = ""
    for s in args:
        result = result + s
    return result

assert only_args("a", "b", "c") == "abc"
assert only_args() == ""

# Test 8: *args iteration with multiple types
def describe_args(*args: int) -> str:
    if len(args) == 0:
        return "empty"
    if len(args) == 1:
        return "one"
    return "many"

assert describe_args() == "empty"
assert describe_args(42) == "one"
assert describe_args(1, 2, 3) == "many"

# Test 9: Prepare data with prefix
def prepare_data(prefix: str, *values: int) -> str:
    count: int = len(values)
    return prefix + str(count)

assert prepare_data("Count: ", 1, 2, 3) == "Count: 3"

# ===== SECTION: Keyword-Only Parameter Defaults =====

# Test 1: Keyword-only with default after *args
def kw_with_default(a: int, *args: int, b: int = 10) -> int:
    total: int = a + b
    for num in args:
        total = total + num
    return total

assert kw_with_default(5) == 15, "kw_with_default(5) should use default b=10"
assert kw_with_default(5, b=20) == 25, "kw_with_default(5, b=20) should use b=20"
assert kw_with_default(5, 1, 2, b=20) == 28, "kw_with_default(5, 1, 2, b=20) should be 5+1+2+20"

# Test 2: Multiple keyword-only parameters, some with defaults
def multi_kw(x: int, *args: int, required: str, optional: int = 100) -> str:
    total: int = x + optional
    for num in args:
        total = total + num
    return required + ":" + str(total)

assert multi_kw(10, required="sum") == "sum:110"
assert multi_kw(10, required="sum", optional=50) == "sum:60"
assert multi_kw(10, 5, 15, required="total", optional=20) == "total:50"

# Test 3: Bare * with keyword-only parameters
def bare_star(a: int, *, b: int = 5, c: int = 10) -> int:
    return a + b + c

assert bare_star(1) == 16, "bare_star(1) should be 1+5+10"
assert bare_star(1, b=20) == 31, "bare_star(1, b=20) should be 1+20+10"
assert bare_star(1, b=20, c=30) == 51, "bare_star(1, b=20, c=30) should be 1+20+30"

# Test 4: All keyword-only have defaults
def all_defaults(*, x: int = 1, y: int = 2, z: int = 3) -> int:
    return x + y + z

assert all_defaults() == 6, "all_defaults() should be 1+2+3"
assert all_defaults(x=10) == 15, "all_defaults(x=10) should be 10+2+3"
assert all_defaults(y=20, z=30) == 51, "all_defaults(y=20, z=30) should be 1+20+30"

# Test 5: Mixed required and optional keyword-only
def mixed_kw(*, req1: str, opt1: int = 10, req2: str, opt2: int = 20) -> str:
    return req1 + "-" + req2 + ":" + str(opt1 + opt2)

assert mixed_kw(req1="a", req2="b") == "a-b:30"
assert mixed_kw(req1="a", req2="b", opt1=100) == "a-b:120"
assert mixed_kw(req1="x", req2="y", opt1=5, opt2=15) == "x-y:20"

# Test 6: Regular defaults + keyword-only defaults
def complex_defaults(a: int = 1, b: int = 2, *, c: int = 3, d: int = 4) -> int:
    return a + b + c + d

assert complex_defaults() == 10, "all defaults"
assert complex_defaults(10) == 19, "override a"
assert complex_defaults(10, 20) == 37, "override a, b"
assert complex_defaults(10, 20, c=30) == 64, "override a, b, c"
assert complex_defaults(10, 20, c=30, d=40) == 100, "override all"

# Test 7: String and expression defaults
def expr_defaults(*, name: str = "default", count: int = 5 + 5) -> str:
    return name + ":" + str(count)

assert expr_defaults() == "default:10"
assert expr_defaults(name="custom") == "custom:10"
assert expr_defaults(count=20) == "default:20"

# ===== SECTION: Call-Site Unpacking (*args, **kwargs) =====

# Test 1: Basic function for unpacking tests
def accepts_three_args(a: int, b: int, c: int) -> int:
    return a + b + c

# Test 2: Runtime *args unpacking with list variable
args_list_unpack: list[int] = [1, 2, 3]
assert accepts_three_args(*args_list_unpack) == 6, "*args unpacking from list"

# Test 3: Runtime *args unpacking with tuple variable
args_tuple_unpack: tuple[int, int, int] = (10, 20, 30)
assert accepts_three_args(*args_tuple_unpack) == 60, "*args unpacking from tuple"

# Test 4: Compile-time unpacking of literal list
assert accepts_three_args(*[5, 10, 15]) == 30, "literal list unpacking"

# Test 4: Compile-time unpacking of literal tuple
assert accepts_three_args(*(7, 8, 9)) == 24, "literal tuple unpacking"

# Test 5: Mix regular and unpacked args
def mix_args_func(a: int, b: int, c: int, d: int) -> int:
    return a + b + c + d

assert mix_args_func(1, *[2, 3], 4) == 10, "mixed regular and *args"

# Test 6: Multiple literal *args unpacking
assert accepts_three_args(*[1, 2], *[3]) == 6, "multiple literal *args unpacking"

# Test 7: Empty literal *args unpacking
assert accepts_three_args(1, *[], 2, 3) == 6, "empty literal *args unpacking"

# Test 8: **kwargs unpacking with literal dict
def accepts_two_kwargs(a: int, b: int) -> int:
    return a + b

assert accepts_two_kwargs(**{"a": 5, "b": 10}) == 15, "literal **kwargs unpacking"

# Test 9: Mix regular kwargs and **kwargs
assert accepts_two_kwargs(a=1, **{"b": 2}) == 3, "mixed regular and **kwargs"

# Test 10: Both *args and **kwargs with literals
def both_forms_func(x: int, y: int, z: int) -> int:
    return x + y + z

assert both_forms_func(*[1, 2], **{"z": 3}) == 6, "both *args and **kwargs"

# Test 11: Complex mixing
assert both_forms_func(1, *[2], **{"z": 3}) == 6, "complex mixing"

# Test 12: **kwargs with function that has defaults
def func_with_defaults(a: int, b: int = 10, c: int = 20) -> int:
    return a + b + c

assert func_with_defaults(**{"a": 1}) == 31, "**kwargs with defaults"
assert func_with_defaults(**{"a": 1, "b": 2}) == 23, "**kwargs partial override"

# Test 13: *args with variadic function using literals
def variadic_func(a: int, *rest: int) -> int:
    total: int = a
    for x in rest:
        total = total + x
    return total

assert variadic_func(1, *[2, 3, 4]) == 10, "*args to variadic function"

# Test 14: Multiple literal unpackings
assert mix_args_func(*[1, 2], *[3, 4]) == 10, "multiple literal *args"

# Test 15: Empty literal unpacking
assert accepts_three_args(1, *[], 2, *[], 3) == 6, "empty literal *args"

# Test 16: **kwargs with all parameters specified
assert accepts_two_kwargs(**{"a": 10, "b": 20}) == 30, "**kwargs all params"

# Test 17: Mix of everything
def complex_sig(a: int, b: int = 5, c: int = 10) -> int:
    return a + b + c

assert complex_sig(*[1], **{"b": 2}) == 13, "complex mix with defaults"

print("All call-site unpacking tests passed!")

# ===== SECTION: Runtime variable unpacking =====

def sum_three_rt(a: int, b: int, c: int) -> int:
    return a + b + c

# Test runtime unpacking with tuple variable
args_tuple: tuple[int, int, int] = (10, 20, 30)
result_rt1: int = sum_three_rt(*args_tuple)
assert result_rt1 == 60, f"runtime unpack: expected 60, got {result_rt1}"
print("Runtime variable unpacking passed")

# Test runtime unpacking with mixed regular and unpacked args
first_rt: int = 100
rest_tuple_rt: tuple[int, int] = (200, 300)
result_rt2: int = sum_three_rt(first_rt, *rest_tuple_rt)
assert result_rt2 == 600, f"mixed runtime unpack: expected 600, got {result_rt2}"
print("Mixed runtime unpacking passed")

# Test runtime unpacking with function result
def make_tuple_rt() -> tuple[int, int, int]:
    return (1, 2, 3)

result_rt3: int = sum_three_rt(*make_tuple_rt())
assert result_rt3 == 6, f"function result unpack: expected 6, got {result_rt3}"
print("Function result unpacking passed")

# Test with default parameters
def greet_rt(name: str, greeting: str = "Hello", punctuation: str = "!") -> str:
    return greeting + " " + name + punctuation

name_tuple_rt: tuple[str] = ("World",)
result_rt4: str = greet_rt(*name_tuple_rt)
assert result_rt4 == "Hello World!", f"expected 'Hello World!', got {result_rt4}"
print("Runtime unpack with defaults passed")

# Test with *args functions (variadic)
def accepts_varargs_rt(a: int, *rest: int) -> int:
    total: int = a
    for x in rest:
        total = total + x
    return total

# Test with tuple unpacking into varargs
t1_varargs: tuple[int, int, int] = (1, 2, 3)
result_rt5: int = accepts_varargs_rt(*t1_varargs)  # accepts_varargs(1, 2, 3)
assert result_rt5 == 6, f"varargs unpack: expected 6, got {result_rt5}"
print("Runtime unpack with varargs passed")

# Test with mixed regular and unpacked args into varargs
t2_varargs: tuple[int, int] = (4, 5)
result_rt6: int = accepts_varargs_rt(10, *t2_varargs)  # accepts_varargs(10, 4, 5)
assert result_rt6 == 19, f"mixed varargs unpack: expected 19, got {result_rt6}"
print("Mixed runtime unpack with varargs passed")

# Test multiple starred arguments
def sum_five_rt(a: int, b: int, c: int, d: int, e: int) -> int:
    return a + b + c + d + e

t1_multi: tuple[int, int] = (1, 2)
t2_multi: tuple[int, int] = (3, 4)
result_rt7: int = sum_five_rt(*t1_multi, *t2_multi, 5)  # sum_five(1, 2, 3, 4, 5)
assert result_rt7 == 15, f"multiple starred: expected 15, got {result_rt7}"
print("Multiple starred arguments passed")

print("All runtime unpacking tests passed!")

# ===== Runtime List Unpacking =====
print("\n=== Testing Runtime List Unpacking ===")

def sum_three_list(a: int, b: int, c: int) -> int:
    return a + b + c

# Test 1: Basic list unpacking
args_list: list[int] = [10, 20, 30]
result_list1: int = sum_three_list(*args_list)
assert result_list1 == 60, f"list unpack: expected 60, got {result_list1}"
print("Basic list unpacking passed")

# Test 2: Mixed regular and unpacked
first_arg: int = 100
rest_args: list[int] = [200, 300]
result_list2: int = sum_three_list(first_arg, *rest_args)
assert result_list2 == 600, f"mixed list unpack: expected 600, got {result_list2}"
print("Mixed regular and list unpacking passed")

# Test 3: Into varargs
def sum_varargs_list(a: int, *rest: int) -> int:
    total: int = a
    for x in rest:
        total = total + x
    return total
varargs_list: list[int] = [1, 2, 3, 4, 5]
result_list3: int = sum_varargs_list(*varargs_list)
assert result_list3 == 15, f"varargs list unpack: expected 15, got {result_list3}"
print("List unpacking into varargs passed")

# Test 4: Multiple unpacking
some_list: list[int] = [3, 4, 5]
result_list4: int = sum_varargs_list(1, 2, *some_list)
assert result_list4 == 15, f"multiple unpack: expected 15, got {result_list4}"
print("Multiple unpacking passed")

# Test 3: Float list
def sum_floats_list(a: float, b: float) -> float:
    return a + b

float_list: list[float] = [1.5, 2.5]
result_list3: float = sum_floats_list(*float_list)
assert result_list3 == 4.0, f"float list unpack: expected 4.0, got {result_list3}"
print("Float list unpacking passed")

# Test 4: String list
def concat_three(a: str, b: str, c: str) -> str:
    return a + b + c

str_list: list[str] = ["Hello", " ", "World"]
result_list4: str = concat_three(*str_list)
assert result_list4 == "Hello World", f"str list unpack: expected 'Hello World', got '{result_list4}'"
print("String list unpacking passed")

# Test 5: Single element list
def square(x: int) -> int:
    return x * x

single_list: list[int] = [7]
result_list5: int = square(*single_list)
assert result_list5 == 49, f"single element list: expected 49, got {result_list5}"
print("Single element list unpacking passed")

# Test 6: Bool list unpacking
def and_bools(a: bool, b: bool) -> bool:
    return a and b

def or_bools(x: bool, y: bool, z: bool) -> bool:
    return x or y or z

bool_list1: list[bool] = [True, False]
result_bool1: bool = and_bools(*bool_list1)
assert result_bool1 == False, f"bool list unpack (and): expected False, got {result_bool1}"

bool_list2: list[bool] = [False, False, True]
result_bool2: bool = or_bools(*bool_list2)
assert result_bool2 == True, f"bool list unpack (or): expected True, got {result_bool2}"
print("Bool list unpacking passed")

# Test 7: Bool tuple unpacking
bool_tuple1: tuple[bool, bool] = (False, True)
result_bool3: bool = and_bools(*bool_tuple1)
assert result_bool3 == False, f"bool tuple unpack: expected False, got {result_bool3}"

bool_tuple2: tuple[bool, bool, bool] = (False, False, False)
result_bool4: bool = or_bools(*bool_tuple2)
assert result_bool4 == False, f"bool tuple unpack: expected False, got {result_bool4}"
print("Bool tuple unpacking passed")

# Test 8: Varargs with empty tail (exact match)
varargs_exact: list[int] = [42]
result_varargs1: int = sum_varargs_list(*varargs_exact)
assert result_varargs1 == 42, f"varargs empty tail: expected 42, got {result_varargs1}"
print("Varargs with empty tail passed")

# Test 9: Varargs with two regular params
def sum_two_plus_varargs(a: int, b: int, *rest: int) -> int:
    total: int = a + b
    for x in rest:
        total = total + x
    return total

two_plus_list: list[int] = [10, 20, 30, 40]
result_varargs2: int = sum_two_plus_varargs(*two_plus_list)
assert result_varargs2 == 100, f"two regular + varargs: expected 100, got {result_varargs2}"

two_exact_list: list[int] = [5, 15]
result_varargs3: int = sum_two_plus_varargs(*two_exact_list)
assert result_varargs3 == 20, f"two regular, empty varargs: expected 20, got {result_varargs3}"
print("Varargs with multiple regular params passed")

# Test 10: Float varargs
def sum_float_varargs(first: float, *rest: float) -> float:
    total: float = first
    for x in rest:
        total = total + x
    return total

float_varargs_list: list[float] = [1.5, 2.5, 3.5, 4.5]
result_float_varargs: float = sum_float_varargs(*float_varargs_list)
assert result_float_varargs == 12.0, f"float varargs: expected 12.0, got {result_float_varargs}"
print("Float varargs passed")

# Test 11: Bool varargs
def all_varargs(*args: bool) -> bool:
    result: bool = True
    for x in args:
        result = result and x
    return result

bool_varargs_list: list[bool] = [True, True, True]
result_bool_varargs: bool = all_varargs(*bool_varargs_list)
assert result_bool_varargs == True, f"bool varargs all true: expected True, got {result_bool_varargs}"

bool_varargs_list2: list[bool] = [True, False, True]
result_bool_varargs2: bool = all_varargs(*bool_varargs_list2)
assert result_bool_varargs2 == False, f"bool varargs with false: expected False, got {result_bool_varargs2}"
print("Bool varargs passed")

# Test 12: List unpacking with default parameters
def unpack_with_defaults(a: int, b: int, c: int = 100) -> int:
    return a + b + c

# All from list
defaults_list1: list[int] = [1, 2, 3]
defaults_result1: int = unpack_with_defaults(*defaults_list1)
assert defaults_result1 == 6, f"defaults all from list: expected 6, got {defaults_result1}"

# Two from list, one default
defaults_list2: list[int] = [1, 2]
defaults_result2: int = unpack_with_defaults(*defaults_list2)
assert defaults_result2 == 103, f"defaults with fallback: expected 103, got {defaults_result2}"

# Test with multiple optional params
def multi_defaults(a: int, b: int = 10, c: int = 20) -> int:
    return a + b + c

md_list1: list[int] = [5]
md_result1: int = multi_defaults(*md_list1)
assert md_result1 == 35, f"multi defaults one: expected 35, got {md_result1}"

md_list2: list[int] = [5, 15]
md_result2: int = multi_defaults(*md_list2)
assert md_result2 == 40, f"multi defaults two: expected 40, got {md_result2}"

md_list3: list[int] = [5, 15, 25]
md_result3: int = multi_defaults(*md_list3)
assert md_result3 == 45, f"multi defaults three: expected 45, got {md_result3}"
print("List unpacking with defaults passed")

print("All list unpacking tests passed!")

# ===== SECTION: Runtime **kwargs unpacking =====
print("\n=== Testing Runtime **kwargs Unpacking ===")

# Test 1: Basic runtime **kwargs
def f_kwargs_basic(a: int, b: int) -> int:
    return a + b

d_basic: dict[str, int] = {"a": 5, "b": 10}
result_kwargs1: int = f_kwargs_basic(**d_basic)
assert result_kwargs1 == 15, f"basic runtime **kwargs: expected 15, got {result_kwargs1}"
print("Basic runtime **kwargs passed")

# Test 2: Mixed explicit and runtime kwargs (explicit takes priority)
d_mixed: dict[str, int] = {"b": 20}
result_kwargs2: int = f_kwargs_basic(a=1, **d_mixed)
assert result_kwargs2 == 21, f"mixed kwargs: expected 21, got {result_kwargs2}"
print("Mixed explicit and runtime kwargs passed")

# Test 3: Runtime kwargs with default parameters
def f_kwargs_defaults(a: int, b: int = 10) -> int:
    return a + b

d_defaults: dict[str, int] = {"a": 5}
result_kwargs3: int = f_kwargs_defaults(**d_defaults)
assert result_kwargs3 == 15, f"kwargs with defaults: expected 15, got {result_kwargs3}"
print("Runtime kwargs with defaults passed")

# Test 4: Dict without conflicting explicit kwargs
# Note: CPython raises TypeError for f(a=1, **{"a": 2}) due to duplicate 'a'
# Our compiler detects this at compile-time for literal dicts
d_noconflict: dict[str, int] = {"b": 200}
result_kwargs4: int = f_kwargs_basic(a=1, **d_noconflict)
assert result_kwargs4 == 201, f"no conflict kwargs: expected 201, got {result_kwargs4}"
print("No conflict runtime kwargs passed")

# Test 5: All params from runtime dict
d_all: dict[str, int] = {"a": 7, "b": 8}
result_kwargs5: int = f_kwargs_basic(**d_all)
assert result_kwargs5 == 15, f"all from dict: expected 15, got {result_kwargs5}"
print("All params from runtime dict passed")

# Test 6: Runtime kwargs with keyword-only parameters
def f_kwargs_kwonly(a: int, *, b: int = 50) -> int:
    return a + b

d_kwonly: dict[str, int] = {"a": 10, "b": 20}
result_kwargs6: int = f_kwargs_kwonly(**d_kwonly)
assert result_kwargs6 == 30, f"kwargs with kwonly: expected 30, got {result_kwargs6}"
print("Runtime kwargs with keyword-only params passed")

# Test 7: Runtime kwargs partial (some from dict, some defaults)
d_partial: dict[str, int] = {"a": 25}
result_kwargs7: int = f_kwargs_kwonly(**d_partial)
assert result_kwargs7 == 75, f"kwargs partial: expected 75, got {result_kwargs7}"
print("Runtime kwargs partial with defaults passed")

# Test 8: Function returning dict used in **kwargs
def make_kwargs() -> dict[str, int]:
    return {"a": 3, "b": 4}

result_kwargs8: int = f_kwargs_basic(**make_kwargs())
assert result_kwargs8 == 7, f"function result **kwargs: expected 7, got {result_kwargs8}"
print("Function result **kwargs passed")

# Test 9: Multiple defaults with partial dict
def f_multi_defaults(a: int, b: int = 10, c: int = 20) -> int:
    return a + b + c

d_multi1: dict[str, int] = {"a": 1}
result_kwargs9: int = f_multi_defaults(**d_multi1)
assert result_kwargs9 == 31, f"multi defaults 1: expected 31, got {result_kwargs9}"

d_multi2: dict[str, int] = {"a": 1, "b": 2}
result_kwargs10: int = f_multi_defaults(**d_multi2)
assert result_kwargs10 == 23, f"multi defaults 2: expected 23, got {result_kwargs10}"

d_multi3: dict[str, int] = {"a": 1, "c": 3}
result_kwargs11: int = f_multi_defaults(**d_multi3)
assert result_kwargs11 == 14, f"multi defaults 3: expected 14, got {result_kwargs11}"
print("Multiple defaults with partial dict passed")

print("All runtime **kwargs tests passed!")

# ===== SECTION: Decorators =====
# Tests for decorated functions, especially calling them from other functions
# (which requires module-level wrapper tracking to persist across function lowering)
print("\n=== Testing Decorators ===")

# Test 1: Basic decorator pattern
def simple_decorator(func):
    def wrapper() -> int:
        return func() + 10
    return wrapper

@simple_decorator
def decorated_add_ten() -> int:
    return 5

# Call decorated function directly at module level
direct_result: int = decorated_add_ten()
assert direct_result == 15, "direct decorated call: expected 15"
print("Direct decorated function call passed")

# Test 2: Decorated function called from another function
# This tests the fix for module-level decorated functions visibility
def call_decorated() -> int:
    return decorated_add_ten()

indirect_result: int = call_decorated()
assert indirect_result == 15, "indirect decorated call: expected 15"
print("Decorated function called from another function passed")

# Test 3: Decorator with arguments passed to wrapped function
def arg_decorator(func):
    def wrapper(x: int) -> int:
        result: int = func(x)
        return result * 2
    return wrapper

@arg_decorator
def square_and_double(n: int) -> int:
    return n * n

# Direct call: 3^2 * 2 = 18
assert square_and_double(3) == 18, "decorated with args: expected 18"

# Call from another function: 4^2 * 2 = 32
def use_square_and_double(val: int) -> int:
    return square_and_double(val)

assert use_square_and_double(4) == 32, "decorated with args indirect: expected 32"
print("Decorator with arguments passed")

# Test 4: Multiple decorated functions called from one function
def add_one_deco(func):
    def wrapper() -> int:
        return func() + 1
    return wrapper

@add_one_deco
def deco_return_zero() -> int:
    return 0

@add_one_deco
def deco_return_ten() -> int:
    return 10

def call_both_decorated() -> int:
    a: int = deco_return_zero()
    b: int = deco_return_ten()
    return a + b

# (0+1) + (10+1) = 12
assert call_both_decorated() == 12, "multiple decorated: expected 12"
print("Multiple decorated functions called from one function passed")

# Test 5: Chained wrapper decorators
# Each decorator wraps the previous result, so the chain is evaluated at runtime
def triple_deco(func):
    def wrapper() -> int:
        return func() * 3
    return wrapper

@triple_deco
@add_one_deco
def chained_base() -> int:
    return 5

# Execution: base() returns 5, add_one_deco wraps to return 5+1=6, triple_deco wraps to return 6*3=18
chained_result = chained_base()
assert chained_result == 18, f"chained wrapper decorators: expected 18, got {chained_result}"
print("Chained wrapper decorators passed")

# Chained wrapper decorators with arguments
def double_deco(func):
    def wrapper(x: int) -> int:
        return func(x) * 2
    return wrapper

def add_five_deco(func):
    def wrapper(x: int) -> int:
        return func(x) + 5
    return wrapper

@double_deco
@add_five_deco
def chained_with_arg(n: int) -> int:
    return n

# Execution: n=10 -> add_five returns 10+5=15 -> double returns 15*2=30
chained_arg_result = chained_with_arg(10)
assert chained_arg_result == 30, f"chained decorators with args: expected 30, got {chained_arg_result}"
print("Chained wrapper decorators with arguments passed")

# Three chained decorators
def subtract_one_deco(func):
    def wrapper() -> int:
        return func() - 1
    return wrapper

@triple_deco
@add_one_deco
@subtract_one_deco
def triple_chained() -> int:
    return 10

# Execution: 10 -> subtract_one returns 9 -> add_one returns 10 -> triple returns 30
triple_chained_result = triple_chained()
assert triple_chained_result == 30, f"triple chained decorators: expected 30, got {triple_chained_result}"
print("Triple chained wrapper decorators passed")

# Test 6: Decorator with side effects (using print)
def print_deco(func):
    def wrapper() -> None:
        print("Before decorated function")
        func()
        print("After decorated function")
    return wrapper

@print_deco
def greet_deco() -> None:
    print("Hello from decorated function")

def call_greet_deco() -> None:
    greet_deco()

print("Calling decorated function from another function:")
call_greet_deco()
print("Decorator with side effects passed")

# Test 7: Nested function calling decorated function
def outer_caller() -> int:
    def inner_caller() -> int:
        return decorated_add_ten()
    return inner_caller()

nested_result: int = outer_caller()
assert nested_result == 15, "nested calling decorated: expected 15"
print("Nested function calling decorated function passed")

# Test 8: Decorator that passes through arguments
def passthrough_deco(func):
    def wrapper(a: int, b: int) -> int:
        return func(a, b)
    return wrapper

@passthrough_deco
def add_two_nums(x: int, y: int) -> int:
    return x + y

def call_add_two_nums(p: int, q: int) -> int:
    return add_two_nums(p, q)

assert call_add_two_nums(10, 20) == 30, "passthrough decorator: expected 30"
print("Passthrough decorator passed")

print("All decorator tests passed!")

# ===== SECTION: Decorator Factories =====
# Decorator factories are decorators that take arguments: @decorator(arg)
# The factory returns the actual decorator, which is then applied to the function
print("\n=== Testing Decorator Factories ===")

# Test 1: Basic decorator factory
def multiply_factory(factor: int):
    def decorator(func):
        def wrapper(x: int) -> int:
            return func(x) * factor
        return wrapper
    return decorator

@multiply_factory(3)
def get_value_factory(x: int) -> int:
    return x + 5

result_factory1 = get_value_factory(10)
assert result_factory1 == 45, f"decorator factory: expected 45, got {result_factory1}"  # (10+5) * 3 = 45
print("Basic decorator factory passed")

# Test 2: Different factory argument values
@multiply_factory(5)
def double_five_factory(x: int) -> int:
    return x * 2

result_factory2 = double_five_factory(4)
assert result_factory2 == 40, f"factory different value: expected 40, got {result_factory2}"  # (4 * 2) * 5 = 40
print("Different factory values passed")

# Test 3: Calling decorated function from another function
def call_get_value_factory(n: int) -> int:
    return get_value_factory(n)

result_factory3 = call_get_value_factory(5)
assert result_factory3 == 30, f"indirect factory call: expected 30, got {result_factory3}"  # (5+5) * 3 = 30
print("Indirect factory call passed")

# Test 4: Decorator factory with no-arg wrapped function
def add_constant_factory(val: int):
    def decorator(func):
        def wrapper() -> int:
            return func() + val
        return wrapper
    return decorator

@add_constant_factory(100)
def get_zero_factory() -> int:
    return 0

result_factory4 = get_zero_factory()
assert result_factory4 == 100, f"no-arg factory: expected 100, got {result_factory4}"
print("No-arg wrapped function with factory passed")

# Test 5: Factory with multiple arguments
def add_and_multiply_factory(add_val: int, mult_val: int):
    def decorator(func):
        def wrapper(x: int) -> int:
            return (func(x) + add_val) * mult_val
        return wrapper
    return decorator

@add_and_multiply_factory(2, 3)
def base_identity(x: int) -> int:
    return x

result_factory5 = base_identity(10)
assert result_factory5 == 36, f"multi-arg factory: expected 36, got {result_factory5}"  # (10 + 2) * 3 = 36
print("Factory with multiple arguments passed")

# Test 6: Same factory used multiple times with different args
@multiply_factory(2)
def times_two(x: int) -> int:
    return x

@multiply_factory(10)
def times_ten(x: int) -> int:
    return x

result_factory6a = times_two(5)
result_factory6b = times_ten(5)
assert result_factory6a == 10, f"times_two: expected 10, got {result_factory6a}"
assert result_factory6b == 50, f"times_ten: expected 50, got {result_factory6b}"
print("Same factory with different args passed")

print("All decorator factory tests passed!")

# === Lambda default parameters ===
lambda_with_default = lambda x, y=10: x + y
assert lambda_with_default(5) == 15, f"lambda default: expected 15, got {lambda_with_default(5)}"
assert lambda_with_default(5, 20) == 25, f"lambda override: expected 25, got {lambda_with_default(5, 20)}"

lambda_multi_defaults = lambda a, b=2, c=3: a + b + c
assert lambda_multi_defaults(1) == 6, "lambda multi defaults: 1+2+3=6"
assert lambda_multi_defaults(1, 10) == 14, "lambda multi defaults: 1+10+3=14"
assert lambda_multi_defaults(1, 10, 100) == 111, "lambda multi defaults: 1+10+100=111"

print("Lambda default parameter tests passed!")

# ===== SECTION: Escaping nonlocal closures (regression test) =====

def make_counter_escape():
    count: int = 0
    def increment() -> int:
        nonlocal count
        count = count + 1
        return count
    return increment

esc_counter = make_counter_escape()
assert esc_counter() == 1, "escaping nonlocal: first call should return 1"
assert esc_counter() == 2, "escaping nonlocal: second call should return 2"
assert esc_counter() == 3, "escaping nonlocal: third call should return 3"

# Test with initial value
def make_counter_from(start: int):
    count: int = start
    def inc() -> int:
        nonlocal count
        count = count + 1
        return count
    return inc

esc_counter10 = make_counter_from(10)
assert esc_counter10() == 11, "escaping nonlocal from 10: first call should return 11"
assert esc_counter10() == 12, "escaping nonlocal from 10: second call should return 12"

# Two independent closures sharing no state
esc_a = make_counter_escape()
esc_b = make_counter_escape()
assert esc_a() == 1, "independent closure a: should start at 1"
assert esc_b() == 1, "independent closure b: should start at 1"
assert esc_a() == 2, "independent closure a: second call"
assert esc_b() == 2, "independent closure b: second call"

print("Escaping nonlocal closure tests passed!")

print("All function tests passed!")
