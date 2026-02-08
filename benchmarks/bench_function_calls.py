# Benchmark: Function call overhead
def add(a: int, b: int) -> int:
    return a + b

def multiply(a: int, b: int) -> int:
    return a * b

def compute(x: int, y: int) -> int:
    result: int = add(x, y)
    result = multiply(result, 2)
    result = add(result, x)
    return result

def main() -> None:
    iterations: int = 100000
    total: int = 0

    i: int = 0
    while i < iterations:
        total = total + compute(i, i + 1)
        i = i + 1

    print("Function call result:", total)

if __name__ == "__main__":
    main()
