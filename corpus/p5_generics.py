"""Phase 5E — generics: TypeVar, Generic[T], generic methods/fields, instantiation.

Uniform-Tagged storage means a generic class compiles ONCE; `Stack[int]` and
`Stack[str]` share one physical layout, so the output is identical regardless of
T. Type-arg substitution refines the *static* types (so `Stack[int].pop()` is an
`int`) without changing the emitted code.
"""

from typing import TypeVar, Generic

T = TypeVar("T")


class Box(Generic[T]):
    def __init__(self, value: T):
        self.value = value

    def get(self) -> T:
        return self.value

    def replace(self, v: T):
        self.value = v


class Stack(Generic[T]):
    items: list[T]

    def __init__(self):
        self.items = []

    def push(self, x: T):
        self.items.append(x)

    def pop(self) -> T:
        return self.items.pop()

    def size(self) -> int:
        return len(self.items)


# Box[int] / Box[str] — same layout, precise element types.
bi = Box[int](42)
print(bi.get())
bi.replace(100)
print(bi.get())

bs = Box[str]("hello")
print(bs.get())

# Stack[int]: pop() is statically an int.
si: Stack[int] = Stack[int]()
si.push(1)
si.push(2)
si.push(3)
print(si.size())
print(si.pop() + 10)   # int arithmetic on the substituted return type
print(si.pop())
print(si.size())

# Stack[str]: same code, str elements.
ss = Stack[str]()
ss.push("a")
ss.push("b")
print(ss.pop())
print(ss.size())

# A bare (un-parameterized) Stack still works — element type erases to dynamic.
sd = Stack()
sd.push(7)
print(sd.pop())
