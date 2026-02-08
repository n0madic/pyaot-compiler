# Dead code detection test cases
# This file tests the compiler's ability to detect unreachable isinstance checks

# Test 1: Always False isinstance (then-branch dead)
# isinstance(x: int, str) is always False since int is never str
def test_incompatible_types():
    x: int = 42
    if isinstance(x, str):  # WARNING: isinstance check is always False
        # This branch is unreachable
        assert False, "unreachable"
    assert True

# Test 2: Always True isinstance (else-branch dead)
# isinstance(x: str, str) is always True since x is already str
def test_redundant_check():
    x: str = "hello"
    if isinstance(x, str):  # WARNING: isinstance check is always True
        assert True
    else:
        # This branch is unreachable
        assert False, "unreachable"

# Test 3: Valid Union narrowing (no warning)
# isinstance on Union types is useful and should not warn
def test_valid_union_narrowing():
    x: int | str = 42
    if isinstance(x, int):  # No warning - valid narrowing
        assert x == 42
    else:
        assert False, "should be int"

# Test 4: Multiple incompatible checks in different functions
def test_incompatible_float():
    y: float = 3.14
    if isinstance(y, str):  # WARNING: isinstance check is always False
        assert False, "unreachable"
    assert True

def test_incompatible_bool():
    z: bool = True
    if isinstance(z, str):  # WARNING: isinstance check is always False
        assert False, "unreachable"
    assert True

# Test 5: Redundant check with list type
def test_redundant_list_check():
    items: list[int] = [1, 2, 3]
    if isinstance(items, list):  # WARNING: isinstance check is always True
        assert len(items) == 3
    else:
        assert False, "unreachable"

# Test 6: Valid isinstance usage (no warning expected)
def test_valid_isinstance_any():
    # For Union types, isinstance is valid and useful
    value: int | str | float = "test"
    if isinstance(value, str):  # No warning - valid narrowing
        assert value == "test"
    elif isinstance(value, int):
        pass
    else:
        pass

# Run all tests
test_incompatible_types()
test_redundant_check()
test_valid_union_narrowing()
test_incompatible_float()
test_incompatible_bool()
test_redundant_list_check()
test_valid_isinstance_any()

print("All dead code detection tests passed!")
