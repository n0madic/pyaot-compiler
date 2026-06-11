# Phase 8H D4 — by-name field access on a Dyn receiver.
# Fields of a dynamically-typed instance resolve through the runtime
# FIELD_NAME_REGISTRY; a miss raises AttributeError (caught, not a crash).


class Node:
    def __init__(self, data: float):
        self.data = data
        self.grad = 0.0


class Pair:
    def __init__(self, left, right):
        self.left = left
        self.right = right


def pick(flag):
    if flag:
        return Node(1.5)
    return "not a node"


# Dyn receiver (unannotated function return) — field read by name
n = pick(True)
print(n.data)

# field write by name, then read back
n.grad = 2.5
print(n.grad)

# Dyn elements out of a heterogeneous tuple
def make_pair():
    return Pair(Node(10.0), Node(20.0))


p = make_pair()
print(p.left.data + p.right.data)

# isinstance-ternary pattern (microgpt's `other.data`)
def add_data(a, b):
    bb = b if isinstance(b, Node) else Node(float(b))
    return a.data + bb.data


print(add_data(Node(3.0), Node(4.0)))
print(add_data(Node(3.0), 2))

# class elements through sum()'s inferred __add__ returns
class Acc:
    def __init__(self, v: int):
        self.v = v

    def __add__(self, other):
        return Acc(self.v + other.v)

    def __radd__(self, other):
        return Acc(self.v + other)


s = sum([Acc(1), Acc(2), Acc(3)])
print(s.v)

# AttributeError on a missing field / non-instance
try:
    print(n.missing)
except AttributeError:
    print("missing field caught")

bad = pick(False)
try:
    print(bad.data)
except AttributeError:
    print("non-instance caught")

print("p8h dyn attr passed!")
