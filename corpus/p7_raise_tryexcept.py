# Phase 7A gate — raise + try/except over builtin exceptions.
# No finally / with / custom classes / instance surface here (later stages).

# ── basic catch ──
caught = False
try:
    raise ValueError("boom")
except ValueError:
    caught = True
print("basic catch:", caught)

# ── no exception: except skipped ──
ran = False
skipped = True
try:
    ran = True
except ValueError:
    skipped = False
print("no exception:", ran, skipped)

# ── specific handler chain picks the right clause ──
def classify(kind: int) -> str:
    try:
        if kind == 0:
            raise ValueError("v")
        if kind == 1:
            raise TypeError("t")
        if kind == 2:
            raise KeyError("k")
        return "none"
    except ValueError:
        return "value"
    except TypeError:
        return "type"
    except KeyError:
        return "key"

print(classify(0), classify(1), classify(2), classify(3))

# ── tuple clause (OR-chain) ──
def multi(kind: int) -> str:
    try:
        if kind == 0:
            raise ValueError("v")
        raise IndexError("i")
    except (ValueError, IndexError):
        return "either"

print(multi(0), multi(1))

# ── Exception catches subclass-tagged builtins ──
def base_catch() -> str:
    try:
        raise RuntimeError("r")
    except Exception:
        return "base"

print(base_catch())

# ── bare except ──
def bare() -> str:
    try:
        raise AttributeError("a")
    except:
        return "bare"

print(bare())

# ── runtime-raised exceptions are catchable ──
def div(a: int, b: int) -> int:
    try:
        return a // b
    except ZeroDivisionError:
        return -1

print(div(10, 2), div(10, 0))

def parse(s: str) -> int:
    try:
        return int(s)
    except ValueError:
        return -1

print(parse("42"), parse("nope"))

# ── nested try, inner catches ──
def nested_inner() -> str:
    try:
        try:
            raise ValueError("inner")
        except ValueError:
            return "inner-caught"
    except ValueError:
        return "outer-caught"

print(nested_inner())

# ── nested try, no inner match → outer catches ──
def nested_outer() -> str:
    try:
        try:
            raise TypeError("inner")
        except ValueError:
            return "wrong"
    except TypeError:
        return "outer"

print(nested_outer())

# ── bare raise re-raises ──
def reraise() -> str:
    try:
        try:
            raise ValueError("once")
        except ValueError:
            raise
    except ValueError:
        return "reraised"

print(reraise())

# ── unmatched handler propagates to the caller ──
def raiser():
    raise KeyError("deep")

def call_catch() -> str:
    try:
        raiser()
        return "no"
    except KeyError:
        return "from-callee"

print(call_catch())

# ── raise without arguments list (bare class) ──
def bare_class() -> str:
    try:
        raise ValueError
    except ValueError:
        return "bare-class"

print(bare_class())

# ── raise without message ──
def no_msg() -> str:
    try:
        raise TypeError()
    except TypeError:
        return "no-msg"

print(no_msg())

# ── variables assigned in try survive the unwind ──
def preserve() -> int:
    x = 0
    y = 0.0
    s = ""
    try:
        x = 41
        y = 2.5
        s = "kept"
        raise ValueError("u")
    except ValueError:
        pass
    if s == "kept" and y == 2.5:
        return x + 1
    return -1

print(preserve())

# ── try inside a loop with break/continue ──
def loop_try() -> int:
    total = 0
    for i in range(6):
        try:
            if i == 2:
                raise ValueError("skip")
            if i == 4:
                break
            total = total + i
        except ValueError:
            continue
    return total

print(loop_try())

# ── return inside try (normal path pops the frame) ──
def ret_in_try(flag: bool) -> str:
    try:
        if flag:
            return "early"
        raise ValueError("late")
    except ValueError:
        return "handled"

print(ret_in_try(True), ret_in_try(False))

# ── as-binding accepted (instance surface tested in 7B) ──
def as_bind() -> str:
    try:
        raise ValueError("bound")
    except ValueError as e:
        return "as-ok"

print(as_bind())

# ── exception raised inside except handler propagates outward ──
def raise_in_handler() -> str:
    try:
        try:
            raise ValueError("first")
        except ValueError:
            raise TypeError("second")
    except TypeError:
        return "chained"

print(raise_in_handler())

print("p7_raise_tryexcept done")
