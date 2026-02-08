# Comprehensive generator tests
# Tests: creation, next(), for loops, send(), close()

# =============================================================================
# Basic generator definitions
# =============================================================================

def simple_gen():
    yield 1
    yield 2
    yield 3

def five_gen():
    yield 10
    yield 20
    yield 30
    yield 40
    yield 50

# =============================================================================
# Test 1: Generator creation
# =============================================================================

def test_creation():
    g = simple_gen()
    # Generator should be created without running
    print("test_creation passed")

test_creation()

# =============================================================================
# Test 2: next() iteration
# =============================================================================

def test_next():
    g = simple_gen()

    v1: int = next(g)
    assert v1 == 1, "expected 1"

    v2: int = next(g)
    assert v2 == 2, "expected 2"

    v3: int = next(g)
    assert v3 == 3, "expected 3"

    print("test_next passed")

test_next()

# =============================================================================
# Test 3: for loop iteration
# =============================================================================

def test_for_loop():
    total: int = 0
    for x in simple_gen():
        total = total + x

    assert total == 6, "expected sum = 6"
    print("test_for_loop passed")

test_for_loop()

# =============================================================================
# Test 4: Multiple generators
# =============================================================================

def test_multiple_generators():
    g1 = simple_gen()
    g2 = simple_gen()

    # Interleaved iteration
    v1: int = next(g1)
    v2: int = next(g2)
    assert v1 == 1, "g1 first should be 1"
    assert v2 == 1, "g2 first should be 1"

    v3: int = next(g1)
    assert v3 == 2, "g1 second should be 2"

    v4: int = next(g2)
    v5: int = next(g2)
    assert v4 == 2, "g2 second should be 2"
    assert v5 == 3, "g2 third should be 3"

    v6: int = next(g1)
    assert v6 == 3, "g1 third should be 3"

    print("test_multiple_generators passed")

test_multiple_generators()

# =============================================================================
# Test 5: Longer generator sequence
# =============================================================================

def test_longer_sequence():
    total: int = 0
    for x in five_gen():
        total = total + x

    # 10 + 20 + 30 + 40 + 50 = 150
    assert total == 150, "expected sum = 150"
    print("test_longer_sequence passed")

test_longer_sequence()

# =============================================================================
# Test 6: Collecting generator values into list
# =============================================================================

def test_collect_to_list():
    results: list[int] = []
    for x in simple_gen():
        results.append(x)

    assert len(results) == 3, "expected 3 elements"
    assert results[0] == 1, "first is 1"
    assert results[1] == 2, "second is 2"
    assert results[2] == 3, "third is 3"

    print("test_collect_to_list passed")

test_collect_to_list()

# =============================================================================
# Test 7: send() basic - value ignored
# =============================================================================

def test_send_basic():
    g = simple_gen()

    # Use next() to start the generator (equivalent to send(None))
    # This works in both CPython and our compiler
    val1 = next(g)
    assert val1 == 1, "expected 1"

    val2 = g.send(42)  # After first yield, can send any value (ignored here)
    assert val2 == 2, "expected 2"

    val3 = g.send(100)
    assert val3 == 3, "expected 3"

    print("test_send_basic passed")

test_send_basic()

# =============================================================================
# Test 8: send() with echo pattern
# =============================================================================

def echo_gen():
    received = yield 1  # yields 1, then received = sent value
    yield received      # yields whatever was sent

def test_send_echo():
    g = echo_gen()

    val1 = next(g)
    assert val1 == 1, "expected 1"

    val2 = g.send(42)
    assert val2 == 42, "expected 42"

    print("test_send_echo passed")

test_send_echo()

# =============================================================================
# Test 9: send() with accumulator pattern
# =============================================================================

def accumulator_gen():
    total = yield 0     # yield 0, total = first sent value
    total = yield total # yield total, total = next sent value
    yield total         # yield final total

def test_send_accumulator():
    g = accumulator_gen()

    val1 = next(g)
    assert val1 == 0, "expected 0"

    val2 = g.send(10)
    assert val2 == 10, "expected 10"

    val3 = g.send(25)
    assert val3 == 25, "expected 25"

    print("test_send_accumulator passed")

test_send_accumulator()

# =============================================================================
# Test 10: close() on exhausted generator
# =============================================================================

def test_close_exhausted():
    g = simple_gen()

    # Exhaust the generator
    for x in g:
        pass

    # close() on exhausted generator should be fine
    g.close()

    print("test_close_exhausted passed")

test_close_exhausted()

# =============================================================================
# Test 11: Reuse generator variable
# =============================================================================

def test_reuse_variable():
    g = simple_gen()
    v1: int = next(g)
    assert v1 == 1, "first is 1"

    # Reassign to new generator
    g = simple_gen()
    v2: int = next(g)
    assert v2 == 1, "new generator starts at 1"

    print("test_reuse_variable passed")

test_reuse_variable()

# =============================================================================
# Test 12: Multiple send() echo generators
# =============================================================================

def test_multiple_echo():
    g1 = echo_gen()
    g2 = echo_gen()

    # Start both
    v1: int = next(g1)
    v2: int = next(g2)
    assert v1 == 1, "g1 yields 1"
    assert v2 == 1, "g2 yields 1"

    # Send different values
    r1: int = g1.send(100)
    r2: int = g2.send(200)
    assert r1 == 100, "g1 echoes 100"
    assert r2 == 200, "g2 echoes 200"

    print("test_multiple_echo passed")

test_multiple_echo()

# =============================================================================
# Test 13: Chained generator iteration
# =============================================================================

def test_chained_iteration():
    result: list[int] = []

    for x in simple_gen():
        result.append(x)

    for x in five_gen():
        result.append(x)

    assert len(result) == 8, "expected 8 elements"
    assert result[0] == 1, "first is 1"
    assert result[1] == 2, "second is 2"
    assert result[2] == 3, "third is 3"
    assert result[3] == 10, "fourth is 10"
    assert result[4] == 20, "fifth is 20"
    assert result[5] == 30, "sixth is 30"
    assert result[6] == 40, "seventh is 40"
    assert result[7] == 50, "eighth is 50"

    print("test_chained_iteration passed")

test_chained_iteration()

# =============================================================================
# Test 14: Partial iteration then for loop
# =============================================================================

def test_partial_then_for():
    g = five_gen()

    # Take first two with next()
    v1: int = next(g)
    v2: int = next(g)
    assert v1 == 10, "first is 10"
    assert v2 == 20, "second is 20"

    # Continue with for loop
    rest: list[int] = []
    for x in g:
        rest.append(x)

    assert len(rest) == 3, "3 remaining"
    assert rest[0] == 30, "30"
    assert rest[1] == 40, "40"
    assert rest[2] == 50, "50"

    print("test_partial_then_for passed")

test_partial_then_for()

# =============================================================================
# Test 15: While-loop generator with parameters
# =============================================================================

def range_gen(start: int, end: int):
    i: int = start
    while i < end:
        yield i
        i = i + 1

def test_while_loop_generator():
    g = range_gen(5, 10)

    v1: int = next(g)
    assert v1 == 5, "first is 5"

    v2: int = next(g)
    assert v2 == 6, "second is 6"

    v3: int = next(g)
    assert v3 == 7, "third is 7"

    v4: int = next(g)
    assert v4 == 8, "fourth is 8"

    v5: int = next(g)
    assert v5 == 9, "fifth is 9"

    print("test_while_loop_generator passed")

test_while_loop_generator()

# =============================================================================
# Test 16: For loop over while-loop generator
# =============================================================================

def test_for_over_while_gen():
    total: int = 0
    for x in range_gen(1, 6):
        total = total + x

    # 1 + 2 + 3 + 4 + 5 = 15
    assert total == 15, "expected sum = 15"

    print("test_for_over_while_gen passed")

test_for_over_while_gen()

# =============================================================================
# Test 17: Multiple while-loop generators
# =============================================================================

def test_multiple_while_gens():
    g1 = range_gen(0, 3)
    g2 = range_gen(10, 13)

    # Interleaved iteration
    v1: int = next(g1)
    v2: int = next(g2)
    assert v1 == 0, "g1 starts at 0"
    assert v2 == 10, "g2 starts at 10"

    v3: int = next(g1)
    v4: int = next(g2)
    assert v3 == 1, "g1 second is 1"
    assert v4 == 11, "g2 second is 11"

    print("test_multiple_while_gens passed")

test_multiple_while_gens()

# =============================================================================
# Test 18: Simple generator expression
# =============================================================================

def test_simple_genexp():
    g = (x for x in range_gen(1, 4))

    v1: int = next(g)
    assert v1 == 1, "first is 1"

    v2: int = next(g)
    assert v2 == 2, "second is 2"

    v3: int = next(g)
    assert v3 == 3, "third is 3"

    print("test_simple_genexp passed")

test_simple_genexp()

# =============================================================================
# Test 19: Generator expression with transformation
# =============================================================================

def test_genexp_transform():
    g = (x * x for x in range_gen(1, 5))

    results: list[int] = []
    for val in g:
        results.append(val)

    # 1*1, 2*2, 3*3, 4*4 = 1, 4, 9, 16
    assert len(results) == 4, "4 results"
    assert results[0] == 1, "1*1 = 1"
    assert results[1] == 4, "2*2 = 4"
    assert results[2] == 9, "3*3 = 9"
    assert results[3] == 16, "4*4 = 16"

    print("test_genexp_transform passed")

test_genexp_transform()

# =============================================================================
# Test 20: Generator expression with condition
# =============================================================================

def test_genexp_filter():
    # Only even numbers: 2, 4
    g = (x for x in range_gen(1, 6) if x % 2 == 0)

    v1: int = next(g)
    assert v1 == 2, "first even is 2"

    v2: int = next(g)
    assert v2 == 4, "second even is 4"

    print("test_genexp_filter passed")

test_genexp_filter()

# =============================================================================
# Test 21: Generator expression for loop iteration
# =============================================================================

def test_genexp_for_loop():
    total: int = 0
    for x in (v * 2 for v in range_gen(1, 4)):
        total = total + x

    # 1*2 + 2*2 + 3*2 = 2 + 4 + 6 = 12
    assert total == 12, "expected sum = 12"

    print("test_genexp_for_loop passed")

test_genexp_for_loop()

# =============================================================================
# Test 22: Multiple yields in while loop (two yields)
# =============================================================================

def two_yields_gen():
    i: int = 0
    while i < 3:
        yield i
        yield i * 2
        i = i + 1

def test_two_yields():
    g = two_yields_gen()

    # Iteration 1: i=0
    v1: int = next(g)
    assert v1 == 0, "first yield: i=0"
    v2: int = next(g)
    assert v2 == 0, "second yield: i*2=0"

    # Iteration 2: i=1
    v3: int = next(g)
    assert v3 == 1, "first yield: i=1"
    v4: int = next(g)
    assert v4 == 2, "second yield: i*2=2"

    # Iteration 3: i=2
    v5: int = next(g)
    assert v5 == 2, "first yield: i=2"
    v6: int = next(g)
    assert v6 == 4, "second yield: i*2=4"

    print("test_two_yields passed")

test_two_yields()

# =============================================================================
# Test 23: Three yields with intermediate calculations
# =============================================================================

def three_yields_gen(n: int):
    i: int = 0
    while i < n:
        yield i
        x: int = i * 2
        yield x
        y: int = i + 10
        yield y
        i = i + 1

def test_three_yields():
    g = three_yields_gen(2)

    # Iteration 1: i=0
    v1: int = next(g)
    assert v1 == 0, "yield i: 0"
    v2: int = next(g)
    assert v2 == 0, "yield x: i*2=0"
    v3: int = next(g)
    assert v3 == 10, "yield y: i+10=10"

    # Iteration 2: i=1
    v4: int = next(g)
    assert v4 == 1, "yield i: 1"
    v5: int = next(g)
    assert v5 == 2, "yield x: i*2=2"
    v6: int = next(g)
    assert v6 == 11, "yield y: i+10=11"

    print("test_three_yields passed")

test_three_yields()

# =============================================================================
# Test 24: Multiple yields with for loop iteration
# =============================================================================

def test_multi_yield_iteration():
    result: list[int] = []
    for val in two_yields_gen():
        result.append(val)

    assert len(result) == 6, "should have 6 values"
    assert result[0] == 0, "first: 0"
    assert result[1] == 0, "second: 0"
    assert result[2] == 1, "third: 1"
    assert result[3] == 2, "fourth: 2"
    assert result[4] == 2, "fifth: 2"
    assert result[5] == 4, "sixth: 4"

    print("test_multi_yield_iteration passed")

test_multi_yield_iteration()

# =============================================================================
# Test 25: Multiple yields with simple expressions
# =============================================================================

def multi_yield_expressions():
    i: int = 1
    while i < 4:
        yield i
        x: int = i + 1
        yield x
        y: int = i * 2
        yield y
        i = i + 1

def test_multi_yield_expressions():
    g = multi_yield_expressions()

    # i=1
    assert next(g) == 1, "i"
    assert next(g) == 2, "x=i+1"
    assert next(g) == 2, "y=i*2"

    # i=2
    assert next(g) == 2, "i"
    assert next(g) == 3, "x=i+1"
    assert next(g) == 4, "y=i*2"

    # i=3
    assert next(g) == 3, "i"
    assert next(g) == 4, "x=i+1"
    assert next(g) == 6, "y=i*2"

    print("test_multi_yield_expressions passed")

test_multi_yield_expressions()

# =============================================================================
# Test 26: yield from generator
# =============================================================================

def yf_inner():
    yield 1
    yield 2
    yield 3

def yf_outer():
    yield from yf_inner()
    yield 4

def test_yield_from_generator():
    yf_total: int = 0
    for x in yf_outer():
        yf_total = yf_total + x
    # 1 + 2 + 3 + 4 = 10
    assert yf_total == 10, f"Expected 10, got {yf_total}"
    print("test_yield_from_generator passed")

test_yield_from_generator()

# =============================================================================
# Test 27: yield from list
# =============================================================================

def yf_list_gen():
    yield from [10, 20, 30]

def test_yield_from_list():
    yf_result: list[int] = []
    for x in yf_list_gen():
        yf_result.append(x)
    assert yf_result == [10, 20, 30], f"Expected [10, 20, 30], got {yf_result}"
    print("test_yield_from_list passed")

test_yield_from_list()

# =============================================================================
# Test 28: yield from with next()
# =============================================================================

def yf_next_gen():
    yield from [1, 2, 3]

def test_yield_from_next():
    g = yf_next_gen()
    yf_v1: int = next(g)
    assert yf_v1 == 1, f"Expected 1, got {yf_v1}"
    yf_v2: int = next(g)
    assert yf_v2 == 2, f"Expected 2, got {yf_v2}"
    yf_v3: int = next(g)
    assert yf_v3 == 3, f"Expected 3, got {yf_v3}"
    print("test_yield_from_next passed")

test_yield_from_next()

# =============================================================================
# Test 29: yield from with multiple trailing yields
# =============================================================================

def yf_multi_trailing():
    yield from [1, 2]
    yield 10
    yield 20

def test_yield_from_multi_trailing():
    yf_mt_result: list[int] = []
    for x in yf_multi_trailing():
        yf_mt_result.append(x)
    assert yf_mt_result == [1, 2, 10, 20], f"Expected [1, 2, 10, 20], got {yf_mt_result}"
    print("test_yield_from_multi_trailing passed")

test_yield_from_multi_trailing()

# =============================================================================
# Test 30: yield from generator with trailing yields
# =============================================================================

def yf_gen_trailing():
    yield from yf_inner()
    yield 100
    yield 200

def test_yield_from_gen_trailing():
    yf_gt_result: list[int] = []
    for x in yf_gen_trailing():
        yf_gt_result.append(x)
    assert yf_gt_result == [1, 2, 3, 100, 200], f"Expected [1, 2, 3, 100, 200], got {yf_gt_result}"
    print("test_yield_from_gen_trailing passed")

test_yield_from_gen_trailing()

# =============================================================================
# Test 31: yield from with partial next() then for loop
# =============================================================================

def yf_partial_gen():
    yield from [5, 10, 15, 20]

def test_yield_from_partial():
    g = yf_partial_gen()
    yf_p1: int = next(g)
    assert yf_p1 == 5, f"Expected 5, got {yf_p1}"

    yf_rest: list[int] = []
    for x in g:
        yf_rest.append(x)
    assert yf_rest == [10, 15, 20], f"Expected [10, 15, 20], got {yf_rest}"
    print("test_yield_from_partial passed")

test_yield_from_partial()

# =============================================================================
# All tests passed
# =============================================================================

print("All generator tests passed!")
