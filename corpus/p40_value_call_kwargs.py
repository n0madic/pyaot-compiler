# Value-call passing keyword args into a kwonly / **kwargs closure.
# A closure called through a VALUE (the uniform indirect ABI) binds its
# keyword-only / **kwargs parameters from a keyword dict built at the call site;
# the common (no-keyword) path passes the null sentinel, which the closure's
# uniform thunk normalizes to an empty dict (so **kwargs inspection never crashes).


# ── **kwargs: inspect the dict (iterate / len / subscript) ──
def make_sum_kwargs():
    def f(**kwargs) -> int:
        total: int = 0
        for k in kwargs:
            total = total + kwargs[k]
        return total
    return f


g = make_sum_kwargs()
assert g(a=1, b=2, c=3) == 6, "kwargs sum"
assert g() == 0, "empty kwargs (null sentinel -> empty dict)"
assert g(x=10) == 10, "single kwarg"
print("**kwargs inspect:", g(a=1, b=2, c=3), g(), g(x=10))


def make_count_kwargs():
    def f(**kwargs) -> int:
        return len(kwargs)
    return f


k = make_count_kwargs()
assert k() == 0, "len of empty kwargs"
assert k(p=1, q=2) == 2, "len of kwargs"
print("**kwargs len:", k(), k(p=1, q=2))


# ── **d forward + named/**d merge into a closure ──
d_forward = {"x": 10, "y": 20}
assert g(**d_forward) == 30, "**d forward"
assert g(a=1, **d_forward) == 31, "named + **d merge"
print("**d forward/merge:", g(**d_forward), g(a=1, **d_forward))


# ── keyword-only parameters bound by keyword / by default ──
def make_kwonly():
    def f(a: int, *, b: int = 10) -> int:
        return a + b
    return f


h = make_kwonly()
assert h(5) == 15, "kwonly default"
assert h(5, b=20) == 25, "kwonly by keyword"
print("kwonly:", h(5), h(5, b=20))


def make_kwonly_required():
    def f(*, name: str, count: int = 1) -> str:
        return name + ":" + str(count)
    return f


m = make_kwonly_required()
assert m(name="hi") == "hi:1", "kwonly required + default"
assert m(name="x", count=3) == "x:3", "kwonly required + override"
print("kwonly required:", m(name="hi"), m(name="x", count=3))


# ── *args + **kwargs closure: positional via *args, keyword via **kwargs ──
def make_both():
    def f(*args: int, **kwargs: int) -> int:
        total: int = 0
        for v in args:
            total = total + v
        for kk in kwargs:
            total = total + kwargs[kk]
        return total
    return f


b = make_both()
assert b(1, 2, 3) == 6, "both: positional only"
assert b(a=10, b=20) == 30, "both: keyword only"
assert b(1, 2, x=3, y=4) == 10, "both: mixed"
print("*args+**kwargs:", b(1, 2, 3), b(a=10, b=20), b(1, 2, x=3, y=4))


# ── a wrapper closure forwarding `func(*args, **kwargs)` with keywords ──
# (the wrapper captures `func`, a **kwargs closure passed as a VALUE, and forwards
# both positional and keyword args through the uniform indirect ABI)
def passthrough(func):
    def wrapper(*args, **kwargs):
        return func(*args, **kwargs)
    return wrapper


def kw_consumer(**kwargs) -> int:
    return len(kwargs)


wrapped = passthrough(kw_consumer)
assert wrapped() == 0, "wrapper-forward empty"
assert wrapped(a=1, b=2) == 2, "wrapper-forward keywords"
print("wrapper forward:", wrapped(), wrapped(a=1, b=2))

print("All value-call kwargs tests passed!")
