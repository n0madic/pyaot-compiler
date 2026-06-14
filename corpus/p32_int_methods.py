# int / bool methods `bit_length` / `bit_count` / `conjugate` / `__index__`, §9.
#
# All four dispatch on an int / bool receiver (bool is an int subtype, so it
# inherits them). `bit_length`/`bit_count` route to BIGNUM-AWARE runtime counts
# (`rt_int_bit_length`/`rt_int_bit_count` now take a tagged `Value` and classify
# fixnum vs heap `BigInt`); `conjugate`/`__index__` return the receiver's int
# VALUE via `rt_int_index` (which widens a bool to its int 0/1 and preserves a
# bignum), so a bool receiver yields an Int-typed result, never a tagged bool.
#
# `==` asserts are the spec (Principle 9); prints feed the differential harness.


# ===== bit_length(): bits to represent abs(n); 0 for 0 =====
assert (0).bit_length() == 0
assert (5).bit_length() == 3
assert (255).bit_length() == 8
assert (1024).bit_length() == 11
assert (-7).bit_length() == 3       # sign ignored
n = 1000
assert n.bit_length() == 10         # variable receiver
print((255).bit_length())           # 8

# loop-variable receiver (each element is Int-typed)
expected = [1, 2, 4, 8]
i = 0
for x in [1, 2, 8, 255]:
    assert x.bit_length() == expected[i]
    i += 1
print(i)                            # 4


# ===== bit_count(): set bits of abs(n) (Python 3.10+) =====
assert (0).bit_count() == 0
assert (255).bit_count() == 8
assert (-7).bit_count() == 3
assert (7).bit_count() == 3
print((255).bit_count())            # 8


# ===== bool is an int subtype: methods work on bools =====
assert True.bit_length() == 1
assert False.bit_length() == 0
assert True.bit_count() == 1
assert False.bit_count() == 0
print(True.bit_length(), False.bit_length())  # 1 0


# ===== conjugate() / __index__() return the int value =====
assert (42).conjugate() == 42
assert (42).__index__() == 42
assert (-5).conjugate() == -5
# bool widens to int (Int-typed result, usable in arithmetic)
assert True.conjugate() == 1
assert False.conjugate() == 0
assert True.__index__() == 1
assert (True.conjugate() + 5) == 6
print((42).conjugate(), True.conjugate())  # 42 1


# ===== BIGNUM-aware: arbitrary-precision receivers =====
big = 2 ** 100
assert big.bit_length() == 101
assert big.bit_count() == 1
assert (2 ** 64 - 1).bit_length() == 64
assert (2 ** 64 - 1).bit_count() == 64
assert big.conjugate() == big       # bignum preserved
assert big.__index__() == big
print((2 ** 100).bit_length())      # 101
print((2 ** 128 - 1).bit_count())   # 128


# ===== cross with already-green features (f-string, indexing into a list) =====
vals = [1, 7, 255, 1023]
bl = [v.bit_length() for v in vals]
assert bl == [1, 3, 8, 10]
print(bl)
print(f"bits of 255 = {(255).bit_length()}, ones = {(255).bit_count()}")

print("All int-method tests passed!")
