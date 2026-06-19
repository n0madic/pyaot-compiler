# Gradual builtin op on an IMMEDIATE receiver — TypeError, not SIGSEGV.
#
# `len(x)`, `e in x`, and `x[i]` tag-dispatch through a runtime ABI
# (`rt_obj_len` / `rt_obj_contains` / `rt_any_getitem`). When `x` is a gradual
# `Dyn` value that carries an IMMEDIATE at run time (`int`/`bool`/`None`, not a
# heap pointer), the ABI used to blind-`unwrap_ptr` it — fabricating a garbage
# address the tag dispatch then dereferenced (SIGSEGV). Each ABI now guards the
# immediate receiver and raises the matching CPython `TypeError`:
#   * `len(42)`  → "object of type 'int' has no len()"
#   * `5 in 42`  → "argument of type 'int' is not iterable"
#   * `42[0]`    → "'int' object is not subscriptable"
#
# This is the direct-`len(dyn)` family (a gradual builtin op whose RECEIVER is an
# immediate), distinct from the §1 coercion seams (p46/p47) — there the value
# crosses into a typed slot; here it is consumed in place. Both runtimes raise
# `TypeError`, so a plain `except TypeError` matches byte-for-byte.


def as_dyn(x):
    return x


# ===== error path: immediate receivers raise TypeError on all three ops =====
def expect_type_error(label, fn):
    try:
        fn()
        print("ERROR:", label, "did not raise")
    except TypeError:
        print("caught", label, "TypeError")


# `len` on int / bool / None immediates
expect_type_error("len(int)", lambda: len(as_dyn(42)))
expect_type_error("len(bool)", lambda: len(as_dyn(True)))
expect_type_error("len(None)", lambda: len(as_dyn(None)))

# `in` with an immediate container
expect_type_error("in(int)", lambda: 5 in as_dyn(42))
expect_type_error("in(None)", lambda: 1 in as_dyn(None))

# subscript on an immediate receiver
expect_type_error("sub(int)", lambda: as_dyn(99)[0])
expect_type_error("sub(bool)", lambda: as_dyn(False)[0])


# ===== correct path: the same ops on genuine containers still work =====
assert len(as_dyn([1, 2, 3])) == 3
assert len(as_dyn("hello")) == 5
assert len(as_dyn({1: 1, 2: 2})) == 2
assert (2 in as_dyn([1, 2, 3])) is True
assert (9 in as_dyn([1, 2, 3])) is False
assert as_dyn([7, 8, 9])[1] == 8
assert as_dyn((4, 5, 6))[0] == 4
assert as_dyn("abc")[2] == "c"
print("len list:", len(as_dyn([1, 2, 3, 4])))
print("in list:", 2 in as_dyn([1, 2, 3]))
print("subscript list:", as_dyn([10, 20, 30])[2])
print("subscript str:", as_dyn("xyz")[0])


print("Gradual builtin on immediate (TypeError, not SIGSEGV) tests passed!")
