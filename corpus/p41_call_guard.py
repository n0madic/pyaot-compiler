# Runtime callable guard on the uniform value-call path. A genuinely-`Dyn` value
# that is NOT a closure — an int, a string, None, or (the subtle case) a DATA
# tuple, which shares the `TupleObj` layout with a closure but now carries a
# distinct `Closure` type tag — raises `TypeError: object is not callable` instead
# of crashing on a bad slot-0 read. A real closure through the same `Dyn` path
# still calls correctly.


def call_it(x):  # x is Dyn (unannotated) -> the uniform indirect-call path
    return x()


# A data tuple as a Dyn callee (the closure/tuple tag-collision SEGV vector).
try:
    call_it((1, 2))
    print("FAIL: tuple was called")
except TypeError:
    print("tuple not callable: OK")

# An int as a Dyn callee.
try:
    call_it(5)
    print("FAIL: int was called")
except TypeError:
    print("int not callable: OK")

# None as a Dyn callee.
try:
    call_it(None)
    print("FAIL: None was called")
except TypeError:
    print("None not callable: OK")

# A string as a Dyn callee.
try:
    call_it("hi")
    print("FAIL: str was called")
except TypeError:
    print("str not callable: OK")


# A genuine closure flowing through the SAME `Dyn` `call_it` path still works.
def adder(n: int):
    def add() -> int:
        return n + 1
    return add


assert call_it(adder(41)) == 42, "real closure through Dyn path"
print("real closure through Dyn path: OK")

print("All call-guard tests passed!")
