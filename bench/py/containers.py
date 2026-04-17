# Container allocation + iteration + mutation.
# Stresses the list/dict runtime paths: alloc, append, key lookup,
# item update, and ordered iteration.

def main() -> None:
    n: int = 100_000

    # list: allocation + append + indexed iteration + mutation
    lst: list[int] = []
    for i in range(n):
        lst.append(i)

    total: int = 0
    for i in range(n):
        total = total + lst[i]
    for i in range(n):
        lst[i] = lst[i] + 1

    # dict: insertion, lookup, update, iteration
    d: dict[int, int] = {}
    for i in range(n):
        d[i] = i * 2
    hits: int = 0
    for i in range(n):
        if d[i] == i * 2:
            hits = hits + 1

    iter_sum: int = 0
    for v in d.values():
        iter_sum = iter_sum + v

    print("containers:", total, hits, iter_sum)


if __name__ == "__main__":
    main()
