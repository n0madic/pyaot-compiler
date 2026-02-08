# Benchmark: Dictionary operations
def main() -> None:
    iterations: int = 10000

    # Dictionary creation and insertion
    data: dict[int, int] = {}
    i: int = 0
    while i < iterations:
        data[i] = i * 2
        i = i + 1

    # Dictionary lookup
    lookup_sum: int = 0
    j: int = 0
    while j < iterations:
        if j in data:
            lookup_sum = lookup_sum + data[j]
        j = j + 1

    print("Lookup sum:", lookup_sum)

    # Dictionary iteration
    key_sum: int = 0
    for key in data:
        key_sum = key_sum + key

    print("Key sum:", key_sum)

    # Dictionary size
    print("Dict size:", len(data))

if __name__ == "__main__":
    main()
