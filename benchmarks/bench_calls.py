# Benchmark: microgpt proxy — a small autograd-style class whose hot path is
# dunder calls (__add__/__mul__) plus a closure, allocating instances on every
# iteration. This is the primary target of the MIR inliner (9C.4).


class Vec:
    def __init__(self, x: float, y: float):
        self.x = x
        self.y = y

    def __add__(self, other):
        return Vec(self.x + other.x, self.y + other.y)

    def __mul__(self, other):
        return Vec(
            self.x * other.x - self.y * other.y,
            self.x * other.y + self.y * other.x,
        )


def make_step(scale: float):
    def step(v, w):
        return (v + w) * Vec(scale, scale * 0.1)
    return step


def main() -> None:
    f = make_step(0.6)
    v = Vec(1.0, 0.0)
    w = Vec(0.25, -0.125)
    acc = 0.0
    for i in range(3000000):
        v = f(v, w)
        if i % 10000 == 0:
            acc += v.x + v.y
    print("bench_calls checksum:", acc)
    print("bench_calls final:", v.x, v.y)


main()
