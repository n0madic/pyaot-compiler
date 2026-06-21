# Consolidated test file for functions

from typing import Callable

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

# ===== SECTION: Three-level closure nesting =====

def test_lambda_grandparent_capture() -> None:
    """Lambda inside nested function captures from grandparent scope"""
    x: int = 10
    def middle() -> int:
        f = lambda: x  # lambda captures x from grandparent
        return f()
    result: int = middle()
    assert result == 10, "lambda grandparent capture should equal 10"

def test_lambda_in_nested_with_args() -> None:
    """Lambda with args captures from grandparent"""
    base: int = 100
    def middle() -> int:
        adder = lambda y: base + y  # captures base from grandparent
        return adder(5)
    result: int = middle()
    assert result == 105, "lambda with args grandparent capture should equal 105"

def test_four_level_readonly() -> None:
    """Four-level read-only capture"""
    x: int = 1
    def level1() -> int:
        def level2() -> int:
            def level3() -> int:
                return x
            return level3()
        return level2()
    result: int = level1()
    assert result == 1, "four-level read-only capture should equal 1"

def test_four_level_nonlocal() -> None:
    """Four-level nonlocal chain"""
    counter: int = 0
    def level1() -> None:
        nonlocal counter
        def level2() -> None:
            nonlocal counter
            def level3() -> None:
                nonlocal counter
                counter = counter + 10
            level3()
        level2()
    level1()
    assert counter == 10, "four-level nonlocal should equal 10"

def test_returned_closure_three_levels() -> None:
    """Closure returned from three-level nesting (deferred execution)"""
    x: int = 42
    def middle():
        def inner() -> int:
            return x
        return inner
    f = middle()
    result: int = f()
    assert result == 42, "returned closure three-level should equal 42"

def test_mixed_capture_levels() -> None:
    """Captures from multiple ancestor scopes"""
    a: int = 1
    def level1() -> int:
        b: int = 2
        def level2() -> int:
            c: int = 3
            def level3() -> int:
                return a + b + c  # captures from three different levels
            return level3()
        return level2()
    result: int = level1()
    assert result == 6, "mixed capture levels should equal 6"

def test_lambda_chain_capture() -> None:
    """Lambda captures variable that was itself captured by enclosing function"""
    x: int = 7
    def middle() -> int:
        f = lambda y: x + y
        return f(3)
    result: int = middle()
    assert result == 10, "lambda chain capture should equal 10"

def test_curried_chain_three_call_int() -> None:
    """Curried 3-level closure factory called externally as `chain(a)(b)(c)`,
    result assigned to an `int`-typed local. The dispatcher synthesized by
    `emit_capture_dispatch` allocates each n_captures branch with a uniform
    `result_ty`, but the static call-graph filter narrows different branches
    to functions with different return types (inner1 → Any/closure, inner2 →
    Int). Without `abi_repair`'s Refine bridge at the Call→CallDirect
    narrowing point, `type_inference` widens the dispatch dest to Union, and
    the merge-block Phi boxes Int sources into tagged `Value` bits — printing
    `49` for payload `6` (`(6 << 3) | 1`) instead of `6`."""
    def chain(a: int):
        def inner1(b: int):
            def inner2(c: int) -> int:
                return a + b + c
            return inner2
        return inner1
    result: int = chain(1)(2)(3)
    assert result == 6, "curried chain(1)(2)(3) with int annotation should equal 6"

def test_curried_chain_three_call_no_annotation() -> None:
    """Same curried 3-level factory, but the result variable has no
    annotation — exercises the no-anno path where the assignment's
    `var_type` is widened to Union by prescan, forcing the value to be
    boxed before storage. Pre-fix this either printed garbage or SEGV-d
    when downstream consumers (`rt_print_obj`) decoded raw int bits as
    a heap pointer."""
    def chain(a: int):
        def inner1(b: int):
            def inner2(c: int) -> int:
                return a + b + c
            return inner2
        return inner1
    result_no_anno = chain(1)(2)(3)
    assert result_no_anno == 6, "curried chain(1)(2)(3) no annotation should equal 6"

def test_curried_chain_direct_print() -> None:
    """The curried chain called and consumed directly (no intermediate typed
    local). Under the uniform value-call ABI every closure shares one
    `(args, kwargs) -> Value` entry, so the genuinely-`Dyn` intermediate results
    (`chain(1)` / `chain(1)(2)`) are callable through the single indirect ABI and
    the final `int` is boxed to a tagged `Value` — `print` / `==` consume it
    correctly. (Formerly a SEGV: the multi-candidate dispatch wrote raw int bits
    into an Any-typed dest that `rt_print_obj` decoded as a heap pointer.)"""
    def chain(a: int):
        def inner1(b: int):
            def inner2(c: int) -> int:
                return a + b + c
            return inner2
        return inner1
    print(chain(1)(2)(3))
    assert chain(2)(3)(4) == 9, "curried chain consumed directly should equal 9"

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

# ===== SECTION: Lambda body containing comprehensions =====
# Regression: list/dict/set comprehensions desugar by pushing init+loop
# stmts into `scope.pending_stmts`, expecting the enclosing context to
# flush them before the value is read. `convert_lambda` previously wrapped
# only the single body expression in a Return statement and dropped the
# pending stmts on the floor — so `lambda n: [i for i in range(n)]`
# returned a fresh empty list (or `None` when the empty-list bind was
# itself dropped). Lambdas now harvest pending stmts before emitting
# the return.

# Plain list-comp inside a lambda body
lc_lambda = lambda n: [i for i in range(n)]
lc_result = lc_lambda(4)
assert lc_result == [0, 1, 2, 3], f"lambda + listcomp expected [0,1,2,3], got {lc_result}"

# List-comp with a captured outer variable
lc_capture_offset: int = 100
lc_capture_lambda = lambda n: [i + lc_capture_offset for i in range(n)]
lc_capture_result = lc_capture_lambda(3)
assert lc_capture_result == [100, 101, 102], (
    f"lambda + listcomp with capture expected [100,101,102], got {lc_capture_result}"
)

# Nested list-comp (matrix builder — the microgpt.py `matrix` lambda shape)
matrix_lambda = lambda rows, cols: [[0 for _ in range(cols)] for _ in range(rows)]
matrix_result = matrix_lambda(2, 3)
assert len(matrix_result) == 2, f"matrix outer len: {len(matrix_result)}"
assert matrix_result[0] == [0, 0, 0], f"matrix row 0: {matrix_result[0]}"
assert matrix_result[1] == [0, 0, 0], f"matrix row 1: {matrix_result[1]}"

# Dict-comp inside a lambda body. Compare element-wise — direct
# `dict == dict` literal-equality has a pre-existing pyaot quirk
# unrelated to this fix.
dict_lambda = lambda n: {i: i * i for i in range(n)}
dict_result = dict_lambda(3)
assert len(dict_result) == 3, f"lambda + dictcomp len: {len(dict_result)}"
assert dict_result[0] == 0 and dict_result[1] == 1 and dict_result[2] == 4, (
    f"lambda + dictcomp values: {dict_result}"
)

# Set-comp inside a lambda body. Compare via membership / size — set
# literal equality has the same caveat as dicts at module scope.
set_lambda = lambda n: {i % 3 for i in range(n)}
set_result = set_lambda(7)
assert len(set_result) == 3, f"lambda + setcomp len: {len(set_result)}"
assert 0 in set_result and 1 in set_result and 2 in set_result, (
    f"lambda + setcomp members: {set_result}"
)

# Lambda used as a callback whose comprehension references a captured var
items = [1, 2, 3]
build_pairs = lambda s: [(it, s) for it in items]
pairs_result = build_pairs("x")
assert pairs_result == [(1, "x"), (2, "x"), (3, "x")], (
    f"lambda + listcomp + capture mismatch: {pairs_result}"
)

# Lambda inside a list-comp (outer scope's pending stmts must not leak
# into the lambda body, and the lambda's pending stmts must not leak
# back into the outer comp). Compare element-wise.
outer_listcomp_with_lambda = [(lambda y: [y * j for j in range(3)])(i) for i in range(2)]
assert len(outer_listcomp_with_lambda) == 2
assert outer_listcomp_with_lambda[0][0] == 0 and outer_listcomp_with_lambda[0][1] == 0 and outer_listcomp_with_lambda[0][2] == 0
assert outer_listcomp_with_lambda[1][0] == 0 and outer_listcomp_with_lambda[1][1] == 1 and outer_listcomp_with_lambda[1][2] == 2

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

# All-kwargs call with more than 2 params (regression: type checker false positive)
def add_three_kw(x: int, y: int, z: int) -> int:
    return x + y + z

assert add_three_kw(x=1, y=2, z=3) == 6, "all kwargs in order"
assert add_three_kw(z=3, y=2, x=1) == 6, "all kwargs reverse order"
assert add_three_kw(y=10, x=5, z=20) == 35, "all kwargs arbitrary order"

# Mixed positional and kwargs with reordering
assert add_three_kw(1, z=3, y=2) == 6, "positional + reordered kwargs"

print("Keyword argument tests passed!")

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
test_lambda_grandparent_capture()
test_lambda_in_nested_with_args()
test_four_level_readonly()
test_four_level_nonlocal()
test_returned_closure_three_levels()
test_mixed_capture_levels()
test_lambda_chain_capture()
test_curried_chain_three_call_int()
test_curried_chain_three_call_no_annotation()
test_curried_chain_direct_print()

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

assert count_kwargs(a=1, b=2, c=3) == 3
assert count_kwargs() == 0
assert count_kwargs(x=10, y=20) == 2

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

# ===== SECTION: Return type inference for unannotated functions =====

def _rti_double(x: int):
    return x * 2
assert _rti_double(21) == 42, "return type inference: int arithmetic"

def _rti_greet(name: str):
    return "Hello, " + name
assert _rti_greet("World") == "Hello, World", "return type inference: str concat"

def _rti_is_positive(x: int):
    return x > 0
assert _rti_is_positive(5) == True, "return type inference: comparison true"
assert _rti_is_positive(-3) == False, "return type inference: comparison false"

def _rti_first(items: list[int]):
    return items[0]
assert _rti_first([10, 20, 30]) == 10, "return type inference: list indexing"

def _rti_add(a: int, b: int):
    return a + b
assert _rti_add(3, 4) == 7, "return type inference: two int params"

# Nested calls with inferred return types
def _rti_square(x: int):
    return x * x

def _rti_sum_squares(a: int, b: int):
    return _rti_square(a) + _rti_square(b)

assert _rti_sum_squares(3, 4) == 25, "nested calls with inferred return: 3² + 4² = 25"

# Bidirectional: empty containers in function args
def _rti_sum_list(items: list[int]) -> int:
    return sum(items)
assert _rti_sum_list([]) == 0, "bidirectional: empty list in function arg"

# Bidirectional: empty containers in return
def _rti_empty_int_list() -> list[int]:
    return []
_rti_empty = _rti_empty_int_list()
assert len(_rti_empty) == 0, "bidirectional: empty list in return"

print("Return type inference tests passed!")

# ===== Regression: closure with no captures as key function =====
# Non-capturing closures (defined as inner functions) should work in sorted/min/max.

def test_non_capturing_key():
    def by_length(s: str) -> int:
        return len(s)

    words: list[str] = ["banana", "fig", "apple"]
    result: list[str] = sorted(words, key=by_length)
    assert result == ["fig", "apple", "banana"], f"non-capturing key: {result}"

test_non_capturing_key()
print("Non-capturing closure as key function: PASS")

# ===== Regression: function passed as argument =====
def apply_fn(f, x: int) -> int:
    return f(x)

def double_it(x: int) -> int:
    return x * 2

assert apply_fn(double_it, 5) == 10, "function passed as argument"
assert apply_fn(double_it, 0) == 0, "function passed as argument (zero)"

print("Function passed as argument: PASS")

# ===== Regression: HOF key= callback in a `return` terminator =====
# The phase4 safety scan must visit block terminators (`return ...`), not
# only block statements — `return`/`raise`/branch/`yield` are HirTerminator
# variants. A `sorted(..., key=lambda ...)` returned directly must still
# have its key lambda marked phase4_unsafe so it is NOT return-ABI-flipped;
# otherwise the raw-scalar sort-key ABI mismatches and the sort misbehaves.
def sort_pairs_by_second(pairs):
    return sorted(pairs, key=lambda p: p[1])

_hof_pairs = [(1, 30), (2, 10), (3, 20)]
_hof_sorted = sort_pairs_by_second(_hof_pairs)
assert _hof_sorted == [(2, 10), (3, 20), (1, 30)], f"key= in return: {_hof_sorted}"
print("HOF key= callback in return position: PASS")

# ===== Regression: HOF key= callback inside an f-string format spec =====
# A `sorted(..., key=lambda ...)` embedded in `f"{...}"` lives inside a
# FormatSpec node. The closure-capture pre-scan must recurse through
# FormatSpec to harvest the key lambda's parameter type; otherwise the
# param stays `Any`, the lambda is lowered with the tagged-Value ABI
# while `sorted` delivers raw scalars, and the lambda reads garbage
# (previously surfaced as `TypeError: unary - on NoneType`).
_hof_nums = [5, 1, 4, 2, 3]
_hof_msg = f"min-by-neg={sorted(_hof_nums, key=lambda n: -n)[-1]}"
assert _hof_msg == "min-by-neg=1", f"key= in f-string: {_hof_msg}"
print("HOF key= callback in f-string format spec: PASS")

# ===== Whole-project code-review regression: complete free-var capture
# coverage — closures referencing an outer var from a loop `else`, `try`/`else`,
# `match` case, slice bounds or a walrus value (formerly test_review_wave3a.py).
def _rv_for_else_capture() -> int:
    base = 100

    def inner() -> int:
        total = 0
        for i in range(3):
            total += i
        else:
            total += base
        return total

    return inner()


def _rv_while_else_capture() -> int:
    bonus = 7

    def inner() -> int:
        total = 0
        n = 0
        while n < 3:
            total += n
            n += 1
        else:
            total += bonus
        return total

    return inner()


def _rv_try_else_capture() -> int:
    extra = 11

    def inner() -> int:
        total = 0
        try:
            total += 1
        except ValueError:
            total += 999
        else:
            total += extra
        return total

    return inner()


def _rv_match_capture(n: int) -> int:
    factor = 10

    def inner(x: int) -> int:
        match x:
            case 0:
                return factor
            case _:
                return x * factor

    return inner(n)


def _rv_slice_capture() -> list[int]:
    lo = 1
    hi = 3
    data = [10, 20, 30, 40, 50]

    def inner() -> list[int]:
        return data[lo:hi]

    return inner()


def _rv_walrus_capture() -> int:
    seed = 5

    def inner() -> int:
        return (x := seed + 1) + x

    return inner()


print(_rv_for_else_capture())
print(_rv_while_else_capture())
print(_rv_try_else_capture())
print(_rv_match_capture(0))
print(_rv_match_capture(3))
print(_rv_slice_capture())
print(_rv_walrus_capture())


# Regression: a function that returns a value on one path and falls off / bare
# `return`s (→ None) on another. Under -O the function is inlined and the
# value/None paths merge into one dest; the None path used to be stored as a
# raw `i8 0` and read back as `False` instead of `None`.
def _maybe(x: int):
    if x:
        return x
    return


def _test_mixed_value_void() -> None:
    assert _maybe(0) is None, "falls off -> None"
    assert _maybe(5) == 5, "value path returns 5"
    # Loop where both the value and the None path are live across iterations.
    acc = 0
    for v in [0, 5, 0, 9]:
        r = _maybe(v)
        if r is not None:
            acc += r
    assert acc == 14, "sum of non-None results"
    print("mixed value/void return test passed")


_test_mixed_value_void()


# Regression: UNANNOTATED capturing closures called by value. The solver used
# to resolve a call to a capturing nested def / lambda held in a variable as a
# dynamic (Any) call, so the enclosing function's return type stayed Any at
# lowering time while WPA narrowed the value to a raw primitive — the consumer
# then read the raw bits through a tagged lens (`True` instead of 43) or
# crashed on an indirect-call ABI mismatch (assign-then-call).
def _closure_immediate():
    x = 42

    def inner():
        return x + 1

    return inner()


def _closure_assign_then_call():
    x = 42

    def inner():
        return x + 1

    g = inner
    return g()


def _closure_lambda_arith():
    c = 100
    g = lambda a, b: a + b - c
    return g(20, 5)


def _test_unannotated_capturing_closures() -> None:
    assert _closure_immediate() == 43, "immediate capturing-closure call"
    assert _closure_assign_then_call() == 43, "assign-then-call capturing closure"
    assert _closure_lambda_arith() == -75, "capturing lambda arithmetic"
    print("unannotated capturing closure test passed")


_test_unannotated_capturing_closures()

# ===== SECTION: Folded point-tests (p2/p6/p10/p13/p36-p41 + decorator factory & class decorators) =====

# ===== FOLDED: p2_funcs.py (basic functions / recursion / classify) =====
def _fold_p2_funcs():
    def add(a: int, b: int) -> int:
        return a + b

    def factorial(n: int) -> int:
        if n <= 1:
            return 1
        return n * factorial(n - 1)

    def fib(n: int) -> int:
        if n < 2:
            return n
        return fib(n - 1) + fib(n - 2)

    def is_even(n: int) -> bool:
        return n % 2 == 0

    def area(r: float) -> float:
        return 3.14159 * r * r

    def greet(name: str) -> str:
        return name

    def countdown(n: int) -> int:
        total = 0
        while n > 0:
            total = total + n
            n = n - 1
        return total

    def classify(n: int) -> str:
        if n < 0:
            return "negative"
        elif n == 0:
            return "zero"
        else:
            return "positive"

    assert add(3, 4) == 7
    assert factorial(5) == 120
    assert fib(10) == 55
    assert is_even(4) == True
    assert is_even(7) == False
    assert area(2.0) == 12.56636
    assert greet("hello") == "hello"
    assert countdown(5) == 15
    assert classify(-3) == "negative"
    assert classify(0) == "zero"
    assert classify(42) == "positive"
    assert add(factorial(4), fib(7)) == 37


_fold_p2_funcs()


# ===== FOLDED: p6_closures.py (nested defs, returned closures, late binding) =====
def _fold_p6_closures():
    # nested def reading an enclosing local (basic capture)
    def twice(x: int) -> int:
        def inner(y: int) -> int:
            return y * 2
        return inner(x)

    assert twice(21) == 42

    # returned closures: independent cells per activation
    def make_adder(n: int) -> Callable[[int], int]:
        def add(x: int) -> int:
            return x + n
        return add

    add5 = make_adder(5)
    add7 = make_adder(7)
    assert add5(1) == 6
    assert add7(1) == 8
    assert add5(add7(0)) == 12

    # outer rebinding visible through the shared cell (late binding)
    def make_getter() -> int:
        x = 10

        def get() -> int:
            return x

        first = get()
        x = 20
        return first * 1000 + get()

    assert make_getter() == 10020

    # recursion via self-capture
    def make_fact() -> Callable[[int], int]:
        def fact(n: int) -> int:
            if n <= 1:
                return 1
            return n * fact(n - 1)
        return fact

    fact = make_fact()
    assert fact(6) == 720

    # transitive (two-level) capture bubbling
    def outer(a: int) -> int:
        def mid(b: int) -> int:
            def inner(c: int) -> int:
                return a + b + c
            return inner(b * 10)
        return mid(a * 10)

    assert outer(3) == 333

    # stored closures: a list of function values, called by index
    fs: list[Callable[[], int]] = []

    def make_const(k: int) -> Callable[[], int]:
        def const() -> int:
            return k
        return const

    for i in range(3):
        fs.append(make_const(i * 11))
    assert fs[0]() == 0
    assert fs[1]() == 11
    assert fs[2]() == 22

    # classic late-binding pitfall: all loop closures see the FINAL i
    def make_loop_closures() -> list[Callable[[], int]]:
        out: list[Callable[[], int]] = []
        for i in range(3):
            def f() -> int:
                return i
            out.append(f)
        return out

    loop_fs = make_loop_closures()
    assert loop_fs[0]() == 2
    assert loop_fs[1]() == 2
    assert loop_fs[2]() == 2

    # multiple captures of mixed types
    def describe(name: str, base: int) -> Callable[[int], str]:
        def fmt(extra: int) -> str:
            return name + ": " + str(base + extra)
        return fmt

    f = describe("total", 100)
    assert f(11) == "total: 111"
    assert f(22) == "total: 122"

    # a top-level function used as a value (thunk)
    def square(x: int) -> int:
        return x * x

    def apply(g: Callable[[int], int], x: int) -> int:
        return g(x)

    assert apply(square, 4) == 16
    assert apply(make_adder(3), 4) == 7


_fold_p6_closures()


# ===== FOLDED: p6_lambda_hof.py (lambdas, HOFs, compose, late binding) =====
def _fold_p6_lambda_hof():
    # plain lambdas (no capture)
    double = lambda x: x * 2
    assert double(21) == 42

    add = lambda a, b: a + b
    assert add(3, 4) == 7

    # lambda capturing an enclosing binding (here: a local)
    base = 100
    shift = lambda d: base + d
    assert shift(1) == 101

    # lambdas passed to user higher-order functions
    def apply(f: Callable[[int], int], x: int) -> int:
        return f(x)

    assert apply(lambda y: y + 1, 41) == 42
    assert apply(double, 5) == 10

    def apply_twice(f: Callable[[int], int], x: int) -> int:
        return f(f(x))

    assert apply_twice(lambda n: n * 3, 2) == 18

    # a HOF returning a lambda (capture of both params)
    def compose(f: Callable[[int], int], g: Callable[[int], int]) -> Callable[[int], int]:
        return lambda x: f(g(x))

    inc = lambda v: v + 1
    assert compose(double, inc)(10) == 22
    assert compose(inc, double)(10) == 21

    # lambda capturing the loop variable inside a function (late binding)
    def lambda_rows() -> list[Callable[[int], int]]:
        rows: list[Callable[[int], int]] = []
        for k in range(3):
            rows.append(lambda x: x + k)
        return rows

    rows = lambda_rows()
    assert rows[0](100) == 102
    assert rows[1](100) == 102
    assert rows[2](100) == 102

    # conditional lambda selection
    def pick(flag: bool) -> Callable[[int], int]:
        if flag:
            return lambda n: n - 1
        return lambda n: n + 1

    assert pick(True)(10) == 9
    assert pick(False)(10) == 11

    # lambdas over strings
    shout = lambda s: s + "!"
    assert shout("hello") == "hello!"


_fold_p6_lambda_hof()


# ===== FOLDED: p6_nonlocal_global.py (nonlocal/global cells) =====
def _fold_p6_nonlocal_global():
    # counter closure: nonlocal rebinding through the shared cell
    def make_counter() -> Callable[[], int]:
        n = 0

        def inc() -> int:
            nonlocal n
            n = n + 1
            return n

        return inc

    c1 = make_counter()
    c2 = make_counter()
    assert c1() == 1
    assert c1() == 2
    assert c1() == 3
    assert c2() == 1

    # augmented nonlocal assignment
    def make_acc(start: int) -> Callable[[int], int]:
        total = start

        def add(x: int) -> int:
            nonlocal total
            total += x
            return total

        return add

    acc = make_acc(100)
    assert acc(1) == 101
    assert acc(2) == 103
    assert acc(3) == 106

    # two-level bubbling: the innermost writes the outermost's cell
    def level0() -> int:
        v = 1

        def level1() -> int:
            def level2() -> None:
                nonlocal v
                v = v * 10

            level2()
            level2()
            return v

        return level1()

    assert level0() == 100

    # two closures sharing one cell (reader + writer)
    def make_pair() -> int:
        state = 5

        def read() -> int:
            return state

        def bump() -> None:
            nonlocal state
            state = state + 1

        bump()
        bump()
        return read()

    assert make_pair() == 7

    # the late-binding list: every closure sees the final loop value
    fs: list[Callable[[], int]] = []
    for i in range(3):
        fs.append(lambda: i)
    assert [f() for f in fs] == [2, 2, 2]


_fold_p6_nonlocal_global()


# ===== FOLDED: p6_nonlocal_global.py (module-global mutation) =====
# Kept at module scope because it exercises module-level `global` statements.
_png_count = 0


def _png_bump_global() -> None:
    global _png_count
    _png_count = _png_count + 1


def _png_show_count() -> int:
    return _png_count


_png_bump_global()
_png_bump_global()
_png_bump_global()
assert _png_count == 3
assert _png_show_count() == 3

_png_scale = 7


def _png_scaled(x: int) -> int:
    return x * _png_scale


assert _png_scaled(6) == 42
_png_scale = 9
assert _png_scaled(6) == 54


# ===== FOLDED: p6_varargs.py (*args / **kwargs collection) =====
def _fold_p6_varargs():
    def total(*nums):
        s = 0
        for n in nums:
            s += n
        return s

    assert total() == 0
    assert total(1, 2, 3) == 6
    assert total(10, 20) == 30

    def greet(greeting, *names):
        out = greeting
        for n in names:
            out += " " + n
        return out

    assert greet("Hi") == "Hi"
    assert greet("Hi", "Alice", "Bob") == "Hi Alice Bob"

    def describe(**attrs):
        keys = sorted(attrs.keys())
        out = ""
        for k in keys:
            out += k + "=" + str(attrs[k]) + ";"
        return out

    assert describe() == ""
    assert describe(a=1, b=2) == "a=1;b=2;"
    assert describe(z=26, a=1, m=13) == "a=1;m=13;z=26;"

    def both(first, *rest, **opts):
        return str(first) + "/" + str(len(rest)) + "/" + str(len(opts))

    assert both(1) == "1/0/0"
    assert both(1, 2, 3) == "1/2/0"
    assert both(1, 2, x=9) == "1/1/1"
    assert both(0, 1, 2, 3, a=1, b=2) == "0/3/2"

    # forwarding *args through another varargs function
    def forward(*a):
        return total(*a)

    assert forward(4, 5, 6) == 15
    assert forward() == 0

    # len() and iteration over the *args tuple
    def count_and_sum(*xs):
        return len(xs)

    assert count_and_sum(1, 2, 3, 4) == 4


_fold_p6_varargs()


# ===== FOLDED: p6_defaults_kwargs.py (defaults, keyword reordering, kwonly) =====
# Module scope: keyword arguments bound to regular positional parameters are only
# supported on top-level functions, so these defs stay at module level (`_pdk_`).
def _pdk_power(base, exp=2):
    result = 1
    for _ in range(exp):
        result = result * base
    return result


def _pdk_greet(name, greeting="Hello", punct="!"):
    return greeting + ", " + name + punct


def _pdk_make(width, *, height=10, label="box"):
    return label + ":" + str(width) + "x" + str(height)


def _pdk_config(name, **opts):
    base = name
    keys = sorted(opts.keys())
    for k in keys:
        base += " " + k + "=" + str(opts[k])
    return base


def _pdk_flags(a=1, b=True, c="x", d=None):
    return str(a) + "/" + str(b) + "/" + c + "/" + str(d)


def _fold_p6_defaults_kwargs():
    # positional defaults
    assert _pdk_power(5) == 25
    assert _pdk_power(5, 3) == 125
    assert _pdk_power(base=4) == 16
    assert _pdk_power(exp=3, base=2) == 8

    # multiple defaults, keyword reordering
    assert _pdk_greet("World") == "Hello, World!"
    assert _pdk_greet("World", "Hi") == "Hi, World!"
    assert _pdk_greet("World", punct="?") == "Hello, World?"
    assert _pdk_greet("World", greeting="Hey", punct=".") == "Hey, World."
    assert _pdk_greet(greeting="Yo", name="Sam") == "Yo, Sam!"

    # keyword-only parameters
    assert _pdk_make(5) == "box:5x10"
    assert _pdk_make(5, height=20) == "box:5x20"
    assert _pdk_make(5, label="rect", height=7) == "rect:5x7"

    # a fixed param plus **kwargs for extras
    assert _pdk_config("srv") == "srv"
    assert _pdk_config("srv", port=8080, debug=1) == "srv debug=1 port=8080"

    # default values of several literal kinds
    assert _pdk_flags() == "1/True/x/None"
    assert _pdk_flags(2, False, "y", 3) == "2/False/y/3"
    assert _pdk_flags(c="z") == "1/True/z/None"


_fold_p6_defaults_kwargs()


# ===== FOLDED: p6_decorators.py (forwarding wrappers, stacking, factory) =====
# Module scope: decorators on nested `def`s are out of scope, so this source's
# defs stay at module top level (identifiers prefixed `_pdc_`).
_pdc_log: list[str] = []


# a logging wrapper that forwards *args/**kwargs
def _pdc_logged(func: Callable[..., int]) -> Callable[..., int]:
    def wrapper(*args, **kwargs) -> int:
        _pdc_log.append("call")
        return func(*args, **kwargs)
    return wrapper


@_pdc_logged
def _pdc_add(a, b):
    return a + b


# a counting wrapper that keeps state via nonlocal
def _pdc_counted(func: Callable[..., int]) -> Callable[..., int]:
    count = 0

    def wrapper(*args, **kwargs) -> int:
        nonlocal count
        count = count + 1
        _pdc_log.append("n=" + str(count))
        return func(*args, **kwargs)

    return wrapper


@_pdc_counted
def _pdc_square(x):
    return x * x


# stacked decorators apply innermost-first; order visible via the log
def _pdc_deco_a(func: Callable[..., int]) -> Callable[..., int]:
    def wrapper(*args, **kwargs) -> int:
        _pdc_log.append("a")
        return func(*args, **kwargs)
    return wrapper


def _pdc_deco_b(func: Callable[..., int]) -> Callable[..., int]:
    def wrapper(*args, **kwargs) -> int:
        _pdc_log.append("b")
        return func(*args, **kwargs)
    return wrapper


@_pdc_deco_a
@_pdc_deco_b
def _pdc_hello():
    _pdc_log.append("hello")
    return 0


# a decorator factory @repeat(3)
def _pdc_repeat(n: int) -> Callable[[Callable[..., int]], Callable[..., int]]:
    def decorator(func: Callable[..., int]) -> Callable[..., int]:
        def wrapper(*args, **kwargs) -> int:
            result = 0
            for _ in range(n):
                result = func(*args, **kwargs)
            return result
        return wrapper
    return decorator


@_pdc_repeat(3)
def _pdc_announce(msg):
    _pdc_log.append(msg)
    return 1


# decorators over non-int return types (float / str)
def _pdc_trace_f(func: Callable[..., float]) -> Callable[..., float]:
    def wrapper(*args, **kwargs) -> float:
        _pdc_log.append("f")
        return func(*args, **kwargs)
    return wrapper


@_pdc_trace_f
def _pdc_scaled(x) -> float:
    return x * 1.5


def _pdc_trace_s(func: Callable[..., str]) -> Callable[..., str]:
    def wrapper(*args, **kwargs) -> str:
        _pdc_log.append("s")
        return func(*args, **kwargs)
    return wrapper


@_pdc_trace_s
def _pdc_shout(text) -> str:
    return text + "!"


assert _pdc_add(2, 3) == 5
assert _pdc_add(10, 20) == 30
assert _pdc_square(4) == 16
assert _pdc_square(5) == 25
assert _pdc_square(6) == 36
assert _pdc_hello() == 0
assert _pdc_announce("hi") == 1
assert _pdc_scaled(4) == 6.0
assert _pdc_shout("hey") == "hey!"

# side-effect order across the whole script (matches the original prints)
assert _pdc_log == [
    "call", "call",
    "n=1", "n=2", "n=3",
    "a", "b", "hello",
    "hi", "hi", "hi",
    "f", "s",
]


# ===== FOLDED: p10_kwargs_evalorder.py (argument eval order, written-order) =====
# Module scope: keyword arguments bound to regular positional parameters are only
# supported on top-level functions, so these defs stay at module level (`_peo_`).
_peo_log: list[str] = []


def _peo_trace(label: str, val: int) -> int:
    _peo_log.append("eval " + label)
    return val


def _peo_f(a: int, b: int, c: int) -> None:
    _peo_log.append("f " + str(a) + " " + str(b) + " " + str(c))


def _peo_g(a: int, b: int = 10, c: int = 20) -> None:
    _peo_log.append("g " + str(a) + " " + str(b) + " " + str(c))


def _peo_h(a: int, b: int = 1, *args: int, **kw: int) -> None:
    out = ""
    for k in sorted(kw.keys()):
        out += k + "=" + str(kw[k]) + ";"
    _peo_log.append("h " + str(a) + " " + str(b) + " " + str(len(args)) + " " + out)


def _fold_p10_kwargs_evalorder():
    # Keywords written out of parameter order: must evaluate a, c, b.
    _peo_f(_peo_trace("f.a", 1), c=_peo_trace("f.c", 3), b=_peo_trace("f.b", 2))
    # All keywords, fully reversed.
    _peo_f(c=_peo_trace("f2.c", 30), b=_peo_trace("f2.b", 20), a=_peo_trace("f2.a", 10))
    # Defaults skipped in the middle: written order is a then c.
    _peo_g(_peo_trace("g.a", 1), c=_peo_trace("g.c", 99))
    # Keyword for an earlier param evaluated after a later one.
    _peo_g(b=_peo_trace("g2.b", 5), a=_peo_trace("g2.a", 4))
    # **kwargs leftovers interleaved with a fixed-param keyword.
    _peo_h(_peo_trace("h.a", 1), z=_peo_trace("h.z", 26), b=_peo_trace("h.b", 2), y=_peo_trace("h.y", 25))

    assert _peo_log == [
        "eval f.a", "eval f.c", "eval f.b", "f 1 2 3",
        "eval f2.c", "eval f2.b", "eval f2.a", "f 10 20 30",
        "eval g.a", "eval g.c", "g 1 10 99",
        "eval g2.b", "eval g2.a", "g 4 5 20",
        "eval h.a", "eval h.z", "eval h.b", "eval h.y", "h 1 2 0 y=25;z=26;",
    ]


_fold_p10_kwargs_evalorder()


# ===== FOLDED: p10_kwargs_methods.py (method kwargs, virtual dispatch, sort) =====
# Module scope: classes can't be defined inside a function, so the class defs and
# the trace log stay at module top level (identifiers prefixed `_pkm_`).
_pkm_log: list[str] = []


def _pkm_trace(label: str, val: int) -> int:
    _pkm_log.append("eval " + label)
    return val


class _pkm_Greeter:
    def __init__(self, name: str):
        self.name = name

    def greet(self, greeting: str = "Hello", punct: str = "!") -> str:
        return greeting + ", " + self.name + punct

    def add(self, a: int, b: int = 10, c: int = 100) -> int:
        return a + b * 2 + c * 3

    def collect(self, first: int, **extra: int) -> str:
        out = str(first)
        for k in sorted(extra.keys()):
            out += ";" + k + "=" + str(extra[k])
        return out

    @staticmethod
    def shout(word: str, times: int = 2) -> str:
        return word * times

    @classmethod
    def tag(cls, label: str = "g") -> str:
        return "<" + label + ">"


class _pkm_Base:
    def describe(self, prefix: str = "p", n: int = 1) -> str:
        return prefix + ":" + str(n)


class _pkm_Derived(_pkm_Base):
    def describe(self, prefix: str = "p", n: int = 1) -> str:
        return "[" + prefix + ":" + str(n) + "]"


class _pkm_Child(_pkm_Base):
    def describe(self, prefix: str = "p", n: int = 1) -> str:
        inner = super().describe(n=99, prefix=prefix)
        return "{" + inner + "}"


def _fold_p10_kwargs_methods():
    g = _pkm_Greeter("Ada")

    # defaults / keyword permutations
    assert g.greet() == "Hello, Ada!"
    assert g.greet(punct="?") == "Hello, Ada?"
    assert g.greet(greeting="Hi") == "Hi, Ada!"
    assert g.greet("Hey", punct=".") == "Hey, Ada."
    assert g.greet(punct="!!", greeting="Yo") == "Yo, Ada!!"
    assert g.add(1) == 321
    assert g.add(1, c=2) == 27
    assert g.add(1, b=5) == 311
    assert g.add(c=1, a=2, b=3) == 11

    # written-order side effects across reordered keywords
    assert g.add(_pkm_trace("a", 1), c=_pkm_trace("c", 2), b=_pkm_trace("b", 3)) == 13
    assert _pkm_log == ["eval a", "eval c", "eval b"]

    # **kwargs leftovers on a method
    assert g.collect(1, z=26, b=2) == "1;b=2;z=26"
    assert g.collect(first=5, y=25) == "5;y=25"

    # staticmethod / classmethod with keywords
    assert _pkm_Greeter.shout("ha", times=3) == "hahaha"
    assert _pkm_Greeter.shout(word="oi") == "oioi"
    assert g.shout("eh", times=1) == "eh"
    assert _pkm_Greeter.tag(label="x") == "<x>"
    assert _pkm_Greeter.tag() == "<g>"

    # virtual dispatch with keywords (override has its own defaults)
    objs = [_pkm_Base(), _pkm_Derived(), _pkm_Child()]
    described_n5 = [o.describe(n=5) for o in objs]
    assert described_n5 == ["p:5", "[p:5]", "{p:99}"]
    described_kw = [o.describe(prefix="kw") for o in objs]
    assert described_kw == ["kw:1", "[kw:1]", "{kw:99}"]

    # super().m(kw=)
    assert _pkm_Child().describe() == "{p:99}"

    # list.sort matrix
    xs = [3, 1, 2]
    xs.sort()
    assert xs == [1, 2, 3]
    xs.sort(reverse=True)
    assert xs == [3, 2, 1]
    xs.sort(reverse=False)
    assert xs == [1, 2, 3]
    xs.sort(key=lambda v: -v)
    assert xs == [3, 2, 1]
    xs.sort(key=None, reverse=True)
    assert xs == [3, 2, 1]

    ws = ["bbb", "a", "cc"]
    ws.sort(key=len)
    assert ws == ["a", "cc", "bbb"]
    ws.sort(key=len, reverse=True)
    assert ws == ["bbb", "cc", "a"]

    # stability under key sort: equal keys keep order, reverse does not flip them
    pairs = [(2, "a"), (1, "b"), (2, "c"), (1, "d")]
    pairs.sort(key=lambda p: p[0])
    assert pairs == [(1, "b"), (1, "d"), (2, "a"), (2, "c")]
    pairs = [(2, "a"), (1, "b"), (2, "c"), (1, "d")]
    pairs.sort(key=lambda p: p[0], reverse=True)
    assert pairs == [(2, "a"), (2, "c"), (1, "b"), (1, "d")]

    # key closure capture + side-effect order of sort kwargs
    factor = -1
    nums = [5, 1, 4]
    nums.sort(key=lambda v: v * factor, reverse=_pkm_trace("rv", 0) == 1)
    assert nums == [5, 4, 1]
    assert _pkm_log == ["eval a", "eval c", "eval b", "eval rv"]


_fold_p10_kwargs_methods()


# ===== FOLDED: p13_spread.py (*seq spread into non-*args callees) =====
# The decorated callee is hoisted to module scope (decorators on nested `def`s
# are out of scope); the rest of the probes live in the wrapper below.
def _psp_logged(func: Callable[..., int]) -> Callable[..., int]:
    def wrapper(*args: int, **kwargs: int) -> int:
        return func(*args, **kwargs)
    return wrapper


@_psp_logged
def _psp_add2(x: int, y: int) -> int:
    return x + y


def _fold_p13_spread():
    # Fixed-arity, no defaults
    def f3(a: int, b: int, c: int) -> int:
        return a + b + c

    assert f3(*[1, 2, 3]) == 6
    assert f3(*(7, 8, 9)) == 24

    nums_list: list[int] = [10, 20, 30]
    nums_tuple: tuple[int, int, int] = (4, 5, 6)
    assert f3(*nums_list) == 60
    assert f3(*nums_tuple) == 15

    def make3() -> tuple[int, int, int]:
        return (100, 200, 300)

    assert f3(*make3()) == 600

    # Mixed plain + spread
    def f4(a: int, b: int, c: int, d: int) -> int:
        return a * 1000 + b * 100 + c * 10 + d

    assert f4(1, *[2, 3], 4) == 1234
    mid: list[int] = [2, 3]
    assert f4(1, *mid, 4) == 1234

    first: int = 9
    rest_pair: tuple[int, int] = (8, 7)
    assert f3(first, *rest_pair) == 24

    # Multiple spreads
    assert f4(*[1, 2], *[3, 4]) == 1234
    a2: tuple[int, int] = (1, 2)
    b2: tuple[int, int] = (3, 4)

    def f5(a: int, b: int, c: int, d: int, e: int) -> int:
        return a + b + c + d + e

    assert f5(*a2, *b2, 5) == 15

    # Empty spread
    empty: list[int] = []
    assert f3(1, *empty, 2, 3) == 6
    assert f3(*[], 1, 2, 3) == 6

    # str parameters (gradual heap)
    def cat3(a: str, b: str, c: str) -> str:
        return a + b + c

    assert cat3(*["x", "y", "z"]) == "xyz"
    words: list[str] = ["Hello", " ", "World"]
    assert cat3(*words) == "Hello World"

    # float parameters (Raw(F64) laundered through a typed slot)
    def addf(a: float, b: float) -> float:
        return a + b

    floats: list[float] = [1.5, 2.5]
    assert addf(*floats) == 4.0
    assert addf(*[10.25, 0.75]) == 11.0

    # bool parameters (Raw(I8) laundered through a typed slot)
    def andb(a: bool, b: bool) -> bool:
        return a and b

    bools: list[bool] = [True, False]
    assert andb(*bools) == False
    flags: tuple[bool, bool] = (True, True)
    assert andb(*flags) == True

    # Defaults filled from a short spread
    def with_def(a: int, b: int, c: int = 100) -> int:
        return a + b + c

    assert with_def(*[1, 2]) == 103
    assert with_def(*[1, 2, 3]) == 6
    two: list[int] = [1, 2]
    three: list[int] = [1, 2, 3]
    assert with_def(*two) == 103
    assert with_def(*three) == 6

    def multi_def(a: int, b: int = 10, c: int = 20) -> int:
        return a + b + c

    assert multi_def(*[5]) == 35
    assert multi_def(*[5, 15]) == 40
    assert multi_def(*[5, 15, 25]) == 45

    def greet(name: str, greeting: str = "Hello", punct: str = "!") -> str:
        return greeting + " " + name + punct

    only_name: tuple[str] = ("World",)
    assert greet(*only_name) == "Hello World!"
    assert greet(*["Sun", "Hi"]) == "Hi Sun!"

    # *args callee
    def va(a: int, *rest: int) -> int:
        total: int = a
        for x in rest:
            total += x
        return total

    assert va(*[1, 2, 3]) == 6
    assert va(10, *[4, 5]) == 19
    assert va(*[42]) == 42

    def va2(a: int, b: int, *rest: int) -> int:
        total: int = a + b
        for x in rest:
            total += x
        return total

    assert va2(*[10, 20, 30, 40]) == 100
    assert va2(*[5, 15]) == 20
    lead: list[int] = [1, 2]
    assert va2(1, 2, *[3, 4, 5]) == 15

    # Interaction: comprehension as the spread source
    assert f3(*[x * 2 for x in range(3)]) == 6

    # Interaction: spread inside a loop
    def f1(x: int) -> int:
        return x * x

    loop_total: int = 0
    for i in range(4):
        one: list[int] = [i]
        loop_total += f1(*one)
    assert loop_total == 14

    # Interaction: left-to-right evaluation order
    order_log: list[int] = []

    def rec(n: int) -> int:
        order_log.append(n)
        return n

    def src() -> list[int]:
        order_log.append(99)
        return [7, 8]

    def take4(a: int, b: int, c: int, d: int) -> int:
        return a * 1000 + b * 100 + c * 10 + d

    r: int = take4(rec(1), *src(), rec(2))
    assert order_log == [1, 99, 2]
    assert r == 1782

    # Decorated function (its slot is a (*args, **kwargs) wrapper) — module-level _psp_add2
    pair: list[int] = [10, 20]
    assert _psp_add2(*pair) == 30
    assert _psp_add2(*[3, 4]) == 7
    assert _psp_add2(1, *[6]) == 7


_fold_p13_spread()


# ===== FOLDED: p36_mutable_defaults.py (mutable / computed defaults) =====
# Module scope: mutable / computed parameter defaults are only supported on
# top-level functions, so these defs stay at module top level (prefixed `_pmd_`).
def _pmd_append_to_list(x: int, lst: list[int] = []) -> list[int]:
    lst.append(x)
    return lst


def _pmd_collect(k: str, v: int, d: dict[str, int] = {}) -> dict[str, int]:
    d[k] = v
    return d


def _pmd_expr_defaults(*, name: str = "default", count: int = 5 + 5) -> str:
    return name + ":" + str(count)


def _pmd_accumulate(x: int, acc: list[int] = [0]) -> list[int]:
    acc.append(x)
    return acc


def _pmd_with_literals(a: int, b: int = 10, c: str = "z", d: bool = True) -> str:
    return str(a) + str(b) + c + str(d)


def _fold_p36_mutable_defaults():
    # Mutable list default: shared across calls (aliasing trap)
    r1: list[int] = _pmd_append_to_list(1)
    assert len(r1) == 1
    assert r1[0] == 1

    r2: list[int] = _pmd_append_to_list(2)
    assert len(r2) == 2
    assert r2[0] == 1
    assert r2[1] == 2

    r3: list[int] = _pmd_append_to_list(3)
    assert len(r3) == 3
    assert r3[2] == 3

    assert r1 == r2
    assert r2 == r3
    assert r1 is r2
    assert r2 is r3

    fresh: list[int] = [100]
    r4: list[int] = _pmd_append_to_list(4, fresh)
    assert len(r4) == 2
    assert r4[0] == 100
    assert r4[1] == 4
    assert r4 is fresh

    r5: list[int] = _pmd_append_to_list(5)
    assert len(r5) == 4
    assert r5[3] == 5
    assert r5 is r1

    # Mutable dict default: shared across calls
    d1 = _pmd_collect("a", 1)
    d2 = _pmd_collect("b", 2)
    assert len(d2) == 2
    assert d1 is d2
    assert d1["a"] == 1
    assert d1["b"] == 2

    # Computed default: an arithmetic expression, evaluated once
    assert _pmd_expr_defaults() == "default:10"
    assert _pmd_expr_defaults(name="custom") == "custom:10"
    assert _pmd_expr_defaults(count=20) == "default:20"

    # A non-empty list default starts from its initial elements (once)
    a1 = _pmd_accumulate(1)
    assert a1 == [0, 1]
    a2 = _pmd_accumulate(2)
    assert a2 == [0, 1, 2]
    assert a1 is a2

    # Literal defaults are unchanged (regression): per-call fresh, not shared
    assert _pmd_with_literals(1) == "110zTrue"
    assert _pmd_with_literals(1, 2) == "12zTrue"
    assert _pmd_with_literals(1, 2, "q") == "12qTrue"
    assert _pmd_with_literals(1, 2, "q", False) == "12qFalse"


_fold_p36_mutable_defaults()


# ===== FOLDED: p36 computed default over a module global (def-time, module scope) =====
# Kept at module scope because the computed default reads a module global at def time.
_md_BASE: int = 100


def _md_scaled(x: int, factor: int = _md_BASE * 2) -> int:
    return x + factor


assert _md_scaled(1) == 201
assert _md_scaled(1, 0) == 1
assert _md_scaled(5) == 205


# ===== FOLDED: p37_kwargs_spread.py (**dict spread into direct calls) =====
# Module scope: **dict / keyword binding to regular positional parameters is only
# supported on top-level functions, so these defs stay at module level (`_pks_`).
def _pks_accepts_two(a: int, b: int) -> int:
    return a + b


def _pks_with_defaults(a: int, b: int = 10, c: int = 20) -> int:
    return a + b + c


def _pks_three(x: int, y: int, z: int) -> int:
    return x + y + z


def _pks_kwonly(a: int, *, b: int = 50) -> int:
    return a + b


def _pks_make_kwargs() -> dict[str, int]:
    return {"a": 3, "b": 4}


def _fold_p37_kwargs_spread():
    # literal **{...} dicts (compile-time flatten)
    assert _pks_accepts_two(**{"a": 5, "b": 10}) == 15
    assert _pks_accepts_two(a=1, **{"b": 2}) == 3
    assert _pks_accepts_two(**{"a": 10, "b": 20}) == 30
    assert _pks_with_defaults(**{"a": 1}) == 31
    assert _pks_with_defaults(**{"a": 1, "b": 2}) == 23
    assert _pks_with_defaults(**{"a": 1, "c": 3}) == 14
    # literal **{...} combined with a literal *[...] positional spread
    assert _pks_three(*[1, 2], **{"z": 3}) == 6
    assert _pks_three(1, *[2], **{"z": 3}) == 6

    # runtime **d dicts (per-parameter binding)
    d_basic: dict[str, int] = {"a": 5, "b": 10}
    assert _pks_accepts_two(**d_basic) == 15

    d_mixed: dict[str, int] = {"b": 20}
    assert _pks_accepts_two(a=1, **d_mixed) == 21

    d_defaults: dict[str, int] = {"a": 5}
    assert _pks_with_defaults(**d_defaults) == 35

    d_partial_b: dict[str, int] = {"a": 1, "b": 2}
    assert _pks_with_defaults(**d_partial_b) == 23

    d_partial_c: dict[str, int] = {"a": 1, "c": 3}
    assert _pks_with_defaults(**d_partial_c) == 14

    d_kwonly_full: dict[str, int] = {"a": 10, "b": 20}
    assert _pks_kwonly(**d_kwonly_full) == 30

    d_kwonly_part: dict[str, int] = {"a": 25}
    assert _pks_kwonly(**d_kwonly_part) == 75

    # **d from a function result, evaluated once
    assert _pks_accepts_two(**_pks_make_kwargs()) == 7


_fold_p37_kwargs_spread()


# ===== FOLDED: p38_unbox_bool.py (Dyn -> : bool checked unbox) =====
def _fold_p38_unbox_bool():
    def echo_bool(x):
        flag: bool = x  # Dyn -> bool slot: the checked unbox
        assert flag == x
        assert (not flag) == (not x)
        if flag:
            assert x == True
        else:
            assert x == False
        return flag

    assert echo_bool(True) == True
    assert echo_bool(False) == False

    def combine(p, q):
        a: bool = p
        b: bool = q
        assert (a and b) == (p and q)
        assert (a or b) == (p or q)
        assert (a == b) == (p == q)
        return a and b

    assert combine(True, True) == True
    assert combine(True, False) == False
    assert combine(False, False) == False

    # A : bool slot reassigned from another Dyn value within the same function
    def toggle(first, second):
        state: bool = first
        assert state == first
        state = second
        assert state == second
        return state

    assert toggle(True, False) == False
    assert toggle(False, True) == True


_fold_p38_unbox_bool()


# ===== FOLDED: p39_closure_values.py (closure/lambda VALUES with Callable sigs) =====
def _fold_p39_closure_values():
    # 1. A lambda bound to a variable, then called
    add = lambda x, y: x + y
    assert add(2, 3) == 5
    assert add(40, 2) == 42

    is_pos = lambda x: x > 0
    assert is_pos(5) == True
    assert is_pos(-3) == False

    double = lambda x: x * 2
    assert double(21) == 42

    # 2. A lambda capturing an enclosing variable, bound and called
    def lambda_capture() -> int:
        base: int = 100
        inc = lambda d: base + d
        return inc(5)

    assert lambda_capture() == 105

    # 3. A single-level factory returning an annotated nested closure
    def adder_factory(n: int):
        def add_n(x: int) -> int:
            return x + n

        return add_n

    add5 = adder_factory(5)
    assert add5(10) == 15
    assert add5(100) == 105

    def scale_factory(k: int):
        def scale(x: int) -> int:
            return x * k

        return scale

    triple = scale_factory(3)
    assert triple(7) == 21

    # 4. A returned closure stored and invoked more than once
    def counter_base(start: int):
        def step(d: int) -> int:
            return start + d

        return step

    s = counter_base(10)
    assert s(1) == 11
    assert s(2) == 12
    assert s(40) == 50


_fold_p39_closure_values()


# ===== FOLDED: p40_value_call_kwargs.py (value-call into kwonly / **kwargs closures) =====
def _fold_p40_value_call_kwargs():
    # **kwargs: inspect the dict (iterate / len / subscript)
    def make_sum_kwargs():
        def f(**kwargs) -> int:
            total: int = 0
            for k in kwargs:
                total = total + kwargs[k]
            return total
        return f

    g = make_sum_kwargs()
    assert g(a=1, b=2, c=3) == 6
    assert g() == 0
    assert g(x=10) == 10

    def make_count_kwargs():
        def f(**kwargs) -> int:
            return len(kwargs)
        return f

    k = make_count_kwargs()
    assert k() == 0
    assert k(p=1, q=2) == 2

    # **d forward + named/**d merge into a closure
    d_forward = {"x": 10, "y": 20}
    assert g(**d_forward) == 30
    assert g(a=1, **d_forward) == 31

    # keyword-only parameters bound by keyword / by default
    def make_kwonly():
        def f(a: int, *, b: int = 10) -> int:
            return a + b
        return f

    h = make_kwonly()
    assert h(5) == 15
    assert h(5, b=20) == 25

    def make_kwonly_required():
        def f(*, name: str, count: int = 1) -> str:
            return name + ":" + str(count)
        return f

    m = make_kwonly_required()
    assert m(name="hi") == "hi:1"
    assert m(name="x", count=3) == "x:3"

    # *args + **kwargs closure: positional via *args, keyword via **kwargs
    def make_both():
        def f(*args: int, **kwargs: int) -> int:
            total: int = 0
            for v in args:
                total = total + v
            for kk in kwargs:
                total = total + kwargs[kk]
            return total
        return f

    b = make_both()
    assert b(1, 2, 3) == 6
    assert b(a=10, b=20) == 30
    assert b(1, 2, x=3, y=4) == 10

    # a wrapper closure forwarding func(*args, **kwargs) with keywords
    def passthrough(func):
        def wrapper(*args, **kwargs):
            return func(*args, **kwargs)
        return wrapper

    def kw_consumer(**kwargs) -> int:
        return len(kwargs)

    wrapped = passthrough(kw_consumer)
    assert wrapped() == 0
    assert wrapped(a=1, b=2) == 2


_fold_p40_value_call_kwargs()


# ===== FOLDED: p41_call_guard.py (runtime callable guard on value-call path) =====
def _fold_p41_call_guard():
    def call_it(x):  # x is Dyn (unannotated) -> the uniform indirect-call path
        return x()

    # A data tuple as a Dyn callee (the closure/tuple tag-collision SEGV vector).
    raised = False
    try:
        call_it((1, 2))
    except TypeError:
        raised = True
    assert raised, "tuple not callable"

    # An int as a Dyn callee.
    raised = False
    try:
        call_it(5)
    except TypeError:
        raised = True
    assert raised, "int not callable"

    # None as a Dyn callee.
    raised = False
    try:
        call_it(None)
    except TypeError:
        raised = True
    assert raised, "None not callable"

    # A string as a Dyn callee.
    raised = False
    try:
        call_it("hi")
    except TypeError:
        raised = True
    assert raised, "str not callable"

    # A genuine closure flowing through the SAME Dyn call_it path still works.
    def adder(n: int):
        def add() -> int:
            return n + 1
        return add

    assert call_it(adder(41)) == 42


_fold_p41_call_guard()



# ===== FOLDED: test_decorator_factory.py (@decorator(arg), capture counts, *args) =====
# Module scope: all callees are decorated, so they stay at module top level
# (identifiers prefixed `_pdf_`).
def _pdf_multiply(factor: int):
    def decorator(func):
        def wrapper(x: int) -> int:
            return func(x) * factor
        return wrapper
    return decorator


@_pdf_multiply(3)
def _pdf_get_value(x: int) -> int:
    return x + 5


@_pdf_multiply(5)
def _pdf_double_five(x: int) -> int:
    return x * 2


def _pdf_call_get_value(n: int) -> int:
    return _pdf_get_value(n)


def _pdf_add_constant(val: int):
    def decorator(func):
        def wrapper() -> int:
            return func() + val
        return wrapper
    return decorator


@_pdf_add_constant(100)
def _pdf_get_zero() -> int:
    return 0


def _pdf_make_linear(a: int, b: int):
    def decorator(func):
        def wrapper(x: int) -> int:
            return func(x) * a + b
        return wrapper
    return decorator


@_pdf_make_linear(3, 7)
def _pdf_identity_3cap(x: int) -> int:
    return x


def _pdf_make_quadratic(a: int, b: int, c: int):
    def decorator(func):
        def wrapper(x: int) -> int:
            return func(x) * a + b * x + c
        return wrapper
    return decorator


@_pdf_make_quadratic(2, 3, 5)
def _pdf_identity_4cap(x: int) -> int:
    return x


def _pdf_make_poly_5cap(a: int, b: int, c: int, d: int):
    def decorator(func):
        def wrapper(x: int) -> int:
            return func(x) + a + b + c + d
        return wrapper
    return decorator


@_pdf_make_poly_5cap(1, 2, 3, 4)
def _pdf_base_5cap(x: int) -> int:
    return x * 10


def _pdf_make_sum_6cap(v1: int, v2: int, v3: int, v4: int, v5: int):
    def decorator(func):
        def wrapper(x: int) -> int:
            return func(x) + v1 + v2 + v3 + v4 + v5
        return wrapper
    return decorator


@_pdf_make_sum_6cap(1, 2, 3, 4, 5)
def _pdf_base_6cap(x: int) -> int:
    return x


def _pdf_make_sum_7cap(v1: int, v2: int, v3: int, v4: int, v5: int, v6: int):
    def decorator(func):
        def wrapper(x: int) -> int:
            return func(x) + v1 + v2 + v3 + v4 + v5 + v6
        return wrapper
    return decorator


@_pdf_make_sum_7cap(1, 2, 3, 4, 5, 6)
def _pdf_base_7cap(x: int) -> int:
    return x


def _pdf_make_sum_8cap(v1: int, v2: int, v3: int, v4: int, v5: int, v6: int, v7: int):
    def decorator(func):
        def wrapper(x: int) -> int:
            return func(x) + v1 + v2 + v3 + v4 + v5 + v6 + v7
        return wrapper
    return decorator


@_pdf_make_sum_8cap(1, 2, 3, 4, 5, 6, 7)
def _pdf_base_8cap(x: int) -> int:
    return x


def _pdf_simple_decorator(func):
    def wrapper(x: int, y: int) -> int:
        result: int = func(x, y)
        return result
    return wrapper


@_pdf_simple_decorator
def _pdf_add_values(x: int, y: int) -> int:
    return x + y


def _pdf_double_result(func):
    def wrapper(a: int, b: int) -> int:
        r: int = func(a, b)
        return r * 2
    return wrapper


@_pdf_double_result
def _pdf_mul_two(a: int, b: int) -> int:
    return a * b


def _pdf_log_decorator(func):
    def wrapper(x: int) -> int:
        return func(x)
    return wrapper


@_pdf_log_decorator
def _pdf_inc(x: int) -> int:
    return x + 1


def _pdf_varargs_decorator(func):
    def wrapper(*args):
        return func(*args)
    return wrapper


@_pdf_varargs_decorator
def _pdf_add_va(x: int, y: int) -> int:
    return x + y


@_pdf_varargs_decorator
def _pdf_double_va(x: int) -> int:
    return x * 2


@_pdf_varargs_decorator
def _pdf_add3_va(x: int, y: int, z: int) -> int:
    return x + y + z


@_pdf_varargs_decorator
def _pdf_greet_va(name: str) -> str:
    return "Hello, " + name


@_pdf_varargs_decorator
def _pdf_zero_va() -> int:
    return 42


def _pdf_logging_decorator(func):
    def wrapper(*args):
        result = func(*args)
        return result * 2
    return wrapper


@_pdf_logging_decorator
def _pdf_mul_va(x: int, y: int) -> int:
    return x * y


@_pdf_varargs_decorator
def _pdf_sub_va(x: int, y: int) -> int:
    return x - y


def _pdf_my_deco_f(f):
    def wrapper(*args):
        return f(*args)
    return wrapper


@_pdf_my_deco_f
def _pdf_add_nf(x: int, y: int) -> int:
    return x + y


def _pdf_my_deco_g(g):
    def wrapper_g(x: int) -> int:
        return g(x) + 1
    return wrapper_g


@_pdf_my_deco_g
def _pdf_inc_g(x: int) -> int:
    return x


def _pdf_plain_deco(func):
    def plain_wrapper(x: int, y: int) -> int:
        return func(x, y)
    return plain_wrapper


@_pdf_plain_deco
def _pdf_add_plain(x: int, y: int) -> int:
    return x + y


assert _pdf_get_value(10) == 45      # (10+5) * 3
assert _pdf_double_five(4) == 40     # (4 * 2) * 5
assert _pdf_call_get_value(5) == 30  # (5+5) * 3
assert _pdf_get_zero() == 100
assert _pdf_identity_3cap(10) == 37  # 10 * 3 + 7
assert _pdf_identity_4cap(4) == 25   # 4*2 + 3*4 + 5
assert _pdf_base_5cap(5) == 60       # 50 + 1+2+3+4
assert _pdf_base_6cap(100) == 115    # 100 + 1+2+3+4+5
assert _pdf_base_7cap(100) == 121    # 100 + 1+2+3+4+5+6
assert _pdf_base_8cap(100) == 128    # 100 + 1+2+3+4+5+6+7
assert _pdf_add_values(10, 20) == 30
assert _pdf_mul_two(3, 4) == 24
assert _pdf_inc(5) == 6
assert _pdf_add_va(1, 2) == 3
assert _pdf_double_va(5) == 10
assert _pdf_add3_va(1, 2, 3) == 6
assert _pdf_greet_va("World") == "Hello, World"
assert _pdf_zero_va() == 42
assert _pdf_mul_va(3, 4) == 24
assert _pdf_sub_va(10, 3) == 7
assert _pdf_add_nf(3, 4) == 7
assert _pdf_inc_g(10) == 11
_pdf_nums_list: list[int] = [10, 20]
assert _pdf_add_plain(*_pdf_nums_list) == 30


# ===== FOLDED: test_class_decorators.py (class decorators / factory form) =====
# Module scope: classes can't be defined inside a function and the decorators
# apply to class statements (identifiers prefixed `_pcd_`).
_pcd_marks: list = []
_pcd_count: list = [0]


# A plain side-effecting decorator: runs for effect, returns the class.
def _pcd_register(cls: int) -> int:
    _pcd_marks.append(cls)
    _pcd_count[0] = _pcd_count[0] + 1
    return cls


# A parameterized factory decorator @label("name").
def _pcd_label(name: str):
    def deco(cls: int) -> int:
        _pcd_marks.append(cls)
        return cls

    return deco


@_pcd_register
class _pcd_Widget:
    def __init__(self, n: int):
        self.n = n

    def doubled(self) -> int:
        return self.n * 2


@_pcd_label("gadget")
class _pcd_Gadget:
    def __init__(self, m: int):
        self.m = m

    def tripled(self) -> int:
        return self.m * 3


# Stacked: both decorators run (innermost first), the class is unchanged.
@_pcd_register
@_pcd_label("both")
class _pcd_Both:
    def __init__(self, k: int):
        self.k = k


# (a) The side effects ran.
assert _pcd_count[0] == 2
assert len(_pcd_marks) == 4

# (b) The decorated classes still construct and behave normally.
_pcd_w = _pcd_Widget(5)
assert _pcd_w.doubled() == 10
assert _pcd_w.n == 5

_pcd_g = _pcd_Gadget(7)
assert _pcd_g.tripled() == 21

_pcd_b = _pcd_Both(9)
assert _pcd_b.k == 9

# The decorator received the class id (an int) — distinct ids for distinct classes.
assert _pcd_marks[0] != _pcd_marks[1]


# =============================================================================
# Nested classes (capture-free) — a `class` defined inside a function body.
# Names are program-unique (the flat class map keys on the bare name). Assert
# on values/behavior, not raw type()/repr() of nested instances.
# =============================================================================

_NC_GLOBAL = 1000


def test_nested_class():
    # A capture-free nested class: methods use only `self` + module globals.
    class _NcAccu:
        def __init__(self, start: int):
            self.total = start

        def add(self, x: int) -> int:
            self.total += x
            return self.total

        def with_global(self) -> int:
            return self.total + _NC_GLOBAL

    a = _NcAccu(5)
    assert a.add(3) == 8, "nested class method"
    assert a.add(2) == 10, "nested class method 2"
    assert a.with_global() == 1010, "nested class reads module global"

    # isinstance against a nested class + a nested-class annotation.
    class _NcBox:
        def __init__(self, v: int):
            self.v = v

    def unwrap(b: _NcBox) -> int:
        return b.v

    box = _NcBox(42)
    assert isinstance(box, _NcBox), "isinstance nested class"
    assert unwrap(box) == 42, "nested-class annotation param"

    # A nested exception subclass raised + caught.
    class _NcErr(Exception):
        pass

    caught = ""
    try:
        raise _NcErr("nested boom")
    except _NcErr as e:
        caught = str(e)
    assert caught == "nested boom", "nested exception subclass"

    print("test_nested_class passed")


def test_nested_class_generator_method():
    # FIX 1 x FIX 2: a generator *method* inside a nested class.
    class _NcSeq:
        def __init__(self, n: int):
            self.n = n

        def each(self):
            i = 0
            while i < self.n:
                yield i * 2
                i += 1

    s = _NcSeq(4)
    assert list(s.each()) == [0, 2, 4, 6], "nested-class generator method"

    print("test_nested_class_generator_method passed")


def test_method_spread():
    # Priority 1, Feature C: `*args` / `**kwargs` spread in METHOD calls (routes
    # through the dynamic dispatcher rt_obj_method — no static arity needed).
    class _P1Calc:
        def __init__(self, base: int):
            self.base = base

        def add3(self, a: int, b: int, c: int) -> int:
            return self.base + a + b + c

        def greet(self, name: str = "world", punct: str = "!") -> str:
            return "hi " + name + punct

        def total(self, *args) -> int:
            t = self.base
            for v in args:
                t += v
            return t

    c = _P1Calc(100)
    xs = [1, 2, 3]
    # `*args` spread over a list
    assert c.add3(*xs) == 106, "method *args"
    # positional + `*args` + `**kwargs` mixed
    assert c.add3(1, *[2], **{"c": 3}) == 106, "method mixed spread"
    # `**kwargs` spread
    assert c.greet(**{"name": "bob", "punct": "?"}) == "hi bob?", "method **kwargs"
    # `*args` into defaulted params
    assert c.greet(*["alice"]) == "hi alice!", "method *args into defaults"
    # `*args` into a varargs method
    assert c.total(*[1, 2, 3, 4]) == 110, "method *args into *args"
    # spread over non-list iterables (range / tuple)
    assert c.total(*range(1, 5)) == 110, "method *range spread"
    assert c.total(*(10, 20)) == 130, "method *tuple spread"

    print("test_method_spread passed")


# Priority 1, Fix A: `*args`/`**kwargs` spread in static/class-method calls by
# class name (`Cls.smeth(*xs)`). Needs a TOP-LEVEL class (the desugar records
# top-level class shapes in a pre-pass).
class _P1Static:
    @staticmethod
    def add3(a, b, c):
        return a + b + c

    @staticmethod
    def greet(name="world", punct="!"):
        return "hi " + name + punct

    @staticmethod
    def total(*args):
        t = 0
        for v in args:
            t += v
        return t

    @classmethod
    def make(cls, x, y):
        return x * y


def test_static_method_spread():
    xs = [1, 2, 3]
    # staticmethod via class name + `*args`
    assert _P1Static.add3(*xs) == 6, "staticmethod *args"
    # positional + `*args` + `**kwargs`
    assert _P1Static.add3(1, *[2], **{"c": 3}) == 6, "staticmethod mixed spread"
    # `**kwargs` spread
    assert _P1Static.greet(**{"name": "bob", "punct": "?"}) == "hi bob?", "staticmethod **kwargs"
    # `*args` into defaulted params
    assert _P1Static.greet(*["alice"]) == "hi alice!", "staticmethod *args into defaults"
    # `*args` into a varargs staticmethod (list / range / tuple)
    assert _P1Static.total(*[1, 2, 3, 4]) == 10, "staticmethod *args into *args"
    assert _P1Static.total(*range(1, 5)) == 10, "staticmethod *range"
    assert _P1Static.total(*(10, 20)) == 30, "staticmethod *tuple"
    # classmethod via class name + `*args`
    assert _P1Static.make(*[6, 7]) == 42, "classmethod *args"
    # non-spread forms still resolve statically (regression guard)
    assert _P1Static.add3(10, 20, 30) == 60, "staticmethod non-spread"
    assert _P1Static.make(3, 4) == 12, "classmethod non-spread"

    print("test_static_method_spread passed")


# ===== SECTION: Lambda with defaults / *args / **kwargs =====
# A lambda is lowered as a def whose body is one `return`, so it reuses the
# same parameter machinery — defaults, keyword-only, `*args`, `**kwargs` —
# and is called through the single uniform indirect ABI (as a value or
# immediately). A keyword-only param consumed from the keyword mapping must
# NOT reappear in `**kwargs` (the leftover-forwarding fix).

# Module-level capture default goes through the once-eval def desugar.
_fold_n = 5
_fold_default = lambda x, y=_fold_n: x + y


def test_lambda_variadic():
    # literal default, via a value (indirect path) and called immediately
    f = lambda x, y=10: x + y
    assert f(5) == 15, "lambda default omitted"
    assert f(5, 1) == 6, "lambda default supplied"
    assert (lambda x, y=10: x + y)(7) == 17, "immediate lambda default"

    # *args
    g = lambda *a: sum(a)
    assert g(1, 2, 3) == 6, "lambda *args"
    assert g() == 0, "lambda *args empty"

    # **kwargs
    h = lambda **kw: len(kw)
    assert h(a=1, b=2) == 2, "lambda **kwargs"
    assert h() == 0, "lambda **kwargs empty"

    # mixed: fixed + *args + keyword-only default + **kwargs — the consumed
    # keyword-only `sep` must be bound out of the keyword mapping, so `**kw`
    # holds only {q, r} (len 2), NOT {sep, q, r} (len 3).
    k = lambda x, *a, sep="-", **kw: sep.join(
        [str(x)] + [str(v) for v in a] + [str(len(kw))]
    )
    assert k(1, 2, 3, sep="|", q=9, r=8) == "1|2|3|2", "lambda mixed kwonly leftover"
    assert k(1, 2, 3) == "1-2-3-0", "lambda mixed defaults"

    # keyword-only with and without default
    m = lambda x, *, kk=1: x + kk
    assert m(1) == 2, "kwonly default"
    assert m(1, kk=2) == 3, "kwonly supplied"
    req = lambda x, *, k: x * k
    assert req(3, k=4) == 12, "kwonly required by keyword"

    # literal defaults of various non-int types
    n = lambda a, b=True, c=(), d="z": (a, b, c, d)
    assert n(1) == (1, True, (), "z"), "varied literal defaults"
    assert n(1, False, (5,), "q") == (1, False, (5,), "q"), "varied defaults supplied"

    # lambda in expression context (list of lambdas)
    fns = [lambda x, y=1: x + y, lambda x, y=2: x * y]
    assert fns[0](10) == 11, "list-of-lambda 0"
    assert fns[1](10) == 20, "list-of-lambda 1"

    # capture + defaults in a nested context
    def outer():
        b = 100
        return lambda x, y=1: x + y + b

    assert outer()(5) == 106, "nested capture + default omitted"
    assert outer()(5, 2) == 107, "nested capture + default supplied"

    # module-level capture default (def-desugar path)
    assert _fold_default(1) == 6, "module capture default omitted"
    assert _fold_default(1, 20) == 21, "module capture default supplied"

    # lambda as map / sorted key
    assert list(map(lambda v: v * 2, [1, 2, 3])) == [2, 4, 6], "lambda map"
    assert sorted([3, 1, 2], key=lambda v: -v) == [3, 2, 1], "lambda sorted key"

    # nested lambda capturing the enclosing lambda's parameter, with a default
    comp = lambda a: (lambda b=10: a + b)
    assert comp(5)() == 15, "nested-lambda capture default omitted"
    assert comp(5)(2) == 7, "nested-lambda capture default supplied"

    print("test_lambda_variadic passed")


test_nested_class()
test_nested_class_generator_method()
test_method_spread()
test_static_method_spread()
test_lambda_variadic()


print("All function tests passed!")
