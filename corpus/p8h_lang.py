# Phase 8H stage C — front-half features.
# lambda defaults via the module-level def desugar (#9), `for line in f:`
# over a File VARIABLE (#15b), and os.environ writes (#14a).

import os

# --- #9: module-level lambda with defaults ---
scale = lambda x, k=2: x * k
print(scale(10))
print(scale(10, 3))
print(scale(10, k=5))

add3 = lambda a, b=10, c=100: a + b + c
print(add3(1))
print(add3(1, 2))
print(add3(1, 2, 3))
print(add3(1, c=7))

# A no-defaults lambda keeps the closure path.
double = lambda v: v * 2
print(double(21))

# --- #15b: iterate a File stored in a variable ---
path = "/tmp/p8h_lang_lines.txt"
with open(path, "w") as w:
    w.write("alpha\nbeta\ngamma\n")
f = open(path)
for line in f:
    print(line.strip())
f.close()

# The syntactic form still works through the same lowering path.
for line in open(path):
    print(len(line))
os.remove(path)

# --- #14a: os.environ writes are visible to subsequent reads ---
os.environ["P8H_TEST_VAR"] = "p8h-value"
print(os.getenv("P8H_TEST_VAR"))
print(os.environ["P8H_TEST_VAR"])
print(os.environ.get("P8H_TEST_VAR"))
v = os.getenv("P8H_MISSING_VAR")
print(v)

print("p8h lang passed!")
