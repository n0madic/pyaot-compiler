# Benchmark: a hot loop inside a never-raising try — measures the cost of the
# has_try memory-backing of locals (PITFALLS B17) and, later, the effect of
# cold-block placement for handler code (9C.5).


def safe_div(a: int, b: int) -> int:
    try:
        return a // b
    except ZeroDivisionError:
        return 0


def main() -> None:
    total = 0
    try:
        for i in range(2000000):
            total = (total + safe_div(1000000, i % 97 + 1)) % 1000000007
    except ValueError:
        total = -1
    print("bench_exc_hotpath checksum:", total)


main()
