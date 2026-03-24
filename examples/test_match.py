# Test match statement (pattern matching)

# ===== SECTION: Basic Literal Matching =====

def test_match_int_literal():
    x: int = 1
    result: int = 0
    match x:
        case 1:
            result = 10
        case 2:
            result = 20
        case _:
            result = -1
    assert result == 10

test_match_int_literal()

def test_match_int_literal_second():
    x: int = 2
    result: int = 0
    match x:
        case 1:
            result = 10
        case 2:
            result = 20
        case _:
            result = -1
    assert result == 20

test_match_int_literal_second()

def test_match_int_literal_default():
    x: int = 999
    result: int = 0
    match x:
        case 1:
            result = 10
        case 2:
            result = 20
        case _:
            result = -1
    assert result == -1

test_match_int_literal_default()

def test_match_str_literal():
    x: str = "hello"
    result: int = 0
    match x:
        case "hello":
            result = 1
        case "world":
            result = 2
        case _:
            result = -1
    assert result == 1

test_match_str_literal()

def test_match_str_literal_second():
    x: str = "world"
    result: int = 0
    match x:
        case "hello":
            result = 1
        case "world":
            result = 2
        case _:
            result = -1
    assert result == 2

test_match_str_literal_second()

# ===== SECTION: Singleton Patterns =====

def test_match_true():
    x: bool = True
    result: int = 0
    match x:
        case True:
            result = 1
        case False:
            result = 2
    assert result == 1

test_match_true()

def test_match_false():
    x: bool = False
    result: int = 0
    match x:
        case True:
            result = 1
        case False:
            result = 2
    assert result == 2

test_match_false()

# ===== SECTION: Capture Patterns (as) =====

def test_match_capture():
    x: int = 42
    result: int = 0
    match x:
        case y:
            result = y
    assert result == 42

test_match_capture()

def test_match_capture_with_literal_first():
    x: int = 42
    result: int = 0
    match x:
        case 1:
            result = 1
        case y:
            result = y + 100
    assert result == 142

test_match_capture_with_literal_first()

def test_match_capture_literal_matches():
    x: int = 1
    result: int = 0
    match x:
        case 1:
            result = 1
        case y:
            result = y + 100
    assert result == 1

test_match_capture_literal_matches()

# ===== SECTION: Or Patterns =====

def test_match_or_pattern():
    x: int = 2
    result: int = 0
    match x:
        case 1 | 2 | 3:
            result = 10
        case _:
            result = -1
    assert result == 10

test_match_or_pattern()

def test_match_or_pattern_first():
    x: int = 1
    result: int = 0
    match x:
        case 1 | 2 | 3:
            result = 10
        case _:
            result = -1
    assert result == 10

test_match_or_pattern_first()

def test_match_or_pattern_third():
    x: int = 3
    result: int = 0
    match x:
        case 1 | 2 | 3:
            result = 10
        case _:
            result = -1
    assert result == 10

test_match_or_pattern_third()

def test_match_or_pattern_no_match():
    x: int = 99
    result: int = 0
    match x:
        case 1 | 2 | 3:
            result = 10
        case _:
            result = -1
    assert result == -1

test_match_or_pattern_no_match()

# ===== SECTION: Guard Clauses =====

def test_match_guard_true():
    x: int = 5
    result: int = 0
    match x:
        case n if n > 3:
            result = 1
        case _:
            result = -1
    assert result == 1

test_match_guard_true()

def test_match_guard_false():
    x: int = 2
    result: int = 0
    match x:
        case n if n > 3:
            result = 1
        case _:
            result = -1
    assert result == -1

test_match_guard_false()

def test_match_guard_with_literal():
    x: int = 10
    result: int = 0
    match x:
        case 10 if x > 5:
            result = 1
        case 10:
            result = 2
        case _:
            result = -1
    assert result == 1

test_match_guard_with_literal()

def test_match_guard_literal_guard_false():
    x: int = 3
    result: int = 0
    match x:
        case 3 if x > 5:
            result = 1
        case 3:
            result = 2
        case _:
            result = -1
    assert result == 2

test_match_guard_literal_guard_false()

# ===== SECTION: Sequence Patterns =====

def test_match_list_exact():
    x: list[int] = [1, 2]
    result: int = 0
    match x:
        case [a, b]:
            result = a + b
        case _:
            result = -1
    assert result == 3

test_match_list_exact()

def test_match_list_wrong_length():
    x: list[int] = [1, 2, 3]
    result: int = 0
    match x:
        case [a, b]:
            result = a + b
        case _:
            result = -1
    assert result == -1

test_match_list_wrong_length()

def test_match_list_with_values():
    x: list[int] = [1, 2, 3]
    result: int = 0
    match x:
        case [1, second, 3]:
            result = second
        case _:
            result = -1
    assert result == 2

test_match_list_with_values()

def test_match_list_value_no_match():
    x: list[int] = [1, 2, 4]
    result: int = 0
    match x:
        case [1, second, 3]:
            result = second
        case _:
            result = -1
    assert result == -1

test_match_list_value_no_match()

# ===== SECTION: Starred Patterns =====

def test_match_starred_empty():
    x: list[int] = [1, 2]
    result: int = 0
    match x:
        case [first, *rest]:
            result = first + len(rest)
    assert result == 2  # 1 + len([2]) = 2

test_match_starred_empty()

def test_match_starred_multiple():
    x: list[int] = [1, 2, 3, 4, 5]
    result: int = 0
    match x:
        case [first, *middle, last]:
            result = first + last + len(middle)
    assert result == 9  # 1 + 5 + len([2, 3, 4]) = 9

test_match_starred_multiple()

def test_match_starred_at_start():
    x: list[int] = [1, 2, 3, 4, 5]
    result: int = 0
    match x:
        case [*init, last]:
            result = len(init) + last
    assert result == 9  # len([1, 2, 3, 4]) + 5 = 9

test_match_starred_at_start()

# ===== SECTION: Multiple Cases =====

def test_match_multi_case():
    x: int = 42
    result: str = ""
    match x:
        case 0:
            result = "zero"
        case 1:
            result = "one"
        case 42:
            result = "answer"
        case _:
            result = "other"
    assert result == "answer"

test_match_multi_case()

# ===== SECTION: Nested Expressions =====

def test_match_expression_subject():
    result: int = 0
    match 1 + 1:
        case 2:
            result = 1
        case _:
            result = -1
    assert result == 1

test_match_expression_subject()

def test_match_function_subject():
    def get_val() -> int:
        return 42

    result: int = 0
    match get_val():
        case 42:
            result = 1
        case _:
            result = -1
    assert result == 1

test_match_function_subject()

# ===== SECTION: Complex Guards =====

def test_match_complex_guard():
    x: int = 10
    y: int = 5
    result: int = 0
    match x:
        case n if n > 0 and y < 10:
            result = n + y
        case _:
            result = -1
    assert result == 15

test_match_complex_guard()

# Issue #1: sequence pattern short-circuit (was crashing on out-of-bounds)
def test_match_sequence_short_subject():
    x: list[int] = [1]
    result: int = 0
    match x:
        case [a, b, c]:
            result = 1
        case _:
            result = -1
    assert result == -1

test_match_sequence_short_subject()

def test_match_sequence_empty_subject():
    x: list[int] = []
    result: int = 0
    match x:
        case [a]:
            result = 1
        case _:
            result = -1
    assert result == -1

test_match_sequence_empty_subject()

def test_match_sequence_multiple_fallthrough():
    x: list[int] = [5]
    result: int = 0
    match x:
        case [a, b, c]:
            result = 3
        case [a, b]:
            result = 2
        case [a]:
            result = 1
        case _:
            result = -1
    assert result == 1

test_match_sequence_multiple_fallthrough()

# Issue #3: or-pattern second alternative
def test_match_or_pattern_second_alt():
    x: int = 2
    result: int = 0
    match x:
        case 1 | 2 | 3:
            result = x
        case _:
            result = -1
    assert result == 2

test_match_or_pattern_second_alt()

# Issue #4: mapping pattern key containment (control flow correctness)
def test_match_mapping_key_missing():
    d: dict[str, str] = {"a": "hello"}
    result: str = ""
    match d:
        case {"b": v}:
            result = v
        case {"a": v}:
            result = v
        case _:
            result = "none"
    assert result == "hello"

test_match_mapping_key_missing()

def test_match_mapping_multiple_keys():
    d: dict[str, str] = {"x": "ten", "y": "twenty"}
    result: str = ""
    match d:
        case {"a": v1, "b": v2}:
            result = v1
        case {"x": v1, "y": v2}:
            result = v1
        case _:
            result = "none"
    assert result == "ten"

test_match_mapping_multiple_keys()

# Issue #10: singleton pattern matching
def test_match_singleton_bool():
    x: bool = True
    result: int = 0
    match x:
        case True:
            result = 1
        case False:
            result = 2
    assert result == 1

test_match_singleton_bool()

def test_match_singleton_false():
    x: bool = False
    result: int = 0
    match x:
        case True:
            result = 1
        case False:
            result = 2
    assert result == 2

test_match_singleton_false()

print("All match tests passed!")
