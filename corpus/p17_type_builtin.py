# `type()` builtin, incl. `type(x).__name__` (PLAN §6).
#
# MODEL: a pyaot "type object" is its repr STRING (a StrObj). `type(x)` lowers to
# `CallBuiltin(Type)` → `rt_builtin_type` → `<class '...'>` (builtins via the value
# tag, user instances via the registered module-qualified qualname). That single
# string satisfies the whole corpus surface: `str(type(x))` (str of a StrObj is
# idempotent), `print(type(x))`, and `==` on the name.
#
# `type(x).__name__` is the BARE name. It must come from the SAME runtime source,
# not a parallel compile-time table (PLAN §6 trap): a lowering peephole routes
# `type(<1 arg>).__name__` through `rt_type_name_extract`, which takes the
# `<class 'mod.Name'>` string and returns its last dotted segment.
#
# DOCUMENTED DIVERGENCES (out of scope, not probed): `type(x) is T` /
# `type(x) is type(y)` (pointer-identity on distinct StrObjs) and `repr(type(x))`
# (would add quotes). CPython prints the type object the same as `str()`, so
# `print(type(x))` DOES match here.


# ===== str(type(v)) for each builtin — module-qualified for none, bare here ====
type_list: list[int] = [1, 2, 3]
type_tuple: tuple[int, int] = (1, 2)
type_dict: dict[str, int] = {"a": 1}
type_set: set[int] = {1, 2, 3}

assert str(type(42)) == "<class 'int'>"
assert str(type(3.14)) == "<class 'float'>"
assert str(type(True)) == "<class 'bool'>"
assert str(type(False)) == "<class 'bool'>"
assert str(type("hello")) == "<class 'str'>"
assert str(type(None)) == "<class 'NoneType'>"
assert str(type(type_list)) == "<class 'list'>"
assert str(type(type_tuple)) == "<class 'tuple'>"
assert str(type(type_dict)) == "<class 'dict'>"
assert str(type(type_set)) == "<class 'set'>"


# ===== type(v).__name__ for each builtin — bare name from the runtime extractor =
assert type(42).__name__ == "int"
assert type(3.14).__name__ == "float"
assert type(True).__name__ == "bool"
assert type(False).__name__ == "bool"
assert type("hello").__name__ == "str"
assert type(None).__name__ == "NoneType"
assert type(type_list).__name__ == "list"
assert type(type_tuple).__name__ == "tuple"
assert type(type_dict).__name__ == "dict"
assert type(type_set).__name__ == "set"


# ===== user class: qualified vs bare from the SAME source =====
class Widget:
    def __init__(self) -> None:
        self.x = 1


assert str(type(Widget())) == "<class '__main__.Widget'>"
assert type(Widget()).__name__ == "Widget"


# ===== interaction probes (the one-source principle, crossing green features) ===
# Bind the extracted name in a var, then use it.
name_var = type("x").__name__
assert name_var == "str"
assert name_var + "!" == "str!"

# `.__name__` inside an f-string (crosses f-string formatting).
assert f"{type(42).__name__}" == "int"
assert f"a {type(Widget()).__name__} b" == "a Widget b"

# Two same-typed values compare equal by name (crosses `==` on the extractor out).
assert type(1).__name__ == type(2).__name__
assert type("p").__name__ == type("q").__name__
assert type(1).__name__ != type(1.0).__name__


# ===== print(type(v)) directly — matches CPython (value IS the repr string) =====
print(type(42))
print(type(3.14))
print(type("hello"))
print(type(None))
print(type(type_list))
print(type(Widget()))
print(type(42).__name__)
print(type(Widget()).__name__)

print("type() tests passed")
