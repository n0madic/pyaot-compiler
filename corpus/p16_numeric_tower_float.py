# int→float numeric tower through a float slot (PLAN §8).
#
# An int / bool / gradual value flowing into a `float` slot — a `-> float` return
# or an annotated `: float` local — is a REAL coercion: pyaot models `float` as
# `Raw(F64)` and `int` as a tagged fixnum-or-bignum, so int→f64 at the store is a
# genuine conversion (with a bignum arm), never a noop. The coercion lands as a
# CHECKED `Tagged → Raw(F64)` unbox (`rt_unbox_float`), covering int→f64,
# bool→f64 and gradual Dyn→f64 uniformly.
#
# ANNOTATION-AS-CONTRACT DIVERGENCE: CPython ignores `-> float` / `: float` and
# keeps the raw int (so `print(f())` would show `0`); pyaot treats the annotation
# as a contract that coerces to `0.0`. The divergence is observable ONLY via
# repr-print, so every case ASSERTS numerically (`== 0.0`; both sides agree that
# `0 == 0.0`) and PRINTS only float-FORCED results (`x + 0.5`), identical on both.
# Same philosophy as the tuple-slice-slot probe.


# ===== return: int / bool through `-> float` =====
def ret_int_zero() -> float:
    return 0


def ret_int_seven() -> float:
    return 7


def ret_bool(b: bool) -> float:
    return b


assert ret_int_zero() == 0.0
assert ret_int_seven() == 7.0
assert ret_bool(True) == 1.0
assert ret_bool(False) == 0.0
# Float-forced prints: adding a float makes the result a float on BOTH sides.
print(ret_int_zero() + 0.5)    # 0.5
print(ret_int_seven() + 0.5)   # 7.5
print(ret_bool(False) + 0.5)   # 0.5
print(ret_bool(True) + 0.5)    # 1.5


# ===== annotated `: float` local from an int =====
def local_from_int() -> float:
    y: float = 5
    assert y == 5.0
    return y + 0.0


print(local_from_int())        # 5.0


# ===== unannotated mixed return (inferred Dyn) bound to a `: float` local =====
# A function returning both `1.5` and `0` has its inferred return demoted to Dyn
# (`raw_uniform`: the unboxed union stays tagged — CPython-faithful). Binding it
# to a `: float` local is then a gradual Dyn→f64 coercion at the slot.
def mixed(flag: bool):
    if flag:
        return 1.5
    return 0


def use_mixed() -> float:
    a: float = mixed(True)
    b: float = mixed(False)
    assert a == 1.5
    assert b == 0.0
    return a + b


print(use_mixed())             # 1.5


# ===== bignum arm: a heap BigInt through `-> float` =====
# 2 ** 62 is above the 61-bit fixnum range (a heap BigInt) and an exact power of
# two (exactly f64-representable). The `rt_unbox_float` BigInt arm rounds it to
# the nearest f64. Do NOT print it (avoid repr-format dependence) — validate with
# `==` only.
def big_pow() -> float:
    return 2 ** 62


assert big_pow() == 4611686018427387904.0


# ===== interaction: `-> float` int returns feeding `sum` over a float list =====
def one() -> float:
    return 1


def sum_floats() -> float:
    xs = [one(), one(), 0.5]
    return sum(xs)


assert sum_floats() == 2.5
print(sum_floats())            # 2.5


print("Numeric tower (int->float slot) tests passed!")
