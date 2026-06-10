# ── positional defaults ──
def power(base, exp=2):
    result = 1
    for _ in range(exp):
        result = result * base
    return result


print(power(5))
print(power(5, 3))
print(power(base=4))
print(power(exp=3, base=2))


# ── multiple defaults, keyword reordering ──
def greet(name, greeting="Hello", punct="!"):
    return greeting + ", " + name + punct


print(greet("World"))
print(greet("World", "Hi"))
print(greet("World", punct="?"))
print(greet("World", greeting="Hey", punct="."))
print(greet(greeting="Yo", name="Sam"))


# ── keyword-only parameters ──
def make(width, *, height=10, label="box"):
    return label + ":" + str(width) + "x" + str(height)


print(make(5))
print(make(5, height=20))
print(make(5, label="rect", height=7))


# ── a fixed param plus **kwargs for extras ──
def config(name, **opts):
    base = name
    keys = sorted(opts.keys())
    for k in keys:
        base += " " + k + "=" + str(opts[k])
    return base


print(config("srv"))
print(config("srv", port=8080, debug=1))


# ── default values of several literal kinds ──
def flags(a=1, b=True, c="x", d=None):
    return str(a) + "/" + str(b) + "/" + c + "/" + str(d)


print(flags())
print(flags(2, False, "y", 3))
print(flags(c="z"))
