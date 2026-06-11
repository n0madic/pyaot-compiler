# Benchmark: fixnum integer arithmetic in tight loops (collatz + iterative fib).
# Prints a checksum so the harness can diff the output against CPython.


def collatz_steps(n: int) -> int:
    steps = 0
    while n != 1:
        if n % 2 == 0:
            n = n // 2
        else:
            n = 3 * n + 1
        steps += 1
    return steps


def fib(n: int) -> int:
    a = 0
    b = 1
    for _ in range(n):
        t = a + b
        a = b
        b = t
    return a


def main() -> None:
    checksum = 0
    for n in range(1, 60000):
        checksum = (checksum + collatz_steps(n)) % 1000000007
    for _ in range(100000):
        checksum = (checksum + fib(60)) % 1000000007
    print("bench_int_loop checksum:", checksum)


main()
