# Benchmark: Intensive arithmetic operations
def fibonacci(n: int) -> int:
    if n <= 1:
        return n
    return fibonacci(n - 1) + fibonacci(n - 2)

def main() -> None:
    # Compute fibonacci multiple times to make it more intensive
    total: int = 0
    i: int = 0
    while i < 10:
        total = total + fibonacci(32)
        i = i + 1

    print("Result:", total)

if __name__ == "__main__":
    main()
