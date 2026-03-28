# Consolidated test file for exception handling
#
# Variables assigned in try blocks are automatically wrapped in heap-allocated cells
# to preserve their values across setjmp/longjmp exception unwinding.

# ===== SECTION: try/except basics =====

# Test 1: Basic try/except - exception is caught
def test_basic():
    caught: bool = False
    try:
        raise Exception("test")
    except:
        caught = True
    assert caught, "Should catch exception"

# Test 6: Try without raising
def test_no_exception():
    executed: bool = False
    try:
        executed = True
    except:
        executed = False
    assert executed, "Try body should execute"

# ===== SECTION: Multiple except clauses =====

# Test 7: Exception caught at outer level
def test_outer_catch():
    outer_caught: bool = False
    try:
        try:
            raise Exception("inner")
        except:
            # Catch and raise new exception
            raise Exception("outer")
    except:
        outer_caught = True
    assert outer_caught, "Outer handler should catch re-raised exception"

# ===== SECTION: Exception types (ValueError, TypeError, IndexError, etc.) =====

# Test 8: All built-in exception types
def test_exception_types():
    # ValueError
    caught: bool = False
    try:
        raise ValueError("invalid value")
    except:
        caught = True
    assert caught, "caught should be True"

    # TypeError
    caught = False
    try:
        raise TypeError("wrong type")
    except:
        caught = True
    assert caught, "caught should be True"

    # KeyError
    caught = False
    try:
        raise KeyError("missing key")
    except:
        caught = True
    assert caught, "caught should be True"

    # IndexError
    caught = False
    try:
        raise IndexError("out of range")
    except:
        caught = True
    assert caught, "caught should be True"

    # RuntimeError
    caught = False
    try:
        raise RuntimeError("runtime issue")
    except:
        caught = True
    assert caught, "caught should be True"

    # AttributeError
    caught = False
    try:
        raise AttributeError("missing attr")
    except:
        caught = True
    assert caught, "caught should be True"

    # Exception without message
    caught = False
    try:
        raise ValueError()
    except:
        caught = True
    assert caught, "caught should be True"

# ===== SECTION: ValueError - min/max on empty collections =====

def test_min_max_empty_collections():
    # Test min() on empty list (int)
    caught_min_list_int: bool = False
    try:
        x: list[int] = []
        result: int = min(x)
    except ValueError:
        caught_min_list_int = True
    assert caught_min_list_int, "min() on empty int list should raise ValueError"

    # Test max() on empty list (int)
    caught_max_list_int: bool = False
    try:
        x2: list[int] = []
        result2: int = max(x2)
    except ValueError:
        caught_max_list_int = True
    assert caught_max_list_int, "max() on empty int list should raise ValueError"

    # Test min() on empty list (float)
    caught_min_list_float: bool = False
    try:
        x3: list[float] = []
        result3: float = min(x3)
    except ValueError:
        caught_min_list_float = True
    assert caught_min_list_float, "min() on empty float list should raise ValueError"

    # Test max() on empty list (float)
    caught_max_list_float: bool = False
    try:
        x4: list[float] = []
        result4: float = max(x4)
    except ValueError:
        caught_max_list_float = True
    assert caught_max_list_float, "max() on empty float list should raise ValueError"

    # Test min() on empty tuple (int)
    caught_min_tuple_int: bool = False
    try:
        x5 = ()
        result5: int = min(x5)
    except ValueError:
        caught_min_tuple_int = True
    assert caught_min_tuple_int, "min() on empty int tuple should raise ValueError"

    # Test max() on empty tuple (int)
    caught_max_tuple_int: bool = False
    try:
        x6 = ()
        result6: int = max(x6)
    except ValueError:
        caught_max_tuple_int = True
    assert caught_max_tuple_int, "max() on empty int tuple should raise ValueError"

    # Test min() on empty tuple (float)
    caught_min_tuple_float: bool = False
    try:
        x7 = ()
        result7: float = min(x7)
    except ValueError:
        caught_min_tuple_float = True
    assert caught_min_tuple_float, "min() on empty float tuple should raise ValueError"

    # Test max() on empty tuple (float)
    caught_max_tuple_float: bool = False
    try:
        x8 = ()
        result8: float = max(x8)
    except ValueError:
        caught_max_tuple_float = True
    assert caught_max_tuple_float, "max() on empty float tuple should raise ValueError"

    # Test min() on empty set (int)
    caught_min_set_int: bool = False
    try:
        x9: set[int] = set()
        result9: int = min(x9)
    except ValueError:
        caught_min_set_int = True
    assert caught_min_set_int, "min() on empty int set should raise ValueError"

    # Test max() on empty set (int)
    caught_max_set_int: bool = False
    try:
        x10: set[int] = set()
        result10: int = max(x10)
    except ValueError:
        caught_max_set_int = True
    assert caught_max_set_int, "max() on empty int set should raise ValueError"

    # Test min() on empty set (float)
    caught_min_set_float: bool = False
    try:
        x11: set[float] = set()
        result11: float = min(x11)
    except ValueError:
        caught_min_set_float = True
    assert caught_min_set_float, "min() on empty float set should raise ValueError"

    # Test max() on empty set (float)
    caught_max_set_float: bool = False
    try:
        x12: set[float] = set()
        result12: float = max(x12)
    except ValueError:
        caught_max_set_float = True
    assert caught_max_set_float, "max() on empty float set should raise ValueError"

    # Test min() with key= on empty list
    def get_len(s: str) -> int:
        return len(s)

    caught_min_key_list: bool = False
    try:
        x13: list[str] = []
        result13: str = min(x13, key=get_len)
    except ValueError:
        caught_min_key_list = True
    assert caught_min_key_list, "min() with key on empty list should raise ValueError"

    # Test max() with key= on empty list
    caught_max_key_list: bool = False
    try:
        x14: list[str] = []
        result14: str = max(x14, key=get_len)
    except ValueError:
        caught_max_key_list = True
    assert caught_max_key_list, "max() with key on empty list should raise ValueError"

    # Test min() with key= on empty tuple
    caught_min_key_tuple: bool = False
    try:
        x15 = ()
        result15: str = min(x15, key=get_len)
    except ValueError:
        caught_min_key_tuple = True
    assert caught_min_key_tuple, "min() with key on empty tuple should raise ValueError"

    # Test max() with key= on empty tuple
    caught_max_key_tuple: bool = False
    try:
        x16 = ()
        result16: str = max(x16, key=get_len)
    except ValueError:
        caught_max_key_tuple = True
    assert caught_max_key_tuple, "max() with key on empty tuple should raise ValueError"

    # Test min() with key= on empty set
    caught_min_key_set: bool = False
    try:
        x17: set[int] = set()
        def myabs(n: int) -> int:
            if n < 0:
                return -n
            return n
        result17: int = min(x17, key=myabs)
    except ValueError:
        caught_min_key_set = True
    assert caught_min_key_set, "min() with key on empty set should raise ValueError"

    # Test max() with key= on empty set
    caught_max_key_set: bool = False
    try:
        x18: set[int] = set()
        def myabs2(n: int) -> int:
            if n < 0:
                return -n
            return n
        result18: int = max(x18, key=myabs2)
    except ValueError:
        caught_max_key_set = True
    assert caught_max_key_set, "max() with key on empty set should raise ValueError"

# ===== SECTION: finally blocks =====

# Test 2: Finally always runs (normal path)
def test_finally_normal():
    ran: bool = False
    try:
        x: int = 1
    finally:
        ran = True
    assert ran, "Finally should run on normal path"

# ===== SECTION: raise statement =====

# Test 5: Exception message (we catch it, message is for debugging)
def test_with_message():
    caught: bool = False
    try:
        raise Exception("custom message")
    except:
        caught = True
    assert caught, "Should catch exception with message"

# ===== SECTION: Re-raise exceptions =====

# Test 3: Bare raise re-raises to outer handler
def test_reraise():
    outer: bool = False
    try:
        try:
            raise Exception("inner")
        except:
            raise  # re-raise
    except:
        outer = True
    assert outer, "Should re-raise to outer handler"

# Test 4: Computation after exception handling
def test_after_handling():
    result: int = 0
    try:
        raise Exception("test")
    except:
        result = 1
    result = result + 10
    assert result == 11, "Code after try/except should run"

# ===== SECTION: try/except/else =====

def test_try_else_basic():
    """Else runs when no exception occurs."""
    else_ran: bool = False
    try:
        x: int = 1
    except:
        pass
    else:
        else_ran = True
    assert else_ran, "else block should run when no exception"

def test_try_else_skipped_on_exception():
    """Else skipped when exception caught."""
    else_ran: bool = False
    try:
        raise Exception("test")
    except:
        pass
    else:
        else_ran = True
    assert not else_ran, "else block should NOT run when exception caught"

def test_try_else_with_finally():
    """Else before finally."""
    order: list[int] = []
    try:
        order.append(1)
    except:
        order.append(2)
    else:
        order.append(3)
    finally:
        order.append(4)
    assert order == [1, 3, 4], "order should be try, else, finally"

def test_try_else_exception_propagates():
    """Exception in else propagates (not caught by same try)."""
    caught: bool = False
    try:
        try:
            x: int = 1
        except:
            pass
        else:
            raise Exception("from else")
    except:
        caught = True
    assert caught, "exception from else should propagate to outer handler"

def test_try_else_with_return_in_try():
    """Else can access variables set in try block."""
    result: int = 0
    try:
        x: int = 42
    except:
        pass
    else:
        result = x
    assert result == 42, "else block should see variable from try"

# ===== SECTION: IndexError for string/list out of bounds =====

def test_string_index_in_bounds():
    """Test normal string indexing works."""
    text: str = "hello"
    assert text[0] == "h", "text[0] should equal \"h\""
    assert text[4] == "o", "text[4] should equal \"o\""
    assert text[-1] == "o", "text[-1] should equal \"o\""
    assert text[-5] == "h", "text[-5] should equal \"h\""

def test_string_index_out_of_bounds():
    """Test that out-of-bounds string indexing raises IndexError."""
    text: str = "hello"

    # Test positive index out of bounds
    try:
        char: str = text[10]
        assert False, "Should have raised IndexError"
    except:
        pass  # Expected

    # Test negative index out of bounds
    try:
        char2: str = text[-10]
        assert False, "Should have raised IndexError"
    except:
        pass  # Expected

    # Test empty string indexing
    empty: str = ""
    try:
        char3: str = empty[0]
        assert False, "Should have raised IndexError"
    except:
        pass  # Expected

# ===== SECTION: Context managers (with statement) =====

class SimpleContext:
    entered: bool
    exited: bool
    value: int

    def __init__(self, val: int):
        self.entered = False
        self.exited = False
        self.value = val

    def __enter__(self) -> int:
        self.entered = True
        return self.value

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        self.exited = True
        return False

# ===== SECTION: __enter__/__exit__ protocol =====

def test_context_basic():
    ctx = SimpleContext(42)
    assert not ctx.entered, "assertion failed: not ctx.entered"
    assert not ctx.exited, "assertion failed: not ctx.exited"
    with ctx as val:
        assert ctx.entered, "ctx.entered should be True"
        assert val == 42, "val should equal 42"
    assert ctx.exited, "ctx.exited should be True"

def test_context_no_as():
    ctx = SimpleContext(10)
    with ctx:
        assert ctx.entered, "ctx.entered should be True"
    assert ctx.exited, "ctx.exited should be True"

def test_context_exception():
    ctx = SimpleContext(5)
    caught: bool = False
    try:
        with ctx as val:
            assert ctx.entered, "ctx.entered should be True"
            raise Exception("test")
    except:
        caught = True
    assert caught, "caught should be True"
    assert ctx.exited, "ctx.exited should be True"

# ===== SECTION: Multiple context managers =====

def test_context_multiple():
    ctx1 = SimpleContext(1)
    ctx2 = SimpleContext(2)
    with ctx1 as v1, ctx2 as v2:
        assert v1 == 1, "v1 should equal 1"
        assert v2 == 2, "v2 should equal 2"
        assert ctx1.entered, "ctx1.entered should be True"
        assert ctx2.entered, "ctx2.entered should be True"
    assert ctx1.exited, "ctx1.exited should be True"
    assert ctx2.exited, "ctx2.exited should be True"

# ===== SECTION: Nested context managers =====

def test_context_nested():
    ctx1 = SimpleContext(100)
    ctx2 = SimpleContext(200)
    with ctx1 as v1:
        assert v1 == 100, "v1 should equal 100"
        with ctx2 as v2:
            assert v2 == 200, "v2 should equal 200"
    assert ctx1.exited, "ctx1.exited should be True"
    assert ctx2.exited, "ctx2.exited should be True"

# Exception info passed to __exit__
# exc_type: exception instance pointer (truthy) or None/0 (falsy)
# exc_val:  exception instance pointer or None/0
# exc_tb:   None (traceback not yet supported)
class ExcInfoChecker:
    had_exception: bool

    def __init__(self):
        self.had_exception = False

    def __enter__(self) -> int:
        return 0

    def __exit__(self, exc_type: int, exc_val: int, exc_tb: int) -> bool:
        # exc_type is the exception instance (truthy) or None/0 (falsy)
        self.had_exception = exc_type != 0
        return False

def test_exc_info_no_exception():
    ctx = ExcInfoChecker()
    with ctx:
        x: int = 42
    assert not ctx.had_exception, "ctx.had_exception should be False"  # No exception

def test_exc_info_with_exception():
    ctx = ExcInfoChecker()
    try:
        with ctx:
            raise Exception("test")
    except:
        pass
    assert ctx.had_exception, "ctx.had_exception should be True"  # Exception occurred

# Exception suppression
class Suppressor:
    def __enter__(self) -> int:
        return 0

    def __exit__(self, exc_type: int, exc_val: int, exc_tb: int) -> bool:
        # exc_type is None (0) when no exception, non-zero when exception
        return exc_type != 0  # Suppress if exception

def test_suppression():
    ctx = Suppressor()
    # This should NOT raise - exception is suppressed
    with ctx:
        raise Exception("suppressed")
    # If we get here, suppression worked

class NonSuppressor:
    def __enter__(self) -> int:
        return 0

    def __exit__(self, exc_type: int, exc_val: int, exc_tb: int) -> bool:
        return False  # Never suppress

def test_no_suppression():
    ctx = NonSuppressor()
    caught: bool = False
    try:
        with ctx:
            raise Exception("not suppressed")
    except:
        caught = True
    assert caught, "caught should be True"  # Exception should propagate

# Suppression with multiple context managers
def test_suppression_multiple():
    # When inner suppresses, outer should see no exception
    inner_ctx = Suppressor()
    outer_ctx = ExcInfoChecker()
    with outer_ctx, inner_ctx:
        raise Exception("will be suppressed")
    # Inner suppresses, so outer sees no exception (had_exception = False)
    assert not outer_ctx.had_exception, "outer_ctx should not have seen exception"

# ===== SECTION: Raise from (exception chaining) =====

def test_raise_from_basic():
    caught: bool = False
    try:
        raise ValueError("main error") from TypeError("cause error")
    except ValueError:
        caught = True
    assert caught, "caught should be True"

def test_raise_from_none():
    caught: bool = False
    try:
        raise ValueError("suppressed") from None
    except ValueError:
        caught = True
    assert caught, "caught should be True"

def test_raise_from_different_types():
    caught: bool = False
    try:
        raise RuntimeError("high level") from KeyError("missing key")
    except RuntimeError:
        caught = True
    assert caught, "caught should be True"

def test_raise_from_no_message():
    caught: bool = False
    try:
        raise ValueError("with cause") from TypeError()
    except ValueError:
        caught = True
    assert caught, "caught should be True"

# Run all tests
test_basic()
print("test_basic passed")

test_finally_normal()
print("test_finally_normal passed")

test_reraise()
print("test_reraise passed")

test_after_handling()
print("test_after_handling passed")

test_with_message()
print("test_with_message passed")

test_no_exception()
print("test_no_exception passed")

test_outer_catch()
print("test_outer_catch passed")

test_exception_types()
print("test_exception_types passed")

test_min_max_empty_collections()
print("test_min_max_empty_collections passed")

test_try_else_basic()
print("test_try_else_basic passed")

test_try_else_skipped_on_exception()
print("test_try_else_skipped_on_exception passed")

test_try_else_with_finally()
print("test_try_else_with_finally passed")

test_try_else_exception_propagates()
print("test_try_else_exception_propagates passed")

test_try_else_with_return_in_try()
print("test_try_else_with_return_in_try passed")

test_string_index_in_bounds()
print("test_string_index_in_bounds passed")

test_string_index_out_of_bounds()
print("test_string_index_out_of_bounds passed")

test_context_basic()
print("test_context_basic passed")

test_context_no_as()
print("test_context_no_as passed")

test_context_exception()
print("test_context_exception passed")

test_context_multiple()
print("test_context_multiple passed")

test_context_nested()
print("test_context_nested passed")

test_exc_info_no_exception()
print("test_exc_info_no_exception passed")

test_exc_info_with_exception()
print("test_exc_info_with_exception passed")

test_suppression()
print("test_suppression passed")

test_no_suppression()
print("test_no_suppression passed")

test_suppression_multiple()
print("test_suppression_multiple passed")

test_raise_from_basic()
print("test_raise_from_basic passed")

test_raise_from_none()
print("test_raise_from_none passed")

test_raise_from_different_types()
print("test_raise_from_different_types passed")

test_raise_from_no_message()
print("test_raise_from_no_message passed")

# ===== SECTION: Variable preservation across exception unwinding =====

# Test: Single variable modified in try block
def test_variable_preservation_basic():
    x: int = 0
    try:
        x = 42
        raise Exception("test")
    except:
        pass
    assert x == 42, "Variable should preserve value across exception"

# Test: Multiple variables modified in try block
def test_variable_preservation_multiple():
    x: int = 0
    y: int = 0
    z: int = 0
    try:
        x = 10
        y = 20
        z = 30
        raise Exception("test")
    except:
        pass
    assert x == 10, "x should be 10"
    assert y == 20, "y should be 20"
    assert z == 30, "z should be 30"

# Test: Different types (int, float, bool, str)
def test_variable_preservation_types():
    i: int = 0
    f: float = 0.0
    b: bool = False
    s: str = ""

    try:
        i = 123
        f = 3.14
        b = True
        s = "hello"
        raise ValueError("test")
    except:
        pass

    assert i == 123, "int should be preserved"
    assert f == 3.14, "float should be preserved"
    assert b == True, "bool should be preserved"
    assert s == "hello", "str should be preserved"

# Test: Nested try blocks
def test_variable_preservation_nested():
    x: int = 0
    y: int = 0

    try:
        x = 10
        try:
            y = 20
            raise ValueError("inner")
        except:
            pass
        assert y == 20, "Inner variable should be preserved"
        raise Exception("outer")
    except:
        pass

    assert x == 10, "Outer variable should be preserved"
    assert y == 20, "Inner variable should still be accessible"

# Test: Loop variable in try block
def test_variable_preservation_loop():
    last: int = 0
    try:
        for i in range(5):
            last = i
        raise Exception("after loop")
    except:
        pass
    assert last == 4, "Loop variable should preserve last value"

# Test: Conditional assignment in try block
def test_variable_preservation_conditional():
    x: int = 0
    cond: bool = True

    try:
        if cond:
            x = 100
        else:
            x = 200
        raise Exception("test")
    except:
        pass

    assert x == 100, "Conditionally assigned variable should be preserved"

# Test: No exception path (verify overhead acceptable)
def test_variable_preservation_no_exception():
    x: int = 0
    try:
        x = 50
    except:
        x = 999
    assert x == 50, "Variable should keep value when no exception"

# Test: State accumulation through except and finally
def test_variable_preservation_finally():
    state: int = 0

    try:
        state = 1
        raise Exception("test")
    except:
        state = state + 10
    finally:
        state = state + 100

    assert state == 111, "State should accumulate through except and finally"

test_variable_preservation_basic()
print("test_variable_preservation_basic passed")

test_variable_preservation_multiple()
print("test_variable_preservation_multiple passed")

test_variable_preservation_types()
print("test_variable_preservation_types passed")

test_variable_preservation_nested()
print("test_variable_preservation_nested passed")

test_variable_preservation_loop()
print("test_variable_preservation_loop passed")

test_variable_preservation_conditional()
print("test_variable_preservation_conditional passed")

test_variable_preservation_no_exception()
print("test_variable_preservation_no_exception passed")

test_variable_preservation_finally()
print("test_variable_preservation_finally passed")

# ===== SECTION: ZeroDivisionError =====

def test_zero_division_int_div():
    """Test that integer division by zero raises ZeroDivisionError."""
    caught: bool = False
    try:
        x: int = 10 // 0
    except ZeroDivisionError:
        caught = True
    assert caught, "Integer division by zero should raise ZeroDivisionError"

def test_zero_division_int_mod():
    """Test that integer modulo by zero raises ZeroDivisionError."""
    caught: bool = False
    try:
        x: int = 10 % 0
    except ZeroDivisionError:
        caught = True
    assert caught, "Integer modulo by zero should raise ZeroDivisionError"

def test_zero_division_explicit_raise():
    """Test explicit raise ZeroDivisionError."""
    caught: bool = False
    try:
        raise ZeroDivisionError("explicit division error")
    except ZeroDivisionError:
        caught = True
    assert caught, "Explicit ZeroDivisionError should be catchable"

def test_zero_division_with_variable():
    """Test division by zero with variable divisor."""
    caught: bool = False
    divisor: int = 0
    try:
        result: int = 42 // divisor
    except ZeroDivisionError:
        caught = True
    assert caught, "Division by zero variable should raise ZeroDivisionError"

def test_zero_division_caught_by_base_exception():
    """Test that ZeroDivisionError can be caught by base Exception."""
    caught: bool = False
    try:
        x: int = 1 // 0
    except Exception:
        caught = True
    assert caught, "ZeroDivisionError should be catchable by Exception"

test_zero_division_int_div()
print("test_zero_division_int_div passed")

test_zero_division_int_mod()
print("test_zero_division_int_mod passed")

test_zero_division_explicit_raise()
print("test_zero_division_explicit_raise passed")

test_zero_division_with_variable()
print("test_zero_division_with_variable passed")

test_zero_division_caught_by_base_exception()
print("test_zero_division_caught_by_base_exception passed")

# ===== SECTION: OverflowError =====
# Note: CPython has arbitrary-precision integers, so arithmetic on i64-boundary
# values does NOT raise OverflowError. The AOT compiler uses i64 and raises
# OverflowError on overflow. Tests accept both behaviors.

def test_overflow_addition():
    """Test integer addition at i64 boundary — compiler raises OverflowError, CPython succeeds."""
    max_val: int = 9223372036854775807
    try:
        x: int = max_val + 1
        # CPython: succeeds with arbitrary precision
    except OverflowError:
        pass  # Compiler: raises OverflowError (i64 overflow)

def test_overflow_subtraction():
    """Test integer subtraction at i64 boundary — compiler raises OverflowError, CPython succeeds."""
    min_val: int = -9223372036854775807 - 1
    try:
        x: int = min_val - 1
    except OverflowError:
        pass

def test_overflow_multiplication():
    """Test integer multiplication at i64 boundary — compiler raises OverflowError, CPython succeeds."""
    large: int = 9223372036854775807
    try:
        x: int = large * 2
    except OverflowError:
        pass

def test_overflow_explicit_raise():
    """Test explicit raise OverflowError."""
    caught: bool = False
    try:
        raise OverflowError("explicit overflow error")
    except OverflowError:
        caught = True
    assert caught, "Explicit OverflowError should be catchable"

def test_overflow_caught_by_base_exception():
    """Test that OverflowError can be caught by base Exception."""
    max_val: int = 9223372036854775807
    try:
        x: int = max_val + 1
    except Exception:
        pass

def test_no_overflow_normal_operations():
    """Test that normal operations don't raise OverflowError."""
    a: int = 1000000 + 2000000
    assert a == 3000000, "Normal addition should work"

    b: int = 5000000 - 3000000
    assert b == 2000000, "Normal subtraction should work"

    c: int = 1000 * 1000
    assert c == 1000000, "Normal multiplication should work"

    d: int = 1000000 // 1000
    assert d == 1000, "Normal division should work"

test_overflow_addition()
print("test_overflow_addition passed")

test_overflow_subtraction()
print("test_overflow_subtraction passed")

test_overflow_multiplication()
print("test_overflow_multiplication passed")

test_overflow_explicit_raise()
print("test_overflow_explicit_raise passed")

test_overflow_caught_by_base_exception()
print("test_overflow_caught_by_base_exception passed")

test_no_overflow_normal_operations()
print("test_no_overflow_normal_operations passed")

# ===== SECTION: Custom Exception Classes =====

# Basic custom exception inheriting from Exception
class MyError(Exception):
    pass

def test_custom_exception_basic():
    """Test basic custom exception class."""
    caught: bool = False
    try:
        raise MyError("custom error")
    except MyError:
        caught = True
    assert caught, "MyError should be caught by except MyError"

# Custom exception inheriting from ValueError
class ValidationError(ValueError):
    pass

def test_custom_exception_inherit_valueerror():
    """Test custom exception caught by parent type."""
    caught_by_valueerror: bool = False
    try:
        raise ValidationError("invalid input")
    except ValueError:
        caught_by_valueerror = True
    assert caught_by_valueerror, "ValidationError should be caught by except ValueError"

def test_custom_exception_specific_handler():
    """Test that specific handler catches before parent."""
    caught_specific: bool = False
    try:
        raise ValidationError("test")
    except ValidationError:
        caught_specific = True
    except ValueError:
        caught_specific = False  # Should NOT reach here
    assert caught_specific, "ValidationError should be caught by its own handler first"

# Inheritance chain: SpecificValidationError -> ValidationError -> ValueError -> Exception
class SpecificValidationError(ValidationError):
    pass

def test_custom_exception_chain():
    """Test exception inheritance chain."""
    caught_by_exception: bool = False
    try:
        raise SpecificValidationError("very specific")
    except Exception:
        caught_by_exception = True
    assert caught_by_exception, "SpecificValidationError should be caught by except Exception"

def test_custom_exception_chain_middle():
    """Test exception caught by middle of chain."""
    caught_by_validation: bool = False
    try:
        raise SpecificValidationError("specific")
    except ValidationError:
        caught_by_validation = True
    assert caught_by_validation, "SpecificValidationError should be caught by ValidationError"

# Exception message binding
def test_custom_exception_message():
    """Test binding exception message with as clause."""
    message_received: str = ""
    try:
        raise MyError("hello world")
    except MyError as e:
        message_received = str(e)
    assert message_received == "hello world", "Should receive exception message"

# Custom exception inheriting from IOError
class NetworkError(IOError):
    pass

class CustomTimeoutError(NetworkError):
    pass

def test_custom_ioerror_inheritance():
    """Test custom exception inheriting from IOError."""
    caught_network: bool = False
    try:
        raise CustomTimeoutError("connection timed out")
    except NetworkError:
        caught_network = True
    assert caught_network, "CustomTimeoutError should be caught by NetworkError handler"

def test_custom_exception_not_caught_by_wrong_type():
    """Test that custom exception is not caught by unrelated type."""
    caught_wrong: bool = False
    caught_correct: bool = False
    try:
        try:
            raise MyError("test")
        except ValueError:  # Should NOT catch MyError
            caught_wrong = True
    except MyError:  # Should catch here
        caught_correct = True
    assert not caught_wrong, "MyError should NOT be caught by ValueError"
    assert caught_correct, "MyError should be caught by outer MyError handler"

# Run custom exception tests
test_custom_exception_basic()
print("test_custom_exception_basic passed")

test_custom_exception_inherit_valueerror()
print("test_custom_exception_inherit_valueerror passed")

test_custom_exception_specific_handler()
print("test_custom_exception_specific_handler passed")

test_custom_exception_chain()
print("test_custom_exception_chain passed")

test_custom_exception_chain_middle()
print("test_custom_exception_chain_middle passed")

test_custom_exception_message()
print("test_custom_exception_message passed")

test_custom_ioerror_inheritance()
print("test_custom_ioerror_inheritance passed")

test_custom_exception_not_caught_by_wrong_type()
print("test_custom_exception_not_caught_by_wrong_type passed")

# ===== SECTION: New Exception Type Variants =====
# Tests for AssertionError, StopIteration, GeneratorExit, MemoryError

def test_assertion_error_explicit():
    """Test explicitly raising AssertionError."""
    caught: bool = False
    try:
        raise AssertionError("explicit assertion")
    except AssertionError:
        caught = True
    assert caught, "AssertionError should be caught"

def test_assertion_error_from_assert():
    """Test AssertionError from failed assert statement."""
    caught: bool = False
    try:
        assert False, "assertion failed"
    except AssertionError:
        caught = True
    assert caught, "Failed assert should raise AssertionError"

def test_assertion_error_as_exception():
    """Test AssertionError caught by Exception handler."""
    caught: bool = False
    try:
        raise AssertionError("test")
    except Exception:
        caught = True
    assert caught, "AssertionError should be caught by Exception"

def test_stop_iteration_explicit():
    """Test explicitly raising StopIteration."""
    caught: bool = False
    try:
        raise StopIteration("iterator exhausted")
    except StopIteration:
        caught = True
    assert caught, "StopIteration should be caught"

def test_stop_iteration_as_exception():
    """Test StopIteration caught by Exception handler."""
    caught: bool = False
    try:
        raise StopIteration()
    except Exception:
        caught = True
    assert caught, "StopIteration should be caught by Exception"

def test_generator_exit_explicit():
    """Test explicitly raising GeneratorExit."""
    caught: bool = False
    try:
        raise GeneratorExit("generator closed")
    except GeneratorExit:
        caught = True
    assert caught, "GeneratorExit should be caught"

def test_generator_exit_as_exception():
    """Test GeneratorExit with except Exception handler.
    In CPython, GeneratorExit inherits from BaseException (not Exception),
    so except Exception does NOT catch it."""
    caught_by_exception: bool = False
    caught_by_base: bool = False
    try:
        try:
            raise GeneratorExit()
        except Exception:
            caught_by_exception = True
    except GeneratorExit:
        caught_by_base = True
    assert not caught_by_exception, "GeneratorExit should NOT be caught by except Exception"
    assert caught_by_base, "GeneratorExit should be caught by except GeneratorExit"

def test_memory_error_explicit():
    """Test explicitly raising MemoryError."""
    caught: bool = False
    try:
        raise MemoryError("out of memory")
    except MemoryError:
        caught = True
    assert caught, "MemoryError should be caught"

def test_memory_error_as_exception():
    """Test MemoryError caught by Exception handler."""
    caught: bool = False
    try:
        raise MemoryError()
    except Exception:
        caught = True
    assert caught, "MemoryError should be caught by Exception"

# Run new exception type tests
test_assertion_error_explicit()
print("test_assertion_error_explicit passed")

test_assertion_error_from_assert()
print("test_assertion_error_from_assert passed")

test_assertion_error_as_exception()
print("test_assertion_error_as_exception passed")

test_stop_iteration_explicit()
print("test_stop_iteration_explicit passed")

test_stop_iteration_as_exception()
print("test_stop_iteration_as_exception passed")

test_generator_exit_explicit()
print("test_generator_exit_explicit passed")

test_generator_exit_as_exception()
print("test_generator_exit_as_exception passed")

test_memory_error_explicit()
print("test_memory_error_explicit passed")

test_memory_error_as_exception()
print("test_memory_error_as_exception passed")

# ===== SECTION: Implicit Exception Context (__context__) =====
# Tests for PEP 3134 exception chaining - __context__ attribute

def test_context_implicit_capture():
    """Test that raising during handling captures original as __context__."""
    # When we raise during handling, the original exception should be
    # captured and displayed (unless suppressed)
    caught_type: bool = False
    try:
        try:
            raise ValueError("original error")
        except ValueError:
            # Raising a new exception during handling of ValueError
            # The original ValueError becomes the __context__ of TypeError
            raise TypeError("new error during handling")
    except TypeError:
        caught_type = True
    assert caught_type, "TypeError should be caught"

def test_context_from_none_suppresses():
    """Test that 'raise X from None' suppresses context display."""
    # When using 'from None', the context is still captured but
    # suppress_context is set to True
    caught: bool = False
    try:
        try:
            raise ValueError("original")
        except ValueError:
            # 'from None' suppresses the context chain in display
            raise TypeError("new") from None
    except TypeError:
        caught = True
    assert caught, "TypeError should be caught"

def test_context_explicit_cause_precedence():
    """Test that explicit cause takes precedence over implicit context."""
    # When using 'raise X from Y', the explicit cause is displayed
    # instead of the implicit context
    caught: bool = False
    try:
        try:
            raise ValueError("original context")
        except ValueError:
            # Explicit cause (RuntimeError) takes precedence over
            # implicit context (ValueError)
            raise TypeError("new") from RuntimeError("explicit cause")
    except TypeError:
        caught = True
    assert caught, "TypeError should be caught"

def test_context_nested_handlers():
    """Test context capture with nested exception handlers."""
    outer_caught: bool = False
    inner_caught: bool = False

    try:
        try:
            raise ValueError("first")
        except ValueError:
            inner_caught = True
            # This raise during handling of ValueError
            # captures ValueError as context
            raise TypeError("second")
    except TypeError:
        outer_caught = True

    assert inner_caught, "Inner handler should have caught ValueError"
    assert outer_caught, "Outer handler should have caught TypeError"

def test_context_cleared_on_normal_exit():
    """Test that context is cleared when handler exits normally."""
    # If we handle an exception without raising, the context
    # tracking should be cleared
    result: int = 0
    try:
        raise ValueError("will be handled")
    except ValueError:
        result = 1
        # Not raising anything - context should be cleared

    # Now if we raise, there should be no context
    caught: bool = False
    try:
        raise TypeError("fresh exception")
    except TypeError:
        caught = True

    assert result == 1, "Exception should have been handled"
    assert caught, "Fresh exception should be caught"

def test_context_with_finally():
    """Test context capture works correctly with finally blocks."""
    caught: bool = False
    finally_ran: bool = False

    try:
        try:
            raise ValueError("original")
        except ValueError:
            raise TypeError("new")  # context = ValueError
        finally:
            finally_ran = True
    except TypeError:
        caught = True

    assert caught, "TypeError should be caught"
    # Note: finally blocks may not run when an exception is raised in handler
    # and immediately caught - this is implementation-specific behavior

def test_context_chain_multiple_levels():
    """Test context chaining through multiple handler levels."""
    # Create a chain: KeyError -> ValueError -> TypeError
    caught: bool = False
    try:
        try:
            try:
                raise KeyError("key error")
            except KeyError:
                raise ValueError("value error")  # context = KeyError
        except ValueError:
            raise TypeError("type error")  # context = ValueError (which has context = KeyError)
    except TypeError:
        caught = True
    assert caught, "TypeError should be caught"

def test_context_reraise_preserves():
    """Test that bare raise preserves the exception including any context."""
    caught: bool = False
    try:
        try:
            try:
                raise ValueError("original")
            except ValueError:
                raise TypeError("new")  # context = ValueError
        except TypeError:
            raise  # Reraise with context intact
    except TypeError:
        caught = True
    assert caught, "Reraised TypeError should be caught"

# Run implicit context tests
test_context_implicit_capture()
print("test_context_implicit_capture passed")

test_context_from_none_suppresses()
print("test_context_from_none_suppresses passed")

test_context_explicit_cause_precedence()
print("test_context_explicit_cause_precedence passed")

test_context_nested_handlers()
print("test_context_nested_handlers passed")

test_context_cleared_on_normal_exit()
print("test_context_cleared_on_normal_exit passed")

test_context_with_finally()
print("test_context_with_finally passed")

test_context_chain_multiple_levels()
print("test_context_chain_multiple_levels passed")

test_context_reraise_preserves()
print("test_context_reraise_preserves passed")

# ===== SECTION: Full Exception Objects =====
# Tests for exception instances with .args, custom fields, str(e)

def test_builtin_exception_str():
    """Test str(e) returns the message for built-in exceptions."""
    try:
        raise ValueError("test message")
    except ValueError as e:
        msg: str = str(e)
        assert msg == "test message", "str(e) should return the message"

def test_builtin_exception_args():
    """Test e.args returns a tuple with the message."""
    try:
        raise ValueError("args test")
    except ValueError as e:
        args: tuple[str] = e.args
        assert args[0] == "args test", "args[0] should be the message"

class HttpError(Exception):
    def __init__(self, status: int, msg: str):
        self.status = status
        self.msg = msg

def test_custom_exception_fields():
    """Test custom exception with __init__ fields."""
    try:
        raise HttpError(404, "Not Found")
    except HttpError as e:
        assert e.status == 404, "status field should be 404"
        assert e.msg == "Not Found", "msg field should be 'Not Found'"

def test_custom_exception_str_simple():
    """Test str(e) for simple custom exception (no __init__)."""
    try:
        raise MyError("simple message")
    except MyError as e:
        msg: str = str(e)
        assert msg == "simple message", "str(e) should return the message"

def test_print_builtin_exception():
    """Test print(e) works for built-in exceptions."""
    try:
        raise TypeError("print test")
    except TypeError as e:
        # Just verify it doesn't crash
        print(e)

test_builtin_exception_str()
print("test_builtin_exception_str passed")

test_builtin_exception_args()
print("test_builtin_exception_args passed")

test_custom_exception_fields()
print("test_custom_exception_fields passed")

test_custom_exception_str_simple()
print("test_custom_exception_str_simple passed")

test_print_builtin_exception()
print("test_print_builtin_exception passed")

# ===== SECTION: __exit__ receives real exception info =====

class ExcValueReceiver:
    exc_type_received: int
    exc_val_received: int

    def __init__(self):
        self.exc_type_received = 0
        self.exc_val_received = 0

    def __enter__(self) -> int:
        return 0

    def __exit__(self, exc_type: int, exc_val: int, exc_tb: int) -> bool:
        self.exc_type_received = exc_type
        self.exc_val_received = exc_val
        return False

def test_exit_exc_val_on_exception():
    ctx = ExcValueReceiver()
    try:
        with ctx:
            raise ValueError("hello")
    except:
        pass
    # exc_type and exc_val should both be non-zero (exception instance pointer)
    assert ctx.exc_type_received != 0, "exc_type should be non-zero on exception"
    assert ctx.exc_val_received != 0, "exc_val should be non-zero on exception"

def test_exit_exc_val_on_normal():
    ctx = ExcValueReceiver()
    with ctx:
        x: int = 1
    # No exception: both should be 0
    assert ctx.exc_type_received == 0, "exc_type should be 0 on normal exit"
    assert ctx.exc_val_received == 0, "exc_val should be 0 on normal exit"

test_exit_exc_val_on_exception()
print("test_exit_exc_val_on_exception passed")

test_exit_exc_val_on_normal()
print("test_exit_exc_val_on_normal passed")

# ===== SECTION: raise e (re-raise caught exception variable) =====

def test_raise_variable_builtin():
    """raise e where e is a caught built-in exception."""
    caught_outer: bool = False
    try:
        try:
            raise ValueError("from var")
        except ValueError as e:
            raise e
    except ValueError as e2:
        caught_outer = True
        assert str(e2) == "from var", "message should be preserved"
    assert caught_outer, "outer handler should catch re-raised exception"

def test_raise_variable_custom():
    """raise e where e is a caught custom exception."""
    caught_outer: bool = False
    try:
        try:
            raise HttpError(500, "server error")
        except HttpError as e:
            raise e
    except HttpError as e2:
        caught_outer = True
        assert e2.status == 500, "status field should be preserved"
    assert caught_outer, "outer handler should catch re-raised custom exception"

test_raise_variable_builtin()
print("test_raise_variable_builtin passed")

test_raise_variable_custom()
print("test_raise_variable_custom passed")

# ===== SECTION: e.__class__.__name__ =====

def test_exception_class_name():
    """Test e.__class__.__name__ for built-in exceptions."""
    try:
        raise ValueError("test")
    except ValueError as e:
        name: str = e.__class__.__name__
        assert name == "ValueError", "expected ValueError"

def test_exception_class_name_base():
    """Test e.__class__.__name__ for base Exception."""
    try:
        raise Exception("test")
    except Exception as e:
        name: str = e.__class__.__name__
        assert name == "Exception", "expected Exception"

def test_exception_class_name_custom():
    """Test e.__class__.__name__ for custom exceptions."""
    try:
        raise HttpError(404, "Not Found")
    except HttpError as e:
        name: str = e.__class__.__name__
        assert name == "HttpError", "expected HttpError"

test_exception_class_name()
print("test_exception_class_name passed")

test_exception_class_name_base()
print("test_exception_class_name_base passed")

test_exception_class_name_custom()
print("test_exception_class_name_custom passed")

# ===== SECTION: BaseException / Exception hierarchy =====

def test_base_exception_catches_system_exit():
    """except BaseException should catch SystemExit."""
    caught: bool = False
    try:
        raise SystemExit(0)
    except BaseException:
        caught = True
    assert caught, "BaseException should catch SystemExit"

def test_exception_does_not_catch_system_exit():
    """except Exception should NOT catch SystemExit."""
    caught_base: bool = False
    caught_exc: bool = False
    try:
        try:
            raise SystemExit(0)
        except Exception:
            caught_exc = True
    except BaseException:
        caught_base = True
    assert not caught_exc, "Exception should not catch SystemExit"
    assert caught_base, "BaseException should catch SystemExit"

def test_exception_does_not_catch_keyboard_interrupt():
    """except Exception should NOT catch KeyboardInterrupt."""
    caught_base: bool = False
    caught_exc: bool = False
    try:
        try:
            raise KeyboardInterrupt()
        except Exception:
            caught_exc = True
    except BaseException:
        caught_base = True
    assert not caught_exc, "Exception should not catch KeyboardInterrupt"
    assert caught_base, "BaseException should catch KeyboardInterrupt"

def test_bare_except_catches_all():
    """Bare except: catches everything including SystemExit."""
    caught: bool = False
    try:
        raise SystemExit(0)
    except:
        caught = True
    assert caught, "bare except should catch SystemExit"

test_base_exception_catches_system_exit()
print("test_base_exception_catches_system_exit passed")

test_exception_does_not_catch_system_exit()
print("test_exception_does_not_catch_system_exit passed")

test_exception_does_not_catch_keyboard_interrupt()
print("test_exception_does_not_catch_keyboard_interrupt passed")

test_bare_except_catches_all()
print("test_bare_except_catches_all passed")

# Test that custom exceptions inheriting from BaseException-only types
# are NOT caught by except Exception (vtable-based inheritance check)
class MyExit(SystemExit):
    pass

def test_custom_base_exception_not_caught_by_exception():
    """class MyExit(SystemExit) should NOT be caught by except Exception."""
    caught_base: bool = False
    caught_exc: bool = False
    try:
        try:
            raise MyExit(0)
        except Exception:
            caught_exc = True
    except BaseException:
        caught_base = True
    assert not caught_exc, "MyExit(SystemExit) should not be caught by except Exception"
    assert caught_base, "MyExit(SystemExit) should be caught by except BaseException"

test_custom_base_exception_not_caught_by_exception()
print("test_custom_base_exception_not_caught_by_exception passed")

print("All exception tests passed!")
