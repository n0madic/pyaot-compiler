# B10 — cross-instance field-type inference as solver variables: unannotated
# fields are typed by the join of every `obj.field = value` write across the
# whole module (not just the best-effort `self.field = ...` scan), and the
# autograd pattern (`child.grad = child.grad + local * out.grad` over
# children lists) stays CPython-exact. Fields fed through Dyn receivers or
# Dyn-typed values soundly demote to Dyn (tagged); recovering their float
# specialization needs parameter inference (the deferred half of B10).


# ── the autograd pattern: self-referential writes through non-self receivers ──
class Value:
    def __init__(self, data):
        self.data = data
        self.grad = 0.0
        self._prev = []
        self._local = []

    def __add__(self, other):
        out = Value(self.data + other.data)
        out._prev = [self, other]
        out._local = [1.0, 1.0]
        return out

    def __mul__(self, other):
        out = Value(self.data * other.data)
        out._prev = [self, other]
        out._local = [other.data, self.data]
        return out

    def backward_step(self):
        for i in range(len(self._prev)):
            child = self._prev[i]
            child.grad = child.grad + self._local[i] * self.grad


a = Value(2.0)
b = Value(3.0)
c = a * b
d = c + a
d.grad = 1.0
d.backward_step()
c.backward_step()
print(a.data, b.data, c.data, d.data)
print(a.grad, b.grad, c.grad, d.grad)


# ── mixed int/float writes demote the field (and the program still compiles) ──
class Mixed:
    def __init__(self, flag):
        if flag:
            self.v = 1.5
        else:
            self.v = 7


m1 = Mixed(True)
m2 = Mixed(False)
print(m1.v, m2.v)


# ── a subclass writing an inherited field feeds the base class's variable ──
class Counter:
    def __init__(self):
        self.total = 0.0

    def bump(self, amount):
        self.total = self.total + amount


class DoubleCounter(Counter):
    def bump2(self, amount):
        self.total = self.total + amount * 2.0


dc = DoubleCounter()
dc.bump(1.25)
dc.bump2(2.0)
print(dc.total)


# ── an annotated field stays authoritative ──
class Tagged:
    label: str

    def __init__(self, label: str):
        self.label = label


t = Tagged("ok")
print(t.label)

print("b10 field inference passed!")
