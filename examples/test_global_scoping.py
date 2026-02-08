# Consolidated test file for global variable scoping

# ===== SECTION: Global statement basics =====

# Module-level variable for global tests
counter: int = 0

def test_basic_global() -> None:
    global counter
    counter = 100
    assert counter == 100, "counter should equal 100"

def test_read_global() -> None:
    global counter
    assert counter == 100, "counter should equal 100"

# ===== SECTION: Global variable modification =====

# Test 2: Global modification in nested function
outer_value: int = 0

def test_nested_global_access() -> None:
    global outer_value
    outer_value = 50

    def inner() -> None:
        global outer_value
        assert outer_value == 50, "outer_value should equal 50"
        outer_value = 75

    inner()
    assert outer_value == 75, "outer_value should equal 75"

# ===== SECTION: Global in nested functions =====

# Test 3: Multiple globals in one function
x_global: int = 0
y_global: int = 0

def test_multiple_globals() -> None:
    global x_global
    global y_global
    x_global = 10
    y_global = 20
    assert x_global + y_global == 30, "x_global + y_global should equal 30"

def test_read_multiple_globals() -> None:
    global x_global
    global y_global
    assert x_global == 10, "x_global should equal 10"
    assert y_global == 20, "y_global should equal 20"

# Test 4: Global with computation
result_global: int = 0

def test_global_computation() -> None:
    global result_global
    result_global = 5 * 10 + 3
    assert result_global == 53, "result_global should equal 53"

# ===== SECTION: Global in loops/conditionals =====

# Test 5: Global inside loop
loop_sum: int = 0

def test_global_in_loop() -> None:
    global loop_sum
    loop_sum = 0
    i: int = 0
    while i < 5:
        loop_sum = loop_sum + i
        i = i + 1
    assert loop_sum == 10, "loop_sum should equal 10"  # 0+1+2+3+4

# Test 6: Global inside conditional
cond_value: int = 0

def test_global_in_conditional() -> None:
    global cond_value
    flag: int = 1
    if flag == 1:
        cond_value = 100
    else:
        cond_value = 200
    assert cond_value == 100, "cond_value should equal 100"

# Test 7: Global with function calls
call_count: int = 0

def increment_counter() -> None:
    global call_count
    call_count = call_count + 1

def test_global_with_function_calls() -> None:
    global call_count
    call_count = 0
    increment_counter()
    increment_counter()
    increment_counter()
    assert call_count == 3, "call_count should equal 3"

# ===== SECTION: Global shadowing =====

# Test 8: Local variable shadowing (not global)
shadow_test: int = 100

def test_local_shadowing() -> None:
    # This is a LOCAL variable, not the global one
    shadow_test: int = 999
    assert shadow_test == 999, "shadow_test should equal 999"

def test_global_unchanged_after_shadow() -> None:
    global shadow_test
    # Global should still be 100 since the previous function used local
    assert shadow_test == 100, "shadow_test should equal 100"

# Test 9: Global accessed from deeply nested function
deep_value: int = 0

def test_deeply_nested_global() -> None:
    global deep_value
    deep_value = 1

    def level1() -> None:
        global deep_value
        deep_value = deep_value + 10

        def level2() -> None:
            global deep_value
            deep_value = deep_value + 100

        level2()

    level1()
    assert deep_value == 111, "deep_value should equal 111"  # 1 + 10 + 100

# Test 10: Global reset and reuse
reuse_var: int = 0

def test_global_reset() -> None:
    global reuse_var
    reuse_var = 42
    assert reuse_var == 42, "reuse_var should equal 42"
    reuse_var = 0
    assert reuse_var == 0, "reuse_var should equal 0"
    reuse_var = 123
    assert reuse_var == 123, "reuse_var should equal 123"

# ===== SECTION: Global heap types (str, list, dict) =====

global_str: str = ""

def test_global_string_basic() -> None:
    global global_str
    global_str = "hello"
    assert global_str == "hello", "global_str should equal \"hello\""

def test_global_string_reassign() -> None:
    global global_str
    global_str = "world"
    assert global_str == "world", "global_str should equal \"world\""

def test_global_string_concat() -> None:
    global global_str
    global_str = "hello" + " " + "world"
    assert global_str == "hello world", "global_str should equal \"hello world\""

global_list: list[int] = []

def test_global_list_basic() -> None:
    global global_list
    global_list = [1, 2, 3]
    assert len(global_list) == 3, "len(global_list) should equal 3"
    assert global_list[0] == 1, "global_list[0] should equal 1"
    assert global_list[1] == 2, "global_list[1] should equal 2"
    assert global_list[2] == 3, "global_list[2] should equal 3"

def test_global_list_append() -> None:
    global global_list
    global_list = []
    global_list.append(10)
    global_list.append(20)
    global_list.append(30)
    assert len(global_list) == 3, "len(global_list) should equal 3"
    assert global_list[0] == 10, "global_list[0] should equal 10"
    assert global_list[1] == 20, "global_list[1] should equal 20"
    assert global_list[2] == 30, "global_list[2] should equal 30"

def test_global_list_modify() -> None:
    global global_list
    global_list = [1, 2, 3]
    global_list[1] = 100
    assert global_list[1] == 100, "global_list[1] should equal 100"

global_dict: dict[str, int] = {}

def test_global_dict_basic() -> None:
    global global_dict
    global_dict = {"a": 1, "b": 2}
    assert global_dict["a"] == 1, "global_dict[\"a\"] should equal 1"
    assert global_dict["b"] == 2, "global_dict[\"b\"] should equal 2"

def test_global_dict_modify() -> None:
    global global_dict
    global_dict = {}
    global_dict["x"] = 100
    global_dict["y"] = 200
    assert global_dict["x"] == 100, "global_dict[\"x\"] should equal 100"
    assert global_dict["y"] == 200, "global_dict[\"y\"] should equal 200"

# ===== SECTION: Global floats and bools =====

global_float: float = 0.0

def test_global_float_basic() -> None:
    global global_float
    global_float = 3.14
    assert global_float == 3.14, "global_float should equal 3.14"

def test_global_float_arithmetic() -> None:
    global global_float
    global_float = 1.5 + 2.5
    assert global_float == 4.0, "global_float should equal 4.0"

global_bool: bool = False

def test_global_bool_basic() -> None:
    global global_bool
    global_bool = True
    assert global_bool == True, "global_bool should equal True"

def test_global_bool_toggle() -> None:
    global global_bool
    global_bool = False
    assert global_bool == False, "global_bool should equal False"
    global_bool = True
    assert global_bool == True, "global_bool should equal True"

# ===== SECTION: Global propagation to deeply nested functions =====

prop_counter: int = 0

def test_basic_propagation() -> None:
    global prop_counter
    prop_counter = 0

    def inner() -> None:
        global prop_counter  # CPython requires explicit global declaration
        prop_counter = prop_counter + 1

    inner()
    inner()
    inner()
    assert prop_counter == 3, "prop_counter should equal 3"

prop_deep_value: int = 0

def test_deeply_nested_propagation() -> None:
    global prop_deep_value
    prop_deep_value = 10

    def level1() -> None:
        global prop_deep_value  # CPython requires explicit global declaration
        prop_deep_value = prop_deep_value + 100

        def level2() -> None:
            global prop_deep_value  # CPython requires explicit global declaration
            prop_deep_value = prop_deep_value + 1000

        level2()

    level1()
    assert prop_deep_value == 1110, "prop_deep_value should equal 1110"

mixed_global: int = 0

def test_mixed_global_local() -> None:
    global mixed_global
    mixed_global = 5

    def inner() -> None:
        global mixed_global  # CPython requires explicit global declaration
        local_var: int = 10
        mixed_global = mixed_global + local_var

    inner()
    assert mixed_global == 15, "mixed_global should equal 15"

multi_a: int = 0
multi_b: int = 0

def test_multiple_globals_propagation() -> None:
    global multi_a
    global multi_b
    multi_a = 1
    multi_b = 2

    def inner() -> None:
        global multi_a  # CPython requires explicit global declaration
        global multi_b
        multi_a = multi_a * 10
        multi_b = multi_b * 10

    inner()
    assert multi_a == 10, "multi_a should equal 10"
    assert multi_b == 20, "multi_b should equal 20"

# Run all tests
test_basic_global()
test_read_global()
test_nested_global_access()
test_multiple_globals()
test_read_multiple_globals()
test_global_computation()
test_global_in_loop()
test_global_in_conditional()
test_global_with_function_calls()
test_local_shadowing()
test_global_unchanged_after_shadow()
test_deeply_nested_global()
test_global_reset()

test_global_string_basic()
test_global_string_reassign()
test_global_string_concat()

test_global_list_basic()
test_global_list_append()
test_global_list_modify()

test_global_dict_basic()
test_global_dict_modify()

test_global_float_basic()
test_global_float_arithmetic()

test_global_bool_basic()
test_global_bool_toggle()

test_basic_propagation()
test_deeply_nested_propagation()
test_mixed_global_local()
test_multiple_globals_propagation()

print("All global scoping tests passed!")
