# Test decorator factories: @decorator(arg) pattern

# Test 1: Basic decorator factory
def multiply(factor: int):
    def decorator(func):
        def wrapper(x: int) -> int:
            return func(x) * factor
        return wrapper
    return decorator

@multiply(3)
def get_value(x: int) -> int:
    return x + 5

result = get_value(10)
assert result == 45, f"expected 45, got {result}"  # (10+5) * 3 = 45
print("Decorator factory: PASS")

# Test 2: Different factory argument values
@multiply(5)
def double_five(x: int) -> int:
    return x * 2

result2 = double_five(4)
assert result2 == 40, f"expected 40, got {result2}"  # (4 * 2) * 5 = 40
print("Different factory values: PASS")

# Test 3: Calling decorated function from another function
def call_get_value(n: int) -> int:
    return get_value(n)

result3 = call_get_value(5)
assert result3 == 30, f"expected 30, got {result3}"  # (5+5) * 3 = 30
print("Indirect call: PASS")

# Test 4: Decorator factory with no-arg wrapped function
def add_constant(val: int):
    def decorator(func):
        def wrapper() -> int:
            return func() + val
        return wrapper
    return decorator

@add_constant(100)
def get_zero() -> int:
    return 0

result4 = get_zero()
assert result4 == 100, f"expected 100, got {result4}"
print("No-arg wrapped function: PASS")

# Test 5: Wrapper with 3 captures (func + 2 factory args)
def make_linear(a: int, b: int):
    def decorator(func):
        def wrapper(x: int) -> int:
            # wrapper captures: func, a, b = 3 captures total
            return func(x) * a + b
        return wrapper
    return decorator

@make_linear(3, 7)
def identity_3cap(x: int) -> int:
    return x

result5 = identity_3cap(10)
assert result5 == 37, f"expected 37, got {result5}"  # 10 * 3 + 7 = 37
print("3 captures (func + 2 args): PASS")

# Test 6: Wrapper with 4 captures (func + 3 factory args)
def make_quadratic(a: int, b: int, c: int):
    def decorator(func):
        def wrapper(x: int) -> int:
            # wrapper captures: func, a, b, c = 4 captures total
            return func(x) * a + b * x + c
        return wrapper
    return decorator

@make_quadratic(2, 3, 5)
def identity_4cap(x: int) -> int:
    return x

result6 = identity_4cap(4)
assert result6 == 25, f"expected 25, got {result6}"  # 4 * 2 + 3 * 4 + 5 = 8 + 12 + 5 = 25
print("4 captures (func + 3 args): PASS")

# Test 7: Wrapper with 5 captures (func + 4 factory args)
def make_poly_5cap(a: int, b: int, c: int, d: int):
    def decorator(func):
        def wrapper(x: int) -> int:
            # wrapper captures: func, a, b, c, d = 5 captures total
            return func(x) + a + b + c + d
        return wrapper
    return decorator

@make_poly_5cap(1, 2, 3, 4)
def base_5cap(x: int) -> int:
    return x * 10

result7 = base_5cap(5)
assert result7 == 60, f"expected 60, got {result7}"  # 5 * 10 + 1 + 2 + 3 + 4 = 60
print("5 captures (func + 4 args): PASS")

# Test 8: Wrapper with 6 captures (func + 5 factory args)
def make_sum_6cap(v1: int, v2: int, v3: int, v4: int, v5: int):
    def decorator(func):
        def wrapper(x: int) -> int:
            # wrapper captures: func, v1..v5 = 6 captures total
            return func(x) + v1 + v2 + v3 + v4 + v5
        return wrapper
    return decorator

@make_sum_6cap(1, 2, 3, 4, 5)
def base_6cap(x: int) -> int:
    return x

result8 = base_6cap(100)
assert result8 == 115, f"expected 115, got {result8}"  # 100 + 1+2+3+4+5 = 115
print("6 captures (func + 5 args): PASS")

# Test 9: Wrapper with 7 captures (func + 6 factory args)
def make_sum_7cap(v1: int, v2: int, v3: int, v4: int, v5: int, v6: int):
    def decorator(func):
        def wrapper(x: int) -> int:
            # wrapper captures: func, v1..v6 = 7 captures total
            return func(x) + v1 + v2 + v3 + v4 + v5 + v6
        return wrapper
    return decorator

@make_sum_7cap(1, 2, 3, 4, 5, 6)
def base_7cap(x: int) -> int:
    return x

result9 = base_7cap(100)
assert result9 == 121, f"expected 121, got {result9}"  # 100 + 1+2+3+4+5+6 = 121
print("7 captures (func + 6 args): PASS")

# Test 10: Wrapper with 8 captures (func + 7 factory args) - MAXIMUM SUPPORTED
def make_sum_8cap(v1: int, v2: int, v3: int, v4: int, v5: int, v6: int, v7: int):
    def decorator(func):
        def wrapper(x: int) -> int:
            # wrapper captures: func, v1..v7 = 8 captures total (maximum)
            return func(x) + v1 + v2 + v3 + v4 + v5 + v6 + v7
        return wrapper
    return decorator

@make_sum_8cap(1, 2, 3, 4, 5, 6, 7)
def base_8cap(x: int) -> int:
    return x

result10 = base_8cap(100)
assert result10 == 128, f"expected 128, got {result10}"  # 100 + 1+2+3+4+5+6+7 = 128
print("8 captures (func + 7 args): PASS")

# ===== Regression: indirect call through function parameter in wrapper =====
# This tests the case where a wrapper function calls `func(x, y)` where `func`
# is a parameter holding a function pointer. Previously this silently returned None.

def simple_decorator(func):
    def wrapper(x: int, y: int) -> int:
        result: int = func(x, y)
        return result
    return wrapper

@simple_decorator
def add_values(x: int, y: int) -> int:
    return x + y

result_indirect = add_values(10, 20)
assert result_indirect == 30, f"indirect call in wrapper: expected 30, got {result_indirect}"
print("Indirect call through wrapper parameter: PASS")

# Decorator wrapper that transforms the result
def double_result(func):
    def wrapper(a: int, b: int) -> int:
        r: int = func(a, b)
        return r * 2
    return wrapper

@double_result
def mul_two(a: int, b: int) -> int:
    return a * b

result_double = mul_two(3, 4)
assert result_double == 24, f"double_result decorator: expected 24, got {result_double}"
print("Decorator wrapper with result transformation: PASS")

# Chained simple decorators (identity-like)
def log_decorator(func):
    def wrapper(x: int) -> int:
        return func(x)
    return wrapper

@log_decorator
def inc(x: int) -> int:
    return x + 1

assert inc(5) == 6, f"chained identity wrapper: expected 6, got {inc(5)}"
print("Identity wrapper decorator: PASS")

# ===== Decorator with *args wrapper =====
# Tests for decorators that use *args to forward arguments to the wrapped function

def varargs_decorator(func):
    def wrapper(*args):
        return func(*args)
    return wrapper

# *args with two int args
@varargs_decorator
def add_va(x: int, y: int) -> int:
    return x + y

result_va1 = add_va(1, 2)
assert result_va1 == 3, f"*args 2 ints: expected 3, got {result_va1}"
print("*args wrapper (2 ints): PASS")

# *args with single int arg
@varargs_decorator
def double_va(x: int) -> int:
    return x * 2

result_va2 = double_va(5)
assert result_va2 == 10, f"*args 1 int: expected 10, got {result_va2}"
print("*args wrapper (1 int): PASS")

# *args with three int args
@varargs_decorator
def add3_va(x: int, y: int, z: int) -> int:
    return x + y + z

result_va3 = add3_va(1, 2, 3)
assert result_va3 == 6, f"*args 3 ints: expected 6, got {result_va3}"
print("*args wrapper (3 ints): PASS")

# *args with string arg
@varargs_decorator
def greet_va(name: str) -> str:
    return "Hello, " + name

result_va4 = greet_va("World")
assert result_va4 == "Hello, World", f"*args str: expected 'Hello, World', got '{result_va4}'"
print("*args wrapper (str): PASS")

# *args with no args
@varargs_decorator
def zero_va() -> int:
    return 42

result_va5 = zero_va()
assert result_va5 == 42, f"*args 0 args: expected 42, got {result_va5}"
print("*args wrapper (0 args): PASS")

# *args wrapper with side effects
def logging_decorator(func):
    def wrapper(*args):
        result = func(*args)
        return result * 2
    return wrapper

@logging_decorator
def mul_va(x: int, y: int) -> int:
    return x * y

result_va6 = mul_va(3, 4)
assert result_va6 == 24, f"*args with modification: expected 24, got {result_va6}"
print("*args wrapper with result modification: PASS")

# Multiple functions decorated with the same *args decorator
@varargs_decorator
def sub_va(x: int, y: int) -> int:
    return x - y

result_va7 = sub_va(10, 3)
assert result_va7 == 7, f"*args reuse: expected 7, got {result_va7}"
print("*args wrapper reuse: PASS")

# ===== Decorator with non-"func" parameter name (V2-P11a) =====
# Tests that func-ptr detection works even when the decorator names its
# parameter something other than "func" (e.g. "f").

def my_deco_f(f):
    def wrapper(*args):
        return f(*args)
    return wrapper

@my_deco_f
def add_nf(x: int, y: int) -> int:
    return x + y

result_nf = add_nf(3, 4)
assert result_nf == 7, f"non-func param decorator: expected 7, got {result_nf}"
print("Non-func-named decorator param (*args): PASS")

def my_deco_g(g):
    def wrapper_g(x: int) -> int:
        return g(x) + 1
    return wrapper_g

@my_deco_g
def inc_g(x: int) -> int:
    return x

result_g = inc_g(10)
assert result_g == 11, f"non-func param (simple call): expected 11, got {result_g}"
print("Non-func-named decorator param (simple call): PASS")

# ===== List unpacking in decorated function call (V2-P11b) =====
# Tests that *list works when calling a non-varargs decorated function.

def plain_deco(func):
    def plain_wrapper(x: int, y: int) -> int:
        return func(x, y)
    return plain_wrapper

@plain_deco
def add_plain(x: int, y: int) -> int:
    return x + y

nums_list: list[int] = [10, 20]
result_list = add_plain(*nums_list)
assert result_list == 30, f"*list in decorated call: expected 30, got {result_list}"
print("List unpacking in decorated call: PASS")

print("All decorator factory tests passed!")
