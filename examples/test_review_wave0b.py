"""Deep-structure repro for Wave 0b: iterative GC mark must not overflow the
native stack on a long reference chain (the old recursive mark_object would).
"""

from __future__ import annotations

from typing import Optional


class Node:
    val: int
    next: Optional[Node]

    def __init__(self, val: int) -> None:
        self.val = val
        self.next = None


def build_chain(n: int) -> Node:
    head = Node(0)
    cur = head
    for i in range(1, n):
        node = Node(i)
        cur.next = node
        cur = node
    return head


def main() -> None:
    head = build_chain(200000)
    # Force allocation churn -> GC collections while the deep chain is live.
    junk = []
    for i in range(1000):
        junk.append([i, i + 1])
    count = 0
    cur: Optional[Node] = head
    while cur is not None:
        count += 1
        cur = cur.next
    print(count)


main()
