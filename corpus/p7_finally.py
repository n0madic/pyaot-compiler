# Phase 7B gate — finally, else, raise-from chaining, exception instance surface.

order: list[str] = []

# ── finally on the normal path ──
try:
    order.append("try")
finally:
    order.append("finally")
print(order)

# ── finally on the exceptional path (caught outside) ──
steps: list[str] = []
try:
    try:
        steps.append("body")
        raise ValueError("x")
    finally:
        steps.append("finally")
except ValueError:
    steps.append("caught")
print(steps)

# ── try/except/else/finally ordering ──
def full(raise_it: bool) -> str:
    log = ""
    try:
        log = log + "T"
        if raise_it:
            raise ValueError("v")
    except ValueError:
        log = log + "E"
    else:
        log = log + "L"
    finally:
        log = log + "F"
    return log

print(full(False), full(True))

# ── return inside try still runs finally ──
trace: list[str] = []

def ret_through_finally() -> int:
    try:
        trace.append("in")
        return 7
    finally:
        trace.append("fin")

print(ret_through_finally(), trace)

# ── return inside except runs finally ──
def ret_from_handler() -> int:
    try:
        raise ValueError("v")
    except ValueError:
        return 1
    finally:
        trace.append("fin2")

print(ret_from_handler(), trace)

# ── break/continue through finally ──
def loop_finally() -> int:
    count = 0
    for i in range(5):
        try:
            if i == 1:
                continue
            if i == 3:
                break
            count = count + 1
        finally:
            count = count + 10
    return count

print(loop_finally())

# ── exception in else is not caught by the same try ──
def else_raises() -> str:
    try:
        try:
            x = 1
        except ValueError:
            return "wrong"
        else:
            raise TypeError("from-else")
    except TypeError:
        return "outer"

print(else_raises())

# ── nested finally ──
def nested_finally() -> str:
    log = ""
    try:
        try:
            log = log + "a"
            raise ValueError("v")
        finally:
            log = log + "b"
    except ValueError:
        log = log + "c"
    finally:
        log = log + "d"
    return log

print(nested_finally())

# ── raise X from Y / from None ──
def raise_from() -> str:
    try:
        raise ValueError("main") from TypeError("cause")
    except ValueError:
        return "from-caught"

print(raise_from())

def raise_from_none() -> str:
    try:
        try:
            raise ValueError("orig")
        except ValueError:
            raise TypeError("new") from None
    except TypeError:
        return "from-none"

print(raise_from_none())

# ── instance surface: str(e), print(e), e.args, e.__class__.__name__ ──
try:
    raise ValueError("boom")
except ValueError as e:
    print(str(e))
    print(e)
    print(e.args[0])
    print(e.__class__.__name__)

try:
    raise RuntimeError("rt-message")
except RuntimeError as e2:
    print(str(e2))
    print(e2.__class__.__name__)

# str(e) of a message-less exception is empty; its args tuple is ()
try:
    raise TypeError()
except TypeError as e3:
    print("empty:[" + str(e3) + "]")
    print("args-len:", len(e3.args))

# ── tuple-clause `as` binding keeps the exception-message surface ──
def tuple_clause_msg(kind: int) -> str:
    try:
        if kind == 0:
            raise ValueError("tc-value")
        raise RuntimeError("tc-runtime")
    except (ValueError, RuntimeError) as e:
        print(e)
        return str(e)

print(tuple_clause_msg(0), tuple_clause_msg(1))

try:
    raise ValueError("tc-args")
except (ValueError, RuntimeError) as e4:
    print(e4.args[0])

# ── min()/max() on empty input raise with the live-oracle message ──
try:
    empty_l: list[int] = []
    bad = min(empty_l)
except ValueError as e5:
    print(e5)
try:
    empty_l2: list[int] = []
    bad2 = max(empty_l2)
except ValueError as e6:
    print(e6)

# ── min/max with a builtin key function ──
vals: list[int] = [3, -7, 2, -1]
print(min(vals, key=abs), max(vals, key=abs))

# ── starred unpacking of a literal RHS ──
first, *mid, last = [1, 2, 3, 4]
print(first, mid, last)

print("p7_finally done")
