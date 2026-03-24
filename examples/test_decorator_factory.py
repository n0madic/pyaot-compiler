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

print("All decorator factory tests passed!")
