# Phase 1 — rt_unbox_bool: the third checked-unbox shape (Tagged -> Raw(I8)),
# completing the rt_unbox_float / rt_unbox_int family. A gradual (Dyn) value
# stored into an annotated `: bool` local takes the CHECKED unbox instead of a
# blind bit-reinterpret: a tagged bool unboxes, anything else would raise
# TypeError in the runtime guard.
#
# An unannotated parameter is Dyn, so storing it into a `: bool` slot is exactly
# the Dyn -> Raw(I8) checked coercion. Calling these top-level defs directly needs
# no call bridge, so #1 ships green on its own.
#
# This probe exercises ONLY the success path (a Dyn value that is genuinely a
# bool at run time), because that is the only path that can byte-match CPython:
# CPython ignores the `: bool` annotation entirely, so a non-bool into a `: bool`
# slot keeps the value (no exception). The divergent wrong-shape guard is covered
# by a runtime unit test (crates/runtime/src/tests.rs), not the differential
# corpus.


def echo_bool(x):
    flag: bool = x  # Dyn -> bool slot: the checked unbox
    print(flag)
    print(not flag)
    if flag:
        print("flag truthy")
    else:
        print("flag falsy")
    return flag


echo_bool(True)
echo_bool(False)


def combine(p, q):
    a: bool = p
    b: bool = q
    print(a and b)
    print(a or b)
    print(a == b)
    return a and b


combine(True, True)
combine(True, False)
combine(False, False)


# A `: bool` slot reassigned from another Dyn value within the same function
# (the widened Assign seam also takes the checked unbox).
def toggle(first, second):
    state: bool = first
    print(state)
    state = second
    print(state)


toggle(True, False)
toggle(False, True)


print("p38 unbox bool passed!")
