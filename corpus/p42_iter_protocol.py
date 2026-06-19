# Lazy user-class iterator protocol: `for x in instance` / `iter()` / `next()`
# over a class defining `__iter__`/`__next__`. The for-loop desugar drives a
# runtime `IteratorObj{kind=Instance}` whose `IterNext` calls the class's
# compiled `<iternext>` thunk — `try: return self.__next__() except
# StopIteration: return UNBOUND` — bridging the user iterator's exception
# protocol to the runtime's exhausted-flag protocol. Differential vs CPython.


class CountUp:
    """Self-iterator: __iter__ returns self; __next__ raises StopIteration."""
    def __init__(self, n: int):
        self.i = 0
        self.n = n

    def __iter__(self):
        return self

    def __next__(self) -> int:
        if self.i >= self.n:
            raise StopIteration
        v = self.i
        self.i = self.i + 1
        return v


# for x in instance (self-iterator)
print("for-loop:")
for x in CountUp(4):
    print(x)

# empty iteration
for x in CountUp(0):
    print("unreachable", x)
print("empty done")

# iter()/next() explicit, with StopIteration on exhaustion
it = iter(CountUp(3))
print(next(it), next(it), next(it))
try:
    next(it)
except StopIteration:
    print("StopIteration caught")

# next() called DIRECTLY on a self-iterator instance (no intervening iter())
cu = CountUp(2)
print("direct-next:", next(cu), next(cu))

# break-early (the iterator is abandoned mid-stream)
print("break-early:")
for x in CountUp(100):
    if x >= 3:
        break
    print(x)

# list() / sum() / "in" over an instance
print("list:", list(CountUp(5)))
print("sum:", sum(CountUp(5)))


class Squares:
    """A distinct iterator object yielded by a separate iterable."""
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


class SquaresIterable:
    """`__iter__` returns a FRESH `Squares` each call (re-iterable)."""
    def __init__(self, n: int):
        self.n = n

    def __iter__(self):
        return Squares(self.n)


si = SquaresIterable(4)
print("squares:", list(si))
print("squares-again:", list(si))  # fresh iterator → same result


class CountUpChild(CountUp):
    """Inherits __iter__ and __next__ from CountUp (reuses the base thunk)."""
    pass


print("inherited:", list(CountUpChild(3)))


class Boom:
    """__next__ raises a NON-StopIteration exception — it must propagate."""
    def __iter__(self):
        return self

    def __next__(self) -> int:
        raise ValueError("boom")


try:
    for x in Boom():
        print("unreachable", x)
except ValueError as e:
    print("ValueError propagated:", e)

print("all done")
