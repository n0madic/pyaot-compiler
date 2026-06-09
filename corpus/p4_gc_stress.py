# Phase 4 — GC soak (PITFALLS B5/B15). The root set is derived purely from each
# local's `Repr::is_gc_root()`; this program keeps container / iterator / element
# locals simultaneously live across allocating build and iteration loops, forcing
# many collections in between. If any live heap root is missed, the collector
# frees it underfoot and the output diverges from CPython (or crashes).

# A survivor string and a survivor list that must outlive every loop below.
survivor = "the survivor string that must never be freed"
keep = [1, 2, 3, 4, 5]

# Build a list by repeated concatenation: each iteration allocates a fresh list
# (and grows the heap) while `survivor` / `keep` stay live across the allocation.
nums = []
for i in range(4000):
    nums = nums + [i]
print(len(nums))
print(nums[0])
print(nums[3999])

# Build a dict in a loop (each insert may rehash/allocate) with bignum-ish values
# (`i * i * i`) that promote to heap integers — heap roots created mid-loop.
squares = {}
for j in range(2000):
    squares[j] = j * j * j
print(len(squares))
print(squares[1999])

# Iterate one live container while building another: the iterator local, the
# per-iteration element, the growing result, AND `survivor` are all live across
# the allocating `+`.
doubled = []
for n in nums:
    doubled = doubled + [n * 2]
print(len(doubled))
print(doubled[100])

# A bignum accumulator re-boxed every iteration while iterating a live container.
big = 0
for n in nums:
    big = big + n * 1000000000000000000
print(big)

# Comprehensions allocate a result list plus the per-element heap values, all
# live across the build, while `survivor` / `nums` stay rooted.
comp = [[k, k + 1] for k in range(3000)]
print(len(comp))
print(comp[2999])

# An iteration-builtin pipeline: sorted(...) materializes a list, sum reduces a
# comprehension, all while the survivors persist.
print(sum([x for x in range(5000) if x % 7 == 0]))
print(len(sorted([n % 100 for n in nums])))

# The survivors are still intact after all that allocation.
print(survivor)
print(len(survivor))
print(keep)
print(keep[2])
