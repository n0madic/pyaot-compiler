# Closure + higher-order hot path.
# Nested closures with local captures, plus comprehension-backed
# map/filter/reduce workloads.

from functools import reduce


def scale_by(multiplier: int, values: list[int]) -> int:
    # Captures `multiplier` from the enclosing scope.
    def scaled(v: int) -> int:
        return v * multiplier
    total: int = 0
    for v in values:
        total = total + scaled(v)
    return total


def main() -> None:
    n: int = 20_000

    # Closure capture from enclosing scope, called in a hot loop.
    xs: list[int] = [i for i in range(n)]
    total: int = scale_by(3, xs)

    # Comprehension-based map + filter (more idiomatic and well-supported).
    doubled: list[int] = [v * 2 for v in xs]
    evens: list[int] = [v for v in doubled if v % 4 == 0]
    sum_evens: int = reduce(lambda a, b: a + b, evens, 0)

    print("closures:", total, len(doubled), len(evens), sum_evens)


if __name__ == "__main__":
    main()
