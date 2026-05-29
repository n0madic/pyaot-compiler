"""Repro for Wave 3a: complete free-var capture coverage.

Each nested function references an outer variable from a position the free-var
visitor previously skipped (loop `else`, `try`/`else`, `match` case, slice
bounds, walrus value). A missed capture silently miscompiles the closure.
"""


def for_else_capture() -> int:
    base = 100

    def inner() -> int:
        total = 0
        for i in range(3):
            total += i
        else:
            total += base
        return total

    return inner()


def while_else_capture() -> int:
    bonus = 7

    def inner() -> int:
        total = 0
        n = 0
        while n < 3:
            total += n
            n += 1
        else:
            total += bonus
        return total

    return inner()


def try_else_capture() -> int:
    extra = 11

    def inner() -> int:
        total = 0
        try:
            total += 1
        except ValueError:
            total += 999
        else:
            total += extra
        return total

    return inner()


def match_capture(n: int) -> int:
    factor = 10

    def inner(x: int) -> int:
        match x:
            case 0:
                return factor
            case _:
                return x * factor

    return inner(n)


def slice_capture() -> list[int]:
    lo = 1
    hi = 3
    data = [10, 20, 30, 40, 50]

    def inner() -> list[int]:
        return data[lo:hi]

    return inner()


def walrus_capture() -> int:
    seed = 5

    def inner() -> int:
        return (x := seed + 1) + x

    return inner()


def main() -> None:
    print(for_else_capture())
    print(while_else_capture())
    print(try_else_capture())
    print(match_capture(0))
    print(match_capture(3))
    print(slice_capture())
    print(walrus_capture())


main()
