# Phase 3c — raw-int loop specialization (typeck's interval proof). Every line
# must match CPython byte-for-byte whether the value runs on the raw machine path
# (Raw(I64) imul / srem / sdiv with the Python floor correction) or the tagged
# baseline: precision must never change correctness (Principle 2). Bounds are
# deliberately small so the corpus run stays fast.

# 1. Narrowed induction variable + derived expressions: i, i*3, i*3 % 7 and
#    i*3 // 7 are all provably in ±2^48, so each runs raw.
for i in range(50):
    print(i, i * 3, i * 3 % 7, i * 3 // 7)

# 2. Floor semantics with NEGATIVE operands — the critical raw-vs-CPython case.
#    srem/sdiv truncate toward zero; the codegen correction must floor toward −∞.
print((-7) // 2, (-7) % 2)
print(7 // 2, 7 % 2)
print((-1) // 3, (-1) % 3)
print((-13) // 4, (-13) % 4)

# A bounded NEGATIVE-step loop whose induction variable goes negative and feeds a
# raw % / //; the floor correction is exercised on live raw operands.
for k in range(5, -6, -1):
    print(k, k % 3, k // 3)

# 3. The proof must REFUSE: x doubles 60 times → 2**60 exceeds the ±2^48 bound, so
#    x stays tagged and promotes to a heap bignum (byte-exact with CPython). A
#    raw narrowing here would miscompile.
x = 1
for _ in range(60):
    x = x * 2
print(x)

# 4. An unboundable while (collatz-shaped): n escapes any static bound via 3*n+1,
#    so it stays tagged and its %/// stay bignum-safe on the tagged baseline.
n = 27
steps = 0
while n != 1:
    if n % 2 == 0:
        n = n // 2
    else:
        n = 3 * n + 1
    steps = steps + 1
print(steps)

# 5. The induction variable read AFTER the loop (its final value survives).
last = -1
for j in range(10):
    last = j
print(last)

# 6. A small accumulator stays tagged while the index runs raw (the bench_containers
#    shape: append/checksum of i*7 % 13 over a literal-bounded loop).
total = 0
for m in range(100):
    total = total + (m * 7 % 13)
print(total)
