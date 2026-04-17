# Integer arithmetic hot loop.
# Sums the first 10M naturals through range() iteration — the canonical
# "how fast can we do an integer tight loop" probe.

def main() -> None:
    n: int = 10_000_000
    total: int = 0
    for i in range(n):
        total = total + i
    print("int_arith:", total)


if __name__ == "__main__":
    main()
