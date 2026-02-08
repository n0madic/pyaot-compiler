# Benchmark: Prime number computation
def is_prime(n: int) -> bool:
    if n <= 1:
        return False
    if n <= 3:
        return True
    if n % 2 == 0 or n % 3 == 0:
        return False

    i: int = 5
    while i * i <= n:
        if n % i == 0 or n % (i + 2) == 0:
            return False
        i = i + 6

    return True

def main() -> None:
    limit: int = 10000
    count: int = 0

    n: int = 2
    while n < limit:
        if is_prime(n):
            count = count + 1
        n = n + 1

    print("Primes found:", count)

if __name__ == "__main__":
    main()
