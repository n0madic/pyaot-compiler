# Generator + comprehension iteration.
# Area G §G.3 / §G.10 territory — gen-exprs with zip/enumerate fused
# iteration, plus a plain comprehension for contrast.

def main() -> None:
    n: int = 1_000_000

    # Plain gen-expr wrapped in sum().
    total_sq: int = sum(x * x for x in range(n))

    # Gen-expr over a list with enumerate fusion.
    xs: list[int] = [i for i in range(1_000)]
    total_en: int = sum(i * v for i, v in enumerate(xs))

    # Nested comprehension — list-of-lists builder.
    grid: list[list[int]] = [[i + j for j in range(10)] for i in range(1_000)]
    cells: int = 0
    for row in grid:
        cells = cells + len(row)

    print("generators:", total_sq, total_en, cells)


if __name__ == "__main__":
    main()
