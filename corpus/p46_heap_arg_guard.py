# Gradual heap-arg shape guard at the call/arg/return seam (PLAN §1, closed).
#
# When a genuinely-`Dyn` value flows into a typed `Heap` parameter (a builtin
# container `str`/`list`/`dict`/`set`/`tuple`, or a user-class instance),
# lowering now emits a CHECKED `Tagged -> Heap(shape)` coercion: a runtime guard
# (`rt_check_heap_kind` / `rt_check_instance`) validates the tag at the boundary.
# A correct-shape value passes through untouched; a wrong-shape value raises a
# defined `TypeError` AT THE BOUNDARY instead of crashing later at a container op
# (the pre-§1 "TaggedToHeap trust"). This is the `Heap` analogue of §8's checked
# `rt_unbox_float` for `float` params.
#
# A genuinely-`Dyn` value is produced by an UNANNOTATED passthrough parameter
# (gradual `Dyn`, NOT a Union — the static type at the call site is `Dyn`, while
# the runtime value varies). `flag`-mixed `if/return` branches would instead join
# to a `Union`, which the type checker rejects from a typed `Heap` slot.
#
# ERROR-PATH DIVERGENCE (safe): a `Dyn` int into a container param ->
#   * pyaot raises `TypeError` at the call boundary (the shape guard);
#   * CPython raises `TypeError` inside the body at `len(x)` (no len on an int).
# Both are caught and print one fixed string, so stdout is identical. The bodies
# have NO side effect before `len(x)`, so the earlier-vs-later raise is unobservable.


def as_dyn(x):
    # Unannotated param -> gradual `Dyn`; the inferred return type is `Dyn`.
    return x


# ===== correct path: a Dyn value that IS the shape passes the guard =====
def takes_str(x: str) -> int:
    return len(x)


def takes_list(x: list) -> int:
    return len(x)


def takes_dict(x: dict) -> int:
    return len(x)


def takes_set(x: set) -> int:
    return len(x)


def takes_tuple(x: tuple) -> int:
    return len(x)


assert takes_str(as_dyn("hello")) == 5
assert takes_list(as_dyn([1, 2, 3])) == 3
assert takes_dict(as_dyn({1: 10, 2: 20})) == 2
assert takes_set(as_dyn({1, 2, 3, 4})) == 4
assert takes_tuple(as_dyn((7, 8))) == 2
print("correct str len:", takes_str(as_dyn("world!")))
print("correct list len:", takes_list(as_dyn([1, 2, 3, 4, 5])))
print("correct dict len:", takes_dict(as_dyn({1: 1})))
print("correct set len:", takes_set(as_dyn({9})))
print("correct tuple len:", takes_tuple(as_dyn((1, 2, 3))))


# ===== error path: a Dyn int -> TypeError (guard here, `len(int)` in CPython) =====
try:
    takes_str(as_dyn(42))
    print("ERROR: str guard did not fire")
except TypeError:
    print("caught str TypeError")

try:
    takes_list(as_dyn(42))
    print("ERROR: list guard did not fire")
except TypeError:
    print("caught list TypeError")

try:
    takes_dict(as_dyn(42))
    print("ERROR: dict guard did not fire")
except TypeError:
    print("caught dict TypeError")

try:
    takes_set(as_dyn(42))
    print("ERROR: set guard did not fire")
except TypeError:
    print("caught set TypeError")

try:
    takes_tuple(as_dyn(42))
    print("ERROR: tuple guard did not fire")
except TypeError:
    print("caught tuple TypeError")


# ===== class instance: subclass-aware CORRECT path only =====
# A `Dog` (subclass of `Animal`) into an `Animal` param passes the instance
# guard (`rt_check_instance` is subclass-aware). The class REJECT path diverges
# (CPython raises `AttributeError` at `.name`, not `TypeError`), so it is pinned
# by a runtime unit test, not here.
class Animal:
    def __init__(self, name: str) -> None:
        self.name = name


class Dog(Animal):
    pass


def animal_name(a: Animal) -> str:
    return a.name


assert animal_name(as_dyn(Animal("Rex"))) == "Rex"
assert animal_name(as_dyn(Dog("Fido"))) == "Fido"
print("animal:", animal_name(as_dyn(Animal("Rex"))))
print("dog-as-animal:", animal_name(as_dyn(Dog("Fido"))))


print("Heap arg guard (PLAN §1) tests passed!")
