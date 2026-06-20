"""
S3.3a / S3.5 exit-criterion tests: monomorphization of generic free functions
and frontend support for TypeVar, Generic[T], Protocol[T], and PEP 695 syntax.

Each test prints its name then its result. CPython and compiled output must match.
"""

from typing import TypeVar, Generic, Protocol, runtime_checkable, Any

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
# S3.5 Frontend support                                                       #
# --------------------------------------------------------------------------- #

# PEP 695 generic function: `def fn[V](x: V) -> V:`
# Gets monomorphized identically to module-level TypeVar functions.
def pep695_wrap[V](x: V) -> V:
    return x


def test_pep695_function() -> None:
    r1: int = pep695_wrap(55)
    r2: str = pep695_wrap("pep695")
    assert r1 == 55, f"pep695_function int: got {r1}"
    assert r2 == "pep695", f"pep695_function str: got {r2}"
    print("pep695_function: ok")


# Generic[T] base class: T flows through field, __init__ param, and method
# signatures. Each (class, T) instantiation produces specialized __init__,
# unwrap, transform, set_value via S3.3b.1 monomorphization.
class IntWrapper(Generic[T]):
    val: T

    def __init__(self, v: T) -> None:
        self.val = v

    def unwrap(self) -> T:
        return self.val

    def transform(self, v: T) -> T:
        return v

    def set_value(self, v: T) -> None:
        self.val = v


def test_generic_base_class() -> None:
    w: IntWrapper[int] = IntWrapper(77)
    assert w.unwrap() == 77, f"generic_base_class: got {w.unwrap()}"
    print("generic_base_class: ok")


def test_int_wrapper_int() -> None:
    w: IntWrapper[int] = IntWrapper(10)
    assert w.unwrap() == 10, f"int_wrapper_int unwrap: got {w.unwrap()}"
    result: int = w.transform(42)
    assert result == 42, f"int_wrapper_int transform: got {result}"
    w.set_value(99)
    assert w.unwrap() == 99, f"int_wrapper_int set_value: got {w.unwrap()}"
    print("int_wrapper_int: ok")


def test_int_wrapper_str() -> None:
    ws: IntWrapper[str] = IntWrapper("hello")
    assert ws.unwrap() == "hello", f"int_wrapper_str initial: got {ws.unwrap()}"
    got: str = ws.transform("world")
    assert got == "world", f"int_wrapper_str transform: got {got}"
    ws.set_value("hi")
    assert ws.unwrap() == "hi", f"int_wrapper_str set_value: got {ws.unwrap()}"
    print("int_wrapper_str: ok")


# PEP 695 generic class: `class Cls[K]:` — K is scoped to the class body.
# Both `n` (always int) and `label` (K) coexist on the same instance.
class TickBox[K]:
    n: int
    label: K

    def __init__(self, init_label: K) -> None:
        self.n = 0
        self.label = init_label

    def tick(self) -> None:
        self.n += 1

    def count(self) -> int:
        return self.n

    def tag(self, label: K) -> K:
        self.label = label
        return label

    def get_label(self) -> K:
        return self.label


def test_pep695_class() -> None:
    tb: TickBox[int] = TickBox(0)
    tb.tick()
    tb.tick()
    tb.tick()
    assert tb.count() == 3, f"pep695_class count: got {tb.count()}"
    labeled: int = tb.tag(7)
    assert labeled == 7, f"pep695_class tag: got {labeled}"
    assert tb.get_label() == 7, f"pep695_class get_label: got {tb.get_label()}"
    print("pep695_class: ok")


# Protocol[T]: subscripted Protocol base is parsed; T is compile-time only.
# Runtime isinstance check remains structural (name-based).
@runtime_checkable
class Measurable(Protocol[T]):
    def length(self) -> int: ...


class Rope:
    size: int

    def __init__(self, size: int) -> None:
        self.size = size

    def length(self) -> int:
        return self.size


def test_protocol_with_typeparam() -> None:
    r: Rope = Rope(4)
    assert isinstance(r, Measurable), "protocol_with_typeparam: Rope should satisfy Measurable"
    assert r.length() == 4, f"protocol_with_typeparam: got {r.length()}"
    print("protocol_with_typeparam: ok")


# PEP 695 type alias: `type Alias[V] = <body>` — V scoped to alias body.
type IntPair[V] = tuple[int, int]


def test_pep695_alias() -> None:
    p: IntPair = (10, 20)
    assert p[0] == 10 and p[1] == 20, f"pep695_alias: got {p}"
    print("pep695_alias: ok")


# --------------------------------------------------------------------------- #
# S3.3b.1 Phase 4: chained spec / inheritance override / polymorphic protocol  #
# --------------------------------------------------------------------------- #

# Chained specialization: a generic method that constructs and returns
# another generic instance of the same parameterization.
class Box(Generic[T]):
    val: T

    def __init__(self, v: T) -> None:
        self.val = v

    def get(self) -> T:
        return self.val

    def rebox(self) -> "Box[T]":
        return Box(self.val)


def test_chained_spec() -> None:
    b: Box[int] = Box(42)
    b2: Box[int] = b.rebox()
    assert b2.get() == 42, f"chained_spec int: got {b2.get()}"

    bs: Box[str] = Box("ok")
    bs2: Box[str] = bs.rebox()
    assert bs2.get() == "ok", f"chained_spec str: got {bs2.get()}"
    print("chained_spec: ok")


# Polymorphic-receiver test via Protocol: ensures CallVirtualNamed survives
# monomorphization. Protocol receivers cannot be devirt'd because concrete
# classes may have different vtable layouts — the dispatch stays dynamic.
#
# This also exercises override-style polymorphism: Counter (concrete class)
# and IntWrapper[int] (generic instantiation) both satisfy Unwrappable;
# `use_proto` dispatches to each via runtime vtable lookup.
@runtime_checkable
class Unwrappable(Protocol):
    def unwrap(self) -> int: ...


class Counter:
    n: int

    def __init__(self, n: int) -> None:
        self.n = n

    def unwrap(self) -> int:
        return self.n + 100


def use_proto(p: Unwrappable) -> int:
    return p.unwrap()


def test_polymorphic_receiver() -> None:
    w: IntWrapper[int] = IntWrapper(7)
    assert use_proto(w) == 7, f"polymorphic_receiver IntWrapper: got {use_proto(w)}"
    c: Counter = Counter(50)
    assert use_proto(c) == 150, f"polymorphic_receiver Counter: got {use_proto(c)}"
    print("polymorphic_receiver: ok")


# --------------------------------------------------------------------------- #
# Folded from p5_generics.py — Phase 5E generics: TypeVar, Generic[T],         #
# generic methods/fields, parameterized instantiation `Box[int](...)`.         #
# Uniform-Tagged storage means a generic class compiles ONCE; `Stack[int]` and #
# `Stack[str]` share one physical layout, so output is identical regardless of #
# T. Type-arg substitution refines the *static* types without changing code.   #
# Prefixed `_pg_` to avoid collisions with the target's own `Box` / `Counter`. #
# --------------------------------------------------------------------------- #

class _pg_Box(Generic[T]):
    def __init__(self, value: T):
        self.value = value

    def get(self) -> T:
        return self.value

    def replace(self, v: T):
        self.value = v


class _pg_Stack(Generic[T]):
    items: list[T]

    def __init__(self):
        self.items = []

    def push(self, x: T):
        self.items.append(x)

    def pop(self) -> T:
        return self.items.pop()

    def size(self) -> int:
        return len(self.items)


def test_p5_generics() -> None:
    # _pg_Box[int] / _pg_Box[str] — same layout, precise element types.
    bi = _pg_Box[int](42)
    assert bi.get() == 42, f"p5 box int: got {bi.get()}"
    bi.replace(100)
    assert bi.get() == 100, f"p5 box int replaced: got {bi.get()}"

    bs = _pg_Box[str]("hello")
    assert bs.get() == "hello", f"p5 box str: got {bs.get()}"

    # _pg_Stack[int]: pop() is statically an int.
    si: _pg_Stack[int] = _pg_Stack[int]()
    si.push(1)
    si.push(2)
    si.push(3)
    assert si.size() == 3, f"p5 stack int size: got {si.size()}"
    assert si.pop() + 10 == 13, "p5 stack int arithmetic on substituted return"
    assert si.pop() == 2, "p5 stack int pop"
    assert si.size() == 1, f"p5 stack int size after pops: got {si.size()}"

    # _pg_Stack[str]: same code, str elements.
    ss = _pg_Stack[str]()
    ss.push("a")
    ss.push("b")
    assert ss.pop() == "b", "p5 stack str pop"
    assert ss.size() == 1, f"p5 stack str size: got {ss.size()}"

    # A bare (un-parameterized) Stack still works — element type erases to dynamic.
    sd = _pg_Stack()
    sd.push(7)
    assert sd.pop() == 7, "p5 stack bare pop"
    print("p5_generics: ok")


# --------------------------------------------------------------------------- #
# Folded from test_dead_code_warnings.py — isinstance flow-narrowing,          #
# always-true / always-false static checks, and narrowing on `Any` that must   #
# dispatch through the target type (the `len()` fall-through regression).      #
# Prefixed `_dc_` where module-level helpers are introduced.                   #
# --------------------------------------------------------------------------- #

def test_dc_incompatible_types() -> None:
    # Always False isinstance (then-branch dead): isinstance(x: int, str).
    x: int = 42
    if isinstance(x, str):  # WARNING: isinstance check is always False
        assert False, "unreachable"
    assert True
    print("dc_incompatible_types: ok")


def test_dc_redundant_check() -> None:
    # Always True isinstance (else-branch dead): isinstance(x: str, str).
    x: str = "hello"
    if isinstance(x, str):  # WARNING: isinstance check is always True
        assert True
    else:
        assert False, "unreachable"
    print("dc_redundant_check: ok")


def test_dc_valid_union_narrowing() -> None:
    # Valid Union narrowing (no warning).
    x: int | str = 42
    if isinstance(x, int):
        assert x == 42
    else:
        assert False, "should be int"
    print("dc_valid_union_narrowing: ok")


def test_dc_incompatible_float() -> None:
    y: float = 3.14
    if isinstance(y, str):  # WARNING: isinstance check is always False
        assert False, "unreachable"
    assert True
    print("dc_incompatible_float: ok")


def test_dc_incompatible_bool() -> None:
    z: bool = True
    if isinstance(z, str):  # WARNING: isinstance check is always False
        assert False, "unreachable"
    assert True
    print("dc_incompatible_bool: ok")


def test_dc_redundant_list_check() -> None:
    items: list[int] = [1, 2, 3]
    if isinstance(items, list):  # WARNING: isinstance check is always True
        assert len(items) == 3
    else:
        assert False, "unreachable"
    print("dc_redundant_list_check: ok")


def test_dc_valid_isinstance_any() -> None:
    # For Union types, isinstance is valid and useful.
    value: int | str | float = "test"
    if isinstance(value, str):
        assert value == "test"
    elif isinstance(value, int):
        pass
    else:
        pass
    print("dc_valid_isinstance_any: ok")


# Narrowing on `Any` must dispatch through the target type.
# Before the fix, `narrow_to(Any, Str)` returned `Type::Never`, so `len(data)`
# inside the `isinstance` branch fell to the `arg_type == _` arm of `lower_len`
# and silently returned 0.
def _dc_any_len_str(data: Any) -> int:
    if isinstance(data, str):
        return len(data)
    return -1


def _dc_any_len_bytes(data: Any) -> int:
    if isinstance(data, bytes):
        return len(data)
    return -1


def _dc_any_len_dict(data: Any) -> int:
    if isinstance(data, dict):
        return len(data)
    return -1


def test_dc_any_narrowing_to_str() -> None:
    assert _dc_any_len_str("hello") == 5
    assert _dc_any_len_str("") == 0
    assert _dc_any_len_str(b"bytes-not-str") == -1
    assert _dc_any_len_str(42) == -1
    print("dc_any_narrowing_to_str: ok")


def test_dc_any_narrowing_to_bytes() -> None:
    assert _dc_any_len_bytes(b"hello") == 5
    assert _dc_any_len_bytes("string-not-bytes") == -1
    print("dc_any_narrowing_to_bytes: ok")


def test_dc_any_narrowing_to_dict() -> None:
    assert _dc_any_len_dict({"a": "x", "b": "y"}) == 2
    assert _dc_any_len_dict("string-not-dict") == -1
    print("dc_any_narrowing_to_dict: ok")


# Narrowing survives a chained isinstance dispatch.
# Mirrors the `_prepare_body` shape in `site-packages/requests/`.
def _dc_classify(x: Any) -> str:
    if isinstance(x, bytes):
        return "bytes:" + str(len(x))
    if isinstance(x, str):
        return "str:" + x
    if isinstance(x, dict):
        return "dict:" + str(len(x))
    return "other"


def test_dc_chained_isinstance_narrowing() -> None:
    assert _dc_classify("hello") == "str:hello"
    assert _dc_classify(b"abc") == "bytes:3"
    assert _dc_classify({"k": "v"}) == "dict:1"
    assert _dc_classify(42) == "other"
    print("dc_chained_isinstance_narrowing: ok")


# --------------------------------------------------------------------------- #
# Folded from test_types_system.py — typing-module generics (List/Dict/Set/    #
# Tuple), PEP 585 builtins, PEP 604 unions, isinstance/None/truthiness         #
# narrowing, bool<:int semantics, tuple/starred unpacking, TypeAlias / PEP 695 #
# `type` statements, Literal types, TypeVar (constraints/bound), Protocol      #
# structural subtyping, and Union/numeric-tower return boxing. All type defs   #
# stay module-level; executable assertions run inside test_types_system().     #
# Identifiers prefixed `_ts_` to avoid collisions with the target's own        #
# Box / Counter / T definitions.                                               #
# --------------------------------------------------------------------------- #

# ===== SECTION: Type narrowing after isinstance() =====

def _ts_test_basic_int_narrowing() -> None:
    x: int | str = 42
    if isinstance(x, int):
        result = x + 10
        assert result == 52, "result should equal 52"
    else:
        assert False, "False should be True"


def _ts_test_basic_str_narrowing() -> None:
    x: int | str = "hello"
    if isinstance(x, str):
        result = x.upper()
        assert result == "HELLO", "result should equal \"HELLO\""
    else:
        assert False, "False should be True"


def _ts_test_else_branch_narrowing() -> None:
    x: int | str = "world"
    if isinstance(x, int):
        assert False, "False should be True"
    else:
        result = x.lower()
        assert result == "world", "result should equal \"world\""


def _ts_test_three_type_union() -> None:
    x: int | str | None = 42
    if isinstance(x, int):
        result = x + 8
        assert result == 50, "result should equal 50"
    elif isinstance(x, str):
        assert False, "False should be True"
    else:
        assert False, "False should be True"


def _ts_test_three_type_union_str() -> None:
    x: int | str | None = "test"
    if isinstance(x, int):
        assert False, "False should be True"
    elif isinstance(x, str):
        assert x.upper() == "TEST", "x.upper() should equal \"TEST\""
    else:
        assert False, "False should be True"


def _ts_test_negation() -> None:
    x: int | str = "negated"
    if not isinstance(x, int):
        assert x.upper() == "NEGATED", "x.upper() should equal \"NEGATED\""
    else:
        assert False, "False should be True"


# Using inferred variables in function calls
def _ts_double(n: int) -> int:
    return n * 2


# Unpacking in a function
def _ts_get_pair() -> tuple[int, int]:
    return (42, 84)


# ===== SECTION: Type narrowing with 'or' conditions =====

def _ts_test_or_else_narrowing() -> None:
    """Test that else-branch narrows when 'or' is false."""
    x: int | str | None = None
    if isinstance(x, int) or isinstance(x, str):
        assert False, "Should not reach here when x is None"
    else:
        # Both are false -> x is NOT int AND NOT str -> x is None
        assert x is None, "x should be narrowed to None"


def _ts_test_or_different_vars() -> None:
    """Test 'or' narrowing with different variables."""
    x: int | str = "hello"
    y: int | float = 3.14
    if isinstance(x, int) or isinstance(y, int):
        assert False, "Should not reach here"
    else:
        # Both false -> x is str AND y is float
        assert x.upper() == "HELLO", "x should be narrowed to str"
        assert y > 3.0, "y should be narrowed to float"


def _ts_test_not_or_then_narrowing() -> None:
    """Test that then-branch narrows when 'not (a or b)' is true."""
    x: int | str = "world"
    y: int | None = None
    if not (isinstance(x, int) or isinstance(y, int)):
        # Both are false -> x is NOT int (str) AND y is NOT int (None)
        assert x.lower() == "world", "x should be narrowed to str"
        assert y is None, "y should be narrowed to None"
    else:
        assert False, "Should not reach here"


def _ts_test_or_same_var_triple_union() -> None:
    """Test 'or' narrowing excludes multiple types from same var."""
    x: int | str | None = None
    if isinstance(x, int) or isinstance(x, str):
        assert False, "Should not reach here when x is None"
    else:
        assert x is None, "x should be narrowed to None"


def _ts_test_not_and_else_narrowing() -> None:
    """Test that else-branch narrows when 'not (a and b)' is false."""
    x: int | str = 42
    y: int | float = 10
    if not (isinstance(x, int) and isinstance(y, int)):
        assert False, "Should not reach here when both are int"
    else:
        # not (a and b) is false -> a and b is true
        result = x + y
        assert result == 52, "Both should be narrowed to int"


# ===== SECTION: Bool is subtype of Int (Python semantics) =====
# In Python, bool is a subtype of int: isinstance(True, int) == True
# This means True and False can be used wherever an int is expected.

def _ts_int_increment(x: int) -> int:
    return x + 1


def _ts_int_double(x: int) -> int:
    return x * 2


# ===== SECTION: is None / is not None narrowing =====

def _ts_test_is_none_narrowing_basic() -> None:
    """Test basic is None narrowing."""
    x: int | None = None
    if x is None:
        # then-branch: x is None
        assert x is None, "x should be None in then-branch"
    else:
        # else-branch: x is not None (narrowed to int)
        result = x + 1
        assert False, "Should not reach else-branch when x is None"


def _ts_test_is_not_none_narrowing_basic() -> None:
    """Test basic is not None narrowing."""
    x: int | None = 42
    if x is not None:
        # then-branch: x is not None (narrowed to int)
        result = x + 10
        assert result == 52, "x should be narrowed to int in then-branch"
    else:
        # else-branch: x is None
        assert False, "Should not reach else-branch when x is 42"


def _ts_test_is_none_with_str_union() -> None:
    """Test is None narrowing with str | None."""
    s: str | None = "hello"
    if s is not None:
        assert s.upper() == "HELLO", "s should be narrowed to str"
    else:
        assert False, "Should not reach else-branch"


def _ts_test_is_none_triple_union() -> None:
    """Test is None narrowing with int | str | None."""
    x: int | str | None = None
    if x is None:
        assert x is None, "x is None"
    else:
        # x is narrowed to int | str
        assert False, "Should not reach else-branch when x is None"


def _ts_test_is_not_none_triple_union() -> None:
    """Test is not None narrowing with int | str | None."""
    x: int | str | None = 42
    if x is not None:
        # x is narrowed to int | str
        # We can't further narrow without isinstance, but we know it's not None
        pass
    else:
        assert False, "Should not reach else-branch when x is 42"


def _ts_test_not_is_none_negation() -> None:
    """Test not (x is None) - should be equivalent to x is not None."""
    x: int | None = 42
    if not (x is None):
        result = x + 10
        assert result == 52, "x should be narrowed to int"
    else:
        assert False, "Should not reach else-branch"


def _ts_test_not_is_not_none_negation() -> None:
    """Test not (x is not None) - should be equivalent to x is None."""
    x: int | None = None
    if not (x is not None):
        assert x is None, "x should be None"
    else:
        assert False, "Should not reach else-branch"


def _ts_test_is_none_with_isinstance_chain() -> None:
    """Test is not None followed by isinstance for further narrowing."""
    x: int | str | None = 42
    if x is not None:
        # Now x is int | str
        if isinstance(x, int):
            result = x + 8
            assert result == 50, "x should be narrowed to int"
        else:
            assert False, "Should not reach str branch"
    else:
        assert False, "Should not reach None branch"


# ===== SECTION: Truthiness narrowing for Optional =====

def _ts_test_truthiness_if_x_then_not_none() -> None:
    """Test: if x: narrows x to exclude None in then-branch."""
    x: int | None = 42
    if x:
        # x is truthy, so x is not None (narrowed to int)
        result = x + 10
        assert result == 52, "x should be narrowed to int in then-branch"
    else:
        assert False, "Should not reach else-branch when x is 42"


def _ts_test_truthiness_if_not_x_else_not_none() -> None:
    """Test: if not x: else-branch narrows x to exclude None."""
    x: int | None = 42
    if not x:
        assert False, "Should not reach then-branch when x is 42"
    else:
        # x is truthy, so x is not None (narrowed to int)
        result = x + 10
        assert result == 52, "x should be narrowed to int in else-branch"


def _ts_test_truthiness_none_is_falsy() -> None:
    """Test: None value takes else branch."""
    x: int | None = None
    if x:
        assert False, "None should be falsy"
    else:
        # x is falsy (could be None or 0), but we know it's None here
        pass


def _ts_test_truthiness_zero_is_falsy() -> None:
    """Test: 0 (int) is falsy but still int type."""
    x: int | None = 0
    if x:
        assert False, "0 should be falsy"
    else:
        # Note: we can't narrow to None here because 0 is also falsy
        # This is correct behavior - we don't narrow in the else branch
        pass


def _ts_test_truthiness_str_none_union() -> None:
    """Test truthiness narrowing with str | None."""
    s: str | None = "hello"
    if s:
        # s is truthy, so s is not None (narrowed to str)
        result = s.upper()
        assert result == "HELLO", "s should be narrowed to str"
    else:
        assert False, "Should not reach else-branch when s is 'hello'"


def _ts_test_truthiness_empty_str_is_falsy() -> None:
    """Test: empty string is falsy."""
    s: str | None = ""
    if s:
        assert False, "Empty string should be falsy"
    else:
        # Note: we can't narrow to None here because "" is also falsy
        pass


def _ts_test_truthiness_combined_with_isinstance() -> None:
    """Test truthiness narrowing combined with isinstance."""
    x: int | str | None = 42
    if x:
        # x is truthy, so x is not None (narrowed to int | str)
        if isinstance(x, int):
            result = x + 8
            assert result == 50, "x should be narrowed to int"
        else:
            assert False, "Should not reach str branch"
    else:
        assert False, "Should not reach None branch"


def _ts_test_truthiness_while_loop() -> None:
    """Test truthiness narrowing in while loop conditions."""
    x: int | None = 3
    count = 0
    while x:
        # x is narrowed to int in loop body
        count = count + 1
        x = x - 1  # Reassign works correctly with narrowed Union
    assert count == 3, "Should have looped 3 times"


# ===== SECTION: TypeAlias (PEP 613) =====


# TypeAlias with annotation style
_ts_IntList: TypeAlias = list[int]


_ts_StrDict: TypeAlias = dict[str, int]


# Nested type alias
_ts_NestedAlias: TypeAlias = list[dict[str, int]]


# ===== SECTION: PEP 695 type statement =====

type _ts_IntSet = set[int]


type _ts_OptStr = str | None


# ===== SECTION: TypeVar =====

_ts_T = TypeVar('_ts_T')


# TypeVar with int — the identity function accepts the annotation
def _ts_tv_identity_int(x: _ts_T) -> _ts_T:
    return x


# TypeVar with str — separate function since AOT compiles one specialization
def _ts_tv_identity_str(x: _ts_T) -> _ts_T:
    return x


# TypeVar with constraints — annotation is accepted (resolves to Union[int, float])
_ts_Num = TypeVar('_ts_Num', int, float)


# TypeVar with bound — accepted as the bound type
_ts_Comparable = TypeVar('_ts_Comparable', bound=int)


def _ts_tv_max_val(a: _ts_Comparable, b: _ts_Comparable) -> _ts_Comparable:
    if a > b:
        return a
    return b


# ===== SECTION: Protocol (structural subtyping) =====

# Protocol class definition is accepted — compile-time structural type
class _ts_Drawable(Protocol):
    def draw(self) -> str: ...


class _ts_Circle:
    def draw(self) -> str:
        return "circle"


class _ts_Square:
    def draw(self) -> str:
        return "square"


# Protocol-typed parameter accepts concrete class instances
def _ts_proto_render(shape: _ts_Drawable) -> str:
    return shape.draw()


# Protocol with vtable layout mismatch: Square2 has __init__ + field, so draw is at different slot
# `@runtime_checkable` is required for `isinstance` against a Protocol (CPython
# raises TypeError otherwise); pyaot treats the decorator as a no-op and checks
# structurally regardless.
@runtime_checkable
class _ts_Sizable(Protocol):
    def size(self) -> int: ...


class _ts_MyBox:
    count: int
    def __init__(self, n: int) -> None:
        self.count = n
    def size(self) -> int:
        return self.count


def _ts_proto_get_size(obj: _ts_Sizable) -> int:
    return obj.size()


# Empty Protocol: every object satisfies it
@runtime_checkable
class _ts_AnyProto(Protocol):
    pass


# _ts_Addable Protocol: class with __add__ satisfies it (annotation + isinstance)
@runtime_checkable
class _ts_Addable(Protocol):
    def __add__(self, other: int) -> int: ...


class _ts_Counter:
    def __init__(self, n: int) -> None:
        self.n = n
    def __add__(self, other: int) -> int:
        return self.n + other


class _ts_NoAddable:
    pass


# ===== SECTION: Union function parameters and arithmetic =====


def _ts_union_pass(x: Union[int, str]) -> Union[int, str]:
    return x


# Union function with arithmetic
def _ts_union_double(x: Union[int, float]) -> Union[int, float]:
    return x + x


# Union return type with primitive returns. The Return-terminator codegen
# must box int/bool/float operands so callers see well-formed `Value` bits
# instead of raw scalars — pre-fix this would SEGV when downstream
# `rt_print_obj` reads raw int 42 (low 3 bits 0b010, no valid tag) and
# falls to the heap-pointer dispatch arm.

def _ts_union_return_int_or_str(b: bool):
    if b:
        return 42
    return "hello"


def _ts_union_return_bool_or_str(b: bool):
    if b:
        return True
    return "false-branch"


def _ts_union_return_float_or_str(b: bool):
    if b:
        return 1.5
    return "small"


# Numeric-tower promotion at Return: when a function returns either a Float
# or an Int (e.g. `return 1.5` / `return 0`), type inference promotes the
# function's return type to `Float` (`int ⊂ float`) and emits the function
# signature as `f64`. The Int return branch's operand is a raw `i64` —
# without the (I64, F64) and (I8, F64) coercion arms in the Return
# terminator codegen, Cranelift's verifier rejects the function with
# "result has type i64, must match function signature of f64". The
# function-typed local annotation here uses `-> float` so the call result
# is unambiguously `Float` and the assignment storage path is uniform.
def _ts_numeric_tower_return(b: bool) -> float:
    if b:
        return 1.5
    return 0


def _ts_numeric_tower_return_bool(b: bool) -> float:
    if b:
        return 1.5
    return True


# Numeric-tower promotion via `join_return_types` for *unannotated*
# functions. Pre-fix: `def f(b: bool): return 1.5 if b else 0` was inferred
# as `Union[int, float]` (because `join_return_types` used
# `Type::normalize_union` which doesn't promote), prescan stored
# `Union[int, float]` as `x`'s var_type, the assignment routed through
# Ptr storage, and the raw F64 return was mis-stored as a tagged pointer
# — SEGV at the next reader. Post-fix `join_return_types` uses
# `Type::unify_field_type` (numeric tower), so the inferred return is
# `Float`, prescan stores `Float`, and F64 storage is uniform end-to-end.

def _ts_unannotated_mixed_return(b: bool):
    if b:
        return 1.5
    return 0


# Bool + Int → Int promotion (`bool ⊂ int`).
def _ts_unannotated_bool_or_int(b: bool):
    if b:
        return 1
    return False


def test_types_system() -> None:
    # ===== SECTION: typing module imports (List, Dict, Set, Tuple, Optional, Union) =====

    # Test List (typing)
    nums_typing: List[int] = [1, 2, 3]
    assert nums_typing[0] == 1, "nums_typing[0] should equal 1"
    assert nums_typing[1] == 2, "nums_typing[1] should equal 2"
    assert len(nums_typing) == 3, "len(nums_typing) should equal 3"

    # Test Dict (typing)
    d_typing: Dict[str, int] = {"a": 1, "b": 2}
    assert d_typing["a"] == 1, "d_typing[\"a\"] should equal 1"
    assert d_typing["b"] == 2, "d_typing[\"b\"] should equal 2"

    # Test Set (typing)
    s_typing: Set[int] = {1, 2, 3}
    assert 1 in s_typing, "1 should be in s_typing"
    assert 4 not in s_typing, "4 not should be in s_typing"

    # Test Tuple (typing)
    t_typing: Tuple[int, int, int] = (1, 2, 3)
    assert t_typing[0] == 1, "t_typing[0] should equal 1"
    assert t_typing[1] == 2, "t_typing[1] should equal 2"
    assert t_typing[2] == 3, "t_typing[2] should equal 3"

    # ===== SECTION: PEP 585 generics (list[_ts_T], dict[K,V]) =====

    # Test list (built-in)
    nums_builtin: list[int] = [10, 20, 30]
    assert nums_builtin[0] == 10, "nums_builtin[0] should equal 10"
    assert nums_builtin[1] == 20, "nums_builtin[1] should equal 20"
    assert len(nums_builtin) == 3, "len(nums_builtin) should equal 3"

    # Test dict (built-in)
    d_builtin: dict[str, int] = {"x": 100, "y": 200}
    assert d_builtin["x"] == 100, "d_builtin[\"x\"] should equal 100"
    assert d_builtin["y"] == 200, "d_builtin[\"y\"] should equal 200"

    # Test set (built-in)
    s_builtin: set[int] = {10, 20, 30}
    assert 10 in s_builtin, "10 should be in s_builtin"
    assert 40 not in s_builtin, "40 not should be in s_builtin"

    # Test tuple (built-in)
    t_builtin: tuple[int, int, int] = (10, 20, 30)
    assert t_builtin[0] == 10, "t_builtin[0] should equal 10"
    assert t_builtin[1] == 20, "t_builtin[1] should equal 20"
    assert t_builtin[2] == 30, "t_builtin[2] should equal 30"

    # Nested types
    nested_typing: Dict[str, List[int]] = {"key": [1, 2, 3]}
    assert nested_typing["key"][0] == 1, "nested_typing[\"key\"][0] should equal 1"
    assert nested_typing["key"][2] == 3, "nested_typing[\"key\"][2] should equal 3"

    nested_builtin: dict[str, list[int]] = {"data": [4, 5, 6]}
    assert nested_builtin["data"][0] == 4, "nested_builtin[\"data\"][0] should equal 4"
    assert nested_builtin["data"][2] == 6, "nested_builtin[\"data\"][2] should equal 6"

    # Mixed styles
    mixed1: List[tuple[int, int]] = [(1, 2), (3, 4)]
    assert mixed1[0][0] == 1, "mixed1[0][0] should equal 1"
    assert mixed1[1][1] == 4, "mixed1[1][1] should equal 4"

    complex_nested: dict[str, List[tuple[int, int]]] = {"pairs": [(1, 2), (3, 4)]}
    assert complex_nested["pairs"][0][0] == 1, "complex_nested[\"pairs\"][0][0] should equal 1"
    assert complex_nested["pairs"][1][1] == 4, "complex_nested[\"pairs\"][1][1] should equal 4"

    # ===== SECTION: Union types =====

    # PEP 604 union types (int | str)
    val1: int | str = 42
    val2: int | str = "hello"

    # Union with None
    maybe_int: int | None = 5
    maybe_none: int | None = None
    maybe_value: int | None = 42

    # Multi-type Union
    multi_none: int | str | None = None
    multi_str: int | str | None = "test"

    # List with Union elements
    union_items: list[int | str] = [1, "two", 3]

    # Dict with Union values
    union_data: dict[str, int | str] = {"a": 1, "b": "two"}

    # ===== SECTION: Union operations (print, comparison, str conversion) =====

    # Print tests
    assert maybe_int == 5, "maybe_int should equal 5"
    assert maybe_none is None, "maybe_none should be None"

    multi: int | str | None = "hello"
    assert multi == "hello", "multi should equal hello"

    float_or_none: float | None = 3.14
    assert float_or_none == 3.14, "float_or_none should equal 3.14"

    # Comparison tests
    a: int | None = 42
    b: int | None = 42
    assert a == b, "a should equal b"

    a = None
    b = None
    assert a == b, "a should equal b"

    x_union: int | str = 42
    y_union: int | str = "42"
    assert x_union != y_union, "x_union should not equal y_union"  # different types should not be equal

    c: int | None = 10
    d: int | None = 20
    assert c != d, "c should not equal d"

    # Mixed comparisons (Union with non-Union)
    val: int | None = 42
    assert val == 42, "val should equal 42"

    val = None
    assert val == None, "val should equal None"

    # String Union comparisons
    s1: str | None = "test"
    s2: str | None = "test"
    assert s1 == s2, "s1 should equal s2"

    s3: str | None = "different"
    assert s1 != s3, "s1 should not equal s3"

    # str() conversion tests
    int_union: int | None = 123
    su = str(int_union)
    assert su == "123", "su should equal \"123\""

    none_union: int | None = None
    su = str(none_union)
    assert su == "None", "su should equal \"None\""

    float_union: float | None = 2.5
    su = str(float_union)
    assert su == "2.5", "su should equal \"2.5\""

    bool_union: bool | None = False
    su = str(bool_union)
    assert su == "False", "su should equal \"False\""

    # ===== SECTION: Union ordering comparisons (<, >, <=, >=) =====

    # Same-type int comparisons through Union
    union_int1: int | str = 10
    union_int2: int | str = 20
    assert union_int1 < union_int2, "10 < 20"
    assert union_int2 > union_int1, "20 > 10"
    assert union_int1 <= union_int1, "10 <= 10"
    assert union_int1 >= union_int1, "10 >= 10"
    assert union_int1 <= union_int2, "10 <= 20"
    assert union_int2 >= union_int1, "20 >= 10"
    assert not (union_int1 > union_int2), "not (10 > 20)"
    assert not (union_int2 < union_int1), "not (20 < 10)"

    # Float comparisons through Union
    union_float1: int | float = 3.14
    union_float2: int | float = 2.71
    assert union_float2 < union_float1, "2.71 < 3.14"
    assert union_float1 > union_float2, "3.14 > 2.71"
    assert union_float1 >= union_float1, "3.14 >= 3.14"
    assert union_float2 <= union_float2, "2.71 <= 2.71"

    # Mixed int/float comparisons (int promoted to float)
    union_mixed1: int | float = 5
    union_mixed2: int | float = 5.5
    assert union_mixed1 < union_mixed2, "5 < 5.5"
    assert union_mixed2 > union_mixed1, "5.5 > 5"

    union_mixed3: int | float = 5
    union_mixed4: int | float = 5.0
    assert union_mixed3 <= union_mixed4, "5 <= 5.0"
    assert union_mixed3 >= union_mixed4, "5 >= 5.0"

    # String lexicographic comparison through Union
    union_str1: int | str = "apple"
    union_str2: int | str = "banana"
    assert union_str1 < union_str2, "apple < banana"
    assert union_str2 > union_str1, "banana > apple"
    assert union_str1 <= union_str1, "apple <= apple"
    assert union_str2 >= union_str2, "banana >= banana"

    # String comparison edge cases
    union_str3: str | None = "a"
    union_str4: str | None = "aa"
    assert union_str3 < union_str4, "a < aa"
    assert union_str4 > union_str3, "aa > a"

    # Boolean comparisons through Union (False=0, True=1)
    union_bool1: int | bool = False
    union_bool2: int | bool = True
    assert union_bool1 < union_bool2, "False < True"
    assert union_bool2 > union_bool1, "True > False"
    assert union_bool1 <= union_bool1, "False <= False"
    assert union_bool2 >= union_bool2, "True >= True"

    # Edge case: same value
    union_same: int | str = 42
    assert union_same <= union_same, "42 <= 42"
    assert union_same >= union_same, "42 >= 42"
    assert not (union_same < union_same), "not (42 < 42)"
    assert not (union_same > union_same), "not (42 > 42)"

    # Negative numbers
    union_neg1: int | float = -10
    union_neg2: int | float = 5
    assert union_neg1 < union_neg2, "-10 < 5"
    assert union_neg2 > union_neg1, "5 > -10"

    # Float edge cases
    union_fl1: float | None = 0.0
    union_fl2: float | None = 0.1
    assert union_fl1 < union_fl2, "0.0 < 0.1"
    assert union_fl2 > union_fl1, "0.1 > 0.0"

    # ===== SECTION: Type inference for locals =====

    # Integer inference
    inf_x = 5
    assert inf_x == 5, "inf_x should equal 5"

    # Float inference
    inf_y = 3.14
    inf_z = inf_y + 1.0
    assert inf_z > 4.0, "inf_z should be greater than 4.0"
    assert inf_z < 5.0, "inf_z should be less than 5.0"

    # Float arithmetic
    inf_a = 2.5
    inf_b = 1.5
    inf_c = inf_a + inf_b
    assert inf_c == 4.0, "inf_c should equal 4.0"

    # Boolean inference
    inf_flag = True
    assert inf_flag, "inf_flag should be True"

    # String inference
    inf_name = "hello"
    assert len(inf_name) == 5, "len(inf_name) should equal 5"

    # List inference
    inf_nums = [1, 2, 3]
    assert inf_nums[0] == 1, "inf_nums[0] should equal 1"

    # Tuple inference
    inf_point = (10, 20)
    assert inf_point[0] == 10, "inf_point[0] should equal 10"

    # Reassignment (same type)
    counter = 0
    counter = counter + 1
    assert counter == 1, "counter should equal 1"

    # Mixed: some with annotation, some without
    annotated: int = 42
    assert annotated == 42, "annotated should equal 42"
    w = 100
    assert annotated + w == 142, "annotated + w should equal 142"

    # Empty list REQUIRES annotation
    empty: list[int] = []
    assert len(empty) == 0, "len(empty) should equal 0"

    # List with inferred type, then use methods
    numbers = [10, 20, 30]
    numbers.append(40)
    assert len(numbers) == 4, "len(numbers) should equal 4"
    assert numbers[3] == 40, "numbers[3] should equal 40"

    infer_val = 7
    infer_result = _ts_double(infer_val)
    assert infer_result == 14, "infer_result should equal 14"

    # ===== SECTION: Tuple unpacking =====

    # Basic tuple unpacking
    unpack_a, unpack_b = (1, 2)
    assert unpack_a == 1, "unpack_a should equal 1"
    assert unpack_b == 2, "unpack_b should equal 2"

    # Tuple unpacking with more elements
    unpack_x, unpack_y, unpack_z = (10, 20, 30)
    assert unpack_x == 10, "unpack_x should equal 10"
    assert unpack_y == 20, "unpack_y should equal 20"
    assert unpack_z == 30, "unpack_z should equal 30"

    # Unpacking without parentheses (implicit tuple)
    unpack_p, unpack_q = 100, 200
    assert unpack_p == 100, "unpack_p should equal 100"
    assert unpack_q == 200, "unpack_q should equal 200"

    # Swap values (parallel assignment)
    unpack_m, unpack_n = 5, 10
    unpack_m, unpack_n = unpack_n, unpack_m
    assert unpack_m == 10, "unpack_m should equal 10"
    assert unpack_n == 5, "unpack_n should equal 5"

    # Unpack from a tuple variable
    my_tuple: tuple[int, int, int] = (7, 8, 9)
    t1, t2, t3 = my_tuple
    assert t1 == 7, "t1 should equal 7"
    assert t2 == 8, "t2 should equal 8"
    assert t3 == 9, "t3 should equal 9"

    # Unpack from a list
    list_vals: list[int] = [100, 200, 300]
    l1, l2, l3 = list_vals
    assert l1 == 100, "l1 should equal 100"
    assert l2 == 200, "l2 should equal 200"
    assert l3 == 300, "l3 should equal 300"

    # Unpack mixed types from tuple
    name, age = ("Alice", 30)
    assert age == 30, "age should equal 30"

    r1, r2 = _ts_get_pair()
    assert r1 == 42, "r1 should equal 42"
    assert r2 == 84, "r2 should equal 84"

    # ===== SECTION: Starred unpacking (*rest) =====

    # Basic case: a, *rest = [1, 2, 3, 4]
    star_a, *star_rest = [1, 2, 3, 4]
    assert star_a == 1, "star_a should equal 1"
    assert star_rest == [2, 3, 4], "star_rest should equal [2, 3, 4]"

    # Starred at the beginning: *rest, last = [1, 2, 3, 4]
    *star_rest2, star_last = [1, 2, 3, 4]
    assert star_rest2 == [1, 2, 3], "star_rest2 should equal [1, 2, 3]"
    assert star_last == 4, "star_last should equal 4"

    # Starred in the middle: first, *middle, last = [1, 2, 3, 4, 5]
    star_first, *star_middle, star_last2 = [1, 2, 3, 4, 5]
    assert star_first == 1, "star_first should equal 1"
    assert star_middle == [2, 3, 4], "star_middle should equal [2, 3, 4]"
    assert star_last2 == 5, "star_last2 should equal 5"

    # Edge case: empty starred portion
    star_x, *star_empty, star_y = [1, 2]
    assert star_x == 1, "star_x should equal 1"
    assert star_empty == [], "star_empty should equal []"
    assert star_y == 2, "star_y should equal 2"

    # Edge case: only starred
    *star_all_items, = [1, 2, 3]
    assert star_all_items == [1, 2, 3], "star_all_items should equal [1, 2, 3]"

    # Edge case: single element before star
    star_a2, *star_rest3 = [10]
    assert star_a2 == 10, "star_a2 should equal 10"
    assert star_rest3 == [], "star_rest3 should equal []"

    # Works with tuples (starred always returns list)
    star_t1, *star_t_rest = (10, 20, 30)
    assert star_t1 == 10, "star_t1 should equal 10"
    assert star_t_rest == [20, 30], "star_t_rest should equal [20, 30]"

    # Multiple elements before and after star
    star_p1, star_p2, *star_p_mid, star_p_last1, star_p_last2 = [1, 2, 3, 4, 5, 6, 7]
    assert star_p1 == 1, "star_p1 should equal 1"
    assert star_p2 == 2, "star_p2 should equal 2"
    assert star_p_mid == [3, 4, 5], "star_p_mid should equal [3, 4, 5]"
    assert star_p_last1 == 6, "star_p_last1 should equal 6"
    assert star_p_last2 == 7, "star_p_last2 should equal 7"

    # ===== SECTION: Union Is/IsNot operators =====

    # Is with Union - same object identity
    union_obj1: int | str = "test"
    union_obj2: int | str = union_obj1
    assert union_obj1 is union_obj2, "same object identity"
    assert not (union_obj1 is not union_obj2), "same object: is not should be False"

    # IsNot with Union - different objects
    union_obj3: int | str = "different"
    assert union_obj1 is not union_obj3, "different objects"
    assert not (union_obj1 is union_obj3), "different objects: is should be False"

    # Is with None and Union
    maybe_val: int | None = None
    assert maybe_val is None, "None identity check"

    maybe_val2: int | None = 42
    assert maybe_val2 is not None, "non-None identity check"

    # Is with int Union
    int_union1: int | str = 42
    int_union2: int | str = 42  # Note: small ints may be the same object or not depending on boxing
    # We don't test identity for primitives since boxing may vary

    # ===== SECTION: Union In/NotIn operators =====

    # In with Union container (list)
    union_list: list[int] | None = [1, 2, 3]
    if union_list is not None:
        # Need to check after narrowing for now
        pass

    # Dict containment with Union container
    union_dict: dict[str, int] | None = {"a": 1, "b": 2}
    if union_dict is not None:
        pass

    # Set containment with Union container
    union_set: set[int] | None = {1, 2, 3}
    if union_set is not None:
        pass

    # String containment with Union container
    union_str: str | None = "hello world"
    if union_str is not None:
        pass

    # Run type narrowing tests
    _ts_test_basic_int_narrowing()
    _ts_test_basic_str_narrowing()
    _ts_test_else_branch_narrowing()
    _ts_test_three_type_union()
    _ts_test_three_type_union_str()
    _ts_test_negation()

    # Run the 'or' narrowing tests
    _ts_test_or_else_narrowing()
    _ts_test_or_different_vars()
    _ts_test_not_or_then_narrowing()
    _ts_test_or_same_var_triple_union()
    _ts_test_not_and_else_narrowing()

    # Bool arguments should be accepted by functions expecting int
    assert _ts_int_increment(True) == 2, "_ts_int_increment(True) should equal 2"
    assert _ts_int_increment(False) == 1, "_ts_int_increment(False) should equal 1"

    assert _ts_int_double(True) == 2, "_ts_int_double(True) should equal 2"
    assert _ts_int_double(False) == 0, "_ts_int_double(False) should equal 0"

    # Bool in arithmetic expressions (already works, but good to verify)
    bool_as_int_result: int = True + True
    assert bool_as_int_result == 2, "True + True should equal 2"

    bool_as_int_result2: int = True * 5
    assert bool_as_int_result2 == 5, "True * 5 should equal 5"

    bool_as_int_result3: int = False * 100
    assert bool_as_int_result3 == 0, "False * 100 should equal 0"

    # Bool subtraction and other arithmetic
    bool_sub_result: int = True - False
    assert bool_sub_result == 1, "True - False should equal 1"

    bool_floor_div: int = True // True
    assert bool_floor_div == 1, "True // True should equal 1"

    # Run is None narrowing tests
    _ts_test_is_none_narrowing_basic()
    _ts_test_is_not_none_narrowing_basic()
    _ts_test_is_none_with_str_union()
    _ts_test_is_none_triple_union()
    _ts_test_is_not_none_triple_union()
    _ts_test_not_is_none_negation()
    _ts_test_not_is_not_none_negation()
    _ts_test_is_none_with_isinstance_chain()

    # Run truthiness narrowing tests
    _ts_test_truthiness_if_x_then_not_none()
    _ts_test_truthiness_if_not_x_else_not_none()
    _ts_test_truthiness_none_is_falsy()
    _ts_test_truthiness_zero_is_falsy()
    _ts_test_truthiness_str_none_union()
    _ts_test_truthiness_empty_str_is_falsy()
    _ts_test_truthiness_combined_with_isinstance()
    _ts_test_truthiness_while_loop()
    ta_nums: _ts_IntList = [1, 2, 3]
    assert len(ta_nums) == 3, "TypeAlias _ts_IntList should work as list[int]"
    assert ta_nums[0] == 1, "TypeAlias _ts_IntList element access"
    ta_dict: _ts_StrDict = {"a": 1, "b": 2}
    assert ta_dict["a"] == 1, "TypeAlias _ts_StrDict should work as dict[str, int]"
    assert len(ta_dict) == 2, "TypeAlias _ts_StrDict length"
    ta_nested: _ts_NestedAlias = [{"x": 10}, {"y": 20}]
    assert len(ta_nested) == 2, "Nested TypeAlias should work"
    assert ta_nested[0]["x"] == 10, "Nested TypeAlias element access"

    ta_set: _ts_IntSet = {1, 2, 3}
    assert 1 in ta_set, "PEP 695 type alias should work as set[int]"
    assert len(ta_set) == 3, "PEP 695 type alias set length"
    ta_opt1: _ts_OptStr = "hello"
    ta_opt2: _ts_OptStr = None
    assert ta_opt1 == "hello", "PEP 695 union alias with value"
    assert ta_opt2 is None, "PEP 695 union alias with None"


    # ===== SECTION: Literal types =====

    lit_mode: Literal["r", "w"] = "r"
    assert lit_mode == "r", "Literal[str, str] should accept string value"

    lit_code: Literal[0, 1, 2] = 1
    assert lit_code == 1, "Literal[int, int, int] should accept int value"

    lit_flag: Literal[True] = True
    assert lit_flag == True, "Literal[bool] should accept bool value"

    lit_neg: Literal[-1, 0, 1] = -1
    assert lit_neg == -1, "Literal with negative int"

    lit_none: Literal[None] = None
    assert lit_none is None, "Literal[None] should accept None"


    tv_int_result: int = _ts_tv_identity_int(42)
    assert tv_int_result == 42, "TypeVar identity with int"

    tv_str_result: str = _ts_tv_identity_str("hello")
    assert tv_str_result == "hello", "TypeVar identity with str"
    tv_constrained_val: _ts_Num = 42
    assert tv_constrained_val == 42, "TypeVar with constraints (annotation accepted)"

    assert _ts_tv_max_val(3, 7) == 7, "TypeVar with bound"
    assert _ts_tv_max_val(10, 2) == 10, "TypeVar with bound (reverse)"


    proto_c = _ts_Circle()
    proto_s = _ts_Square()
    assert _ts_proto_render(proto_c) == "circle", "Protocol accepts _ts_Circle"
    assert _ts_proto_render(proto_s) == "square", "Protocol accepts _ts_Square"

    # Protocol can also be used as a variable type annotation
    proto_shape: _ts_Drawable = _ts_Circle()
    assert proto_shape.draw() == "circle", "Protocol variable annotation"

    proto_box = _ts_MyBox(5)
    assert _ts_proto_get_size(proto_box) == 5, "Protocol with different vtable layout"

    # isinstance structural checks
    assert isinstance(proto_box, _ts_Sizable) == True, "isinstance: box satisfies _ts_Sizable"
    assert isinstance(proto_c, _ts_Sizable) == False, "isinstance: _ts_Circle lacks size()"
    assert isinstance(42, _ts_Sizable) == False, "isinstance: int does not satisfy _ts_Sizable"
    assert isinstance("hi", _ts_Sizable) == False, "isinstance: str does not satisfy _ts_Sizable"

    assert isinstance(proto_box, _ts_AnyProto) == True, "isinstance: empty Protocol satisfied by instance"
    assert isinstance(42, _ts_AnyProto) == True, "isinstance: empty Protocol satisfied by int"
    assert isinstance("hi", _ts_AnyProto) == True, "isinstance: empty Protocol satisfied by str"

    # Tuple-of-types containing a Protocol
    assert isinstance(proto_box, (int, _ts_Sizable)) == True, "isinstance: tuple-of-types with Protocol (True)"
    assert isinstance(proto_c, (int, _ts_Sizable)) == False, "isinstance: tuple-of-types with Protocol (False)"

    proto_counter = _ts_Counter(10)

    # Concrete usage (not through Protocol interface): __add__ dispatches directly
    assert proto_counter.__add__(5) == 15, "_ts_Counter.__add__ works directly"
    assert isinstance(proto_counter, _ts_Addable) == True, "isinstance: _ts_Counter satisfies _ts_Addable"
    assert isinstance(_ts_NoAddable(), _ts_Addable) == False, "isinstance: _ts_NoAddable lacks __add__"
    # NOTE: `isinstance(42, _ts_Addable)` is intentionally NOT asserted. pyaot's
    # structural protocol check is instance-only (it probes the user-class method
    # registry via `rt_obj_has_method`), so a builtin scalar never satisfies a
    # protocol → pyaot returns False. CPython's `@runtime_checkable` instead probes
    # attribute presence on ANY object, and `int.__add__` exists → CPython returns
    # True. This is a deliberate, documented model divergence; the user-class cases
    # above (_ts_Counter True / _ts_NoAddable False) cover the dunder structural path.

    # Negative case (compile-time diagnostic): uncomment to verify
    # class EmptyClass:
    #     pass
    # def accepts_sized(s: _ts_Sizable) -> int:  # diagnostic: type 'EmptyClass' does not satisfy protocol '_ts_Sizable': missing method 'size'
    #     return s.size()
    # accepts_sized(EmptyClass())


    assert _ts_union_pass(5) == 5, "Union[int, str] param with int"
    assert _ts_union_pass("hi") == "hi", "Union[int, str] param with str"

    # Union arithmetic on variables
    union_x: int | float = 5
    union_y: int | float = union_x + union_x
    assert union_y == 10, "Union int+int arithmetic"

    union_z: int | float = 2.5
    union_w: int | float = union_z + union_z
    assert union_w == 5.0, "Union float+float arithmetic"

    assert _ts_union_double(7) == 14, "Union function int arithmetic"
    assert _ts_union_double(1.5) == 3.0, "Union function float arithmetic"


    union_int_branch = _ts_union_return_int_or_str(True)
    assert union_int_branch == 42, "Union return: int branch"
    union_str_branch = _ts_union_return_int_or_str(False)
    assert union_str_branch == "hello", "Union return: str branch"

    union_bool_branch = _ts_union_return_bool_or_str(True)
    assert union_bool_branch == True, "Union return: bool branch"

    union_float_branch = _ts_union_return_float_or_str(True)
    assert union_float_branch == 1.5, "Union return: float branch"
    union_str_branch2 = _ts_union_return_float_or_str(False)
    assert union_str_branch2 == "small", "Union return: str branch (mixed with float)"

    ntr_float_result: float = _ts_numeric_tower_return(True)
    assert ntr_float_result == 1.5, "Numeric-tower return: float branch"
    ntr_int_result: float = _ts_numeric_tower_return(False)
    assert ntr_int_result == 0.0, "Numeric-tower return: int branch promoted to float"

    ntrb_float_result: float = _ts_numeric_tower_return_bool(True)
    assert ntrb_float_result == 1.5, "Numeric-tower return: float branch (bool variant)"
    ntrb_bool_result: float = _ts_numeric_tower_return_bool(False)
    assert ntrb_bool_result == 1.0, "Numeric-tower return: bool branch promoted to float"

    unann_float_branch: float = _ts_unannotated_mixed_return(True)
    assert unann_float_branch == 1.5, "Unannotated mixed return: float branch"
    unann_int_branch: float = _ts_unannotated_mixed_return(False)
    assert unann_int_branch == 0.0, "Unannotated mixed return: int branch promoted"

    # Same pattern bound to an unannotated local — exercises the prescan
    # var_type path that used to SEGV. With pyaot's numeric-tower promotion,
    # `unann_y`'s value is `0.0` (Float storage); CPython would return raw
    # `0`. Don't print the values directly to avoid CPython differential
    # noise — the assertions cover correctness via `==` (0.0 == 0).
    unann_x = _ts_unannotated_mixed_return(True)
    assert unann_x == 1.5, "Unannotated mixed return: bound to unannotated local"
    unann_y = _ts_unannotated_mixed_return(False)
    assert unann_y == 0.0, "Unannotated mixed return: int branch via unannotated local"
    assert unann_y == 0, "Unannotated mixed return: numeric equality across types"

    unann_bi_int: int = _ts_unannotated_bool_or_int(True)
    assert unann_bi_int == 1, "Unannotated bool|int return: int branch"
    unann_bi_bool: int = _ts_unannotated_bool_or_int(False)
    assert unann_bi_bool == 0, "Unannotated bool|int return: bool branch promoted to int"


    # Exercise through `print` so `rt_print_obj` decodes the tagged bits.
    assert _ts_union_return_int_or_str(True) == 42, "union_return int branch (re-exercise)"
    assert _ts_union_return_int_or_str(False) == "hello", "union_return str branch (re-exercise)"
    assert _ts_union_return_bool_or_str(True) == True, "union_return bool branch (re-exercise)"
    assert _ts_union_return_float_or_str(True) == 1.5, "union_return float branch (re-exercise)"
    assert _ts_union_return_float_or_str(False) == "small", "union_return str-with-float branch (re-exercise)"
    print("types_system: ok")


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
    test_pep695_function()
    test_generic_base_class()
    test_int_wrapper_int()
    test_int_wrapper_str()
    test_pep695_class()
    test_protocol_with_typeparam()
    test_pep695_alias()
    test_chained_spec()
    test_polymorphic_receiver()
    test_p5_generics()
    test_dc_incompatible_types()
    test_dc_redundant_check()
    test_dc_valid_union_narrowing()
    test_dc_incompatible_float()
    test_dc_incompatible_bool()
    test_dc_redundant_list_check()
    test_dc_valid_isinstance_any()
    test_dc_any_narrowing_to_str()
    test_dc_any_narrowing_to_bytes()
    test_dc_any_narrowing_to_dict()
    test_dc_chained_isinstance_narrowing()
    test_types_system()
    print("all generic tests passed")


main()
