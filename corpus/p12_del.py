"""Backlog §3 — the `del` statement: `del d[k]`, `del li[i]`, `del name`,
`del obj.attr`.

Container deletes (`del d[k]` / `del li[i]` / `del dq[i]` / a class
`__delitem__`) are runtime calls that raise `KeyError`/`IndexError` like CPython.
Name/attribute deletes store an UNBOUND sentinel into the slot; a guarded read
of the now-unbound slot raises `UnboundLocalError`/`NameError`/`AttributeError`.
Every raising case is wrapped in `try`/`except` so the script exits 0 and diffs
byte-exact vs CPython (the printed text is fixed, never the exception message).
"""


# ── del d[k] — present, missing (KeyError), survivor ────────────────────────
d = {"a": 1, "b": 2, "c": 3}
del d["b"]
print(len(d))
print(d["a"], d["c"])
print("b" in d)
try:
    del d["zzz"]
    print("no error")
except KeyError:
    print("KeyError caught")
print(len(d))  # the dict survived the failed delete


# ── del li[i] — positive, negative, OOB (IndexError), shift ─────────────────
xs = [10, 20, 30, 40, 50]
del xs[0]
print(xs)
del xs[-1]
print(xs)
del xs[1]
print(xs)
try:
    del xs[99]
    print("no error")
except IndexError:
    print("IndexError caught")
print(xs)


# NOTE: `del dq[i]` on a `collections.deque` is implemented (the `del` Generic
# path routes to `rt_any_delitem` → `rt_deque_delete`), but populating a deque
# (`deque([...])` / `.append`) is a separate backlog gap (§10), so it cannot be
# exercised here yet — it is covered by a runtime unit test instead.


# ── user class with __delitem__ (`del self.data[i]` on an attribute base) ───
class IntList:
    def __init__(self) -> None:
        self.data = [10, 20, 30]

    def __delitem__(self, i: int) -> None:
        del self.data[i]  # subscript del on an attribute base, inside a method

    def __len__(self) -> int:
        return len(self.data)


container = IntList()
del container[1]
print(container.data)
print(len(container))


# ── del name (local): del then rebind + read is fine ────────────────────────
def rebind() -> int:
    x = 5
    del x
    x = 10
    return x


print(rebind())


# ── del name (local): del then read raises UnboundLocalError ────────────────
def use_after_del() -> None:
    y = 7
    del y
    try:
        print(y)
        print("no error")
    except UnboundLocalError:
        print("UnboundLocalError caught")


use_after_del()


# ── del name (module global): del then read raises NameError ────────────────
g = 99
del g
try:
    print(g)
    print("no error")
except NameError:
    print("NameError caught")


# ── del obj.attr: del then read raises AttributeError, then reassign + read ─
class Holder:
    def __init__(self) -> None:
        self.payload = 42


h = Holder()
print(h.payload)
del h.payload
try:
    print(h.payload)
    print("no error")
except AttributeError:
    print("AttributeError caught")
h.payload = 7
print(h.payload)


# ── interaction probes: del in if / loop bodies, multi-target ───────────────
d3 = {"x": 1, "y": 2, "z": 3}
if "y" in d3:
    del d3["y"]
print("y" in d3, len(d3))

words = {"a": 1, "b": 2, "c": 3, "d": 4}
for k in ["a", "c"]:
    del words[k]
print(sorted(words.keys()))


def multi_target() -> None:
    a = 1
    b = 2
    del a, b
    try:
        print(a)
        print("no error")
    except UnboundLocalError:
        print("UnboundLocalError caught (a)")
    try:
        print(b)
        print("no error")
    except UnboundLocalError:
        print("UnboundLocalError caught (b)")


multi_target()
