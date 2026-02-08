# Benchmark: Matrix multiplication (pure computation)
def matrix_multiply() -> int:
    size: int = 100
    result: int = 0

    # Matrix multiplication
    i: int = 0
    while i < size:
        j: int = 0
        while j < size:
            k: int = 0
            sum_val: int = 0
            while k < size:
                sum_val = sum_val + i * j + k
                k = k + 1
            result = result + sum_val
            j = j + 1
        i = i + 1

    return result

def main() -> None:
    result: int = matrix_multiply()
    print("Matrix result:", result)

if __name__ == "__main__":
    main()
