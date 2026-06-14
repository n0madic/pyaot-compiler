# test_functions.py lift, Phase 2 (b1) — closure/lambda VALUES carry a static
# `Callable` signature. A lambda or nested `def` used as a first-class value is
# now typed `Callable(sig)` (its visible param/return reprs) instead of `Dyn`, so
# binding it to a variable and calling that variable rides the native
# indirect-call ABI (`CallIndirect`) — no new runtime, the Tagged baseline.
#
# This probe exercises ONLY the patterns whose signatures are statically
# determinable: a lambda bound to a name, and a single-level factory returning an
# annotated nested closure. Genuinely-`Dyn` callees (a closure whose return type
# widens to `Dyn` across a multi-level return, an unannotated parameter holding a
# closure, decorated slots) still require a sound dynamic-call ABI and stay out of
# scope (see PLAN.md).


# -- 1. A lambda bound to a variable, then called --
add = lambda x, y: x + y
print(add(2, 3))
print(add(40, 2))

is_pos = lambda x: x > 0
print(is_pos(5))
print(is_pos(-3))

double = lambda x: x * 2
print(double(21))


# -- 2. A lambda capturing an enclosing variable, bound and called --
def lambda_capture() -> int:
    base: int = 100
    inc = lambda d: base + d
    return inc(5)


print(lambda_capture())


# -- 3. A single-level factory returning an annotated nested closure --
def adder_factory(n: int):
    def add_n(x: int) -> int:
        return x + n

    return add_n


add5 = adder_factory(5)
print(add5(10))
print(add5(100))


def scale_factory(k: int):
    def scale(x: int) -> int:
        return x * k

    return scale


triple = scale_factory(3)
print(triple(7))


# -- 4. A returned closure stored and invoked more than once --
def counter_base(start: int):
    def step(d: int) -> int:
        return start + d

    return step


s = counter_base(10)
print(s(1))
print(s(2))
print(s(40))


print("p39 closure values passed!")
