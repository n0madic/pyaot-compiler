# Benchmark: container workloads — list append/index, dict insert/get/update,
# membership tests, iteration. Exact integer checksum.


def main() -> None:
    xs = []
    for i in range(1000000):
        xs.append(i * 3 % 1009)

    total = 0
    for _ in range(5):
        for v in xs:
            total += v
    print("bench_containers list checksum:", total % 1000000007)

    counts = {}
    for v in xs:
        if v in counts:
            counts[v] = counts[v] + 1
        else:
            counts[v] = 1

    acc = 0
    for k in counts:
        acc = (acc + k * counts[k]) % 1000000007
    print("bench_containers dict checksum:", acc)

    hits = 0
    for i in range(1000000):
        if i % 1009 in counts:
            hits += 1
    print("bench_containers membership hits:", hits)


main()
