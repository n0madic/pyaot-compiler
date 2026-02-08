# Benchmark: List sorting performance
def main() -> None:
    # Test 1: Sort already sorted list (best case for Timsort)
    size: int = 10000
    sorted_list: list[int] = []
    i: int = 0
    while i < size:
        sorted_list.append(i)
        i = i + 1

    # Sort 10 times
    j: int = 0
    while j < 10:
        sorted_list.sort()
        j = j + 1

    # Test 2: Sort reverse sorted list (worst case for bubble sort)
    reverse_list: list[int] = []
    k: int = size - 1
    while k >= 0:
        reverse_list.append(k)
        k = k - 1

    # Sort 10 times
    m: int = 0
    while m < 10:
        reverse_list.sort()
        m = m + 1

    # Test 3: Sort random-ish list
    random_list: list[int] = []
    n: int = 0
    while n < size:
        # Simple pseudo-random pattern
        val: int = (n * 7 + 13) % size
        random_list.append(val)
        n = n + 1

    # Sort 10 times
    p: int = 0
    while p < 10:
        random_list.sort()
        p = p + 1

    print("Sorted list sum:", sorted_list[0] + sorted_list[size-1])
    print("Reverse list sum:", reverse_list[0] + reverse_list[size-1])
    print("Random list sum:", random_list[0] + random_list[size-1])

if __name__ == "__main__":
    main()
