"""Phase 5B — inheritance: super(), C3 MRO, virtual dispatch, isinstance."""


class Animal:
    def __init__(self, name: str):
        self.name = name

    def speak(self) -> str:
        return "..."

    def describe(self) -> str:
        # Calls self.speak() — virtual dispatch from within a base method.
        return self.name + " says " + self.speak()


class Dog(Animal):
    def __init__(self, name: str, breed: str):
        super().__init__(name)  # super() chain into Animal.__init__
        self.breed = breed

    def speak(self) -> str:
        return "Woof"


class Puppy(Dog):
    def speak(self) -> str:
        return "Yip"


class Cat(Animal):
    def speak(self) -> str:
        return "Meow"


# Polymorphic dispatch over a base-typed list of mixed subclasses.
animals: list[Animal] = [Dog("Rex", "Lab"), Cat("Felix"), Puppy("Buddy", "Pug"), Animal("Thing")]
for a in animals:
    print(a.describe())

# Direct calls + inherited / own fields (slot-stable across the hierarchy).
d = Dog("Fido", "Beagle")
print(d.describe())
print(d.speak())
print(d.name)
print(d.breed)

p = Puppy("Spot", "Terrier")
print(p.describe())
print(p.name)
print(p.breed)

# isinstance across the single-inheritance hierarchy.
print(isinstance(d, Dog))
print(isinstance(d, Animal))
print(isinstance(d, Cat))
print(isinstance(p, Animal))
print(isinstance(p, Dog))
print(isinstance(p, Puppy))
c = Cat("Tom")
print(isinstance(c, Dog))
print(isinstance(c, Animal))


# Multiple inheritance with a diamond, to exercise C3 (MRO = D, B, C, A).
class A:
    def who(self) -> str:
        return "A"


class B(A):
    def who(self) -> str:
        return "B"


class C(A):
    def who(self) -> str:
        return "C"


class D(B, C):
    pass


print(D().who())
print(B().who())
print(C().who())
print(A().who())
