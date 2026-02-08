# Benchmark: String operations (emphasizing long patterns for BMH)
def main() -> None:
    iterations: int = 10000

    # Create a large source string for search operations
    source: str = "The quick brown fox jumps over the lazy dog. " * 100  # 4500 chars

    # Test 1: find() with long patterns (triggers BMH >= 4 chars)
    find_count: int = 0
    i: int = 0
    while i < iterations:
        # Long pattern that appears once
        pos1: int = source.find("quick brown")  # 11 chars
        pos2: int = source.find("lazy dog")     # 8 chars
        pos3: int = source.find("jumps over")   # 10 chars
        if pos1 >= 0 and pos2 >= 0 and pos3 >= 0:
            find_count = find_count + 1
        i = i + 1

    print("Find operations:", find_count)

    # Test 2: contains (in operator) with long patterns
    contains_count: int = 0
    j: int = 0
    while j < iterations:
        if "quick brown" in source and "lazy dog" in source:
            contains_count = contains_count + 1
        j = j + 1

    print("Contains operations:", contains_count)

    # Test 3: count() with long patterns
    count_sum: int = 0
    k: int = 0
    while k < iterations:
        c1: int = source.count("brown fox")   # 9 chars
        c2: int = source.count("over the")    # 8 chars
        count_sum = count_sum + c1 + c2
        k = k + 1

    print("Count sum:", count_sum)

    # Test 4: replace() with long patterns
    replace_count: int = 0
    m: int = 0
    while m < 100:  # Fewer iterations since replace allocates
        replaced: str = source.replace("quick brown", "QUICK BROWN")
        if "QUICK BROWN" in replaced:
            replace_count = replace_count + 1
        m = m + 1

    print("Replace operations:", replace_count)

    # Test 5: split() with longer separator
    split_count: int = 0
    n: int = 0
    while n < 100:
        parts: list[str] = source.split("over the")
        split_count = split_count + len(parts)
        n = n + 1

    print("Split parts total:", split_count)

if __name__ == "__main__":
    main()
