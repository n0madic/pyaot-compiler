# Test multiple exception types in a single except handler

# Test 1: First type matches
def test_multi_except_value_error() -> str:
    try:
        x: int = int("abc")
        return "no error"
    except (ValueError, TypeError) as e:
        return "caught"

assert test_multi_except_value_error() == "caught", "multi except ValueError failed"

# Test 2: Second type in tuple matches
def test_multi_except_second_type() -> str:
    try:
        x: int = 1 // 0
        return "no error"
    except (ValueError, ZeroDivisionError) as e:
        return "caught"

assert test_multi_except_second_type() == "caught", "multi except second type failed"

# Test 3: No match in tuple, falls to next handler
def test_multi_except_no_match() -> str:
    try:
        x: int = 1 // 0
        return "no error"
    except (ValueError, KeyError):
        return "wrong handler"
    except ZeroDivisionError:
        return "correct"

assert test_multi_except_no_match() == "correct", "multi except no match failed"

print("All multi-except tests passed!")
