# Benchmark: List operations
def main() -> None:
    iterations: int = 10000

    # List creation and appending
    numbers: list[int] = []
    i: int = 0
    while i < iterations:
        numbers.append(i)
        i = i + 1

    # List iteration and sum
    total: int = 0
    for num in numbers:
        total = total + num

    print("List sum:", total)

    # List slicing
    slice_sum: int = 0
    middle: list[int] = numbers[1000:9000]
    for num in middle:
        slice_sum = slice_sum + num

    print("Slice sum:", slice_sum)

    # List comprehension with filter
    evens: list[int] = []
    for num in numbers:
        if num % 2 == 0:
            evens.append(num)

    print("Even count:", len(evens))

if __name__ == "__main__":
    main()
