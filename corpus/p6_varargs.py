# ── *args: a function over an arbitrary number of positionals ──
def total(*nums):
    s = 0
    for n in nums:
        s += n
    return s


print(total())
print(total(1, 2, 3))
print(total(10, 20))


# ── fixed param followed by *args ──
def greet(greeting, *names):
    out = greeting
    for n in names:
        out += " " + n
    return out


print(greet("Hi"))
print(greet("Hi", "Alice", "Bob"))


# ── **kwargs collecting keyword args ──
def describe(**attrs):
    keys = sorted(attrs.keys())
    out = ""
    for k in keys:
        out += k + "=" + str(attrs[k]) + ";"
    return out


print(describe())
print(describe(a=1, b=2))
print(describe(z=26, a=1, m=13))


# ── fixed + *args + **kwargs together ──
def both(first, *rest, **opts):
    return str(first) + "/" + str(len(rest)) + "/" + str(len(opts))


print(both(1))
print(both(1, 2, 3))
print(both(1, 2, x=9))
print(both(0, 1, 2, 3, a=1, b=2))


# ── forwarding `*args` through another varargs function ──
def forward(*a):
    return total(*a)


print(forward(4, 5, 6))
print(forward())


# ── len() and iteration over the *args tuple ──
def count_and_sum(*xs):
    return len(xs)


print(count_and_sum(1, 2, 3, 4))
