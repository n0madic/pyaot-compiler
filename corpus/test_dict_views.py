# Dict view objects: `dict.keys()` / `dict.values()` / `dict.items()` return
# LIVE views over the dict (dict_keys / dict_values / dict_items), not list
# snapshots. Covers liveness after mutation, len / iteration / `in`, the
# set-like operators on keys and items views, `==`, `reversed`, repr, the
# list/sorted/tuple materializations, `isdisjoint`, the
# "dictionary changed size during iteration" RuntimeError, the TypeError on
# subscripting a view, and the same surface through a gradual (Dyn) receiver.
# Also: a method declared `def m(*args)` (no explicit self) packs the instance
# as args[0].


def _views_are_live() -> None:
    d = {"a": 1, "b": 2}
    ks = d.keys()
    vs = d.values()
    its = d.items()
    assert len(ks) == 2 and len(vs) == 2 and len(its) == 2
    d["c"] = 3
    assert len(ks) == 3 and len(vs) == 3 and len(its) == 3
    assert list(ks) == ["a", "b", "c"]
    assert list(vs) == [1, 2, 3]
    assert list(its) == [("a", 1), ("b", 2), ("c", 3)]
    del d["a"]
    assert list(ks) == ["b", "c"]
    assert "a" not in ks and "c" in ks
    d.clear()
    assert len(ks) == 0 and list(vs) == [] and list(its) == []
    assert not ks
    d["z"] = 26
    assert ks and list(ks) == ["z"]


_views_are_live()


def _view_iteration() -> None:
    d = {"x": 10, "y": 20, "z": 30}
    out = []
    for k in d.keys():
        out.append(k)
    assert out == ["x", "y", "z"]
    total = 0
    for v in d.values():
        total += v
    assert total == 60
    pairs = []
    for k, v in d.items():
        pairs.append(k + str(v))
    assert pairs == ["x10", "y20", "z30"]
    assert [k.upper() for k in d.keys()] == ["X", "Y", "Z"]
    assert sum(d.values()) == 60
    assert max(d.values()) == 30 and min(d.values()) == 10
    assert sorted(d.items(), key=lambda kv: -kv[1]) == [("z", 30), ("y", 20), ("x", 10)]
    assert max(d.items(), key=lambda kv: kv[1]) == ("z", 30)
    assert ",".join(d.keys()) == "x,y,z"
    assert list(enumerate(d.values())) == [(0, 10), (1, 20), (2, 30)]
    assert list(zip(d.keys(), d.values())) == [("x", 10), ("y", 20), ("z", 30)]
    assert dict(d.items()) == d
    assert set(d.keys()) == {"x", "y", "z"}
    assert tuple(d.values()) == (10, 20, 30)
    assert sorted(d.keys(), reverse=True) == ["z", "y", "x"]
    it = iter(d.values())
    assert next(it) == 10 and next(it) == 20
    assert any(v > 25 for v in d.values()) and all(v > 5 for v in d.values())
    l2: list[str] = []
    l2.extend(d.keys())
    assert l2 == ["x", "y", "z"]
    a, b, c = d.values()
    assert (a, b, c) == (10, 20, 30)
    first, *rest = d.keys()
    assert first == "x" and rest == ["y", "z"]


_view_iteration()


def _view_membership() -> None:
    d = {1: "one", 2: "two"}
    assert 1 in d.keys() and 3 not in d.keys()
    assert "two" in d.values() and "three" not in d.values()
    assert (1, "one") in d.items() and (1, "two") not in d.items()
    assert (5, "one") not in d.items()


_view_membership()


def _view_set_ops() -> None:
    d = {"a": 1, "b": 2, "c": 3}
    e = {"b": 20, "c": 30, "d": 40}
    assert d.keys() & e.keys() == {"b", "c"}
    assert d.keys() | e.keys() == {"a", "b", "c", "d"}
    assert d.keys() - e.keys() == {"a"}
    assert d.keys() ^ e.keys() == {"a", "d"}
    assert d.keys() & {"a", "zz"} == {"a"}
    assert {"a", "zz"} & d.keys() == {"a"}
    assert {"a", "zz"} | d.keys() == {"a", "b", "c", "zz"}
    assert {"a", "zz"} - d.keys() == {"zz"}
    assert d.keys() - ["a", "b"] == {"c"}
    assert d.items() & {("a", 1), ("b", 99)} == {("a", 1)}
    assert d.items() - {("a", 1)} == {("b", 2), ("c", 3)}
    r = d.keys() & e.keys()
    assert isinstance(r, set) and len(r) == 2
    assert d.keys().isdisjoint({"x", "y"})
    assert not d.keys().isdisjoint(["c"])
    assert d.items().isdisjoint([("a", 2)])
    assert d.keys() == {"a", "b", "c"}
    assert {"a", "b", "c"} == d.keys()
    assert d.keys() != {"a"}
    assert d.keys() == {"c": 0, "b": 0, "a": 0}.keys()
    assert d.items() == {("a", 1), ("b", 2), ("c", 3)}
    assert d.keys() != ["a", "b", "c"]
    assert d.keys() <= {"a", "b", "c", "d"} and d.keys() < {"a", "b", "c", "d"}
    assert not (d.keys() < {"a", "b", "c"}) and d.keys() >= {"a"}
    vs = d.values()
    assert vs == vs and d.values() != d.values()


_view_set_ops()


def _view_repr_and_reversed() -> None:
    d = {"a": 1, "b": 2}
    assert repr(d.keys()) == "dict_keys(['a', 'b'])"
    assert str(d.values()) == "dict_values([1, 2])"
    assert repr(d.items()) == "dict_items([('a', 1), ('b', 2)])"
    assert f"{d.keys()}" == "dict_keys(['a', 'b'])"
    assert repr({}.keys()) == "dict_keys([])"
    assert list(reversed(d.keys())) == ["b", "a"]
    assert list(reversed(d.values())) == [2, 1]
    assert list(reversed(d.items())) == [("b", 2), ("a", 1)]
    assert str([d.keys()]) == "[dict_keys(['a', 'b'])]"
    assert type(d.keys()).__name__ == "dict_keys"
    assert type(d.items()).__name__ == "dict_items"


_view_repr_and_reversed()


def _view_errors() -> None:
    d = {"a": 1, "b": 2}
    msgs = []
    try:
        d.keys()[0]
    except TypeError as e:
        msgs.append(str(e))
    try:
        d.values()[0]
    except TypeError as e:
        msgs.append(str(e))
    try:
        d.items()[0]
    except TypeError as e:
        msgs.append(str(e))
    assert msgs == [
        "'dict_keys' object is not subscriptable",
        "'dict_values' object is not subscriptable",
        "'dict_items' object is not subscriptable",
    ], msgs
    caught = ""
    try:
        for k in d.keys():
            d[k + k] = 0
    except RuntimeError as e:
        caught = str(e)
    assert caught == "dictionary changed size during iteration"
    caught = ""
    d2 = {1: 1, 2: 2}
    try:
        for v in d2.values():
            d2[v + 10] = v
    except RuntimeError as e:
        caught = str(e)
    assert caught == "dictionary changed size during iteration"
    caught = ""
    d3 = {1: 1, 2: 2}
    try:
        for k, v in d3.items():
            del d3[k]
    except RuntimeError as e:
        caught = str(e)
    assert caught == "dictionary changed size during iteration"
    # Overwriting values (no size change) during iteration is allowed.
    d4 = {"p": 1, "q": 2}
    for k, v in d4.items():
        d4[k] = v * 100
    assert d4 == {"p": 100, "q": 200}


_view_errors()


def _dyn_ident(x):
    return x


def _views_through_dyn() -> None:
    d = _dyn_ident({"a": 1, "b": 2})
    ks = d.keys()
    d.setdefault("c", 3)
    assert len(ks) == 3
    assert list(ks) == ["a", "b", "c"]
    assert "b" in ks
    assert ks == {"a", "b", "c"}
    assert ks & {"a", "q"} == {"a"}
    assert repr(d.values()) == "dict_values([1, 2, 3])"
    assert sorted(d.items()) == [("a", 1), ("b", 2), ("c", 3)]
    assert ks.isdisjoint({"z"})
    v = _dyn_ident(d.items())
    assert len(v) == 3 and ("c", 3) in v
    assert [k for k, _ in v] == ["a", "b", "c"]
    msg = ""
    try:
        _dyn_ident(ks)[0]
    except TypeError as e:
        msg = str(e)
    assert msg == "'dict_keys' object is not subscriptable"


_views_through_dyn()


def _keys_of(d: dict[str, int]) -> list[str]:
    return list(d.keys())


def _view_params() -> None:
    d = {"m": 1, "n": 2}
    assert _keys_of(d) == ["m", "n"]
    counts: dict[str, int] = {}
    for w in ["a", "b", "a"]:
        counts[w] = counts.get(w, 0) + 1
    assert sorted(counts.items()) == [("a", 2), ("b", 1)]
    assert [k for k, v in counts.items() if v > 1] == ["a"]
    assert {v: k for k, v in counts.items()} == {2: "a", 1: "b"}
    nested = {"x": {"y": 1}}
    assert [list(inner.keys()) for inner in nested.values()] == [["y"]]


_view_params()


class _VarArgsSelf:
    def __init__(self) -> None:
        self.tag = "t"

    def m(*args):
        return len(args)

    def first(*args):
        return args[0].tag


def _varargs_method_self() -> None:
    o = _VarArgsSelf()
    assert o.m() == 1
    assert o.m(1, 2) == 3
    assert o.first() == "t"


_varargs_method_self()


print("All dict view tests passed!")
