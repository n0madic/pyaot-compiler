# Consolidated test file for classes and OOP

from typing import Any
from abc import abstractmethod

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

# Create instance via constructor
p = Point(3, 4)

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

# Basic classmethod - cls is passed as first argument (as class_id integer)
class ClassMethodBasic:
    count: int = 0  # Class attribute with type annotation

    @classmethod
    def increment(cls: int) -> int:
        # cls receives the class_id as an integer
        ClassMethodBasic.count = ClassMethodBasic.count + 1
        return ClassMethodBasic.count

    @classmethod
    def get_count(cls: int) -> int:
        return ClassMethodBasic.count

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

    @classmethod
    def add_to_value(cls: int, x: int) -> int:
        return ClassMethodWithArgs.value + x

    @classmethod
    def multiply_value(cls: int, x: int, y: int) -> int:
        return ClassMethodWithArgs.value * x * y

# Test classmethod with args on class
assert ClassMethodWithArgs.add_to_value(5) == 15, "ClassMethodWithArgs.add_to_value(5) should equal 15"
assert ClassMethodWithArgs.multiply_value(2, 3) == 60, "ClassMethodWithArgs.multiply_value(2, 3) should equal 60"

# Test classmethod with args on instance
cwa = ClassMethodWithArgs()
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
class Flags:
    value: int
    def __init__(self, value: int):
        self.value = value
    def __and__(self, other: Flags) -> Flags:
        return Flags(self.value & other.value)
    def __or__(self, other: Flags) -> Flags:
        return Flags(self.value | other.value)
    def __xor__(self, other: Flags) -> Flags:
        return Flags(self.value ^ other.value)
    def __lshift__(self, other: Flags) -> Flags:
        return Flags(self.value << other.value)
    def __rshift__(self, other: Flags) -> Flags:
        return Flags(self.value >> other.value)

fl1 = Flags(12)   # 0b1100
fl2 = Flags(10)   # 0b1010

br = fl1 & fl2
assert br.value == 8, f"& failed: {br.value}"  # 0b1000

br = fl1 | fl2
assert br.value == 14, f"| failed: {br.value}"  # 0b1110

br = fl1 ^ fl2
assert br.value == 6, f"^ failed: {br.value}"  # 0b0110

br = fl1 << Flags(2)
assert br.value == 48, f"<< failed: {br.value}"  # 0b110000

br = fl1 >> Flags(2)
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
cp_raw = copy.copy(cp_orig)
# The __copy__ method returns CopyPoint, but copy.copy returns Any.
# Test via the __copy__ method directly to verify it works.
cp_copy = cp_orig.__copy__()
# __copy__ multiplies coords by 10
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
    def __deepcopy__(self) -> DeepContainer:
        # Custom deep copy: copy list manually, transform tag
        new_items: list[int] = []
        for item in self.items:
            new_items.append(item)
        return DeepContainer(new_items, self.tag + "_copy")

dc_orig = DeepContainer([1, 2, 3], "original")
# Test via __deepcopy__ directly to verify (copy.deepcopy returns Any)
dc_deep = dc_orig.__deepcopy__()
# __deepcopy__ appends "_copy" to tag
assert dc_deep.tag == "original_copy", f"__deepcopy__ tag failed: {dc_deep.tag}"
assert dc_deep.items[0] == 1 and dc_deep.items[1] == 2 and dc_deep.items[2] == 3
# Verify independence
dc_deep.items.append(4)
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

print("All class tests passed!")
