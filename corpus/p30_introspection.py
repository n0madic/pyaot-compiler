# Introspection builtins `getattr` / `setattr` / `hasattr` / `issubclass`, §5.
#
# All four collapse onto existing machinery — ZERO runtime changes — because
# every shape here is on a CONCRETE class instance / user-class name:
#   * `getattr(o, "x")`  ≡ `o.x`        — frontend desugar onto Attribute read
#   * `setattr(o, "x", v)` ≡ `o.x = v`  — frontend desugar onto SetAttr write
#   * `hasattr(o, "x")`                 — compile-time Bool from o's ClassInfo
#   * `issubclass(A, B)`               — compile-time Bool via the C3-MRO check
# A string-literal name argument is required (dynamic getattr is out of scope);
# `issubclass` takes user-class names (builtin-type / tuple forms are rejected).
#
# `==` asserts are the spec (Principle 9); prints feed the differential harness.


# ===== a class hierarchy for issubclass (C3-MRO check) =====
class Animal:
    def __init__(self, name: str) -> None:
        self.name = name

    def speak(self) -> str:
        return "..."


class Dog(Animal):
    def speak(self) -> str:
        return "woof"


class Cat(Animal):
    def speak(self) -> str:
        return "meow"


# self-subclass (reflexive), direct subclass, and the False cases (siblings /
# child-vs-parent reversed).
assert issubclass(Dog, Animal) is True
assert issubclass(Cat, Animal) is True
assert issubclass(Animal, Animal) is True
assert issubclass(Dog, Dog) is True
assert issubclass(Dog, Cat) is False
assert issubclass(Cat, Dog) is False
assert issubclass(Animal, Dog) is False
print(issubclass(Dog, Animal))   # True
print(issubclass(Dog, Cat))      # False
print(issubclass(Animal, Animal))  # True


# ===== hasattr on a concrete instance (present field, present method, absent) =====
class HasAttrTest:
    def __init__(self) -> None:
        self.x = 10
        self.name = "hat"

    def method(self) -> int:
        return self.x


hat = HasAttrTest()
assert hasattr(hat, "x") is True        # present field
assert hasattr(hat, "name") is True     # present field
assert hasattr(hat, "method") is True   # present method
assert hasattr(hat, "missing") is False  # absent name
assert hasattr(hat, "xyz") is False     # absent name
print(hasattr(hat, "x"))        # True
print(hasattr(hat, "method"))   # True
print(hasattr(hat, "missing"))  # False


# ===== setattr / getattr round-trips on a concrete instance =====
hat2 = HasAttrTest()
assert getattr(hat2, "x") == 10
setattr(hat2, "x", 42)
assert getattr(hat2, "x") == 42
assert hat2.x == 42  # the write is visible to direct attribute access too

setattr(hat2, "name", "world")
assert getattr(hat2, "name") == "world"
assert hat2.name == "world"
# setattr evaluates to None
assert setattr(hat2, "x", 7) is None
assert hat2.x == 7
print(getattr(hat2, "x"))     # 7
print(getattr(hat2, "name"))  # world


# ===== cross with already-green features (f-string, arithmetic) =====
d = Dog("Rex")
assert getattr(d, "name") == "Rex"
# getattr result used in arithmetic
total = getattr(hat2, "x") + 100
assert total == 107
print(total)  # 107
# getattr result inside an f-string
print(f"name={getattr(d, 'name')} x={getattr(hat2, 'x')}")  # name=Rex x=7

# getattr on a polymorphic instance, crossed with a compile-time issubclass gate
if issubclass(Dog, Animal):
    animals = [Dog("D"), Cat("C")]
    for a in animals:
        print(f"{getattr(a, 'name')}: {a.speak()}")  # D: woof / C: meow

print("All introspection tests passed!")
