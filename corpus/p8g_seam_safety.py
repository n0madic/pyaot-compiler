"""Phase 8 seam-safety regressions: the stdlib/container seam used to pass a
mismatched heap shape (or a bare None/null) straight to the frozen runtime,
which dereferenced it without a guard — a family of SEGVs and silent wrong
values. Each line below crashed or silently misbehaved before the fix."""

import os
import json

# `sep.join(iterable)` accepts ANY iterable (CPython), but rt_str_join reads a
# ListObj — a str / tuple / generator argument used to SEGV. Now materialized.
print(",".join("abc"))
print("-".join(("x", "y")))
print(",".join(str(i) for i in range(3)))
print("".join([c.upper() for c in "hi"]))

# `dict.get(missing)` on a heap-valued dict: the None-on-miss was misread as a
# `str` pointer (typed as the value type, not Optional) → SEGV / crash.
sd = {"a": "x", "b": "y"}
print(sd.get("a"), sd.get("missing"), sd.get("missing", "DEF"))
print(os.environ.get("PYAOT_DEFINITELY_MISSING_XYZ"))
print(os.environ.get("PYAOT_DEFINITELY_MISSING_XYZ", "fallback"))

# `json.loads(...)[key]`: a str key on the `Any`/Dyn result went through the
# i64-index getter and silently returned None. Now routes to the dict getter.
doc = json.loads('{"name": "ada", "age": 36, "tags": ["x", "y"]}')
print(doc["name"], doc["age"], doc["tags"])
arr = json.loads('[10, 20, 30]')
print(arr[1])

# `list.index(missing)` raised nothing (returned -1); now raises ValueError.
xs = [10, 20, 30]
print(xs.index(20))
try:
    xs.index(99)
    print("no raise")
except ValueError:
    print("index raised ValueError")
