# Phase 8H stage A — stdlib edge-case parity (offline).
# Covers: urlencode str()-ification of non-str values (#13), posixpath
# basename/dirname edges (#14b/c), quote's safe="/" default (#14d),
# json.dumps ensure_ascii (#14e), slice step=0 -> ValueError (#14f),
# int/bool with a float presentation type (#15a), and the CPython __str__
# of HTTPError/URLError (#16).

from urllib.parse import quote, urlencode
from urllib.error import HTTPError, URLError
import os
import json

# --- #13: urlencode must str()-ify non-str values (was a SEGV) ---
print(urlencode({"i": 5, "b": True, "f": 2.5, "s": "x y"}))
print(urlencode({"neg": -7, "none_flag": False}))

# --- #14b/c: posixpath basename/dirname edge cases ---
print(os.path.basename("/x/y/"))
print(os.path.basename("/x/y"))
print(os.path.basename("/"))
print(os.path.basename("x"))
print(os.path.basename(""))
print(os.path.dirname("/x/y/"))
print(os.path.dirname("/x/y"))
print(os.path.dirname("/"))
print(os.path.dirname("x"))
print(os.path.dirname(""))
print(os.path.dirname("//x"))
print(os.path.split("/x/y/"))
print(os.path.split("/"))

# --- #14d: quote's default safe is "/" ---
print(quote("a/b c"))
print(quote("a/b c", ""))

# --- #14e: json.dumps escapes non-ASCII (ensure_ascii=True) ---
print(json.dumps({"k": "привет"}))
print(json.dumps(["emoji: 😀", "ascii"]))

# --- #14f: slice step=0 raises ValueError ---
xs = [1, 2, 3]
try:
    print(xs[::0])
except ValueError as e:
    print("list step0:", e)
s = "abc"
try:
    print(s[::0])
except ValueError as e:
    print("str step0:", e)
t = (1, 2, 3)
try:
    print(t[::0])
except ValueError as e:
    print("tuple step0:", e)

# --- #15a: int/bool with a float presentation type ---
print(f"{3:10.4f}")
print(f"{True:.2f}")
print(f"{42:e}")
print(f"{1:%}")
print(f"{7:g}")

# --- #16: HTTPError/URLError __str__ matches CPython ---
try:
    raise HTTPError("https://example.com/x", 404, "Not Found", None, None)
except HTTPError as e:
    print(e)
try:
    raise URLError("nope")
except URLError as e:
    print(e)

print("p8h stdlib edges passed!")
