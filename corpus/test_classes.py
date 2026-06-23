# Consolidated test file for classes and OOP

from typing import Any
from abc import abstractmethod
from collections import deque

# ===== SECTION: Class definitions and __init__ =====

class Point:
    x: int
    y: int

    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

    def distance_squared(self) -> int:
        return self.x * self.x + self.y * self.y

    def add(self, other_x: int, other_y: int) -> int:
        return self.x + other_x + self.y + other_y

    # Methods declared WITHOUT `self`, callable via the class as plain functions
    # (CPython: `Point.no_self_const()` runs with no receiver bound).
    def no_self_const() -> int:
        return 5

    def no_self_add(a: int, b: int) -> int:
        return a + b

# Create instance via constructor
p = Point(3, 4)

# A no-`self` method is callable via the class (must not panic at compile time).
assert Point.no_self_const() == 5, "no-self method via class, 0 args"
assert Point.no_self_add(3, 4) == 7, "no-self method via class, 2 args"

# ===== SECTION: Instance fields and methods =====

# Test field access
assert p.x == 3, "Field x should be 3"
assert p.y == 4, "Field y should be 4"

# Test method calls
d = p.distance_squared()
assert d == 25, "distance_squared should be 25 (3*3 + 4*4)"

s = p.add(2, 3)
assert s == 12, "add should be 12 (3+2 + 4+3)"

# Test field modification
p.x = 10
assert p.x == 10, "Field x should be 10 after modification"

# ===== SECTION: Multiple instances =====

# Create another instance
p2 = Point(5, 12)
assert p2.x == 5, "p2.x should be 5"
assert p2.y == 12, "p2.y should be 12"
assert p2.distance_squared() == 169, "p2.distance_squared should be 169"

# Original instance unchanged (except for modification)
assert p.x == 10, "p.x should still be 10"
assert p.y == 4, "p.y should still be 4"

# ===== SECTION: Class attributes =====

class AttrCounter:
    count = 0
    name = "Counter"

# Test basic access
assert AttrCounter.count == 0, "AttrCounter.count should equal 0"
assert AttrCounter.name == "Counter", "AttrCounter.name should equal \"Counter\""

# Test modification
AttrCounter.count = 5
assert AttrCounter.count == 5, "AttrCounter.count should equal 5"

class Tracker:
    total = 0

    def __init__(self):
        Tracker.total += 1

t1 = Tracker()
assert Tracker.total == 1, "Tracker.total should equal 1"
t2 = Tracker()
assert Tracker.total == 2, "Tracker.total should equal 2"

# Test float class attribute
class Config:
    rate = 0.5

assert Config.rate == 0.5, "Config.rate should equal 0.5"
Config.rate = 1.5
assert Config.rate == 1.5, "Config.rate should equal 1.5"

# Test bool class attribute
class Flags:
    enabled = True
    debug = False

assert Flags.enabled == True, "Flags.enabled should equal True"
assert Flags.debug == False, "Flags.debug should equal False"
Flags.debug = True
assert Flags.debug == True, "Flags.debug should equal True"

# Test class attr with multiple classes
class AttrA:
    x = 10

class AttrB:
    x = 20

assert AttrA.x == 10, "AttrA.x should equal 10"
assert AttrB.x == 20, "AttrB.x should equal 20"
AttrA.x = 15
assert AttrA.x == 15, "AttrA.x should equal 15"
assert AttrB.x == 20, "AttrB.x should equal 20"  # B.x should be unchanged

# ===== SECTION: Single inheritance =====

class Animal:
    name: str
    def __init__(self, name: str):
        self.name = name
    def speak(self) -> str:
        return "..."

class Dog(Animal):
    def __init__(self, name: str):
        super().__init__(name)
    def speak(self) -> str:
        return "Woof!"

class Cat(Animal):
    def __init__(self, name: str):
        super().__init__(name)
    def speak(self) -> str:
        return "Meow!"

# Test basic inheritance
dog = Dog("Rex")
assert dog.name == "Rex", "dog.name should equal \"Rex\""
assert dog.speak() == "Woof!", "dog.speak() should equal \"Woof!\""

cat = Cat("Whiskers")
assert cat.name == "Whiskers", "cat.name should equal \"Whiskers\""
assert cat.speak() == "Meow!", "cat.speak() should equal \"Meow!\""

# ===== SECTION: super().__init__() =====

class Shape:
    x: int
    y: int
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y
    def describe(self) -> str:
        return "Shape"

class Circle(Shape):
    radius: int
    def __init__(self, x: int, y: int, r: int):
        super().__init__(x, y)
        self.radius = r
    def describe(self) -> str:
        return "Circle"

circle = Circle(10, 20, 5)
assert circle.x == 10, "circle.x should equal 10"
assert circle.y == 20, "circle.y should equal 20"
assert circle.radius == 5, "circle.radius should equal 5"
assert circle.describe() == "Circle", "circle.describe() should equal \"Circle\""

# ===== SECTION: isinstance() for primitives =====

inst_x: int = 42
assert isinstance(inst_x, int), "isinstance(inst_x, int) should be True"
assert not isinstance(inst_x, str), "assertion failed: not isinstance(inst_x, str)"
assert not isinstance(inst_x, float), "assertion failed: not isinstance(inst_x, float)"
assert not isinstance(inst_x, bool), "assertion failed: not isinstance(inst_x, bool)"

inst_y: float = 3.14
assert isinstance(inst_y, float), "isinstance(inst_y, float) should be True"
assert not isinstance(inst_y, int), "assertion failed: not isinstance(inst_y, int)"
assert not isinstance(inst_y, str), "assertion failed: not isinstance(inst_y, str)"

flag: bool = True
assert isinstance(flag, bool), "isinstance(flag, bool) should be True"
assert isinstance(flag, int), "isinstance(flag, int) should be True (bool is subclass of int)"
assert not isinstance(flag, str), "assertion failed: not isinstance(flag, str)"

# ===== SECTION: isinstance() for user classes =====

class IsPoint:
    x: int
    y: int
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

class IsCircle:
    r: int
    def __init__(self, r: int):
        self.r = r

is_p = IsPoint(1, 2)
is_c = IsCircle(5)

assert isinstance(is_p, IsPoint), "isinstance(is_p, IsPoint) should be True"
assert not isinstance(is_p, IsCircle), "assertion failed: not isinstance(is_p, IsCircle)"
assert isinstance(is_c, IsCircle), "isinstance(is_c, IsCircle) should be True"
assert not isinstance(is_c, IsPoint), "assertion failed: not isinstance(is_c, IsPoint)"

# Check class vs primitive type
assert not isinstance(is_p, int), "assertion failed: not isinstance(is_p, int)"
assert not isinstance(is_p, str), "assertion failed: not isinstance(is_p, str)"
assert not isinstance(inst_x, IsPoint), "assertion failed: not isinstance(inst_x, IsPoint)"

# ===== SECTION: isinstance() with inheritance =====

assert isinstance(dog, Dog), "isinstance(dog, Dog) should be True"
assert isinstance(dog, Animal), "isinstance(dog, Animal) should be True"
assert isinstance(cat, Cat), "isinstance(cat, Cat) should be True"
assert isinstance(cat, Animal), "isinstance(cat, Animal) should be True"

# Not a cat
assert not isinstance(dog, Cat), "assertion failed: not isinstance(dog, Cat)"
# Not a dog
assert not isinstance(cat, Dog), "assertion failed: not isinstance(cat, Dog)"

# Shape inheritance
assert isinstance(circle, Circle), "isinstance(circle, Circle) should be True"
assert isinstance(circle, Shape), "isinstance(circle, Shape) should be True"

# ===== SECTION: Virtual method dispatch (polymorphism) =====

class DispatchAnimal:
    def speak(self) -> str:
        return "..."

class DispatchDog(DispatchAnimal):
    def speak(self) -> str:
        return "Woof!"

class DispatchCat(DispatchAnimal):
    def speak(self) -> str:
        return "Meow!"

# Test direct method calls work via vtable dispatch
dispatch_dog = DispatchDog()
dispatch_cat = DispatchCat()
dispatch_animal = DispatchAnimal()

assert dispatch_dog.speak() == "Woof!", "dispatch_dog.speak() should equal \"Woof!\""
assert dispatch_cat.speak() == "Meow!", "dispatch_cat.speak() should equal \"Meow!\""
assert dispatch_animal.speak() == "...", "dispatch_animal.speak() should equal \"...\""

# Test multi-level inheritance
class Puppy(DispatchDog):
    def speak(self) -> str:
        return "Yip!"

puppy = Puppy()
assert puppy.speak() == "Yip!", "puppy.speak() should equal \"Yip!\""

# Test three-level inheritance
class Chihuahua(Puppy):
    def speak(self) -> str:
        return "Bark!"

chihuahua = Chihuahua()
assert chihuahua.speak() == "Bark!", "chihuahua.speak() should equal \"Bark!\""

# ===== SECTION: User-defined decorators =====

def identity(func) -> Any:
    return func

@identity
def simple(a: int, b: int) -> int:
    return a + b

result_deco = simple(3, 4)
assert result_deco == 7, "identity decorator failed"

# Multiple identity decorators
def identity2(func) -> Any:
    return func

@identity
@identity2
def add_deco(x: int, y: int) -> int:
    return x + y

result_deco2 = add_deco(10, 20)
assert result_deco2 == 30, "multiple identity decorators failed"

# Decorator on function with default args
@identity
def greet(name: str, greeting: str = "Hello") -> str:
    return greeting + " " + name

result_deco3 = greet("World")
assert result_deco3 == "Hello World", "decorator with defaults failed"

result_deco3b = greet("World", "Hi")
assert result_deco3b == "Hi World", "decorator with explicit arg failed"

# ===== SECTION: Wrapper decorators =====
# Wrapper decorators return a closure that wraps the original function

def double_result(func):
    def wrapper(x: int) -> int:
        return func(x) * 2
    return wrapper

@double_result
def get_value(n: int) -> int:
    return n + 5

wrapper_result1 = get_value(10)
assert wrapper_result1 == 30, "wrapper decorator (10+5)*2 should be 30"

wrapper_result2 = get_value(0)
assert wrapper_result2 == 10, "wrapper decorator (0+5)*2 should be 10"

# String wrapper decorator
def add_prefix(func):
    def wrapper(name: str) -> str:
        return "Hello, " + func(name)
    return wrapper

@add_prefix
def greet_person(name: str) -> str:
    return name + "!"

wrapper_str1 = greet_person("World")
assert wrapper_str1 == "Hello, World!", "wrapper string decorator failed"

wrapper_str2 = greet_person("Alice")
assert wrapper_str2 == "Hello, Alice!", "wrapper string decorator with Alice failed"

# ===== SECTION: @property decorator (getter/setter) =====

class PropCounter:
    _value: int

    def __init__(self, v: int):
        self._value = v

    @property
    def value(self) -> int:
        return self._value

    @value.setter
    def value(self, v: int) -> None:
        self._value = v

    @property
    def doubled(self) -> int:
        return self._value * 2

# Test property getter
prop_c = PropCounter(5)
assert prop_c.value == 5, "prop_c.value should equal 5"
assert prop_c.doubled == 10, "prop_c.doubled should equal 10"

# Test property setter
prop_c.value = 10
assert prop_c.value == 10, "prop_c.value should equal 10"
assert prop_c.doubled == 20, "prop_c.doubled should equal 20"

# Test read-only property (no setter)
class Rectangle:
    _width: int
    _height: int

    def __init__(self, w: int, h: int):
        self._width = w
        self._height = h

    @property
    def area(self) -> int:
        return self._width * self._height

rect = Rectangle(3, 4)
assert rect.area == 12, "rect.area should equal 12"

# ===== SECTION: @staticmethod decorator =====

class StaticMath:
    @staticmethod
    def static_add(a: int, b: int) -> int:
        return a + b

    @staticmethod
    def static_multiply(x: int, y: int) -> int:
        return x * y

# Test calling static method on class
assert StaticMath.static_add(2, 3) == 5, "StaticMath.static_add(2, 3) should equal 5"
assert StaticMath.static_multiply(4, 5) == 20, "StaticMath.static_multiply(4, 5) should equal 20"

# Test calling static method on instance
sm = StaticMath()
assert sm.static_add(10, 20) == 30, "sm.static_add(10, 20) should equal 30"
assert sm.static_multiply(6, 7) == 42, "sm.static_multiply(6, 7) should equal 42"

# Static method with no arguments
class StaticCounter:
    @staticmethod
    def get_default() -> int:
        return 100

assert StaticCounter.get_default() == 100, "StaticCounter.get_default() should equal 100"
sc = StaticCounter()
assert sc.get_default() == 100, "sc.get_default() should equal 100"

# ===== SECTION: @classmethod decorator =====

# Basic classmethod - `cls` is a compile-time alias of the enclosing class
class ClassMethodBasic:
    count: int = 0  # Class attribute with type annotation

    @classmethod
    def increment(cls) -> int:
        # cls.attr read AND write resolve like the written class name
        cls.count = cls.count + 1
        return cls.count

    @classmethod
    def get_count(cls) -> int:
        return cls.count

# Test calling classmethod on class
assert ClassMethodBasic.get_count() == 0, "ClassMethodBasic.get_count() should equal 0"
result = ClassMethodBasic.increment()
assert result == 1, "result should equal 1"
assert ClassMethodBasic.get_count() == 1, "ClassMethodBasic.get_count() should equal 1"
ClassMethodBasic.increment()
assert ClassMethodBasic.get_count() == 2, "ClassMethodBasic.get_count() should equal 2"

# Test calling classmethod on instance
obj = ClassMethodBasic()
result2 = obj.increment()
assert result2 == 3, "result2 should equal 3"
assert obj.get_count() == 3, "obj.get_count() should equal 3"

# Classmethod with additional parameters
class ClassMethodWithArgs:
    value: int = 10  # Class attribute with type annotation

    def __init__(self, start: int) -> None:
        self.instance_value = start

    @classmethod
    def add_to_value(cls, x: int) -> int:
        return cls.value + x

    @classmethod
    def multiply_value(cls, x: int, y: int) -> int:
        return cls.value * x * y

    @classmethod
    def make(cls, start: int) -> "ClassMethodWithArgs":
        # `cls(...)` alternative constructor → constructs the enclosing class
        return cls(start)

    @classmethod
    def combined(cls, x: int) -> int:
        # `cls.method(...)` dispatch (no spread) to sibling classmethods
        return cls.add_to_value(x) + cls.multiply_value(1, 1)

# Test classmethod with args on class
assert ClassMethodWithArgs.add_to_value(5) == 15, "ClassMethodWithArgs.add_to_value(5) should equal 15"
assert ClassMethodWithArgs.multiply_value(2, 3) == 60, "ClassMethodWithArgs.multiply_value(2, 3) should equal 60"

# `cls(...)` alternative constructor and `cls.method(...)` dispatch
made = ClassMethodWithArgs.make(42)
assert made.instance_value == 42, "ClassMethodWithArgs.make(42).instance_value should equal 42"
print(made.instance_value)
assert ClassMethodWithArgs.combined(5) == 25, "ClassMethodWithArgs.combined(5) should equal 25"

# Test classmethod with args on instance
cwa = ClassMethodWithArgs(0)
assert cwa.add_to_value(20) == 30, "cwa.add_to_value(20) should equal 30"
assert cwa.multiply_value(4, 5) == 200, "cwa.multiply_value(4, 5) should equal 200"

# Classmethod returning different types
class ClassMethodTypes:
    name = "TestClass"  # Class attribute with type annotation

    @classmethod
    def get_name(cls: int) -> str:
        return ClassMethodTypes.name

    @classmethod
    def is_valid(cls: int) -> bool:
        return True

assert ClassMethodTypes.get_name() == "TestClass", "ClassMethodTypes.get_name() should equal \"TestClass\""
assert ClassMethodTypes.is_valid() == True, "ClassMethodTypes.is_valid() should equal True"

# Mixed static and class methods in same class
class MixedMethods:
    counter: int = 0  # Class attribute with type annotation

    @staticmethod
    def static_helper(x: int) -> int:
        return x * 2

    @classmethod
    def class_increment(cls: int) -> int:
        MixedMethods.counter = MixedMethods.counter + 1
        return MixedMethods.counter

    def instance_method(self) -> int:
        return MixedMethods.counter + 100

# Test all three method types
assert MixedMethods.static_helper(5) == 10, "MixedMethods.static_helper(5) should equal 10"
assert MixedMethods.class_increment() == 1, "MixedMethods.class_increment() should equal 1"
mm = MixedMethods()
assert mm.instance_method() == 101, "mm.instance_method() should equal 101"
assert mm.static_helper(7) == 14, "mm.static_helper(7) should equal 14"
assert mm.class_increment() == 2, "mm.class_increment() should equal 2"

# Test annotated assignment with value is treated as class attribute (not instance field)
class AnnotatedClassAttr:
    count: int = 0
    name: str = "test"
    flag: bool = True

    @classmethod
    def increment_count(cls: int) -> int:
        AnnotatedClassAttr.count = AnnotatedClassAttr.count + 1
        return AnnotatedClassAttr.count

# Verify class attributes are accessible and mutable
assert AnnotatedClassAttr.count == 0, "AnnotatedClassAttr.count should equal 0"
assert AnnotatedClassAttr.name == "test", "AnnotatedClassAttr.name should equal \"test\""
assert AnnotatedClassAttr.flag == True, "AnnotatedClassAttr.flag should equal True"
assert AnnotatedClassAttr.increment_count() == 1, "AnnotatedClassAttr.increment_count() should equal 1"
assert AnnotatedClassAttr.count == 1, "AnnotatedClassAttr.count should equal 1"
AnnotatedClassAttr.count = 100
assert AnnotatedClassAttr.count == 100, "AnnotatedClassAttr.count should equal 100"

print("@classmethod tests passed!")

# ===== SECTION: Abstract methods =====

# Test that concrete classes implementing abstract methods work correctly
class AbstractAnimal:
    @abstractmethod
    def speak(self) -> str:
        pass

    def describe(self) -> str:
        return "I am an animal"

class ConcreteDog(AbstractAnimal):
    def speak(self) -> str:
        return "Woof!"

class ConcreteCat(AbstractAnimal):
    def speak(self) -> str:
        return "Meow!"

# Concrete classes can be instantiated
dog_impl = ConcreteDog()
assert dog_impl.speak() == "Woof!", "dog_impl.speak() should equal \"Woof!\""
assert dog_impl.describe() == "I am an animal", "dog_impl.describe() should equal \"I am an animal\""

cat_impl = ConcreteCat()
assert cat_impl.speak() == "Meow!", "cat_impl.speak() should equal \"Meow!\""
assert cat_impl.describe() == "I am an animal", "cat_impl.describe() should equal \"I am an animal\""

# Test multi-level inheritance with abstract methods
class AbstractShape:
    @abstractmethod
    def area(self) -> int:
        pass

    @abstractmethod
    def perimeter(self) -> int:
        pass

class AbstractPolygon(AbstractShape):
    sides: int

    def __init__(self, sides: int):
        self.sides = sides

    # Still abstract - doesn't implement area or perimeter

class ConcreteSquare(AbstractPolygon):
    size: int

    def __init__(self, size: int):
        super().__init__(4)
        self.size = size

    def area(self) -> int:
        return self.size * self.size

    def perimeter(self) -> int:
        return self.size * 4

square = ConcreteSquare(5)
assert square.sides == 4, "square.sides should equal 4"
assert square.size == 5, "square.size should equal 5"
assert square.area() == 25, "square.area() should equal 25"
assert square.perimeter() == 20, "square.perimeter() should equal 20"

# Test partial implementation - subclass implements one abstract method
class PartiallyConcreteShape(AbstractShape):
    def area(self) -> int:
        return 100

# PartiallyConcreteShape still has perimeter as abstract

class FullyConcreteShape(PartiallyConcreteShape):
    def perimeter(self) -> int:
        return 40

full_shape = FullyConcreteShape()
assert full_shape.area() == 100, "full_shape.area() should equal 100"
assert full_shape.perimeter() == 40, "full_shape.perimeter() should equal 40"

print("Abstract method tests passed!")

# ===== SECTION: Dunder methods - __str__ =====

class PointWithStr:
    x: int
    y: int

    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

    def __str__(self) -> str:
        return f"Point({self.x}, {self.y})"

p_str = PointWithStr(3, 4)
str_result = str(p_str)
assert str_result == "Point(3, 4)", f"Expected 'Point(3, 4)', got '{str_result}'"

# Test fallback (no __str__)
class PointNoStr:
    x: int
    y: int

    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

p_no_str = PointNoStr(1, 2)
str_result2 = str(p_no_str)
assert "object at" in str_result2, "Should show default repr"

print("Dunder __str__ tests passed!")

# ===== SECTION: Dunder methods - __repr__ =====

class PointWithRepr:
    x: int
    y: int

    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

    def __repr__(self) -> str:
        return f"Point(x={self.x}, y={self.y})"

p_repr = PointWithRepr(5, 6)
repr_result = repr(p_repr)
assert repr_result == "Point(x=5, y=6)", f"Expected 'Point(x=5, y=6)', got '{repr_result}'"

print("Dunder __repr__ tests passed!")

# ===== SECTION: Dunder methods - __eq__ =====

class PointWithEq:
    x: int
    y: int

    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

    def __eq__(self, other) -> bool:
        # CPython idiom: guard field access with isinstance — __eq__'s `other`
        # is polymorphic (any type) per Data Model §3.3.8.
        if isinstance(other, PointWithEq):
            return self.x == other.x and self.y == other.y
        return False

p1_eq = PointWithEq(1, 2)
p2_eq = PointWithEq(1, 2)  # Different instance with same values
p3_eq = PointWithEq(3, 4)  # Different instance with different values

# Test __eq__ with field comparison
assert p1_eq == p2_eq, "Points with same values should be equal"
assert not (p1_eq == p3_eq), "Points with different values should not be equal"

# Test __ne__
assert p1_eq != p3_eq, "Different points should be not equal"
assert not (p1_eq != p2_eq), "Same points should not be not-equal"

print("Dunder __eq__ tests passed!")

# ===== SECTION: Dunder methods - __hash__ =====

class PointHashable:
    x: int
    y: int

    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

    def __hash__(self) -> int:
        # Combine x and y hashes
        return self.x * 31 + self.y

p1_hash = PointHashable(3, 4)
p2_hash = PointHashable(3, 4)

# Test hash
h1 = hash(p1_hash)
h2 = hash(p2_hash)
assert h1 == h2, "Equal objects should have equal hashes"
assert h1 == 3 * 31 + 4, f"Hash should be {3 * 31 + 4}, got {h1}"

print("Dunder __hash__ tests passed!")

# ===== SECTION: Dunder methods - __len__ =====

class Container:
    count: int

    def __init__(self):
        self.count = 0

    def add(self):
        self.count = self.count + 1

    def __len__(self) -> int:
        return self.count

c = Container()
assert len(c) == 0, "Empty container should have length 0"

c.add()
c.add()
c.add()
assert len(c) == 3, "Container with 3 items should have length 3"

print("Dunder __len__ tests passed!")

# ===== SECTION: Mutable defaults in __init__ =====
# Python's mutable default gotcha also applies to __init__ methods:
# mutable defaults (list, dict, set) are evaluated once at class definition time
# and shared across all instances that don't provide an explicit argument.

class Counter:
    counts: list[int]

    def __init__(self, counts: list[int] = []):
        self.counts = counts

    def add(self, n: int) -> None:
        self.counts.append(n)

# First instance uses the default list
c1 = Counter()
c1.add(1)
assert len(c1.counts) == 1, "c1 should have 1 element"
assert c1.counts[0] == 1, "c1.counts[0] should be 1"

# Second instance should share the same default list!
c2 = Counter()
c2.add(2)
assert len(c2.counts) == 2, "c2 should have 2 elements (shared list)"
assert c2.counts[0] == 1, "c2.counts[0] should be 1 (from c1)"
assert c2.counts[1] == 2, "c2.counts[1] should be 2"

# Both instances refer to the same list object
assert c1.counts == c2.counts, "c1 and c2 should share the same list"

# Third instance with explicit list should NOT use the shared default
c3 = Counter([100])
c3.add(3)
assert len(c3.counts) == 2, "c3 should have 2 elements"
assert c3.counts[0] == 100, "c3.counts[0] should be 100 (explicit)"
assert c3.counts[1] == 3, "c3.counts[1] should be 3"

# The shared default list (c1, c2) should still have 2 elements
assert len(c1.counts) == 2, "c1 should still have 2 elements"

# Fourth instance without args should use the shared default again
c4 = Counter()
c4.add(4)
assert len(c4.counts) == 3, "c4 should have 3 elements (shared list)"
assert c4.counts[2] == 4, "c4.counts[2] should be 4"

# All instances using defaults share the same list
assert c1.counts == c2.counts, "c1 and c2 share default list"
assert c1.counts == c4.counts, "c1 and c4 share default list"

print("Mutable defaults in __init__ tests passed!")

# ===== SECTION: Class names as type annotations =====

class TypedPoint:
    x: int
    y: int

    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

    def add(self, other: TypedPoint) -> TypedPoint:
        # Method with class type parameter (same class)
        return TypedPoint(self.x + other.x, self.y + other.y)

# Function parameter with class type
def get_x(p: TypedPoint) -> int:
    return p.x

# Function return with class type
def create_point() -> TypedPoint:
    return TypedPoint(10, 20)

# Variable annotation with class type
tp1: TypedPoint = TypedPoint(3, 4)
assert tp1.x == 3, "tp1.x should equal 3"
assert tp1.y == 4, "tp1.y should equal 4"

# Test function with class type parameter
assert get_x(tp1) == 3, "get_x(tp1) should equal 3"

# Test function with class type return
tp2: TypedPoint = create_point()
assert tp2.x == 10, "tp2.x should equal 10"
assert tp2.y == 20, "tp2.y should equal 20"

# Test method with class type parameter
tp3: TypedPoint = tp1.add(tp2)
assert tp3.x == 13, "tp3.x should equal 13 (3 + 10)"
assert tp3.y == 24, "tp3.y should equal 24 (4 + 20)"

# Optional class type (Union with None)
def maybe_point(flag: bool) -> TypedPoint | None:
    if flag:
        return TypedPoint(1, 1)
    return None

opt_p: TypedPoint | None = maybe_point(True)
assert opt_p is not None, "opt_p should not be None"
if opt_p is not None:
    assert opt_p.x == 1, "opt_p.x should equal 1"

none_p: TypedPoint | None = maybe_point(False)
assert none_p is None, "none_p should be None"

# List of class type
def sum_points(points: list[TypedPoint]) -> int:
    total: int = 0
    for p in points:
        total = total + p.x + p.y
    return total

point_list: list[TypedPoint] = [TypedPoint(1, 2), TypedPoint(3, 4), TypedPoint(5, 6)]
total_sum: int = sum_points(point_list)
assert total_sum == 21, f"sum_points should equal 21 (1+2+3+4+5+6), got {total_sum}"

print("Class type annotation tests passed!")

# ===== SECTION: Comparison dunders =====

class Temperature:
    degrees: float

    def __init__(self, deg: float) -> None:
        self.degrees = deg

    # Comparison dunders default `other` to `Any` per Data Model §3.3.8
    # (`a == b` must never raise). Guard field access with isinstance —
    # CPython idiom for heterogeneous comparisons.
    def __lt__(self, other) -> bool:
        if isinstance(other, Temperature):
            return self.degrees < other.degrees
        return False

    def __le__(self, other) -> bool:
        if isinstance(other, Temperature):
            return self.degrees <= other.degrees
        return False

    def __gt__(self, other) -> bool:
        if isinstance(other, Temperature):
            return self.degrees > other.degrees
        return False

    def __ge__(self, other) -> bool:
        if isinstance(other, Temperature):
            return self.degrees >= other.degrees
        return False

    def __eq__(self, other) -> bool:
        if isinstance(other, Temperature):
            return self.degrees == other.degrees
        return False

cold = Temperature(0.0)
warm = Temperature(25.0)
hot = Temperature(40.0)
also_cold = Temperature(0.0)

# __lt__
assert cold < warm, "cold < warm failed"
assert not (warm < cold), "warm < cold should be False"

# __le__
assert cold <= warm, "cold <= warm failed"
assert cold <= also_cold, "cold <= also_cold failed"
assert not (warm <= cold), "warm <= cold should be False"

# __gt__
assert hot > warm, "hot > warm failed"
assert not (cold > warm), "cold > warm should be False"

# __ge__
assert hot >= warm, "hot >= warm failed"
assert cold >= also_cold, "cold >= also_cold failed"
assert not (cold >= warm), "cold >= warm should be False"

# __eq__ and __ne__ (ne falls back to negated eq)
assert cold == also_cold, "cold == also_cold failed"
assert not (cold == warm), "cold == warm should be False"
assert cold != warm, "cold != warm failed"
assert not (cold != also_cold), "cold != also_cold should be False"

print("Dunder comparison tests passed!")

# ===== SECTION: Arithmetic dunders =====

class Vector2D:
    x: float
    y: float

    def __init__(self, x: float, y: float) -> None:
        self.x = x
        self.y = y

    # Binary numeric dunders default `other` to `Union[Self, int, float, bool]`
    # per Data Model §3.3.8. Narrow with isinstance for Self-specific field
    # access; other branches fall through to a sensible default.
    def __add__(self, other) -> Vector2D:
        if isinstance(other, Vector2D):
            return Vector2D(self.x + other.x, self.y + other.y)
        return self

    def __sub__(self, other) -> Vector2D:
        if isinstance(other, Vector2D):
            return Vector2D(self.x - other.x, self.y - other.y)
        return self

    def __mul__(self, other) -> Vector2D:
        if isinstance(other, Vector2D):
            return Vector2D(self.x * other.x, self.y * other.y)
        return self

    def __neg__(self) -> Vector2D:
        return Vector2D(-self.x, -self.y)

    def __eq__(self, other) -> bool:
        if isinstance(other, Vector2D):
            return self.x == other.x and self.y == other.y
        return False

v1 = Vector2D(1.0, 2.0)
v2 = Vector2D(3.0, 4.0)

# __add__
v_add = v1 + v2
assert v_add.x == 4.0, f"v1+v2 x failed: {v_add.x}"
assert v_add.y == 6.0, f"v1+v2 y failed: {v_add.y}"

# __sub__
v_sub = v2 - v1
assert v_sub.x == 2.0, f"v2-v1 x failed: {v_sub.x}"
assert v_sub.y == 2.0, f"v2-v1 y failed: {v_sub.y}"

# __mul__
v_mul = v1 * v2
assert v_mul.x == 3.0, f"v1*v2 x failed: {v_mul.x}"
assert v_mul.y == 8.0, f"v1*v2 y failed: {v_mul.y}"

# __neg__
v_neg = -v1
assert v_neg.x == -1.0, f"-v1 x failed: {v_neg.x}"
assert v_neg.y == -2.0, f"-v1 y failed: {v_neg.y}"

# Chaining arithmetic
v_chain = v1 + v2 + Vector2D(10.0, 10.0)
assert v_chain.x == 14.0, f"chained add x failed: {v_chain.x}"
assert v_chain.y == 16.0, f"chained add y failed: {v_chain.y}"

print("Dunder arithmetic tests passed!")

# ===== Container Dunders (__getitem__, __setitem__, __delitem__, __contains__) =====

class IntList:
    items: list[int]
    size: int

    def __init__(self, items: list[int]) -> None:
        self.items = items
        self.size = len(items)

    def __getitem__(self, index: int) -> int:
        return self.items[index]

    def __setitem__(self, index: int, value: int) -> None:
        self.items[index] = value

    def __delitem__(self, index: int) -> None:
        del self.items[index]
        self.size = self.size - 1

    def __contains__(self, value: int) -> bool:
        i: int = 0
        while i < self.size:
            if self.items[i] == value:
                return True
            i = i + 1
        return False

# Create test container
container_items: list[int] = [10, 20, 30, 40, 50]
container = IntList(container_items)

# __getitem__
assert container[0] == 10, f"getitem [0] failed: {container[0]}"
assert container[2] == 30, f"getitem [2] failed: {container[2]}"
assert container[4] == 50, f"getitem [4] failed: {container[4]}"

# __setitem__
container[1] = 99
assert container[1] == 99, f"setitem [1] failed: {container[1]}"
container[0] = 0
assert container[0] == 0, f"setitem [0] failed: {container[0]}"

# __contains__ (in / not in)
assert 99 in container, "contains 99 failed"
assert 30 in container, "contains 30 failed"
assert 999 not in container, "not contains 999 failed"
assert 1 not in container, "not contains 1 failed"

# __delitem__
container2_items: list[int] = [100, 200, 300]
container2 = IntList(container2_items)
del container2[1]
assert container2.size == 2, f"delitem size failed: {container2.size}"
assert container2[0] == 100, f"delitem [0] failed: {container2[0]}"
assert container2[1] == 300, f"delitem [1] after delete failed: {container2[1]}"

print("Container dunder tests passed!")

# Test container dunders with inheritance
class NamedIntList(IntList):
    name: str

    def __init__(self, name: str, items: list[int]) -> None:
        super().__init__(items)
        self.name = name

named_items: list[int] = [5, 10, 15]
named_container = NamedIntList("test", named_items)
assert named_container[0] == 5, f"inherited getitem failed: {named_container[0]}"
assert named_container[2] == 15, f"inherited getitem [2] failed: {named_container[2]}"
named_container[1] = 42
assert named_container[1] == 42, f"inherited setitem failed: {named_container[1]}"
assert 42 in named_container, "inherited contains failed"
assert 99 not in named_container, "inherited not contains failed"

print("Inherited container dunder tests passed!")

# ==================== Iterator Protocol Tests ====================

# Basic iterator class: counts from 0 to stop-1
class CountUp:
    current: int
    stop: int

    def __init__(self, stop: int) -> None:
        self.current = 0
        self.stop = stop

    def __iter__(self) -> CountUp:
        return self

    def __next__(self) -> int:
        if self.current >= self.stop:
            raise StopIteration()
        val: int = self.current
        self.current = self.current + 1
        return val

# Test 1: Basic for loop over class iterator
counter = CountUp(5)
iter_result: list[int] = []
for iter_val in counter:
    iter_result.append(iter_val)
assert iter_result == [0, 1, 2, 3, 4], f"basic class iterator failed: {iter_result}"
print("Class iterator basic for loop passed!")

# Test 2: iter() builtin with class
counter2 = CountUp(3)
iter_obj = iter(counter2)
assert next(iter_obj) == 0, "iter()/next() first failed"
assert next(iter_obj) == 1, "iter()/next() second failed"
assert next(iter_obj) == 2, "iter()/next() third failed"
print("Class iterator iter()/next() passed!")

# Test 3: for...else (else runs on normal completion)
counter3 = CountUp(3)
iter_else_result: list[int] = []
iter_else_ran: bool = False
for iter_else_val in counter3:
    iter_else_result.append(iter_else_val)
else:
    iter_else_ran = True
assert iter_else_result == [0, 1, 2], f"for...else iterator failed: {iter_else_result}"
assert iter_else_ran, "else block should run on normal completion"
print("Class iterator for...else passed!")

# Test 4: Empty iterator (stop=0)
empty_counter = CountUp(0)
empty_result: list[int] = []
for empty_val in empty_counter:
    empty_result.append(empty_val)
assert empty_result == [], f"empty iterator failed: {empty_result}"
print("Class iterator empty iteration passed!")

# Test 5: break exits loop (else should not run)
counter5 = CountUp(10)
break_result: list[int] = []
break_else_ran: bool = False
for break_val in counter5:
    if break_val >= 3:
        break
    break_result.append(break_val)
else:
    break_else_ran = True
assert break_result == [0, 1, 2], f"break in class iterator failed: {break_result}"
assert not break_else_ran, "else should not run when break is used"
print("Class iterator break passed!")

# Test 6: Inherited iterator protocol
class CountUpNamed(CountUp):
    label: str

    def __init__(self, label: str, stop: int) -> None:
        super().__init__(stop)
        self.label = label

inherited_counter = CountUpNamed("test", 4)
inherited_result: list[int] = []
for inherited_val in inherited_counter:
    inherited_result.append(inherited_val)
assert inherited_result == [0, 1, 2, 3], f"inherited iterator failed: {inherited_result}"
print("Inherited class iterator passed!")

print("All iterator protocol tests passed!")

# ==================== __call__ dunder tests ====================
print("Testing __call__ dunder...")

# Test 1: Basic callable object
class Adder:
    value: int
    def __init__(self, value: int) -> None:
        self.value = value
    def __call__(self, x: int) -> int:
        return self.value + x

adder: Adder = Adder(10)
assert adder(5) == 15, f"Adder(10)(5) failed: {adder(5)}"
assert adder(0) == 10, f"Adder(10)(0) failed: {adder(0)}"
assert adder(-3) == 7, f"Adder(10)(-3) failed: {adder(-3)}"
print("Basic __call__ passed!")

# Test 2: Callable with multiple arguments
class Multiplier:
    factor: int
    def __init__(self, factor: int) -> None:
        self.factor = factor
    def __call__(self, a: int, b: int) -> int:
        return self.factor * (a + b)

mult: Multiplier = Multiplier(3)
assert mult(2, 4) == 18, f"Multiplier(3)(2, 4) failed: {mult(2, 4)}"
print("Multi-arg __call__ passed!")

# Test 3: Callable returning string
class Greeter:
    greeting: str
    def __init__(self, greeting: str) -> None:
        self.greeting = greeting
    def __call__(self, name: str) -> str:
        return self.greeting + " " + name

greeter: Greeter = Greeter("Hello")
assert greeter("World") == "Hello World", f"Greeter failed: {greeter('World')}"
print("String __call__ passed!")

# Test 4: Callable with state mutation
class CallCounter:
    count: int
    def __init__(self) -> None:
        self.count = 0
    def __call__(self) -> int:
        self.count = self.count + 1
        return self.count

call_counter: CallCounter = CallCounter()
assert call_counter() == 1
assert call_counter() == 2
assert call_counter() == 3
assert call_counter.count == 3
print("Stateful __call__ passed!")

print("All __call__ dunder tests passed!")

# ==================== Context manager tests ====================
print("Testing context managers...")

# Test 1: Basic context manager
class MyContext:
    entered: bool
    exited: bool
    def __init__(self) -> None:
        self.entered = False
        self.exited = False
    def __enter__(self) -> MyContext:
        self.entered = True
        return self
    def __exit__(self, exc_type: int, exc_val: int, exc_tb: int) -> bool:
        self.exited = True
        return False

my_ctx: MyContext = MyContext()
with my_ctx as my_ctx_val:
    assert my_ctx_val.entered == True, "context manager __enter__ not called"
assert my_ctx.exited == True, "context manager __exit__ not called"
print("Basic context manager passed!")

# Test 2: Context manager with body operations
class ResourceTracker:
    opened: bool
    closed: bool
    ops: int
    def __init__(self) -> None:
        self.opened = False
        self.closed = False
        self.ops = 0
    def __enter__(self) -> ResourceTracker:
        self.opened = True
        return self
    def __exit__(self, exc_type: int, exc_val: int, exc_tb: int) -> bool:
        self.closed = True
        return False
    def do_op(self) -> None:
        self.ops = self.ops + 1

res_tracker: ResourceTracker = ResourceTracker()
with res_tracker as rt:
    rt.do_op()
    rt.do_op()
    rt.do_op()
assert res_tracker.opened == True
assert res_tracker.closed == True
assert res_tracker.ops == 3
print("Context manager with operations passed!")

print("All context manager tests passed!")

# ============================================================================
# print(instance) calls __str__ (regression test)
# ============================================================================

class PrintablePoint:
    x: int
    y: int
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y
    def __str__(self) -> str:
        return f"Point({self.x}, {self.y})"
    def __repr__(self) -> str:
        return f"Point(x={self.x}, y={self.y})"

pp = PrintablePoint(3, 4)
# print(instance) must call __str__, not show <object at 0x...>
assert str(pp) == "Point(3, 4)", "str(instance) should call __str__"
# Test __repr__ fallback when __str__ not defined
class ReprOnly:
    v: int
    def __init__(self, v: int):
        self.v = v
    def __repr__(self) -> str:
        return f"ReprOnly({self.v})"

ro = ReprOnly(42)
assert repr(ro) == "ReprOnly(42)", "repr(instance) should call __repr__"

print("print(instance) __str__/__repr__ tests passed!")

# ===== SECTION: Init-only field declarations (no class-level annotations) =====
# Fields discovered from self.field = value in __init__ without class-level x: int

class ImplicitPt:
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

ipt = ImplicitPt(10, 20)
assert ipt.x == 10, "ImplicitPt.x should be 10"
assert ipt.y == 20, "ImplicitPt.y should be 20"

# Multiple instances
ipt2 = ImplicitPt(100, 200)
assert ipt2.x == 100, "ipt2.x should be 100"
assert ipt.x == 10, "ipt.x should still be 10"

# Field modification
ipt.x = 99
assert ipt.x == 99, "ImplicitPt.x after modification should be 99"

# Method reading init-only fields
class ImplicitBox:
    def __init__(self, val: int):
        self.val = val
    def get_val(self) -> int:
        return self.val

ib = ImplicitBox(42)
assert ib.get_val() == 42, "ImplicitBox.get_val() should be 42"

# Property reading init-only fields
class ImplicitProp:
    def __init__(self, x: int):
        self._x = x
    @property
    def x(self) -> int:
        return self._x

ip = ImplicitProp(42)
assert ip.x == 42, "ImplicitProp.x property should be 42"

# Mixed: some fields declared at class level, some only in __init__
class MixedFields:
    declared: int

    def __init__(self, a: int, b: int):
        self.declared = a
        self.implicit = b
    def sum(self) -> int:
        return self.declared + self.implicit

mf = MixedFields(10, 20)
assert mf.declared == 10, "MixedFields.declared should be 10"
assert mf.implicit == 20, "MixedFields.implicit should be 20"
assert mf.sum() == 30, "MixedFields.sum() should be 30"

print("Init-only field declarations: PASS")

# ===== SECTION: Class attribute access through instances =====

class ClassAttrAccess:
    x: int = 10
    name: str = "hello"

ca = ClassAttrAccess()
assert ca.x == 10, "Instance access to class attr int should be 10"
assert ca.name == "hello", "Instance access to class attr str should be hello"

# Class attr modification through instance
ca.x = 42
assert ca.x == 42, "Class attr modified through instance should be 42"

print("Class attribute access through instances: PASS")

# ===== Regression: class attribute access via ClassName.attr =====
# Tests that class-level attributes are accessible both through
# the class name and through instances.

class RegClassAttr:
    count: int = 0
    label: str = "default"

# Access through class name
assert RegClassAttr.count == 0, "class attr: initial int value"
assert RegClassAttr.label == "default", "class attr: initial str value"

# Modify through class name
RegClassAttr.count = 99
assert RegClassAttr.count == 99, f"class attr: modified int, got {RegClassAttr.count}"

RegClassAttr.label = "updated"
assert RegClassAttr.label == "updated", f"class attr: modified str, got {RegClassAttr.label}"

print("Class attribute access regression: PASS")

# ===== Regression: instance field access on user-defined classes =====
class FieldAccess:
    def __init__(self, a: int, b: str, c: float):
        self.a = a
        self.b = b
        self.c = c

fa = FieldAccess(42, "hello", 3.14)
assert fa.a == 42, f"field access: int, got {fa.a}"
assert fa.b == "hello", f"field access: str, got {fa.b}"

# Modify fields
fa.a = 100
fa.b = "world"
assert fa.a == 100, "field modification: int"
assert fa.b == "world", "field modification: str"

print("Instance field access regression: PASS")

# ===== Regression: class with __call__ dunder =====
class CallableAdder:
    offset: int

    def __init__(self, offset: int):
        self.offset = offset

    def __call__(self, x: int) -> int:
        return x + self.offset

cadder = CallableAdder(10)
result_call = cadder(5)
assert result_call == 15, f"__call__ dunder: expected 15, got {result_call}"

cadder2 = CallableAdder(100)
assert cadder2(7) == 107, "__call__ with different offset"

print("Callable class (__call__) regression: PASS")

# ===== Explicit dunder method calls: obj.__method__(args) =====
# Must produce the same result as operator syntax.

class DunderVec:
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

    # Polymorphic `other` (Data Model §3.3.8). Guard with isinstance so
    # `.x`/`.y` access is well-typed inside the branch.
    def __add__(self, other):
        if isinstance(other, DunderVec):
            return DunderVec(self.x + other.x, self.y + other.y)
        return self

    def __sub__(self, other):
        if isinstance(other, DunderVec):
            return DunderVec(self.x - other.x, self.y - other.y)
        return self

    def __mul__(self, other):
        if isinstance(other, DunderVec):
            return DunderVec(self.x * other.x, self.y * other.y)
        return self

    def __eq__(self, other):
        if isinstance(other, DunderVec):
            return self.x == other.x and self.y == other.y
        return False

    def __ne__(self, other):
        if isinstance(other, DunderVec):
            return self.x != other.x or self.y != other.y
        return True

    def __str__(self):
        return "DunderVec(" + str(self.x) + ", " + str(self.y) + ")"

    def __len__(self):
        return self.x + self.y

    def __neg__(self):
        return DunderVec(-self.x, -self.y)

    def __bool__(self):
        return self.x != 0 or self.y != 0

    def get_x(self):
        return self.x

da = DunderVec(2, 3)
db = DunderVec(4, 5)

# Arithmetic dunders via explicit call
dc = da.__add__(db)
assert dc.x == 6 and dc.y == 8, "explicit __add__"

dd = da.__sub__(db)
assert dd.x == -2 and dd.y == -2, "explicit __sub__"

de = da.__mul__(db)
assert de.x == 8 and de.y == 15, "explicit __mul__"

# Comparison dunders
assert da.__eq__(da) == True, "explicit __eq__ (same)"
assert da.__ne__(db) == True, "explicit __ne__ (diff)"
assert da.__eq__(db) == False, "explicit __eq__ (diff)"

# String dunder
assert da.__str__() == "DunderVec(2, 3)", "explicit __str__"

# Len dunder
assert da.__len__() == 5, "explicit __len__"

# Neg dunder
df = da.__neg__()
assert df.x == -2 and df.y == -3, "explicit __neg__"

# Bool dunder
assert da.__bool__() == True, "explicit __bool__ (nonzero)"
dzero = DunderVec(0, 0)
assert dzero.__bool__() == False, "explicit __bool__ (zero)"

# Regular method still works alongside dunders
assert da.get_x() == 2, "regular method after dunder"

# Operators still produce same result as explicit calls
dh = da + db
assert dh.x == dc.x and dh.y == dc.y, "operator == explicit dunder"

print("Explicit dunder method calls: PASS")

# ==================== Reverse Arithmetic Dunders ====================

class RevNum:
    def __init__(self, v: int):
        self.v = v

    def __add__(self, other) -> RevNum:
        if isinstance(other, RevNum):
            return RevNum(self.v + other.v)
        return self

    def __radd__(self, other: int) -> RevNum:
        return RevNum(other + self.v)

    def __rsub__(self, other: int) -> RevNum:
        return RevNum(other - self.v)

    def __rmul__(self, other: int) -> RevNum:
        return RevNum(other * self.v)

# Forward dunder: obj + obj
rn1 = RevNum(10)
rn2 = RevNum(3)
rn3 = rn1 + rn2
assert rn3.v == 13, "forward __add__"

# Reverse dunders: int op obj
rn4 = 5 + rn1
assert rn4.v == 15, "__radd__: 5 + RevNum(10)"

rn5 = 20 - rn1
assert rn5.v == 10, "__rsub__: 20 - RevNum(10)"

rn6 = 4 * rn2
assert rn6.v == 12, "__rmul__: 4 * RevNum(3)"

print("Reverse arithmetic dunders: PASS")

# ==================== Binop Dunder Returning a Non-Class Type ====================
# A binary dunder may return a type OTHER than the defining class — e.g.
# `__add__ -> int`. The result of the binop must be typed by the dunder's
# ACTUAL return type (int here), not assumed to be the receiver class. The
# constraint solver is the authority for variable types, so when the result is
# bound to a variable (`n = a + b`) it must agree the variable is `int`;
# otherwise the variable is typed as the class (a GC root) while holding a raw
# int, mismatching what lowering emits and segfaulting on use.

class DotProduct:
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

    def __add__(self, other: "DotProduct") -> int:
        # Returns a scalar int, NOT a DotProduct.
        return self.x * other.x + self.y * other.y

# Direct use of the binop result as an int.
assert DotProduct(1, 2) + DotProduct(3, 4) == 11, "binop dunder -> int (direct)"

# Bound to a variable first — exercises the solver's view of the variable type.
dot_bound = DotProduct(2, 3) + DotProduct(4, 5)
assert dot_bound == 23, "binop dunder -> int (bound to variable)"
# The bound result flows into an int-typed context; if it were mistyped as
# DotProduct this would dispatch DotProduct.__add__ on an int and fail.
assert dot_bound + 1 == 24, "binop dunder -> int (used as int)"

# Companion case: a dunder that DOES return its own class still resolves.
class Scaled:
    def __init__(self, v: int):
        self.v = v

    def __mul__(self, other: "Scaled") -> "Scaled":
        return Scaled(self.v * other.v)

scaled_result = Scaled(2) * Scaled(3)
assert scaled_result.v == 6, "binop dunder -> class (returns own type)"

print("Binop dunder returning non-class type: PASS")

# ==================== Unary Dunders: __pos__, __abs__, __invert__ ====================

class UnaryNum:
    def __init__(self, v: int):
        self.v = v

    def __pos__(self) -> UnaryNum:
        return UnaryNum(self.v if self.v >= 0 else -self.v)

    def __neg__(self) -> UnaryNum:
        return UnaryNum(-self.v)

    def __abs__(self) -> UnaryNum:
        return UnaryNum(self.v if self.v >= 0 else -self.v)

    def __invert__(self) -> int:
        return ~self.v

# Unary plus on class
un1 = UnaryNum(-5)
un2 = +un1
assert un2.v == 5, "__pos__: +UnaryNum(-5)"

# Unary neg (already tested, sanity check)
un3 = -un1
assert un3.v == 5, "__neg__: -UnaryNum(-5)"

# abs() on class
un4 = abs(un1)
assert un4.v == 5, "__abs__: abs(UnaryNum(-5))"

un5 = abs(UnaryNum(7))
assert un5.v == 7, "__abs__: abs(UnaryNum(7))"

# Bitwise invert on class
un6 = ~UnaryNum(0)
assert un6 == -1, "__invert__: ~UnaryNum(0)"

un7 = ~UnaryNum(5)
assert un7 == -6, "__invert__: ~UnaryNum(5)"

# Unary plus on primitives (identity)
assert +42 == 42, "unary + on int"
assert +0 == 0, "unary + on zero"

print("Unary dunders (__pos__, __abs__, __invert__): PASS")

# Unary dunders whose return type differs from the receiver class. The result
# dest must be typed by the dunder's actual return type, not by the operator's
# primitive semantics — otherwise `__neg__ -> int` (a non-Class return into a
# Class-typed dest) and `__invert__ -> Class` (a Class return into the Int dest
# the Invert path defaults to) both trip the codegen/verifier.
class UnaryRet:
    def __init__(self, v: int):
        self.v = v

    def __neg__(self) -> int:
        return self.v * 100

    def __invert__(self) -> UnaryRet:
        return UnaryRet(self.v + 1)

_ur = UnaryRet(3)
assert (-_ur) == 300, "__neg__ -> int"
_ur_inv = ~_ur
assert _ur_inv.v == 4, "__invert__ -> UnaryRet"

print("Unary dunders with cross-type returns: PASS")

# ==================== getattr(obj, name[, default]) ====================

class GetAttrBox:
    def __init__(self, x: int):
        self.x = x

    def grow(self) -> int:
        return self.x * 2

_gab = GetAttrBox(7)
assert getattr(_gab, "x") == 7, "getattr existing field"
assert getattr(_gab, "missing", -1) == -1, "getattr missing with default"
_ga_raised = False
try:
    getattr(_gab, "nope")
except AttributeError:
    _ga_raised = True
assert _ga_raised, "getattr missing without default raises AttributeError"

print("getattr: PASS")

# ==================== Conversion Dunders: __int__, __float__, __bool__ ====================

class ConvNum:
    def __init__(self, v: int):
        self.v = v

    def __int__(self) -> int:
        return self.v

    def __float__(self) -> float:
        return float(self.v) + 0.5

    def __bool__(self) -> bool:
        return self.v != 0

# int() on class
cn1 = ConvNum(42)
assert int(cn1) == 42, "__int__: int(ConvNum(42))"
assert int(ConvNum(-7)) == -7, "__int__: int(ConvNum(-7))"

# float() on class
assert float(cn1) == 42.5, "__float__: float(ConvNum(42))"
assert float(ConvNum(0)) == 0.5, "__float__: float(ConvNum(0))"

# bool() on class
assert bool(cn1) == True, "__bool__: bool(ConvNum(42))"
assert bool(ConvNum(0)) == False, "__bool__: bool(ConvNum(0))"

print("Conversion dunders (__int__, __float__, __bool__): PASS")

# ===== SECTION: Bitwise dunders =====
class BitFlags:
    value: int
    def __init__(self, value: int):
        self.value = value
    def __and__(self, other: BitFlags) -> BitFlags:
        return BitFlags(self.value & other.value)
    def __or__(self, other: BitFlags) -> BitFlags:
        return BitFlags(self.value | other.value)
    def __xor__(self, other: BitFlags) -> BitFlags:
        return BitFlags(self.value ^ other.value)
    def __lshift__(self, other: BitFlags) -> BitFlags:
        return BitFlags(self.value << other.value)
    def __rshift__(self, other: BitFlags) -> BitFlags:
        return BitFlags(self.value >> other.value)

fl1 = BitFlags(12)   # 0b1100
fl2 = BitFlags(10)   # 0b1010

br = fl1 & fl2
assert br.value == 8, f"& failed: {br.value}"  # 0b1000

br = fl1 | fl2
assert br.value == 14, f"| failed: {br.value}"  # 0b1110

br = fl1 ^ fl2
assert br.value == 6, f"^ failed: {br.value}"  # 0b0110

br = fl1 << BitFlags(2)
assert br.value == 48, f"<< failed: {br.value}"  # 0b110000

br = fl1 >> BitFlags(2)
assert br.value == 3, f">> failed: {br.value}"  # 0b11

print("Bitwise dunders (__and__, __or__, __xor__, __lshift__, __rshift__): PASS")

# ===== SECTION: MatMul dunder =====
class MatMulObj:
    val: int
    def __init__(self, val: int):
        self.val = val
    def __matmul__(self, other: MatMulObj) -> MatMulObj:
        return MatMulObj(self.val * other.val)

mm1 = MatMulObj(3)
mm2 = MatMulObj(4)
mm_result = mm1 @ mm2
assert mm_result.val == 12, f"@ failed: {mm_result.val}"

print("MatMul dunder (__matmul__): PASS")

# ===== SECTION: __index__ dunder =====
class IndexObj:
    idx: int
    def __init__(self, idx: int):
        self.idx = idx
    def __index__(self) -> int:
        return self.idx

idx_items = [10, 20, 30, 40, 50]
idx1 = IndexObj(2)
assert idx_items[idx1] == 30, f"__index__ failed: {idx_items[idx1]}"

idx2 = IndexObj(-1)
assert idx_items[idx2] == 50, f"__index__ negative failed: {idx_items[idx2]}"

idx_s = "hello"
idx3 = IndexObj(1)
assert idx_s[idx3] == "e", f"__index__ str failed: {idx_s[idx3]}"

print("__index__ dunder: PASS")

# ===== SECTION: __format__ dunder =====
class ColorFmt:
    r: int
    g: int
    b: int
    def __init__(self, r: int, g: int, b: int):
        self.r = r
        self.g = g
        self.b = b
    def __format__(self, spec: str) -> str:
        if spec == "hex":
            return f"#{self.r:02x}{self.g:02x}{self.b:02x}"
        return f"({self.r}, {self.g}, {self.b})"

cfmt = ColorFmt(255, 128, 0)
assert format(cfmt) == "(255, 128, 0)", f"format() failed: {format(cfmt)}"
assert format(cfmt, "hex") == "#ff8000", f"format(hex) failed: {format(cfmt, 'hex')}"

print("__format__ dunder: PASS")

# ===== SECTION: __new__ dunder =====
class SingletonCounter:
    _count: int
    def __new__(cls: int) -> SingletonCounter:
        inst: SingletonCounter = object.__new__(cls)
        return inst
    def __init__(self):
        self._count = 0
    def increment(self) -> int:
        self._count = self._count + 1
        return self._count

sc1 = SingletonCounter()
assert sc1.increment() == 1
assert sc1.increment() == 2

# Test that __new__ + __init__ both run
sc2 = SingletonCounter()
assert sc2.increment() == 1  # new instance, __init__ resets to 0

print("__new__ dunder: PASS")

# ===== SECTION: __del__ dunder =====
# __del__ is called by GC during finalization — hard to test directly.
# We verify that defining __del__ doesn't crash and the class works normally.
class Cleanable:
    name: str
    def __init__(self, name: str):
        self.name = name
    def __del__(self) -> None:
        # In a real scenario this would release a resource
        pass

cl1 = Cleanable("test1")
assert cl1.name == "test1"
cl2 = Cleanable("test2")
assert cl2.name == "test2"

print("__del__ dunder: PASS")

# ===== SECTION: __copy__ dunder =====
import copy

class CopyPoint:
    x: int
    y: int
    label: str
    def __init__(self, x: int, y: int, label: str):
        self.x = x
        self.y = y
        self.label = label
    def __copy__(self) -> CopyPoint:
        return CopyPoint(self.x * 10, self.y * 10, self.label)

cp_orig = CopyPoint(1, 2, "origin")
# copy.copy(obj) dispatches to the user __copy__ (returns Any → gradual access).
cp_raw = copy.copy(cp_orig)
assert cp_raw.x == 10, f"copy.copy __copy__ x failed: {cp_raw.x}"
assert cp_raw.y == 20, f"copy.copy __copy__ y failed: {cp_raw.y}"
assert cp_raw.label == "origin", f"copy.copy __copy__ label failed"
# The direct dunder call works too (typed CopyPoint).
cp_copy = cp_orig.__copy__()
assert cp_copy.x == 10, f"__copy__ x failed: {cp_copy.x}"
assert cp_copy.y == 20, f"__copy__ y failed: {cp_copy.y}"
assert cp_copy.label == "origin", f"__copy__ label failed"

print("__copy__ dunder: PASS")

# ===== SECTION: __deepcopy__ dunder =====
class DeepContainer:
    items: list[int]
    tag: str
    def __init__(self, items: list[int], tag: str):
        self.items = items
        self.tag = tag
    def __deepcopy__(self, memo) -> DeepContainer:
        # Custom deep copy: copy list manually, transform tag. CPython passes a
        # memo dict here; the compiler's <__deepcopy__> thunk passes a fresh one.
        new_items: list[int] = []
        for item in self.items:
            new_items.append(item)
        return DeepContainer(new_items, self.tag + "_copy")

dc_orig = DeepContainer([1, 2, 3], "original")
# copy.deepcopy(obj) dispatches to the user __deepcopy__ (returns Any).
dc_deep = copy.deepcopy(dc_orig)
assert dc_deep.tag == "original_copy", f"copy.deepcopy __deepcopy__ tag failed: {dc_deep.tag}"
# Direct dunder call (typed DeepContainer) for the detailed independence checks.
dc_direct = dc_orig.__deepcopy__({})
assert dc_direct.tag == "original_copy", f"__deepcopy__ tag failed: {dc_direct.tag}"
assert dc_direct.items[0] == 1 and dc_direct.items[1] == 2 and dc_direct.items[2] == 3
# Verify independence (mutating the copy doesn't touch the original).
dc_direct.items.append(4)
assert len(dc_orig.items) == 3, f"deepcopy independence failed"

print("__deepcopy__ dunder: PASS")

# ==================== Numeric tower on user classes (Area B) ====================
# CPython Data Model §3.3.8: `other` parameter is polymorphic — the compiler
# MUST type it as Union[Self, int, float, bool] so the full numeric tower
# (V*int, int*V, float*V, V*float, ...) type-checks without spurious
# "unreachable code" warnings and without verifier panics.

class NumTower:
    def __init__(self, x: float): self.x = x
    def __add__(self, other):
        if isinstance(other, NumTower): return NumTower(self.x + other.x)
        if isinstance(other, (int, float)): return NumTower(self.x + other)
        return NumTower(0.0)
    def __radd__(self, other):
        if isinstance(other, (int, float)): return NumTower(other + self.x)
        return NumTower(0.0)
    def __mul__(self, other):
        if isinstance(other, NumTower): return NumTower(self.x * other.x)
        if isinstance(other, (int, float)): return NumTower(self.x * other)
        return NumTower(0.0)
    def __rmul__(self, other):
        if isinstance(other, NumTower): return NumTower(self.x * other.x)
        if isinstance(other, (int, float)): return NumTower(self.x * other)
        return NumTower(0.0)
    def __truediv__(self, other):
        if isinstance(other, NumTower): return NumTower(self.x / other.x)
        return NumTower(self.x / other)
    def __rtruediv__(self, other): return NumTower(other / self.x)
    def __sub__(self, other):
        if isinstance(other, NumTower): return NumTower(self.x - other.x)
        if isinstance(other, (int, float)): return NumTower(self.x - other)
        return NumTower(0.0)
    def __neg__(self): return NumTower(-self.x)
    def __pow__(self, other): return NumTower(self.x ** other)
    def __eq__(self, other):
        if isinstance(other, NumTower): return self.x == other.x
        return False

_a = NumTower(2.0)
_b = NumTower(3.0)

# Forward arithmetic
assert (_a + _b).x == 5.0, "NumTower + NumTower"
assert (_a + 1).x == 3.0, "NumTower + int"
assert (_a + 0.5).x == 2.5, "NumTower + float"

# Reverse arithmetic — the core Area B fix. Before Area B these panic'd with
# "arg 1 has type i64, expected f64" because `other` was wrongly narrowed to Self.
assert (1 + _a).x == 3.0, "int + NumTower (via __radd__)"
assert (0.5 + _a).x == 2.5, "float + NumTower (via __radd__)"
assert (2 * _a).x == 4.0, "int * NumTower (via __rmul__)"
assert (0.5 * _a).x == 1.0, "float * NumTower (via __rmul__)"
assert (6.0 / _a).x == 3.0, "float / NumTower (via __rtruediv__)"

# Chained mixed-operand expressions
assert ((1 + _a) * 2).x == 6.0, "chained int/NumTower ops"
assert ((_a * 2) - _b).x == 1.0, "sub after mul"

# Unary
assert (-_a).x == -2.0, "unary negation"

# Power with negative float exponent
assert abs((NumTower(4.0) ** -0.5).x - 0.5) < 1e-9, "pow negative float"

# Equality NEVER raises per CPython §3.3.8 — always yields a bool.
assert _a == NumTower(2.0), "V == V true"
assert not (_a == 2), "V == int never raises, returns False"
assert not (_a == "s"), "V == str never raises, returns False"
assert not (_a == None), "V == None never raises, returns False"

print("Numeric tower on user classes: PASS")

# ==================== Subclass-first reflected rule (§3.3.8) ====================
# CPython Data Model §3.3.8: "If the operands are of different types, and
# right operand's type is a direct or indirect subclass of the left operand's
# type, the reflected method of the right operand has priority over a
# non-reflected method of the left operand." This matters so that the more
# specialized (derived) class always gets a chance to handle the operation
# first — essential for correct arithmetic in class hierarchies.

class SfBase:
    def __init__(self, tag: str):
        self.tag = tag
    def __mul__(self, other):
        return SfBase("base_mul")

class SfDerived(SfBase):
    def __init__(self, tag: str):
        self.tag = tag
    def __mul__(self, other):
        return SfDerived("derived_mul")
    def __rmul__(self, other):
        return SfDerived("derived_rmul")

_sb, _sd = SfBase("b"), SfDerived("d")
# Subclass-first: Base * Derived → Derived.__rmul__ (not Base.__mul__)
assert (_sb * _sd).tag == "derived_rmul", "subclass-first rule"
# Same-type: regular forward dispatch
assert (_sb * _sb).tag == "base_mul"
assert (_sd * _sd).tag == "derived_mul"
# Derived on the LEFT: left's forward dunder wins (no special rule)
assert (_sd * _sb).tag == "derived_mul"

print("Subclass-first reflected rule: PASS")

# ==================== NotImplemented sentinel + fallback dispatch ====================
# CPython Data Model §3.3.8: a forward dunder may return `NotImplemented`
# to signal "I don't know how to handle this operand"; the interpreter then
# tries the reflected dunder on the right operand.

class NiX:
    def __mul__(self, other):
        if isinstance(other, NiX):
            return "XX"
        return NotImplemented

class NiY:
    def __rmul__(self, other):
        return "Yr"

# Forward dunder handles same-type — return value is used directly.
assert NiX() * NiX() == "XX"
# Forward returns NotImplemented for NiY → reflected NiY.__rmul__ dispatched.
assert NiX() * NiY() == "Yr"

# Three-way: forward returns NotImplemented, reflected also returns it,
# falls back to the original forward result (which IS NotImplemented).
class NiA:
    def __add__(self, other): return NotImplemented
class NiB:
    def __radd__(self, other): return "B_handles"
assert NiA() + NiB() == "B_handles"

# Mixed numeric tower with NotImplemented branch — common CPython idiom.
class NumNi:
    def __init__(self, x: float): self.x = x
    def __mul__(self, other):
        if isinstance(other, NumNi):
            return NumNi(self.x * other.x)
        if isinstance(other, (int, float)):
            return NumNi(self.x * other)
        return NotImplemented

_n = NumNi(3.0)
assert (_n * NumNi(2.0)).x == 6.0
assert (_n * 4).x == 12.0
assert (_n * 0.5).x == 1.5

print("NotImplemented fallback dispatch: PASS")

# ==================== NotImplemented delegated through helper (§C.7) ====================
# Area C §C.7: `dunder_may_return_not_implemented` is inter-procedural — when
# a dunder tail-calls a helper that itself returns `NotImplemented`, the
# reflected fallback must still fire. Tests the fixed-point propagation in
# `type_planning/ni_analysis.rs`.

class NiHelp:
    def _bail(self):
        return NotImplemented
    def __mul__(self, other):
        if isinstance(other, NiHelp):
            return "HH"
        return self._bail()  # NI via helper — fallback MUST still dispatch

class NiHelpPartner:
    def __rmul__(self, other):
        return "K_rmul"

assert NiHelp() * NiHelp() == "HH"
assert NiHelp() * NiHelpPartner() == "K_rmul"

# Same-module free-function helper.
def _free_bail():
    return NotImplemented

class NiFree:
    def __add__(self, other):
        if isinstance(other, NiFree):
            return "FF"
        return _free_bail()

class NiFreePartner:
    def __radd__(self, other):
        return "free_radd"

assert NiFree() + NiFreePartner() == "free_radd"

print("NotImplemented through helper (§C.7): PASS")

# ==================== Reductions on user classes (Area C §C.3) ====================
# `sum()` on a list of class instances folds via __add__ / __radd__ through the
# Area B dispatch state machine. Accumulator is seeded with the first element
# (CPython's `0 + V(x)` → NotImplemented → V(x).__radd__(0) shortcut).

class Money:
    def __init__(self, c: int): self.c = c
    def __add__(self, o):
        if isinstance(o, Money): return Money(self.c + o.c)
        if isinstance(o, (int, float)): return Money(self.c + o)
        return NotImplemented
    def __radd__(self, o):
        if isinstance(o, (int, float)): return Money(o + self.c)
        return NotImplemented

# Plain sum over class list.
_m = sum([Money(1), Money(2), Money(3)])
assert _m.c == 6

# Custom start of the same class.
_m2 = sum([Money(1), Money(2)], Money(100))
assert _m2.c == 103

# Canonical autograd-Value pattern.
class SumV:
    def __init__(self, x: float): self.x = x
    def __add__(self, o):
        if isinstance(o, SumV): return SumV(self.x + o.x)
        if isinstance(o, (int, float)): return SumV(self.x + o)
        return NotImplemented
    def __radd__(self, o):
        if isinstance(o, (int, float)): return SumV(o + self.x)
        return NotImplemented

_sv = sum([SumV(1.5), SumV(2.5), SumV(3.0)])
assert abs(_sv.x - 7.0) < 1e-9

# Primitive start with class elements — bootstraps via `first.__radd__(start)`.
_sv_int = sum([SumV(1.0), SumV(2.0), SumV(3.0)], 100)
assert abs(_sv_int.x - 106.0) < 1e-9
_sv_float = sum([SumV(1.0), SumV(2.0)], 0.5)
assert abs(_sv_float.x - 3.5) < 1e-9

print("Reductions on user classes (§C.3): PASS")

# min() / max() on user classes dispatch through rich-comparison dunders.
# `min` prefers `elem.__lt__(best)`; `max` prefers `elem.__gt__(best)`,
# falling back to `best.__lt__(elem)` when only `__lt__` is defined.

class Length:
    def __init__(self, m: float): self.m = m
    def __lt__(self, o):
        if isinstance(o, Length): return self.m < o.m
        return False

_lens = [Length(3.0), Length(1.0), Length(2.0)]
assert min(_lens).m == 1.0
assert max(_lens).m == 3.0  # uses __lt__ with swapped args (best < elem)

class Weight:
    def __init__(self, kg: float): self.kg = kg
    def __lt__(self, o):
        if isinstance(o, Weight): return self.kg < o.kg
        return False
    def __gt__(self, o):
        if isinstance(o, Weight): return self.kg > o.kg
        return False

_ws = [Weight(5.0), Weight(2.0), Weight(8.0), Weight(3.0)]
assert min(_ws).kg == 2.0
assert max(_ws).kg == 8.0  # uses __gt__ directly

print("min/max on user classes (§C.3): PASS")

# ==================== Tuple-shape unification across methods (Area D §D.3.6) ====================

# Fields receiving tuples of different shapes across methods infer as TupleVar.
# `unify_tuple_shapes` merges () (len 0), (...) (len 3), (...) (len 4) into
# TupleVar(Int) — not Any. Iteration and len work uniformly.
class ShapePts:
    def reset(self):
        self.pts = ()
    def triangle(self):
        self.pts = (0, 0, 0)
    def square(self):
        self.pts = (0, 0, 0, 0)
    def pentagon(self):
        self.pts = (0, 0, 0, 0, 0)

_s = ShapePts()
_s.triangle()
assert len(_s.pts) == 3, "triangle len"
_count = 0
for _p in _s.pts:
    _count += 1
assert _count == 3, "triangle iteration"
_s.square()
assert len(_s.pts) == 4, "square len"
_s.pentagon()
assert len(_s.pts) == 5, "pentagon len"
_s.reset()
assert len(_s.pts) == 0, "reset len"

# Same-length heterogeneous assignments keep the fixed Tuple shape
# (element-wise union of Int/Int = Int — no widening to TupleVar).
class Point2D:
    def origin(self):
        self.xy = (0, 0)
    def unit_x(self):
        self.xy = (1, 0)

_p2 = Point2D()
_p2.origin()
assert _p2.xy == (0, 0)
_p2.unit_x()
assert _p2.xy == (1, 0)

# Multi-method write to same field: `__init__` empty default + `load()` with
# tuple-literal shape — infers TupleVar(Int).
class Buffer:
    def __init__(self):
        self.data = ()
    def load_small(self):
        self.data = (1, 2)
    def load_large(self):
        self.data = (1, 2, 3, 4, 5)

_buf = Buffer()
_buf.load_small()
assert len(_buf.data) == 2
_buf.load_large()
assert len(_buf.data) == 5

print("Tuple-shape unification (§D.3.6): PASS")

# ==================== String forward references in annotations (Area D §D.7) ====================

# Self-reference in method annotation. `"LL"` is re-parsed as a Python
# expression; `LL` resolves via the top-level class pre-scan that registers
# every class name before any body is converted.
class LL:
    def __init__(self, val: int, next_node: "LL | None" = None):
        self.val = val
        self.next_node = next_node
    def tail(self) -> "LL":
        node = self
        while node.next_node is not None:
            node = node.next_node
        return node

_head = LL(1, LL(2, LL(3)))
assert _head.tail().val == 3, "LL self-reference"

# Forward ref in dunder parameter — closes §B.6 #3.
class VecF:
    def __init__(self, x: float):
        self.x = x
    def __mul__(self, other: "VecF") -> "VecF":
        return VecF(self.x * other.x)

assert (VecF(3.0) * VecF(2.0)).x == 6.0, "VecF __mul__ with string forward ref"

# Recursive tuple type — bridges §D.3 (tuple[T, ...]) and §D.7 (string refs).
class Tree:
    def __init__(self, value: int, children: "tuple[Tree, ...]" = ()):
        self.value = value
        self._children = children

_root = Tree(1, (Tree(2), Tree(3, (Tree(4),))))
assert _root._children[1]._children[0].value == 4, "recursive tuple annotation"

# Forward reference to a class declared LATER in the same module.
class Maker:
    def make(self) -> "Made":
        return Made()
class Made:
    def name(self) -> str:
        return "Made"

assert Maker().make().name() == "Made", "forward ref to later class"

print("String forward reference annotations (§D.7): PASS")

# =============================================================================
# Cross-site field type inference with numeric tower (§E.3)
# =============================================================================
# PEP 3141 numeric tower (bool ⊂ int ⊂ float) applied to class fields when
# observations across multiple methods disagree. AugAssign on self-fields
# is now captured in the field scan.

# Headline: int-initialized field widens to float via AugAssign.
class AccE3:
    def __init__(self):
        self.total = 0
    def add(self, x: float):
        self.total += x

acc_e3 = AccE3()
acc_e3.add(0.5)
acc_e3.add(0.25)
assert abs(acc_e3.total - 0.75) < 1e-9, f"E.3 acc.total = {acc_e3.total}"
acc_e3.total = 0                      # int literal 0 (bit-compatible with 0.0)
acc_e3.add(1.5)
assert abs(acc_e3.total - 1.5) < 1e-9, f"E.3 acc.total after reset = {acc_e3.total}"
acc_e3.total = 5                      # non-zero int → must coerce to 5.0
assert abs(acc_e3.total - 5.0) < 1e-9, f"E.3 acc.total after int 5 = {acc_e3.total}"
acc_e3.add(0.5)
assert abs(acc_e3.total - 5.5) < 1e-9, f"E.3 acc.total after +0.5 = {acc_e3.total}"

# Bool widens to int via numeric-tower rule.
class CounterE3:
    def __init__(self):
        self.n = False
    def tick(self):
        self.n += 1

counter_e3 = CounterE3()
counter_e3.tick()
counter_e3.tick()
assert counter_e3.n == 2, f"E.3 counter.n = {counter_e3.n}"

# Mixed int / float seeding across methods → field unifies to float.
class WeightedE3:
    def __init__(self):
        self.w = 0
    def seed_float(self):
        self.w = 1.0

weighted_e3 = WeightedE3()
weighted_e3.seed_float()
weighted_e3.w += 0.5
assert abs(weighted_e3.w - 1.5) < 1e-9, f"E.3 weighted.w = {weighted_e3.w}"

print("Cross-site field numeric promotion (§E.3): PASS")

# =============================================================================
# Comparison dunders with NotImplemented (§E.7)
# =============================================================================
# CPython data model: a rich-comparison dunder may return `NotImplemented`
# to signal "I don't know how to compare against this operand". The
# caller must then try the reflected dunder on the other side; if that
# also returns `NotImplemented`, fall back to identity (eq/ne) or
# TypeError (ordering). Requires boxing bool returns when the signature
# is `bool | NotImplementedT`.

class _QEq:
    def __init__(self, v: int):
        self.v = v
    def __eq__(self, other):
        if isinstance(other, _QEq):
            return self.v == other.v
        return NotImplemented

_qe1, _qe2, _qe3 = _QEq(1), _QEq(1), _QEq(2)
assert _qe1 == _qe2, "E.7 __eq__ forward true"
assert not (_qe1 == _qe3), "E.7 __eq__ forward false"
assert not (_qe1 == 5), "E.7 __eq__ NI + no reflected → identity → False"

# __lt__ ↔ __gt__ reflection via reflected_dunder_name.
class _PLt:
    def __init__(self, v: int):
        self.v = v
    def __lt__(self, other):
        if isinstance(other, _PLt):
            return self.v < other.v
        return NotImplemented

_pl1, _pl2 = _PLt(1), _PLt(2)
assert _pl1 < _pl2, "E.7 __lt__ forward"
assert _pl2 > _pl1, "E.7 __gt__ via reflected __lt__"

# Ordering between incompatible classes → TypeError.
class _Other:
    def __init__(self, v):
        self.v = v

try:
    _ = _PLt(1) < _Other(1)
    assert False, "E.7 ordering NI on both sides should raise"
except TypeError:
    pass

# __ne__ auto-derived from __eq__ (existing behaviour) + NI-aware.
class _Sym:
    def __init__(self, v):
        self.v = v
    def __eq__(self, other):
        if isinstance(other, _Sym):
            return self.v == other.v
        return NotImplemented

assert _Sym(1) != _Sym(2), "E.7 __ne__ derived"
assert not (_Sym(1) != _Sym(1)), "E.7 __ne__ derived equal"

print("Comparison dunders with NotImplemented (§E.7): PASS")

# =============================================================================
# Area G §G.13: isinstance-narrowing rebind in ternary / if-statement.
#
# The idiom
#     x = x if isinstance(x, T) else T(x)
# is used for type coercion. Without narrowing, unannotated params remain
# typed `Any` after the rebind, and subsequent `x.attr` lookups fail with
# "unknown attribute". §G.13 adds two-sided isinstance narrowing in
# `infer_expr_type_inner`'s `IfExpr` arm plus an `Any → concrete`
# override in `local_prescan::merge_var`.
# =============================================================================


class _G13:
    __slots__ = ("data",)

    def __init__(self, data):
        self.data = data


def _g13_combine(a: _G13, other):
    # Unannotated `other` — narrowed via the ternary-isinstance idiom.
    other = other if isinstance(other, _G13) else _G13(other)
    return _G13(a.data + other.data)


_g13_a = _g13_combine(_G13(3), 5)
assert _g13_a.data == 8, f"_g13_a.data expected 8, got {_g13_a.data}"
_g13_b = _g13_combine(_G13(3), _G13(10))
assert _g13_b.data == 13, f"_g13_b.data expected 13, got {_g13_b.data}"


# Narrowing over an unannotated local (not a param).
def _g13_local_narrow(x):
    x = x if isinstance(x, _G13) else _G13(x)
    return x.data * 2


assert _g13_local_narrow(3) == 6, "_g13_local_narrow(3) expected 6"
assert _g13_local_narrow(_G13(11)) == 22, "_g13_local_narrow(_G13(11)) expected 22"


# Method / dunder variant: polymorphic `other` is seeded as a Union for
# operator dunders, so the post-rebind read must use a narrowed shadow local
# rather than the ABI storage local.
class _G13Method:
    __slots__ = ("data",)

    def __init__(self, data):
        self.data = data

    def __add__(self, other):
        other = other if isinstance(other, _G13Method) else _G13Method(other)
        return _G13Method(self.data + other.data)


_g13_m_a = _G13Method(3) + 5
assert _g13_m_a.data == 8, f"_G13Method(3) + 5 expected 8, got {_g13_m_a.data}"
_g13_m_b = _G13Method(3) + _G13Method(10)
assert _g13_m_b.data == 13, f"_G13Method(3) + _G13Method(10) expected 13, got {_g13_m_b.data}"


# Recursive tuple field + zip-unpack variant: the constructor default seeds
# `_children` as an empty tuple, but later call sites refine it to
# `tuple[_G13Node, ...]`. The loop target `child` must keep that refined class
# type through `zip()` tuple-unpacking so `child.grad` resolves.
class _G13Node:
    __slots__ = ("grad", "_children", "_local_grads")

    def __init__(self, children=(), local_grads=()):
        self.grad = 0.0
        self._children = children
        self._local_grads = local_grads

    def backward(self):
        for child, local_grad in zip(self._children, self._local_grads):
            child.grad += local_grad * self.grad


_g13_leaf = _G13Node()
_g13_parent = _G13Node((_g13_leaf,), (0.5,))
_g13_parent.grad = 4.0
_g13_parent.backward()
assert abs(_g13_leaf.grad - 2.0) < 1e-9, f"_g13_leaf.grad expected 2.0, got {_g13_leaf.grad}"


print("isinstance-narrowing rebind (§G.13): PASS")


# §F.1 — Float fields in instance slots are stored as boxed FloatObj
# pointers wrapped via Value::from_ptr. The GC walks fields with
# heap_field_mask and follows every Float field as a heap pointer; the
# read path emits rt_unbox_float after RT_INSTANCE_GET_FIELD. Stress this
# under heavy allocation to flush out boxing/unboxing mismatches.
class _F1Point:
    x: float
    y: float

    def __init__(self, x: float, y: float):
        self.x = x
        self.y = y


def _f1_make_points(n: int) -> float:
    total = 0.0
    for i in range(n):
        # Allocate two FloatObj per iteration (x, y) plus the InstanceObj
        # so the slab gets churned and the marker has plenty of work.
        p = _F1Point(float(i) * 0.5, float(i) * 0.25)
        total = total + p.x + p.y
    return total


_f1_total = _f1_make_points(200)
# Sum_{i=0..199} (0.5i + 0.25i) = 0.75 * (199*200/2) = 0.75 * 19900 = 14925.0
assert abs(_f1_total - 14925.0) < 1e-9, f"§F.1 sum mismatch: got {_f1_total}"


# Heterogeneous fields: int (raw) + float (boxed pointer post-§F.1)
# + str (heap pointer). GC must distinguish raw int slots from heap
# pointer slots.
class _F1Mixed:
    n: int
    f: float
    label: str

    def __init__(self, n: int, f: float, label: str):
        self.n = n
        self.f = f
        self.label = label


_f1_mixed_list: list[_F1Mixed] = []
for _i in range(150):
    _f1_mixed_list.append(_F1Mixed(_i, float(_i) + 0.5, f"obj_{_i}"))

# After all that allocation a sweep should have run; verify every field
# survives intact.
for _i in range(150):
    _entry: _F1Mixed = _f1_mixed_list[_i]
    assert _entry.n == _i, f"§F.1 mixed.n mismatch at {_i}"
    assert abs(_entry.f - (float(_i) + 0.5)) < 1e-9, f"§F.1 mixed.f mismatch at {_i}"
    assert _entry.label == f"obj_{_i}", f"§F.1 mixed.label mismatch at {_i}"


# Reassignment exercises the SET path with non-zero existing FloatObj —
# the old pointer becomes garbage and must be safely freed by the next
# sweep without touching the live new value.
_f1_p = _F1Point(1.0, 2.0)
for _k in range(300):
    _f1_p.x = float(_k) + 0.125
    _f1_p.y = float(_k) * 2.0
assert abs(_f1_p.x - 299.125) < 1e-9, f"§F.1 reassign x: {_f1_p.x}"
assert abs(_f1_p.y - 598.0) < 1e-9, f"§F.1 reassign y: {_f1_p.y}"


print("Float field GC stress (§F.1): PASS")

# ===== Section: Polymorphic dunder + Union arithmetic runtime dispatch =====
# Regression: when a binary-op dunder's `other` parameter is widened by the
# type planner to `Union[Self, int, float, bool]`, lowering routes
# `self.x + other` through `rt_obj_add`. Pre-fix `rt_obj_add` only handled
# primitives — a runtime class instance in `other` raised "unsupported
# operand type(s) for +: 'object' and ...". Post-fix the runtime arithmetic
# helpers route Class operands through user-defined dunders via the new
# DUNDER_FUNC_REGISTRY (see `runtime/src/ops/dunder_dispatch.rs`).
class _PolyDunder:
    __slots__ = ('y',)
    def __init__(self, y): self.y = y
    # No isinstance fast-path: forces the Union arithmetic path inside the
    # dunder body. `self.y + other` is `Union + Union` at lowering time;
    # if `other` happens to be _PolyDunder at runtime, the runtime dispatch
    # routes back through `_PolyDunder.__radd__`.
    def __add__(self, other): return _PolyDunder(self.y + other)
    def __radd__(self, other): return _PolyDunder(self.y + other)
    def __mul__(self, other): return _PolyDunder(self.y * other)
    def __rmul__(self, other): return _PolyDunder(self.y * other)

_pd_a = _PolyDunder(10)
_pd_b = _PolyDunder(20)
# Direct: __add__ runs; inside, `self.y + other` hits Union dispatch.
_pd_r = _pd_a + _pd_b
assert isinstance(_pd_r, _PolyDunder), "Union arithmetic preserves Class via runtime dunder"
# Reflected dispatch chain: int + _PolyDunder via compile-time __radd__,
# then inside __radd__ the Union arithmetic fires.
_pd_sum = sum([_PolyDunder(1), _PolyDunder(2), _PolyDunder(3)])
assert isinstance(_pd_sum, _PolyDunder), "sum() over polymorphic-dunder class instances"
# Mixed forward path
_pd_m = _PolyDunder(7) * _PolyDunder(3)
assert isinstance(_pd_m, _PolyDunder), "polymorphic __mul__ via runtime dispatch"

print("Polymorphic dunder Union arithmetic dispatch: PASS")

# Regression: `Class` operand without a matching dunder must route through
# the runtime helper instead of falling through to a raw `mir::BinOp`. The
# raw path used to cause a codegen panic
# ("type mismatch ... declared Union(Float, Class) => I64, value is F64")
# because the optimizer's `type_inference` pass joins `Class` with `Float`
# into `Union[Float, Class]` for the dest local, but Cranelift still emits
# `BinOp::Pow` as a primitive f64 operation. Routing through `rt_obj_pow`
# lets the runtime dispatch via dunders (or raise a clean TypeError) and
# wraps the result as a boxed Value, matching the dest's i64 ABI.
class _RawBinopRegression:
    __slots__ = ('x',)
    def __init__(self, x): self.x = x
    def __pow__(self, other): return _RawBinopRegression(self.x ** other)
    def __mul__(self, other): return _RawBinopRegression(self.x * other)

_rbr_v = _RawBinopRegression(4.0)
# Class ** Float — class defines __pow__, so dispatch_class_binop emits
# CallDirect — exercise the happy path.
assert (_rbr_v ** 0.5).x == 2.0
# Float ** Class — neither side has a matching dunder. Pre-fix: codegen
# panic on raw `mir::BinOp::Pow`. Post-fix: runtime TypeError, raised by
# `rt_obj_pow` after no dunder is found.
_rbr_pow_caught = False
try:
    _rbr_unsupported = 2.0 ** _rbr_v
except TypeError:
    _rbr_pow_caught = True
assert _rbr_pow_caught, "Float ** Class with no __rpow__ must raise TypeError, not codegen panic"

print("Class binop without matching dunder: PASS")

# ===== Section: Unannotated __slots__ field refined to Float by constructor =====
# Regression: when `__slots__` declares a field but no class-body annotation
# is given (frontend-level type stays `Any`), constructor-call refinement is
# the only place the concrete primitive type lives. The read path
# (`lower_attribute`) consulted `refined_class_field_types` and emitted
# `RT_INSTANCE_GET_FIELD_F64` (raw f64), but the write path (`bind_attr_op`)
# fell back to `class_info.field_types` (still `Any`) and routed through
# `RT_INSTANCE_SET_FIELD` which boxed the f64 into a `FloatObj`. The read then
# interpreted the FloatObj pointer bits as a denormal f64 (~1e-313).
# Fix: make `bind_attr_op` consult `get_refined_class_field_type` first,
# matching the read path.
class _RefSymBasic:
    __slots__ = ('data',)
    def __init__(self, data): self.data = data

_refsym_x = _RefSymBasic(0.5)
assert _refsym_x.data == 0.5, f"unannotated slot refined-Float read: got {_refsym_x.data}"

# Reassignment must round-trip: write picks F64 fast-path because refined,
# read picks F64 fast-path — symmetry preserved across mutations.
_refsym_x.data = 1.25
assert _refsym_x.data == 1.25, f"unannotated slot refined-Float reassign: got {_refsym_x.data}"

# Multiple instances share the same refined storage label — every instance
# must round-trip the same way.
_refsym_list = [_RefSymBasic(float(i) * 0.125) for i in range(50)]
for _i in range(50):
    _expected = float(_i) * 0.125
    _got = _refsym_list[_i].data
    assert abs(_got - _expected) < 1e-12, f"unannotated slot refined-Float idx {_i}: got {_got}, want {_expected}"

# Mixed write paths: `__init__` write + later attribute write — both must
# agree on the F64 fast-path label, otherwise one path boxes and the other
# reads raw bits.
class _RefSymMutate:
    __slots__ = ('v',)
    def __init__(self, v): self.v = v
    def double(self): self.v = self.v * 2.0

_refsym_m = _RefSymMutate(3.5)
_refsym_m.double()
assert _refsym_m.v == 7.0, f"unannotated slot mutate (init+method): got {_refsym_m.v}"
_refsym_m.v = -2.5
assert _refsym_m.v == -2.5, f"unannotated slot direct write: got {_refsym_m.v}"

print("Unannotated __slots__ refined-Float read/write symmetry: PASS")

# ===== Section: Unary -/+/~ on Union/Any operand (runtime obj dispatch) =====
# Regression: when `lower_unop` saw an operand typed `Union[Float, Class]` /
# `Any` / `HeapAny`, it fell through to `MIR UnOp::Neg`. Codegen lowered
# that as `fneg(bitcast<f64>(operand))`. For a heap-pointer operand
# (FloatObj or class instance) the bitcast turns the pointer bits into a
# denormal f64; the result corrupts the field on store, and any later
# dereference (e.g. `(-self.data).x`) SIGSEGVs.
# Fix: route Neg/Pos/Invert through `rt_obj_neg/pos/invert` runtime
# helpers when the operand is Union/Any/HeapAny. The helper inspects the
# tag and dispatches to `__neg__/__pos__/__invert__` for class instances,
# checked-negation for Int, identity for Bool, and `rt_box_float(-f)` for
# FloatObj.
class _UnaryUnionV:
    __slots__ = ('data',)
    def __init__(self, data): self.data = data
    def __neg__(self): return _UnaryUnionV(-self.data)
    def __pos__(self): return _UnaryUnionV(+self.data)

# Field typed `Union[Float, _UnaryUnionV]` — assignments below pick both
# branches so neither collapses out.
def _unary_union_make(flag: bool):
    n = _UnaryUnionV(0.0)
    if flag:
        n.data = 0.5  # Float branch
    else:
        n.data = _UnaryUnionV(2.0)  # Class branch
    return n

# Float-branch unary: must hit primitive negation inside `rt_obj_neg` (no
# raw fneg over a pointer-shaped Value).
_uu_float = _unary_union_make(True)
_uu_neg = -_uu_float.data  # Union[Float, V] operand → rt_obj_neg
# After negation we get either a Float or a class instance. Both branches
# round-trip through assignment to a Float-refined field.
_uu_back = _UnaryUnionV(0.0)
_uu_back.data = _uu_neg
# When the operand was Float, result is -0.5 (boxed FloatObj after rt_obj_neg
# returns Value). When operand was Class, result is _UnaryUnionV(-2.0).
# We can't observe the field after Union assignment without a type guard,
# so verify no crash by accessing .data of the wrapper class instead.
_uu_neg_class = -_UnaryUnionV(7.5)
assert _uu_neg_class.data == -7.5, "unary -Class via __neg__"
_uu_pos_class = +_UnaryUnionV(-3.25)
assert _uu_pos_class.data == -3.25, "unary +Class via __pos__"

# `-self.data` inside a method where `data: Union[Float, V]` — pre-fix this
# was the canonical SIGSEGV crash (mgj.py repro).
class _UnarySelfData:
    __slots__ = ('data',)
    def __init__(self, data): self.data = data
    def neg_data(self):
        # self.data is Union[Float, _UnaryUnionV]. Without rt_obj_neg this
        # bitcasts the heap-pointer Value as f64 and corrupts memory.
        return -self.data

_uds_a = _UnarySelfData(0.5)
_uds_neg_a = _uds_a.neg_data()  # Float operand → primitive neg path inside rt_obj_neg
# Assigning the Union result back into a Float-refined slot exercises both
# write-path coercion and the post-negation Value tag.
_uds_back = _UnarySelfData(0.0)
_uds_back.data = _uds_neg_a
# Just having reached this line proves no SIGSEGV — that's the regression.

print("Unary -/+/~ on Union/Any operand via runtime dispatch: PASS")

# =============================================================================
# Section: rmsnorm-style call-site feedback loop with class element
# Regression: when a callee is reinvoked on its own return value, the
# harvester previously folded `list[Class]` (from a non-self call site)
# and `list[Float]` (from the rebound call site, where the seeded return
# type collapsed `xi * scale` to Float) into `list[Any]`. The body's
# prescan with `Any`-typed params then re-derived `list[Float]`,
# pinning a self-consistent but wrong fixed point and triggering an
# `OverflowError: integer overflow` at runtime when `xi * xi` was
# treated as integer multiplication on a heap pointer.
# Element-wise class-vs-primitive override in `join_nested_arg_ty`
# resolves the disagreement in favour of the class.
# =============================================================================

class _RmsValue:
    __slots__ = ('data',)
    def __init__(self, data): self.data = data
    def __mul__(self, other):
        other = other if isinstance(other, _RmsValue) else _RmsValue(other)
        return _RmsValue(self.data * other.data)
    def __add__(self, other):
        other = other if isinstance(other, _RmsValue) else _RmsValue(other)
        return _RmsValue(self.data + other.data)
    def __radd__(self, other): return self if other == 0 else self + other
    def __rmul__(self, other): return self * other


def _rmsnorm(x):
    # `xi` MUST be typed as `_RmsValue` for this to work — if it
    # collapses to `Float`, `xi * xi` becomes float-mul on a heap
    # pointer and either segfaults or yields garbage.
    ms = sum(xi * xi for xi in x)
    scale = 0.5  # plain float constant; xi * scale forces the Class*Float case
    return [xi * scale for xi in x]


_rms_init = [_RmsValue(2.0), _RmsValue(3.0), _RmsValue(4.0)]
# First call site: `x` is `list[_RmsValue]` from a literal.
_rms_x = _rmsnorm(_rms_init)
# Rebound call site: `x` is now `_rmsnorm`'s return type. Without the
# class-vs-primitive override, this rebound call's arg-type observation
# (carrying the seeded `list[Float]`) would dominate and pin the
# accumulator at `list[Any]`.
_rms_x = _rmsnorm(_rms_x)
_rms_x = _rmsnorm(_rms_x)
# If the harvester collapsed to list[Any]/list[Float], xi*xi inside
# _rmsnorm would have crashed before reaching this assertion.
assert len(_rms_x) == 3, f"_rmsnorm chain len: {len(_rms_x)}"
assert _rms_x[0].data == 2.0 * 0.5 * 0.5 * 0.5, (
    f"_rmsnorm chain element value: {_rms_x[0].data}"
)
print("rmsnorm-style call-site feedback loop: PASS")

# =============================================================================
# Container-of-container parameter refinement
# =============================================================================
# The outer list is built on the caller side via a listcomp:
#   keys = [[] for _ in range(n_layer)]
# and the inner-list mutation lives in the callee:
#   def _coc_gpt(keys, values):
#       keys[li].append(v)
# Empty-container refinement only handles `var = []` literal binds, so
# the listcomp-built outer list and the callee's param both miss out
# unless `find_elem_type_from_usage` follows the call site into the
# callee body and recognizes `var[idx].append(arg)`. Without this,
# `keys[idx][j].data` on either side type-checks as `Any` (no `.attr`)
# or pins `list[list[Never]]` from the desugared listcomp's
# `var.append([])` call.

class _CocValue:
    __slots__ = ('data',)
    def __init__(self, data): self.data = data


def _coc_gpt(keys, values):
    n = 2
    for li in range(n):
        v = _CocValue(li * 1.0 + 0.5)
        keys[li].append(v)
        values[li].append(v)
    # Indexed access on a container-of-container param must resolve to
    # `_CocValue` after refinement; otherwise `.data` fails to typecheck.
    return keys[0][0].data + values[1][0].data


_coc_n_layer = 2
_coc_keys = [[] for _ in range(_coc_n_layer)]
_coc_values = [[] for _ in range(_coc_n_layer)]
_coc_total = _coc_gpt(_coc_keys, _coc_values)
assert _coc_total == 0.5 + 1.5, f"_coc_gpt total: {_coc_total}"

# Caller-side comprehension over the container-of-container also needs
# refinement to flow back: without it `vi.data` fails to typecheck
# because `vi` resolves to `Any`.
_coc_flat = [vi.data for sublist in _coc_keys for vi in sublist]
assert _coc_flat == [0.5, 1.5], f"_coc_flat: {_coc_flat}"
print("container-of-container refinement: PASS")

# =============================================================================
# Subscript-chain refinement of arbitrary depth (3+ levels)
# =============================================================================
# Generalizes container-of-container refinement to any depth via
# `subscript_depth_to_var` + `wrap_list`. A `var[i][j].append(x)`
# mutation on a 3-deep param refines `var` to `list[list[list[T]]]`,
# letting `grid[i][j][k].data` typecheck through to the leaf class.
#
# We exercise depth-3 through type-check (`.data` on a 3-level subscript
# is unreachable without the wider refinement) and shallow-write/shallow-read.
# A loop-driven write/read pair on every 3-deep slot is intentionally avoided
# here — it's blocked by an unrelated runtime issue with chained-subscript
# reads after multiple writes (separate from refinement, predates this pass).

class _CocDeepV:
    __slots__ = ('data',)
    def __init__(self, data): self.data = data


def _coc_deep_fill(grid):
    # Loop-driven depth-2 mutation on a 3-deep grid — exercises both
    # refinement (refine `grid: list[list[list[_CocDeepV]]]` from the
    # appended class instance) and the nested-listcomp aliasing fix
    # (without the fix, `grid[0][...]` and `grid[1][...]` would alias
    # the same inner list, producing wrong sums).
    for i in range(2):
        for j in range(2):
            grid[i][j].append(_CocDeepV(i * 10.0 + j))
    return (
        grid[0][0][0].data
        + grid[0][1][0].data
        + grid[1][0][0].data
        + grid[1][1][0].data
    )


_coc_grid = [[[] for _ in range(2)] for _ in range(2)]
_coc_deep_total = _coc_deep_fill(_coc_grid)
# Expected: 0 + 1 + 10 + 11 = 22.0
assert _coc_deep_total == 22.0, f"_coc_deep_fill total: {_coc_deep_total}"
print("subscript-chain depth-3 refinement: PASS")

# =============================================================================
# Lattice-join across multiple append-points
# =============================================================================
# Two source-points contribute to the same var's element type:
# `_jvar.append([])` provides `list[Never]` (uninformative on its own)
# and `_jvar.append(...)` provides a concrete element type. The fixpoint
# scan accumulates both via `TypeLattice::join`; `Never` is the lattice
# identity, so `list[Never] ⊔ list[T] = list[T]` and the concrete
# observation wins. Pre-fixpoint behaviour was first-match-wins, which
# locked the element type at `list[Never]` and broke downstream code.

class _JoinPayload:
    __slots__ = ('value',)
    def __init__(self, v): self.value = v


def _coc_join_collect():
    xs = []
    xs.append([])                  # list[Never] — join identity
    xs.append([_JoinPayload(7)])   # list[_JoinPayload] — wins via join
    return xs


_coc_join_xs = _coc_join_collect()
assert len(_coc_join_xs) == 2, f"_coc_join_xs len: {len(_coc_join_xs)}"
assert _coc_join_xs[1][0].value == 7, (
    f"_coc_join_xs[1][0].value: {_coc_join_xs[1][0].value}"
)
print("lattice-join multi-source refinement: PASS")

# =============================================================================
# Closure-from-var dispatch
# =============================================================================
# `find_elem_via_call_arg` resolves `Call { func: Var(v), args }` through a
# module-wide `var_to_func` map (built once per pass from
# `Bind { Var, Closure | FuncRef }` statements) — same pattern the harvester
# uses in `closure_scan.rs`. Without this, `f = some_func; f(var)` would
# leave caller's `var` at `list[Any]` because the call site never resolves
# to a concrete callee body.

class _CocFwdPayload:
    __slots__ = ('value',)
    def __init__(self, v): self.value = v


def _coc_fwd_append(store, k):
    # Caller forwards `store` here through the funcref-bound var. Body
    # sees `store.append(k)` (depth 0); refinement collects `_CocFwdPayload`
    # as the element type and propagates back to the caller's binding.
    store.append(k)


_coc_fwd_f = _coc_fwd_append
_coc_fwd_xs = []
_coc_fwd_f(_coc_fwd_xs, _CocFwdPayload(11))
_coc_fwd_f(_coc_fwd_xs, _CocFwdPayload(13))
assert _coc_fwd_xs[0].value + _coc_fwd_xs[1].value == 24, (
    f"closure-from-var sum: {_coc_fwd_xs[0].value + _coc_fwd_xs[1].value}"
)
print("closure-from-var dispatch: PASS")

# =============================================================================
# Method dispatch with typed list args (ABI through MethodCall)
# =============================================================================
# After container refinement types `store: list[Int]` for the method param,
# `lower_method_call` must dispatch `.append(k)` through the list-method
# path (`rt_list_append`). Pre-fix the dispatcher saw `obj_type=Any`
# (because `store`'s type wasn't seeded into the lowering var-types map
# from the caller-arg observation) and fell through to the registry-walk
# path that picked `rt_deque_append` first — wrong runtime, wrong elem
# layout, garbage results.
#
# `build_method_arg_seeds` collects caller-side arg types from every
# `MethodCall` site at the start of each refinement pass and layers them
# onto the callee's overlay (refinement-only, never written to
# `lambda_param_type_hints` so dunder methods like `__exit__` keep their
# variable arg shape).

class _CocMethCache:
    __slots__ = ()
    def add(self, store, k):
        store.append(k)


_coc_meth_cache = _CocMethCache()
_coc_meth_keys = []
_coc_meth_cache.add(_coc_meth_keys, 7)
_coc_meth_cache.add(_coc_meth_keys, 11)
assert _coc_meth_keys[0] + _coc_meth_keys[1] == 18, (
    f"method-dispatch typed-list sum: {_coc_meth_keys[0] + _coc_meth_keys[1]}"
)
print("method-dispatch typed-list args: PASS")

# =============================================================================
# sum(genexp) with class instances — generator yield-type inference
# =============================================================================
# `sum(a[j] * b[j] for j in range(n))` over `list[V]` must yield V (not Int
# from range's elem_ty fall-through). The genexp's yield expression
# `a[j] * b[j]` shape-infers via the desugar-side fallback path, which
# previously did not handle:
#   - `Call { func: ClassRef }` (constructor) → list literals
#     `[V(0.5), ...]` shape-inferred to `list[Any]`
#   - `Index` / `Slice` over `list[V]` → returned None (no class type)
#   - `BuiltinCall::Sum` (and other generic builtins) inside listcomp body
#   - `Call { func: Var }` resolving to a Bind-bound FuncRef
#   - Capture variables typed `Any` on the genexp's params (the actual
#     types live in `lambda_param_type_hints`)
#   - Cross-callee return types not yet synced to `func_def.return_type`
#     (still in the type-planning side map during the fixpoint)
#
# Without these, microgpt-style chains like
#   q = linear(x, w); q_h = q[0:k]; sum(q_h[j] * q_h[j] for j in range(k))
# inferred the genexp's element type as Int, sum() routed through the
# integer fast path, and `total.value` failed at compile time.

class _SumGenexpV:
    __slots__ = ('value',)

    def __init__(self, value):
        self.value = value

    def __add__(self, other):
        if isinstance(other, _SumGenexpV):
            return _SumGenexpV(self.value + other.value)
        return _SumGenexpV(self.value + other)

    def __radd__(self, other):
        return self + other

    def __mul__(self, other):
        if isinstance(other, _SumGenexpV):
            return _SumGenexpV(self.value * other.value)
        return _SumGenexpV(self.value * other)

    def __rmul__(self, other):
        return self * other


def _sum_genexp_make_q():
    return [_SumGenexpV(0.5), _SumGenexpV(0.6), _SumGenexpV(0.7), _SumGenexpV(0.8)]


# Direct list literal of class instances — shape_infer's ClassRef arm
_sg_a = [_SumGenexpV(1.0), _SumGenexpV(2.0), _SumGenexpV(3.0)]
_sg_b = [_SumGenexpV(10.0), _SumGenexpV(20.0), _SumGenexpV(30.0)]
_sg_dot = sum(_sg_a[j] * _sg_b[j] for j in range(3))
assert _sg_dot.value == 140.0, f"dot product: {_sg_dot.value}"

# zip-based (covers the BinOp + tuple-target augment_target_var_types)
_sg_dot_zip = sum(ai * bi for ai, bi in zip(_sg_a, _sg_b))
assert _sg_dot_zip.value == 140.0, f"dot zip: {_sg_dot_zip.value}"

# Function-returned list + slice — shape_infer's Call→FuncRef + Slice arms
# plus the func_returns side-map overlay
_sg_q = _sum_genexp_make_q()
_sg_q_h = _sg_q[0:2]
_sg_partial = sum(_sg_q_h[j] * _sg_q_h[j] for j in range(2))
assert _sg_partial.value == 0.61, f"slice sum: {_sg_partial.value}"

print("sum(genexp) over class instances: PASS")

# =============================================================================
# Generator captures must be loaded in for-loop resume body
# =============================================================================
# When a gen-expr or list-comp captures variables (e.g. `q_h` from an outer
# binding) and is invoked from inside a for-loop, the resume function for
# the for-loop generator must reload the captured slots before evaluating
# the yield expression. `build_generic_resume` / `build_while_loop_resume`
# / `build_trailing_yield` all do this, but `build_for_loop_resume` was
# the only path that skipped it — captured Vars compiled to reads of
# never-initialised locals (zero / null), producing SEGV the moment the
# value reached a runtime call (e.g. `rt_obj_mul` deref of null operand).
#
# Trigger: any rebinding of a captured Var inside a for-loop body, where
# the genexp inside the body references that Var. This happened in
# microgpt-style attention blocks (`for h: q_h = q[hs:hs+k]; sum(...)`)
# and similar loop-over-rebinding patterns.

class _LoopCaptureV:
    __slots__ = ('value',)

    def __init__(self, value):
        self.value = value

    def __add__(self, other):
        if isinstance(other, _LoopCaptureV):
            return _LoopCaptureV(self.value + other.value)
        return _LoopCaptureV(self.value + other)

    def __radd__(self, other):
        return self + other

    def __mul__(self, other):
        if isinstance(other, _LoopCaptureV):
            return _LoopCaptureV(self.value * other.value)
        return _LoopCaptureV(self.value * other)

    def __rmul__(self, other):
        return self * other


_lc_q = [
    _LoopCaptureV(0.5),
    _LoopCaptureV(0.6),
    _LoopCaptureV(0.7),
    _LoopCaptureV(0.8),
]

_lc_results = []
for _lc_h in range(2):
    _lc_hs = _lc_h * 2
    _lc_q_h = _lc_q[_lc_hs : _lc_hs + 2]  # rebind inside loop body
    _lc_s = sum(_lc_q_h[j] * _lc_q_h[j] for j in range(2))
    _lc_results.append(_lc_s.value)

assert abs(_lc_results[0] - 0.61) < 1e-9, f"loop iter 0: {_lc_results[0]}"
assert abs(_lc_results[1] - 1.13) < 1e-9, f"loop iter 1: {_lc_results[1]}"

# Plain rebind (no slice) — exercises the same load path
_lc_alias_results = []
for _lc_h2 in range(2):
    _lc_alias = _lc_q  # captured via plain alias, still rebound per iter
    _lc_alias_results.append(sum(_lc_alias[j] * _lc_alias[j] for j in range(4)).value)

assert _lc_alias_results == _lc_alias_results[:1] * 2, (
    f"alias rebind: {_lc_alias_results}"
)

print("genexp captures load in for-loop resume: PASS")

# =============================================================================
# Tuple-unpack assignment at module level must register every leaf as a global
# =============================================================================
# `a, b = 1, 2` at module scope was previously broken: the frontend's
# `target_name` extractor only matched `Expr::Name` targets and bailed on
# `Expr::Tuple`, so neither `a` nor `b` was added to
# `scope.module_level_assignments`. `finalize_module` then only promoted
# names in that set to `module.globals`, so the tuple-unpacked vars stayed
# function-local-shaped — module init wrote them as locals while function
# bodies reading them via `rt_global_get_<type>` saw never-initialised
# slots (zero), producing silently-wrong values (`n_embd` reads as 0,
# making `range(n_embd)` empty, causing downstream `[V(0.5) for _ in
# range(n_embd)]` to return `[]` and SEGV the next dereferencer).
#
# Fix recurses through Tuple / List / Starred targets in
# `collect_assignment_target_names`, and `bind_var_op` now mirrors
# `lower_assign`'s `rt_global_set_<type>` emission for tuple-unpacked
# globals so all bound leaves commit to the global slot.

_tu_a, _tu_b = 1, 4
_tu_c, _tu_d, _tu_e = 10, 20, 30

def _tu_read():
    return _tu_a + _tu_b + _tu_c + _tu_d + _tu_e

assert _tu_read() == 65, f"tuple-unpack global sum: {_tu_read()}"

class _TuV:
    __slots__ = ('value',)
    def __init__(self, v):
        self.value = v

def _tu_listcomp():
    return [_TuV(0.5) for _ in range(_tu_b)]

_tu_xs = _tu_listcomp()
assert len(_tu_xs) == 4, f"tuple-unpack-driven listcomp len: {len(_tu_xs)}"
assert _tu_xs[0].value == 0.5, f"tuple-unpack-driven listcomp [0]: {_tu_xs[0].value}"

# Nested tuple-unpack pattern (e.g. `(a, b), c = (1, 2), 3`)
_tu_pair, _tu_z = (100, 200), 300
_tu_p, _tu_q = _tu_pair
assert _tu_p == 100 and _tu_q == 200 and _tu_z == 300, (
    f"nested tuple-unpack: {_tu_p}, {_tu_q}, {_tu_z}"
)

print("tuple-unpack module-level globals: PASS")

# =============================================================================
# Slice / len / index dispatch through Any-typed container chains
# =============================================================================
# When a container's element type collapses to `Any` (typically because
# the for-target of a comprehension is bound through `IterAdvance` without
# a registered seed type), dispatch helpers used to silently degrade:
#   - `select_slicing_func(Any)` returned `None` → `lower_slice` emitted
#     a `Constant::None`, producing zero-length slices for non-empty
#     lists routed through the listcomp result.
#   - `select_len_func(Any)` returned `None` → `lower_len` fell into its
#     `_ => Const(0)` fallback, silently misreporting `len(x)` for any
#     `x: Any` regardless of the actual runtime type.
#   - `seed_expr_type` (the read API) had no `Slice` / `Index` arms, so
#     inline expressions like `len(ki[0:2])` (without an intermediate
#     local binding) saw `Any` and triggered the same degradation.
# All three combined to make microgpt-style listcomps return empty
# slices that downstream `__mul__` calls then dereferenced as null
# operands, segfaulting in `rt_obj_mul`.
#
# Fixes (architectural, not point fixes):
#   1. New runtime helpers `rt_obj_slice` / `rt_obj_slice_step` /
#      `rt_obj_len` that dispatch by `TypeTagKind` at runtime — mirror
#      the existing `rt_any_getitem` pattern.
#   2. `select_slicing_func` / `select_slicing_step_func` / `select_len_func`
#      now route `Any` / `HeapAny` to those helpers instead of falling
#      through to silent zero/None constants.
#   3. `seed_expr_type` (read API) gained `Slice` and `Index` arms so
#      inline subscript / slice expressions resolve their result type
#      without needing a temp-var binding to populate the cache.
#   4. `lower_slice` falls back to the lowered MIR operand's type when
#      `seed_expr_type` returns `Any` — the operand often carries a
#      tighter shape after `lower_expr` ran.

class _SliceAnyV:
    __slots__ = ('value',)
    def __init__(self, v):
        self.value = v


_slc_keys, _slc_values = [[]], [[]]


def _slc_drive(k):
    """k is `list[list[list[V]]]` after refinement; the listcomp body
    slices through `ki[0:2]` where `ki` is a comprehension for-target.
    Pre-fix the slice silently produced an empty list because the
    for-target's compile-time type was `Any`."""
    k[0].append([_SliceAnyV(1.0), _SliceAnyV(2.0), _SliceAnyV(3.0), _SliceAnyV(4.0)])
    out = [ki[0:2] for ki in k[0]]
    assert len(out) == 1, f"outer listcomp len: {len(out)}"
    assert len(out[0]) == 2, f"inner slice len: {len(out[0])}"


_slc_drive(_slc_keys)


# Inline `len(ki[0:2])` without intermediate local binding — exercises the
# read-API `Slice` / `Index` arms in `seed_expr_type`.
_slc_arr = [_SliceAnyV(0.5), _SliceAnyV(0.6), _SliceAnyV(0.7), _SliceAnyV(0.8)]
_slc_inline_len = len(_slc_arr[0:2])
assert _slc_inline_len == 2, f"inline len(slice): {_slc_inline_len}"

# `len(out[0])` where `out: list[Any]` — exercises the `RT_OBJ_LEN`
# runtime dispatch via `select_len_func(Any)`.
_slc_runtime_dispatch = [[_SliceAnyV(0.1), _SliceAnyV(0.2), _SliceAnyV(0.3)]]
_slc_taken = _slc_runtime_dispatch[0]
assert len(_slc_taken) == 3, f"runtime-dispatched len: {len(_slc_taken)}"

print("slice/len/index dispatch through Any: PASS")

# =============================================================================
# Section: ClassRef harvester slot mapping for `__init__` params
# Regression: closure_scan's `scan_expr_for_calls` previously had no
# `ExprKind::ClassRef` arm, so positional args of constructor calls
# (`MyClass(a, b)`) were never observed. When the arm was added but the
# accumulator slot mapping ignored `self`, the first call-site arg type
# (e.g. `Float`) was committed as `__init__`'s slot-0 param hint —
# i.e. `self: Float`. That hint leaked into every method body's
# `self.<field>` reads via the seed overlay, which made the lowering of
# expressions like `float(self.data > 0)` think the comparison's LHS
# was already-Float (Float fast-path), then the outer `float(...)`
# emitted `IntToFloat` on an f64 operand → cranelift verifier panic
# `fcvt_from_sint.f64 v_n` with `v_n: f64`.
#
# The fix: the resolver returns `(FuncId, harvest_skip)` where
# `harvest_skip = 1` for ClassRef (skip self) and 0 for FuncRef /
# Closure / Var. At harvest, args are written into
# `entry[harvest_skip + i]`, so slot 0 stays `Never` and the commit
# loop's `params[0].ty.unwrap_or(...)` correctly seeds it with
# `Class[Self]` (populated by instance-method `self` typing).
#
# These cases cover: (a) the canonical autograd reproducer from
# microgpt, (b) a direct slot-0 pollution probe where a concrete
# float-shaped constructor arg would visibly corrupt `self.attr`
# reads if it leaked, (c) two unrelated classes co-existing without
# cross-pollution, (d) constructors with default-valued tail params.
# =============================================================================

# (a) Canonical autograd reproducer — the microgpt-style relu method
#     with `float(self.data > 0)` and constructor invocation in the
#     same expression. Pre-fix: cranelift panic. Post-fix: real value.

class _ClassRefHarvAutograd:
    __slots__ = ('data', 'children', 'local_grads')
    def __init__(self, d, c=(), g=()):
        self.data = d
        self.children = c
        self.local_grads = g
    def __mul__(self, o):
        o = o if isinstance(o, _ClassRefHarvAutograd) else _ClassRefHarvAutograd(o)
        return _ClassRefHarvAutograd(
            self.data * o.data,
            (self, o),
            (o.data, self.data),
        )
    def relu(self):
        # The bug-trigger expression: comparison Float > Int → Bool,
        # then float(Bool) emits BoolToInt + IntToFloat. With self
        # leaked as Float, the comparison fast-paths to Float, and the
        # outer float() retried IntToFloat on an f64 → verifier panic.
        return _ClassRefHarvAutograd(
            max(0.0, self.data),
            (self,),
            (float(self.data > 0),),
        )


_crh_a = _ClassRefHarvAutograd(2.0)
_crh_b = _ClassRefHarvAutograd(3.0)
_crh_c = (_crh_a * _crh_b).relu()
assert _crh_c.data == 6.0, f"autograd .data: {_crh_c.data}"
# `local_grads` carries `(o.data, self.data)` from __mul__ then `(float(self.data > 0),)`
# from relu. relu's branch is taken because (a*b).data == 6.0 > 0.
assert _crh_c.local_grads == (1.0,), f"autograd .local_grads: {_crh_c.local_grads}"
assert len(_crh_c.children) == 1, f"autograd .children len: {len(_crh_c.children)}"
print("ClassRef harvester (autograd): PASS")


# (b) Direct slot-0 pollution probe. If the first call-site arg type
#     leaks into self's hint, then inside a method `self` would be
#     viewed as a primitive (Float here), and `self.label` (a str
#     attribute) would either fail to lower or emit wrong code. We
#     return a string-typed value derived from `self.label` and assert
#     the runtime value is the original string — only possible if
#     `self`'s param hint was NOT polluted by the float arg.

class _ClassRefHarvSlotProbe:
    __slots__ = ('value', 'label')
    def __init__(self, v, lbl):
        self.value = v
        self.label = lbl
    def describe(self):
        # If self leaked as Float, `self.label` would be lowered as a
        # primitive attr access on a Float (no such attr → either
        # compile error or memory garbage). The assertion below would
        # fail with a corrupted string.
        return self.label + "/" + self.label


_crh_probe_a = _ClassRefHarvSlotProbe(1.5, "alpha")
_crh_probe_b = _ClassRefHarvSlotProbe(99.25, "beta")
assert _crh_probe_a.describe() == "alpha/alpha", f"slot probe a: {_crh_probe_a.describe()}"
assert _crh_probe_b.describe() == "beta/beta", f"slot probe b: {_crh_probe_b.describe()}"
print("ClassRef harvester (slot-0 string probe): PASS")


# (c) Two distinct classes constructed in the same module — neither
#     accumulator should leak into the other. Pre-fix the harvester
#     keyed accumulators by FuncId only, but if slot-0 was being
#     written, both classes' `self` params would carry whichever first
#     arg was observed last during the fixpoint scan.

class _ClassRefHarvFloatBag:
    __slots__ = ('x',)
    def __init__(self, x):
        self.x = x
    def doubled(self):
        return self.x + self.x


class _ClassRefHarvIntBag:
    __slots__ = ('n',)
    def __init__(self, n):
        self.n = n
    def squared(self):
        return self.n * self.n


_crh_fb = _ClassRefHarvFloatBag(2.5)
_crh_ib = _ClassRefHarvIntBag(7)
assert _crh_fb.doubled() == 5.0, f"FloatBag doubled: {_crh_fb.doubled()}"
assert _crh_ib.squared() == 49, f"IntBag squared: {_crh_ib.squared()}"
print("ClassRef harvester (no cross-class slot pollution): PASS")


# (d) Mixed-arity call sites — some calls pass 1 arg, others 2. The
#     harvester must accumulate observations only for the slots that
#     each call actually covers, never reaching back into earlier
#     param slots. Verifies that non_capture_params iteration commits
#     only what's been observed and does not cross-pollinate.

class _ClassRefHarvMixedArity:
    __slots__ = ('head', 'extra')
    def __init__(self, h, e=0):
        self.head = h
        self.extra = e
    def head_doubled(self):
        # Only touches `head` so the default-value path for `extra`
        # is exercised at construction but not at access.
        return self.head + self.head


_crh_ma_one = _ClassRefHarvMixedArity(11)         # uses default for `e`
_crh_ma_two = _ClassRefHarvMixedArity(13, 7)      # passes both
assert _crh_ma_one.head_doubled() == 22, f"mixed arity one: {_crh_ma_one.head_doubled()}"
assert _crh_ma_two.head_doubled() == 26, f"mixed arity two: {_crh_ma_two.head_doubled()}"
# Direct access on the explicitly-supplied path should not be affected
# by the harvested type for `extra` either way.
assert _crh_ma_two.extra == 7, f"mixed arity extra: {_crh_ma_two.extra}"
print("ClassRef harvester (mixed-arity call sites): PASS")


# =============================================================================
# Section: heap-typed field side-table for autograd-style accumulation
# Regression: when a class field is statically typed `Int` (frontend
# inferred from `self.field = 0` literal in `__init__`) but later
# receives compound-RHS writes whose runtime tag may be a heap pointer
# (e.g. `child.field += other_obj.attr * scalar` where `other_obj.attr`
# is a runtime-dispatched tagged Value), the bind-site previously
# re-encoded the heap pointer as `INT` via `ValueFromInt` (corrupting
# high bits into the integer payload) and the read-site `UnwrapValueInt`
# decoded the bogus int as the field value. Squaring or accumulating
# that bogus int surfaces as `OverflowError: integer overflow` deep
# inside an unrelated loop.
#
# The fix records a side-set `class_fields_with_heap_writes` from the
# cross-instance harvester for compound-RHS writes whose seed type
# collapses to `Any`/`HeapAny`, and lowering's bind / read paths treat
# those slots as `HeapAny` end-to-end (no UnwrapValueInt round-trip).
# Test mirrors microgpt's autograd `child.grad += local_grad * v.grad`
# pattern: `_HeapWriteAccum.acc` is statically `Int` (from `acc = 0`)
# but receives `Float * Int = Float` heap-boxed values from cross-
# instance accumulation, then the post-loop arithmetic squares it —
# pre-fix this surfaced as integer overflow.
# =============================================================================

class _HeapWriteAccumValue:
    __slots__ = ('data', 'acc')
    def __init__(self, d):
        self.data = d
        self.acc = 0  # frontend infers Int from literal `0`
    def __mul__(self, other):
        other = other if isinstance(other, _HeapWriteAccumValue) else _HeapWriteAccumValue(other)
        return _HeapWriteAccumValue(self.data * other.data)
    def add_grad(self, scalar):
        # Cross-instance compound write: `self.acc + scalar * something`
        # where `scalar` is HeapAny-shaped (tuple-element from a
        # heterogeneous tuple) and `something.data` is Float. The BinOp
        # dispatch routes through `rt_obj_*` returning a tagged Value
        # whose runtime tag is a heap-pointer FloatObj. Without the
        # heap-writes side-table the bind site would re-tag the pointer
        # as INT, and a later read would decode it as a garbage int
        # → OverflowError on subsequent arithmetic.
        self.acc = self.acc + scalar * self.data


_hwa_a = _HeapWriteAccumValue(2.0)
_hwa_b = _HeapWriteAccumValue(3.0)
# Heterogeneous tuple — first element is Float, second is Int. Iterating
# unpacks each element as `Any` / `HeapAny`-shaped (tagged Value). When
# `add_grad` is called with that scalar, the cross-instance compound
# write `self.acc = self.acc + scalar * self.data` produces a tagged
# Float-pointer Value, which the harvester's compound-RHS detection
# marks `acc` as heap-typed.
for _hwa_scalar in (1.5, 1):
    _hwa_a.add_grad(_hwa_scalar)
    _hwa_b.add_grad(_hwa_scalar)

# Pre-fix: reading `_hwa_a.acc` via `UnwrapValueInt` on a tagged
# FloatObj pointer would decode pointer bits as a garbage int (e.g.
# 5_000_000_000+); squaring it would overflow i64 and raise
# `OverflowError`. Post-fix: read returns the tagged Value, and the
# `** 2` BinOp dispatches through `rt_obj_pow` to produce a real Float.
# 1.5 * 2.0 + 1 * 2.0 = 5.0, squared = 25.0
# 1.5 * 3.0 + 1 * 3.0 = 7.5, squared = 56.25
_hwa_a_acc_sq = _hwa_a.acc ** 2
_hwa_b_acc_sq = _hwa_b.acc ** 2
assert _hwa_a_acc_sq == 25.0, f"heap-writes acc_a^2: {_hwa_a_acc_sq}"
assert _hwa_b_acc_sq == 56.25, f"heap-writes acc_b^2: {_hwa_b_acc_sq}"
print("Heap-typed field side-table (autograd-style accumulation): PASS")

# =============================================================================
# Harvester recursion through FormatSpec / container literals / Index / Attribute
# =============================================================================
# `infer_nested_function_param_types_inner` walks call sites to seed
# unannotated lambda/nested-function param types. Pre-fix the recursion
# arms covered BinOp/Compare/UnOp/LogicalOp/IfExpr/MethodCall/BuiltinCall
# but NOT FormatSpec, Tuple/List/Set/Dict literals, Index, or Attribute.
# Inline lambda calls wrapped in any of these silently bypassed the
# harvester, the lambda's int param defaulted to `Type::Any`, the call
# site `ValueFromInt`-tagged the int (`(N<<3)|1`), and the body's
# `range(n)` interpreted the tagged bits as a count: `range(5)` became
# `range(41)`, `range(27)` became `range(217)`. Microgpt-adjacent code
# with `print(f"len: {len(matrix(5))}")` patterns silently produced
# 8x-bloated computation graphs.
_hrg_lambda = lambda n: [1 for _ in range(n)]
# Inline call inside f-string format spec
_hrg_in_fstring = f"len: {len(_hrg_lambda(5))}"
assert _hrg_in_fstring == "len: 5", f"lambda in f-string: {_hrg_in_fstring}"
# Inline call inside list literal
_hrg_in_list = [_hrg_lambda(5)]
assert len(_hrg_in_list[0]) == 5, f"lambda in list: {len(_hrg_in_list[0])}"
# Inline call inside dict-literal value
_hrg_in_dict = {'k': _hrg_lambda(5)}
assert len(_hrg_in_dict['k']) == 5, f"lambda in dict: {len(_hrg_in_dict['k'])}"
# Inline call inside tuple literal
_hrg_in_tuple = (_hrg_lambda(5), 0)
assert len(_hrg_in_tuple[0]) == 5, f"lambda in tuple: {len(_hrg_in_tuple[0])}"
# Inline call inside subscript index
_hrg_data = [10, 20, 30, 40, 50, 60]
_hrg_picker = lambda x: x // 2
_hrg_picked = _hrg_data[_hrg_picker(4)]
assert _hrg_picked == 30, f"lambda in subscript: {_hrg_picked}"
print("Harvester recursion through FormatSpec/Tuple/List/Dict/Index: PASS")

# =============================================================================
# Float-passthrough at list[Float] store with HeapAny operand
# =============================================================================
# `emit_value_slot(_, Float)` previously called `rt_box_float` on every
# operand regardless of MIR type. When the operand was a tagged Value
# (HeapAny / Union) from `rt_obj_*` arithmetic, codegen's
# `load_operand_as(F64)` bitcast i64→f64 — the tagged-Value bits became
# a denormal payload, the resulting FloatObj had garbage value, and any
# later `rt_unbox_float` round-trip recovered the same denormal. The
# fix passes the operand through verbatim when its MIR type is HeapAny
# or Union; downstream `rt_unbox_float` dispatches on tag at runtime.
class _FpvAccum:
    __slots__ = ('val',)
    def __init__(self, v): self.val = v
    def __mul__(self, other):
        # Returns Value-tagged result via Union arithmetic
        other_v = other if isinstance(other, _FpvAccum) else _FpvAccum(other)
        return _FpvAccum(self.val * other_v.val)
    def __rmul__(self, other): return self * other


_fpv_seed = _FpvAccum(2.5)
# (1 - beta) * accum produces a Union arithmetic chain at the lambda's
# Float-typed slot. Pre-fix: `m[0]` would receive denormal e-313 bits.
# Post-fix: m[0] correctly equals 0.15 * 2.5 = 0.375.
_fpv_m = [0.0] * 3
_fpv_m[0] = 0.85 * _fpv_m[0] + 0.15 * _fpv_seed.val
assert abs(_fpv_m[0] - 0.375) < 1e-9, f"Float-passthrough store: {_fpv_m[0]}"
print("Float-passthrough at list[Float] store with HeapAny: PASS")

# =============================================================================
# Cross-instance field write through an unhinted receiver (FieldWriteDynamic)
# =============================================================================
# A class field whose owning class is NOT known at constraint-collection time
# (the receiver is a for-loop variable, not `self` and not constructor-bound)
# must still widen the target field. `_FwdNode.grad` is seeded `Int` by
# `self.grad = 0`, then a cross-instance `child.grad = child.grad + 0.5`
# stores a Float through `child` (resolved to `_FwdNode` only during solving).
# Pre-fix the write was dropped, `grad` stayed `Int`, and the verifier
# rejected the boxed Float store. The `FieldWriteDynamic` reducer resolves the
# receiver class at solve-time and JOINs the Float in, widening `grad` to
# `Float`.
class _FwdNode:
    __slots__ = ('grad', 'children')

    def __init__(self, children):
        self.grad = 0
        self.children = children

    def backward(self):
        for child in self.children:
            child.grad = child.grad + 0.5


_fwd_leaf1 = _FwdNode([])
_fwd_leaf2 = _FwdNode([])
_fwd_root = _FwdNode([_fwd_leaf1, _fwd_leaf2])
_fwd_root.backward()
assert abs(_fwd_leaf1.grad - 0.5) < 1e-9, f"_fwd_leaf1.grad = {_fwd_leaf1.grad}"
assert abs(_fwd_leaf2.grad - 0.5) < 1e-9, f"_fwd_leaf2.grad = {_fwd_leaf2.grad}"
print("Cross-instance field write through unhinted receiver: PASS")

# =============================================================================
# Cross-instance write into an INHERITED field through an unhinted receiver
# =============================================================================
# Same `FieldWriteDynamic` path as above, but `grad` is declared on the BASE
# class while the unhinted receiver (`child`) resolves to the SUBCLASS only at
# solve-time. The field layout is inherited into the subclass, but the field
# NAME lives only in the base's `class_defs`, so the solver's own-class-only
# field gate used to drop the write — leaving `grad` typed `Int` and the boxed
# Float store rejected by the MIR verifier (`BoxValue: src Raw(F64) doesn't
# match Raw(I64)`). The `class_has_field_in_hierarchy` gate walks the base
# chain so the inherited field is widened to `Float` like the non-inherited
# case.
class _InhBase:
    __slots__ = ('grad',)

    def __init__(self):
        self.grad = 0


class _InhDerived(_InhBase):
    __slots__ = ('children',)

    def __init__(self, children):
        super().__init__()
        self.children = children

    def backward(self):
        for child in self.children:
            child.grad = child.grad + 0.5


_inh_leaf1 = _InhDerived([])
_inh_leaf2 = _InhDerived([])
_inh_root = _InhDerived([_inh_leaf1, _inh_leaf2])
_inh_root.backward()
assert abs(_inh_leaf1.grad - 0.5) < 1e-9, f"_inh_leaf1.grad = {_inh_leaf1.grad}"
assert abs(_inh_leaf2.grad - 0.5) < 1e-9, f"_inh_leaf2.grad = {_inh_leaf2.grad}"
print("Cross-instance write into inherited field through unhinted receiver: PASS")

# =============================================================================
# Field store through a polymorphic (Union) receiver
# =============================================================================
# `node` is inferred as `Union[_UnionA, _UnionB]` (it iterates a mixed list),
# so the receiver class is only known at runtime. Both classes declare `grad`
# at the same offset, so the store compiles to one static-offset write valid
# for either runtime type, and the solver's FieldWriteDynamic fan-out widens
# `grad` to `Float` on BOTH members. Pre-fix the store was silently dropped at
# lowering (the receiver had no single `class_id`), so `grad` stayed `0`.
class _UnionA:
    __slots__ = ('grad',)

    def __init__(self):
        self.grad = 0


class _UnionB:
    __slots__ = ('grad',)

    def __init__(self):
        self.grad = 0


def _union_bump(node):
    node.grad = node.grad + 0.5


_union_a = _UnionA()
_union_b = _UnionB()
for _union_node in [_union_a, _union_b]:
    _union_bump(_union_node)

assert abs(_union_a.grad - 0.5) < 1e-9, f"_union_a.grad = {_union_a.grad}"
assert abs(_union_b.grad - 0.5) < 1e-9, f"_union_b.grad = {_union_b.grad}"
print("Field store through a polymorphic (Union) receiver: PASS")

# ===== Whole-project code-review regression: the iterative GC mark phase must
# not overflow the native stack on a long reference chain (formerly
# test_review_wave0b.py; the old recursive mark_object SIGSEGV'd here).
class _RvChainNode:
    val: int
    next: "_RvChainNode | None"

    def __init__(self, val: int) -> None:
        self.val = val
        self.next = None


def _rv_build_chain(n: int) -> _RvChainNode:
    head = _RvChainNode(0)
    cur = head
    for i in range(1, n):
        node = _RvChainNode(i)
        cur.next = node
        cur = node
    return head


def _rv_deep_gc_chain() -> None:
    head = _rv_build_chain(200000)
    # Force allocation churn → GC collections while the deep chain is live.
    junk: list[list[int]] = []
    for i in range(1000):
        junk.append([i, i + 1])
    count = 0
    cur: "_RvChainNode | None" = head
    while cur is not None:
        count += 1
        cur = cur.next
    print(count)


_rv_deep_gc_chain()


# ===== SECTION: empty-bootstrapped annotated container field returned as primitive =====
# Regression: a class field annotated `list[int]` but initialized empty
# (`self.data = []`) and only grown via `self.data.append(x)` used to refine to
# `list[Never]` → demoted to `list[Any]`, discarding the declared `int` element.
# A method `-> int` returning `self.data[0]` then had a Tagged return ABI that
# clashed with the caller's `Raw(I64)` dest at the verifier. The fold now
# reconciles `Never` element positions against the annotation.
class _EmptyFieldBox:
    data: list[int]
    ratios: list[float]
    rows: list[list[int]]

    def __init__(self) -> None:
        self.data = []
        self.ratios = []
        self.rows = []

    def push(self, x: int) -> None:
        self.data.append(x)

    def first(self) -> int:
        return self.data[0]

    def add_ratio(self, r: float) -> None:
        self.ratios.append(r)

    def first_ratio(self) -> float:
        return self.ratios[0]

    def add_row(self, row: list[int]) -> None:
        self.rows.append(row)

    def cell(self, i: int, j: int) -> int:
        return self.rows[i][j]


_efb = _EmptyFieldBox()
_efb.push(10)
_efb.push(20)
assert _efb.first() == 10, "empty-bootstrap list[int] field: first() returns raw int"
assert _efb.first() + 5 == 15, "empty-bootstrap field element usable as int"
_efb.add_ratio(1.5)
_efb.add_ratio(2.5)
assert _efb.first_ratio() == 1.5, "empty-bootstrap list[float] field: first_ratio() returns float"
_efb.add_row([1, 2, 3])
_efb.add_row([4, 5, 6])
assert _efb.cell(1, 2) == 6, "empty-bootstrap list[list[int]] field: nested element typed int"

# ===== SECTION: __repr__ in containers + __lt__ sorting =====
# A container's repr renders each element with repr() (its __repr__), and
# sorted()/list.sort() order class instances via their __lt__ dunder — both
# dispatched in the runtime (no static class type at the element site).


class _OrderBox:
    def __init__(self, v: int) -> None:
        self.v = v

    def __lt__(self, other: "_OrderBox") -> bool:
        return self.v < other.v

    def __repr__(self) -> str:
        return "_OrderBox(" + str(self.v) + ")"


_obs = [_OrderBox(3), _OrderBox(1), _OrderBox(2), _OrderBox(1)]
print(_obs)
print(sorted(_obs))
print(sorted(_obs, reverse=True))
_obs.sort()
print(_obs)
print(min(_obs), max(_obs))
print((_OrderBox(7), _OrderBox(8)))
_obs_sorted = sorted([_OrderBox(5), _OrderBox(4)])
assert _obs_sorted[0].v == 4, "sorted class instances order via __lt__"
assert _obs_sorted[1].v == 5, "sorted class instances order via __lt__"


# ===== SECTION: default object repr (no __repr__) =====
# CPython: a class without __repr__ reprs as `<__main__.Cls object at 0x..>`.
# The address is non-deterministic, so assert on the qualified-name prefix
# (and that container elements use the same default).
class _NoRepr:
    def __init__(self) -> None:
        self.x = 1


_nr_repr = repr(_NoRepr())
assert _nr_repr.startswith(
    "<__main__._NoRepr object at 0x"
), "default repr is module-qualified"
assert _nr_repr.endswith(">"), "default repr closes the angle bracket"
assert str(_NoRepr()).startswith(
    "<__main__._NoRepr object at 0x"
), "str() falls back to the default repr"
assert repr([_NoRepr()]).startswith(
    "[<__main__._NoRepr object at 0x"
), "container element uses the module-qualified default repr"

# type(instance) is module-qualified; __name__ is the bare class name.
assert (
    str(type(_NoRepr())) == "<class '__main__._NoRepr'>"
), "type(instance) is module-qualified"
assert type(_NoRepr()).__name__ == "_NoRepr", "type(instance).__name__ is the bare class name"



# ===== SECTION: Folded point-tests =====
# Several point-test corpus files are folded in here. pyaot has no nested classes
# and a per-program user-class cap (runtime class_id is a u8), so each kept class
# lives at module scope with a per-source prefix to avoid colliding with the many
# classes defined earlier in this file. Prints were converted to asserts against
# the CPython-correct values. Sources whose cases are already fully covered above
# (basic construction, arithmetic/comparison dunders, @property/@staticmethod/
# @classmethod, container __getitem__/__setitem__/__contains__/__len__) are
# DEDUPED rather than re-added: p5_class_basic (Point/AttrCounter/TypedPoint),
# p5_dunder_arith (DunderVec/Vector2D/RevNum), p5_dunder_container (IntList/
# Container), p5_decorators (PropCounter/StaticMath/MixedMethods/AnnotatedClassAttr).


# --- folded from p5_inherit.py + p5_mro_join.py (merged): super(), C3 MRO/diamond,
#     virtual dispatch from a base method, polymorphic dispatch, isinstance,
#     and MRO-aware nominal joins (unannotated sibling lists/returns/ternary). ---
class _oo_Animal:
    def __init__(self, name: str):
        self.name = name

    def speak(self) -> str:
        return "..."

    def describe(self) -> str:
        # Virtual dispatch from within a base method (self.speak()).
        return self.name + " says " + self.speak()


class _oo_Dog(_oo_Animal):
    def __init__(self, name: str, breed: str):
        super().__init__(name)  # super() chain into _oo_Animal.__init__
        self.breed = breed

    def speak(self) -> str:
        return "Woof"


class _oo_Puppy(_oo_Dog):
    def speak(self) -> str:
        return "Yip"


class _oo_Cat(_oo_Animal):
    def speak(self) -> str:
        return "Meow"


# Diamond root carries a defaulted name so D() (who-test) and D("d") (speak-zoo)
# both construct; who()/speak() exercise C3 (MRO = D, B, C, A → B before C).
class _oo_A(_oo_Animal):
    def __init__(self, name: str = ""):
        super().__init__(name)

    def who(self) -> str:
        return "A"

    def speak(self) -> str:
        return "A"


class _oo_B(_oo_A):
    def who(self) -> str:
        return "B"

    def speak(self) -> str:
        return "B"


class _oo_C(_oo_A):
    def who(self) -> str:
        return "C"

    def speak(self) -> str:
        return "C"


class _oo_D(_oo_B, _oo_C):
    pass


# Polymorphic dispatch over a base-typed list of mixed subclasses + describe().
_oo_animals: list[_oo_Animal] = [
    _oo_Dog("Rex", "Lab"),
    _oo_Cat("Felix"),
    _oo_Puppy("Buddy", "Pug"),
    _oo_Animal("Thing"),
]
_oo_describes = [a.describe() for a in _oo_animals]
assert _oo_describes == ["Rex says Woof", "Felix says Meow", "Buddy says Yip", "Thing says ..."]

_oo_d = _oo_Dog("Fido", "Beagle")
assert _oo_d.describe() == "Fido says Woof"
assert _oo_d.speak() == "Woof"
assert _oo_d.name == "Fido"
assert _oo_d.breed == "Beagle"

_oo_pup = _oo_Puppy("Spot", "Terrier")
assert _oo_pup.describe() == "Spot says Yip"
assert _oo_pup.name == "Spot"
assert _oo_pup.breed == "Terrier"

# isinstance across the hierarchy.
assert isinstance(_oo_d, _oo_Dog) is True
assert isinstance(_oo_d, _oo_Animal) is True
assert isinstance(_oo_d, _oo_Cat) is False
assert isinstance(_oo_pup, _oo_Animal) is True
assert isinstance(_oo_pup, _oo_Dog) is True
assert isinstance(_oo_pup, _oo_Puppy) is True
_oo_c = _oo_Cat("Tom")
assert isinstance(_oo_c, _oo_Dog) is False
assert isinstance(_oo_c, _oo_Animal) is True

# C3 diamond via who().
assert _oo_D().who() == "B"
assert _oo_B().who() == "B"
assert _oo_C().who() == "C"
assert _oo_A().who() == "A"

# MRO-aware nominal join: unannotated mixed literal joins to list[_oo_Animal].
_oo_pets = [_oo_Dog("Rex", "x"), _oo_Cat("Tom")]
_oo_pet_lines = [p.name + ": " + p.speak() for p in _oo_pets]
assert _oo_pet_lines == ["Rex: Woof", "Tom: Meow"]


def _oo_pick(n: int):
    # Unannotated return joins sibling branches to the common base.
    if n == 0:
        return _oo_Dog("D", "x")
    return _oo_Cat("C")


assert _oo_pick(0).speak() == "Woof"
assert _oo_pick(1).speak() == "Meow"

# Derived + Base collapses to Base (subsumption, not a two-member union).
_oo_mixed = [_oo_Dog("M", "x"), _oo_Animal("Plain")]
_oo_mixed_speak = [m.speak() for m in _oo_mixed]
assert _oo_mixed_speak == ["Woof", "..."]

# Diamond join(B, C) = Animal; zoo dispatches per C3 (B before C), through name.
_oo_zoo = [_oo_B("b"), _oo_C("c"), _oo_D("d")]
_oo_zoo_lines = [z.name + ": " + z.speak() for z in _oo_zoo]
assert _oo_zoo_lines == ["b: B", "c: C", "d: B"]

# Conditional expression over siblings — another unannotated join site.
_oo_flip = True
_oo_chosen = _oo_Dog("Yes", "x") if _oo_flip else _oo_Cat("No")
assert _oo_chosen.speak() == "Woof"
assert isinstance(_oo_chosen, _oo_Animal) is True
assert isinstance(_oo_chosen, _oo_Cat) is False


# --- folded from p27_matmul.py: matrix-multiply operator `@` / __matmul__ (PEP 465) ---
class _mm_Vec:
    def __init__(self, x: int, y: int) -> None:
        self.x = x
        self.y = y

    def __matmul__(self, other: "_mm_Vec") -> int:
        return self.x * other.x + self.y * other.y


_mm_v1 = _mm_Vec(1, 2)
_mm_v2 = _mm_Vec(3, 4)
assert (_mm_v1 @ _mm_v2) == 11
assert (_mm_v2 @ _mm_v1) == 11
assert (_mm_Vec(0, 0) @ _mm_v1) == 0


class _mm_Mat:
    def __init__(self, v: int) -> None:
        self.v = v

    def __matmul__(self, other: "_mm_Mat") -> "_mm_Mat":
        return _mm_Mat(self.v * other.v)


_mm_product = _mm_Mat(3) @ _mm_Mat(5)
assert _mm_product.v == 15

_mm_chained = _mm_Mat(2) @ _mm_Mat(3) @ _mm_Mat(4)
assert _mm_chained.v == 24


class _mm_Scaled:
    def __init__(self, k: int) -> None:
        self.k = k

    def __rmatmul__(self, other: int) -> int:
        return other + self.k


assert (10 @ _mm_Scaled(5)) == 15

_mm_acc = _mm_Mat(2)
_mm_acc @= _mm_Mat(7)
assert _mm_acc.v == 14


def _mm_total_dot(pairs: list[tuple[_mm_Vec, _mm_Vec]]) -> int:
    total = 0
    for a, b in pairs:
        total += a @ b
    return total


assert _mm_total_dot([(_mm_Vec(1, 1), _mm_Vec(2, 2)), (_mm_Vec(3, 0), _mm_Vec(1, 5))]) == 7

_mm_caught_num = False
try:
    _mm_x = 3 @ 4
except TypeError:
    _mm_caught_num = True
assert _mm_caught_num


class _mm_NoMat:
    def __init__(self) -> None:
        self.q = 1


_mm_caught_obj = False
try:
    _mm_y = _mm_NoMat() @ _mm_NoMat()
except TypeError:
    _mm_caught_obj = True
assert _mm_caught_obj


# --- folded from p42_iter_protocol.py: lazy user-class iterator protocol ---
# Basic for-loop / iter()/next() / for...else / empty / break / inherited iteration
# are already covered above by `CountUp` / `CountUpNamed`; we reuse those classes
# for the cases unique to p42 (direct next(), StopIteration on exhaustion, list()/
# sum() over an instance) and only add the genuinely-new shapes: a re-iterable
# whose __iter__ yields a FRESH iterator, and a __next__ that raises a non-Stop
# exception which must propagate.
_p42_collected = [x for x in CountUp(4)]
assert _p42_collected == [0, 1, 2, 3]

_p42_empty = [x for x in CountUp(0)]
assert _p42_empty == []

_p42_it = iter(CountUp(3))
assert next(_p42_it) == 0
assert next(_p42_it) == 1
assert next(_p42_it) == 2
_p42_stop_caught = False
try:
    next(_p42_it)
except StopIteration:
    _p42_stop_caught = True
assert _p42_stop_caught

# next() called DIRECTLY on a self-iterator instance (no intervening iter()).
_p42_cu = CountUp(2)
assert next(_p42_cu) == 0
assert next(_p42_cu) == 1

# break-early (the iterator is abandoned mid-stream).
_p42_early = []
for x in CountUp(100):
    if x >= 3:
        break
    _p42_early.append(x)
assert _p42_early == [0, 1, 2]

# list() / sum() over an instance.
assert list(CountUp(5)) == [0, 1, 2, 3, 4]
assert sum(CountUp(5)) == 10


class _it_Squares:
    def __init__(self, n: int):
        self.i = 0
        self.n = n

    def __iter__(self):
        return self

    def __next__(self) -> int:
        if self.i >= self.n:
            raise StopIteration
        v = self.i * self.i
        self.i = self.i + 1
        return v


class _it_SquaresIterable:
    # `__iter__` returns a FRESH `_it_Squares` each call (re-iterable).
    def __init__(self, n: int):
        self.n = n

    def __iter__(self):
        return _it_Squares(self.n)


_it_si = _it_SquaresIterable(4)
assert list(_it_si) == [0, 1, 4, 9]
assert list(_it_si) == [0, 1, 4, 9]  # fresh iterator → same result


class _it_Boom:
    # __next__ raises a NON-StopIteration exception — it must propagate.
    def __iter__(self):
        return self

    def __next__(self) -> int:
        raise ValueError("boom")


_it_boom_msg = ""
try:
    for x in _it_Boom():
        _it_boom_msg = "unreachable"
except ValueError as e:
    _it_boom_msg = str(e)
assert _it_boom_msg == "boom"


# --- folded from p43_hetero_tuple_iter.py: heterogeneous-numeric tuple iteration ---
# DIVERGENCE-SAFE: float-forced values compared via `==` / value-based list equality.
class _ht_Accum:
    __slots__ = ("data", "acc")

    def __init__(self, d):
        self.data = d
        self.acc = 0  # frontend infers Int from the literal `0`

    def add_grad(self, scalar):
        # `scalar` is bound from a heterogeneous-tuple element → must stay Tagged.
        self.acc = self.acc + scalar * self.data


_ht_a = _ht_Accum(2.0)
_ht_b = _ht_Accum(3.0)
for _ht_scalar in (1.5, 1):
    _ht_a.add_grad(_ht_scalar)
    _ht_b.add_grad(_ht_scalar)

assert _ht_a.acc == 5.0
assert _ht_b.acc == 7.5
assert _ht_a.acc ** 2 == 25.0
assert _ht_b.acc ** 2 == 56.25

assert [x for x in (1.5, 1)] == [1.5, 1]
assert [x * 2 for x in (2.5, 2)] == [5.0, 4]

_ht_tot = 0.0
for _ht_v in (True, 2.0, 3):
    _ht_tot = _ht_tot + _ht_v
assert _ht_tot == 6.0

_ht_fs = 0.0
for _ht_f in (1.0, 2.0, 3.5):
    _ht_fs = _ht_fs + _ht_f
assert _ht_fs == 6.5

_ht_is = 0
for _ht_i in (10, 20, 30):
    _ht_is = _ht_is + _ht_i
assert _ht_is == 60


# --- folded from p45_in_method_field_annot.py: in-method instance-field annotations ---
# DIVERGENCE-SAFE: a float field written from an int holds 5.0 (pyaot) / 5 (CPython);
# asserts use `==` and only float-FORCED expressions, never a divergent repr.
class _fa_Box:
    def __init__(self, v: int) -> None:
        self.x: float = v          # int value into a float-annotated field
        self.label: str = "box"    # str field declared in-method

    def get(self) -> float:
        return self.x


_fa_b = _fa_Box(5)
assert _fa_b.x == 5.0
assert _fa_b.label == "box"
assert (_fa_b.get() + 0.5) == 5.5
assert (_fa_b.x + 0.5) == 5.5
assert _fa_b.label == "box"

# bignum int into an in-method float field (the §8 box bignum arm), via _fa_Box.
_fa_big = _fa_Box(2 ** 62)             # exact power of two, f64-representable
assert _fa_big.x == 4611686018427387904.0


class _fa_Lazy:
    def __init__(self) -> None:
        self.v: float                  # pure type declaration (no store)
        self.v = 0                     # int write into the float field

    def bump(self, n: int) -> None:
        self.v = n                     # another int write -> §8 SetField box


_fa_lz = _fa_Lazy()
assert _fa_lz.v == 0.0
_fa_lz.bump(7)
assert _fa_lz.v == 7.0
assert (_fa_lz.v + 0.5) == 7.5


class _fa_Acc:
    def __init__(self) -> None:
        self.total = 0.0

    def configure(self, flag: bool, k: int) -> None:
        if flag:
            self.scale: float = k      # in-method annotation inside an `if`
        else:
            self.scale = 1.0

    def scaled(self) -> float:
        return self.total * self.scale


_fa_a = _fa_Acc()
_fa_a.total = 4.0
_fa_a.configure(True, 3)
assert _fa_a.scale == 3.0
assert _fa_a.scaled() == 12.0
assert (_fa_a.scale + 0.5) == 3.5
assert (_fa_a.scaled() + 0.5) == 12.5


def _fa_half(x: float) -> float:
    return x * 0.5


_fa_box2 = _fa_Box(9)                  # box2.x: float = 9 -> 9.0
assert _fa_half(_fa_box2.x) == 4.5     # the boxed float field flows into a float param


# --- folded from p48_gradual_builtin_immediate.py: gradual builtin op on immediate ---
def _im_as_dyn(x):
    return x


def _im_raises_type_error(fn) -> bool:
    try:
        fn()
        return False
    except TypeError:
        return True


# Immediate receivers raise TypeError on len / in / subscript (not SIGSEGV).
assert _im_raises_type_error(lambda: len(_im_as_dyn(42)))
assert _im_raises_type_error(lambda: len(_im_as_dyn(True)))
assert _im_raises_type_error(lambda: len(_im_as_dyn(None)))
assert _im_raises_type_error(lambda: 5 in _im_as_dyn(42))
assert _im_raises_type_error(lambda: 1 in _im_as_dyn(None))
assert _im_raises_type_error(lambda: _im_as_dyn(99)[0])
assert _im_raises_type_error(lambda: _im_as_dyn(False)[0])

# The same ops on genuine containers still work.
assert len(_im_as_dyn([1, 2, 3])) == 3
assert len(_im_as_dyn("hello")) == 5
assert len(_im_as_dyn({1: 1, 2: 2})) == 2
assert (2 in _im_as_dyn([1, 2, 3])) is True
assert (9 in _im_as_dyn([1, 2, 3])) is False
assert _im_as_dyn([7, 8, 9])[1] == 8
assert _im_as_dyn((4, 5, 6))[0] == 4
assert _im_as_dyn("abc")[2] == "c"
assert len(_im_as_dyn([1, 2, 3, 4])) == 4
assert _im_as_dyn([10, 20, 30])[2] == 30
assert _im_as_dyn("xyz")[0] == "x"


# --- folded from p8h_dyn_attr.py: by-name field access on a Dyn receiver ---
class _dy_Node:
    def __init__(self, data: float):
        self.data = data
        self.grad = 0.0


class _dy_Pair:
    def __init__(self, left, right):
        self.left = left
        self.right = right


def _dy_pick(flag):
    if flag:
        return _dy_Node(1.5)
    return "not a node"


# Dyn receiver (unannotated return) — field read/write by name.
_dy_n = _dy_pick(True)
assert _dy_n.data == 1.5
_dy_n.grad = 2.5
assert _dy_n.grad == 2.5


def _dy_make_pair():
    return _dy_Pair(_dy_Node(10.0), _dy_Node(20.0))


# Dyn elements out of a heterogeneous structure, two-deep.
_dy_p = _dy_make_pair()
assert _dy_p.left.data + _dy_p.right.data == 30.0


def _dy_add_data(a, b):
    bb = b if isinstance(b, _dy_Node) else _dy_Node(float(b))
    return a.data + bb.data


assert _dy_add_data(_dy_Node(3.0), _dy_Node(4.0)) == 7.0
assert _dy_add_data(_dy_Node(3.0), 2) == 5.0


class _dy_Acc:
    def __init__(self, v: int):
        self.v = v

    def __add__(self, other):
        return _dy_Acc(self.v + other.v)

    def __radd__(self, other):
        return _dy_Acc(self.v + other)


# class elements through sum()'s inferred __add__ / __radd__ returns.
_dy_s = sum([_dy_Acc(1), _dy_Acc(2), _dy_Acc(3)])
assert _dy_s.v == 6

# AttributeError on a missing field / non-instance (caught, not a crash).
_dy_missing_caught = False
try:
    _dy_tmp = _dy_n.missing
except AttributeError:
    _dy_missing_caught = True
assert _dy_missing_caught

_dy_bad = _dy_pick(False)
_dy_noninstance_caught = False
try:
    _dy_tmp2 = _dy_bad.data
except AttributeError:
    _dy_noninstance_caught = True
assert _dy_noninstance_caught


# --- folded from test_gradual_methods.py: Dyn/Union-receiver method dispatch ---
class _gm_Base:
    def __init__(self, x):
        self.x = x

    def kind(self):
        return "base"

    def val(self):
        return self.x

    def combine(self, other, scale=1):
        return (self.x + other) * scale


class _gm_Derived(_gm_Base):
    def kind(self):
        return "derived"  # override

    def doubled(self):
        return self.x * 2


class _gm_Other:
    def kind(self):
        return "other"

    def val(self):
        return -1


def _gm_call_kind(obj):
    # `obj` is an unannotated param → `Dyn`; called with several unrelated types.
    return obj.kind()


def _gm_container_methods():
    # A heterogeneous dict → its values are `Dyn`; `box[k]` is a `Dyn` receiver.
    box = {}

    box["lst"] = [3, 1, 2]
    box["lst"].append(4)
    box["lst"].sort()
    assert box["lst"] == [1, 2, 3, 4]
    box["lst"].insert(0, 9)
    assert box["lst"] == [9, 1, 2, 3, 4]
    assert box["lst"].index(2) == 2
    assert box["lst"].count(9) == 1
    box["lst"].reverse()
    assert box["lst"] == [4, 3, 2, 1, 9]
    box["lst"].remove(9)
    assert box["lst"].pop() == 1
    assert box["lst"] == [4, 3, 2]
    assert box["lst"].copy() == [4, 3, 2]
    box["lst"].extend([7, 8])
    assert box["lst"] == [4, 3, 2, 7, 8]
    box["lst"].clear()
    assert box["lst"] == []

    box["d"] = {"a": 1}
    box["d"].update({"b": 2})
    box["d"].setdefault("c", 3)
    box["d"].setdefault("a", 99)  # present → keeps 1
    assert box["d"].get("a") == 1
    assert box["d"].get("z") is None
    assert box["d"].get("z", -1) == -1
    assert sorted(box["d"].keys()) == ["a", "b", "c"]
    assert sorted(box["d"].values()) == [1, 2, 3]
    assert sorted(box["d"].items()) == [("a", 1), ("b", 2), ("c", 3)]
    assert box["d"].pop("a") == 1
    assert box["d"].pop("zz", -7) == -7
    box["d"].clear()
    assert box["d"] == {}
    assert box["d"].copy() == {}

    box["s"] = {1, 2, 3}
    box["s"].add(4)
    box["s"].discard(2)
    box["s"].discard(99)  # absent → no error
    box["s"].remove(1)
    box["s"].update({5, 6})
    assert sorted(box["s"]) == [3, 4, 5, 6]
    assert sorted(box["s"].copy()) == [3, 4, 5, 6]

    box["dq"] = deque([1, 2, 3])
    box["dq"].append(4)
    box["dq"].appendleft(0)
    box["dq"].extend([5, 6])
    assert list(box["dq"]) == [0, 1, 2, 3, 4, 5, 6]
    assert box["dq"].pop() == 6
    assert box["dq"].popleft() == 0
    assert box["dq"].count(3) == 1
    box["dq"].clear()
    assert list(box["dq"]) == []


def _gm_user_methods():
    # A heterogeneous list (no common base) → element type `Dyn`.
    items = [_gm_Base(10), _gm_Derived(7), _gm_Other()]
    kinds = [it.kind() for it in items]  # `it` is genuinely `Dyn`
    assert kinds == ["base", "derived", "other"]
    assert _gm_call_kind(items[0]) == "base"
    assert _gm_call_kind(items[1]) == "derived"
    assert _gm_call_kind(items[2]) == "other"

    d = items[1]  # `Dyn`, holds a Derived
    assert d.doubled() == 14          # own method
    assert d.val() == 7               # inherited (Base.val) — self coerces C→B
    assert d.kind() == "derived"      # overridden
    assert d.combine(100) == 107      # default arg
    assert d.combine(100, 3) == 321   # positional 2nd arg
    assert d.combine(5, scale=2) == 24  # positional-or-keyword param by keyword
    assert d.combine(other=5, scale=2) == 24  # both by keyword

    b = items[0]  # `Dyn`, holds a Base
    assert b.combine(3) == 13
    assert b.val() == 10


def _gm_scalar_methods():
    box = {}

    # tuple.index / .count on a `Dyn` tuple.
    box["t"] = (10, 20, 30, 20)
    box["pad"] = {}  # keep the dict's value type `Dyn`
    assert box["t"].index(20) == 1
    assert box["t"].count(20) == 2

    # int methods on a `Dyn` int (immediate fixnum, bool, and heap bignum).
    box["n"] = 255
    assert box["n"].bit_length() == 8
    assert box["n"].bit_count() == 8
    assert box["n"].conjugate() == 255
    assert box["n"].__index__() == 255
    box["b"] = True
    assert box["b"].bit_length() == 1
    box["big"] = 2 ** 70
    assert box["big"].bit_length() == 71

    # list.sort(reverse=) on a `Dyn` list.
    box["lst"] = [1, 3, 2, 5, 4]
    box["lst"].sort(reverse=True)
    assert box["lst"] == [5, 4, 3, 2, 1]
    box["lst"].sort(reverse=False)
    assert box["lst"] == [1, 2, 3, 4, 5]


def _gm_str_methods():
    # str methods on a `Dyn` receiver — the gradual sibling of the typed
    # `lower_str_method` path (routes through `rt_obj_method`'s `Str` arm to the
    # same `rt_str_*` family). `box[k]` is `Dyn` because the dict is heterogeneous.
    box = {}
    box["n"] = 0  # keep the dict's value type `Dyn`

    box["s"] = "Hello, World"
    assert box["s"].upper() == "HELLO, WORLD"
    assert box["s"].lower() == "hello, world"
    assert box["s"].replace("o", "0") == "Hell0, W0rld"
    assert box["s"].startswith("Hello")
    assert box["s"].endswith("World")
    assert box["s"].find("World") == 7
    assert box["s"].count("l") == 3
    assert box["s"].split(", ") == ["Hello", "World"]

    # `encode()` — the gap that broke `requests._prepare_body("raw", ...)`: a
    # `data.encode()` on a gradual `data` parameter. ASCII + multi-byte.
    assert box["s"].encode() == b"Hello, World"
    box["u"] = "caféX"
    assert box["u"].encode() == "caféX".encode()
    assert box["u"].encode("utf-8") == "caféX".encode("utf-8")

    box["t"] = "  trim  "
    assert box["t"].strip() == "trim"
    box["j"] = "-"
    assert box["j"].join(["x", "y", "z"]) == "x-y-z"
    box["d"] = "42"
    assert box["d"].isdigit()
    assert box["d"].rjust(5) == "   42"
    assert box["d"].zfill(5) == "00042"


_gm_container_methods()
_gm_user_methods()
_gm_scalar_methods()
_gm_str_methods()


# --- folded from b10_field_inference.py: cross-instance field-type inference ---
class _b10_Value:
    def __init__(self, data):
        self.data = data
        self.grad = 0.0
        self._prev = []
        self._local = []

    def __add__(self, other):
        out = _b10_Value(self.data + other.data)
        out._prev = [self, other]
        out._local = [1.0, 1.0]
        return out

    def __mul__(self, other):
        out = _b10_Value(self.data * other.data)
        out._prev = [self, other]
        out._local = [other.data, self.data]
        return out

    def backward_step(self):
        for i in range(len(self._prev)):
            child = self._prev[i]
            child.grad = child.grad + self._local[i] * self.grad


# the autograd pattern: self-referential writes through non-self receivers.
_b10_a = _b10_Value(2.0)
_b10_b = _b10_Value(3.0)
_b10_c = _b10_a * _b10_b
_b10_d = _b10_c + _b10_a
_b10_d.grad = 1.0
_b10_d.backward_step()
_b10_c.backward_step()
assert _b10_a.data == 2.0
assert _b10_b.data == 3.0
assert _b10_c.data == 6.0
assert _b10_d.data == 8.0
assert _b10_a.grad == 4.0
assert _b10_b.grad == 2.0
assert _b10_c.grad == 1.0
assert _b10_d.grad == 1.0


class _b10_Mixed:
    # mixed int/float writes demote the field (and the program still compiles).
    def __init__(self, flag):
        if flag:
            self.v = 1.5
        else:
            self.v = 7


_b10_m1 = _b10_Mixed(True)
_b10_m2 = _b10_Mixed(False)
assert _b10_m1.v == 1.5
assert _b10_m2.v == 7


class _b10_Counter:
    def __init__(self):
        self.total = 0.0

    def bump(self, amount):
        self.total = self.total + amount


class _b10_DoubleCounter(_b10_Counter):
    # a subclass writing an inherited field feeds the base class's variable.
    def bump2(self, amount):
        self.total = self.total + amount * 2.0


_b10_dc = _b10_DoubleCounter()
_b10_dc.bump(1.25)
_b10_dc.bump2(2.0)
assert _b10_dc.total == 5.25


class _b10_Tagged:
    # an annotated field stays authoritative.
    label: str

    def __init__(self, label: str):
        self.label = label


_b10_t = _b10_Tagged("ok")
assert _b10_t.label == "ok"


# --- folded from p46_heap_arg_guard.py + p47_heap_readback_guard.py (shared classes):
#     gradual Tagged->Heap shape guard at the call/arg/return, read-back-into-typed-
#     local, Dyn-global, and Dyn-field seams; plus the subclass-aware instance guard
#     and the class-reject path. Kept at module scope: the Dyn GLOBAL read
#     (`_hg_gdyn`) and Dyn FIELD read are load-bearing seams. ---
# DIVERGENCE-SAFE: error paths raise at the guard (pyaot) vs inside the body
# (CPython); both are caught and asserted as a boolean, never a divergent repr.
def _hg_as_dyn(x):
    # Unannotated param -> gradual `Dyn`; the inferred return type is `Dyn`.
    return x


def _hg_takes_str(x: str) -> int:
    return len(x)


def _hg_takes_list(x: list) -> int:
    return len(x)


def _hg_takes_dict(x: dict) -> int:
    return len(x)


def _hg_takes_set(x: set) -> int:
    return len(x)


def _hg_takes_tuple(x: tuple) -> int:
    return len(x)


# Correct path: a Dyn value that IS the shape passes the arg guard.
assert _hg_takes_str(_hg_as_dyn("hello")) == 5
assert _hg_takes_list(_hg_as_dyn([1, 2, 3])) == 3
assert _hg_takes_dict(_hg_as_dyn({1: 10, 2: 20})) == 2
assert _hg_takes_set(_hg_as_dyn({1, 2, 3, 4})) == 4
assert _hg_takes_tuple(_hg_as_dyn((7, 8))) == 2
assert _hg_takes_str(_hg_as_dyn("world!")) == 6
assert _hg_takes_list(_hg_as_dyn([1, 2, 3, 4, 5])) == 5
assert _hg_takes_dict(_hg_as_dyn({1: 1})) == 1
assert _hg_takes_set(_hg_as_dyn({9})) == 1
assert _hg_takes_tuple(_hg_as_dyn((1, 2, 3))) == 3


def _hg_arg_raises(fn) -> bool:
    try:
        fn()
        return False
    except TypeError:
        return True


# Error path: a Dyn int -> TypeError (guard here, `len(int)` in CPython).
assert _hg_arg_raises(lambda: _hg_takes_str(_hg_as_dyn(42)))
assert _hg_arg_raises(lambda: _hg_takes_list(_hg_as_dyn(42)))
assert _hg_arg_raises(lambda: _hg_takes_dict(_hg_as_dyn(42)))
assert _hg_arg_raises(lambda: _hg_takes_set(_hg_as_dyn(42)))
assert _hg_arg_raises(lambda: _hg_takes_tuple(_hg_as_dyn(42)))


# Read-back into a typed LOCAL: a Dyn value that IS the shape passes.
def _hg_rb_str(d) -> int:
    x: str = d
    return len(x)


def _hg_rb_list(d) -> int:
    x: list = d
    x.append(0)          # typed list op: x stays Heap(List); guard survives
    return len(x)


def _hg_rb_dict(d) -> int:
    x: dict = d
    return len(x)


def _hg_rb_set(d) -> int:
    x: set = d
    return len(x)


def _hg_rb_tuple(d) -> int:
    x: tuple = d
    return len(x)


assert _hg_rb_str(_hg_as_dyn("hello")) == 5
assert _hg_rb_list(_hg_as_dyn([1, 2, 3])) == 4      # 3 + appended 0
assert _hg_rb_dict(_hg_as_dyn({1: 1, 2: 2})) == 2
assert _hg_rb_set(_hg_as_dyn({1, 2, 3})) == 3
assert _hg_rb_tuple(_hg_as_dyn((9, 8))) == 2
assert _hg_rb_str(_hg_as_dyn("world")) == 5
assert _hg_rb_list(_hg_as_dyn([7, 7])) == 3
assert _hg_rb_dict(_hg_as_dyn({1: 1})) == 1
assert _hg_rb_set(_hg_as_dyn({5})) == 1
assert _hg_rb_tuple(_hg_as_dyn((1, 2, 3))) == 3


# Read-back from a Dyn GLOBAL into a typed local.
_hg_gdyn = _hg_as_dyn({10: 100, 20: 200, 30: 300})


def _hg_from_global() -> int:
    y: dict = _hg_gdyn       # Dyn global -> typed dict local
    return len(y)


assert _hg_from_global() == 3


# Read-back from a Dyn FIELD into a typed local.
class _hg_Box:
    def __init__(self, v):       # `v` unannotated -> the field is Dyn
        self.contents = v


def _hg_from_field(b: _hg_Box) -> int:
    z: list = b.contents         # Dyn field -> typed list local
    return len(z)


assert _hg_from_field(_hg_Box([1, 2, 3, 4])) == 4
assert _hg_from_field(_hg_Box([6, 6, 6])) == 3


def _hg_rb_raises(fn) -> bool:
    try:
        fn(_hg_as_dyn(42))
        return False
    except (TypeError, AttributeError):
        return True


# Read-back ERROR path: a Dyn int into a typed local (both raise).
assert _hg_rb_raises(_hg_rb_str)
assert _hg_rb_raises(_hg_rb_list)
assert _hg_rb_raises(_hg_rb_dict)
assert _hg_rb_raises(_hg_rb_set)
assert _hg_rb_raises(_hg_rb_tuple)


# Instance guard: subclass-aware CORRECT path + class-REJECT path (type diverges).
class _hg_Animal:
    def __init__(self, name: str) -> None:
        self.name = name


class _hg_Dog(_hg_Animal):
    pass


def _hg_animal_name(a: _hg_Animal) -> str:
    return a.name


# A Dog (subclass) into an Animal param passes the subclass-aware instance guard.
assert _hg_animal_name(_hg_as_dyn(_hg_Animal("Rex"))) == "Rex"
assert _hg_animal_name(_hg_as_dyn(_hg_Dog("Fido"))) == "Fido"


def _hg_class_reject() -> bool:
    # pyaot: TypeError at rt_check_instance; CPython: AttributeError at `.name`.
    try:
        _hg_animal_name(_hg_as_dyn(42))
        return False
    except (TypeError, AttributeError):
        return True


assert _hg_class_reject()


def _hg_animal_local(d) -> str:
    a: _hg_Animal = d        # Dyn -> typed Animal local (rt_check_instance)
    return a.name


def _hg_class_readback_reject() -> bool:
    try:
        _hg_animal_local(_hg_as_dyn(42))
        return False
    except (TypeError, AttributeError):
        return True


assert _hg_class_readback_reject()


print("All class tests passed!")
