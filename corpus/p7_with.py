# Phase 7D gate — context managers (with statement).

log: list[str] = []

class Tracker:
    name: str

    def __init__(self, name: str):
        self.name = name

    def __enter__(self) -> str:
        log.append(self.name + ":enter")
        return self.name

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        log.append(self.name + ":exit")
        return False

# ── normal path ──
with Tracker("a") as v:
    log.append("body:" + v)
print(log)

# ── exception path: __exit__ runs, exception propagates ──
log2: list[str] = []

class Tracker2:
    def __enter__(self) -> int:
        log2.append("enter")
        return 1

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        log2.append("exit:" + str(bool(exc_type)))
        return False

def with_raises() -> str:
    try:
        with Tracker2():
            log2.append("body")
            raise ValueError("inside")
    except ValueError:
        return "propagated"

print(with_raises(), log2)

# ── suppression: truthy __exit__ swallows the exception ──
class Quiet:
    def __enter__(self) -> int:
        return 0

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        return bool(exc_type)

def suppressed() -> str:
    with Quiet():
        raise ValueError("swallowed")
    return "after"

print(suppressed())

# ── multiple items nest left-to-right ──
log3: list[str] = []

class Item:
    tag: str

    def __init__(self, tag: str):
        self.tag = tag

    def __enter__(self) -> str:
        log3.append("e" + self.tag)
        return self.tag

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        log3.append("x" + self.tag)
        return False

with Item("1") as a, Item("2") as b:
    log3.append(a + b)
print(log3)

# ── inner suppressor hides exception from the outer manager ──
class SawIt:
    saw: bool

    def __init__(self):
        self.saw = False

    def __enter__(self) -> int:
        return 0

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        self.saw = bool(exc_type)
        return False

outer = SawIt()
with outer, Quiet():
    raise ValueError("inner only")
print("outer saw:", outer.saw)

# ── return inside with runs __exit__ ──
log4: list[str] = []

class R:
    def __enter__(self) -> int:
        return 5

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        log4.append("exit")
        return False

def ret_in_with() -> int:
    with R() as n:
        return n + 1

print(ret_in_with(), log4)

# ── break/continue out of with inside a loop ──
def loop_with() -> int:
    total = 0
    for i in range(5):
        with R():
            if i == 1:
                continue
            if i == 3:
                break
            total = total + i
    return total

print(loop_with(), len(log4))

# ── varargs __exit__ ──
class Star:
    def __enter__(self) -> int:
        return 9

    def __exit__(self, *a) -> bool:
        return False

with Star() as s:
    print("star:", s)

# ── tuple target ──
class Pair:
    def __enter__(self):
        return (10, 20)

    def __exit__(self, *a) -> bool:
        return False

with Pair() as (x, y):
    print("pair:", x, y)

# ── forward-reference string annotation on __enter__ ──
class SelfYield:
    tag: int

    def __init__(self, tag: int):
        self.tag = tag

    def __enter__(self) -> "SelfYield":
        return self

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        return False

with SelfYield(7) as sy:
    print("selfyield:", sy.tag)

# ── nested with + try around it ──
def nested() -> str:
    t = SawIt()
    try:
        with t:
            with Tracker("n") as inner:
                raise KeyError("deep")
    except KeyError:
        if t.saw:
            return "all-good"
        return "outer-missed"

print(nested())

print("p7_with done")
