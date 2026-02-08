# Benchmark: Intensive list operations
def main() -> None:
    # Create large list
    iterations: int = 100000
    numbers: list[int] = []
    i: int = 0
    while i < iterations:
        numbers.append(i)
        i = i + 1

    # Multiple passes of computation
    j: int = 0
    while j < 10:
        total: int = 0
        for num in numbers:
            total = total + num
        j = j + 1

    print("Result:", total)

if __name__ == "__main__":
    main()
