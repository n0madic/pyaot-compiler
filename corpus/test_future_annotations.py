"""Smoke test for `from __future__ import annotations` (PEP 563).

Area D §D.7 — parse-through semantic: our AOT frontend already eagerly
re-parses string annotations and uses the top-level class pre-scan for
forward refs, so `__future__ import annotations` is a documentation
marker rather than a runtime-behaviour switch.
"""

from __future__ import annotations


class Payload:
    def __init__(self, x: int, rest: tuple[Payload, ...] = ()) -> None:
        self.x = x
        self.rest = rest

    def sum_x(self) -> int:
        total = self.x
        for p in self.rest:
            total += p.sum_x()
        return total


p = Payload(1, (Payload(2), Payload(3, (Payload(4),))))
assert p.sum_x() == 1 + 2 + (3 + 4)
print("Future-annotations smoke test: PASS")
