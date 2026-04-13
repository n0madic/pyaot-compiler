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

# Test 7: Narrowing on `Any` must dispatch through the target type.
# Before the fix, `narrow_to(Any, Str)` returned `Type::Never` (since
# `Any` doesn't match `Str` in `types_match_for_isinstance`), so
# `len(data)` inside the `isinstance` branch fell through to the
# `arg_type == _` arm of `lower_len` and silently returned 0.
from typing import Any

def _any_len_str(data: Any) -> int:
    if isinstance(data, str):
        return len(data)
    return -1

def _any_len_bytes(data: Any) -> int:
    if isinstance(data, bytes):
        return len(data)
    return -1

def _any_len_dict(data: Any) -> int:
    if isinstance(data, dict):
        return len(data)
    return -1

def test_any_narrowing_to_str():
    assert _any_len_str("hello") == 5
    assert _any_len_str("") == 0
    assert _any_len_str(b"bytes-not-str") == -1
    assert _any_len_str(42) == -1

def test_any_narrowing_to_bytes():
    assert _any_len_bytes(b"hello") == 5
    assert _any_len_bytes("string-not-bytes") == -1

def test_any_narrowing_to_dict():
    assert _any_len_dict({"a": "x", "b": "y"}) == 2
    assert _any_len_dict("string-not-dict") == -1

# Test 8: Narrowing survives a chained isinstance dispatch.
# Mirrors the `_prepare_body` shape in `site-packages/requests/`.
def _classify(x: Any) -> str:
    if isinstance(x, bytes):
        return "bytes:" + str(len(x))
    if isinstance(x, str):
        return "str:" + x
    if isinstance(x, dict):
        return "dict:" + str(len(x))
    return "other"

def test_chained_isinstance_narrowing():
    assert _classify("hello") == "str:hello"
    assert _classify(b"abc") == "bytes:3"
    assert _classify({"k": "v"}) == "dict:1"
    assert _classify(42) == "other"

# Run all tests
test_incompatible_types()
test_redundant_check()
test_valid_union_narrowing()
test_incompatible_float()
test_incompatible_bool()
test_redundant_list_check()
test_valid_isinstance_any()
test_any_narrowing_to_str()
test_any_narrowing_to_bytes()
test_any_narrowing_to_dict()
test_chained_isinstance_narrowing()

print("All dead code detection tests passed!")
