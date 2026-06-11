# Phase 3 numeric specialization — exercises unboxed float arithmetic (Raw(F64)
# fadd/fsub/fmul) and literal-bounded raw-int range loops (Raw(I64) icmp + add)
# end-to-end. The differential gate cannot tell optimized from unoptimized code,
# so this file's job is to prove the specialized paths still match CPython
# byte-for-byte (Principle 2: precision must not change correctness).

# Unboxed float accumulation (raw fadd, no boxing / GC traffic).
acc = 0.0
for i in range(10):
    acc = acc + 0.5
print(acc)

# Raw float * and - (fmul / fsub); tagged true-division stays exact.
x = 2.5
y = x * x - 1.0
print(y)
print(7.0 / 2.0)
print(10 / 4)

# Literal-bounded raw-int loop; the cursor runs raw, the accumulator stays
# tagged (bignum-safe).
s = 0
for k in range(1, 11):
    s = s + k
print(s)

# Negative literal step.
t = 0
for d in range(20, 0, -3):
    t = t + d
print(t)

# Loop variable used in body arithmetic. typeck's interval pass proves `n ∈ [1,5]`
# and `n * n ∈ [1,25]`, so BOTH the cursor and `n * n` now run on the raw machine
# path; the accumulator `sq` has no static bound and stays tagged (bignum-safe).
sq = 0
for n in range(1, 6):
    sq = sq + n * n
print(sq)

# Mixed int/float stays tagged but correct (runtime promotes).
print(3 + 1.5)
print(2 * 2.0)

# A float-parameter function (Raw(F64) ABI).
def poly(a: float) -> float:
    return a * a + 2.0 * a + 1.0


print(poly(3.0))


# A non-literal range bound (`range(n + 1)`) is NOT narrowed — the cursor stays
# tagged inside this function.
def tri(n: int) -> int:
    total = 0
    for i in range(n + 1):
        total = total + i
    return total


print(tri(100))
