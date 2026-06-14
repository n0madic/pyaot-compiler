# Backlog §1 — `**dict` spread into a direct call.
#
# A `**{literal}` dict (string-literal keys, known at compile time) flattens into
# keyword arguments at compile time; a non-literal `**d` (a variable / call
# result) is evaluated once and each named parameter is bound from it at run time
# (`dict[name]` for a required param, `dict.get(name, default)` for a defaulted
# one). Mirrors CPython's call protocol on the shapes the corpus exercises.

def accepts_two(a: int, b: int) -> int:
    return a + b

def with_defaults(a: int, b: int = 10, c: int = 20) -> int:
    return a + b + c

def three(x: int, y: int, z: int) -> int:
    return x + y + z

def kwonly(a: int, *, b: int = 50) -> int:
    return a + b


# ── literal **{...} dicts (compile-time flatten) ──
assert accepts_two(**{"a": 5, "b": 10}) == 15
assert accepts_two(a=1, **{"b": 2}) == 3
assert accepts_two(**{"a": 10, "b": 20}) == 30
assert with_defaults(**{"a": 1}) == 31
assert with_defaults(**{"a": 1, "b": 2}) == 23
assert with_defaults(**{"a": 1, "c": 3}) == 14
# literal **{...} combined with a literal `*[...]` positional spread
assert three(*[1, 2], **{"z": 3}) == 6
assert three(1, *[2], **{"z": 3}) == 6
print("literal **kwargs spread passed")


# ── runtime **d dicts (per-parameter binding) ──
d_basic: dict[str, int] = {"a": 5, "b": 10}
assert accepts_two(**d_basic) == 15

d_mixed: dict[str, int] = {"b": 20}
assert accepts_two(a=1, **d_mixed) == 21

d_defaults: dict[str, int] = {"a": 5}
assert with_defaults(**d_defaults) == 35

d_partial_b: dict[str, int] = {"a": 1, "b": 2}
assert with_defaults(**d_partial_b) == 23

d_partial_c: dict[str, int] = {"a": 1, "c": 3}
assert with_defaults(**d_partial_c) == 14

d_kwonly_full: dict[str, int] = {"a": 10, "b": 20}
assert kwonly(**d_kwonly_full) == 30

d_kwonly_part: dict[str, int] = {"a": 25}
assert kwonly(**d_kwonly_part) == 75
print("runtime **kwargs spread passed")


# ── **d from a function result, evaluated once ──
def make_kwargs() -> dict[str, int]:
    return {"a": 3, "b": 4}

assert accepts_two(**make_kwargs()) == 7
print("function-result **kwargs spread passed")

print("All **kwargs spread tests passed")
