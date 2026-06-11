# Phase 9 — GC root-set narrowing via liveness (B15 -> real dataflow).
# Exercises every shape the narrowing must keep sound; the differential gate
# runs this file under the gc_stress runtime too (a collection at EVERY
# allocation), where a missed use / wrong classification is a use-after-free.


# ── a str consumed before a series of allocations (provably un-rooted)
#    next to a str that lives ACROSS the allocation loop (must stay rooted) ──
def churn(n):
    early = "consumed-before-allocs"
    print(early)  # last use of `early` before any of the loop's allocations
    survivor = "lives-across-" + str(n)
    parts = []
    for i in range(n):
        parts.append("chunk" + str(i))  # allocates every iteration
    print(survivor)
    return len(parts)


print(churn(5))


# ── uses(I) rule: the argument of an allocating call, dead right after ──
def make_str(i):
    return "made-" + str(i)


def append_made(n):
    xs = []
    for i in range(n):
        xs.append(make_str(i))  # `make_str(i)` temp is used BY the allocating append
    return xs


print(append_made(4))


# ── handler rule: a pre-try value printed in the handler after an
#    allocating try body ──
def try_with_pre_value(flag):
    pre = "pre-try-" + str(flag)
    try:
        noise = []
        for i in range(6):
            noise.append("alloc" + str(i))
        if flag:
            raise ValueError("boom")
        return "no-raise"
    except ValueError:
        return pre  # `pre` must have survived the longjmp


print(try_with_pre_value(True))
print(try_with_pre_value(False))


# ── generator of strings: locals live across yields / allocations ──
def gen_strs(n):
    prefix = "gen-"
    for i in range(n):
        yield prefix + str(i)


for s in gen_strs(3):
    print(s)


# ── bignum promotion (an allocating BinOp on the tagged baseline)
#    with a live str across it ──
def bignum_with_live_str():
    label = "bignum-label"
    big = 2 ** 100
    big = big + 1  # tagged add on a heap BigInt — allocates
    print(label)
    return big


print(bignum_with_live_str())

print("p9 root narrowing passed!")
