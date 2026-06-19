# Heterogeneous-numeric tuple iteration: the element type of `for x in (1.5, 1)`
# must NOT collapse to `float` via the numeric tower. The runtime tuple stores
# each element as a tagged Value (a boxed float AND a tagged int), and the
# iterator yields Tagged — so a `Raw(F64)` element type would raw-unbox the
# tagged `int` element as a FloatObj pointer (SIGSEGV). typeck's `iter_elem_ty`
# routes the tuple-element fold through the Raw-uniformity guard: a mixed
# `(float, int)` tuple iterates as `Dyn` (Tagged), a homogeneous `(float,
# float)` stays precise. (Regression for the autograd-accumulation crash.)


# --- The autograd-accumulation crash pattern (mixed float/int tuple). ---
class _Accum:
    __slots__ = ("data", "acc")

    def __init__(self, d):
        self.data = d
        self.acc = 0  # frontend infers Int from the literal `0`

    def add_grad(self, scalar):
        # `scalar` is bound from a heterogeneous-tuple element, so its param
        # type must stay Tagged — never a raw float that unboxes a tagged int.
        self.acc = self.acc + scalar * self.data


_a = _Accum(2.0)
_b = _Accum(3.0)
for _scalar in (1.5, 1):
    _a.add_grad(_scalar)
    _b.add_grad(_scalar)

print(_a.acc)            # 1.5*2.0 + 1*2.0 = 5.0
print(_b.acc)            # 1.5*3.0 + 1*3.0 = 7.5
print(_a.acc ** 2)       # 25.0
print(_b.acc ** 2)       # 56.25


# --- Iterating the mixed tuple directly into a list. ---
print([x for x in (1.5, 1)])
print([x * 2 for x in (2.5, 2)])


# --- Mixed bool/float and bool/int (same numeric-tower hazard). ---
_tot = 0.0
for _v in (True, 2.0, 3):
    _tot = _tot + _v
print(_tot)              # 1 + 2.0 + 3 = 6.0


# --- Homogeneous tuples keep precise iteration (control). ---
_fs = 0.0
for _f in (1.0, 2.0, 3.5):
    _fs = _fs + _f
print(_fs)               # 6.5

_is = 0
for _i in (10, 20, 30):
    _is = _is + _i
print(_is)               # 60

print("p43 hetero-tuple iteration: PASS")
