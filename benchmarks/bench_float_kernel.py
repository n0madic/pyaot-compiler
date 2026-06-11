# Benchmark: annotated float kernel (mandelbrot escape counts) — the Raw(F64)
# specialization target. The checksum is an exact integer (sum of iteration
# counts), so the output diffs cleanly against CPython.


def mandelbrot(width: int, height: int, max_iter: int) -> int:
    total = 0
    for y in range(height):
        ci: float = y * 2.0 / height - 1.0
        for x in range(width):
            cr: float = x * 3.5 / width - 2.5
            zr: float = 0.0
            zi: float = 0.0
            it = 0
            while it < max_iter and zr * zr + zi * zi <= 4.0:
                t: float = zr * zr - zi * zi + cr
                zi = 2.0 * zr * zi + ci
                zr = t
                it += 1
            total += it
    return total


def main() -> None:
    total = mandelbrot(400, 250, 256)
    print("bench_float_kernel checksum:", total)


main()
