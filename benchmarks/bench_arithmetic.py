# Benchmark: Arithmetic operations
def fibonacci(n: int) -> int:
    if n <= 1:
        return n
    return fibonacci(n - 1) + fibonacci(n - 2)

def main() -> None:
    iterations: int = 1000000
    result: int = 0

    # Integer arithmetic
    i: int = 0
    while i < iterations:
        result = result + i * 2 - 1
        i = i + 1

    print("Arithmetic result:", result)

    # Fibonacci (recursive calls)
    fib_result: int = fibonacci(30)
    print("Fibonacci(30):", fib_result)

if __name__ == "__main__":
    main()
