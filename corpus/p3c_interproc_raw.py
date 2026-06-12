from typing import Callable

# Phase 3c — interprocedural raw-int specialization (PLAN backlog #7, Part A).
# Every line must match CPython byte-for-byte whether a param/return runs raw
# (Raw(I64)) or on the tagged baseline — precision never changes correctness
# (Principle 2). The bignum cases below would MISCOMPILE (an unchecked untag of a
# heap BigInt read as garbage) if a non-specializable or unbounded position were
# wrongly made raw, so a clean run IS the soundness proof.

# (a) The bench_exc_hotpath shape, minimized: bounded args flow across a direct
#     call edge, so safe_div's params and return all go raw (Raw(I64)).
def safe_div(a: int, b: int) -> int:
    return a // b


acc = 0
for i in range(200):
    acc = (acc + safe_div(1000, i % 13 + 1)) % 100003
print("a:", acc)

# (b) Address-taken: `dbl` is passed as a value (MakeClosure → its FuncId is in
#     the address-taken set), so it must stay tagged even though it is ALSO called
#     directly with a bounded arg. The indirect call hands it a tagged bignum.
def dbl(n: int) -> int:
    return n + n


def apply_fn(f: Callable[[int], int], x: int) -> int:
    return f(x)


print("b:", apply_fn(dbl, 10 ** 30), dbl(21))

# (c) Per-position: `a` is bounded at both call sites (goes raw); `b` is unbounded
#     at the second site (a heap bignum), so its entry interval joins to ⊤ and it
#     stays tagged. The mixed result is a bignum that must print exactly.
def mix(a: int, b: int) -> int:
    return a * 2 + b


print("c:", mix(3, 100), mix(7, 10 ** 40))

# (d) A recursive bounded function — the interprocedural fixpoint must TERMINATE
#     (widening pins the self-call's climbing arg interval); the result is exact
#     regardless of whether the param ends up raw or tagged.
def countdown(n: int) -> int:
    if n <= 0:
        return 0
    return countdown(n - 1) + n


print("d:", countdown(50))

# (e) A raw-param / raw-return function called from INSIDE a try (the Tail
#     trampoline seam — PITFALLS B3): the raw ABI must flow through the trampoline
#     unchanged on both the normal and the exceptional edge.
def add3(a: int, b: int, c: int) -> int:
    return a + b + c


def run() -> int:
    s = 0
    try:
        for i in range(500):
            s = (s + add3(i, i * 2, 1)) % 100003
    except ValueError:
        s = -1
    return s


print("e:", run())
