# GC stress test — allocation-heavy tight loop.
# Builds many short-lived class instances to trigger multiple mark-sweep
# cycles and measure collection latency tail. Uses an index-based next
# pointer instead of `Node | None` because the current narrowing pass
# doesn't follow nullable references through a local binding.

class Node:
    value: int
    next_idx: int  # -1 = terminator; otherwise index into `chain`

    def __init__(self, value: int) -> None:
        self.value = value
        self.next_idx = -1


def main() -> None:
    final_sum: int = 0
    # Many short-lived chains: each iteration drops the previous chain.
    for _ in range(200):
        chain: list[Node] = []
        for i in range(1_000):
            chain.append(Node(i))
        for i in range(999):
            chain[i].next_idx = i + 1
        s: int = 0
        idx: int = 0
        while idx != -1:
            s = s + chain[idx].value
            idx = chain[idx].next_idx
        final_sum = final_sum + s
    print("gc_stress:", final_sum)


if __name__ == "__main__":
    main()
