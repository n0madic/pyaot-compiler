# Float arithmetic hot loop.
# Exercises IntToFloat coercion, FP multiplication, and FP accumulation.

def main() -> None:
    n: int = 1_000_000
    total: float = 0.0
    for i in range(n):
        total = total + float(i) * 0.5
    print("float_arith:", total)


if __name__ == "__main__":
    main()
