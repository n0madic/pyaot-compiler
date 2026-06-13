# Tuple slice assigned to an annotated fixed-arity `tuple[…]` slot.
#
# Slicing a tuple yields a variable-length `tuple[T, ...]` (the joined element
# type — see `slice_ty`), but a fixed-arity `tuple[T, …]` annotation is a common
# (CPython-legal, never arity-enforced) slot for it. The repr-contract check used
# to reject this `tuple` → `tuple` store; it now admits it when every element's
# `Repr` matches per index — a fixed and a variable tuple are one physical
# `TupleObj`, so only an element-repr mismatch (int read through a float slot)
# would be a reinterpret hazard. `len()` reflects the real runtime length, not
# the (possibly-wrong) annotated arity.
#
# Probes three element-repr families so the per-index match is exercised end to
# end: Tagged `int`, `Raw(F64)` `float`, and `Heap(Str)`. Interaction probes
# cross the slice-into-slot with iteration and nested/flat unpacking.


# ===== Tagged int elements =====
nums: tuple[int, int, int, int, int, int] = (0, 1, 2, 3, 4, 5)

s_mid: tuple[int, int, int, int, int, int] = nums[1:4]
print(len(s_mid), s_mid[0], s_mid[1], s_mid[2])      # 3 1 2 3

s_head: tuple[int, int, int, int, int, int] = nums[:3]
print(len(s_head), s_head[0], s_head[2])             # 3 0 2

s_tail: tuple[int, int, int, int, int, int] = nums[3:]
print(len(s_tail), s_tail[0], s_tail[2])             # 3 3 5

s_full: tuple[int, int, int, int, int, int] = nums[:]
print(len(s_full), s_full[0], s_full[5])             # 6 0 5

s_step: tuple[int, int, int, int, int, int] = nums[::2]
print(len(s_step), s_step[0], s_step[1], s_step[2])  # 3 0 2 4

s_neg: tuple[int, int, int, int, int, int] = nums[-2:]
print(len(s_neg), s_neg[0], s_neg[1])                # 2 4 5

s_rev: tuple[int, int, int, int, int, int] = nums[::-1]
print(len(s_rev), s_rev[0], s_rev[5])                # 6 5 0


# ===== Raw(F64) float elements =====
fs: tuple[float, float, float, float] = (1.0, 2.0, 3.0, 4.0)
fslice: tuple[float, float, float, float] = fs[1:3]
print(len(fslice), fslice[0], fslice[1])             # 2 2.0 3.0


# ===== Heap(Str) elements =====
ws: tuple[str, str, str] = ("alpha", "beta", "gamma")
wslice: tuple[str, str, str] = ws[0:2]
print(len(wslice), wslice[0], wslice[1])             # 2 alpha beta


# ===== Interaction: iterate a slice stored in an annotated slot =====
total = 0
for v in s_mid:
    total += v
print(total)                                         # 6


# ===== Interaction: unpack a slice stored in an annotated slot =====
u: tuple[int, int, int, int, int, int] = nums[2:5]
a, b, c = u
print(a, b, c)                                        # 2 3 4

# Nested unpacking from a sliced-into-slot tuple paired with a literal.
p, (q, r) = u[0], (u[1], u[2])
print(p, q, r)                                        # 2 3 4

print("Tuple slice into annotated slot tests passed!")
