# Test: Type Inference Improvements
# Covers: return type inference, bidirectional propagation, Union types,
# lambda parameter inference, sorted key=, and error reporting.

# =============================================================================
# 1. Return type inference for unannotated functions
# =============================================================================

def double(x: int):
    return x * 2

assert double(21) == 42, "return type inference: int arithmetic"

def greet(name: str):
    return "Hello, " + name

assert greet("World") == "Hello, World", "return type inference: str concat"

def is_positive(x: int):
    return x > 0

assert is_positive(5) == True, "return type inference: comparison true"
assert is_positive(-3) == False, "return type inference: comparison false"

def first(items: list[int]):
    return items[0]

assert first([10, 20, 30]) == 10, "return type inference: list indexing"

def add(a: int, b: int):
    return a + b

assert add(3, 4) == 7, "return type inference: two int params"

# =============================================================================
# 2. Nested function calls with inferred return types
# =============================================================================

def square(x: int):
    return x * x

def sum_squares(a: int, b: int):
    return square(a) + square(b)

assert sum_squares(3, 4) == 25, "nested calls: 3² + 4² = 25"

# =============================================================================
# 3. Bidirectional propagation: empty containers
# =============================================================================

nums: list[int] = []
nums.append(1)
nums.append(2)
assert len(nums) == 2, "bidirectional: empty list with type hint"
assert nums[0] == 1, "bidirectional: list[int] elem access"

d: dict[str, int] = {}
d["a"] = 1
assert d["a"] == 1, "bidirectional: empty dict with type hint"

# =============================================================================
# 4. Bidirectional propagation: empty containers in function args
# =============================================================================

def sum_list(items: list[int]) -> int:
    return sum(items)

result = sum_list([])
assert result == 0, "bidirectional: empty list in function arg"

# =============================================================================
# 5. Bidirectional propagation: empty containers in return
# =============================================================================

def empty_int_list() -> list[int]:
    return []

empty = empty_int_list()
assert len(empty) == 0, "bidirectional: empty list in return"

# =============================================================================
# 6. IfExpr with Union types (both branches boxed correctly)
# =============================================================================

x = 42 if True else "hello"
assert x == 42, "ifexpr union: true branch int"

y = 42 if False else "hello"
assert y == "hello", "ifexpr union: false branch str"

# =============================================================================
# 7. Lambda parameter inference: sorted key=
# =============================================================================

words: list[str] = ["banana", "apple", "fig"]
sorted_words = sorted(words, key=lambda w: len(w))
assert sorted_words[0] == "fig", "sorted key=lambda: shortest first"
assert sorted_words[2] == "banana", "sorted key=lambda: longest last"

# =============================================================================
# 8. Lambda parameter inference: reduce
# =============================================================================

from functools import reduce
nums2: list[int] = [1, 2, 3, 4, 5]
total = reduce(lambda a, b: a + b, nums2)
assert total == 15, "reduce lambda: sum 1..5"

# =============================================================================
# 9. Chained method calls preserve types
# =============================================================================

s = "  Hello World  "
trimmed = s.strip().upper()
assert trimmed == "HELLO WORLD", "chained methods: strip().upper()"

parts = "a,b,c".split(",")
assert len(parts) == 3, "method return: split() → list[str]"
assert parts[0] == "a", "method return: split() first element"

# =============================================================================
# 10. Builtin return types
# =============================================================================

assert len([1, 2, 3]) == 3, "builtin len: int"
assert abs(-5) == 5, "builtin abs: int"
assert int("42") == 42, "builtin int: from str"
assert str(42) == "42", "builtin str: from int"
assert bool(1) == True, "builtin bool: from int"


# =============================================================================
# All passed
# =============================================================================

print("All type inference tests passed!")
