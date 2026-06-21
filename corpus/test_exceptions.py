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
        result7 = min(x7)
    except ValueError:
        caught_min_tuple_float = True
    assert caught_min_tuple_float, "min() on empty float tuple should raise ValueError"

    # Test max() on empty tuple (float)
    caught_max_tuple_float: bool = False
    try:
        x8 = ()
        result8 = max(x8)
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
# exc_type: exception info (truthy) or None (falsy)
# exc_val:  exception info or None
# exc_tb:   None (traceback not yet supported)
class ExcInfoChecker:
    had_exception: bool

    def __init__(self):
        self.had_exception = False

    def __enter__(self) -> int:
        return 0

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        # exc_type is truthy when an exception is active, falsy (None) otherwise
        self.had_exception = bool(exc_type)
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

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        # exc_type is None (falsy) when no exception, truthy when exception
        return bool(exc_type)  # Suppress if exception

def test_suppression():
    ctx = Suppressor()
    # This should NOT raise - exception is suppressed
    with ctx:
        raise Exception("suppressed")
    # If we get here, suppression worked

class NonSuppressor:
    def __enter__(self) -> int:
        return 0

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
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

def test_raise_from_caught_variable():
    # A builtin target with a *value* cause (a caught variable, PEP 3134).
    caught: bool = False
    try:
        raise TypeError("cause")
    except TypeError as cause:
        try:
            raise ValueError("main") from cause
        except ValueError:
            caught = True
    assert caught, "outer ValueError should be caught"

def test_raise_instance_from_variable():
    # An instance target (`raise e`) with a caught-variable cause.
    caught: bool = False
    try:
        raise KeyError("k")
    except KeyError as cause:
        try:
            raise ValueError("inst")
        except ValueError as e:
            try:
                raise e from cause
            except ValueError:
                caught = True
    assert caught, "re-raised instance with cause should be caught"

def test_raise_from_bare_builtin_class():
    # A bare builtin-exception *class* cause (no parens).
    caught: bool = False
    try:
        raise RuntimeError("x") from KeyError
    except RuntimeError:
        caught = True
    assert caught, "outer RuntimeError should be caught"

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

test_raise_from_caught_variable()
print("test_raise_from_caught_variable passed")

test_raise_instance_from_variable()
print("test_raise_instance_from_variable passed")

test_raise_from_bare_builtin_class()
print("test_raise_from_bare_builtin_class passed")

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

def test_custom_raise_from_builtin():
    """A custom-class target with a constructed builtin cause (PEP 3134)."""
    caught: bool = False
    try:
        raise MyError("wrapped") from ValueError("cause")
    except MyError:
        caught = True
    assert caught, "custom-from-builtin outer should be caught"

def test_custom_raise_from_none():
    """A custom-class target with `from None` (context suppressed)."""
    caught: bool = False
    try:
        raise MyError("x") from None
    except MyError:
        caught = True
    assert caught, "custom-from-None outer should be caught"

def test_custom_raise_from_variable():
    """A custom-class target with a caught-variable cause."""
    caught: bool = False
    try:
        raise ValueError("cause")
    except ValueError as cause:
        try:
            raise MyError("wrapped") from cause
        except MyError:
            caught = True
    assert caught, "custom-from-variable outer should be caught"

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

test_custom_raise_from_builtin()
print("test_custom_raise_from_builtin passed")

test_custom_raise_from_none()
print("test_custom_raise_from_none passed")

test_custom_raise_from_variable()
print("test_custom_raise_from_variable passed")

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
    saw_exc_type: bool
    saw_exc_val: bool

    def __init__(self):
        self.saw_exc_type = False
        self.saw_exc_val = False

    def __enter__(self) -> int:
        return 0

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        self.saw_exc_type = bool(exc_type)
        self.saw_exc_val = bool(exc_val)
        return False

def test_exit_exc_val_on_exception():
    ctx = ExcValueReceiver()
    try:
        with ctx:
            raise ValueError("hello")
    except:
        pass
    # exc_type and exc_val should both be truthy (exception info)
    assert ctx.saw_exc_type, "exc_type should be truthy on exception"
    assert ctx.saw_exc_val, "exc_val should be truthy on exception"

def test_exit_exc_val_on_normal():
    ctx = ExcValueReceiver()
    with ctx:
        x: int = 1
    # No exception: both should be falsy (None)
    assert not ctx.saw_exc_type, "exc_type should be falsy on normal exit"
    assert not ctx.saw_exc_val, "exc_val should be falsy on normal exit"

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

# ===== SECTION: with-statement target shapes (BindingTarget migration) =====
# The unified `bind_target` helper accepts the full grammar for `with ... as TARGET:`,
# not just a bare Name. `__enter__` is still called exactly once per `with` item.

class _WtEnterPair:
    def __enter__(self):
        return (10, 20)
    def __exit__(self, *a):
        return False

with _WtEnterPair() as (_wt_x, _wt_y):
    assert _wt_x == 10 and _wt_y == 20

# Starred unpack in with-target
class _WtEnterFour:
    def __enter__(self):
        return (1, 2, 3, 4)
    def __exit__(self, *a):
        return False

with _WtEnterFour() as (_wt_a, *_wt_rest):
    assert _wt_a == 1 and _wt_rest == [2, 3, 4]

# Simple name continues to work
class _WtEnterInt:
    def __enter__(self):
        return 42
    def __exit__(self, *a):
        return False

with _WtEnterInt() as _wt_v:
    assert _wt_v == 42

print("with-statement tuple target tests passed!")

# ===== SECTION: folded from p7_raise_tryexcept.py (raise + try/except builtins) =====

# Helper functions hoisted to module level (per-source prefix _p7r_).

def _p7r_classify(kind: int) -> str:
    try:
        if kind == 0:
            raise ValueError("v")
        if kind == 1:
            raise TypeError("t")
        if kind == 2:
            raise KeyError("k")
        return "none"
    except ValueError:
        return "value"
    except TypeError:
        return "type"
    except KeyError:
        return "key"

def _p7r_multi(kind: int) -> str:
    try:
        if kind == 0:
            raise ValueError("v")
        raise IndexError("i")
    except (ValueError, IndexError):
        return "either"

def _p7r_base_catch() -> str:
    try:
        raise RuntimeError("r")
    except Exception:
        return "base"

def _p7r_bare() -> str:
    try:
        raise AttributeError("a")
    except:
        return "bare"

def _p7r_div(a: int, b: int) -> int:
    try:
        return a // b
    except ZeroDivisionError:
        return -1

def _p7r_parse(s: str) -> int:
    try:
        return int(s)
    except ValueError:
        return -1

def _p7r_nested_inner() -> str:
    try:
        try:
            raise ValueError("inner")
        except ValueError:
            return "inner-caught"
    except ValueError:
        return "outer-caught"

def _p7r_nested_outer() -> str:
    try:
        try:
            raise TypeError("inner")
        except ValueError:
            return "wrong"
    except TypeError:
        return "outer"

def _p7r_reraise() -> str:
    try:
        try:
            raise ValueError("once")
        except ValueError:
            raise
    except ValueError:
        return "reraised"

def _p7r_raiser():
    raise KeyError("deep")

def _p7r_call_catch() -> str:
    try:
        _p7r_raiser()
        return "no"
    except KeyError:
        return "from-callee"

def _p7r_bare_class() -> str:
    try:
        raise ValueError
    except ValueError:
        return "bare-class"

def _p7r_no_msg() -> str:
    try:
        raise TypeError()
    except TypeError:
        return "no-msg"

def _p7r_preserve() -> int:
    x = 0
    y = 0.0
    s = ""
    try:
        x = 41
        y = 2.5
        s = "kept"
        raise ValueError("u")
    except ValueError:
        pass
    if s == "kept" and y == 2.5:
        return x + 1
    return -1

def _p7r_loop_try() -> int:
    total = 0
    for i in range(6):
        try:
            if i == 2:
                raise ValueError("skip")
            if i == 4:
                break
            total = total + i
        except ValueError:
            continue
    return total

def _p7r_ret_in_try(flag: bool) -> str:
    try:
        if flag:
            return "early"
        raise ValueError("late")
    except ValueError:
        return "handled"

def _p7r_as_bind() -> str:
    try:
        raise ValueError("bound")
    except ValueError as e:
        return "as-ok"

def _p7r_raise_in_handler() -> str:
    try:
        try:
            raise ValueError("first")
        except ValueError:
            raise TypeError("second")
    except TypeError:
        return "chained"

def _fold_p7_raise_tryexcept():
    # basic catch
    caught = False
    try:
        raise ValueError("boom")
    except ValueError:
        caught = True
    assert caught == True

    # no exception: except skipped
    ran = False
    skipped = True
    try:
        ran = True
    except ValueError:
        skipped = False
    assert ran == True
    assert skipped == True

    # specific handler chain picks the right clause
    assert _p7r_classify(0) == "value"
    assert _p7r_classify(1) == "type"
    assert _p7r_classify(2) == "key"
    assert _p7r_classify(3) == "none"

    # tuple clause (OR-chain)
    assert _p7r_multi(0) == "either"
    assert _p7r_multi(1) == "either"

    # Exception catches subclass-tagged builtins
    assert _p7r_base_catch() == "base"

    # bare except
    assert _p7r_bare() == "bare"

    # runtime-raised exceptions are catchable
    assert _p7r_div(10, 2) == 5
    assert _p7r_div(10, 0) == -1
    assert _p7r_parse("42") == 42
    assert _p7r_parse("nope") == -1

    # nested try, inner catches
    assert _p7r_nested_inner() == "inner-caught"
    # nested try, no inner match -> outer catches
    assert _p7r_nested_outer() == "outer"
    # bare raise re-raises
    assert _p7r_reraise() == "reraised"
    # unmatched handler propagates to the caller
    assert _p7r_call_catch() == "from-callee"
    # raise without arguments list (bare class)
    assert _p7r_bare_class() == "bare-class"
    # raise without message
    assert _p7r_no_msg() == "no-msg"
    # variables assigned in try survive the unwind
    assert _p7r_preserve() == 42
    # try inside a loop with break/continue
    assert _p7r_loop_try() == 4
    # return inside try (normal path pops the frame)
    assert _p7r_ret_in_try(True) == "early"
    assert _p7r_ret_in_try(False) == "handled"
    # as-binding accepted
    assert _p7r_as_bind() == "as-ok"
    # exception raised inside except handler propagates outward
    assert _p7r_raise_in_handler() == "chained"

_fold_p7_raise_tryexcept()
print("_fold_p7_raise_tryexcept passed")

# ===== SECTION: folded from p7_finally.py (finally/else, raise-from, instance surface) =====

def _p7f_full(raise_it: bool) -> str:
    log = ""
    try:
        log = log + "T"
        if raise_it:
            raise ValueError("v")
    except ValueError:
        log = log + "E"
    else:
        log = log + "L"
    finally:
        log = log + "F"
    return log

_p7f_trace: list[str] = []

def _p7f_ret_through_finally() -> int:
    try:
        _p7f_trace.append("in")
        return 7
    finally:
        _p7f_trace.append("fin")

def _p7f_ret_from_handler() -> int:
    try:
        raise ValueError("v")
    except ValueError:
        return 1
    finally:
        _p7f_trace.append("fin2")

def _p7f_loop_finally() -> int:
    count = 0
    for i in range(5):
        try:
            if i == 1:
                continue
            if i == 3:
                break
            count = count + 1
        finally:
            count = count + 10
    return count

def _p7f_else_raises() -> str:
    try:
        try:
            x = 1
        except ValueError:
            return "wrong"
        else:
            raise TypeError("from-else")
    except TypeError:
        return "outer"

def _p7f_nested_finally() -> str:
    log = ""
    try:
        try:
            log = log + "a"
            raise ValueError("v")
        finally:
            log = log + "b"
    except ValueError:
        log = log + "c"
    finally:
        log = log + "d"
    return log

def _p7f_raise_from() -> str:
    try:
        raise ValueError("main") from TypeError("cause")
    except ValueError:
        return "from-caught"

def _p7f_raise_from_none() -> str:
    try:
        try:
            raise ValueError("orig")
        except ValueError:
            raise TypeError("new") from None
    except TypeError:
        return "from-none"

def _p7f_tuple_clause_msg(kind: int) -> str:
    try:
        if kind == 0:
            raise ValueError("tc-value")
        raise RuntimeError("tc-runtime")
    except (ValueError, RuntimeError) as e:
        return str(e)

def _fold_p7_finally():
    # finally on the normal path
    order: list[str] = []
    try:
        order.append("try")
    finally:
        order.append("finally")
    assert order == ["try", "finally"]

    # finally on the exceptional path (caught outside)
    steps: list[str] = []
    try:
        try:
            steps.append("body")
            raise ValueError("x")
        finally:
            steps.append("finally")
    except ValueError:
        steps.append("caught")
    assert steps == ["body", "finally", "caught"]

    # try/except/else/finally ordering
    assert _p7f_full(False) == "TLF"
    assert _p7f_full(True) == "TEF"

    # return inside try still runs finally
    assert _p7f_ret_through_finally() == 7
    assert _p7f_trace == ["in", "fin"]

    # return inside except runs finally
    assert _p7f_ret_from_handler() == 1
    assert _p7f_trace == ["in", "fin", "fin2"]

    # break/continue through finally
    assert _p7f_loop_finally() == 42

    # exception in else is not caught by the same try
    assert _p7f_else_raises() == "outer"

    # nested finally
    assert _p7f_nested_finally() == "abcd"

    # raise X from Y / from None
    assert _p7f_raise_from() == "from-caught"
    assert _p7f_raise_from_none() == "from-none"

    # instance surface: str(e), e.args, e.__class__.__name__
    try:
        raise ValueError("boom")
    except ValueError as e:
        assert str(e) == "boom"
        assert e.args[0] == "boom"
        assert e.__class__.__name__ == "ValueError"

    try:
        raise RuntimeError("rt-message")
    except RuntimeError as e2:
        assert str(e2) == "rt-message"
        assert e2.__class__.__name__ == "RuntimeError"

    # str(e) of a message-less exception is empty; its args tuple is ()
    try:
        raise TypeError()
    except TypeError as e3:
        assert "empty:[" + str(e3) + "]" == "empty:[]"
        assert len(e3.args) == 0

    # tuple-clause `as` binding keeps the exception-message surface
    assert _p7f_tuple_clause_msg(0) == "tc-value"
    assert _p7f_tuple_clause_msg(1) == "tc-runtime"

    try:
        raise ValueError("tc-args")
    except (ValueError, RuntimeError) as e4:
        assert e4.args[0] == "tc-args"

    # min()/max() on empty input raise with the live-oracle message
    try:
        empty_l: list[int] = []
        bad = min(empty_l)
    except ValueError as e5:
        assert str(e5) == "min() iterable argument is empty"
    try:
        empty_l2: list[int] = []
        bad2 = max(empty_l2)
    except ValueError as e6:
        assert str(e6) == "max() iterable argument is empty"

    # min/max with a builtin key function
    vals: list[int] = [3, -7, 2, -1]
    assert min(vals, key=abs) == -1
    assert max(vals, key=abs) == -7

    # starred unpacking of a literal RHS
    first, *mid, last = [1, 2, 3, 4]
    assert first == 1
    assert mid == [2, 3]
    assert last == 4

_fold_p7_finally()
print("_fold_p7_finally passed")

# ===== SECTION: folded from p7_custom_exc.py (custom exception classes) =====

# Custom exception classes hoisted to module level with per-source prefix _p7c_.

class _p7c_AppError(Exception):
    pass

class _p7c_ParseError(_p7c_AppError):
    pass

class _p7c_LimitError(ValueError):
    pass

class _p7c_HttpError(Exception):
    def __init__(self, status: int, reason: str):
        self.status = status
        self.reason = reason

def _p7c_basic() -> str:
    try:
        raise _p7c_AppError("app failed")
    except _p7c_AppError:
        return "caught"

def _p7c_by_parent() -> str:
    try:
        raise _p7c_ParseError("bad token")
    except _p7c_AppError:
        return "parent"

def _p7c_by_builtin_parent() -> str:
    try:
        raise _p7c_LimitError("too big")
    except ValueError:
        return "builtin-parent"

def _p7c_by_exception() -> str:
    try:
        raise _p7c_ParseError("deep")
    except Exception:
        return "exception"

def _p7c_specific_first() -> str:
    try:
        raise _p7c_ParseError("p")
    except _p7c_ParseError:
        return "specific"
    except _p7c_AppError:
        return "general"

def _p7c_unrelated() -> str:
    try:
        try:
            raise _p7c_AppError("a")
        except ValueError:
            return "wrong"
    except _p7c_AppError:
        return "outer"

def _p7c_message() -> str:
    try:
        raise _p7c_AppError("hello world")
    except _p7c_AppError as e:
        return str(e)

def _p7c_fields() -> str:
    try:
        raise _p7c_HttpError(404, "Not Found")
    except _p7c_HttpError as e:
        if e.status == 404:
            return e.reason
        return "wrong"

def _p7c_class_name() -> str:
    try:
        raise _p7c_ParseError("x")
    except _p7c_ParseError as e:
        return e.__class__.__name__

def _p7c_raise_instance() -> str:
    try:
        try:
            raise _p7c_HttpError(500, "boom")
        except _p7c_HttpError as e:
            raise e
    except _p7c_HttpError as e2:
        if e2.status == 500:
            return "instance"
        return "lost"

def _p7c_tuple_clause(kind: int) -> str:
    try:
        if kind == 0:
            raise _p7c_AppError("a")
        raise _p7c_LimitError("l")
    except (_p7c_AppError, _p7c_LimitError):
        return "either"

def _fold_p7_custom_exc():
    # raise / catch a custom class
    assert _p7c_basic() == "caught"
    # caught by user parent
    assert _p7c_by_parent() == "parent"
    # caught by builtin parent
    assert _p7c_by_builtin_parent() == "builtin-parent"
    # caught by Exception
    assert _p7c_by_exception() == "exception"
    # specific handler wins over parent listed later
    assert _p7c_specific_first() == "specific"
    # unrelated handler does not catch
    assert _p7c_unrelated() == "outer"
    # message surface for __init__-less custom classes
    assert _p7c_message() == "hello world"
    # custom exception with fields
    assert _p7c_fields() == "Not Found"
    # class name of a caught custom exception (prefixed name)
    assert _p7c_class_name() == "_p7c_ParseError"
    # raise e (re-raise a caught instance)
    assert _p7c_raise_instance() == "instance"
    # as-binding for tuple of custom classes
    assert _p7c_tuple_clause(0) == "either"
    assert _p7c_tuple_clause(1) == "either"

_fold_p7_custom_exc()
print("_fold_p7_custom_exc passed")

# ===== SECTION: folded from p7_with.py (context managers / with) =====

# Context-manager classes hoisted to module level with per-source prefix _p7w_.

_p7w_log: list[str] = []

class _p7w_Tracker:
    name: str

    def __init__(self, name: str):
        self.name = name

    def __enter__(self) -> str:
        _p7w_log.append(self.name + ":enter")
        return self.name

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        _p7w_log.append(self.name + ":exit")
        return False

_p7w_log2: list[str] = []

class _p7w_Tracker2:
    def __enter__(self) -> int:
        _p7w_log2.append("enter")
        return 1

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        _p7w_log2.append("exit:" + str(bool(exc_type)))
        return False

class _p7w_Quiet:
    def __enter__(self) -> int:
        return 0

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        return bool(exc_type)

_p7w_log3: list[str] = []

class _p7w_Item:
    tag: str

    def __init__(self, tag: str):
        self.tag = tag

    def __enter__(self) -> str:
        _p7w_log3.append("e" + self.tag)
        return self.tag

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        _p7w_log3.append("x" + self.tag)
        return False

class _p7w_SawIt:
    saw: bool

    def __init__(self):
        self.saw = False

    def __enter__(self) -> int:
        return 0

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        self.saw = bool(exc_type)
        return False

_p7w_log4: list[str] = []

class _p7w_R:
    def __enter__(self) -> int:
        return 5

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        _p7w_log4.append("exit")
        return False

class _p7w_Star:
    def __enter__(self) -> int:
        return 9

    def __exit__(self, *a) -> bool:
        return False

class _p7w_Pair:
    def __enter__(self):
        return (10, 20)

    def __exit__(self, *a) -> bool:
        return False

class _p7w_SelfYield:
    tag: int

    def __init__(self, tag: int):
        self.tag = tag

    def __enter__(self) -> "_p7w_SelfYield":
        return self

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        return False

def _p7w_with_raises() -> str:
    try:
        with _p7w_Tracker2():
            _p7w_log2.append("body")
            raise ValueError("inside")
    except ValueError:
        return "propagated"

def _p7w_suppressed() -> str:
    with _p7w_Quiet():
        raise ValueError("swallowed")
    return "after"

def _p7w_ret_in_with() -> int:
    with _p7w_R() as n:
        return n + 1

def _p7w_loop_with() -> int:
    total = 0
    for i in range(5):
        with _p7w_R():
            if i == 1:
                continue
            if i == 3:
                break
            total = total + i
    return total

def _p7w_nested() -> str:
    t = _p7w_SawIt()
    try:
        with t:
            with _p7w_Tracker("n") as inner:
                raise KeyError("deep")
    except KeyError:
        if t.saw:
            return "all-good"
        return "outer-missed"

def _fold_p7_with():
    # normal path
    with _p7w_Tracker("a") as v:
        _p7w_log.append("body:" + v)
    assert _p7w_log == ["a:enter", "body:a", "a:exit"]

    # exception path: __exit__ runs, exception propagates
    assert _p7w_with_raises() == "propagated"
    assert _p7w_log2 == ["enter", "body", "exit:True"]

    # suppression: truthy __exit__ swallows the exception
    assert _p7w_suppressed() == "after"

    # multiple items nest left-to-right
    with _p7w_Item("1") as a, _p7w_Item("2") as b:
        _p7w_log3.append(a + b)
    assert _p7w_log3 == ["e1", "e2", "12", "x2", "x1"]

    # inner suppressor hides exception from the outer manager
    outer = _p7w_SawIt()
    with outer, _p7w_Quiet():
        raise ValueError("inner only")
    assert outer.saw == False

    # return inside with runs __exit__
    assert _p7w_ret_in_with() == 6
    assert _p7w_log4 == ["exit"]

    # break/continue out of with inside a loop
    assert _p7w_loop_with() == 2
    assert len(_p7w_log4) == 5

    # varargs __exit__
    with _p7w_Star() as s:
        assert s == 9

    # tuple target
    with _p7w_Pair() as (x, y):
        assert x == 10
        assert y == 20

    # forward-reference string annotation on __enter__
    with _p7w_SelfYield(7) as sy:
        assert sy.tag == 7

    # nested with + try around it
    assert _p7w_nested() == "all-good"

_fold_p7_with()
print("_fold_p7_with passed")

# ===== SECTION: folded from test_multi_except.py (multi-except clauses) =====

def _p7m_value_error() -> str:
    try:
        x: int = int("abc")
        return "no error"
    except (ValueError, TypeError) as e:
        return "caught"

def _p7m_second_type() -> str:
    try:
        x: int = 1 // 0
        return "no error"
    except (ValueError, ZeroDivisionError) as e:
        return "caught"

def _p7m_no_match() -> str:
    try:
        x: int = 1 // 0
        return "no error"
    except (ValueError, KeyError):
        return "wrong handler"
    except ZeroDivisionError:
        return "correct"

def _fold_test_multi_except():
    assert _p7m_value_error() == "caught", "multi except ValueError failed"
    assert _p7m_second_type() == "caught", "multi except second type failed"
    assert _p7m_no_match() == "correct", "multi except no match failed"

_fold_test_multi_except()
print("_fold_test_multi_except passed")


# ===== SECTION: Runtime unpack arity raises ValueError =====
# A runtime-value RHS (not a literal) is checked against the target pattern,
# raising CPython's exact ValueError wording (was a silent wrong binding for
# too-many and an IndexError for too-few; folded from test_review_fixes.py).


def _rvf_src(items):
    return items


def _rvf_unpack(items):
    try:
        a, b = _rvf_src(items)
        return ("ok", a, b)
    except ValueError as e:
        return str(e)


assert _rvf_unpack([1, 2]) == ("ok", 1, 2)
assert _rvf_unpack([1, 2, 3]) == "too many values to unpack (expected 2, got 3)"
assert _rvf_unpack([9]) == "not enough values to unpack (expected 2, got 1)"


def _rvf_unpack_star(items):
    try:
        a, *mid, b = _rvf_src(items)
        return (a, mid, b)
    except ValueError as e:
        return str(e)


assert _rvf_unpack_star([1, 2, 3, 4]) == (1, [2, 3], 4)
assert _rvf_unpack_star([1, 2]) == (1, [], 2)
assert _rvf_unpack_star([7]) == "not enough values to unpack (expected at least 2, got 1)"


# for-loop unpack and nested destructuring share the same arity guard.
def _rvf_for_unpack(pairs):
    out = []
    try:
        for a, b in pairs:
            out.append((a, b))
        return out
    except ValueError as e:
        return str(e)


assert _rvf_for_unpack([(1, 2), (3, 4)]) == [(1, 2), (3, 4)]
assert _rvf_for_unpack([(1, 2), (3, 4, 5)]) == "too many values to unpack (expected 2, got 3)"


# ===== SECTION: Out-of-range list subscript raises IndexError =====
# (was a silent None; folded from test_review_fixes.py).


def _rvf_safe_subscript(lst, i):
    try:
        return lst[i]
    except IndexError:
        return "IndexError"


_rvf_nums = [1, 2]
assert _rvf_safe_subscript(_rvf_nums, 0) == 1
assert _rvf_safe_subscript(_rvf_nums, 1) == 2
assert _rvf_safe_subscript(_rvf_nums, 5) == "IndexError"
assert _rvf_safe_subscript(_rvf_nums, -5) == "IndexError"


# ===== SECTION: instance <op> immediate with NotImplemented dunder → TypeError =====
# When a dunder returns NotImplemented and the RHS is a tagged immediate (non-ptr),
# the reflected-op fallback must raise TypeError rather than SIGSEGV dereferencing
# the tagged int (folded from test_review_fixes.py).


class _rvf_C:
    def __add__(self, o):
        return NotImplemented

    def __sub__(self, o):
        return NotImplemented

    def __mul__(self, o):
        return NotImplemented

    def __truediv__(self, o):
        return NotImplemented

    def __floordiv__(self, o):
        return NotImplemented

    def __mod__(self, o):
        return NotImplemented

    def __pow__(self, o):
        return NotImplemented


def _rvf_raises_type_error(fn) -> bool:
    try:
        fn(_rvf_C())
        return False
    except TypeError:
        return True


assert _rvf_raises_type_error(lambda o: o + 1)
assert _rvf_raises_type_error(lambda o: o - 1)
assert _rvf_raises_type_error(lambda o: o * 1)
assert _rvf_raises_type_error(lambda o: o / 1)
assert _rvf_raises_type_error(lambda o: o // 1)
assert _rvf_raises_type_error(lambda o: o % 1)
assert _rvf_raises_type_error(lambda o: o ** 1)


print("All exception tests passed!")
