# int->float numeric tower at the remaining slot seams (PLAN §8, closed).
#
# The `-> float` return and the annotated `: float` LOCAL already coerced an
# int/bool/gradual value into a `float` slot (corpus/p16). This probe covers the
# seams §8 left rejected and now admits, all reducing to "coerce an int/bool/Dyn
# value into a `float`-typed slot":
#   * a `float` PARAMETER — free function, method (positional + keyword), and
#     constructor — via lowering's `coerce_value` (checked `rt_unbox_float`);
#   * a `float` GLOBAL slot and a `float` FIELD slot — via the store-side
#     `box_float_for_slot` (checked unbox then `BoxFloat` to a real `FloatObj`,
#     so the slot's unchecked read stays sound, PITFALLS A2).
#
# ANNOTATION-AS-CONTRACT DIVERGENCE: CPython ignores the `float` annotation and
# keeps the raw int (a bare `print` would show `5`, not `5.0`); pyaot treats the
# annotation as a contract that coerces. The divergence is observable ONLY via
# repr-print, so every case ASSERTS numerically (`== 5.0`; both agree `5 == 5.0`)
# and PRINTS only float-FORCED results (`x + 0.5`), identical on both runtimes.


# ===== free-fn `float` parameter: int + bool args =====
def poly(a: float) -> float:
    return a * 2.0


assert poly(3) == 6.0          # int arg -> 3.0
assert poly(True) == 2.0       # bool arg -> 1.0
assert poly(False) == 0.0      # bool arg -> 0.0
print(poly(3) + 0.5)           # 6.5
print(poly(True))              # 2.0  (body multiplies by a float literal)


# ===== method `float` parameter: positional + keyword =====
class Scaler:
    base: float

    def __init__(self, base: float) -> None:
        # ctor `float` param fed a static int (the constructor seam).
        self.base = base

    def scaled(self, factor: float) -> float:
        return self.base * factor


s = Scaler(2)                  # ctor float param: int 2 -> 2.0
assert s.base == 2.0
assert s.scaled(3) == 6.0      # positional int -> float param
assert s.scaled(factor=4) == 8.0   # keyword int -> float param
print(s.scaled(3) + 0.5)       # 6.5
print(s.scaled(factor=4) + 0.5)    # 8.5


# ===== `float` FIELD written from an int (the store-side SetField box) =====
class Counter:
    value: float

    def __init__(self) -> None:
        self.value = 0.0

    def set_from_int(self, n: int) -> None:
        # `n` is statically `int`; the field is `float` -> box_float_for_slot
        # converts and re-boxes to a genuine FloatObj at the store.
        self.value = n


c = Counter()
assert c.value == 0.0
c.set_from_int(5)
assert c.value == 5.0
print(c.value + 0.5)           # 5.5


# ===== `float` GLOBAL written from an int =====
# `g` is read inside `read_g`, so it lowers to a tagged `GlobalSet` slot (not a
# `Raw(F64)` __main__ local) — exactly the deferred-then-closed global seam.
g: float = 0.0


def read_g() -> float:
    return g + 0.0


def set_g(n: int) -> None:
    global g
    g = n                      # int into a float global -> box_float_for_slot


assert read_g() == 0.0
set_g(5)
assert g == 5.0
assert read_g() == 5.0
print(read_g() + 0.5)          # 5.5


# ===== bignum arm: a heap BigInt through a `float` parameter =====
# 2 ** 62 is above the fixnum range (a heap BigInt) and an exact power of two
# (exactly f64-representable), so the `rt_unbox_float` BigInt arm is exact. Do
# NOT print it (avoid repr-format dependence) — validate with `==` only.
def take_float(x: float) -> float:
    return x


assert take_float(2 ** 62) == 4611686018427387904.0


# ===== interaction: int->float global feeding a float-param free function =====
def scale_by_ten(factor: float) -> float:
    return factor * 10.0


set_g(3)                       # g written from int 3 -> 3.0 in the float slot
assert scale_by_ten(g) == 30.0  # the boxed float global flows into a float param
print(scale_by_ten(g))         # 30.0  (body multiplies by a float literal)


print("Numeric tower (param/global/field seams) tests passed!")
