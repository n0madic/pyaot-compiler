# Exception-handling overhead.
# The non-raising path is the one we care most about — it dominates every
# real program that only raises on error. We also measure the raising
# path, but with a much smaller N.

def maybe_raise(i: int) -> int:
    if i % 100_000 == 0 and i > 0:
        raise ValueError("tick")
    return i


def main() -> None:
    # Non-raising hot loop.
    total: int = 0
    for i in range(500_000):
        try:
            total = total + i
        except ValueError:
            total = total - 1

    # Raising path — much smaller N because each raise unwinds setjmp.
    caught: int = 0
    for i in range(500_000):
        try:
            total = total + maybe_raise(i)
        except ValueError:
            caught = caught + 1

    print("exceptions:", total, caught)


if __name__ == "__main__":
    main()
