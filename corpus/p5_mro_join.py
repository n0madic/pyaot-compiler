"""MRO-aware nominal joins (PLAN item 4): UNANNOTATED sibling lists/returns
join to the nearest common C3 ancestor (not a Union), with field access and
virtual dispatch through the joined base type."""


class Animal:
    def __init__(self, name: str):
        self.name = name

    def speak(self) -> str:
        return "..."


class Dog(Animal):
    def speak(self) -> str:
        return "Woof"


class Cat(Animal):
    def speak(self) -> str:
        return "Meow"


# Unannotated mixed literal: joins to list[Animal] via the MRO lattice.
pets = [Dog("Rex"), Cat("Tom")]
for p in pets:
    print(p.name + ": " + p.speak())


# Unannotated return joins sibling branches to the common base.
def pick(n: int):
    if n == 0:
        return Dog("D")
    return Cat("C")


print(pick(0).speak())
print(pick(1).speak())

# Derived + Base collapses to Base (subsumption, not a two-member union).
mixed = [Dog("M"), Animal("Plain")]
for m in mixed:
    print(m.speak())


# Diamond: join(B, C) = Animal; D() dispatches per C3 (B before C).
class B(Animal):
    def speak(self) -> str:
        return "B"


class C(Animal):
    def speak(self) -> str:
        return "C"


class D(B, C):
    pass


zoo = [B("b"), C("c"), D("d")]
for z in zoo:
    print(z.name + ": " + z.speak())

# Conditional expression over siblings — another unannotated join site.
flip = True
chosen = Dog("Yes") if flip else Cat("No")
print(chosen.speak())
print(isinstance(chosen, Animal))
print(isinstance(chosen, Cat))
