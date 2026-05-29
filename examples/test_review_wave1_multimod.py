"""Repro for Wave 1 multi-module merger fixes (#2 + #3).

Importing from a second module forces compilation through the MIR merger.
A generator in this (main) module then exercises resume-function renumbering:
the merger must preserve `resume_id = original_id + 10000` (so codegen's
`resume_id - 10000` round-trips) and remap the generator func_id baked into
the RT_MAKE_GENERATOR constant operand. Without both fixes this panics a
debug_assert (debug build) or mis-dispatches the resume (release build).
"""

from genmod import bump


def count_up(n: int):
    i: int = 0
    while i < n:
        yield i
        i = i + 1


def squares(n: int):
    i: int = 0
    while i < n:
        yield i * i
        i = i + 1


def main() -> None:
    print(bump(5))

    a: list[int] = []
    for v in count_up(5):
        a.append(v)
    print(a)

    b: list[int] = []
    for v in squares(5):
        b.append(v)
    print(b)


main()
