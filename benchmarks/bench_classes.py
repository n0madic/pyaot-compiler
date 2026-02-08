# Benchmark: Class operations
class Point:
    x: int
    y: int

    def __init__(self, x: int, y: int) -> None:
        self.x = x
        self.y = y

    def distance_squared(self) -> int:
        return self.x * self.x + self.y * self.y

def main() -> None:
    iterations: int = 10000

    # Simple class usage with method calls
    total_dist: int = 0
    i: int = 0
    while i < iterations:
        p = Point(i, i * 2)
        total_dist = total_dist + p.distance_squared()
        i = i + 1

    print("Total distance squared:", total_dist)

if __name__ == "__main__":
    main()
