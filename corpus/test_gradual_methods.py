# Gradual-completeness method dispatch: a `Dyn`/`Union`-typed receiver can call
# methods at run time, the CPython `type(obj).method` model. ONE runtime entry
# (`rt_obj_method`) decides by the receiver's tag — container methods route to
# the typed `rt_list_*`/`rt_dict_*`/`rt_set_*`/`rt_deque_*` family (Phase A); an
# instance routes through the method's uniform thunk (Phase B). The front-half
# emits this for any `recv.m(...)` whose receiver type is `Dyn` or `Union`
# (`lower_dyn_method_call` → `CallRuntime{RT_OBJ_METHOD}`), so inference precision
# stays a pure performance lever, never a correctness requirement (Invariant 2).
#
# Receivers are forced genuinely `Dyn` two ways: a heterogeneous dict's values
# (no common base → value type `Dyn`) and a heterogeneous list's elements. Output
# is kept deterministic (sets/dict-views via `sorted`).
from collections import deque


# ===== Phase A: container methods on a `Dyn` receiver =====
def container_methods():
    # A heterogeneous dict → its values are `Dyn`; `box[k]` is a `Dyn` receiver.
    box = {}

    box["lst"] = [3, 1, 2]
    box["lst"].append(4)
    box["lst"].sort()
    assert box["lst"] == [1, 2, 3, 4]
    box["lst"].insert(0, 9)
    assert box["lst"] == [9, 1, 2, 3, 4]
    assert box["lst"].index(2) == 2
    assert box["lst"].count(9) == 1
    box["lst"].reverse()
    assert box["lst"] == [4, 3, 2, 1, 9]
    box["lst"].remove(9)
    assert box["lst"].pop() == 1
    assert box["lst"] == [4, 3, 2]
    assert box["lst"].copy() == [4, 3, 2]
    box["lst"].extend([7, 8])
    assert box["lst"] == [4, 3, 2, 7, 8]
    box["lst"].clear()
    assert box["lst"] == []

    box["d"] = {"a": 1}
    box["d"].update({"b": 2})
    box["d"].setdefault("c", 3)
    box["d"].setdefault("a", 99)  # present → keeps 1
    assert box["d"].get("a") == 1
    assert box["d"].get("z") is None
    assert box["d"].get("z", -1) == -1
    assert sorted(box["d"].keys()) == ["a", "b", "c"]
    assert sorted(box["d"].values()) == [1, 2, 3]
    assert sorted(box["d"].items()) == [("a", 1), ("b", 2), ("c", 3)]
    assert box["d"].pop("a") == 1
    assert box["d"].pop("zz", -7) == -7
    box["d"].clear()
    assert box["d"] == {}
    assert box["d"].copy() == {}

    box["s"] = {1, 2, 3}
    box["s"].add(4)
    box["s"].discard(2)
    box["s"].discard(99)  # absent → no error
    box["s"].remove(1)
    box["s"].update({5, 6})
    assert sorted(box["s"]) == [3, 4, 5, 6]
    assert sorted(box["s"].copy()) == [3, 4, 5, 6]

    box["dq"] = deque([1, 2, 3])
    box["dq"].append(4)
    box["dq"].appendleft(0)
    box["dq"].extend([5, 6])
    assert list(box["dq"]) == [0, 1, 2, 3, 4, 5, 6]
    assert box["dq"].pop() == 6
    assert box["dq"].popleft() == 0
    assert box["dq"].count(3) == 1
    box["dq"].clear()
    assert list(box["dq"]) == []

    print("container methods on Dyn: ok")


# ===== Phase B: user methods on a genuinely-`Dyn` receiver =====
class Base:
    def __init__(self, x):
        self.x = x

    def kind(self):
        return "base"

    def val(self):
        return self.x

    def combine(self, other, scale=1):
        return (self.x + other) * scale


class Derived(Base):
    def kind(self):
        return "derived"  # override

    def doubled(self):
        return self.x * 2


class Other:
    def kind(self):
        return "other"

    def val(self):
        return -1


def call_kind(obj):
    # `obj` is an unannotated param → `Dyn`; called with several unrelated types.
    return obj.kind()


def user_methods():
    # A heterogeneous list (no common base) → element type `Dyn`.
    items = [Base(10), Derived(7), Other()]
    kinds = [it.kind() for it in items]  # `it` is genuinely `Dyn`
    assert kinds == ["base", "derived", "other"]
    assert call_kind(items[0]) == "base"
    assert call_kind(items[1]) == "derived"
    assert call_kind(items[2]) == "other"

    d = items[1]  # `Dyn`, holds a Derived
    assert d.doubled() == 14          # own method
    assert d.val() == 7               # inherited (Base.val) — self coerces C→B
    assert d.kind() == "derived"      # overridden
    assert d.combine(100) == 107      # default arg
    assert d.combine(100, 3) == 321   # positional 2nd arg
    assert d.combine(5, scale=2) == 24  # positional-or-keyword param by keyword
    assert d.combine(other=5, scale=2) == 24  # both by keyword

    b = items[0]  # `Dyn`, holds a Base
    assert b.combine(3) == 13
    assert b.val() == 10

    print("user methods on Dyn: ok")


# ===== Scalar methods + sort(reverse=) on a `Dyn` receiver =====
def scalar_methods():
    box = {}

    # tuple.index / .count on a `Dyn` tuple.
    box["t"] = (10, 20, 30, 20)
    box["pad"] = {}  # keep the dict's value type `Dyn`
    assert box["t"].index(20) == 1
    assert box["t"].count(20) == 2

    # int methods on a `Dyn` int (immediate fixnum, bool, and heap bignum).
    box["n"] = 255
    assert box["n"].bit_length() == 8
    assert box["n"].bit_count() == 8
    assert box["n"].conjugate() == 255
    assert box["n"].__index__() == 255
    box["b"] = True
    assert box["b"].bit_length() == 1
    box["big"] = 2 ** 70
    assert box["big"].bit_length() == 71

    # list.sort(reverse=) on a `Dyn` list (the keyword the container branch honors;
    # `sort(key=)` is handled upstream by the type-blind frontend desugar).
    box["lst"] = [1, 3, 2, 5, 4]
    box["lst"].sort(reverse=True)
    assert box["lst"] == [5, 4, 3, 2, 1]
    box["lst"].sort(reverse=False)
    assert box["lst"] == [1, 2, 3, 4, 5]

    print("scalar methods on Dyn: ok")


def main():
    container_methods()
    user_methods()
    scalar_methods()
    print("test_gradual_methods passed!")


main()
