# `isinstance(x, (A, B, ...))` tuple-of-types, §7.
#
# The natural completion of §7 (single-type and container targets already work).
# `isinstance(x, (int, str))` desugars in the FRONTEND to an `or` of the existing
# per-element checks — `IsInstance` (runtime, user classes) and `IsInstanceBuiltin`
# (static fold, builtin types) — over a receiver evaluated ONCE. Pure front-half:
# ZERO new HIR node, ZERO runtime / typeck / lowering change. Nested type-tuples
# flatten (CPython semantics); the empty tuple is `False`.
#
# `==`/`assert` are the spec (Principle 9); `print` feeds the differential harness.


# ===== a user-class hierarchy (mirrors the single-type isinstance gate) =====
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


# ===== builtin-only tuples (static fold per element) =====
# A concretely-typed receiver crossed with builtin-type elements — a narrowing
# miss would surface here as a COMPILE error (the static fold rejects `Dyn`).
five = 5
flt = 5.0
txt = "a"
assert isinstance(five, (str, int)) is True
assert isinstance(flt, (str, int)) is False
assert isinstance(txt, (int, str, bytes)) is True
assert isinstance(txt, (int, float)) is False
print(isinstance(five, (str, int)))      # True
print(isinstance(flt, (str, int)))       # False
print(isinstance(txt, (int, str, bytes)))  # True

# bool ⊂ int in Python.
flag = True
assert isinstance(flag, (int,)) is True
assert isinstance(flag, (str,)) is False
print(isinstance(flag, (int,)))          # True


# ===== container KINDS (isinstance matches by kind, ignores element types) =====
lst = [1, 2, 3]
tup = (1, 2)
dct = {"k": 1}
assert isinstance(lst, (dict, list)) is True
assert isinstance(tup, (list, tuple)) is True
assert isinstance(dct, (list, dict)) is True
assert isinstance(lst, (dict, tuple)) is False
print(isinstance(lst, (dict, list)))     # True
print(isinstance(tup, (list, tuple)))    # True


# ===== user classes (runtime inheritance-aware check) =====
d = Dog("rex")
c = Cat("mia")
assert isinstance(d, (Cat, Animal)) is True   # Dog is-a Animal
assert isinstance(c, (Dog,)) is False         # Cat is not-a Dog
assert isinstance(c, (Cat, Dog)) is True
assert isinstance(d, (Cat,)) is False
print(isinstance(d, (Cat, Animal)))      # True
print(isinstance(c, (Dog,)))             # False


# ===== MIXED user-class + builtin element (both kinds in one tuple) =====
# Receiver typed `Dog` (concrete): the `int` element folds to False statically,
# the `Dog` element checks at runtime → overall True.
assert isinstance(d, (int, Dog)) is True
assert isinstance(d, (int, Cat)) is False
assert isinstance(five, (Dog, int)) is True   # builtin element wins on an int
print(isinstance(d, (int, Dog)))         # True
print(isinstance(d, (int, Cat)))         # False


# ===== nested type-tuple flatten (CPython flattens recursively) =====
assert isinstance(five, (str, (bytes, int))) is True
assert isinstance(flt, (str, (bytes, int))) is False
assert isinstance(d, (Cat, (str, Animal))) is True
print(isinstance(five, (str, (bytes, int))))   # True
print(isinstance(d, (Cat, (str, Animal))))     # True


# ===== empty tuple ⇒ False =====
assert isinstance(five, ()) is False
assert isinstance(d, ()) is False
print(isinstance(five, ()))              # False


# ===== single-eval: the receiver is evaluated EXACTLY once =====
counter = 0


def bump() -> int:
    global counter
    counter += 1
    return counter


before = counter
result = isinstance(bump(), (int, str))   # int element is first → still 1 eval
assert result is True
assert counter == before + 1              # advanced by exactly one
# A second call advances again by exactly one (not two, despite two elements).
result2 = isinstance(bump(), (str, float))
assert result2 is False                   # an int is neither str nor float
assert counter == before + 2
print(counter)                           # 2


# ===== cross with green features: `if`, `assert ... and ...`, `or`, comprehension =====
def classify(x: int) -> str:
    if isinstance(x, (int, float)):
        return "number"
    return "other"


assert classify(7) == "number"
print(classify(7))                       # number

# tuple-isinstance under `and` / `or`.
assert isinstance(five, (int, str)) and isinstance(txt, (int, str))
assert isinstance(flt, (int,)) or isinstance(flt, (float,))
print(isinstance(five, (int, str)) and isinstance(txt, (int, str)))  # True
print(isinstance(flt, (int,)) or isinstance(flt, (float,)))          # True

# list comprehension filter, builtin elements over a concretely-typed source.
nums = [1, 2, 3, 4]
kept = [v for v in nums if isinstance(v, (int, float))]
assert kept == [1, 2, 3, 4]
print(kept)                              # [1, 2, 3, 4]

# list comprehension filter, user-class element → meaningful filtering.
animals = [Dog("a"), Cat("b"), Dog("c")]
voices = [a.speak() for a in animals if isinstance(a, (Dog,))]
assert voices == ["woof", "woof"]
print(voices)                            # ['woof', 'woof']

print("p34 isinstance-tuple OK")
