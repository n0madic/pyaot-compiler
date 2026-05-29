# Support module for test_review_wave1_multimod.py. Importing anything from a
# second module routes compilation through the MIR merger, which is where the
# generator resume-id band (#2) and RT_MAKE_GENERATOR operand (#3) remaps live.

OFFSET: int = 100


def bump(x: int) -> int:
    return x + OFFSET
