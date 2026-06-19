# In-method instance-field annotations as field-type contracts (PLAN §8 follow-up).
#
# `self.<name>: T = v` (and the bare `self.<name>: T` declaration) inside a method
# declares the field's type exactly like a class-level `name: T`. CPython does not
# record method-body attribute annotations in `__annotations__` and keeps the raw
# value; this compiler treats the annotation as a contract (a frontend pre-scan
# collects `self.<name>: T` into the class's field annotations before typeck). For
# a `float` field fed an int, the store goes through the §8 numeric-tower box.
#
# DIVERGENCE-SAFE: a `float` field written from an int holds `5.0` here but `5` in
# CPython (repr-print-only divergence, like p16/p44), so assert via `==` and print
# only float-FORCED results (`x + 0.5`). Non-numeric fields (str) print directly.


# ===== float field via in-method annotation, written from an int =====
class Box:
    def __init__(self, v: int) -> None:
        self.x: float = v          # int value into a float-annotated field
        self.label: str = "box"    # str field declared in-method

    def get(self) -> float:
        return self.x


b = Box(5)
assert b.x == 5.0
assert b.label == "box"
print(b.get() + 0.5)               # 5.5
print(b.x + 0.5)                   # 5.5
print(b.label)                     # box


# ===== bare `self.x: float` declaration (no value), written later with an int ===
class Lazy:
    def __init__(self) -> None:
        self.v: float                # pure type declaration (no store)
        self.v = 0                    # int write into the float field

    def bump(self, n: int) -> None:
        self.v = n                    # another int write -> §8 SetField box


lz = Lazy()
assert lz.v == 0.0
lz.bump(7)
assert lz.v == 7.0
print(lz.v + 0.5)                  # 7.5


# ===== annotation in a NON-__init__ method + a nested block =====
class Acc:
    def __init__(self) -> None:
        self.total = 0.0

    def configure(self, flag: bool, k: int) -> None:
        if flag:
            self.scale: float = k     # in-method annotation inside an `if`
        else:
            self.scale = 1.0

    def scaled(self) -> float:
        return self.total * self.scale


a = Acc()
a.total = 4.0
a.configure(True, 3)               # scale: float = 3 (int) -> 3.0
assert a.scale == 3.0
assert a.scaled() == 12.0
print(a.scale + 0.5)               # 3.5
print(a.scaled() + 0.5)            # 12.5


# ===== interaction: in-method float field feeds a float-param free function =====
def half(x: float) -> float:
    return x * 0.5


box2 = Box(9)                       # box2.x: float = 9 -> 9.0
assert half(box2.x) == 4.5         # the boxed float field flows into a float param
print(half(box2.x))                # 4.5  (body multiplies by a float literal)


# ===== bignum int into an in-method float field (the §8 box bignum arm) =====
class Big:
    def __init__(self, n: int) -> None:
        self.val: float = n


big = Big(2 ** 62)                  # exact power of two, f64-representable
assert big.val == 4611686018427387904.0


print("In-method field annotation tests passed!")
