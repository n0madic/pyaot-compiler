# Benchmark: string workloads — str(int) conversion, join, slicing, count,
# find, case mapping. Checksums are exact ints / strings.


def build(n: int) -> str:
    parts = []
    for i in range(n):
        parts.append("item" + str(i))
    return ",".join(parts)


def main() -> None:
    s = build(20000)
    print("bench_str len:", len(s))

    total = 0
    for _ in range(1000):
        total += s.count("9")
        total += s.find("item19999")
    print("bench_str count/find checksum:", total)

    acc = 0
    for _ in range(300):
        u = s.upper()
        acc += len(u)
        mid = s[1000:2000]
        acc += len(mid)
        acc += ord(mid[0])
    print("bench_str case/slice checksum:", acc)


main()
