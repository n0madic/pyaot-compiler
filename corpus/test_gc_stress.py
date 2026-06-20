# Consolidated GC-stress soak.
#
# This file merges several per-phase GC soak programs into one. Each deliberately
# allocates heap objects in loops and across call / exception / generator
# boundaries to surface use-after-free when the runtime is built with
# `--cfg gc_stress_test` (collect-on-every-alloc):
#
#   RUSTFLAGS="--cfg gc_stress_test" cargo build -p pyaot-runtime
#
# The point is the ALLOCATION PATTERN — loops that build lists / dicts / tuples /
# instances, re-box bignums, drive closures / generators, and raise / catch across
# call frames while heap roots must stay live across the intervening collections.
# Do not let any rewrite optimize those allocations away.
#
# Each source's body is wrapped in `def _<sourcename>():` and called: wrapping
# creates roots then drops them on return, which itself exercises the collector.
# The printed regression values (verified byte-exact against CPython) are pinned
# as asserts so the file stays silent except for the final summary line; using a
# value in an assert keeps it live across the surrounding allocations exactly as
# the original `print` did.
#
# Consolidated from: p2_gc_stress, p4_gc_stress, p5_gc_stress, p6_gc_stress,
# p7_gc_stress, p9_root_narrowing_gc_stress, p9_zip_fresh_elems_gc_stress,
# test_gc_simple.

from typing import Callable


# ── p2_gc_stress: heap-string + bignum roots live across a float/bignum loop ──
#
# Two heap roots must stay live across an allocating loop, or the collector frees
# them: `s` (a heap string) and `bacc` (a bignum accumulator re-boxed every
# iteration). The float loop compiles to unboxed `fadd` (no GC traffic) and must
# still match CPython byte-for-byte.
def _p2_gc_stress():
    s = "survivor string that must not be freed"
    facc = 0.0
    for i in range(300000):
        facc = facc + 1.5
    bacc = 0
    for j in range(100000):
        bacc = bacc + 2 ** 64
    assert s == "survivor string that must not be freed"
    assert facc == 450000.0
    assert bacc == 1844674407370955161600000
    assert len(s) == 38


# ── p4_gc_stress: container / iterator / element roots simultaneously live ──
#
# The root set is derived purely from each local's `Repr::is_gc_root()`; keep
# container / iterator / element locals simultaneously live across allocating
# build and iteration loops, forcing many collections in between.
def _p4_gc_stress():
    # A survivor string and a survivor list that must outlive every loop below.
    survivor = "the survivor string that must never be freed"
    keep = [1, 2, 3, 4, 5]

    # Build a list by repeated concatenation: each iteration allocates a fresh
    # list while `survivor` / `keep` stay live across the allocation.
    nums = []
    for i in range(4000):
        nums = nums + [i]
    assert len(nums) == 4000
    assert nums[0] == 0
    assert nums[3999] == 3999

    # Build a dict in a loop (each insert may rehash/allocate) with bignum-ish
    # values (`i * i * i`) that promote to heap integers — roots created mid-loop.
    squares = {}
    for j in range(2000):
        squares[j] = j * j * j
    assert len(squares) == 2000
    assert squares[1999] == 7988005999

    # Iterate one live container while building another: the iterator local, the
    # per-iteration element, the growing result, AND `survivor` are all live
    # across the allocating `+`.
    doubled = []
    for n in nums:
        doubled = doubled + [n * 2]
    assert len(doubled) == 4000
    assert doubled[100] == 200

    # A bignum accumulator re-boxed every iteration while iterating a live
    # container.
    big = 0
    for n in nums:
        big = big + n * 1000000000000000000
    assert big == 7998000000000000000000000

    # Comprehensions allocate a result list plus the per-element heap values, all
    # live across the build, while `survivor` / `nums` stay rooted.
    comp = [[k, k + 1] for k in range(3000)]
    assert len(comp) == 3000
    assert comp[2999] == [2999, 3000]

    # An iteration-builtin pipeline: sorted(...) materializes a list, sum reduces
    # a comprehension, all while the survivors persist.
    assert sum([x for x in range(5000) if x % 7 == 0]) == 1786785
    assert len(sorted([n % 100 for n in nums])) == 4000

    # The survivors are still intact after all that allocation.
    assert survivor == "the survivor string that must never be freed"
    assert len(survivor) == 44
    assert keep == [1, 2, 3, 4, 5]
    assert keep[2] == 3


# ── p5_gc_stress: class-instance graph live across allocating calls ──
#
# Instance fields are uniform tagged `Value` slots traced via `Value::is_ptr()`.
# Keep a class-instance graph (instances holding instances in Tagged fields) live
# across thousands of allocating calls, forcing many collections in between.
class _P5Inner:
    def __init__(self, n: int):
        self.n = n

    def get(self) -> int:
        return self.n


class _P5Outer:
    def __init__(self, inner: "_P5Inner", tag: int):
        self.inner = inner
        self.tag = tag

    def total(self) -> int:
        # Reaches the inner instance through a Tagged field — it must be alive.
        return self.inner.get() + self.tag


def _p5_gc_stress():
    # A survivor instance graph that must outlive every loop below.
    survivor = _P5Outer(_P5Inner(7), 100)

    # Build a list of 5000 Outer instances; each holds an Inner reachable ONLY
    # through its Tagged `inner` field. Every Inner()/Outer()/append allocates,
    # forcing GC while `outers` and `survivor` stay rooted.
    outers: list[_P5Outer] = []
    for i in range(5000):
        inner = _P5Inner(i)
        outer = _P5Outer(inner, i * 2)
        outers.append(outer)

    # Traverse: each `o.total()` dereferences the Tagged `inner` field of an
    # instance that has survived thousands of intervening collections.
    s = 0
    for o in outers:
        s = s + o.total()
    assert len(outers) == 5000
    assert s == 37492500

    # A bignum accumulator re-boxed every iteration while the graph is live.
    big = 0
    for o in outers:
        big = big + o.inner.get() * 1000000000000000000
    assert big == 12497500000000000000000000

    # Mutate fields in a loop (each iteration allocates a fresh Inner that
    # replaces the old one — the replaced instances become garbage mid-loop).
    for o in outers:
        o.inner = _P5Inner(o.tag)
    acc = 0
    for o in outers:
        acc = acc + o.inner.get()
    assert acc == 24995000

    # The survivor graph is still intact after all that allocation.
    assert survivor.total() == 107
    assert survivor.inner.get() == 7


# ── p6_gc_stress: closures (env tuples), cells, and generators ──
#
# Soak the collector with closures (env tuples), cells (shared mutable state),
# and generators (tagged slot arrays) all live across many allocations.
def _p6_make_counter(start: int) -> Callable[[], int]:
    n = start

    def step() -> int:
        nonlocal n
        n = n + 1
        return n

    return step


def _p6_windows(data: list, size: int):
    i = 0
    while i + size <= len(data):
        acc = 0
        j = 0
        while j < size:
            acc = acc + data[i + j]
            j = j + 1
        yield acc
        i = i + 1


def _p6_make_pair() -> Callable[[int], int]:
    state = 0

    def bump(d: int) -> int:
        nonlocal state
        state = state + d
        return state

    return bump


def _p6_value_stream(values: list):
    for v in values:
        yield v * 2
        yield v + 1


def _p6_build_values(count: int) -> list:
    fns: list[Callable[[], int]] = []
    for k in range(count):
        fns.append(_p6_make_counter(k))
    out: list[int] = []
    for f in fns:
        out.append(f())
    return out


def _p6_gc_stress():
    # many independent counter closures, each holding a live cell
    counters: list[Callable[[], int]] = []
    for i in range(200):
        counters.append(_p6_make_counter(i * 1000))

    # Drive them repeatedly, allocating lists each round to force collections.
    checksum = 0
    for _ in range(50):
        junk: list[int] = []
        for c in counters:
            junk.append(c())
        checksum = checksum + junk[0] + junk[len(junk) - 1]
    assert checksum == 9952550

    # generators holding live locals across many resumes
    data: list[int] = []
    for i in range(500):
        data.append(i)

    total = 0
    for w in _p6_windows(data, 4):
        total = total + w
        # allocate garbage every iteration
        tmp: list[int] = [w, w, w]
        total = total + tmp[0]
    assert total == 992012

    # closures capturing a shared cell, mutated through a generator drive
    bumpers: list[Callable[[int], int]] = []
    for i in range(100):
        bumpers.append(_p6_make_pair())

    acc = 0
    for r in range(30):
        box: list[int] = []
        for b in bumpers:
            box.append(b(r))
        acc = acc + box[0]
    assert acc == 4495

    # closures feeding a generator: build many counters, drive them, then a
    # generator streams the pre-computed results.
    stream_total = 0
    for v in _p6_value_stream(_p6_build_values(300)):
        stream_total = stream_total + v
        spill: list[int] = [v, v + 1]
        stream_total = stream_total + spill[0]
    assert stream_total == 271500


# ── p7_gc_stress: allocation-heavy raise/catch across calls ──
#
# Exercises shadow-stack unwinding on longjmp and rootedness of `as e` bindings
# and live containers across try frames.
class _P7WorkError(Exception):
    pass


def _p7_build(n: int) -> list[str]:
    out: list[str] = []
    for i in range(n):
        out.append("item-" + str(i))
    return out


def _p7_risky(n: int) -> list[str]:
    data = _p7_build(n)
    if n % 3 == 0:
        raise _P7WorkError("mod3:" + str(n))
    return data


def _p7_worker(n: int) -> int:
    # Allocate before, inside, and after the try; raise across a call.
    pre = _p7_build(8)
    total = 0
    try:
        data = _p7_risky(n)
        total = total + len(data)
    except _P7WorkError as e:
        msg = str(e)
        if len(msg) > 0:
            total = total - 1
    total = total + len(pre)
    return total


def _p7_nested(depth: int) -> str:
    junk = _p7_build(depth + 2)
    try:
        try:
            if depth > 0:
                raise ValueError("d" + str(depth))
            return "leaf:" + str(len(junk))
        except ValueError as inner:
            keep = _p7_build(6)
            raise _P7WorkError(str(inner) + ":" + str(len(keep)))
    except _P7WorkError as outer:
        return str(outer)


def _p7_churn_in_handler() -> int:
    try:
        raise _P7WorkError("rooted-message-stays")
    except _P7WorkError as e:
        total = 0
        for i in range(40):
            tmp = _p7_build(5)
            total = total + len(tmp)
        return total + len(str(e))


def _p7_gc_stress():
    grand = 0
    for round_i in range(60):
        grand = grand + _p7_worker(round_i % 7)
    assert grand == 553

    # nested try frames under allocation pressure
    acc: list[str] = []
    for d in range(20):
        acc.append(_p7_nested(d % 4))
    assert len(acc) == 20
    assert acc[0] == "leaf:2"
    assert acc[1] == "d1:6"
    assert acc[5] == "d1:6"

    # raise/catch in a tight loop with live containers across frames
    state: dict[str, int] = {}
    for i in range(120):
        key = "k" + str(i % 10)
        try:
            if i % 2 == 0:
                raise _P7WorkError(key)
            state[key] = i
        except _P7WorkError as e:
            state[str(e)] = i * 2
    assert len(state) == 10
    assert state["k0"] == 220
    assert state["k9"] == 119

    # exception value stays rooted while allocating in the handler
    assert _p7_churn_in_handler() == 220


# ── p9_root_narrowing_gc_stress: liveness-based root-set narrowing ──
#
# Exercises every shape the narrowing must keep sound. The function-internal
# `assert`s on `early` / `survivor` / `label` act as the original liveness probes
# (a use of the value across the surrounding allocations).
def _p9rn_churn(n):
    early = "consumed-before-allocs"
    # last use of `early` before any of the loop's allocations
    assert early == "consumed-before-allocs"
    survivor = "lives-across-" + str(n)
    parts = []
    for i in range(n):
        parts.append("chunk" + str(i))  # allocates every iteration
    assert survivor == "lives-across-5"
    return len(parts)


def _p9rn_make_str(i):
    return "made-" + str(i)


def _p9rn_append_made(n):
    xs = []
    for i in range(n):
        xs.append(_p9rn_make_str(i))  # temp used BY the allocating append
    return xs


def _p9rn_try_with_pre_value(flag):
    pre = "pre-try-" + str(flag)
    try:
        noise = []
        for i in range(6):
            noise.append("alloc" + str(i))
        if flag:
            raise ValueError("boom")
        return "no-raise"
    except ValueError:
        return pre  # `pre` must have survived the longjmp


def _p9rn_gen_strs(n):
    prefix = "gen-"
    for i in range(n):
        yield prefix + str(i)


def _p9rn_bignum_with_live_str():
    label = "bignum-label"
    big = 2 ** 100
    big = big + 1  # tagged add on a heap BigInt — allocates
    assert label == "bignum-label"
    return big


def _p9_root_narrowing_gc_stress():
    assert _p9rn_churn(5) == 5
    assert _p9rn_append_made(4) == ["made-0", "made-1", "made-2", "made-3"]
    assert _p9rn_try_with_pre_value(True) == "pre-try-True"
    assert _p9rn_try_with_pre_value(False) == "no-raise"
    expected = ["gen-0", "gen-1", "gen-2"]
    idx = 0
    for s in _p9rn_gen_strs(3):
        assert s == expected[idx]
        idx = idx + 1
    assert idx == 3
    assert _p9rn_bignum_with_live_str() == 1267650600228229401496703205377


# ── p9_zip_fresh_elems_gc_stress: zip over FRESH-element sources ──
#
# A string / enumerate / generator source allocates each element inside the inner
# next(); the zip nexts must root already-obtained items across the remaining
# inner nexts and the result-tuple allocation.
def _p9zip_gen_words():
    for i in range(3):
        yield "w" + str(i)


def _p9_zip_fresh_elems_gc_stress():
    # zip(2) over two strings: both items are fresh StrObj chars
    expected_ab = [("a", "x"), ("b", "y"), ("c", "z")]
    idx = 0
    for a, b in zip("abc", "xyz"):
        assert a == expected_ab[idx][0]
        assert b == expected_ab[idx][1]
        idx = idx + 1
    assert idx == 3

    # zip(2): fresh item1 (string) against an allocating second source
    expected_cw = [("p", "w0"), ("q", "w1"), ("r", "w2")]
    idx = 0
    for c, w in zip("pqr", _p9zip_gen_words()):
        assert c == expected_cw[idx][0]
        assert w == expected_cw[idx][1]
        idx = idx + 1
    assert idx == 3

    # zip over enumerate (its elements are fresh tuples)
    expected_en = [(0, "m", "u"), (1, "n", "v")]
    idx = 0
    for pair, ch in zip(enumerate("mn"), "uv"):
        assert pair[0] == expected_en[idx][0]
        assert pair[1] == expected_en[idx][1]
        assert ch == expected_en[idx][2]
        idx = idx + 1
    assert idx == 2

    # tuple()/list() drain a zip of fresh elements through one more alloc
    assert list(zip("ab", "cd")) == [("a", "c"), ("b", "d")]
    assert tuple(zip("gh", "ij")) == (("g", "i"), ("h", "j"))


# ── test_gc_simple: GC prologue/epilogue generation for heap types ──
#
# Functions with str parameters generate GC code; primitive-only params have no
# GC overhead. Verifies the prologue/epilogue plumbing end-to-end.
def _gcs_use_string(s: str) -> int:
    # s is a str parameter - marked as GC root
    return 42


def _gcs_nested_strings(a: str, b: str) -> int:
    # Multiple GC roots: roots[0] = a, roots[1] = b
    return 100


def _gcs_mixed_params(x: int, s: str, y: int) -> int:
    # Only s is GC root, x and y are primitives
    return x + y


def _gcs_pure_int(x: int, y: int) -> int:
    # No GC code generated - just 'add' + 'ret'
    return x + y


def _gcs_factorial(n: int) -> int:
    if n <= 1:
        return 1
    return n * _gcs_factorial(n - 1)


def _gcs_void_function() -> None:
    x: int = 10
    assert x == 10, "x should equal 10"


def _gcs_void_with_str_param(s: str) -> None:
    # GC root for s, but returns None
    assert True, "True should be True"


def _test_gc_simple():
    r1: int = _gcs_use_string("hello")
    assert r1 == 42, "r1 should equal 42"

    r2: int = _gcs_nested_strings("foo", "bar")
    assert r2 == 100, "r2 should equal 100"

    r3: int = _gcs_mixed_params(10, "test", 20)
    assert r3 == 30, "r3 should equal 30"

    r4: int = _gcs_pure_int(5, 7)
    assert r4 == 12, "r4 should equal 12"

    r5: int = _gcs_factorial(5)
    assert r5 == 120, "r5 should equal 120"

    _gcs_void_function()
    _gcs_void_with_str_param("test")


_p2_gc_stress()
_p4_gc_stress()
_p5_gc_stress()
_p6_gc_stress()
_p7_gc_stress()
_p9_root_narrowing_gc_stress()
_p9_zip_fresh_elems_gc_stress()
_test_gc_simple()

print("All GC stress tests passed!")
