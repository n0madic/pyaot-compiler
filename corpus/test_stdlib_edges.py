"""Consolidated stdlib edge-case parity tests.

Merges the cross-module stdlib edge-case point tests that each span several
stdlib modules into one file:
  - p8g_seam_safety.py    (join / dict.get-miss / json-subscript / None-to-runtime seams; list.index ValueError)
  - p8h_stdlib_edges.py   (urlencode str()-ify, posixpath basename/dirname, quote safe=, json ensure_ascii, slice step=0 ValueError, int/bool float-format, HTTPError/URLError __str__)
  - p8h_checked_unbox.py   (checked Dyn->raw unbox at math.* raw-ABI boundaries)
  - p8h_checked_unbox2.py  (checked-unbox seams: Optional/None->f64 TypeError, raw-i64 gcd/comb/factorial/perm from Dyn)
  - _typed_stdlib_returns  (typed stdlib-object return fed from a gradual Union: the checked `Tagged/Union -> Heap(RuntimeObj)` seam via `rt_check_runtime_obj`, plus bare-name `from io import StringIO` annotation resolution)

Each source body is wrapped in a `def _<sourcename>()` and called below. All
checks are asserts; several seams legitimately raise TypeError/ValueError and
assert the caught outcome.
"""

import math
import os
import json
import re
from re import Match
from io import StringIO
from urllib.parse import quote, urlencode
from urllib.error import HTTPError, URLError


# Module-level helpers hoisted out of the checked-unbox sources (a typed Python
# subset can't nest these cleanly). Prefixed by their source name.


def _cu_pick(flag):
    if flag:
        return 2.25
    return 4.0


def _cu_bad(flag):
    if flag:
        return "not a number"
    return 1.0


def _cu2_opt(flag):
    if flag:
        return 2.25
    return None


def _cu2_pick_int(flag):
    if flag:
        return 12
    return 8


def _cu2_maybe_bool(flag):
    if flag:
        return True
    return 6


def _cu2_wrap(v):
    return v


def _seam_safety():
    # `sep.join(iterable)` accepts ANY iterable (CPython); rt_str_join reads a
    # ListObj, so str / tuple / generator args must be materialized first.
    assert ",".join("abc") == "a,b,c"
    assert "-".join(("x", "y")) == "x-y"
    assert ",".join(str(i) for i in range(3)) == "0,1,2"
    assert "".join([c.upper() for c in "hi"]) == "HI"

    # `dict.get(missing)` on a heap-valued dict: None-on-miss must not be misread
    # as a `str` pointer (Optional value type).
    sd = {"a": "x", "b": "y"}
    assert sd.get("a") == "x"
    assert sd.get("missing") is None
    assert sd.get("missing", "DEF") == "DEF"
    assert os.environ.get("PYAOT_DEFINITELY_MISSING_XYZ") is None
    assert os.environ.get("PYAOT_DEFINITELY_MISSING_XYZ", "fallback") == "fallback"

    # `json.loads(...)[key]`: a str key on the Any/Dyn result must route to the
    # dict getter, not the i64-index getter.
    doc = json.loads('{"name": "ada", "age": 36, "tags": ["x", "y"]}')
    assert doc["name"] == "ada"
    assert doc["age"] == 36
    assert doc["tags"] == ["x", "y"]
    arr = json.loads('[10, 20, 30]')
    assert arr[1] == 20

    # `list.index(missing)` must raise ValueError (not silently return -1).
    xs = [10, 20, 30]
    assert xs.index(20) == 1
    raised = False
    try:
        xs.index(99)
    except ValueError:
        raised = True
    assert raised


def _stdlib_edges():
    # --- #13: urlencode must str()-ify non-str values ---
    assert urlencode({"i": 5, "b": True, "f": 2.5, "s": "x y"}) == "i=5&b=True&f=2.5&s=x+y"
    assert urlencode({"neg": -7, "none_flag": False}) == "neg=-7&none_flag=False"

    # --- #14b/c: posixpath basename/dirname edge cases ---
    assert os.path.basename("/x/y/") == ""
    assert os.path.basename("/x/y") == "y"
    assert os.path.basename("/") == ""
    assert os.path.basename("x") == "x"
    assert os.path.basename("") == ""
    assert os.path.dirname("/x/y/") == "/x/y"
    assert os.path.dirname("/x/y") == "/x"
    assert os.path.dirname("/") == "/"
    assert os.path.dirname("x") == ""
    assert os.path.dirname("") == ""
    assert os.path.dirname("//x") == "//"
    assert os.path.split("/x/y/") == ("/x/y", "")
    assert os.path.split("/") == ("/", "")

    # --- #14d: quote's default safe is "/" ---
    assert quote("a/b c") == "a/b%20c"
    assert quote("a/b c", "") == "a%2Fb%20c"

    # --- #14e: json.dumps escapes non-ASCII (ensure_ascii=True) ---
    assert json.dumps({"k": "привет"}) == '{"k": "\\u043f\\u0440\\u0438\\u0432\\u0435\\u0442"}'
    assert json.dumps(["emoji: 😀", "ascii"]) == '["emoji: \\ud83d\\ude00", "ascii"]'

    # --- #14f: slice step=0 raises ValueError ---
    xs = [1, 2, 3]
    list_err = ""
    try:
        xs[::0]
    except ValueError as e:
        list_err = str(e)
    assert list_err == "slice step cannot be zero"
    s = "abc"
    str_err = ""
    try:
        s[::0]
    except ValueError as e:
        str_err = str(e)
    assert str_err == "slice step cannot be zero"
    t = (1, 2, 3)
    tuple_err = ""
    try:
        t[::0]
    except ValueError as e:
        tuple_err = str(e)
    assert tuple_err == "slice step cannot be zero"

    # --- #15a: int/bool with a float presentation type ---
    assert f"{3:10.4f}" == "    3.0000"
    assert f"{True:.2f}" == "1.00"
    assert f"{42:e}" == "4.200000e+01"
    assert f"{1:%}" == "100.000000%"
    assert f"{7:g}" == "7"

    # --- #16: HTTPError/URLError __str__ matches CPython ---
    http_str = ""
    try:
        raise HTTPError("https://example.com/x", 404, "Not Found", None, None)
    except HTTPError as e:
        http_str = str(e)
    assert http_str == "HTTP Error 404: Not Found"
    url_str = ""
    try:
        raise URLError("nope")
    except URLError as e:
        url_str = str(e)
    assert url_str == "<urlopen error nope>"


def _checked_unbox():
    # Dyn argument (unannotated function return) into a raw-f64 param
    v = _cu_pick(True)
    assert math.sqrt(v) == 1.5
    assert math.floor(_cu_pick(False)) == 4

    # int / bool arguments into a raw-f64 param (CPython promotes)
    assert math.sqrt(16) == 4.0
    assert math.sqrt(True) == 1.0
    assert math.exp(0) == 1.0
    assert math.log(1) == 0.0

    # int literal still works, floats keep the fast path
    assert math.sqrt(2.25) == 1.5
    assert math.pow(2, 10) == 1024.0

    # a wrong tag raises TypeError (caught, not a crash)
    caught = False
    try:
        math.sqrt(_cu_bad(True))
    except TypeError:
        caught = True
    assert caught


def _checked_unbox2():
    # 1. Optional (Union[float, None]) into a raw-f64 param
    assert math.sqrt(_cu2_opt(True)) == 1.5
    none_caught = False
    try:
        math.sqrt(_cu2_opt(False))
    except TypeError:
        none_caught = True
    assert none_caught

    # 2. Dyn from a mixed-list element into raw-f64 (int/bool promote)
    mixed = [2.25, 16, True]
    assert math.sqrt(mixed[0]) == 1.5
    assert math.sqrt(mixed[1]) == 4.0
    assert math.sqrt(mixed[2]) == 1.0
    assert math.floor(mixed[0]) == 2

    # 3. Dyn from dict.get into raw-f64; a miss (None) raises TypeError
    table = {"a": 6.25}
    assert math.sqrt(table.get("a")) == 2.5
    miss_caught = False
    try:
        math.sqrt(table.get("b"))
    except TypeError:
        miss_caught = True
    assert miss_caught

    # 4. Dyn into raw-i64 params (math.gcd / comb / factorial / perm)
    assert math.gcd(_cu2_pick_int(True), 18) == 6
    assert math.comb(_cu2_pick_int(False), 2) == 28

    dyn_ints = [12, 5, "nope"]
    assert math.gcd(dyn_ints[0], 18) == 6
    assert math.comb(dyn_ints[1], 2) == 10
    assert math.factorial(dyn_ints[1]) == 120
    assert math.perm(dyn_ints[1], 2) == 20

    # 5. bool through a Dyn return into raw-i64 (CPython: bool is an int)
    assert math.gcd(_cu2_maybe_bool(True), 8) == 1
    assert math.gcd(_cu2_maybe_bool(False), 8) == 2

    # 6. str through Dyn into raw-i64 raises TypeError (caught, no crash)
    str_caught = False
    try:
        math.gcd(dyn_ints[2], 4)
    except TypeError:
        str_caught = True
    assert str_caught

    # 7. Chained Dyn producers into raw-f64
    assert math.sqrt(_cu2_wrap(_cu2_opt(True))) == 1.5

    # 8. Contrast: statically proven args keep the unchecked fast path
    assert math.gcd(12, 18) == 6
    assert math.sqrt(2.25) == 1.5


# Typed stdlib-object return fed from a gradual Union source — the bundled
# `requests` `-> HTTPResponse` facade pattern. A `try: return <stdlib-obj>
# except <Error> as e: return e` body is inferred as
# `Union[RuntimeObject, BuiltinException]`; that Union flows into a typed
# stdlib-object return slot. typeck admits it behind a runtime tag guard
# (`rt_check_runtime_obj`) instead of an unchecked reinterpret (PITFALLS B18 /
# invariant #2). `_tsr_pick` is unannotated, so its inferred return climbs to
# the union; `_tsr_matched` pins the typed `re.Match` return the union flows into.


def _tsr_pick(ok):
    # Inferred return Union[re.Match, ValueError] (the join of the body returns).
    try:
        if not ok:
            raise ValueError("boom")
        m = re.match(r"(\d+)", "42")
        if m is None:
            raise ValueError("nomatch")
        return m
    except ValueError as e:
        return e


def _tsr_matched(ok) -> Match:
    # The Union flows into the typed `re.Match` return slot — coerced behind the
    # checked `Tagged -> Heap(RuntimeObj(Match))` runtime tag guard.
    return _tsr_pick(ok)


def _tsr_sbuf_pick(ok):
    # Inferred return Union[io.StringIO, ValueError].
    try:
        if not ok:
            raise ValueError("boom")
        return StringIO("io-hi")
    except ValueError as e:
        return e


def _tsr_sbuf(ok) -> StringIO:
    # `-> StringIO` is a BARE-name `from io import StringIO` annotation: the name
    # is BOTH a constructor function and a class, and both must bind so the
    # annotation resolves to `RuntimeObject(StringIO)` (not silently `Dyn`). The
    # union then flows into it behind the same runtime tag guard.
    return _tsr_sbuf_pick(ok)


def _typed_stdlib_returns():
    # Positive: the union resolves to the real Match; the typed-return guard
    # passes and `.group()` dispatches as a typed `re.Match` method (NOT the
    # gradual `rt_obj_method` path), proving the result kept its precise type.
    good = _tsr_matched(True)
    assert good.group(0) == "42"
    assert good.group(1) == "42"

    # Wrong-shape (the B18 mandatory path): the union resolves to the
    # ValueError. pyaot's typed-return guard raises `TypeError` AT THE BOUNDARY;
    # CPython (which ignores the annotation) raises `AttributeError` only when
    # the value is used as a Match. Both are caught → identical outcome, and the
    # pyaot guard fires instead of dereferencing a wrong-shape value (no SEGV).
    rejected = False
    try:
        bad = _tsr_matched(False)
        bad.group(0)
    except (TypeError, AttributeError):
        rejected = True
    assert rejected

    # The bare-name `from io import StringIO` annotation resolves: `.getvalue()`
    # typed-dispatches on the result (constructor + class share the name, both
    # bound). Construction via the same name still works.
    assert StringIO("direct").getvalue() == "direct"
    assert _tsr_sbuf(True).getvalue() == "io-hi"
    sbuf_rejected = False
    try:
        _tsr_sbuf(False).getvalue()
    except (TypeError, AttributeError):
        sbuf_rejected = True
    assert sbuf_rejected


_seam_safety()
_stdlib_edges()
_checked_unbox()
_checked_unbox2()
_typed_stdlib_returns()

print("All stdlib edge tests passed!")
