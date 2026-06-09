# GC stress + float specialization (Phase 3).
#
# Two heap roots must stay live across an allocating loop, or the collector
# frees them and this prints garbage / crashes:
#   * `s`    — a heap string created once and printed at the end;
#   * `bacc` — a heap bignum accumulator, re-boxed every iteration.
# The bignum loop allocates a fresh `2 ** 64` and a fresh sum each step, forcing
# several collections while `s` and `bacc` are live across them. This is what
# keeps the shadow-frame rooting honest now that float arithmetic no longer
# allocates (see below).
#
# The float loop is kept deliberately: after Phase 3b it compiles to unboxed
# `fadd` (no boxing, no GC traffic), exercising the Raw(F64) arithmetic path
# end-to-end. Its result must still match CPython byte-for-byte.
s = "survivor string that must not be freed"
facc = 0.0
for i in range(300000):
    facc = facc + 1.5
bacc = 0
for j in range(100000):
    bacc = bacc + 2 ** 64
print(s)
print(facc)
print(bacc)
print(len(s))
