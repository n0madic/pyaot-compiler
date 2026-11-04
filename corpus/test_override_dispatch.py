# Override dispatch (KNOWN_BUGS Phase 3, "Root B"): a member reached through a
# receiver statically typed as a base class must resolve to the definition of
# the receiver's ACTUAL class — methods with a different signature, dunders,
# properties, static/class methods, class attributes — and class members are
# inherited along the MRO.


class Base:
    tag = "base"

    def __init__(self) -> None:
        self.x = 1

    def m(self, a: int) -> int:
        return a + 1

    def val(self):
        return 1

    def __len__(self) -> int:
        return 0

    def __str__(self) -> str:
        return "Base"

    def __lt__(self, o: "Base") -> bool:
        return False

    @property
    def p(self) -> int:
        return 10

    @staticmethod
    def sm() -> str:
        return "Base.sm"

    @classmethod
    def cm(cls) -> str:
        return cls.tag

    @classmethod
    def make(cls):
        return cls()


class Sub(Base):
    tag = "sub"

    def __init__(self) -> None:
        self.x = 1
        self.y = 0

    def m(self, a: int, b: int = 5) -> int:
        return a + b

    def val(self):
        return 2.5

    def __len__(self) -> int:
        return 3

    def __str__(self) -> str:
        return "Sub"

    def __lt__(self, o: "Base") -> bool:
        return True

    @property
    def p(self) -> int:
        return 20

    @staticmethod
    def sm() -> str:
        return "Sub.sm"


class Redef:
    def f(self) -> str:
        return "first"

    def f(self) -> str:
        return "second"

    @staticmethod
    def s() -> str:
        return "static"

    k = 7


class RedefChild(Redef):
    pass


class E1(Exception):
    pass


class Mixin:
    pass


class E2(Mixin, E1):
    pass


class P:
    pass


class Q:
    pass


class R(P, Q):
    pass


def describe(b: Base) -> list[str]:
    return [
        str(b.m(1)),
        str(b.val()),
        str(len(b)),
        str(b),
        str(b < b),
        str(b.p),
        b.tag,
        getattr(b, "tag"),
        b.sm(),
        b.cm(),
    ]


def poke(b: Base) -> None:
    b.y = 3


def test_override_dispatch() -> None:
    assert describe(Base()) == ["2", "1", "0", "Base", "False", "10", "base", "base", "Base.sm", "base"]
    assert describe(Sub()) == ["6", "2.5", "3", "Sub", "True", "20", "sub", "sub", "Sub.sm", "sub"]


def test_classmethod_cls() -> None:
    assert isinstance(Sub.make(), Sub)
    assert not isinstance(Base.make(), Sub)
    assert Sub.cm() == "sub"
    assert Base.cm() == "base"


def test_instance_shadows_class_attr() -> None:
    s = Sub()
    other = Sub()
    s.tag = "inst"
    assert s.tag == "inst"
    assert other.tag == "sub"
    assert Sub.tag == "sub"
    assert Base.tag == "base"


def test_inherited_members() -> None:
    assert Redef().f() == "second"
    assert RedefChild.s() == "static"
    assert RedefChild().s() == "static"
    assert RedefChild.k == 7
    assert RedefChild().k == 7


def test_multiple_inheritance() -> None:
    caught = ""
    try:
        raise E2("x")
    except E1 as e:
        caught = "E1 " + str(e)
    assert caught == "E1 x"
    assert isinstance(R(), Q)
    assert isinstance(R(), P)


def test_subclass_only_field() -> None:
    s = Sub()
    poke(s)
    assert s.y == 3


test_override_dispatch()
test_classmethod_cls()
test_instance_shadows_class_attr()
test_inherited_members()
test_multiple_inheritance()
test_subclass_only_field()
print("test_override_dispatch passed!")
