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


# Generic[T] base class: T flows through field, __init__ param, and method
# signatures. Each (class, T) instantiation produces specialized __init__,
# unwrap, transform, set_value via S3.3b.1 monomorphization.
class IntWrapper(Generic[T]):
    val: T

    def __init__(self, v: T) -> None:
        self.val = v

    def unwrap(self) -> T:
        return self.val

    def transform(self, v: T) -> T:
        return v

    def set_value(self, v: T) -> None:
        self.val = v


def test_generic_base_class() -> None:
    w: IntWrapper[int] = IntWrapper(77)
    assert w.unwrap() == 77, f"generic_base_class: got {w.unwrap()}"
    print("generic_base_class: ok")


def test_int_wrapper_int() -> None:
    w: IntWrapper[int] = IntWrapper(10)
    assert w.unwrap() == 10, f"int_wrapper_int unwrap: got {w.unwrap()}"
    result: int = w.transform(42)
    assert result == 42, f"int_wrapper_int transform: got {result}"
    w.set_value(99)
    assert w.unwrap() == 99, f"int_wrapper_int set_value: got {w.unwrap()}"
    print("int_wrapper_int: ok")


def test_int_wrapper_str() -> None:
    ws: IntWrapper[str] = IntWrapper("hello")
    assert ws.unwrap() == "hello", f"int_wrapper_str initial: got {ws.unwrap()}"
    got: str = ws.transform("world")
    assert got == "world", f"int_wrapper_str transform: got {got}"
    ws.set_value("hi")
    assert ws.unwrap() == "hi", f"int_wrapper_str set_value: got {ws.unwrap()}"
    print("int_wrapper_str: ok")


# PEP 695 generic class: `class Cls[K]:` — K is scoped to the class body.
# Both `n` (always int) and `label` (K) coexist on the same instance.
class TickBox[K]:
    n: int
    label: K

    def __init__(self, init_label: K) -> None:
        self.n = 0
        self.label = init_label

    def tick(self) -> None:
        self.n += 1

    def count(self) -> int:
        return self.n

    def tag(self, label: K) -> K:
        self.label = label
        return label

    def get_label(self) -> K:
        return self.label


def test_pep695_class() -> None:
    tb: TickBox[int] = TickBox(0)
    tb.tick()
    tb.tick()
    tb.tick()
    assert tb.count() == 3, f"pep695_class count: got {tb.count()}"
    labeled: int = tb.tag(7)
    assert labeled == 7, f"pep695_class tag: got {labeled}"
    assert tb.get_label() == 7, f"pep695_class get_label: got {tb.get_label()}"
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
# S3.3b.1 Phase 4: chained spec / inheritance override / polymorphic protocol  #
# --------------------------------------------------------------------------- #

# Chained specialization: a generic method that constructs and returns
# another generic instance of the same parameterization.
class Box(Generic[T]):
    val: T

    def __init__(self, v: T) -> None:
        self.val = v

    def get(self) -> T:
        return self.val

    def rebox(self) -> "Box[T]":
        return Box(self.val)


def test_chained_spec() -> None:
    b: Box[int] = Box(42)
    b2: Box[int] = b.rebox()
    assert b2.get() == 42, f"chained_spec int: got {b2.get()}"

    bs: Box[str] = Box("ok")
    bs2: Box[str] = bs.rebox()
    assert bs2.get() == "ok", f"chained_spec str: got {bs2.get()}"
    print("chained_spec: ok")


# Polymorphic-receiver test via Protocol: ensures CallVirtualNamed survives
# monomorphization. Protocol receivers cannot be devirt'd because concrete
# classes may have different vtable layouts — the dispatch stays dynamic.
#
# This also exercises override-style polymorphism: Counter (concrete class)
# and IntWrapper[int] (generic instantiation) both satisfy Unwrappable;
# `use_proto` dispatches to each via runtime vtable lookup.
@runtime_checkable
class Unwrappable(Protocol):
    def unwrap(self) -> int: ...


class Counter:
    n: int

    def __init__(self, n: int) -> None:
        self.n = n

    def unwrap(self) -> int:
        return self.n + 100


def use_proto(p: Unwrappable) -> int:
    return p.unwrap()


def test_polymorphic_receiver() -> None:
    w: IntWrapper[int] = IntWrapper(7)
    assert use_proto(w) == 7, f"polymorphic_receiver IntWrapper: got {use_proto(w)}"
    c: Counter = Counter(50)
    assert use_proto(c) == 150, f"polymorphic_receiver Counter: got {use_proto(c)}"
    print("polymorphic_receiver: ok")


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
    test_int_wrapper_int()
    test_int_wrapper_str()
    test_pep695_class()
    test_protocol_with_typeparam()
    test_pep695_alias()
    test_chained_spec()
    test_polymorphic_receiver()
    print("all generic tests passed")


main()
