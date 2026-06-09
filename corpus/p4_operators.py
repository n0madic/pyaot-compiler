# Phase 4A — container operators: `+` / `*` (concat / repeat), `==` / `!=`,
# ordering (`<` `<=` `>` `>=`) on list/tuple, and membership (`in` / `not in`).

# List concatenation and repetition.
a = [1, 2]
b = [3, 4]
print(a + b)
print(a * 3)
print([0] * 5)
print(len(a + b))

# Tuple concatenation.
t = (1, 2) + (3, 4)
print(t)
print(len(t))

# Bytes concatenation and repetition.
print(b"ab" + b"cd")
print(b"xy" * 3)

# Equality (structural) across container kinds.
print([1, 2, 3] == [1, 2, 3])
print([1, 2] == [1, 2, 3])
print([1, 2] != [3, 4])
print((1, 2) == (1, 2))
print({1, 2, 3} == {3, 2, 1})
print({"a": 1} == {"a": 1})
print(b"abc" == b"abc")
print(b"abc" == b"abd")

# Ordering on lists and tuples (lexicographic).
print([1, 2, 3] < [1, 2, 4])
print([1, 2] < [1, 2, 3])
print([2] > [1, 9, 9])
print((1, 2) <= (1, 2))
print((1, 3) >= (1, 2))

# Membership.
xs = [10, 20, 30]
print(20 in xs)
print(25 in xs)
print(25 not in xs)
d = {"k": 1}
print("k" in d)
print("z" in d)
st = {1, 2, 3}
print(2 in st)
print(9 not in st)
print(3 in (1, 2, 3))
print("y" in "python")
print("q" in "python")

# Operators feeding into expressions.
total = len([1, 2] + [3, 4, 5])
print(total)
