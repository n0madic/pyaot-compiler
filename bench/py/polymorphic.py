# Polymorphic arithmetic via dunder dispatch.
# Mirrors the microgpt Value class — the pattern that drives most of the
# devirtualization / inline-cache work and is extremely sensitive to
# per-call vtable dispatch cost.

class Value:
    data: float
    grad: float

    def __init__(self, data: float) -> None:
        self.data = data
        self.grad = 0.0

    def __add__(self, other: "Value") -> "Value":
        return Value(self.data + other.data)

    def __mul__(self, other: "Value") -> "Value":
        return Value(self.data * other.data)


def main() -> None:
    n: int = 200_000
    acc: Value = Value(0.0)
    one: Value = Value(1.0)
    half: Value = Value(0.5)
    for i in range(n):
        acc = acc + one
        acc = acc * half + one
    print("polymorphic:", acc.data)


if __name__ == "__main__":
    main()
