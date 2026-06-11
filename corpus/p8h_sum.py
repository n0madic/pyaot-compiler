# Phase 8H D2 — sum() through the typed HIR node.
# Float/int sums solve to precise result types; class elements ride the
# inferred __add__ returns; generator arguments materialize as list comps.

print(sum([1, 2, 3]))
print(sum([0.5, 1.5, 2.0]))
print(sum([1, 2], 10))
print(sum([0.25, 0.25], 1.0))
print(sum(range(10)))

# result feeds typed numeric code without annotations
# (binary-exact fractions: CPython >= 3.12 uses Neumaier compensated
# summation for floats, our expansion is a naive left fold)
total = sum([0.125, 0.25, 0.5])
print(total * 2.0)
half = sum([1, 2, 3, 4]) / 2
print(half)

# generator-expression argument (materialized as a list comprehension)
print(sum(i * i for i in range(5)))
print(sum(x * 0.5 for x in [1.0, 2.0, 3.0]))
print(sum(i for i in range(10) if i % 2 == 0))

# nested sums
print(sum([sum([1, 2]), sum([3, 4])]))

# bools count as ints
print(sum([True, True, False]))

# empty list with int elements
print(sum([0, 0]) + sum([1]))

print("p8h sum passed!")
