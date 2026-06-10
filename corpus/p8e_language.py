# Phase 8E — language gaps for real scripts: f-string format specs, slicing,
# str.join / list.index, tuple () parameter defaults + __slots__, and the
# return-type inference that lets unannotated dunder/method results stay typed.

# ── f-string format specs ──
step = 42
total = 1000
loss = 3.14159265
acc = 0.8765
print(f"step {step+1:4d} / {total:4d} | loss {loss:.4f}")
print(f"acc={acc*100:.1f}% [{step:05d}] {'ok':>6} {loss:8.3f}")

# ── slicing: list / str / tuple, negative, stepped, open-ended ──
xs = [0, 1, 2, 3, 4, 5, 6, 7]
print(xs[2:5], xs[:3], xs[5:], xs[-3:], xs[:-2], xs[1:7:2], xs[::2], xs[::-1])
hs, head_dim = 2, 3
print(xs[hs:hs + head_dim])
s = "abcdefgh"
print(s[2:5], s[::-1], s[:4])
t = (10, 20, 30, 40, 50)
print(t[1:4])
rows = [[1, 2, 3, 4], [5, 6, 7, 8]]
print([r[1:3] for r in rows])

# ── str.join / list.index / str.strip ──
words = ["emma", "olivia", "ava"]
print("".join(words), "-".join(words), ", ".join(words))
chars = ['a', 'b', 'c', 'd']
print(chars.index('c'))
line = "  hello world  \n"
print("[" + line.strip() + "]")
print("".join([str(n) for n in [1, 2, 3]]))


# ── tuple () parameter default + __slots__ ──
class Node:
    __slots__ = ('data', 'kids')

    def __init__(self, data, kids=()):
        self.data = data
        self.kids = kids

    def child_count(self):
        return len(self.kids)


a = Node(10)
b = Node(20, (a,))
print(a.data, a.child_count(), b.data, b.child_count())


# ── return-type inference: unannotated dunder / method chains stay typed ──
class Value:
    def __init__(self, data):
        self.data = data

    def __add__(self, other):
        o = other if isinstance(other, Value) else Value(other)
        return Value(self.data + o.data)

    def __sub__(self, other):
        o = other if isinstance(other, Value) else Value(other)
        return Value(self.data - o.data)

    def __mul__(self, other):
        o = other if isinstance(other, Value) else Value(other)
        return Value(self.data * o.data)

    def relu(self):
        return Value(self.data if self.data > 0.0 else 0.0)

    def squared(self):
        return self * self


def make(x):
    return Value(x)


logits: list[Value] = [Value(-1.0), Value(2.0), Value(3.0)]
max_val = 2.0
for v in logits:
    rv = (v - max_val).relu()         # method on a dunder result (inferred Value)
    print(f"{rv.data:.4f}")
print(make(5.0).squared().data)       # method chain across an unannotated function

acc2: Value = logits[0]
for i in range(1, 3):
    acc2 = acc2 + logits[i]
print(acc2.data)


# ── comprehension loop variables do not leak into the enclosing scope ──
i = 99
squares = [i * i for i in range(4)]
print(squares, i)                     # i stays 99 (comprehension has its own scope)
pairs = [(j, k) for j in range(2) for k in range(2)]
j = "outer-j"
both = [j + str(k) for j, k in [("a", 1), ("b", 2)]]
print(pairs, both, j)                 # j stays "outer-j"


# ── str() / repr() honour user __str__ / __repr__ (and the __repr__ fallback) ──
class Labelled:
    def __init__(self, n):
        self.n = n

    def __repr__(self):
        return "repr<" + str(self.n) + ">"

    def __str__(self):
        return "str<" + str(self.n) + ">"


class ReprOnly:
    def __init__(self, n):
        self.n = n

    def __repr__(self):
        return "RO(" + str(self.n) + ")"


lab = Labelled(1)
print(lab, str(lab), repr(lab))       # str<1> str<1> repr<1>
ro = ReprOnly(2)
print(ro, str(ro), repr(ro))          # RO(2) RO(2) RO(2) — str falls back to __repr__
print(f"{lab} | {ro!r}")
