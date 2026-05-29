"""Repro for Wave 1 generator fixes (whole-project review).

- gen.send(None) on a not-yet-started generator must prime it, not raise
  TypeError (generator.rs #8).
- itertools.chain over *generators* must not truncate — the chain iterator
  reads each inner generator's exhausted flag from the GeneratorObj layout,
  not the IteratorObj layout (iterator/next.rs #9).
"""

import itertools


def echo():
    received = yield 0
    yield received
    yield received


def gen_a():
    yield 1
    yield 2
    yield 3


def gen_b():
    yield 4
    yield 5


def test_send_none() -> None:
    g = echo()
    primed = g.send(None)  # priming idiom — must not raise TypeError
    print(primed)
    print(g.send(10))


def test_chain_generators() -> None:
    out: list[int] = []
    for v in itertools.chain(gen_a(), gen_b()):
        out.append(v)
    print(out)


def main() -> None:
    test_send_none()
    test_chain_generators()


main()
