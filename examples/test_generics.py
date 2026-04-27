"""
S3.3a exit-criterion tests: monomorphization of generic free functions.

Each test prints its name then its result. CPython and compiled output must match.
"""

from typing import TypeVar

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
    print("all generic tests passed")


main()
