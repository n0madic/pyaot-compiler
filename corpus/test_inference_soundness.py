# Inference soundness (KNOWN_BUGS Phase 2, "Root A"): an inferred container
# element type must never become an unchecked Raw/Heap slot that a write
# through another path (alias, call, field, global, closure, sub-container)
# can falsify, and arithmetic / comparison result types must follow Python.
import itertools
import math
from typing import Optional

GLOBAL_LIST = [1.5]


class Holder:
    def __init__(self) -> None:
        self.items = [1.5]


class Weird:
    def __init__(self, v: int) -> None:
        self.v = v

    def __lt__(self, other: "Weird") -> str:
        return "lt"

    def __eq__(self, other: object) -> bool:
        return isinstance(other, Weird) and self.v == other.v

    def __truediv__(self, other: int) -> str:
        return "div"


def push_int(xs):
    xs.append(4)


def gradual_div(d):
    return d / 2


def make_closure_list():
    v = [0.5]

    def push() -> None:
        v.append(9)

    push()
    return v


def cut(xs: list[int], hi: Optional[int]) -> list[int]:
    return xs[1:hi]


def take(n: Optional[int]) -> list[int]:
    return list(itertools.islice(range(10), n))


def test_container_aliasing() -> None:
    a = [1.0, 2.5]
    b = a
    b.append(7)
    assert a[2] == 7 and str(a[2]) == "7"
    push_int(a)
    assert str(a[3]) == "4"
    c = [1.0, 2]
    assert str(c[1]) == "2"
    d = [1.0]
    d.append(3)
    assert str(d[1]) == "3"
    m = {"x": 1.5}
    m.setdefault("y", 2)
    assert str(m["y"]) == "2"
    m.update({"z": 3})
    assert str(m["z"]) == "3"
    s = [1.5, 2.5]
    s[0:1] = [4]
    assert str(s[0]) == "4"
    nested = [[1.0], [2]]
    assert str(nested[1][0]) == "2"
    u = [1.0] + [2]
    assert str(u[1]) == "2"
    GLOBAL_LIST.append(2)
    assert str(GLOBAL_LIST[1]) == "2"
    h = Holder()
    h.items.append(3)
    assert str(h.items[1]) == "3"
    mat = [[1.5], [2.5]]
    row = mat[0]
    row.append(7)
    assert str(mat[0][1]) == "7"
    assert str(make_closure_list()[1]) == "9"
    t = ([1.5],)
    t[0].append(8)
    assert str(t[0][1]) == "8"
    dl = {"a": [1.5]}
    dl["a"].append(6)
    assert str(dl["a"][1]) == "6"
    q = [1.5]
    for z in [q]:
        z.append(5)
    assert str(q[1]) == "5"
    pick = [1.0] if len(q) > 100 else [2]
    assert str(pick[0]) == "2"


def test_annotated_float_container() -> None:
    # An annotated `list[float]` holding an int never crashes; its element
    # reads take the checked unbox (numeric tower).
    xs: list[float] = [1.0, 2]
    xs.append(3)
    assert xs[1] + xs[2] == 5.0
    assert str(xs) == "[1.0, 2, 3]"


def test_bool_arithmetic() -> None:
    assert str(True + True) == "2"
    assert str(True * 3) == "3"
    assert str(True - False) == "1"
    assert str(True & False) == "False"
    assert str(True | False) == "True"
    assert str(True ^ True) == "False"
    x = True & True
    assert str(x) == "True"
    assert str(True & 3) == "1"


def test_dunder_results() -> None:
    assert (Weird(1) < Weird(2)) == "lt"
    assert Weird(1) == Weird(1)
    assert Weird(1) != Weird(2)
    assert gradual_div(Weird(1)) == "div"
    assert gradual_div(5) == 2.5


def test_big_ints() -> None:
    assert math.factorial(20) == 2432902008176640000
    assert math.factorial(25) == 15511210043330985984000000
    assert math.gcd(3000000000000000000000, 6000000000000000000) == 6000000000000000000
    assert math.gcd(2**70, 2**65) == 2**65
    assert math.lcm(2**40, 3) == 3 * 2**40
    assert math.comb(60, 30) == 118264581564861424
    assert math.perm(30, 20) == 73096577329197271449600000
    assert len("abc") + 2**61 == 2305843009213693955
    try:
        math.factorial(-1)
        assert False
    except ValueError as e:
        assert str(e) == "factorial() not defined for negative values"


def test_sum_and_bounds() -> None:
    empty: list[float] = []
    assert str(sum(empty)) == "0"
    assert str(sum([1.5, 2.5])) == "4.0"
    assert cut([1, 2, 3, 4], None) == [2, 3, 4]
    assert cut([1, 2, 3, 4], 3) == [2, 3]
    assert take(None) == [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
    assert take(3) == [0, 1, 2]


test_container_aliasing()
test_annotated_float_container()
test_bool_arithmetic()
test_dunder_results()
test_big_ints()
test_sum_and_bounds()
print("test_inference_soundness passed!")
