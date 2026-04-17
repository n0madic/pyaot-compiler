# String interning + concatenation.
# Two workloads in one: (1) a tight loop that repeatedly builds the same
# string — exercises the intern pool hit-path; (2) a loop that builds
# distinct strings — exercises the slow allocation path.

def main() -> None:
    n: int = 50_000

    # Intern-hit workload: same literal, repeated.
    hits: int = 0
    for i in range(n):
        s: str = "interned-literal"
        if len(s) > 0:
            hits = hits + 1

    # Miss workload: distinct strings via concatenation.
    acc: str = ""
    for i in range(2_000):
        acc = acc + "x"

    print("strings:", hits, len(acc))


if __name__ == "__main__":
    main()
