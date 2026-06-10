# Phase 7 GC soak — allocation-heavy raise/catch across calls.
# Exercises shadow-stack unwinding on longjmp and rootedness of `as e` bindings
# and live containers across try frames.

class WorkError(Exception):
    pass

def build(n: int) -> list[str]:
    out: list[str] = []
    for i in range(n):
        out.append("item-" + str(i))
    return out

def risky(n: int) -> list[str]:
    data = build(n)
    if n % 3 == 0:
        raise WorkError("mod3:" + str(n))
    return data

def worker(n: int) -> int:
    # Allocate before, inside, and after the try; raise across a call.
    pre = build(8)
    total = 0
    try:
        data = risky(n)
        total = total + len(data)
    except WorkError as e:
        msg = str(e)
        if len(msg) > 0:
            total = total - 1
    total = total + len(pre)
    return total

grand = 0
for round_i in range(60):
    grand = grand + worker(round_i % 7)
print("grand:", grand)

# ── nested try frames under allocation pressure ──
def nested(depth: int) -> str:
    junk = build(depth + 2)
    try:
        try:
            if depth > 0:
                raise ValueError("d" + str(depth))
            return "leaf:" + str(len(junk))
        except ValueError as inner:
            keep = build(6)
            raise WorkError(str(inner) + ":" + str(len(keep)))
    except WorkError as outer:
        return str(outer)

acc: list[str] = []
for d in range(20):
    acc.append(nested(d % 4))
print(len(acc), acc[0], acc[1], acc[5])

# ── raise/catch in a tight loop with live containers across frames ──
state: dict[str, int] = {}
for i in range(120):
    key = "k" + str(i % 10)
    try:
        if i % 2 == 0:
            raise WorkError(key)
        state[key] = i
    except WorkError as e:
        state[str(e)] = i * 2
print(len(state), state["k0"], state["k9"])

# ── exception value stays rooted while allocating in the handler ──
def churn_in_handler() -> int:
    try:
        raise WorkError("rooted-message-stays")
    except WorkError as e:
        total = 0
        for i in range(40):
            tmp = build(5)
            total = total + len(tmp)
        return total + len(str(e))

print(churn_in_handler())

print("p7_gc_stress done")
