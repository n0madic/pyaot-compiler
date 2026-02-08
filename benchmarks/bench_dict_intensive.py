# Benchmark: Intensive dict operations
def main() -> None:
    # Test 1: Large dict creation and lookups
    size: int = 50000
    data: dict[int, int] = {}

    # Insert many elements
    i: int = 0
    while i < size:
        data[i] = i * 2
        i = i + 1

    # Many lookups
    lookup_sum: int = 0
    j: int = 0
    while j < size:
        if j in data:
            lookup_sum = lookup_sum + data[j]
        j = j + 1

    # Iteration
    key_sum: int = 0
    for key in data:
        key_sum = key_sum + key

    print("Size:", len(data))
    print("Lookup sum:", lookup_sum)
    print("Key sum:", key_sum)

if __name__ == "__main__":
    main()
