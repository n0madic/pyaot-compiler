# Gradual heap READ-BACK guard + class-reject path (PLAN §1 follow-up, closed).
#
# Sibling of p46 (the call/arg/return seams). This probe closes the two items
# §1 originally deferred:
#
#  1. READ-BACK seam — a genuinely-`Dyn` value (a `Dyn` GLOBAL / FIELD / local
#     read as `Tagged`) assigned into an annotated guard-backed `Heap` LOCAL
#     (`x: list = <dyn>`) now takes the CHECKED `Tagged -> Heap(shape)` coercion
#     (`rt_check_heap_kind`), the store-into-typed-local analogue of the call
#     seams. A correct-shape value passes; a wrong-shape value raises `TypeError`
#     at the assignment instead of crashing later at a typed op (`.append`,
#     typed `x[i]`) that trusts the layout.
#
#  2. CLASS-REJECT path — a wrong-shape `Dyn` value into an `Animal` instance
#     param/local: pyaot raises `TypeError` at `rt_check_instance`, CPython
#     raises `AttributeError` at `.name` (the annotation is ignored). The
#     exception TYPE diverges, so the reject is caught broadly
#     (`except (TypeError, AttributeError)`) and printed as one fixed string —
#     identical stdout. (The correct subclass-aware path is in p46.)
#
# A genuinely-`Dyn` value comes from an UNANNOTATED passthrough (gradual `Dyn`,
# not a Union). Container error paths are caught broadly: pyaot always raises
# `TypeError` at the read-back guard (before the body op), while CPython raises
# whatever the body op produces on an `int` — `TypeError` for `len(x)`,
# `AttributeError` for `x.append(...)`. The fixed-string output is identical
# either way; a missing guard would SIGSEGV (a typed op on a non-container), not
# raise, so the broad catch cannot mask a pyaot failure.


def as_dyn(x):
    return x


# ===== read-back into a typed LOCAL: a Dyn value that IS the shape passes =====
def rb_str(d) -> int:
    x: str = d
    return len(x)


def rb_list(d) -> int:
    x: list = d
    x.append(0)          # typed list op: x stays Heap(List); guard survives
    return len(x)


def rb_dict(d) -> int:
    x: dict = d
    return len(x)


def rb_set(d) -> int:
    x: set = d
    return len(x)


def rb_tuple(d) -> int:
    x: tuple = d
    return len(x)


assert rb_str(as_dyn("hello")) == 5
assert rb_list(as_dyn([1, 2, 3])) == 4      # 3 + appended 0
assert rb_dict(as_dyn({1: 1, 2: 2})) == 2
assert rb_set(as_dyn({1, 2, 3})) == 3
assert rb_tuple(as_dyn((9, 8))) == 2
print("local read-back str:", rb_str(as_dyn("world")))
print("local read-back list:", rb_list(as_dyn([7, 7])))
print("local read-back dict:", rb_dict(as_dyn({1: 1})))
print("local read-back set:", rb_set(as_dyn({5})))
print("local read-back tuple:", rb_tuple(as_dyn((1, 2, 3))))


# ===== read-back from a Dyn GLOBAL into a typed local =====
gdyn = as_dyn({10: 100, 20: 200, 30: 300})


def from_global() -> int:
    y: dict = gdyn       # Dyn global -> typed dict local
    return len(y)


assert from_global() == 3
print("global read-back:", from_global())


# ===== read-back from a Dyn FIELD into a typed local =====
class Box:
    def __init__(self, v):       # `v` unannotated -> the field is Dyn
        self.contents = v


def from_field(b: Box) -> int:
    z: list = b.contents         # Dyn field -> typed list local
    return len(z)


assert from_field(Box([1, 2, 3, 4])) == 4
print("field read-back:", from_field(Box([6, 6, 6])))


# ===== read-back ERROR path: a Dyn int into a typed local (both TypeError) =====
def try_rb(label, fn):
    try:
        fn(as_dyn(42))
        print("ERROR:", label, "guard did not fire")
    except (TypeError, AttributeError):
        print("caught", label, "read-back error")


try_rb("str", rb_str)
try_rb("list", rb_list)
try_rb("dict", rb_dict)
try_rb("set", rb_set)
try_rb("tuple", rb_tuple)


# ===== class-reject path: a Dyn int into an Animal param (type diverges) =====
class Animal:
    def __init__(self, name: str) -> None:
        self.name = name


def animal_name(a: Animal) -> str:
    return a.name


try:
    animal_name(as_dyn(42))
    print("ERROR: instance guard did not fire")
except (TypeError, AttributeError):
    # pyaot: TypeError at rt_check_instance; CPython: AttributeError at `.name`.
    print("caught class-reject error")


# ===== class-reject via read-back into a typed Animal LOCAL =====
def animal_local(d) -> str:
    a: Animal = d        # Dyn -> typed Animal local (rt_check_instance)
    return a.name


try:
    animal_local(as_dyn(42))
    print("ERROR: instance read-back guard did not fire")
except (TypeError, AttributeError):
    print("caught class read-back error")


print("Heap read-back guard (PLAN §1 follow-up) tests passed!")
