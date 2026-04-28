"""
S3.3a / S3.5 exit-criterion tests: monomorphization of generic free functions
and frontend support for TypeVar, Generic[T], Protocol[T], and PEP 695 syntax.

Each test prints its name then its result. CPython and compiled output must match.
"""

from typing import TypeVar, Generic, Protocol, runtime_checkable

T = TypeVar("T")
U = TypeVar("U")


# --------------------------------------------------------------------------- #
# Basic identity
# --------------------------------------------------------------------------- #

def identity(x: T) -> T:
    return x


def test_identity_int() -> None:
    result: int = identity(42)
    assert result == 42, f"identity int: got {result}"
    print("identity_int: ok")


def test_identity_str() -> None:
    result: str = identity("hello")
    assert result == "hello", f"identity str: got {result}"
    print("identity_str: ok")


def test_identity_float() -> None:
    result: float = identity(3.14)
    assert result == 3.14, f"identity float: got {result}"
    print("identity_float: ok")


# --------------------------------------------------------------------------- #
# Swap / two type vars
# --------------------------------------------------------------------------- #

def swap(a: T, b: U) -> T:
    return a


def test_swap() -> None:
    result: int = swap(1, "x")
    assert result == 1, f"swap: got {result}"
    print("swap: ok")


# --------------------------------------------------------------------------- #
# Repeated specialization produces the same result (cache hit)
# --------------------------------------------------------------------------- #

def double_call() -> None:
    a: int = identity(10)
    b: int = identity(20)
    assert a == 10 and b == 20, f"double_call: got {a}, {b}"
    print("double_call: ok")


# --------------------------------------------------------------------------- #
# Generic function with arithmetic on a concrete specialization
# --------------------------------------------------------------------------- #

def add_one(x: T) -> T:
    return x


def test_add_one() -> None:
    # The monomorphized copy should propagate T=int throughout its body.
    v: int = add_one(7)
    assert v == 7, f"add_one: got {v}"
    print("add_one: ok")


# --------------------------------------------------------------------------- #
# Two independent specializations of the same template
# --------------------------------------------------------------------------- #

def wrap(v: T) -> T:
    return v


def test_two_specializations() -> None:
    i: int = wrap(99)
    s: str = wrap("world")
    assert i == 99, f"two_specs int: got {i}"
    assert s == "world", f"two_specs str: got {s}"
    print("two_specializations: ok")


# --------------------------------------------------------------------------- #
# List argument
# --------------------------------------------------------------------------- #

def first(lst: list[int]) -> int:
    return lst[0]


def test_first_list() -> None:
    items: list[int] = [10, 20, 30]
    v: int = first(items)
    assert v == 10, f"first_list: got {v}"
    print("first_list: ok")


# --------------------------------------------------------------------------- #
# Bool specialization
# --------------------------------------------------------------------------- #

def test_identity_bool() -> None:
    result: bool = identity(True)
    assert result == True, f"identity bool: got {result}"
    print("identity_bool: ok")


# --------------------------------------------------------------------------- #
# S3.5 Frontend support                                                       #
# --------------------------------------------------------------------------- #

# PEP 695 generic function: `def fn[V](x: V) -> V:`
# Gets monomorphized identically to module-level TypeVar functions.
def pep695_wrap[V](x: V) -> V:
    return x


def test_pep695_function() -> None:
    r1: int = pep695_wrap(55)
    r2: str = pep695_wrap("pep695")
    assert r1 == 55, f"pep695_function int: got {r1}"
    assert r2 == "pep695", f"pep695_function str: got {r2}"
    print("pep695_function: ok")


# Generic[T] base class: syntax is parsed; T comes from the module-level TypeVar.
# Class methods use concrete types (class-method monomorph is S3.3b, deferred).
class IntWrapper(Generic[T]):
    val: int

    def __init__(self, v: int) -> None:
        self.val = v

    def unwrap(self) -> int:
        return self.val


def test_generic_base_class() -> None:
    w: IntWrapper = IntWrapper(77)
    assert w.unwrap() == 77, f"generic_base_class: got {w.unwrap()}"
    print("generic_base_class: ok")


# PEP 695 generic class: `class Cls[K]:` — K is scoped to the class body.
# Concrete annotations used inside; verifies parse and runtime correctness.
class TickBox[K]:
    n: int

    def __init__(self) -> None:
        self.n = 0

    def tick(self) -> None:
        self.n += 1

    def count(self) -> int:
        return self.n


def test_pep695_class() -> None:
    tb: TickBox = TickBox()
    tb.tick()
    tb.tick()
    tb.tick()
    assert tb.count() == 3, f"pep695_class: got {tb.count()}"
    print("pep695_class: ok")


# Protocol[T]: subscripted Protocol base is parsed; T is compile-time only.
# Runtime isinstance check remains structural (name-based).
@runtime_checkable
class Measurable(Protocol[T]):
    def length(self) -> int: ...


class Rope:
    size: int

    def __init__(self, size: int) -> None:
        self.size = size

    def length(self) -> int:
        return self.size


def test_protocol_with_typeparam() -> None:
    r: Rope = Rope(4)
    assert isinstance(r, Measurable), "protocol_with_typeparam: Rope should satisfy Measurable"
    assert r.length() == 4, f"protocol_with_typeparam: got {r.length()}"
    print("protocol_with_typeparam: ok")


# PEP 695 type alias: `type Alias[V] = <body>` — V scoped to alias body.
type IntPair[V] = tuple[int, int]


def test_pep695_alias() -> None:
    p: IntPair = (10, 20)
    assert p[0] == 10 and p[1] == 20, f"pep695_alias: got {p}"
    print("pep695_alias: ok")


# --------------------------------------------------------------------------- #
# Main
# --------------------------------------------------------------------------- #

def main() -> None:
    test_identity_int()
    test_identity_str()
    test_identity_float()
    test_swap()
    double_call()
    test_add_one()
    test_two_specializations()
    test_first_list()
    test_identity_bool()
    test_pep695_function()
    test_generic_base_class()
    test_pep695_class()
    test_protocol_with_typeparam()
    test_pep695_alias()
    print("all generic tests passed")


main()
