# Test all print() functionality with output comparison
# Each print() produces output that must match test_print_output.expected

# ===== Section 1: Basic types =====
print(42)
print(-7)
print(0)
print(3.14)
print(1.0)
print(0.0)
print(True)
print(False)
print(None)
print("hello")
print("")

# ===== Section 2: No arguments =====
print()

# ===== Section 3: Multiple arguments (default sep=" ") =====
print(1, 2, 3)
print("a", "b", "c")
print(1, "hello", True, 3.14)
print(1, None, 2)

# ===== Section 4: Custom sep =====
print(1, 2, 3, sep="-")
print("a", "b", "c", sep=", ")
print(1, 2, sep="")

# ===== Section 5: Custom end =====
print("hello", end="")
print(" world")
print("line1", end="\n")
print("line2")

# ===== Section 6: Both sep and end =====
print(1, 2, 3, sep="-", end="!\n")

# ===== Section 7: Container printing - Lists =====
print([1, 2, 3])
print(["a", "b"])
print([])

# ===== Section 8: Container printing - Tuples =====
print((1, 2, 3))
print((42,))
print(("hello", "world"))

# ===== Section 9: Container printing - Nested =====
print([[1, 2], [3, 4]])

# ===== Section 10: Dict printing =====
print({})
d1: dict[str, int] = {"x": 1}
print(d1)

# ===== Section 11: Set printing =====
empty_s: set[int] = set()
print(empty_s)
s1: set[int] = {42}
print(s1)

# ===== Section 12: Bytes printing =====
print(b"hello")
print(b"")

# ===== Section 13: Expressions and variables =====
x: int = 10
y: int = 20
print(x + y)
print(x * y)

# ===== Section 14: Print in loops =====
for i in range(3):
    print(i)

# ===== Section 15: Print with function result =====
def greet(name: str) -> str:
    return "Hello " + name

print(greet("World"))
