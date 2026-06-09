# Test GC infrastructure
# This test verifies GC prologue/epilogue generation for heap types

# Functions with str parameters generate GC code
def use_string(s: str) -> int:
    # s is a str parameter - marked as GC root
    # GC frame: gc_push, roots[0] = s, ..., gc_pop
    return 42


def nested_strings(a: str, b: str) -> int:
    # Multiple GC roots: roots[0] = a, roots[1] = b
    return 100


def mixed_params(x: int, s: str, y: int) -> int:
    # Only s is GC root, x and y are primitives
    return x + y


# Functions with only primitive params have NO GC overhead
def pure_int(x: int, y: int) -> int:
    # No GC code generated - just 'add' + 'ret'
    return x + y


def factorial(n: int) -> int:
    if n <= 1:
        return 1
    return n * factorial(n - 1)


# Test -> None return type
def void_function() -> None:
    x: int = 10
    assert x == 10, "x should equal 10"


def void_with_str_param(s: str) -> None:
    # GC root for s, but returns None
    assert True, "True should be True"


# Test all functions
r1: int = use_string("hello")
assert r1 == 42, "r1 should equal 42"

r2: int = nested_strings("foo", "bar")
assert r2 == 100, "r2 should equal 100"

r3: int = mixed_params(10, "test", 20)
assert r3 == 30, "r3 should equal 30"

r4: int = pure_int(5, 7)
assert r4 == 12, "r4 should equal 12"

r5: int = factorial(5)
assert r5 == 120, "r5 should equal 120"

void_function()
void_with_str_param("test")

print("All GC infrastructure tests passed!")
