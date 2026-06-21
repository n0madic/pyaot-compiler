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
# Test 12b: send() into a while-loop generator preserves the sent value
# across the loop back-edge (regression: the value used to "stick" after
# the first resume because the while-loop resumer linearized through the
# generic path, which models no back-edge).
# =============================================================================

def while_echo_gen():
    r = yield 0          # pre-loop yield; r receives the first sent value
    while True:
        r = yield r      # echo: yield current r, then r = next sent value

def test_while_loop_send_echo():
    g = while_echo_gen()
    v0: int = next(g)
    assert v0 == 0, "initial yield is 0"
    v1: int = g.send(10)
    assert v1 == 10, "echo 10 across the loop back-edge"
    v2: int = g.send(20)
    assert v2 == 20, "echo 20 (back-edge re-entry)"
    v3: int = g.send(30)
    assert v3 == 30, "echo 30"
    print("test_while_loop_send_echo passed")

test_while_loop_send_echo()

# =============================================================================
# Test 12c: send() accumulator — the sent value is used in raw arithmetic
# (`i += 1`) inside the loop. Exercises gen-local re-typing so the tagged
# sent value unboxes to a raw Int before the BinOp.
# =============================================================================

def while_acc_gen():
    i = yield 0          # i receives the sent value
    while i < 100:
        x = yield i      # yield current i; x receives the next sent value
        i += 1           # raw arithmetic on the sent value

def test_while_loop_send_accumulator():
    g = while_acc_gen()
    a0: int = next(g)
    assert a0 == 0, "initial yield is 0"
    a1: int = g.send(5)
    assert a1 == 5, "i bound to sent 5, yields 5"
    a2: int = g.send(0)
    assert a2 == 6, "i += 1 after resume -> 6"
    a3: int = g.send(0)
    assert a3 == 7, "i += 1 again -> 7"
    a4: int = g.send(0)
    assert a4 == 8, "i += 1 again -> 8"
    print("test_while_loop_send_accumulator passed")

test_while_loop_send_accumulator()

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
# Test 15b: Exhausted while-loop generator raises StopIteration via next()/send()
# Regression: explicit next()/send() on a just-exhausted while-loop generator
# must raise StopIteration, not silently return 0. The `exhausted` flag — not
# the 0 return value — is the discriminator (a legit `yield 0` keeps it false).
# =============================================================================

def test_while_gen_exhaust_next_raises():
    gen_next = range_gen(0, 3)
    assert next(gen_next) == 0, "next #1 (first value is 0)"
    assert next(gen_next) == 1, "next #2"
    assert next(gen_next) == 2, "next #3"
    next_raised: bool = False
    try:
        next(gen_next)
    except StopIteration:
        next_raised = True
    assert next_raised, "next() on exhausted while-gen must raise StopIteration"
    print("test_while_gen_exhaust_next_raises passed")

test_while_gen_exhaust_next_raises()

def test_while_gen_exhaust_send_raises():
    gen_send = range_gen(0, 2)
    assert gen_send.send(None) == 0, "send #1 (first value is 0)"
    assert gen_send.send(None) == 1, "send #2"
    send_raised: bool = False
    try:
        gen_send.send(None)
    except StopIteration:
        send_raised = True
    assert send_raised, "send() on exhausted while-gen must raise StopIteration"
    print("test_while_gen_exhaust_send_raises passed")

test_while_gen_exhaust_send_raises()

def test_while_gen_yield_zero_not_swallowed():
    # Control case: a legit value of 0 must be returned, not mistaken for
    # exhaustion. range_gen(0, 1) yields 0, then the while condition fails.
    gen_zero = range_gen(0, 1)
    assert next(gen_zero) == 0, "yield 0 must be returned, not swallowed"
    zero_raised: bool = False
    try:
        next(gen_zero)
    except StopIteration:
        zero_raised = True
    assert zero_raised, "exhaustion after yield 0 must raise StopIteration"
    print("test_while_gen_yield_zero_not_swallowed passed")

test_while_gen_yield_zero_not_swallowed()

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
# Yield with ternary expression
# =============================================================================

# Ternary where then branch is taken
def gen_ternary_then():
    x: int = 5
    y: int = 3
    yield x if x > y else y

gt = gen_ternary_then()
assert next(gt) == 5, "ternary then branch failed"

# Ternary where else branch is taken
def gen_ternary_else():
    x: int = 2
    y: int = 7
    yield x if x > y else y

ge = gen_ternary_else()
assert next(ge) == 7, "ternary else branch failed"

# Multiple yields with ternary
def gen_ternary_multi():
    a: int = 10
    b: int = 3
    yield a if a > b else b
    yield b if b > a else a

gm = gen_ternary_multi()
assert next(gm) == 10, "multi ternary first yield failed"
assert next(gm) == 10, "multi ternary second yield failed"

print("Yield with ternary expression tests passed!")

# =============================================================================
# Truthiness: not operator in generator expressions
# =============================================================================

# Test: not operator in generator yield expression
def test_gen_not_yield():
    g = (not x for x in range_gen(0, 4))
    v0: int = next(g)
    v1: int = next(g)
    v2: int = next(g)
    v3: int = next(g)
    assert v0 == 1, "not 0 should be 1 (True)"
    assert v1 == 0, "not 1 should be 0 (False)"
    assert v2 == 0, "not 2 should be 0 (False)"
    assert v3 == 0, "not 3 should be 0 (False)"
    print("test_gen_not_yield passed")

test_gen_not_yield()

# Test: not operator in generator expression filter (not x == 0 filters zeros)
def test_gen_filter_not_eq():
    g = (x for x in range_gen(0, 6) if not x == 0)
    v1: int = next(g)
    assert v1 == 1, "filter not eq first"
    v2: int = next(g)
    assert v2 == 2, "filter not eq second"
    v3: int = next(g)
    assert v3 == 3, "filter not eq third"
    v4: int = next(g)
    assert v4 == 4, "filter not eq fourth"
    v5: int = next(g)
    assert v5 == 5, "filter not eq fifth"
    print("test_gen_filter_not_eq passed")

test_gen_filter_not_eq()

print("Generator truthiness tests passed!")

# =============================================================================
# All tests passed
# =============================================================================

# ===== Whole-project code-review regression: send(None) priming + chained
# generators (formerly test_review_wave1.py) =====
import itertools


def _rv_echo():
    received = yield 0
    yield received
    yield received


def _rv_gen_a():
    yield 1
    yield 2
    yield 3


def _rv_gen_b():
    yield 4
    yield 5


def _rv_gen_send_none() -> None:
    g = _rv_echo()
    print(g.send(None))  # priming must not raise TypeError
    print(g.send(10))


def _rv_gen_chain() -> None:
    out: list[int] = []
    for v in itertools.chain(_rv_gen_a(), _rv_gen_b()):
        out.append(v)
    print(out)


_rv_gen_send_none()
_rv_gen_chain()


# =============================================================================
# Folded from p6_generators.py (generator functions, yield, yield from,
# next()/list()/sum() drives, multi-arg parameterized generators)
# Generator defs are hoisted to module level (nested generator defs inside a
# function trigger a frontend closure-conversion panic in this subset).
# =============================================================================

# a simple counting generator driven by a for-loop
def _fg_count_up(n):
    i = 0
    while i < n:
        yield i
        i = i + 1


# a generator transforming an input iterable
def _fg_squares(xs):
    for v in xs:
        yield v * v


# a generator with multiple distinct yield points
def _fg_three():
    yield "a"
    yield "b"
    yield "c"


# yield from delegates to a sub-iterable (and to a generator over an iterable)
def _fg_chained():
    yield 0
    yield from [1, 2, 3]
    yield from _fg_squares([2, 3])
    yield 99


# materializing a generator with list()
def _fg_evens(limit):
    i = 0
    while i < limit:
        if i % 2 == 0:
            yield i
        i = i + 1


# next() drives a generator explicitly
def _fg_naturals():
    i = 1
    while True:
        yield i
        i = i + 1


# sum over a generator
def _fg_first_n(n):
    i = 0
    while i < n:
        yield i + 1
        i = i + 1


# a generator parameterized by several args
def _fg_ramp(start, stop, step):
    cur = start
    while cur < stop:
        yield cur
        cur = cur + step


def _fold_p6_generators() -> None:
    counted: list[int] = []
    for x in _fg_count_up(5):
        counted.append(x)
    assert counted == [0, 1, 2, 3, 4], "count_up(5)"

    sq: list[int] = []
    for s in _fg_squares([1, 2, 3, 4]):
        sq.append(s)
    assert sq == [1, 4, 9, 16], "squares of 1..4"

    letters: list[str] = []
    for t in _fg_three():
        letters.append(t)
    assert letters == ["a", "b", "c"], "three distinct yields"

    chain_out: list[int] = []
    for v in _fg_chained():
        chain_out.append(v)
    assert chain_out == [0, 1, 2, 3, 4, 9, 99], "yield from list + generator"

    assert list(_fg_evens(10)) == [0, 2, 4, 6, 8], "list(evens(10))"

    nat = _fg_naturals()
    n1: int = next(nat)
    n2: int = next(nat)
    n3: int = next(nat)
    assert n1 == 1, "naturals next 1"
    assert n2 == 2, "naturals next 2"
    assert n3 == 3, "naturals next 3"

    assert sum(_fg_first_n(5)) == 15, "sum(first_n(5))"

    ramp_out: list[int] = []
    for v in _fg_ramp(0, 10, 3):
        ramp_out.append(v)
    assert ramp_out == [0, 3, 6, 9], "ramp(0, 10, 3)"

    print("_fold_p6_generators passed")


_fold_p6_generators()


# =============================================================================
# Folded from p6_send_close.py (gen.send / gen.close, bare yield coroutines).
# The originals printed running state from inside the generator; here each
# generator yields its accumulated state list so the driver can assert it.
# =============================================================================

# a coroutine-style generator receiving values via send()
def _fg_accumulator():
    total = 0
    totals: list[int] = []
    while True:
        x = yield totals
        total = total + x
        totals.append(total)


# send() that both receives and the body reacts (echo each value twice)
def _fg_echo_twice():
    log: list[str] = []
    while True:
        x = yield log
        log.append("once " + x)
        log.append("twice " + x)


# close() ends a generator early
def _fg_ticking():
    n = 0
    while True:
        n = n + 1
        yield n


def _fold_p6_send_close() -> None:
    acc = _fg_accumulator()
    next(acc)  # prime to the first yield
    acc.send(10)
    acc.send(5)
    seen = acc.send(100)
    assert seen == [10, 15, 115], "accumulator running totals"
    acc.close()

    e = _fg_echo_twice()
    next(e)
    e.send("hi")
    echoed = e.send("yo")
    assert echoed == [
        "once hi",
        "twice hi",
        "once yo",
        "twice yo",
    ], "echo_twice log"
    e.close()

    t = _fg_ticking()
    tick1: int = next(t)
    tick2: int = next(t)
    assert tick1 == 1, "ticking first"
    assert tick2 == 2, "ticking second"
    t.close()

    print("_fold_p6_send_close passed")


_fold_p6_send_close()


# =============================================================================
# Folded from p6_genexpr.py (generator expressions: filter, nested, over a
# string, sum/list/next drives)
# =============================================================================

def _fold_p6_genexpr() -> None:
    xs = [1, 2, 3, 4, 5]

    # a basic generator expression consumed by a for-loop
    doubled: list[int] = []
    for v in (x * 2 for x in xs):
        doubled.append(v)
    assert doubled == [2, 4, 6, 8, 10], "x * 2 genexpr"

    # genexpr materialized with list()
    assert list(n * n for n in xs) == [1, 4, 9, 16, 25], "list(n * n genexpr)"

    # genexpr with a filter
    assert list(x for x in xs if x % 2 == 1) == [1, 3, 5], "odd filter genexpr"

    # sum over a genexpr
    assert sum(x for x in range(10)) == 45, "sum over genexpr"

    # a nested genexpr (inner clause over a literal)
    pairs = ((a, b) for a in [1, 2] for b in [10, 20])
    pair_out: list[tuple[int, int]] = []
    for p in pairs:
        pair_out.append(p)
    assert pair_out == [
        (1, 10),
        (1, 20),
        (2, 10),
        (2, 20),
    ], "nested genexpr pairs"

    # genexpr over a string
    assert list(c for c in "abc") == ["a", "b", "c"], "genexpr over string"

    # a genexpr driven by next()
    g = (i + 1 for i in range(3))
    e1: int = next(g)
    e2: int = next(g)
    e3: int = next(g)
    assert e1 == 1, "genexpr next 1"
    assert e2 == 2, "genexpr next 2"
    assert e3 == 3, "genexpr next 3"

    print("_fold_p6_genexpr passed")


_fold_p6_genexpr()


# =============================================================================
# Nested generator defs (capture-free) — a `def` containing `yield` nested
# inside another function. The wrapper crosses the one nested-call ABI.
# =============================================================================

def test_nested_generator():
    # The original crash arity (a single-param nested generator).
    def gen(n):
        i = 0
        while i < n:
            yield i
            i += 1

    assert list(gen(3)) == [0, 1, 2], "nested generator list"

    # The nested generator passed/called as a value.
    g = gen
    assert list(g(2)) == [0, 1], "nested generator as value"

    # A nested generator with multiple params (arity > 1).
    def pairs(a, b):
        yield a
        yield b
        yield a + b

    p = pairs(10, 20)
    assert next(p) == 10, "nested gen multi-param 1"
    assert next(p) == 20, "nested gen multi-param 2"
    assert next(p) == 30, "nested gen multi-param 3"

    # Driving a nested generator with a for-loop.
    total = 0
    for v in gen(5):
        total += v
    assert total == 10, "nested generator for-loop"

    print("test_nested_generator passed")


test_nested_generator()

# =============================================================================
# yield inside try / with (Phase 6E): table-based unwinding makes a suspended
# frame safe inside a protected region. The try/with body and `else` clause
# support yield; a yield in an `except`/`finally` body is a clean compile error.
# Coverage: try/except (exception after resume is caught + normal flow),
# try/finally (exhaust + close()), with (enter/exit ordering vs next()/close()),
# yield in a try nested in a loop, `x = yield` (send) in a try, `yield from` in
# a try. Only STARTED generators are closed (no GC-triggered cleanup is relied
# on), and `__exit__` args / tracebacks are not asserted (documented boundaries).
# =============================================================================

# try/except: an exception raised after resume is caught by the same handler;
# the normal flow just yields.
def _tg_try_except(do_raise):
    try:
        x = yield 1
        if do_raise:
            raise ValueError("boom")
        yield 2
    except ValueError:
        print("caught")


def test_yield_try_except():
    g = _tg_try_except(False)
    assert next(g) == 1, "first yield"
    assert g.send(0) == 2, "second yield"
    stopped: bool = False
    try:
        next(g)
    except StopIteration:
        stopped = True
    assert stopped, "exhausts after the try body"

    g2 = _tg_try_except(True)
    assert next(g2) == 1, "first yield (raising)"
    stopped2: bool = False
    try:
        g2.send(1)
    except StopIteration:
        stopped2 = True
    assert stopped2, "handler runs then exhausts"
    print("test_yield_try_except passed")


test_yield_try_except()


# try/finally: finally runs at exhaustion AND on close() of a suspended yield.
def _tg_try_finally():
    try:
        yield 1
        yield 2
    finally:
        print("finally ran")


def test_yield_try_finally_exhaust():
    collected: list[int] = []
    for v in _tg_try_finally():
        collected.append(v)
    assert collected == [1, 2], "yields 1, 2"
    print("test_yield_try_finally_exhaust passed")


def test_yield_try_finally_close():
    g = _tg_try_finally()
    assert next(g) == 1, "first yield"
    g.close()  # finally must run during close
    print("test_yield_try_finally_close passed")


test_yield_try_finally_exhaust()
test_yield_try_finally_close()


# with: __enter__ on first next(), __exit__ on normal exit AND on close().
class _TgCM:
    def __init__(self, tag: str):
        self.tag = tag

    def __enter__(self):
        print("enter " + self.tag)
        return self.tag

    def __exit__(self, et, ev, tb):
        print("exit " + self.tag)
        return False


def _tg_with():
    with _TgCM("A") as c:
        yield c
        yield c + "2"


def test_yield_with_exhaust():
    out: list[str] = []
    for v in _tg_with():
        out.append(v)
    assert out == ["A", "A2"], "with yields"
    print("test_yield_with_exhaust passed")


def test_yield_with_close():
    g = _tg_with()
    assert next(g) == "A", "enter then first yield"
    g.close()  # __exit__ must run on close
    print("test_yield_with_close passed")


test_yield_with_exhaust()
test_yield_with_close()


# yield in a try nested inside a for-loop; the post-resume raise is handled.
def _tg_loop_try():
    for i in range(3):
        try:
            yield i
            if i == 1:
                raise ValueError("at one")
        except ValueError:
            print("handled " + str(i))


def test_yield_try_in_loop():
    out: list[int] = []
    for v in _tg_loop_try():
        out.append(v)
    assert out == [0, 1, 2], "loop yields"
    print("test_yield_try_in_loop passed")


test_yield_try_in_loop()


# x = yield (send) inside a try; the finally runs at exhaustion.
def _tg_send_in_try():
    try:
        a = yield 10
        b = yield a + 1
        yield b + 1
    finally:
        print("send finally")


def test_send_in_try():
    g = _tg_send_in_try()
    assert next(g) == 10, "first"
    assert g.send(100) == 101, "a + 1"
    assert g.send(200) == 201, "b + 1"
    stopped: bool = False
    try:
        g.send(0)
    except StopIteration:
        stopped = True
    assert stopped, "exhausts"
    print("test_send_in_try passed")


test_send_in_try()


# yield from inside a try body.
def _tg_yieldfrom_in_try():
    try:
        yield from [1, 2, 3]
        yield 4
    except RuntimeError:
        print("rt")


def test_yieldfrom_in_try():
    out: list[int] = []
    for v in _tg_yieldfrom_in_try():
        out.append(v)
    assert out == [1, 2, 3, 4], "yield from in try"
    print("test_yieldfrom_in_try passed")


test_yieldfrom_in_try()

print("All generator tests passed!")
